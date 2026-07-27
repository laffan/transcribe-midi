//! Tick-to-sample conversion and sample-accurate placement within a buffer.

use super::*;

#[test]
fn samples_per_tick_matches_the_arithmetic() {
    // 120 bpm, 480 ppq, 48 kHz: one beat is 0.5 s = 24000 samples,
    // so one tick is 24000 / 480 = 50 samples.
    assert_eq!(seq().samples_per_tick(), 50.0);
}

#[test]
fn nothing_is_emitted_while_stopped() {
    let mut s = seq();
    s.set_timeline(Timeline::new(vec![ev(0, EventKind::NoteOn, 60)]));
    let mut out = Vec::new();
    s.render(512, &mut out);
    assert!(out.is_empty(), "a stopped sequencer must emit nothing");
    assert_eq!(s.position_ticks(), 0, "and must not advance");
}

#[test]
fn events_land_on_their_exact_sample_offset() {
    let mut s = seq();
    // 50 samples per tick: tick 2 -> 100, tick 5 -> 250.
    s.set_timeline(Timeline::new(vec![
        ev(2, EventKind::NoteOn, 60),
        ev(5, EventKind::NoteOn, 64),
    ]));
    s.play();

    let mut out = Vec::new();
    s.render(512, &mut out);

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].frame_offset, 100);
    assert_eq!(out[0].pitch, 60);
    assert_eq!(out[1].frame_offset, 250);
    assert_eq!(out[1].pitch, 64);
}

#[test]
fn an_event_beyond_the_buffer_waits_for_the_next_one() {
    let mut s = seq();
    s.set_timeline(Timeline::new(vec![ev(20, EventKind::NoteOn, 60)])); // sample 1000
    s.play();

    let mut out = Vec::new();
    s.render(512, &mut out);
    assert!(out.is_empty(), "sample 1000 is outside the first 512-frame buffer");

    s.render(512, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].frame_offset, 1000 - 512, "offset is relative to this buffer");
}

#[test]
fn an_event_exactly_on_a_buffer_boundary_is_not_lost_or_doubled() {
    let mut s = seq();
    // Tick 10 is sample 500. With 500-frame buffers it sits exactly on the seam.
    s.set_timeline(Timeline::new(vec![ev(10, EventKind::NoteOn, 60)]));
    s.play();

    let mut out = Vec::new();
    s.render(500, &mut out);
    assert!(out.is_empty(), "sample 500 is exclusive of the [0,500) window");

    s.render(500, &mut out);
    assert_eq!(out.len(), 1, "and must appear exactly once in the next buffer");
    assert_eq!(out[0].frame_offset, 0);
}

#[test]
fn position_tracks_elapsed_samples() {
    let mut s = seq();
    s.play();
    let mut out = Vec::new();
    for _ in 0..10 {
        s.render(480, &mut out); // 4800 samples total = 96 ticks
    }
    assert_eq!(s.position_ticks(), 96);
}

