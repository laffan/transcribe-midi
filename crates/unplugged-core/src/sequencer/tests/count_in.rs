//! Count-in bars: the click sounds, the timeline does not.

use super::*;

#[test]
fn count_in_suppresses_timeline_events_but_not_the_click() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default());
    s.set_metronome(true);
    // One bar of count-in: notes before tick 1920 must not sound.
    s.set_count_in_until(Some(1920));
    s.set_timeline(Timeline::new(vec![
        ev(0, EventKind::NoteOn, 60),     // inside the count-in
        ev(1920, EventKind::NoteOn, 72),  // at the record point
    ]));
    s.play();

    let mut out = Vec::new();
    s.render(96_000, &mut out); // exactly one bar

    assert!(
        !out.iter().any(|e| e.track != METRONOME_TRACK),
        "no timeline event may sound during the count-in"
    );
    assert_eq!(clicks(&out).len(), 4, "but the click must play");

    // Past the record point the timeline resumes.
    out.clear();
    s.render(4_800, &mut out);
    assert!(
        out.iter().any(|e| e.track != METRONOME_TRACK && e.pitch == 72),
        "playback must resume at the record point"
    );
}

#[test]
fn count_in_reports_its_own_state() {
    let mut s = seq();
    s.set_count_in_until(Some(1920));
    s.play();
    assert!(s.in_count_in());

    let mut out = Vec::new();
    s.render(96_000, &mut out); // one bar, landing exactly on the record point
    assert!(!s.in_count_in(), "the count-in ends at the record point");

    s.set_count_in_until(None);
    assert!(!s.in_count_in());
}

#[test]
fn replacing_the_timeline_mid_playback_keeps_the_cursor_consistent() {
    let mut s = seq();
    s.set_timeline(Timeline::new(vec![ev(0, EventKind::NoteOn, 60)]));
    s.play();
    let mut out = Vec::new();
    s.render(4800, &mut out); // now at tick 96

    // Swap in a timeline with an event before and after the current position.
    s.set_timeline(Timeline::new(vec![
        ev(10, EventKind::NoteOn, 50),  // behind the playhead
        ev(200, EventKind::NoteOn, 80), // ahead of it
    ]));

    out.clear();
    s.render(48_000, &mut out);
    let pitches: Vec<u8> = out.iter().map(|e| e.pitch).collect();
    assert_eq!(pitches, vec![80], "only the event ahead of the playhead should fire");
}
