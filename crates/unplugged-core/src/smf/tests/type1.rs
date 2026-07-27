//! Format 1 export: the conductor track, and what each chunk carries.

use super::*;

#[test]
fn type1_export_has_a_conductor_track_plus_one_chunk_per_track() {
    let proj = project(
        vec![
            track("Piano", vec![note(60, 0, 480, 100, 0)]),
            track("Bass", vec![note(36, 0, 960, 90, 1)]),
        ],
        120.0,
        TimeSignature::default(),
    );

    let bytes = project_to_smf_type1(&proj).unwrap();
    let smf = Smf::parse(&bytes).unwrap();

    assert_eq!(smf.header.format, Format::Parallel, "must be SMF type 1");
    assert_eq!(smf.tracks.len(), 3, "conductor plus two instrument tracks");
}

#[test]
fn the_conductor_track_carries_tempo_and_time_signature() {
    let proj = project(vec![track("T", vec![note(60, 0, 480, 100, 0)])], 96.0,
                    TimeSignature::new(6, 8).unwrap());
    let bytes = project_to_smf_type1(&proj).unwrap();
    let smf = Smf::parse(&bytes).unwrap();

    let mut tempo = None;
    let mut sig = None;
    for event in &smf.tracks[0] {
        match event.kind {
            TrackEventKind::Meta(MetaMessage::Tempo(micros)) => tempo = Some(micros.as_int()),
            TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, ..)) => sig = Some((n, d)),
            _ => {}
        }
    }

    assert_eq!(tempo, Some(bpm_to_micros_per_quarter(96.0).as_int()));
    // 6/8 — the denominator is stored as a power of two, so 8 becomes 3.
    assert_eq!(sig, Some((6, 3)));
}

#[test]
fn a_type1_export_reimports_with_the_same_notes_tempo_and_signature() {
    let proj = project(
        vec![
            track("Piano", vec![note(60, 0, 480, 100, 0), note(64, 480, 240, 80, 0)]),
            track("Bass", vec![note(36, 0, 1920, 90, 1)]),
        ],
        140.0,
        TimeSignature::new(3, 4).unwrap(),
    );

    let bytes = project_to_smf_type1(&proj).unwrap();
    let imported = parse_smf_for_import(&bytes, &p()).unwrap();

    assert_eq!(imported.ppq, Some(DEFAULT_PPQ));
    assert_eq!(imported.time_signature, Some(TimeSignature::new(3, 4).unwrap()));
    assert!((imported.tempo_bpm.unwrap() - 140.0).abs() < 0.01);

    assert_eq!(imported.tracks.len(), 2, "the empty conductor track must not become a track");
    assert_eq!(imported.tracks[0].name.as_deref(), Some("Piano"));
    assert_eq!(imported.tracks[0].notes, vec![note(60, 0, 480, 100, 0), note(64, 480, 240, 80, 0)]);
    assert_eq!(imported.tracks[1].name.as_deref(), Some("Bass"));
    assert_eq!(imported.tracks[1].notes, vec![note(36, 0, 1920, 90, 1)]);
}

#[test]
fn exporting_a_project_with_no_notes_still_produces_a_valid_file() {
    let proj = project(vec![track("Empty", vec![])], 120.0, TimeSignature::default());
    let bytes = project_to_smf_type1(&proj).unwrap();

    let smf = Smf::parse(&bytes).unwrap();
    assert_eq!(smf.tracks.len(), 2);
    assert!(parse_smf_for_import(&bytes, &p()).unwrap().tracks.is_empty());
}

#[test]
fn a_standalone_track_export_carries_its_own_tempo() {
    // Drag-out and single-track export must open at the right tempo rather than
    // defaulting to 120.
    let t = track("Solo", vec![note(60, 0, 480, 100, 0)]);
    let bytes = track_to_standalone_smf(&t, 88.0, TimeSignature::new(5, 4).unwrap(), DEFAULT_PPQ).unwrap();

    let imported = parse_smf_for_import(&bytes, &p()).unwrap();
    assert!((imported.tempo_bpm.unwrap() - 88.0).abs() < 0.01);
    assert_eq!(imported.time_signature, Some(TimeSignature::new(5, 4).unwrap()));
    assert_eq!(imported.tracks.len(), 1);
    assert_eq!(imported.tracks[0].notes.len(), 1);
    assert_eq!(imported.tracks[0].name.as_deref(), Some("Solo"));
}

#[test]
fn channels_survive_the_type1_round_trip() {
    let proj = project(
        vec![track("Multi", vec![
            note(60, 0, 240, 100, 0),
            note(62, 0, 240, 100, 9),
            note(64, 0, 240, 100, 15),
        ])],
        120.0,
        TimeSignature::default(),
    );
    let bytes = project_to_smf_type1(&proj).unwrap();
    let imported = parse_smf_for_import(&bytes, &p()).unwrap();

    let channels: Vec<u8> = imported.tracks[0].notes.iter().map(|n| n.channel).collect();
    assert_eq!(channels, vec![0, 9, 15]);
}
