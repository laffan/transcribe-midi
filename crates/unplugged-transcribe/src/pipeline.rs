//! The four stages, in order, and nothing else.
//!
//! Framing, pitch, onsets, assembly. Kept short on purpose: each stage's real work lives
//! in its own module, so this file reads as the description of the pipeline that the crate
//! doc promises.


use super::assemble::{build_note, segment};
use super::frame::{Analysis, Frame};
use super::options::{TranscribeOptions, Transcription};
use super::{dsp, onset, pitch, tempo};
use super::{FRAME_SIZE, HOP_SECONDS, MAX_HZ, MIN_HZ, SILENCE_FLOOR};

/// Transcribe mono samples into notes.
pub fn transcribe(samples: &[f32], options: TranscribeOptions) -> Transcription {
    let hop = ((options.sample_rate * HOP_SECONDS).round() as usize).max(1);
    let duration_seconds = samples.len() as f64 / options.sample_rate.max(1.0);

    if samples.len() < FRAME_SIZE || options.sample_rate <= 0.0 {
        return Transcription {
            tempo_bpm: options.tempo_bpm.unwrap_or(unplugged_core::DEFAULT_TEMPO),
            duration_seconds,
            ..Default::default()
        };
    }

    // -- frame ------------------------------------------------------------
    let windows: Vec<&[f32]> = (0..)
        .map(|index| index * hop)
        .take_while(|&start| start + FRAME_SIZE <= samples.len())
        .map(|start| &samples[start..start + FRAME_SIZE])
        .collect();

    let frames: Vec<Frame> = windows
        .iter()
        .map(|window| {
            let estimate = pitch::yin(window, options.sample_rate, MIN_HZ, MAX_HZ);
            Frame {
                frequency: estimate.frequency,
                midi: pitch::hz_to_midi(estimate.frequency),
                confidence: estimate.confidence,
                level: dsp::rms(window),
            }
        })
        .collect();

    let peak = frames.iter().map(|f| f.level).fold(0.0f32, f32::max);
    let floor = peak * SILENCE_FLOOR;

    // -- onsets -----------------------------------------------------------
    let window = dsp::hann(FRAME_SIZE);
    let track = onset::detect(&windows, &window, onset::OnsetParams::default());

    // -- tempo ------------------------------------------------------------
    let frames_per_second = options.sample_rate / hop as f64;
    let (tempo_bpm, tempo_estimated, tempo_confidence) = match options.tempo_bpm {
        Some(given) => (given, false, 0.0),
        None => match tempo::estimate(&track.flux, frames_per_second) {
            Some(estimate) => (tempo::tidy(estimate.bpm), true, estimate.confidence),
            None => (unplugged_core::DEFAULT_TEMPO, true, 0.0),
        },
    };

    // -- assemble ---------------------------------------------------------
    let segments = segment(&frames, &track.onsets, floor);
    let notes = segments
        .iter()
        .filter_map(|&(start, end)| build_note(&frames, start, end, hop, tempo_bpm, &options, floor))
        .collect();

    let pitched = frames.iter().filter(|f| f.voiced(floor)).count();

    Transcription {
        notes,
        tempo_bpm,
        tempo_estimated,
        tempo_confidence,
        duration_seconds,
        pitched_fraction: pitched as f32 / frames.len().max(1) as f32,
        analysis: Analysis {
            frames,
            onsets: track.onsets,
            hop_seconds: hop as f64 / options.sample_rate,
            silence_floor: floor,
        },
    }
}
