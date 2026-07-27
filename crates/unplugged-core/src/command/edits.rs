//! Helpers that turn an editor gesture into a transaction.
//!
//! These exist so the piano roll, the keyboard shortcuts and the AI tool loop all produce
//! *the same* commands rather than three parallel implementations. Clamping lives here
//! too: a drag past the top of the keyboard is a note at pitch 127, not an error.

use crate::error::Result;
use crate::model::{Note, Ticks};

use super::session::EditSession;
use super::transaction::{Command, Transaction};


pub fn insert_note(track: usize, note: Note) -> Transaction {
    Transaction::single("Insert note", Command::Insert { track, notes: vec![note] })
}

pub fn delete_notes(track: usize, indices: Vec<usize>) -> Transaction {
    Transaction::single("Delete notes", Command::Delete { track, indices })
}

/// Move notes in time and pitch. Notes are clamped rather than dropped at the edges,
/// so dragging a selection into the wall does not silently lose the outliers.
pub fn move_notes(
    session: &EditSession,
    track: usize,
    indices: &[usize],
    delta_ticks: i64,
    delta_pitch: i32,
) -> Result<Transaction> {
    let notes = session.track(track)?;
    let moved: Vec<Note> = indices
        .iter()
        .map(|&index| {
            let mut note = notes.notes[index];
            note.start_ticks = (note.start_ticks as i64 + delta_ticks).max(0) as Ticks;
            note.pitch = (note.pitch as i32 + delta_pitch).clamp(0, 127) as u8;
            note
        })
        .collect();

    Ok(Transaction::single(
        "Move notes",
        Command::Replace { track, indices: indices.to_vec(), notes: moved },
    ))
}

/// Resize by a tick delta applied to the end. Minimum length is one tick — a
/// zero-length note cannot be represented in SMF.
pub fn resize_notes(
    session: &EditSession,
    track: usize,
    indices: &[usize],
    delta_ticks: i64,
) -> Result<Transaction> {
    let notes = session.track(track)?;
    let resized: Vec<Note> = indices
        .iter()
        .map(|&index| {
            let mut note = notes.notes[index];
            note.duration_ticks = (note.duration_ticks as i64 + delta_ticks).max(1) as Ticks;
            note
        })
        .collect();

    Ok(Transaction::single(
        "Resize notes",
        Command::Replace { track, indices: indices.to_vec(), notes: resized },
    ))
}

pub fn set_velocity(
    session: &EditSession,
    track: usize,
    indices: &[usize],
    velocity: u8,
) -> Result<Transaction> {
    let notes = session.track(track)?;
    let velocity = velocity.clamp(1, 127);
    let updated: Vec<Note> = indices
        .iter()
        .map(|&index| {
            let mut note = notes.notes[index];
            note.velocity = velocity;
            note
        })
        .collect();

    Ok(Transaction::single(
        "Set velocity",
        Command::Replace { track, indices: indices.to_vec(), notes: updated },
    ))
}

/// Snap note starts to the nearest multiple of `grid_ticks`.
pub fn quantize(
    session: &EditSession,
    track: usize,
    indices: &[usize],
    grid_ticks: u32,
) -> Result<Transaction> {
    if grid_ticks == 0 {
        return Ok(Transaction::new("Quantize", Vec::new()));
    }
    let notes = session.track(track)?;
    let grid = grid_ticks as f64;

    let quantized: Vec<Note> = indices
        .iter()
        .map(|&index| {
            let mut note = notes.notes[index];
            note.start_ticks = ((note.start_ticks as f64 / grid).round() * grid) as Ticks;
            note
        })
        .collect();

    Ok(Transaction::single(
        "Quantize",
        Command::Replace { track, indices: indices.to_vec(), notes: quantized },
    ))
}

/// Paste at `at_ticks`, preserving the relative timing within the pasted group.
pub fn paste(track: usize, notes: &[Note], at_ticks: Ticks) -> Transaction {
    let Some(earliest) = notes.iter().map(|n| n.start_ticks).min() else {
        return Transaction::new("Paste", Vec::new());
    };

    let placed: Vec<Note> = notes
        .iter()
        .map(|note| {
            let mut note = *note;
            note.start_ticks = at_ticks + (note.start_ticks - earliest);
            note
        })
        .collect();

    Transaction::single("Paste", Command::Insert { track, notes: placed })
}
