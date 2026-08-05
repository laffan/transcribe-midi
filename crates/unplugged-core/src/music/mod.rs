//! Scales, keys and chords.
//!
//! Phase 6's AI tools speak musically — "fit to E minor", "harmonise a third above",
//! "ii–V–I" — and something has to turn that into pitch numbers. That translation is
//! pure arithmetic with no I/O, so it lives here where it can be tested exhaustively
//! rather than inside a network client whose behaviour depends on a model's output.
//!
//! Everything is expressed in **pitch classes** (0–11, C = 0) plus an octave, because
//! every musical operation here is octave-invariant. Voicing decisions — which octave a
//! harmonised note actually lands in — are made by the caller.
//!
//! The files stack, each built on the one above it:
//!
//! | Module | Holds |
//! |---|---|
//! | [`pitch`] | pitch classes and note names, in both directions |
//! | [`scale`] | the interval sets, and the names they answer to |
//! | [`key`] | a tonic plus a scale: membership, snapping, diatonic steps |
//! | [`chord`] | chord qualities, symbols like `Cmaj7`, and voicing |
//! | [`roman`] | roman numerals and progressions, which need a key to mean anything |

mod chord;
mod key;
mod pitch;
mod roman;
mod scale;

#[cfg(test)]
mod tests;

pub use chord::{Chord, ChordQuality};
pub use key::Key;
pub use pitch::{parse_pitch, parse_pitch_class, pitch_name};
pub use roman::{parse_progression, parse_roman};
pub use scale::Scale;

/// Semitones per octave. Named because `% 12` on its own reads as a magic number.
pub const OCTAVE: i32 = 12;
