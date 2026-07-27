//! Wrapping at the loop end, including notes held across it.

use super::*;

#[test]
fn playback_wraps_at_the_loop_end_within_a_single_buffer() {
    let mut s = seq();
    // Loop ticks 0..10 (samples 0..500). Note at tick 2 (sample 100).
    s.set_timeline(Timeline::new(vec![ev(2, EventKind::NoteOn, 60)]));
    s.set_loop_region(Some((0, 10)));
    s.play();

    let mut out = Vec::new();
    s.render(1200, &mut out); // spans two full loops plus part of a third

    let ons: Vec<u32> = out
        .iter()
        .filter(|e| e.kind == EventKind::NoteOn)
        .map(|e| e.frame_offset)
        .collect();
    assert_eq!(ons, vec![100, 600, 1100], "the note must fire once per loop pass");
}

#[test]
fn a_note_held_across_the_loop_end_is_released_rather_than_hanging() {
    let mut s = seq();
    // Note on at tick 2, off at tick 40 — but the loop ends at tick 10, so the
    // off event is never reached. Without an explicit release this note hangs.
    s.set_timeline(Timeline::new(vec![
        ev(2, EventKind::NoteOn, 60),
        ev(40, EventKind::NoteOff, 60),
    ]));
    s.set_loop_region(Some((0, 10)));
    s.play();

    let mut out = Vec::new();
    s.render(600, &mut out);

    let offs: Vec<&RenderedEvent> = out.iter().filter(|e| e.kind == EventKind::NoteOff).collect();
    assert_eq!(offs.len(), 1, "exactly one release at the wrap");
    assert_eq!(offs[0].pitch, 60);
    assert_eq!(offs[0].frame_offset, 500, "released at the loop boundary");
}

#[test]
fn a_degenerate_loop_region_is_rejected_not_obeyed() {
    let mut s = seq();
    s.set_loop_region(Some((100, 100)));
    assert_eq!(s.loop_region(), None, "a zero-length loop would spin forever");
    s.set_loop_region(Some((200, 100)));
    assert_eq!(s.loop_region(), None, "an inverted loop is meaningless");
}

#[test]
fn render_terminates_even_with_a_loop_shorter_than_the_buffer() {
    let mut s = seq();
    s.set_timeline(Timeline::new(vec![ev(0, EventKind::NoteOn, 60)]));
    s.set_loop_region(Some((0, 1))); // 50 samples, far shorter than the buffer
    s.play();

    let mut out = Vec::new();
    s.render(4096, &mut out); // must not hang
    assert!(out.len() > 1, "the loop should have wrapped many times");
}

