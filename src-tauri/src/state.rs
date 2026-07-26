use std::path::PathBuf;

use unplugged_core::ProjectStore;

/// Application state shared by every command.
///
/// `ProjectStore` holds only a path and does its own filesystem access per call, so no
/// lock is needed here in Phase 1. The sequencer state added in Phase 2 will need its
/// own synchronisation; keeping this struct as the single place state is registered
/// means that arrives in one obvious spot.
pub struct AppState {
    pub store: ProjectStore,
}

impl AppState {
    pub fn new(app_data_dir: PathBuf) -> Self {
        AppState {
            store: ProjectStore::new(app_data_dir.join("projects")),
        }
    }
}
