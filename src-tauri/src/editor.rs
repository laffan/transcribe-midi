//! Commands for the open project: editing, undo/redo and transport.
//!
//! Every one of these routes note mutations through `EditSession::apply`. There is no
//! command here that writes a note by any other means, which is how the spec's "every
//! edit path emits commands from that layer" is actually enforced.

use serde::{Deserialize, Serialize};
use tauri::State;
use unplugged_core::command::{edits, Transaction};
use unplugged_core::{Note, Ticks, Track};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, OpenProject};

/// Everything the editor needs after any mutation.
///
/// Returned wholesale rather than as a diff: a track is a few thousand notes at most,
/// and a single authoritative payload removes any chance of the UI drifting out of sync
/// with Rust after an undo.
#[derive(Debug, Serialize)]
pub struct EditorState {
    pub tracks: Vec<Track>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub dirty: bool,
    /// Indices in the affected track that the UI should select.
    pub affected: Vec<usize>,
    pub affected_track: usize,
}

impl EditorState {
    pub fn of(open: &OpenProject, affected: Vec<usize>, affected_track: usize) -> Self {
        EditorState {
            tracks: open.tracks().to_vec(),
            can_undo: open.session.can_undo(),
            can_redo: open.session.can_redo(),
            undo_label: open.session.undo_label().map(str::to_owned),
            redo_label: open.session.redo_label().map(str::to_owned),
            dirty: open.dirty,
            affected,
            affected_track,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TransportState {
    pub playing: bool,
    pub position_ticks: Ticks,
    pub tempo_bpm: f64,
    pub loop_region: Option<(Ticks, Ticks)>,
    pub engine_running: bool,
    pub sample_rate: f64,
}

/// The editor gestures the frontend can request.
///
/// A closed set rather than raw `Command`s: the frontend describes *intent* ("move these
/// notes by this much") and Rust derives the note values. That keeps the clamping rules
/// in one place, and means a buggy or hostile webview cannot write a note that violates
/// the model's invariants.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditRequest {
    Insert { track: usize, note: Note },
    Delete { track: usize, indices: Vec<usize> },
    Move { track: usize, indices: Vec<usize>, delta_ticks: i64, delta_pitch: i32 },
    Resize { track: usize, indices: Vec<usize>, delta_ticks: i64 },
    SetVelocity { track: usize, indices: Vec<usize>, velocity: u8 },
    Quantize { track: usize, indices: Vec<usize>, grid_ticks: u32 },
    Paste { track: usize, notes: Vec<Note>, at_ticks: Ticks },
}

fn locked<'a>(
    state: &'a State<'_, AppState>,
) -> CommandResult<std::sync::MutexGuard<'a, Option<OpenProject>>> {
    state
        .open
        .lock()
        .map_err(|_| CommandError::from("the editor state lock was poisoned".to_string()))
}

fn no_project() -> CommandError {
    CommandError::from("no project is open".to_string())
}

// ---------------------------------------------------------------------------
// Open / close / save
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn open_project(state: State<'_, AppState>, id: String) -> CommandResult<EditorState> {
    let project = state.store.load(&id)?;
    let open = OpenProject::new(project);

    // Bring the audio engine in line with the project before anything can play.
    state.audio.set_tempo(open.manifest.tempo_bpm);
    state.audio.set_timeline(open.timeline());
    let _ = state.audio.ensure_tracks(open.tracks().len());

    let result = EditorState::of(&open, Vec::new(), 0);
    *locked(&state)? = Some(open);
    Ok(result)
}

#[tauri::command]
pub fn close_project(state: State<'_, AppState>) -> CommandResult<()> {
    let _ = state.audio.stop();
    state.audio.set_timeline(Default::default());
    *locked(&state)? = None;
    Ok(())
}

#[tauri::command]
pub fn save_open_project(state: State<'_, AppState>) -> CommandResult<EditorState> {
    let mut guard = locked(&state)?;
    let open = guard.as_mut().ok_or_else(no_project)?;

    let mut project = open.to_project();
    state.store.save(&mut project)?;

    open.manifest = project.manifest;
    open.dirty = false;
    Ok(EditorState::of(open, Vec::new(), 0))
}

#[tauri::command]
pub fn editor_state(state: State<'_, AppState>) -> CommandResult<EditorState> {
    let guard = locked(&state)?;
    let open = guard.as_ref().ok_or_else(no_project)?;
    Ok(EditorState::of(open, Vec::new(), 0))
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn apply_edit(state: State<'_, AppState>, request: EditRequest) -> CommandResult<EditorState> {
    let mut guard = locked(&state)?;
    let open = guard.as_mut().ok_or_else(no_project)?;

    let transaction: Transaction = match request {
        EditRequest::Insert { track, note } => edits::insert_note(track, note),
        EditRequest::Delete { track, indices } => edits::delete_notes(track, indices),
        EditRequest::Move { track, indices, delta_ticks, delta_pitch } => {
            edits::move_notes(&open.session, track, &indices, delta_ticks, delta_pitch)?
        }
        EditRequest::Resize { track, indices, delta_ticks } => {
            edits::resize_notes(&open.session, track, &indices, delta_ticks)?
        }
        EditRequest::SetVelocity { track, indices, velocity } => {
            edits::set_velocity(&open.session, track, &indices, velocity)?
        }
        EditRequest::Quantize { track, indices, grid_ticks } => {
            edits::quantize(&open.session, track, &indices, grid_ticks)?
        }
        EditRequest::Paste { track, notes, at_ticks } => edits::paste(track, &notes, at_ticks),
    };

    let outcome = open.session.apply(transaction)?;
    open.dirty = true;

    // Playback must reflect the edit immediately — this is what makes a note audible the
    // moment it is drawn while the transport is rolling.
    state.audio.set_timeline(open.timeline());

    Ok(EditorState::of(open, outcome.affected, outcome.track))
}

#[tauri::command]
pub fn undo(state: State<'_, AppState>) -> CommandResult<EditorState> {
    let mut guard = locked(&state)?;
    let open = guard.as_mut().ok_or_else(no_project)?;

    let outcome = open.session.undo()?.unwrap_or_default();
    open.dirty = true;
    state.audio.set_timeline(open.timeline());

    Ok(EditorState::of(open, outcome.affected, outcome.track))
}

#[tauri::command]
pub fn redo(state: State<'_, AppState>) -> CommandResult<EditorState> {
    let mut guard = locked(&state)?;
    let open = guard.as_mut().ok_or_else(no_project)?;

    let outcome = open.session.redo()?.unwrap_or_default();
    open.dirty = true;
    state.audio.set_timeline(open.timeline());

    Ok(EditorState::of(open, outcome.affected, outcome.track))
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

fn transport_snapshot(state: &State<'_, AppState>) -> TransportState {
    TransportState {
        playing: state.audio.is_playing(),
        position_ticks: state.audio.position_ticks(),
        tempo_bpm: state.audio.transport().tempo(),
        loop_region: state.audio.loop_region(),
        engine_running: state.audio.backend().is_running(),
        sample_rate: state.audio.sample_rate(),
    }
}

#[tauri::command]
pub fn transport_play(state: State<'_, AppState>) -> CommandResult<TransportState> {
    state.audio.play().map_err(|e| CommandError::from(e.to_string()))?;
    Ok(transport_snapshot(&state))
}

#[tauri::command]
pub fn transport_stop(state: State<'_, AppState>) -> CommandResult<TransportState> {
    state.audio.stop().map_err(|e| CommandError::from(e.to_string()))?;
    Ok(transport_snapshot(&state))
}

#[tauri::command]
pub fn transport_seek(state: State<'_, AppState>, tick: Ticks) -> CommandResult<TransportState> {
    state.audio.seek(tick);
    Ok(transport_snapshot(&state))
}

#[tauri::command]
pub fn transport_get(state: State<'_, AppState>) -> CommandResult<TransportState> {
    Ok(transport_snapshot(&state))
}

#[tauri::command]
pub fn set_tempo(state: State<'_, AppState>, bpm: f64) -> CommandResult<TransportState> {
    if !bpm.is_finite() || !(unplugged_core::MIN_TEMPO..=unplugged_core::MAX_TEMPO).contains(&bpm) {
        return Err(CommandError::from(format!("tempo {bpm} is out of range")));
    }
    state.audio.set_tempo(bpm);

    if let Ok(mut guard) = state.open.lock() {
        if let Some(open) = guard.as_mut() {
            open.manifest.tempo_bpm = bpm;
            open.dirty = true;
        }
    }
    Ok(transport_snapshot(&state))
}

#[tauri::command]
pub fn set_loop_region(
    state: State<'_, AppState>,
    region: Option<(Ticks, Ticks)>,
) -> CommandResult<TransportState> {
    state.audio.set_loop_region(region);
    Ok(transport_snapshot(&state))
}

// ---------------------------------------------------------------------------
// Live input
// ---------------------------------------------------------------------------

/// Play a note immediately, bypassing the sequencer.
///
/// Used by the on-screen keyboard, and from Phase 4 by external MIDI input. These notes
/// are not on the timeline, so they are not scheduled — they sound now.
#[tauri::command]
pub fn live_note_on(
    state: State<'_, AppState>,
    track: u16,
    pitch: u8,
    velocity: u8,
    channel: u8,
) -> CommandResult<()> {
    state
        .audio
        .note_on(track, pitch.min(127), velocity.clamp(1, 127), channel.min(15))
        .map_err(|e| CommandError::from(e.to_string()))
}

#[tauri::command]
pub fn live_note_off(
    state: State<'_, AppState>,
    track: u16,
    pitch: u8,
    channel: u8,
) -> CommandResult<()> {
    state
        .audio
        .note_off(track, pitch.min(127), channel.min(15))
        .map_err(|e| CommandError::from(e.to_string()))
}

#[tauri::command]
pub fn panic_all_notes_off(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .audio
        .all_notes_off()
        .map_err(|e| CommandError::from(e.to_string()))
}
