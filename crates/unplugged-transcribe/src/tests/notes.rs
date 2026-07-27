//! The notes that come out: how many, where they start, and where they do not.

use super::*;

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

