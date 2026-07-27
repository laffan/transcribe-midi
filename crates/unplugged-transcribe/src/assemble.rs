//! Frames into notes.
//!
//! The segmentation decisions are the ones that make or break a transcription, and they
//! are all judgement calls with an audible failure mode on each side — which is why they
//! are together in one file, with the reasoning next to each threshold.

use unplugged_core::{Note, Ticks};

use super::frame::Frame;
use super::options::{DetectedNote, TranscribeOptions};
use super::{dsp, pitch};
use super::{MIN_NOTE_FRAMES, PITCH_BREAK_SEMITONES};

/// Split the frame track into candidate notes.
///
/// Three things end a note: silence or loss of pitch, a detected onset, and a sustained
/// pitch change. The third is needed because a legato slur from C to G has no attack for
/// the flux to see, and without it the pair would come out as one note at whichever
/// pitch happened to be the median.
///
/// The onset frame index is used as the boundary directly, which places the note a frame
/// or two early — the attack is somewhere inside the window that first saw it. In
/// exchange, a note is never clipped at the front, which is far more audible than a
/// little silence before it.
pub(super) fn segment(frames: &[Frame], onsets: &[usize], floor: f32) -> Vec<(usize, usize)> {
    let is_onset = {
        let mut flags = vec![false; frames.len()];
        for &index in onsets {
            if index < flags.len() {
                flags[index] = true;
            }
        }
        flags
    };

    let mut segments = Vec::new();
    let mut open: Option<usize> = None;
    let mut reference = 0.0f32;

    for (index, frame) in frames.iter().enumerate() {
        let voiced = frame.voiced(floor);

        let Some(start) = open else {
            if voiced {
                open = Some(index);
                reference = pitch::hz_to_midi(frame.frequency);
            }
            continue;
        };

        if !voiced {
            segments.push((start, index));
            open = None;
            continue;
        }

        let midi = pitch::hz_to_midi(frame.frequency);
        let moved = (midi - reference).abs() >= PITCH_BREAK_SEMITONES;

        if is_onset[index] || moved {
            segments.push((start, index));
            open = Some(index);
            reference = midi;
        } else {
            // Track slowly, so a note that drifts a little is not eventually judged
            // against a stale reference from its very first frame.
            reference = reference * 0.85 + midi * 0.15;
        }
    }

    if let Some(start) = open {
        segments.push((start, frames.len()));
    }

    segments
}

pub(super) fn build_note(
    frames: &[Frame],
    start: usize,
    end: usize,
    hop: usize,
    tempo_bpm: f64,
    options: &TranscribeOptions,
    floor: f32,
) -> Option<DetectedNote> {
    if end.saturating_sub(start) < MIN_NOTE_FRAMES {
        return None;
    }

    // Skip the first two frames: an attack transient is inharmonic and drags the pitch
    // estimate around. What is left is the steady part, which is what was played.
    let body_start = (start + 2).min(end);
    let voiced: Vec<&Frame> = frames[body_start..end]
        .iter()
        .filter(|frame| frame.voiced(floor))
        .collect();

    if voiced.len() < MIN_NOTE_FRAMES / 2 {
        return None;
    }

    // Median rather than mean: a single octave-error frame would drag a mean half an
    // octave, and the median simply ignores it.
    let midi_values: Vec<f32> = voiced
        .iter()
        .map(|frame| pitch::hz_to_midi(frame.frequency))
        .collect();
    let median_midi = dsp::median(&midi_values);
    let pitch_number = median_midi.round().clamp(0.0, 127.0) as u8;
    let cents_off = (median_midi - median_midi.round()) * 100.0;

    let confidence = voiced.iter().map(|frame| frame.confidence).sum::<f32>() / voiced.len() as f32;

    // Velocity from the loudest frame in the note, which is the attack.
    let peak_level = voiced.iter().map(|frame| frame.level).fold(0.0f32, f32::max);
    let velocity = level_to_velocity(peak_level, floor);

    let seconds_per_frame = hop as f64 / options.sample_rate;
    let start_seconds = start as f64 * seconds_per_frame;
    let duration_seconds = (end - start) as f64 * seconds_per_frame;

    let ticks_per_second = tempo_bpm / 60.0 * options.ppq as f64;
    let mut start_ticks = (start_seconds * ticks_per_second).round().max(0.0) as Ticks;
    let mut duration_ticks = (duration_seconds * ticks_per_second).round().max(1.0) as Ticks;

    if options.quantize_ticks > 0 {
        let grid = options.quantize_ticks as f64;
        start_ticks = ((start_ticks as f64 / grid).round() * grid) as Ticks;
        // Lengths snap too, but never below one grid step — a note rounded to zero
        // length cannot be written to a MIDI file at all.
        duration_ticks =
            (((duration_ticks as f64 / grid).round() * grid) as Ticks).max(options.quantize_ticks);
    }

    Some(DetectedNote {
        note: Note::new(
            pitch_number,
            start_ticks,
            duration_ticks.max(1),
            velocity,
            options.channel,
        )
        .ok()?,
        start_seconds,
        duration_seconds,
        confidence,
        cents_off,
    })
}

/// Map a peak level to a MIDI velocity.
///
/// Logarithmic, because loudness is: a linear map puts almost everything played at a
/// normal level into the top of the range and makes the result sound machine-flat.
fn level_to_velocity(level: f32, floor: f32) -> u8 {
    let reference = floor.max(1e-6);
    if level <= reference {
        return 1;
    }
    // 40 dB of range spread across the velocity scale.
    let db = 20.0 * (level / reference).log10();
    let scaled = (db / 40.0).clamp(0.0, 1.0);
    (1.0 + scaled * 126.0).round().clamp(1.0, 127.0) as u8
}

