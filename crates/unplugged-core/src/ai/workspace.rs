//! The scratch copy tools operate on, and the bookkeeping that makes it diffable.
//!
//! This file holds the state and the shared helpers only; the tool implementations live
//! in [`super::ops`] and [`super::generate`], and turning a finished workspace into a
//! transaction lives in [`super::diff`]. They are all `impl Workspace` blocks over the
//! fields declared here, which is why those fields are `pub(super)` — visible to the rest
//! of the `ai` module and to nothing else.

use std::collections::HashSet;

use crate::error::{CoreError, Result};
use crate::model::Note;
use crate::music::{self, Key};

use super::context::AiContext;
use super::tools::ToolCall;
use super::{MAX_NOTES_PER_CALL, MAX_WORKSPACE_NOTES};

/// Identity for a note inside a workspace.
///
/// Notes have no id in the domain model — they are addressed by index — but indices
/// shift the moment anything is transposed into a new sort position, and a selection
/// that silently drifted onto different notes between two tool calls would be a very
/// hard bug to see. Ids below `original.len()` are original notes, at their original
/// index; anything higher was created during this conversation.
pub(super) type NoteId = u64;

#[derive(Debug, Clone, Copy)]
pub(super) struct Entry {
    pub(super) id: NoteId,
    pub(super) note: Note,
}

/// A scratch copy of one track that tools operate on.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub(super) context: AiContext,
    pub(super) original: Vec<Note>,
    pub(super) entries: Vec<Entry>,
    pub(super) selection: Vec<NoteId>,
    pub(super) next_id: NoteId,
}

impl Workspace {
    /// `selection` is the editor's current selection, as indices into `notes`.
    pub fn new(context: AiContext, notes: &[Note], selection: &[usize]) -> Self {
        let entries: Vec<Entry> = notes
            .iter()
            .enumerate()
            .map(|(index, note)| Entry { id: index as NoteId, note: *note })
            .collect();

        // An empty editor selection means "the whole track": a prompt like "quantize
        // this" with nothing selected should act on everything, which is what every
        // other editor does and what the user plainly means.
        let selection: Vec<NoteId> = if selection.is_empty() {
            entries.iter().map(|e| e.id).collect()
        } else {
            selection
                .iter()
                .filter(|&&index| index < entries.len())
                .map(|&index| index as NoteId)
                .collect()
        };

        Workspace {
            context,
            original: notes.to_vec(),
            next_id: entries.len() as NoteId,
            entries,
            selection,
        }
    }

    pub fn context(&self) -> &AiContext {
        &self.context
    }

    pub fn notes(&self) -> Vec<Note> {
        self.entries.iter().map(|e| e.note).collect()
    }

    pub fn selection_len(&self) -> usize {
        self.selection.len()
    }

    /// Run one tool call, returning the text handed back to the model.
    pub fn apply(&mut self, call: &ToolCall) -> Result<String> {
        match call {
            ToolCall::SelectNotes(args) => self.select(args),
            ToolCall::Transpose(args) => self.transpose(args.semitones),
            ToolCall::TransposeToKey(args) => self.transpose_to_key(args),
            ToolCall::FitToScale(args) => self.fit_to_scale(args),
            ToolCall::Quantize(args) => self.quantize(args),
            ToolCall::Humanize(args) => self.humanize(args),
            ToolCall::SetVelocity(args) => self.set_velocity(args),
            ToolCall::SetDuration(args) => self.set_duration(args),
            ToolCall::InsertNotes(args) => self.insert_notes(args),
            ToolCall::DeleteNotes(_) => self.delete_notes(),
            ToolCall::Duplicate(args) => self.duplicate(args),
            ToolCall::Invert(args) => self.invert(args),
            ToolCall::Retrograde(_) => self.retrograde(),
            ToolCall::Arpeggiate(args) => self.arpeggiate(args),
            ToolCall::Harmonize(args) => self.harmonize(args),
            ToolCall::InsertChordProgression(args) => self.insert_progression(args),
        }
    }

    // -- internals ----------------------------------------------------------

    pub(super) fn resort(&mut self) {
        // Stable, so two notes at the same start and pitch keep their relative order and
        // their ids stay put — otherwise the diff would report spurious changes.
        self.entries.sort_by_key(|e| e.note.order_key());
    }

    pub(super) fn selected_ids(&self) -> Vec<NoteId> {
        let live: HashSet<NoteId> = self.entries.iter().map(|e| e.id).collect();
        self.selection
            .iter()
            .copied()
            .filter(|id| live.contains(id))
            .collect()
    }

    /// Positions of the selected notes, in musical order.
    pub(super) fn selected_positions(&self) -> Vec<usize> {
        let selected: HashSet<NoteId> = self.selection.iter().copied().collect();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| selected.contains(&e.id))
            .map(|(index, _)| index)
            .collect()
    }

    pub(super) fn require_selection(&self) -> Result<Vec<usize>> {
        let positions = self.selected_positions();
        if positions.is_empty() {
            return Err(CoreError::Invalid(
                "nothing is selected — call select_notes first".into(),
            ));
        }
        Ok(positions)
    }

    /// Edit every selected note in place, then restore sort order.
    pub(super) fn map_selected(&mut self, mut f: impl FnMut(&mut Note)) -> Result<usize> {
        let positions = self.require_selection()?;
        for index in &positions {
            f(&mut self.entries[*index].note);
        }
        self.resort();
        Ok(positions.len())
    }

    pub(super) fn add(&mut self, note: Note) -> NoteId {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Entry { id, note });
        id
    }

    pub(super) fn guard_capacity(&self, adding: usize) -> Result<()> {
        if adding > MAX_NOTES_PER_CALL {
            return Err(CoreError::Invalid(format!(
                "that would add {adding} notes; at most {MAX_NOTES_PER_CALL} per call"
            )));
        }
        if self.entries.len() + adding > MAX_WORKSPACE_NOTES {
            return Err(CoreError::Invalid(format!(
                "the track would exceed {MAX_WORKSPACE_NOTES} notes"
            )));
        }
        Ok(())
    }

    pub(super) fn key_or(&self, given: Option<&String>) -> Result<Key> {
        if let Some(text) = given {
            return Key::parse(text)
                .ok_or_else(|| CoreError::Invalid(format!("\"{text}\" is not a key")));
        }
        self.context.key.ok_or_else(|| {
            CoreError::Invalid(
                "no key given and the track has no key set — pass one, e.g. \"C major\"".into(),
            )
        })
    }

    /// Describe the selection, so the model can see what it just did.
    pub(super) fn describe(&self, verb: &str, count: usize) -> String {
        let positions = self.selected_positions();
        if positions.is_empty() {
            return format!("{verb} {count} notes. Nothing is selected now.");
        }

        let notes: Vec<Note> = positions.iter().map(|&i| self.entries[i].note).collect();
        let low = notes.iter().map(|n| n.pitch).min().unwrap_or(0);
        let high = notes.iter().map(|n| n.pitch).max().unwrap_or(0);
        let start = notes.iter().map(|n| n.start_ticks).min().unwrap_or(0);
        let end = notes.iter().map(Note::end_ticks).max().unwrap_or(0);

        format!(
            "{verb} {count} notes. Selection is now {} notes, {}–{}, bars {}–{}. Track has {} notes.",
            notes.len(),
            music::pitch_name(low),
            music::pitch_name(high),
            self.context.position_label(start),
            self.context.position_label(end),
            self.entries.len(),
        )
    }
}
