//! The thing that applies transactions and remembers how to take them back.
//!
//! Tracks are owned here and lent out immutably, so `apply` is the only door. Validation
//! happens for the whole transaction before any of it lands — a transaction that would
//! fail halfway must leave the session exactly as it was, or undo would be describing a
//! state that never existed.

use crate::error::{CoreError, Result};
use crate::model::{Note, Track};

use super::transaction::{Command, EditOutcome, Transaction};
use super::MAX_HISTORY;

/// Owns the tracks and is the only thing that may mutate them.
#[derive(Debug, Clone)]
pub struct EditSession {
    tracks: Vec<Track>,
    /// The stacks and their label mirrors are `pub(super)` so the tests in this module can
    /// assert they stay in step; nothing outside `command` can reach them.
    pub(super) undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    /// Labels of the undo entries, newest last. Mirrors `undo_stack` for display.
    pub(super) undo_labels: Vec<String>,
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
