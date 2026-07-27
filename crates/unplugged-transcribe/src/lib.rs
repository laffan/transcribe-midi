//! Phase 7: audio to MIDI, monophonic.
//!
//! **Monophonic only, and that is a design decision rather than an omission.** Polyphonic
//! transcription is a different problem — it needs spectral factorisation or a trained
//! model, not a pitch tracker — and a version of this that quietly did its best on a
//! chord would produce plausible-looking nonsense. Given a chord, YIN reports one pitch:
//! usually the loudest partial, sometimes a difference tone, never the chord. So the UI
//! says "one note at a time" and this module makes no attempt to hide it.
//!
//! The pipeline is four stages, each in its own module and tested against synthetic
//! signals a Linux CI host can generate:
//!
//! 1. **Framing** — overlapping windows, 10 ms apart.
//! 2. **Pitch** ([`pitch`]) — YIN per frame, giving frequency and confidence.
//! 3. **Onsets** ([`onset`]) — spectral flux, the only thing that can separate two
//!    repetitions of the same note.
//! 4. **Assembly** — segment at onsets and at voicing changes, take the median pitch of
//!    each segment, then optionally quantise.
//!
//! There is no audio I/O here and no platform code: the input is a slice of samples.
//!
//! | Module | Holds |
//! |---|---|
//! | [`dsp`] | windowing and the FFT the other stages share |
//! | [`pitch`] | YIN, per frame |
//! | [`onset`] | spectral flux |
//! | [`tempo`] | inter-onset intervals into a BPM estimate |
//! | [`frame`] | what one frame measured, and the whole measurement track |
//! | [`options`] | the knobs in, and the shapes out |
//! | [`pipeline`] | the four stages in order — the short file this doc describes |
//! | [`assemble`] | frames into notes: where a note starts, ends, and how loud it is |
//! | [`waveform`] | min/max buckets for the view the user fine-tunes against |

mod assemble;
pub mod dsp;
pub mod frame;
pub mod onset;
pub mod options;
pub mod pipeline;
pub mod pitch;
pub mod tempo;
pub mod waveform;

#[cfg(test)]
mod tests;

pub use frame::{Analysis, Frame};
pub use options::{DetectedNote, TranscribeOptions, Transcription};
pub use pipeline::transcribe;
pub use waveform::peaks;

/// Analysis frame length. About 46 ms at 44.1 kHz.
///
/// Long enough to hold two periods of a low E (82 Hz) with room to spare, which YIN
/// needs, and short enough that a 16th note at 160 bpm still spans several frames.
pub const FRAME_SIZE: usize = 2048;

/// Hop between frames, in seconds.
pub const HOP_SECONDS: f64 = 0.01;

/// Pitch search range: roughly E1 to C7. Below this is rumble, above it is hiss.
pub const MIN_HZ: f64 = 41.0;
pub const MAX_HZ: f64 = 2100.0;

/// How confident YIN must be for a frame to count as pitched.
const MIN_CONFIDENCE: f32 = 0.55;

/// Level, relative to the recording's peak, below which a frame is silence.
///
/// Relative rather than absolute so a quiet take transcribes like a loud one.
const SILENCE_FLOOR: f32 = 0.02;

/// Shortest note kept, in frames. Below this it is a chirp between two notes rather than
/// a note — 60 ms is already faster than anything played deliberately.
const MIN_NOTE_FRAMES: usize = 6;

/// A pitch change this large starts a new note even without an onset. Half a semitone,
/// so ordinary vibrato does not fragment a held note.
const PITCH_BREAK_SEMITONES: f32 = 0.5;
