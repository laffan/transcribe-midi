//! What the AUv3 extension links against.
//!
//! **Scope of this first pass.** The plugin plays a project the standalone app authored,
//! following the host's transport, and emits its notes as MIDI. It does not yet host the
//! editor UI — that needs the whole Tauri command surface re-homed behind a transport
//! that is not Tauri, and doing it in the same step as getting an extension to load at all
//! would mean two unverified things failing together with no way to tell which.
//!
//! So: the projects directory is shared with the app. You author in the app; the plugin
//! plays what you authored, into Logic's instrument, in sync with Logic's transport.
//!
//! Two rules the whole file is shaped by:
//!
//! * **The render path allocates nothing and locks nothing.** It is called on the audio
//!   thread by the host. Everything it touches is preallocated by `prepare`.
//! * **No panic crosses the boundary.** A panic unwinding into Swift is undefined
//!   behaviour, and in a plugin it takes the host down with it — which means it takes
//!   the user's unsaved session down too. Every entry point catches.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use serde::Serialize;
use unplugged_core::host_sync::{HostFollower, HostTransport};
use unplugged_core::sequencer::{RenderedEvent, Sequencer, Timeline};
use unplugged_core::{BuildInfo, ProjectStore};

/// Events one block may produce. Beyond this the block is truncated rather than
/// allocating on the audio thread.
const MAX_EVENTS: usize = 512;

/// Mirrors `CRenderedEvent` in `unplugged-audio`, and the C header the Swift side reads.
/// Field order and padding must match exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CRenderedEvent {
    pub frame_offset: u32,
    pub track: u16,
    /// 0 = note off, 1 = note on.
    pub kind: u8,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
    pub _pad: [u8; 2],
}

impl From<RenderedEvent> for CRenderedEvent {
    fn from(event: RenderedEvent) -> Self {
        use unplugged_core::sequencer::EventKind;
        CRenderedEvent {
            frame_offset: event.frame_offset,
            track: event.track,
            kind: match event.kind {
                EventKind::NoteOn => 1,
                EventKind::NoteOff => 0,
            },
            pitch: event.pitch,
            velocity: event.velocity,
            channel: event.channel,
            _pad: [0; 2],
        }
    }
}

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
    last_tempo: f64,
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

    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------
//
// Every entry point catches panics. Unwinding into Swift is undefined behaviour, and in a
// plugin the process it would take down is the user's DAW.

fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// Hand a `String` to C. Freed with `unplugged_plugin_string_free`.
fn to_c_string(text: String) -> *mut c_char {
    CString::new(text)
        .unwrap_or_else(|_| CString::new("").expect("an empty string has no NUL"))
        .into_raw()
}

unsafe fn plugin<'a>(handle: *mut std::ffi::c_void) -> Option<&'a mut Plugin> {
    (!handle.is_null()).then(|| &mut *(handle as *mut Plugin))
}

/// # Safety
/// `data_dir` must be a NUL-terminated UTF-8 path, or NULL.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_create(
    data_dir: *const c_char,
) -> *mut std::ffi::c_void {
    guard(std::ptr::null_mut(), || {
        if data_dir.is_null() {
            return std::ptr::null_mut();
        }
        let path = PathBuf::from(CStr::from_ptr(data_dir).to_string_lossy().into_owned());
        Box::into_raw(Box::new(Plugin::new(path))) as *mut std::ffi::c_void
    })
}

/// # Safety
/// `handle` must come from `unplugged_plugin_create` and not be in use by the audio thread.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_destroy(handle: *mut std::ffi::c_void) {
    guard((), || {
        if !handle.is_null() {
            drop(Box::from_raw(handle as *mut Plugin));
        }
    })
}

/// # Safety
/// `pointer` must come from one of the functions here that returns a string.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_string_free(pointer: *mut c_char) {
    guard((), || {
        if !pointer.is_null() {
            drop(CString::from_raw(pointer));
        }
    })
}

/// JSON array of the projects in the shared app-data directory.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_projects_json(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || match plugin(handle) {
        Some(plugin) => to_c_string(plugin.projects_json()),
        None => std::ptr::null_mut(),
    })
}

/// Open a project. Returns 0, or non-zero with a message from
/// `unplugged_plugin_last_error`.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`; `id` must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_open(
    handle: *mut std::ffi::c_void,
    id: *const c_char,
) -> i32 {
    guard(-1, || {
        let Some(plugin) = plugin(handle) else { return -1 };
        if id.is_null() {
            plugin.close();
            return 0;
        }
        let id = CStr::from_ptr(id).to_string_lossy().into_owned();
        match plugin.open(&id) {
            Ok(()) => 0,
            Err(error) => {
                plugin.last_error = Some(error);
                1
            }
        }
    })
}

/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_last_error(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        match plugin(handle).and_then(Plugin::take_last_error) {
            Some(message) => to_c_string(message),
            None => std::ptr::null_mut(),
        }
    })
}

/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_prepare(
    handle: *mut std::ffi::c_void,
    sample_rate: f64,
) {
    guard((), || {
        if let Some(plugin) = plugin(handle) {
            plugin.prepare(sample_rate);
        }
    })
}

/// Number of tracks in the open project, so the view can size the channel list.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_track_count(handle: *mut std::ffi::c_void) -> u32 {
    guard(0, || plugin(handle).map_or(0, |p| p.track_count() as u32))
}

/// The host's `fullState`, as JSON.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_state_json(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || match plugin(handle) {
        Some(plugin) => to_c_string(
            serde_json::to_string(&plugin.state()).unwrap_or_else(|_| "{}".into()),
        ),
        None => std::ptr::null_mut(),
    })
}

/// Restore from the host's `fullState`. Never fails destructively.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`; `json` must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_set_state_json(
    handle: *mut std::ffi::c_void,
    json: *const c_char,
) -> i32 {
    guard(-1, || {
        let Some(plugin) = plugin(handle) else { return -1 };
        if json.is_null() {
            plugin.close();
            return 0;
        }
        let raw = CStr::from_ptr(json).to_string_lossy().into_owned();
        // A session written by a newer build must not brick an older one; an unreadable
        // blob means "no project", not "refuse to load".
        let state: PluginState = serde_json::from_str(&raw).unwrap_or_default();
        plugin.set_state(state);
        0
    })
}

/// Version, commit and dirty flag, as JSON.
///
/// This is what the plugin's view shows. It is the only claim about which build is
/// running that comes from the running code rather than from what is on disk.
#[no_mangle]
pub extern "C" fn unplugged_plugin_build_info_json() -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        to_c_string(serde_json::to_string(&BuildInfo::get()).unwrap_or_else(|_| "{}".into()))
    })
}

/// One render block. **Called on the audio thread.**
///
/// # Safety
/// - `handle` must come from `unplugged_plugin_create` and not be used concurrently.
/// - `out` must be valid for `capacity` `CRenderedEvent` writes.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_render(
    handle: *mut std::ffi::c_void,
    host_beats: f64,
    tempo_bpm: f64,
    playing: bool,
    frames: u32,
    out: *mut CRenderedEvent,
    capacity: u32,
) -> u32 {
    guard(0, || {
        if out.is_null() || capacity == 0 {
            return 0;
        }
        let Some(plugin) = plugin(handle) else { return 0 };

        let slice = std::slice::from_raw_parts_mut(out, capacity as usize);
        plugin.render(
            HostTransport { beats: host_beats, tempo_bpm, playing },
            frames,
            slice,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use unplugged_core::{Note, TimeSignature};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("unplugged-plugin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A project on disk with one note per bar, as the app would have written it.
    fn seeded(dir: &Path, name: &str) -> String {
        let store = ProjectStore::new(dir.join("projects"));
        let manifest = store
            .create(name, 120.0, TimeSignature::new(4, 4).unwrap())
            .unwrap();

        let mut project = store.load(&manifest.id).unwrap();
        project.tracks[0].notes = vec![
            Note::new(60, 0, 480, 100, 0).unwrap(),
            Note::new(64, 1920, 480, 100, 0).unwrap(),
        ];
        store.save(&mut project).unwrap();
        manifest.id
    }

    fn blank() -> Vec<CRenderedEvent> {
        vec![
            CRenderedEvent {
                frame_offset: 0,
                track: 0,
                kind: 0,
                pitch: 0,
                velocity: 0,
                channel: 0,
                _pad: [0; 2],
            };
            64
        ]
    }

    #[test]
    fn it_lists_and_opens_projects_the_app_wrote() {
        let dir = temp_dir("list");
        let id = seeded(&dir, "Plugin Test");

        let mut plugin = Plugin::new(dir.clone());
        let json = plugin.projects_json();
        assert!(json.contains("Plugin Test"), "{json}");

        plugin.open(&id).unwrap();
        assert_eq!(plugin.track_count(), 1);
        assert_eq!(plugin.state().project_id.as_deref(), Some(id.as_str()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opening_something_that_is_not_there_fails_without_panicking() {
        let dir = temp_dir("missing");
        let mut plugin = Plugin::new(dir.clone());
        assert!(plugin.open("no-such-project").is_err());
        assert_eq!(plugin.projects_json(), "[]");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_session_referencing_a_deleted_project_still_loads() {
        // The host is restoring a session. Refusing would lose everything else about it,
        // so a missing project must degrade to "nothing open" plus an explanation.
        let dir = temp_dir("stale");
        let mut plugin = Plugin::new(dir.clone());

        plugin.set_state(PluginState { project_id: Some("gone".into()) });
        assert_eq!(plugin.state().project_id, None);
        assert!(plugin.take_last_error().is_some_and(|e| e.contains("gone")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn state_round_trips_through_json() {
        let dir = temp_dir("state");
        let id = seeded(&dir, "Round Trip");

        let mut plugin = Plugin::new(dir.clone());
        plugin.open(&id).unwrap();

        let json = serde_json::to_string(&plugin.state()).unwrap();
        let mut restored = Plugin::new(dir.clone());
        restored.set_state(serde_json::from_str(&json).unwrap());

        assert_eq!(restored.state().project_id.as_deref(), Some(id.as_str()));
        assert_eq!(restored.track_count(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_blob_from_a_newer_build_means_no_project_rather_than_a_refusal() {
        let dir = temp_dir("future");
        let mut plugin = Plugin::new(dir.clone());
        let state: PluginState = serde_json::from_str(r#"{"unknown_field":42}"#).unwrap_or_default();
        plugin.set_state(state);
        assert_eq!(plugin.state().project_id, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rendering_follows_the_host_and_emits_the_projects_notes() {
        let dir = temp_dir("render");
        let id = seeded(&dir, "Render");

        let mut plugin = Plugin::new(dir.clone());
        plugin.prepare(48_000.0);
        plugin.open(&id).unwrap();

        let mut out = blank();
        // 120 bpm, 48 kHz: a bar is two seconds, so walk a second of 512-frame blocks and
        // the note at tick 0 must fire in the first one.
        let mut total = 0;
        let mut first_note_on = None;

        for block in 0..94 {
            let beats = (block * 512) as f64 / 48_000.0 * 2.0;
            let count = plugin.render(
                HostTransport { beats, tempo_bpm: 120.0, playing: true },
                512,
                &mut out,
            ) as usize;
            for event in &out[..count] {
                if event.kind == 1 && first_note_on.is_none() {
                    first_note_on = Some((block, event.pitch));
                }
            }
            total += count;
        }

        assert!(total > 0, "the project's notes never fired");
        let (block, pitch) = first_note_on.expect("a note-on");
        assert_eq!(pitch, 60);
        assert_eq!(block, 0, "the note at tick 0 belongs in the first block");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stopped_host_produces_nothing() {
        let dir = temp_dir("stopped");
        let id = seeded(&dir, "Stopped");

        let mut plugin = Plugin::new(dir.clone());
        plugin.prepare(48_000.0);
        plugin.open(&id).unwrap();

        let mut out = blank();
        for _ in 0..20 {
            let count = plugin.render(
                HostTransport { beats: 0.0, tempo_bpm: 120.0, playing: false },
                512,
                &mut out,
            );
            assert_eq!(count, 0);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rendering_with_nothing_open_is_silent_rather_than_a_crash() {
        let dir = temp_dir("empty");
        let mut plugin = Plugin::new(dir.clone());
        plugin.prepare(44_100.0);

        let mut out = blank();
        for block in 0..10 {
            let count = plugin.render(
                HostTransport { beats: block as f64, tempo_bpm: 120.0, playing: true },
                512,
                &mut out,
            );
            assert_eq!(count, 0);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_output_buffer_is_never_overrun() {
        let dir = temp_dir("overrun");
        let store = ProjectStore::new(dir.join("projects"));
        let manifest = store
            .create("Dense", 120.0, TimeSignature::new(4, 4).unwrap())
            .unwrap();

        // Far more simultaneous notes than the caller's buffer can hold.
        let mut project = store.load(&manifest.id).unwrap();
        project.tracks[0].notes = (0u8..=127)
            .map(|pitch| Note::new(pitch, 0, 480, 100, 0).unwrap())
            .collect();
        store.save(&mut project).unwrap();

        let mut plugin = Plugin::new(dir.clone());
        plugin.prepare(48_000.0);
        plugin.open(&manifest.id).unwrap();

        let mut small = vec![
            CRenderedEvent {
                frame_offset: 0,
                track: 0,
                kind: 0,
                pitch: 0,
                velocity: 0,
                channel: 0,
                _pad: [0; 2],
            };
            8
        ];
        let count = plugin.render(
            HostTransport { beats: 0.0, tempo_bpm: 120.0, playing: true },
            512,
            &mut small,
        );
        assert!(count as usize <= small.len(), "wrote past the caller's buffer");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_host_tempo_wins_over_the_projects() {
        let dir = temp_dir("tempo");
        let store = ProjectStore::new(dir.join("projects"));
        let manifest = store
            .create("Tempo", 90.0, TimeSignature::new(4, 4).unwrap())
            .unwrap();
        let mut project = store.load(&manifest.id).unwrap();
        project.tracks[0].notes = vec![Note::new(60, 960, 480, 100, 0).unwrap()];
        store.save(&mut project).unwrap();

        let mut plugin = Plugin::new(dir.clone());
        plugin.prepare(48_000.0);
        plugin.open(&manifest.id).unwrap();

        // The project says 90; the host says 140. Inside a host, the host is the truth —
        // a plugin that kept its own tempo would drift against everything else.
        let mut out = blank();
        plugin.render(
            HostTransport { beats: 0.0, tempo_bpm: 140.0, playing: true },
            512,
            &mut out,
        );
        assert!((plugin.last_tempo - 140.0).abs() < 1e-9);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_c_abi_survives_null_everywhere() {
        // Every one of these is reachable from Swift, and any of them crashing takes the
        // host down with the user's session.
        unsafe {
            assert!(unplugged_plugin_create(std::ptr::null()).is_null());
            unplugged_plugin_destroy(std::ptr::null_mut());
            unplugged_plugin_string_free(std::ptr::null_mut());
            assert!(unplugged_plugin_projects_json(std::ptr::null_mut()).is_null());
            assert_eq!(unplugged_plugin_open(std::ptr::null_mut(), std::ptr::null()), -1);
            assert!(unplugged_plugin_last_error(std::ptr::null_mut()).is_null());
            unplugged_plugin_prepare(std::ptr::null_mut(), 48_000.0);
            assert_eq!(unplugged_plugin_track_count(std::ptr::null_mut()), 0);
            assert!(unplugged_plugin_state_json(std::ptr::null_mut()).is_null());
            assert_eq!(
                unplugged_plugin_set_state_json(std::ptr::null_mut(), std::ptr::null()),
                -1
            );
            assert_eq!(
                unplugged_plugin_render(std::ptr::null_mut(), 0.0, 120.0, true, 512, std::ptr::null_mut(), 0),
                0
            );

            // The one that always works: the build stamp does not need an instance,
            // because the view shows it before anything is loaded.
            let info = unplugged_plugin_build_info_json();
            assert!(!info.is_null());
            let text = CStr::from_ptr(info).to_string_lossy().into_owned();
            assert!(text.contains("version"), "{text}");
            unplugged_plugin_string_free(info);
        }
    }
}
