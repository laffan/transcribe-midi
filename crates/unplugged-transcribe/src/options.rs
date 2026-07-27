//! The knobs a caller turns, and the shapes the answer comes back in.

use serde::{Deserialize, Serialize};

use unplugged_core::Note;

use super::frame::Analysis;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TranscribeOptions {
    pub sample_rate: f64,
    pub ppq: u16,
    /// Tempo to place notes against. `None` estimates it from the recording.
    pub tempo_bpm: Option<f64>,
    /// Grid to snap to, in ticks. Zero leaves the performance where it was played.
    pub quantize_ticks: u32,
    /// MIDI channel for the produced notes.
    pub channel: u8,
}

impl TranscribeOptions {
    pub fn new(sample_rate: f64, ppq: u16) -> Self {
        TranscribeOptions {
            sample_rate,
            ppq,
            tempo_bpm: None,
            quantize_ticks: 0,
            channel: 0,
        }
    }
}

/// One detected note, with the evidence behind it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectedNote {
    pub note: Note,
    /// Where it was actually played, before quantisation.
    pub start_seconds: f64,
    pub duration_seconds: f64,
    /// Mean YIN confidence across the note, 0–1.
    pub confidence: f32,
    /// How far the measured pitch sat from equal temperament, in cents. Large values
    /// mean an out-of-tune source, not a detection failure.
    pub cents_off: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transcription {
    pub notes: Vec<DetectedNote>,
    /// The tempo used, whether given or estimated.
    pub tempo_bpm: f64,
    /// True when the tempo was estimated rather than supplied.
    pub tempo_estimated: bool,
    /// Confidence in the estimate; zero when it was supplied.
    pub tempo_confidence: f32,
    /// Length of the analysed audio.
    pub duration_seconds: f64,
    /// Fraction of the recording that held a detectable pitch. A low value usually means
    /// the microphone heard the room rather than the instrument.
    pub pitched_fraction: f32,
    /// The per-frame evidence behind the notes.
    pub analysis: Analysis,
}

impl Transcription {
    pub fn notes(&self) -> Vec<Note> {
        self.notes.iter().map(|detected| detected.note).collect()
    }
}
