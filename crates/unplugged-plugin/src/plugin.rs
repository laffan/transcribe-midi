//! Everything the extension owns, and the render path the host calls into.
//!
//! No C types below this line except [`CRenderedEvent`], which is the output buffer's
//! element. That is deliberate: this file is ordinary Rust that the test suite can drive
//! directly, and [`abi`](super::abi) is the only place that deals in raw pointers.
//!
//! [`Plugin::render`] runs on the audio thread. It allocates nothing and locks nothing;
//! everything it touches is sized by [`Plugin::prepare`].

use std::path::PathBuf;

use serde::Serialize;
use unplugged_core::host_sync::{HostFollower, HostTransport};
use unplugged_core::sequencer::{RenderedEvent, Sequencer, Timeline};
use unplugged_core::ProjectStore;

use super::event::CRenderedEvent;
use super::MAX_EVENTS;

/// Everything the extension owns. One per plugin instance.
pub struct Plugin {
    store: ProjectStore,
    sequencer: Sequencer,
    follower: HostFollower,
    /// Preallocated. The audio thread must not allocate.
    events: Vec<RenderedEvent>,
    scratch: Vec<RenderedEvent>,

    ppq: u16,
    sample_rate: f64,
    /// `pub(crate)` so the host-tempo test can assert what the follower resolved to;
    /// there is no reason for anything outside this crate to read it.
    pub(crate) last_tempo: f64,
    /// Id of the open project, so the host can save and restore it.
    open_id: Option<String>,
    track_count: usize,
    last_error: Option<String>,
}

/// What the host persists in its session and hands back on reload.
///
/// Only the project's *id*, not its contents. The project lives in the shared app-data
/// directory and the app is its editor; copying notes into the host's session would give
/// the same project two owners that could disagree.
#[derive(Debug, Clone, Serialize, serde::Deserialize, Default)]
pub struct PluginState {
    #[serde(default)]
    pub project_id: Option<String>,
}

/// A project as the plugin's picker needs it.
#[derive(Debug, Serialize)]
struct ProjectEntry {
    id: String,
    name: String,
    tempo_bpm: f64,
    track_count: usize,
    modified_at_ms: u64,
}

impl Plugin {
    pub fn new(data_dir: PathBuf) -> Self {
        Plugin {
            store: ProjectStore::new(data_dir.join("projects")),
            sequencer: Sequencer::new(48_000.0, unplugged_core::DEFAULT_PPQ, 120.0),
            follower: HostFollower::new(),
            events: Vec::with_capacity(MAX_EVENTS),
            scratch: Vec::with_capacity(MAX_EVENTS),
            ppq: unplugged_core::DEFAULT_PPQ,
            sample_rate: 48_000.0,
            last_tempo: 120.0,
            open_id: None,
            track_count: 0,
            last_error: None,
        }
    }

    pub fn projects_json(&self) -> String {
        // A projects directory that cannot be read is an empty list, not an error: the
        // view has nowhere to show one, and "no projects" is the honest rendering of
        // "the app has never run on this machine" anyway.
        let entries: Vec<ProjectEntry> = self
            .store
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|summary| ProjectEntry {
                id: summary.id,
                name: summary.name,
                tempo_bpm: summary.tempo_bpm,
                track_count: summary.track_count,
                modified_at_ms: summary.modified_at_ms,
            })
            .collect();
        serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into())
    }

    /// Load a project and hand its notes to the sequencer.
    ///
    /// Off the audio thread. The host calls this from the view, or when restoring state.
    pub fn open(&mut self, id: &str) -> Result<(), String> {
        let project = self.store.load(id).map_err(|e| e.to_string())?;

        self.ppq = project.manifest.ppq;
        self.track_count = project.tracks.len();
        self.sequencer = Sequencer::new(self.sample_rate, self.ppq, project.manifest.tempo_bpm);
        self.sequencer
            .set_timeline(Timeline::from_tracks(&project.tracks));
        self.last_tempo = project.manifest.tempo_bpm;
        // The position we had belonged to the previous project's timeline.
        self.follower.reset();
        self.open_id = Some(id.to_string());
        Ok(())
    }

    pub fn close(&mut self) {
        self.sequencer.set_timeline(Timeline::default());
        self.open_id = None;
        self.track_count = 0;
        self.follower.reset();
    }

    /// Sample rate changed, or the host reallocated render resources.
    pub fn prepare(&mut self, sample_rate: f64) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
            self.sequencer.set_sample_rate(sample_rate);
        }
        self.follower.reset();
    }

    /// One render block. **Audio thread.** Allocation-free and lock-free.
    ///
    /// The tempo comes from the host every block rather than from the project: inside a
    /// host, the host's tempo is the truth, and a plugin that insisted on its own would
    /// drift against everything else in the session.
    pub fn render(&mut self, host: HostTransport, frames: u32, out: &mut [CRenderedEvent]) -> u32 {
        self.events.clear();

        if host.tempo_bpm.is_finite() && host.tempo_bpm > 0.0 && host.tempo_bpm != self.last_tempo {
            self.sequencer.set_tempo(host.tempo_bpm);
            self.last_tempo = host.tempo_bpm;
        }

        let samples_per_tick = self.sequencer.samples_per_tick();
        let sync = self.follower.follow(
            host,
            self.sequencer.position_ticks(),
            self.ppq,
            frames,
            samples_per_tick,
        );

        // A seek flushes anything held, which is why it is issued sparingly — see
        // `host_sync`. The releases it produces have to survive into this block's output.
        if let Some(tick) = sync.seek_to {
            self.sequencer.seek(tick, &mut self.events);
        }

        if sync.playing != self.sequencer.is_playing() {
            if sync.playing {
                self.sequencer.play();
            } else {
                self.sequencer.stop(&mut self.events);
            }
        }

        // `render` clears its output, so anything emitted above is carried across.
        if self.events.is_empty() {
            self.sequencer.render(frames, &mut self.events);
        } else {
            self.scratch.clear();
            self.sequencer.render(frames, &mut self.scratch);
            self.events.extend_from_slice(&self.scratch);
        }

        let count = self.events.len().min(out.len()).min(MAX_EVENTS);
        for (slot, event) in out.iter_mut().zip(self.events.iter()).take(count) {
            *slot = CRenderedEvent::from(*event);
        }
        count as u32
    }

    pub fn state(&self) -> PluginState {
        PluginState { project_id: self.open_id.clone() }
    }

    pub fn set_state(&mut self, state: PluginState) {
        match state.project_id {
            Some(id) => {
                if let Err(error) = self.open(&id) {
                    // A session that references a project since deleted or renamed must
                    // not fail to load — the host is restoring, and refusing would lose
                    // everything else about the session too.
                    self.last_error = Some(format!("could not reopen \"{id}\": {error}"));
                    self.close();
                }
            }
            None => self.close(),
        }
    }

    pub fn track_count(&self) -> usize {
        self.track_count
    }

    /// Record a message for the next `unplugged_plugin_last_error` call.
    ///
    /// The C entry points cannot return both a status and a string, so a failure is
    /// reported as a code and the explanation is picked up separately.
    pub fn record_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }
}
