//! The playback engine's timing core.
//!
//! This module is deliberately pure: no atomics, no FFI, no platform types. The audio
//! thread owns a [`Sequencer`] and calls [`Sequencer::render`] once per render quantum;
//! everything about *when* a note fires is decided here.
//!
//! That purity is the point. The Swift audio graph cannot be compiled or run on a
//! non-Apple host, so if tick-to-sample conversion and loop wrapping lived over there
//! they would be untestable. Here they are covered by unit tests that run anywhere.
//!
//! Three files, in the order the data flows:
//!
//! | Module | Holds |
//! |---|---|
//! | [`event`] | what a note boundary is, before and after placement in a buffer |
//! | [`timeline`] | a whole project flattened and sorted, built off the audio thread |
//! | [`scheduler`] | the [`Sequencer`] itself: the cursor, the loop, the metronome |
//!
//! The scheduler is one file on purpose. Everything reachable from
//! [`Sequencer::render`] runs on the audio thread and must not allocate or lock, and that
//! invariant is far easier to audit when the whole call graph sits in front of you.

mod event;
mod scheduler;
mod timeline;

#[cfg(test)]
mod tests;

pub use event::{EventKind, RenderedEvent, TimelineEvent};
pub use scheduler::Sequencer;
pub use timeline::Timeline;

/// Maximum simultaneously-sounding notes tracked for all-notes-off purposes.
///
/// The audio thread must not allocate, so this is a hard, preallocated ceiling rather
/// than a growable list. 512 is far beyond what a MIDI track realistically sounds at
/// once; notes beyond it still play, they just are not tracked for the panic-off.
pub const MAX_SOUNDING: usize = 512;

/// Track index reserved for metronome clicks.
///
/// The audio backend routes this to its own dedicated sampler rather than a project
/// track, so the click is never affected by track mute, solo or gain — and never turns
/// up in an exported file.
pub const METRONOME_TRACK: u16 = u16::MAX;

/// Bar-start click. Higher than the off-beat so downbeats are unmistakable.
pub const METRONOME_DOWNBEAT_PITCH: u8 = 84;
pub const METRONOME_BEAT_PITCH: u8 = 76;
pub const METRONOME_VELOCITY: u8 = 110;
