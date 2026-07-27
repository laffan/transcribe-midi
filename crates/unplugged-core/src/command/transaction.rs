//! What an edit is, before anything has applied it.
//!
//! Every variant of [`Command`] carries enough to state its own inverse, which is what
//! makes undo a property of the data rather than of the code that walks it.

use serde::{Deserialize, Serialize};

use crate::model::Note;

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
