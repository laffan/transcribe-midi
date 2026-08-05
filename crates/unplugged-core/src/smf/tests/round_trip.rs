//! Format 0 storage out and back: every field, and the awkward inputs.

use super::*;

#[test]
fn round_trip_preserves_every_field() {
    // In canonical order — sorted by (start_ticks, pitch). Note that at tick 960
    // pitch 48 precedes pitch 67, and the two are on different channels.
    let original = vec![
        note(60, 0, 480, 100, 0),
        note(64, 480, 240, 80, 0),
        note(48, 960, 480, 1, 15),
        note(67, 960, 1920, 127, 3),
    ];

    let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Piano").unwrap();
    let (parsed, ppq) = smf_bytes_to_notes(&bytes, &p()).unwrap();

    assert_eq!(ppq, Some(DEFAULT_PPQ));
    assert_eq!(parsed, original);
}

#[test]
fn round_trip_survives_an_empty_track() {
    let bytes = notes_to_smf_bytes(&[], DEFAULT_PPQ, "Empty").unwrap();
    let (parsed, ppq) = smf_bytes_to_notes(&bytes, &p()).unwrap();
    assert!(parsed.is_empty());
    assert_eq!(ppq, Some(DEFAULT_PPQ));
}

#[test]
fn overlapping_notes_of_the_same_pitch_stay_separate() {
    // Two of the same pitch overlapping — the naive "one pending note per pitch"
    // implementation collapses these into one note and loses the second.
    let original = vec![note(60, 0, 960, 100, 0), note(60, 480, 960, 90, 0)];
    let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Overlap").unwrap();
    let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
    assert_eq!(parsed.len(), 2, "overlapping same-pitch notes must not merge");
    assert_eq!(parsed, original);
}

#[test]
fn note_on_with_zero_velocity_is_treated_as_note_off() {
    // Hand-built: note-on 60 at tick 0, then note-on 60 vel 0 at tick 480.
    // Almost all real MIDI files in the wild express note-offs this way.
    let events = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(100) },
            },
        },
        TrackEvent {
            delta: u28::new(480),
            kind: TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(0) },
            },
        },
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
    ];
    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(DEFAULT_PPQ))),
        tracks: vec![events],
    };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes).unwrap();

    let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
    assert_eq!(parsed, vec![note(60, 0, 480, 100, 0)]);
}

#[test]
fn adjacent_same_pitch_notes_do_not_lose_their_retrigger() {
    // Note A ends exactly where note B begins. If the off/on ordering at that tick
    // were reversed, B would be cut to nothing on replay.
    let original = vec![note(60, 0, 480, 100, 0), note(60, 480, 480, 100, 0)];
    let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Adjacent").unwrap();
    let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
    assert_eq!(parsed, original);
}

#[test]
fn unterminated_note_is_kept_rather_than_dropped() {
    let events = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(100) },
            },
        },
        TrackEvent { delta: u28::new(960), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
    ];
    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(DEFAULT_PPQ))),
        tracks: vec![events],
    };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes).unwrap();

    let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].duration_ticks, 960);
}

#[test]
fn parsed_notes_come_back_sorted() {
    let original = vec![
        note(72, 1920, 240, 100, 0),
        note(60, 0, 240, 100, 0),
        note(64, 960, 240, 100, 0),
    ];
    let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Sort").unwrap();
    let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
    assert!(parsed.windows(2).all(|w| w[0].order_key() <= w[1].order_key()));
}

#[test]
fn tempo_conversion_matches_the_midi_spec() {
    // 120 bpm is exactly 500000 microseconds per quarter note.
    assert_eq!(bpm_to_micros_per_quarter(120.0).as_int(), 500_000);
    assert_eq!(bpm_to_micros_per_quarter(60.0).as_int(), 1_000_000);
}

#[test]
fn garbage_input_is_an_error_not_a_panic() {
    assert!(smf_bytes_to_notes(b"definitely not a midi file", &p()).is_err());
    assert!(smf_bytes_to_notes(&[], &p()).is_err());
}
