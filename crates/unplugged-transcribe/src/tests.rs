//! Tests for the transcription pipeline.
//!
//! Split from `lib.rs` under the 700-line rule. They are the spec for the pipeline: every
//! signal here is synthesised, so a Linux CI host proves as much as a Mac does.

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

#[test]
fn transcribes_a_simple_melody() {
    let played = [60u8, 62, 64, 65, 67];
    let result = transcribe(&melody(&played, 0.5), options());

    assert_eq!(
        result.notes().iter().map(|n| n.pitch).collect::<Vec<_>>(),
        played,
        "detected {:?}",
        result
            .notes
            .iter()
            .map(|n| (n.note.pitch, n.start_seconds))
            .collect::<Vec<_>>()
    );

    for (index, detected) in result.notes.iter().enumerate() {
        let expected = index as f64 * 0.5;
        assert!(
            (detected.start_seconds - expected).abs() < 0.07,
            "note {index} starts at {:.3}s, expected {expected:.3}s",
            detected.start_seconds
        );
        assert!(detected.confidence > 0.7);
        assert!(detected.cents_off.abs() < 30.0, "{}", detected.cents_off);
    }
}

#[test]
fn separates_repeated_notes_at_the_same_pitch() {
    // Pitch alone cannot see these boundaries; the flux onsets are what find them.
    let result = transcribe(&melody(&[60, 60, 60, 60], 0.45), options());
    assert_eq!(result.notes.len(), 4, "{:?}", result.notes());
}

#[test]
fn silence_produces_nothing() {
    let result = transcribe(&vec![0.0f32; 44100 * 2], options());
    assert!(result.notes.is_empty());
    assert_eq!(result.pitched_fraction, 0.0);
    assert!((result.duration_seconds - 2.0).abs() < 0.01);
}

#[test]
fn a_buffer_shorter_than_one_frame_is_handled() {
    let result = transcribe(&vec![0.1f32; 100], options());
    assert!(result.notes.is_empty());
    assert!(result.tempo_bpm > 0.0, "a usable tempo is still reported");
}

#[test]
fn leading_silence_does_not_become_a_note() {
    let mut signal = vec![0.0f32; 22050];
    signal.extend(note_signal(64, 0.6, 0.7));

    let result = transcribe(&signal, options());
    assert_eq!(result.notes.len(), 1, "{:?}", result.notes());
    assert_eq!(result.notes[0].note.pitch, 64);
    assert!(
        (result.notes[0].start_seconds - 0.5).abs() < 0.07,
        "started at {:.3}s",
        result.notes[0].start_seconds
    );
}

#[test]
fn a_legato_slur_still_becomes_two_notes() {
    // No attack between them, so there is no onset to find — the pitch break is the
    // only evidence, which is exactly why segmentation looks for it.
    let mut signal = Vec::new();
    let hz_a = pitch::midi_to_hz(60.0);
    let hz_b = pitch::midi_to_hz(67.0);
    let half = (SAMPLE_RATE * 0.6) as usize;
    let mut phase = 0.0f32;

    for i in 0..half * 2 {
        let hz = if i < half { hz_a } else { hz_b };
        phase += 2.0 * PI * hz / SAMPLE_RATE as f32;
        signal.push(0.6 * phase.sin());
    }

    let result = transcribe(&signal, options());
    let pitches: Vec<u8> = result.notes().iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 67], "{:?}", result.notes());
}

#[test]
fn vibrato_does_not_fragment_a_held_note() {
    let hz = pitch::midi_to_hz(69.0);
    let samples = (SAMPLE_RATE * 1.2) as usize;
    let mut phase = 0.0f32;
    let signal: Vec<f32> = (0..samples)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            // ±25 cents at 5 Hz — an ordinary singing or string vibrato.
            let bend = 1.0 + 0.0145 * (2.0 * PI * 5.0 * t).sin();
            phase += 2.0 * PI * hz * bend / SAMPLE_RATE as f32;
            0.6 * phase.sin()
        })
        .collect();

    let result = transcribe(&signal, options());
    assert_eq!(result.notes.len(), 1, "{:?}", result.notes());
    assert_eq!(result.notes[0].note.pitch, 69);
}

#[test]
fn a_given_tempo_is_used_verbatim() {
    let mut settings = options();
    settings.tempo_bpm = Some(96.0);

    let result = transcribe(&melody(&[60, 62], 0.5), settings);
    assert_eq!(result.tempo_bpm, 96.0);
    assert!(!result.tempo_estimated);

    // At 96 bpm a quarter note is 625 ms, so 500 ms is 384 ticks.
    let second = result.notes[1].note.start_ticks as i64;
    assert!((second - 384).abs() <= 16, "second note at {second} ticks");
}

#[test]
fn quantization_snaps_to_the_grid() {
    let mut settings = options();
    settings.tempo_bpm = Some(120.0);
    settings.quantize_ticks = 240; // eighth notes

    // At 120 bpm, 0.45 s is 432 ticks — deliberately off the eighth-note grid.
    let result = transcribe(&melody(&[60, 62, 64], 0.45), settings);
    assert!(!result.notes.is_empty());
    for detected in &result.notes {
        assert_eq!(
            detected.note.start_ticks % 240,
            0,
            "note at {} is off the grid",
            detected.note.start_ticks
        );
        assert!(detected.note.duration_ticks >= 240);
    }
}

#[test]
fn velocity_follows_loudness() {
    let mut signal = note_signal(60, 0.5, 0.9);
    signal.extend(note_signal(62, 0.5, 0.15));

    let result = transcribe(&signal, options());
    assert_eq!(result.notes.len(), 2, "{:?}", result.notes());
    assert!(
        result.notes[0].note.velocity > result.notes[1].note.velocity + 10,
        "{} vs {}",
        result.notes[0].note.velocity,
        result.notes[1].note.velocity
    );
    assert!(result.notes.iter().all(|n| n.note.velocity >= 1));
}

#[test]
fn the_notes_come_back_valid_and_ordered() {
    let result = transcribe(&melody(&[55, 60, 64, 67, 72], 0.4), options());
    let notes = result.notes();

    assert!(!notes.is_empty());
    for note in &notes {
        note.validate()
            .expect("every produced note must be representable in SMF");
    }
    for pair in notes.windows(2) {
        assert!(
            pair[0].order_key() <= pair[1].order_key(),
            "notes must come back sorted"
        );
    }
}

#[test]
fn the_analysis_comes_back_with_the_notes() {
    let result = transcribe(&melody(&[60, 62, 64], 0.5), options());
    let analysis = &result.analysis;

    assert!(!analysis.frames.is_empty());
    assert!((analysis.hop_seconds - HOP_SECONDS).abs() < 1e-9);
    assert!(analysis.silence_floor > 0.0);

    // Two boundaries for three notes — the first note's attack is at sample zero and
    // flux has nothing to change from, which segmentation handles by voicing.
    assert_eq!(analysis.onsets.len(), 2, "{:?}", analysis.onsets);

    // The pitch line has to be drawable: a voiced frame carries a fractional MIDI
    // value near the note it belongs to.
    let voiced: Vec<&Frame> = analysis
        .frames
        .iter()
        .filter(|f| f.voiced(analysis.silence_floor, MIN_CONFIDENCE))
        .collect();
    assert!(voiced.len() > analysis.frames.len() / 4);
    for frame in voiced {
        assert!(
            (59.0..=65.0).contains(&frame.midi),
            "stray pitch frame at {}",
            frame.midi
        );
    }
}

#[test]
fn frames_line_up_with_the_notes_they_produced() {
    let result = transcribe(&melody(&[60, 67], 0.6), options());
    let analysis = &result.analysis;

    for detected in &result.notes {
        // The middle of each note should be a voiced frame at that note's pitch.
        let middle = detected.start_seconds + detected.duration_seconds / 2.0;
        let index = (middle / analysis.hop_seconds) as usize;
        let frame = analysis.frames.get(index).expect("frame inside the note");

        assert!(
            (frame.midi - detected.note.pitch as f32).abs() < 1.0,
            "frame {index} reads {:.2}, note is {}",
            frame.midi,
            detected.note.pitch
        );
    }
}

#[test]
fn peaks_keep_the_transient() {
    // A single spike in an otherwise quiet buffer. Averaging would bury it; min/max
    // must not, because that spike is the attack a user is looking for.
    let mut samples = vec![0.01f32; 10_000];
    samples[5_000] = 1.0;
    samples[5_001] = -1.0;

    let buckets = peaks(&samples, 10_000.0, 0.0, 1.0, 100);
    assert_eq!(buckets.len(), 100);

    let (min, max) = buckets[50];
    assert!((max - 1.0).abs() < 1e-6, "the peak survives: {max}");
    assert!((min + 1.0).abs() < 1e-6, "and so does the trough: {min}");

    // Its neighbours stay quiet.
    assert!(buckets[49].1 < 0.02 && buckets[51].1 < 0.02);
}

#[test]
fn peaks_respect_the_requested_window() {
    let samples: Vec<f32> = (0..1000).map(|i| if i < 500 { 0.5 } else { -0.5 }).collect();

    let first = peaks(&samples, 1000.0, 0.0, 0.5, 10);
    assert!(first.iter().all(|&(min, max)| min == 0.5 && max == 0.5));

    let second = peaks(&samples, 1000.0, 0.5, 1.0, 10);
    assert!(second.iter().all(|&(min, max)| min == -0.5 && max == -0.5));
}

#[test]
fn peaks_survive_degenerate_requests() {
    let samples = vec![0.5f32; 100];
    assert!(peaks(&[], 1000.0, 0.0, 1.0, 10).is_empty());
    assert!(peaks(&samples, 1000.0, 0.0, 1.0, 0).is_empty());
    assert!(peaks(&samples, 0.0, 0.0, 1.0, 10).is_empty());
    assert!(peaks(&samples, 1000.0, 1.0, 0.0, 10).is_empty(), "reversed range");
    assert!(peaks(&samples, 1000.0, 5.0, 6.0, 10).is_empty(), "past the end");

    // More buckets than samples: a zoomed-in view is entitled to ask, and every
    // bucket must still carry a value rather than the fold's sentinel.
    let dense = peaks(&samples, 1000.0, 0.0, 0.1, 500);
    assert_eq!(dense.len(), 500);
    assert!(dense.iter().all(|&(min, max)| min == 0.5 && max == 0.5));
}

#[test]
fn a_chord_is_not_pretended_to_be_understood() {
    // Three simultaneous pitches. The contract is that this yields a monophonic line
    // — never three notes at once — because the UI promises one note at a time and
    // silently returning a wrong chord would be worse than returning less.
    let samples = (SAMPLE_RATE * 1.0) as usize;
    let signal: Vec<f32> = (0..samples)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            0.3 * ((2.0 * PI * 261.6 * t).sin()
                + (2.0 * PI * 329.6 * t).sin()
                + (2.0 * PI * 392.0 * t).sin())
        })
        .collect();

    let result = transcribe(&signal, options());
    let simultaneous = result
        .notes()
        .windows(2)
        .filter(|pair| pair[0].start_ticks == pair[1].start_ticks)
        .count();
    assert_eq!(simultaneous, 0, "no two notes may start together");
}

// -- tuning ------------------------------------------------------------

#[test]
fn the_default_tuning_reproduces_the_onset_parameters_it_replaced() {
    // Every transcription made before the dials existed used these numbers. A
    // default that did not land exactly on them would silently change what a take
    // transcribes to.
    let params = TranscribeTuning::default().onset_params();
    let previous = onset::OnsetParams::default();
    assert_eq!(params.delta, previous.delta);
    assert_eq!(params.ratio, previous.ratio);
    assert_eq!(params.min_flux, previous.min_flux);
    assert_eq!(params.min_gap_frames, previous.min_gap_frames);
}

#[test]
fn split_sensitivity_moves_every_onset_threshold_the_same_way() {
    let shy = TranscribeTuning { split_sensitivity: 0.0, ..Default::default() }.onset_params();
    let eager = TranscribeTuning { split_sensitivity: 1.0, ..Default::default() }.onset_params();

    assert!(shy.delta > eager.delta);
    assert!(shy.ratio > eager.ratio);
    assert!(shy.min_flux > eager.min_flux);
    // A ratio at or below one accepts a steady tone's own wobble as an attack, which
    // is the failure the ratio test exists to prevent.
    assert!(eager.ratio > 1.0);
    assert!(eager.delta > 0.0 && eager.min_flux > 0.0);
}

#[test]
fn a_longer_minimum_note_drops_the_shorter_ones() {
    let signal = melody(&[60, 62, 64, 65], 0.2);
    let few = transcribe(
        &signal,
        TranscribeOptions {
            tuning: TranscribeTuning { min_note_ms: 400.0, ..Default::default() },
            ..options()
        },
    );
    let many = transcribe(&signal, options());
    assert!(
        few.notes.len() < many.notes.len(),
        "a 400 ms floor cannot keep 200 ms notes ({} vs {})",
        few.notes.len(),
        many.notes.len()
    );
}

#[test]
fn demanding_more_confidence_never_finds_more_pitch() {
    // The dial is for a breathy or noisy source that comes back empty, so what has
    // to hold is the direction: turning it up cannot invent pitch that a lower
    // setting missed. Tested against a signal with noise in it, because a synthetic
    // sine satisfies any threshold and would prove nothing.
    let mut signal = melody(&[60, 62, 64], 0.3);
    let mut seed = 1u32;
    for sample in &mut signal {
        // A cheap deterministic LCG: no dependency, same noise every run.
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *sample += (seed >> 16) as f32 / 32_768.0 - 1.0;
    }

    let lenient = transcribe(
        &signal,
        TranscribeOptions {
            tuning: TranscribeTuning { min_confidence: 0.2, ..Default::default() },
            ..options()
        },
    );
    let strict = transcribe(
        &signal,
        TranscribeOptions {
            tuning: TranscribeTuning { min_confidence: 0.95, ..Default::default() },
            ..options()
        },
    );

    assert!(
        strict.pitched_fraction <= lenient.pitched_fraction,
        "{} vs {}",
        strict.pitched_fraction,
        lenient.pitched_fraction
    );
}

#[test]
fn nonsense_dials_are_clamped_rather_than_obeyed() {
    let clamped = TranscribeTuning {
        split_sensitivity: 9.0,
        min_note_ms: 0.0,
        pitch_tolerance_semitones: -4.0,
        noise_floor: 100.0,
        min_confidence: f32::NAN,
    }
    .clamped();

    assert_eq!(clamped.split_sensitivity, 1.0);
    assert!(clamped.min_note_ms >= 10.0);
    assert!(clamped.pitch_tolerance_semitones >= 0.1);
    assert!(clamped.noise_floor <= 0.5);
    assert!(clamped.min_confidence.is_finite(), "a NaN must not reach the pipeline");
}

#[test]
fn the_shortest_note_in_frames_follows_the_hop() {
    let tuning = TranscribeTuning { min_note_ms: 100.0, ..Default::default() };
    assert_eq!(tuning.min_note_frames(0.01), 10);
    assert_eq!(tuning.min_note_frames(0.005), 20);
    assert!(tuning.min_note_frames(0.0) >= 1, "never zero, whatever it is asked");
}

// -- progress ----------------------------------------------------------

#[test]
fn progress_runs_from_zero_to_one_without_going_backwards() {
    let mut seen: Vec<f32> = Vec::new();
    transcribe_reporting(&melody(&[60, 64], 0.3), options(), &mut |at| seen.push(at));

    assert!(seen.len() > 2, "a bar needs more than two frames to move");
    assert_eq!(seen.last().copied(), Some(1.0), "it must finish full");
    assert!(seen.windows(2).all(|pair| pair[1] >= pair[0]), "{seen:?}");
    assert!(seen.iter().all(|&at| (0.0..=1.0).contains(&at)));
}

#[test]
fn a_take_too_short_to_analyse_still_completes_the_bar() {
    // Otherwise the overlay sits at zero forever on a take of nothing.
    let mut seen: Vec<f32> = Vec::new();
    transcribe_reporting(&[0.0; 16], options(), &mut |at| seen.push(at));
    assert_eq!(seen, vec![1.0]);
}

#[test]
fn reporting_and_not_reporting_produce_the_same_transcription() {
    let signal = melody(&[60, 62, 64], 0.25);
    let quiet = transcribe(&signal, options());
    let reported = transcribe_reporting(&signal, options(), &mut |_| {});
    assert_eq!(quiet.notes, reported.notes);
}

// -- one voice ---------------------------------------------------------

#[test]
fn a_quantised_transcription_never_overlaps_itself() {
    // Snapping rounds a start back and a length up to a whole grid step, which is
    // exactly how two adjacent notes end up on top of each other.
    let result = transcribe(
        &melody(&[60, 62, 64, 65, 67], 0.18),
        TranscribeOptions { quantize_ticks: u32::from(PPQ) / 4, ..options() },
    );
    assert!(
        unplugged_core::monophony::is_monophonic(&result.notes()),
        "{:?}",
        result.notes()
    );
}
