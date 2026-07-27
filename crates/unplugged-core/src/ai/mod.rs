//! The AI tool surface: what a model is allowed to do to a track, and nothing else.
//!
//! Phase 6's model never sees a `Command` and never touches an [`EditSession`]. It calls
//! the tools defined here against a [`Workspace`] — a scratch copy of one track — and
//! when the loop finishes, the difference between the scratch copy and the original is
//! turned into **one** transaction that the user accepts or rejects. That gives three
//! things the spec asks for, structurally rather than by discipline:
//!
//! * every AI edit goes through the same command layer as every other edit;
//! * the whole conversation is a single undo step, however many tools were called;
//! * a preview diff exists for free, because the diff is how the transaction is built.
//!
//! Everything here is pure and deterministic — including `humanize`, which takes a seed.
//! A model's output is unpredictable enough without the code under it also being random.
//!
//! The module is laid out along the path a request takes:
//!
//! | Module | Holds |
//! |---|---|
//! | [`context`] | bar/beat ↔ tick conversion, and the notes table the model reads |
//! | [`tools`] | the closed set of operations, as deserialisable types |
//! | [`schema`] | the JSON schemas and descriptions sent to the API |
//! | [`workspace`] | the scratch copy, its selection, and the shared helpers |
//! | [`ops`] | tools that change existing notes |
//! | [`generate`] | tools that add new ones |
//! | [`diff`] | scratch copy vs. original → one transaction |
//! | [`rng`] | seeded jitter, so `humanize` repeats |

mod context;
mod diff;
mod generate;
mod ops;
mod rng;
mod schema;
mod tools;
mod workspace;

#[cfg(test)]
mod tests;

pub use context::{notes_table, AiContext};
pub use diff::{NoteChange, NoteDiff};
pub use schema::tool_definitions;
pub use tools::{
    Arpeggiate, Duplicate, Empty, FitToScale, Harmonize, Humanize, InsertChordProgression,
    InsertNotes, Invert, NoteSpec, QuantizeArgs, SelectNotes, SetDuration, SetVelocityArgs,
    ToolCall, Transpose, TransposeToKey,
};
pub use workspace::Workspace;

/// Most notes one tool call may add. A model that asks for more has misunderstood the
/// request, and the error tells it so rather than locking the UI up drawing them.
pub const MAX_NOTES_PER_CALL: usize = 512;

/// Ceiling on the working copy. Well beyond any hand-edited track.
pub const MAX_WORKSPACE_NOTES: usize = 20_000;
