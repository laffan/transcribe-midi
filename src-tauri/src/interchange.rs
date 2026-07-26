//! Phase 5: export, import and sharing.
//!
//! Rust owns the bytes; the platform layer owns where they go. Every command here
//! produces or consumes a `Vec<u8>` and the actual file dialog, share sheet or
//! pasteboard write happens either in `tauri-plugin-dialog` (cross-platform) or in the
//! Swift platform target.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;
use unplugged_core::command::{Command, Transaction};
use unplugged_core::smf;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Sanitise a project or track name for use as a filename.
///
/// `/` and `:` are both path separators depending on the layer looking at the string,
/// and a leading dot hides the file. Everything else is left alone so the exported name
/// still reads like the user's.
fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' | '<' | '>' | '"' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();

    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

#[derive(Debug, Serialize)]
pub struct ExportPayload {
    /// Suggested filename including the `.mid` extension.
    pub filename: String,
    /// The file contents. Crosses the IPC boundary as a JSON number array; a project's
    /// worth of MIDI is kilobytes, so the encoding overhead is irrelevant next to the
    /// simplicity of not managing a temp file for every export.
    pub bytes: Vec<u8>,
}

fn with_open<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&crate::state::OpenProject) -> CommandResult<T>,
) -> CommandResult<T> {
    let guard = state
        .open
        .lock()
        .map_err(|_| CommandError::from("editor state lock poisoned".to_string()))?;
    let open = guard
        .as_ref()
        .ok_or_else(|| CommandError::from("no project is open".to_string()))?;
    f(open)
}

/// The whole project as SMF type 1 — the format Logic Pro and GarageBand import.
#[tauri::command]
pub fn export_project_smf(state: State<'_, AppState>) -> CommandResult<ExportPayload> {
    with_open(&state, |open| {
        let bytes = smf::project_to_smf_type1(&open.to_project())?;
        Ok(ExportPayload {
            filename: format!("{}.mid", safe_filename(&open.manifest.name)),
            bytes,
        })
    })
}

/// A single track as a standalone format-0 file carrying its own tempo.
///
/// This is what drag-out and "export track" produce; a bare note list would open at 120
/// bpm regardless of the project.
#[tauri::command]
pub fn export_track_smf(state: State<'_, AppState>, track: usize) -> CommandResult<ExportPayload> {
    with_open(&state, |open| {
        let t = open
            .tracks()
            .get(track)
            .ok_or_else(|| CommandError::from(format!("no track at index {track}")))?;

        let bytes = smf::track_to_standalone_smf(
            t,
            open.manifest.tempo_bpm,
            open.manifest.time_signature,
            open.manifest.ppq,
        )?;

        Ok(ExportPayload {
            filename: format!(
                "{} - {}.mid",
                safe_filename(&open.manifest.name),
                safe_filename(&t.meta.name)
            ),
            bytes,
        })
    })
}

/// Write an export to disk. The path comes from the platform's save dialog.
#[tauri::command]
pub fn write_export(path: String, bytes: Vec<u8>) -> CommandResult<String> {
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CommandError::from(format!("could not create {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, &bytes)
        .map_err(|e| CommandError::from(format!("could not write {}: {e}", path.display())))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Stage an export in the app's cache directory and return its path.
///
/// Sharing and drag-out both need a real file on disk with the right name: the iOS share
/// sheet takes a URL, and a macOS drag promises one. Writing to the cache means the OS
/// can reclaim it and we never litter the user's documents.
#[tauri::command]
pub fn stage_export(
    app: tauri::AppHandle,
    filename: String,
    bytes: Vec<u8>,
) -> CommandResult<String> {
    use tauri::Manager;

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| CommandError::from(format!("no cache directory: {e}")))?
        .join("exports");

    std::fs::create_dir_all(&dir)
        .map_err(|e| CommandError::from(format!("could not create {}: {e}", dir.display())))?;

    // `filename` came from `ExportPayload`, but it makes a round trip through the
    // webview before arriving here — so it is re-sanitised rather than trusted.
    let path = dir.join(safe_filename(&filename));
    std::fs::write(&path, &bytes)
        .map_err(|e| CommandError::from(format!("could not write {}: {e}", path.display())))?;

    Ok(path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ImportPreview {
    pub track_names: Vec<String>,
    pub note_counts: Vec<usize>,
    pub source_ppq: Option<u16>,
    pub tempo_bpm: Option<f64>,
    pub time_signature: Option<unplugged_core::TimeSignature>,
    /// Set when the file's resolution differs from the project's, so the UI can say that
    /// timing will be rescaled rather than letting it look like a silent change.
    pub will_rescale: bool,
}

#[tauri::command]
pub fn preview_import(state: State<'_, AppState>, path: String) -> CommandResult<ImportPreview> {
    let bytes = std::fs::read(&path)
        .map_err(|e| CommandError::from(format!("could not read {path}: {e}")))?;
    let file = smf::parse_smf_for_import(&bytes, Path::new(&path))?;

    let project_ppq = with_open(&state, |open| Ok(open.manifest.ppq)).unwrap_or(480);

    Ok(ImportPreview {
        track_names: file
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| t.name.clone().unwrap_or_else(|| format!("Imported {}", i + 1)))
            .collect(),
        note_counts: file.tracks.iter().map(|t| t.notes.len()).collect(),
        source_ppq: file.ppq,
        tempo_bpm: file.tempo_bpm,
        time_signature: file.time_signature,
        will_rescale: matches!(file.ppq, Some(ppq) if ppq != project_ppq),
    })
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub editor: crate::editor::EditorState,
    pub tracks_added: usize,
    pub notes_added: usize,
    pub rescaled_from: Option<u16>,
}

/// Import an SMF into the open project as new tracks.
///
/// Timing is rescaled to the project's PPQ. Imported files routinely use 96, 192 or 960
/// and the difference is silent but ruinous otherwise. The project's own tempo is left
/// alone — the file's tempo is reported to the UI, which offers it as a choice rather
/// than overwriting the user's setting behind their back.
#[tauri::command]
pub fn import_smf(app: tauri::AppHandle, path: String) -> CommandResult<ImportResult> {
    use tauri::Manager;

    let bytes = std::fs::read(&path)
        .map_err(|e| CommandError::from(format!("could not read {path}: {e}")))?;
    let file = smf::parse_smf_for_import(&bytes, Path::new(&path))?;

    if file.tracks.is_empty() {
        return Err(CommandError::from(
            "that file contains no notes to import".to_string(),
        ));
    }

    let state = app.state::<AppState>();
    let project_id = {
        let guard = state
            .open
            .lock()
            .map_err(|_| CommandError::from("editor state lock poisoned".to_string()))?;
        guard
            .as_ref()
            .ok_or_else(|| CommandError::from("no project is open".to_string()))?
            .manifest
            .id
            .clone()
    };

    // Create the tracks on disk first so the manifest and the session stay in step.
    let mut created = Vec::new();
    for (index, track) in file.tracks.iter().enumerate() {
        let name = track
            .name
            .clone()
            .unwrap_or_else(|| format!("Imported {}", index + 1));
        created.push(state.store.add_track(&project_id, Some(&name))?);
    }

    // Reload so the session sees the new (empty) tracks, then fill them through the
    // command layer — imported notes are undoable like any other edit.
    let reloaded = state.store.load(&project_id)?;
    let mut guard = state
        .open
        .lock()
        .map_err(|_| CommandError::from("editor state lock poisoned".to_string()))?;
    let open = guard
        .as_mut()
        .ok_or_else(|| CommandError::from("no project is open".to_string()))?;

    let project_ppq = open.manifest.ppq;
    let first_new = reloaded.tracks.len() - created.len();
    *open = crate::state::OpenProject::new(reloaded);

    let mut commands = Vec::new();
    let mut notes_added = 0;
    for (offset, track) in file.tracks.iter().enumerate() {
        let mut notes = track.notes.clone();
        if let Some(source_ppq) = file.ppq {
            smf::rescale_ppq(&mut notes, source_ppq, project_ppq);
        }
        notes_added += notes.len();
        commands.push(Command::Insert {
            track: first_new + offset,
            notes,
        });
    }

    let outcome = open
        .session
        .apply(Transaction::new("Import MIDI", commands))?;
    open.dirty = true;

    let _ = state.audio.ensure_tracks(open.tracks().len());
    state.audio.set_timeline(open.timeline());

    Ok(ImportResult {
        editor: crate::editor::EditorState::of(open, outcome.affected, outcome.track),
        tracks_added: created.len(),
        notes_added,
        rescaled_from: file.ppq.filter(|ppq| *ppq != project_ppq),
    })
}

#[cfg(test)]
mod tests {
    use super::safe_filename;

    #[test]
    fn filenames_lose_path_separators() {
        assert_eq!(safe_filename("a/b"), "a-b");
        assert_eq!(safe_filename("a\\b"), "a-b");
        assert_eq!(safe_filename("a:b"), "a-b");
        // Separators become dashes, then the leading dots are stripped. The property
        // that matters is that nothing traversable survives.
        let escaped = safe_filename("../../etc/passwd");
        assert_eq!(escaped, "-..-etc-passwd");
        assert!(!escaped.contains('/') && !escaped.contains('\\'));
    }

    #[test]
    fn hidden_and_empty_names_are_replaced() {
        assert_eq!(safe_filename(""), "Untitled");
        assert_eq!(safe_filename("   "), "Untitled");
        assert_eq!(safe_filename("..."), "Untitled");
        assert_eq!(safe_filename(".hidden"), "hidden");
    }

    #[test]
    fn ordinary_names_are_left_alone() {
        assert_eq!(safe_filename("My Song (final)"), "My Song (final)");
        assert_eq!(safe_filename("Étude nº1"), "Étude nº1");
    }

    #[test]
    fn very_long_names_are_truncated() {
        assert!(safe_filename(&"x".repeat(500)).chars().count() <= 120);
    }
}
