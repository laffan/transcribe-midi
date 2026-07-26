//! Tauri command surface for Phase 1.
//!
//! These are deliberately thin. Every one of them is a direct call into
//! `unplugged-core`, because this file cannot be compile-tested against the macOS or iOS
//! target on a Linux CI host — so the less logic it contains, the less goes unverified.

use tauri::State;
use unplugged_core::model::ProjectListing;
use unplugged_core::{Project, ProjectManifest, TimeSignature, TrackMeta};

use crate::error::CommandResult;
use crate::state::AppState;

/// Which build is this?
///
/// Trivial in a standalone app, genuinely hard in a plugin — Logic caches Audio Unit
/// scans, keeps the extension alive in a separate hosting process, and will happily run a
/// copy you replaced ten minutes ago. Stamped at compile time and shown in the UI, so the
/// answer comes from the running code rather than from what is installed on disk.
#[tauri::command]
pub fn build_info() -> unplugged_core::BuildInfo {
    unplugged_core::BuildInfo::get()
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> CommandResult<ProjectListing> {
    Ok(state.store.list_with_errors()?)
}

#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    name: String,
    tempo_bpm: f64,
    time_signature: TimeSignature,
) -> CommandResult<ProjectManifest> {
    Ok(state.store.create(&name, tempo_bpm, time_signature)?)
}

#[tauri::command]
pub fn load_project(state: State<'_, AppState>, id: String) -> CommandResult<Project> {
    Ok(state.store.load(&id)?)
}

#[tauri::command]
pub fn save_project(state: State<'_, AppState>, mut project: Project) -> CommandResult<ProjectManifest> {
    state.store.save(&mut project)?;
    Ok(project.manifest)
}

#[tauri::command]
pub fn rename_project(state: State<'_, AppState>, id: String, name: String) -> CommandResult<ProjectManifest> {
    Ok(state.store.rename(&id, &name)?)
}

#[tauri::command]
pub fn delete_project(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.store.delete(&id)?)
}

#[tauri::command]
pub fn add_track(
    state: State<'_, AppState>,
    project_id: String,
    name: Option<String>,
) -> CommandResult<TrackMeta> {
    Ok(state.store.add_track(&project_id, name.as_deref())?)
}

#[tauri::command]
pub fn delete_track(
    state: State<'_, AppState>,
    project_id: String,
    track_id: String,
) -> CommandResult<()> {
    Ok(state.store.delete_track(&project_id, &track_id)?)
}

/// Where projects live on disk. Shown in Settings; also the escape hatch for a user who
/// wants to back up or hand-edit a project.
#[tauri::command]
pub fn projects_root(state: State<'_, AppState>) -> CommandResult<String> {
    Ok(state.store.root().to_string_lossy().into_owned())
}
