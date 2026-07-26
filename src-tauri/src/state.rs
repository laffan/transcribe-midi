use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use unplugged_audio::AudioEngine;
use unplugged_core::command::EditSession;
use unplugged_core::recorder::Recorder;
use unplugged_core::sequencer::Timeline;
use unplugged_core::{Project, ProjectManifest, ProjectStore, Ticks, Track};
use unplugged_midi::MidiInputHost;

/// The project currently open in the editor.
pub struct OpenProject {
    pub manifest: ProjectManifest,
    pub session: EditSession,
    /// Set on any edit, cleared on save.
    pub dirty: bool,
}

impl OpenProject {
    pub fn new(project: Project) -> Self {
        OpenProject {
            manifest: project.manifest,
            session: EditSession::new(project.tracks),
            dirty: false,
        }
    }

    pub fn timeline(&self) -> Timeline {
        Timeline::from_tracks(self.session.tracks())
    }

    pub fn to_project(&self) -> Project {
        Project {
            manifest: self.manifest.clone(),
            tracks: self.session.tracks().to_vec(),
        }
    }

    pub fn tracks(&self) -> &[Track] {
        self.session.tracks()
    }
}

/// Live-input and recording state.
///
/// Separate from `OpenProject` because MIDI keeps arriving whether or not a project is
/// open, and the callback thread must be able to take this lock without contending with
/// the editor.
#[derive(Default)]
pub struct InputState {
    /// Track index that live input and recording are routed to.
    pub armed_track: u16,
    /// Recording is armed and the transport is rolling past the count-in.
    pub recording: bool,
    pub recorder: Recorder,
    /// Where the take began, so a cancelled take can rewind there.
    pub record_start_tick: Ticks,
    /// Bars of count-in before recording arms. Zero disables it.
    pub count_in_bars: u8,
    /// Velocity used by the on-screen keyboard.
    pub keyboard_velocity: u8,
    /// Notes currently sounding from live input, so they can be released on panic or
    /// when the armed track changes underneath them.
    pub sounding: Vec<(u16, u8, u8)>,
}

impl InputState {
    pub fn new() -> Self {
        InputState {
            keyboard_velocity: 100,
            ..Default::default()
        }
    }
}

/// Application state shared by every command.
pub struct AppState {
    pub store: ProjectStore,
    pub audio: AudioEngine,
    pub midi: MidiInputHost,
    pub open: Mutex<Option<OpenProject>>,
    pub input: Arc<Mutex<InputState>>,
    /// Where `ai.json` lives. The projects themselves are the store's business.
    pub data_dir: PathBuf,
    pub ai_prefs: Mutex<crate::ai::AiPreferences>,
    /// The AI proposal awaiting accept or reject.
    ///
    /// Held here rather than sent to the frontend on purpose: it contains a
    /// `Transaction`, and letting the webview hand one back would undo the whole point
    /// of `EditRequest` being a closed set of intents.
    pub pending_ai: Mutex<Option<crate::ai::PendingProposal>>,
}

impl AppState {
    pub fn new(app_data_dir: PathBuf, audio: AudioEngine) -> Self {
        AppState {
            store: ProjectStore::new(app_data_dir.join("projects")),
            audio,
            midi: MidiInputHost::new(),
            open: Mutex::new(None),
            input: Arc::new(Mutex::new(InputState::new())),
            ai_prefs: Mutex::new(crate::ai::AiPreferences::load(&app_data_dir)),
            pending_ai: Mutex::new(None),
            data_dir: app_data_dir,
        }
    }
}
