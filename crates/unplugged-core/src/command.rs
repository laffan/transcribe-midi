//! The single mutation path for note data, with undo/redo.
//!
//! The spec is emphatic that *every* edit — mouse, keyboard, AI — goes through this
//! layer and nothing mutates notes directly. That is enforced structurally rather than
//! by convention: [`EditSession`] owns the tracks and hands out only `&`, so the only
//! way to change a note is [`EditSession::apply`].
//!
//! Undo is implemented by storing the **inverse** of each applied command rather than
//! snapshotting the whole track. Snapshots would be simpler, but Phase 6's AI edits can
//! touch thousands of notes in one transaction and the history would grow without bound.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks, Track};

/// How many transactions of history to keep. Beyond this the oldest is dropped.
pub const MAX_HISTORY: usize = 200;

/// A single note-level mutation.
///
/// Deliberately low-level: musical operations (transpose, quantize, harmonize — the
/// Phase 6 AI tool surface) compose from these rather than being variants of their own.
/// One inverse implementation to get right instead of twenty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Insert notes into a track. Indices are assigned by sort order.
    Insert { track: usize, notes: Vec<Note> },
    /// Remove the notes at these indices. Indices refer to the pre-command state.
    Delete { track: usize, indices: Vec<usize> },
    /// Replace the notes at these indices wholesale. Used for drag, resize, velocity,
    /// transpose and quantize alike — they differ only in how the caller computes the
    /// replacement.
    Replace {
        track: usize,
        indices: Vec<usize>,
        notes: Vec<Note>,
    },
}

impl Command {
    pub fn track(&self) -> usize {
        match self {
            Command::Insert { track, .. }
            | Command::Delete { track, .. }
            | Command::Replace { track, .. } => *track,
        }
    }
}

/// A group of commands applied and undone as one unit.
///
/// The spec requires AI edits to be a single undoable transaction; the same mechanism
/// covers a marquee drag that moves fifty notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub label: String,
    pub commands: Vec<Command>,
}

impl Transaction {
    pub fn new(label: impl Into<String>, commands: Vec<Command>) -> Self {
        Transaction {
            label: label.into(),
            commands,
        }
    }

    pub fn single(label: impl Into<String>, command: Command) -> Self {
        Transaction::new(label, vec![command])
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

/// What changed, so the UI can update a selection without reloading the track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EditOutcome {
    /// Indices (post-command) of notes that now exist as a result of this transaction.
    pub affected: Vec<usize>,
    pub track: usize,
}

/// Owns the tracks and is the only thing that may mutate them.
#[derive(Debug, Clone)]
pub struct EditSession {
    tracks: Vec<Track>,
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    /// Labels of the undo entries, newest last. Mirrors `undo_stack` for display.
    undo_labels: Vec<String>,
    redo_labels: Vec<String>,
}

impl EditSession {
    pub fn new(tracks: Vec<Track>) -> Self {
        EditSession {
            tracks,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_labels: Vec::new(),
            redo_labels: Vec::new(),
        }
    }

    /// Read-only access. There is deliberately no `&mut` accessor.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Append a track.
    ///
    /// Structure, not notes. The undo stack covers note mutations — that is the invariant
    /// this type exists to enforce — and adding a track is not one, so this does not push
    /// a history entry and undoing past it leaves the track in place, empty. That matches
    /// how the Add Track button already behaves; the alternative is a second kind of
    /// history entry that every command would have to reason about.
    pub fn push_track(&mut self, track: Track) {
        self.tracks.push(track);
    }

    pub fn track(&self, index: usize) -> Result<&Track> {
        self.tracks
            .get(index)
            .ok_or_else(|| CoreError::TrackNotFound(index.to_string()))
    }

    pub fn into_tracks(self) -> Vec<Track> {
        self.tracks
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo_labels.last().map(String::as_str)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo_labels.last().map(String::as_str)
    }

    /// Everything on the undo stack, oldest first.
    ///
    /// Exposed so the editor can *show* the chain of transformations rather than only the
    /// top of it. When the main way to change notes is describing what you want, the
    /// sequence you described is the document as much as the notes are, and a label
    /// visible only inside the Edit menu is not a sequence anyone can work with.
    pub fn history(&self) -> &[String] {
        &self.undo_labels
    }

    /// Undone entries, most recently undone last — the redo direction.
    pub fn redo_history(&self) -> &[String] {
        &self.redo_labels
    }

    // -----------------------------------------------------------------------
    // Apply / undo / redo
    // -----------------------------------------------------------------------

    /// Apply a transaction, pushing its inverse onto the undo stack.
    ///
    /// Applying anything clears the redo stack — the classic linear-history model. An
    /// empty transaction is a no-op and does *not* push a history entry, so a drag that
    /// ends where it started leaves no undo step to puzzle over.
    pub fn apply(&mut self, transaction: Transaction) -> Result<EditOutcome> {
        if transaction.is_empty() {
            return Ok(EditOutcome::default());
        }

        let inverse = self.apply_inner(&transaction)?;

        self.undo_stack.push(inverse);
        self.undo_labels.push(transaction.label.clone());
        self.redo_stack.clear();
        self.redo_labels.clear();

        if self.undo_stack.len() > MAX_HISTORY {
            self.undo_stack.remove(0);
            self.undo_labels.remove(0);
        }

        Ok(self.outcome_for(&transaction))
    }

    pub fn undo(&mut self) -> Result<Option<EditOutcome>> {
        let Some(inverse) = self.undo_stack.pop() else {
            return Ok(None);
        };
        let label = self.undo_labels.pop().unwrap_or_default();

        let redo = self.apply_inner(&inverse)?;
        let outcome = self.outcome_for(&inverse);

        self.redo_stack.push(redo);
        self.redo_labels.push(label);
        Ok(Some(outcome))
    }

    pub fn redo(&mut self) -> Result<Option<EditOutcome>> {
        let Some(transaction) = self.redo_stack.pop() else {
            return Ok(None);
        };
        let label = self.redo_labels.pop().unwrap_or_default();

        let inverse = self.apply_inner(&transaction)?;
        let outcome = self.outcome_for(&transaction);

        self.undo_stack.push(inverse);
        self.undo_labels.push(label);
        Ok(Some(outcome))
    }

    /// Apply and return the inverse transaction.
    ///
    /// Validation happens for the whole transaction before anything is mutated, so a
    /// transaction either fully applies or leaves the session untouched. A partially
    /// applied edit would be unundoable.
    fn apply_inner(&mut self, transaction: &Transaction) -> Result<Transaction> {
        for command in &transaction.commands {
            self.validate(command)?;
        }

        let mut inverses = Vec::with_capacity(transaction.commands.len());
        for command in &transaction.commands {
            inverses.push(self.apply_command(command)?);
        }
        // Undoing runs the inverses in reverse order, mirroring how they were applied.
        inverses.reverse();

        Ok(Transaction::new(transaction.label.clone(), inverses))
    }

    fn validate(&self, command: &Command) -> Result<()> {
        let track = self.track(command.track())?;

        match command {
            Command::Insert { notes, .. } => {
                for note in notes {
                    note.validate()?;
                }
            }
            Command::Delete { indices, .. } => {
                for &index in indices {
                    if index >= track.notes.len() {
                        return Err(CoreError::TrackNotFound(format!(
                            "note index {index} out of range (track has {})",
                            track.notes.len()
                        )));
                    }
                }
            }
            Command::Replace { indices, notes, .. } => {
                if indices.len() != notes.len() {
                    return Err(CoreError::TrackNotFound(format!(
                        "replace needs one note per index ({} indices, {} notes)",
                        indices.len(),
                        notes.len()
                    )));
                }
                for &index in indices {
                    if index >= track.notes.len() {
                        return Err(CoreError::TrackNotFound(format!(
                            "note index {index} out of range (track has {})",
                            track.notes.len()
                        )));
                    }
                }
                for note in notes {
                    note.validate()?;
                }
            }
        }
        Ok(())
    }

    fn apply_command(&mut self, command: &Command) -> Result<Command> {
        match command {
            Command::Insert { track, notes } => {
                let index = *track;
                for note in notes {
                    self.tracks[index].insert_note(*note)?;
                }
                // Inverse: delete the notes we just inserted, found by their new
                // positions. Resolved *after* insertion so the indices are correct.
                let indices = self.indices_of(index, notes);
                Ok(Command::Delete { track: index, indices })
            }

            Command::Delete { track, indices } => {
                let track_index = *track;
                let mut sorted = indices.clone();
                // Descending, so each removal cannot shift a later index.
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                sorted.dedup();

                let mut removed = Vec::with_capacity(sorted.len());
                for &index in &sorted {
                    removed.push(self.tracks[track_index].notes.remove(index));
                }
                removed.reverse();

                Ok(Command::Insert { track: track_index, notes: removed })
            }

            Command::Replace { track, indices, notes } => {
                let track_index = *track;

                // Capture the originals before touching anything.
                let originals: Vec<Note> = indices.iter().map(|&i| self.tracks[track_index].notes[i]).collect();

                // A replacement can change `start_ticks` or `pitch`, which changes sort
                // position — so remove-then-insert rather than assign in place, or the
                // list would silently stop being sorted.
                let mut descending: Vec<(usize, Note)> =
                    indices.iter().copied().zip(notes.iter().copied()).collect();
                descending.sort_unstable_by(|a, b| b.0.cmp(&a.0));

                for &(index, _) in &descending {
                    self.tracks[track_index].notes.remove(index);
                }
                for &(_, note) in &descending {
                    self.tracks[track_index].insert_note(note)?;
                }

                // Inverse: put the originals back where the *new* notes now sit.
                let new_indices = self.indices_of(track_index, notes);
                Ok(Command::Replace {
                    track: track_index,
                    indices: new_indices,
                    notes: originals,
                })
            }
        }
    }

    /// Positions of `notes` within a track, matching each exactly once.
    ///
    /// Duplicates matter: two identical notes must map to two distinct indices, so a
    /// matched position is marked used rather than being found again.
    fn indices_of(&self, track: usize, notes: &[Note]) -> Vec<usize> {
        let haystack = &self.tracks[track].notes;
        let mut used = vec![false; haystack.len()];
        let mut out = Vec::with_capacity(notes.len());

        for note in notes {
            if let Some(index) = haystack
                .iter()
                .enumerate()
                .position(|(i, candidate)| !used[i] && candidate == note)
            {
                used[index] = true;
                out.push(index);
            }
        }

        out.sort_unstable();
        out
    }

    fn outcome_for(&self, transaction: &Transaction) -> EditOutcome {
        let track = transaction.commands.first().map(Command::track).unwrap_or(0);

        let mut affected = Vec::new();
        for command in &transaction.commands {
            if command.track() != track {
                continue;
            }
            match command {
                Command::Insert { notes, .. } | Command::Replace { notes, .. } => {
                    affected.extend(self.indices_of(track, notes));
                }
                Command::Delete { .. } => {}
            }
        }
        affected.sort_unstable();
        affected.dedup();

        EditOutcome { affected, track }
    }
}

// ---------------------------------------------------------------------------
// Builders for the common editor gestures
// ---------------------------------------------------------------------------

/// Helpers that turn an editor gesture into a transaction.
///
/// These exist so the piano roll, the keyboard shortcuts and (in Phase 6) the AI tool
/// loop all produce *the same* commands rather than three parallel implementations.
pub mod edits {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InstrumentRef, TrackMeta, DEFAULT_PPQ};

    fn note(pitch: u8, start: u32, dur: u32, vel: u8) -> Note {
        Note::new(pitch, start, dur, vel, 0).unwrap()
    }

    fn session(notes: Vec<Note>) -> EditSession {
        let mut track = Track::new(
            TrackMeta {
                id: "t".into(), name: "T".into(), channel: 0,
                instrument: InstrumentRef::BuiltInSampler, muted: false, soloed: false,
                color: "#fff".into(), key_hint: None,
            },
            DEFAULT_PPQ,
        );
        track.notes = notes;
        track.sort_notes();
        EditSession::new(vec![track])
    }

    fn pitches(s: &EditSession) -> Vec<u8> {
        s.tracks()[0].notes.iter().map(|n| n.pitch).collect()
    }

    // -- basics -------------------------------------------------------------

    #[test]
    fn insert_then_undo_restores_the_original() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        s.apply(edits::insert_note(0, note(64, 480, 480, 100))).unwrap();
        assert_eq!(pitches(&s), vec![60, 64]);

        s.undo().unwrap();
        assert_eq!(pitches(&s), vec![60]);

        s.redo().unwrap();
        assert_eq!(pitches(&s), vec![60, 64]);
    }

    #[test]
    fn delete_then_undo_restores_notes_in_the_right_places() {
        let mut s = session(vec![note(60, 0, 480, 100), note(64, 480, 480, 90), note(67, 960, 480, 80)]);
        s.apply(edits::delete_notes(0, vec![0, 2])).unwrap();
        assert_eq!(pitches(&s), vec![64]);

        s.undo().unwrap();
        assert_eq!(pitches(&s), vec![60, 64, 67], "order must be restored, not appended");
        assert!(s.tracks()[0].is_sorted());
    }

    #[test]
    fn an_empty_transaction_leaves_no_history_entry() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        s.apply(Transaction::new("Nothing", Vec::new())).unwrap();
        assert!(!s.can_undo(), "a no-op drag must not create an undo step");
    }

    #[test]
    fn applying_clears_the_redo_stack() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        s.apply(edits::insert_note(0, note(64, 480, 480, 100))).unwrap();
        s.undo().unwrap();
        assert!(s.can_redo());

        s.apply(edits::insert_note(0, note(67, 960, 480, 100))).unwrap();
        assert!(!s.can_redo(), "a new edit must invalidate the redo branch");
    }

    #[test]
    fn undo_and_redo_report_nothing_when_the_stacks_are_empty() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        assert!(s.undo().unwrap().is_none());
        assert!(s.redo().unwrap().is_none());
    }

    #[test]
    fn history_labels_track_the_stacks() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        assert_eq!(s.undo_label(), None);

        s.apply(edits::insert_note(0, note(64, 480, 480, 100))).unwrap();
        assert_eq!(s.undo_label(), Some("Insert note"));

        s.undo().unwrap();
        assert_eq!(s.redo_label(), Some("Insert note"));
        assert_eq!(s.undo_label(), None);
    }

    // -- ordering invariants ------------------------------------------------

    #[test]
    fn moving_a_note_past_another_keeps_the_list_sorted() {
        // This is the case that breaks an in-place assignment implementation: note 0
        // moves after note 1, so its sort position changes.
        let mut s = session(vec![note(60, 0, 480, 100), note(64, 480, 480, 90)]);

        let tx = edits::move_notes(&s, 0, &[0], 960, 0).unwrap();
        s.apply(tx).unwrap();

        assert!(s.tracks()[0].is_sorted(), "list must remain sorted after a reordering move");
        assert_eq!(pitches(&s), vec![64, 60]);

        s.undo().unwrap();
        assert_eq!(pitches(&s), vec![60, 64]);
        assert!(s.tracks()[0].is_sorted());
    }

    #[test]
    fn transposing_across_another_note_at_the_same_tick_keeps_sorting() {
        // Same start tick, so ordering is decided by pitch.
        let mut s = session(vec![note(60, 0, 480, 100), note(64, 0, 480, 90)]);
        let tx = edits::move_notes(&s, 0, &[0], 0, 12).unwrap(); // 60 -> 72
        s.apply(tx).unwrap();

        assert!(s.tracks()[0].is_sorted());
        assert_eq!(pitches(&s), vec![64, 72]);
    }

    #[test]
    fn undo_restores_exactly_after_a_multi_note_move() {
        let original = vec![note(60, 0, 480, 100), note(64, 480, 240, 90), note(67, 960, 120, 80)];
        let mut s = session(original.clone());

        let tx = edits::move_notes(&s, 0, &[0, 1, 2], 240, 3).unwrap();
        s.apply(tx).unwrap();
        assert_ne!(s.tracks()[0].notes, original);

        s.undo().unwrap();
        assert_eq!(s.tracks()[0].notes, original, "undo must be exact, not approximate");
    }

    // -- clamping -----------------------------------------------------------

    #[test]
    fn moving_past_the_edges_clamps_rather_than_dropping_notes() {
        let mut s = session(vec![note(2, 100, 480, 100), note(125, 100, 480, 100)]);

        let tx = edits::move_notes(&s, 0, &[0, 1], -10_000, 10).unwrap();
        s.apply(tx).unwrap();

        let notes = &s.tracks()[0].notes;
        assert_eq!(notes.len(), 2, "no note may be lost at the boundary");
        assert!(notes.iter().all(|n| n.start_ticks == 0), "time clamps at zero");
        assert_eq!(notes.iter().map(|n| n.pitch).max(), Some(127), "pitch clamps at 127");
    }

    #[test]
    fn resizing_below_zero_leaves_a_representable_note() {
        let mut s = session(vec![note(60, 0, 100, 100)]);
        let tx = edits::resize_notes(&s, 0, &[0], -500).unwrap();
        s.apply(tx).unwrap();
        assert_eq!(s.tracks()[0].notes[0].duration_ticks, 1, "zero-length is unrepresentable in SMF");
    }

    #[test]
    fn velocity_is_clamped_into_the_legal_range() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        s.apply(edits::set_velocity(&s.clone(), 0, &[0], 0).unwrap()).unwrap();
        // Velocity 0 would be read back as a note-off, so it must never be stored.
        assert_eq!(s.tracks()[0].notes[0].velocity, 1);
    }

    // -- duplicates ---------------------------------------------------------

    #[test]
    fn identical_notes_are_tracked_separately() {
        // Two notes that compare equal must not collapse into one on undo.
        let mut s = session(vec![note(60, 0, 480, 100), note(60, 0, 480, 100)]);
        assert_eq!(s.tracks()[0].notes.len(), 2);

        s.apply(edits::delete_notes(0, vec![0])).unwrap();
        assert_eq!(s.tracks()[0].notes.len(), 1);

        s.undo().unwrap();
        assert_eq!(s.tracks()[0].notes.len(), 2, "both duplicates must come back");
    }

    // -- transactions -------------------------------------------------------

    #[test]
    fn a_multi_command_transaction_undoes_as_one_unit() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        let tx = Transaction::new(
            "Harmonize",
            vec![
                Command::Insert { track: 0, notes: vec![note(64, 0, 480, 100)] },
                Command::Insert { track: 0, notes: vec![note(67, 0, 480, 100)] },
            ],
        );
        s.apply(tx).unwrap();
        assert_eq!(pitches(&s), vec![60, 64, 67]);

        s.undo().unwrap();
        assert_eq!(pitches(&s), vec![60], "one undo must revert the whole transaction");
    }

    #[test]
    fn an_invalid_command_leaves_the_session_untouched() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        let before = s.tracks()[0].notes.clone();

        // Second command is invalid; the first must not be applied either.
        let tx = Transaction::new(
            "Half bad",
            vec![
                Command::Insert { track: 0, notes: vec![note(64, 0, 480, 100)] },
                Command::Delete { track: 0, indices: vec![999] },
            ],
        );
        assert!(s.apply(tx).is_err());
        assert_eq!(s.tracks()[0].notes, before, "a rejected transaction must not partially apply");
        assert!(!s.can_undo(), "and must not create a history entry");
    }

    #[test]
    fn replace_rejects_mismatched_index_and_note_counts() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        let tx = Transaction::single(
            "Bad replace",
            Command::Replace { track: 0, indices: vec![0], notes: vec![] },
        );
        assert!(s.apply(tx).is_err());
    }

    #[test]
    fn an_unknown_track_is_an_error() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        assert!(s.apply(edits::insert_note(9, note(60, 0, 480, 100))).is_err());
    }

    // -- musical helpers ----------------------------------------------------

    #[test]
    fn quantize_snaps_to_the_nearest_grid_line_in_both_directions() {
        let mut s = session(vec![note(60, 10, 480, 100), note(64, 230, 480, 100)]);
        let tx = edits::quantize(&s, 0, &[0, 1], 240).unwrap();
        s.apply(tx).unwrap();

        let starts: Vec<u32> = s.tracks()[0].notes.iter().map(|n| n.start_ticks).collect();
        assert_eq!(starts, vec![0, 240], "10 rounds down, 230 rounds up");
    }

    #[test]
    fn quantize_with_a_zero_grid_is_a_no_op_rather_than_a_divide_by_zero() {
        let mut s = session(vec![note(60, 10, 480, 100)]);
        let tx = edits::quantize(&s, 0, &[0], 0).unwrap();
        s.apply(tx).unwrap();
        assert_eq!(s.tracks()[0].notes[0].start_ticks, 10);
    }

    #[test]
    fn paste_preserves_relative_timing_within_the_group() {
        let mut s = session(vec![]);
        let clipboard = vec![note(60, 1000, 240, 100), note(64, 1240, 240, 100)];

        s.apply(edits::paste(0, &clipboard, 0)).unwrap();

        let starts: Vec<u32> = s.tracks()[0].notes.iter().map(|n| n.start_ticks).collect();
        assert_eq!(starts, vec![0, 240], "the 240-tick gap must survive the move to zero");
    }

    #[test]
    fn pasting_an_empty_clipboard_does_nothing() {
        let mut s = session(vec![note(60, 0, 480, 100)]);
        s.apply(edits::paste(0, &[], 480)).unwrap();
        assert_eq!(s.tracks()[0].notes.len(), 1);
        assert!(!s.can_undo());
    }

    // -- history bounds -----------------------------------------------------

    #[test]
    fn history_is_bounded() {
        let mut s = session(vec![]);
        for i in 0..(MAX_HISTORY + 50) {
            s.apply(edits::insert_note(0, note(60, i as u32 * 10, 5, 100))).unwrap();
        }
        assert_eq!(s.undo_stack.len(), MAX_HISTORY, "history must not grow without bound");
        assert_eq!(s.undo_labels.len(), MAX_HISTORY, "labels must stay in step with the stack");
    }

    #[test]
    fn a_long_undo_redo_run_stays_consistent() {
        let original = vec![note(60, 0, 480, 100), note(64, 480, 480, 90)];
        let mut s = session(original.clone());

        for i in 0..20 {
            let tx = edits::move_notes(&s, 0, &[0], 24, if i % 2 == 0 { 1 } else { -1 }).unwrap();
            s.apply(tx).unwrap();
        }
        for _ in 0..20 {
            s.undo().unwrap();
        }
        assert_eq!(s.tracks()[0].notes, original, "20 undos must land exactly on the start state");

        for _ in 0..20 {
            s.redo().unwrap();
        }
        for _ in 0..20 {
            s.undo().unwrap();
        }
        assert_eq!(s.tracks()[0].notes, original, "and again after a full redo/undo cycle");
    }
}
