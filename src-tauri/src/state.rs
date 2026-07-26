use std::path::PathBuf;
use std::sync::Mutex;

use unplugged_audio::AudioEngine;
use unplugged_core::command::EditSession;
use unplugged_core::sequencer::Timeline;
use unplugged_core::{Project, ProjectManifest, ProjectStore, Track};

/// The project currently open in the editor.
///
/// Held in memory so that edits do not round-trip through disk on every keystroke. The
/// manifest and the edit session are kept together because saving needs both.
pub struct OpenProject {
    pub manifest: ProjectManifest,
    pub session: EditSession,
    /// Set on any edit, cleared on save. Lets the UI show unsaved state and lets close
    /// warn rather than silently discarding work.
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

/// Application state shared by every command.
///
/// The open project sits behind a `Mutex` because commands arrive from the webview on
/// arbitrary threads. The audio engine does **not** — its shared transport is built from
/// atomics precisely so the audio thread never waits on this lock.
pub struct AppState {
    pub store: ProjectStore,
    pub audio: AudioEngine,
    pub open: Mutex<Option<OpenProject>>,
}

impl AppState {
    pub fn new(app_data_dir: PathBuf, audio: AudioEngine) -> Self {
        AppState {
            store: ProjectStore::new(app_data_dir.join("projects")),
            audio,
            open: Mutex::new(None),
        }
    }
}
