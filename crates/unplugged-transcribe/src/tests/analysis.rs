//! The frame track handed back with the notes, and that the two agree.

use super::*;

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
        .filter(|f| f.voiced(analysis.silence_floor))
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

