//! Per-frame measurements, and the whole measurement track.
//!
//! These are returned with the transcription rather than discarded, because they *are*
//! the fine-tuning editor: the pitch track is the line the notes sit on, confidence says
//! which notes are worth a second look, and level draws the envelope. Throwing them away
//! is what used to make changing the grid mean recording again.

use serde::{Deserialize, Serialize};

use super::MIN_CONFIDENCE;

/// One frame's worth of measurements.
///
/// Public, and returned with the transcription, because these frames *are* the editor.
/// The pitch track is the line the notes sit on, confidence says which notes are worth a
/// second look, and level draws the envelope. Phase 7 computed all of this and threw it
/// away, which is why changing the grid meant recording again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// Detected fundamental in Hz. Zero when the frame is unpitched.
    pub frequency: f32,
    /// Fractional MIDI note number, so the pitch line is drawn where it was measured
    /// rather than where it was rounded to. Zero when unpitched.
    pub midi: f32,
    pub confidence: f32,
    pub level: f32,
}

impl Frame {
    pub(super) fn voiced(&self, floor: f32) -> bool {
        self.frequency > 0.0 && self.confidence >= MIN_CONFIDENCE && self.level > floor
    }
}

/// Everything the transcription editor needs to draw over the waveform.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub frames: Vec<Frame>,
    /// Frame indices where an attack was detected. Note boundaries snap to these.
    pub onsets: Vec<usize>,
    /// Seconds between frames.
    pub hop_seconds: f64,
    /// The level below which a frame counts as silence, in the same units as
    /// `Frame::level`. Drawn as the noise floor.
    pub silence_floor: f32,
}

