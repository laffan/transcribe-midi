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
//!
//! | Module | Holds |
//! |---|---|
//! | [`transaction`] | what an edit *is*: a command, its grouping, and what it affected |
//! | [`session`] | what applies one: the tracks, the two stacks, and validation |
//! | [`edits`] | gesture → transaction, so every caller emits the same commands |

pub mod edits;
mod session;
mod transaction;

#[cfg(test)]
mod tests;

pub use session::EditSession;
pub use transaction::{Command, EditOutcome, Transaction};

/// How many transactions of history to keep. Beyond this the oldest is dropped.
pub const MAX_HISTORY: usize = 200;
