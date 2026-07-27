//! Play, stop, seek, and changing tempo or sample rate mid-flight.

use super::*;

#[test]
fn stopping_mid_note_releases_it() {
    let mut s = seq();
    s.set_timeline(Timeline::new(vec![
        ev(0, EventKind::NoteOn, 60),
        ev(1000, EventKind::NoteOff, 60),
    ]));
    s.play();

    let mut out = Vec::new();
    s.render(512, &mut out);
    assert_eq!(s.sounding_count(), 1);

    s.stop(&mut out);
    assert!(!s.is_playing());
    assert_eq!(s.sounding_count(), 0);
    assert!(
        out.iter().any(|e| e.kind == EventKind::NoteOff && e.pitch == 60),
        "stop must release the held note"
    );
}

#[test]
fn seeking_releases_held_notes_and_repositions_the_cursor() {
    let mut s = seq();
    s.set_timeline(Timeline::new(vec![
        ev(0, EventKind::NoteOn, 60),
        ev(1000, EventKind::NoteOff, 60),
        ev(480, EventKind::NoteOn, 72),
    ]));
    s.play();

    let mut out = Vec::new();
    s.render(512, &mut out);
    assert_eq!(s.sounding_count(), 1);

    s.seek(480, &mut out);
    assert_eq!(s.position_ticks(), 480);
    assert_eq!(s.sounding_count(), 0, "the held note must not survive the jump");

    out.clear();
    s.render(64, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].pitch, 72, "playback resumes from the new position");
}

#[test]
fn tempo_change_preserves_musical_position() {
    let mut s = seq();
    s.play();
    let mut out = Vec::new();
    s.render(4800, &mut out); // 96 ticks at 120 bpm
    assert_eq!(s.position_ticks(), 96);

    s.set_tempo(240.0);
    assert_eq!(s.position_ticks(), 96, "the playhead must not jump on the timeline");
    assert_eq!(s.samples_per_tick(), 25.0, "but ticks now pass twice as fast");
}

#[test]
fn invalid_tempo_and_sample_rate_are_ignored_rather_than_poisoning_the_clock() {
    let mut s = seq();
    let before = s.samples_per_tick();
    s.set_tempo(0.0);
    s.set_tempo(-5.0);
    s.set_tempo(f64::NAN);
    s.set_sample_rate(0.0);
    s.set_ppq(0);
    assert_eq!(s.samples_per_tick(), before);
    assert!(s.samples_per_tick().is_finite());
}

