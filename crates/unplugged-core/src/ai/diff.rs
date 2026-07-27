//! Turning a finished workspace back into one undoable edit.
//!
//! The diff is not a report generated beside the commit — it *is* how the commit is
//! built, so the preview the user accepts and the transaction that lands can never
//! describe different edits.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::command::{Command, Transaction};
use crate::model::Note;

use super::workspace::{Entry, NoteId, Workspace};

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

impl Workspace {
    /// True when this note is not what it was at the same id in the original.
    fn is_dirty(&self, entry: &Entry) -> bool {
        match self.original.get(entry.id as usize) {
            Some(original) => *original != entry.note,
            None => true,
        }
    }

    /// What changed, for the preview the user accepts or rejects.
    pub fn diff(&self) -> NoteDiff {
        let mut added = Vec::new();
        let mut changed = Vec::new();
        let surviving: HashMap<NoteId, Note> = self
            .entries
            .iter()
            .filter(|e| (e.id as usize) < self.original.len())
            .map(|e| (e.id, e.note))
            .collect();

        for entry in &self.entries {
            match self.original.get(entry.id as usize) {
                Some(before) if *before != entry.note => changed.push(NoteChange {
                    before: *before,
                    after: entry.note,
                }),
                Some(_) => {}
                None => added.push(entry.note),
            }
        }

        let removed: Vec<Note> = self
            .original
            .iter()
            .enumerate()
            .filter(|(index, _)| !surviving.contains_key(&(*index as NoteId)))
            .map(|(_, note)| *note)
            .collect();

        added.sort_by_key(Note::order_key);
        changed.sort_by_key(|c| c.after.order_key());

        NoteDiff { added, removed, changed }
    }

    /// Turn the whole conversation into one transaction against `track`.
    ///
    /// Expressed as a delete followed by an insert rather than a `Replace`: `Replace`
    /// indices refer to the pre-command state, and a transaction that both replaced and
    /// deleted would need its indices to survive the reordering the replace itself
    /// causes. Delete-then-insert has no such coupling and is exactly equivalent.
    pub fn to_transaction(&self, track: usize, label: impl Into<String>) -> Transaction {
        let label = label.into();
        if self.diff().is_empty() {
            return Transaction::new(label, Vec::new());
        }

        let dirty: HashSet<NoteId> = self
            .entries
            .iter()
            .filter(|entry| self.is_dirty(entry))
            .map(|entry| entry.id)
            .collect();
        let surviving: HashSet<NoteId> = self.entries.iter().map(|e| e.id).collect();

        let mut commands = Vec::new();

        // Every original note that was removed *or* modified comes out; the modified
        // ones go back in below with their new values.
        let doomed: Vec<usize> = (0..self.original.len())
            .filter(|index| {
                let id = *index as NoteId;
                !surviving.contains(&id) || dirty.contains(&id)
            })
            .collect();
        if !doomed.is_empty() {
            commands.push(Command::Delete { track, indices: doomed });
        }

        let mut fresh: Vec<Note> = self
            .entries
            .iter()
            .filter(|entry| self.is_dirty(entry))
            .map(|entry| entry.note)
            .collect();
        fresh.sort_by_key(Note::order_key);
        if !fresh.is_empty() {
            commands.push(Command::Insert { track, notes: fresh });
        }

        Transaction::new(label, commands)
    }
}
