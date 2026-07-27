//! Tempo, quantization and velocity — what is measured rather than segmented.

use super::*;

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

