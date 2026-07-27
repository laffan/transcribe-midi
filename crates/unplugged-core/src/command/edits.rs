//! The editor's gestures, each returning a transaction the session can undo.
//!
//! Split from `command.rs` under the 700-line rule. Nothing here mutates anything: a
//! gesture computes the notes it wants and hands them to [`EditSession::apply`], which is
//! the only thing that may touch a track.

use super::*;

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

/// Merge the selected notes into one, spanning from the first attack to the last
/// release.
///
/// The pitch of the *earliest* note wins, because joining is what you do to a line
/// the transcriber chopped up — a held note broken into three by a vibrato wobble, or
/// a slur it heard as two. The note you meant is the one that started, and the rest
/// are the fragments.
///
/// Fewer than two notes is a no-op rather than an error: pressing the key with one
/// note selected should do nothing, not complain.
pub fn join_notes(session: &EditSession, track: usize, indices: &[usize]) -> Result<Transaction> {
    let notes = session.track(track)?;

    let mut selected: Vec<usize> = indices
        .iter()
        .copied()
        .filter(|&index| index < notes.notes.len())
        .collect();
    selected.sort_unstable();
    selected.dedup();

    if selected.len() < 2 {
        return Ok(Transaction::new("Join notes", Vec::new()));
    }

    let first = selected
        .iter()
        .map(|&index| notes.notes[index])
        .min_by_key(Note::order_key)
        .expect("at least two are selected");
    let start = selected
        .iter()
        .map(|&index| notes.notes[index].start_ticks)
        .min()
        .unwrap_or(first.start_ticks);
    let end = selected
        .iter()
        .map(|&index| {
            let note = notes.notes[index];
            note.start_ticks + note.duration_ticks
        })
        .max()
        .unwrap_or(start + 1);

    let joined = Note {
        start_ticks: start,
        duration_ticks: (end - start).max(1),
        ..first
    };

    // Delete then insert, rather than replacing one and deleting the rest: a
    // transaction's commands apply in order, and the indices in a later command would
    // refer to a list the earlier one has already re-sorted.
    Ok(Transaction::new(
        "Join notes",
        vec![
            Command::Delete { track, indices: selected },
            Command::Insert { track, notes: vec![joined] },
        ],
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
