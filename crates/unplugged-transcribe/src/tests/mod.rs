//! Tests for the pipeline, against synthetic signals a CI host can generate.
//!
//! Every fixture here builds its own audio, so nothing depends on a recorded file or on
//! any platform being able to play one.

mod analysis;
mod limits;
mod measurement;
mod notes;
mod waveform;

use super::*;

use std::f32::consts::PI;

const SAMPLE_RATE: f64 = 44100.0;
const PPQ: u16 = 480;

fn options() -> TranscribeOptions {
    TranscribeOptions::new(SAMPLE_RATE, PPQ)
}

/// A note with harmonics and a percussive envelope — close enough to a plucked or
/// struck instrument for the pipeline to behave as it would on a real take.
fn note_signal(midi: u8, seconds: f64, amplitude: f32) -> Vec<f32> {
    let hz = pitch::midi_to_hz(midi as f32);
    let samples = (SAMPLE_RATE * seconds) as usize;
    (0..samples)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let envelope = (-3.0 * t / seconds as f32).exp();
            amplitude
                * envelope
                * ((2.0 * PI * hz * t).sin()
                    + 0.5 * (2.0 * PI * hz * 2.0 * t).sin()
                    + 0.25 * (2.0 * PI * hz * 3.0 * t).sin())
                / 1.75
        })
        .collect()
}

fn melody(pitches: &[u8], seconds_each: f64) -> Vec<f32> {
    let mut signal = Vec::new();
    for &midi in pitches {
        signal.extend(note_signal(midi, seconds_each, 0.7));
    }
    signal
}

