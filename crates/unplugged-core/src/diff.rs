//! What changed about a list of notes.
//!
//! Lifted out of `ai.rs`, where it grew up, because nothing about it is the model's: a
//! transcription, a described edit and a hand adjustment all produce the same three
//! questions — what arrived, what left, and what moved. The preview drawn on the roll is
//! this type, whoever made the change.

use serde::{Deserialize, Serialize};

use crate::Note;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteChange {
    pub before: Note,
    pub after: Note,
}

/// The preview the user sees before anything is committed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteDiff {
    pub added: Vec<Note>,
    pub removed: Vec<Note>,
    pub changed: Vec<NoteChange>,
}

impl NoteDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// One line for the console and the accept/reject prompt.
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "No change".into();
        }
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!("{} added", self.added.len()));
        }
        if !self.removed.is_empty() {
            parts.push(format!("{} removed", self.removed.len()));
        }
        if !self.changed.is_empty() {
            parts.push(format!("{} changed", self.changed.len()));
        }
        parts.join(", ")
    }
}

/// The difference between two lists of notes.
///
/// Notes are matched on **pitch and start tick**, which is what identity means to the
/// eye: a note in the same place at the same pitch is the same note, however its length
/// or velocity changed. Anything else needs a stable id, and note lists do not have one —
/// they are re-sorted on every edit.
///
/// Used where a change did not come from the workspace that produced it: a proposal whose
/// notes the user has adjusted by hand no longer matches the transaction it arrived with,
/// and the roll has to draw what is actually on offer.
pub fn between(before: &[Note], after: &[Note]) -> NoteDiff {
    use std::collections::HashMap;

    let mut remaining: HashMap<(u8, crate::Ticks), Vec<Note>> = HashMap::new();
    for note in before {
        remaining
            .entry((note.pitch, note.start_ticks))
            .or_default()
            .push(*note);
    }

    let mut diff = NoteDiff::default();
    for note in after {
        match remaining.get_mut(&(note.pitch, note.start_ticks)).and_then(Vec::pop) {
            Some(previous) if previous == *note => {}
            Some(previous) => diff.changed.push(NoteChange { before: previous, after: *note }),
            None => diff.added.push(*note),
        }
    }
    diff.removed = remaining.into_values().flatten().collect();
    diff.removed.sort_by_key(Note::order_key);
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, start: u32, duration: u32) -> Note {
        Note::new(pitch, start, duration, 90, 0).unwrap()
    }

    #[test]
    fn an_unchanged_list_produces_no_diff() {
        let notes = vec![note(60, 0, 100), note(64, 240, 100)];
        assert!(between(&notes, &notes).is_empty());
    }

    #[test]
    fn a_note_in_the_same_place_that_grew_is_a_change_not_a_swap() {
        let diff = between(&[note(60, 0, 100)], &[note(60, 0, 480)]);
        assert!(diff.added.is_empty() && diff.removed.is_empty());
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].after.duration_ticks, 480);
    }

    #[test]
    fn moving_a_note_reads_as_one_arriving_and_one_leaving() {
        // Identity is pitch and place, so a note dragged elsewhere is not "the same note
        // moved" — there is nothing in a note list that could say otherwise.
        let diff = between(&[note(60, 0, 100)], &[note(60, 480, 100)]);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn additions_and_removals_are_counted_separately() {
        let diff = between(
            &[note(60, 0, 100), note(64, 240, 100)],
            &[note(60, 0, 100), note(67, 480, 100)],
        );
        assert_eq!(diff.added, vec![note(67, 480, 100)]);
        assert_eq!(diff.removed, vec![note(64, 240, 100)]);
        assert_eq!(diff.summary(), "1 added, 1 removed");
    }

    #[test]
    fn duplicates_at_one_place_are_paired_off_rather_than_double_counted() {
        let twice = vec![note(60, 0, 100), note(60, 0, 100)];
        let once = vec![note(60, 0, 100)];
        let diff = between(&twice, &once);
        assert_eq!(diff.removed.len(), 1);
        assert!(diff.added.is_empty() && diff.changed.is_empty());
    }

    #[test]
    fn everything_arriving_from_nothing_is_all_additions() {
        let diff = between(&[], &[note(60, 0, 100), note(64, 0, 100)]);
        assert_eq!(diff.added.len(), 2);
        assert!(diff.removed.is_empty());
    }
}
