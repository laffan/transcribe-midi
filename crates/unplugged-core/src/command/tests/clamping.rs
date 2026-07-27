//! Edits past the edges of pitch, time and velocity clamp rather than fail.

use super::*;

#[test]
fn moving_past_the_edges_clamps_rather_than_dropping_notes() {
    let mut s = session(vec![note(2, 100, 480, 100), note(125, 100, 480, 100)]);

    let tx = edits::move_notes(&s, 0, &[0, 1], -10_000, 10).unwrap();
    s.apply(tx).unwrap();

    let notes = &s.tracks()[0].notes;
    assert_eq!(notes.len(), 2, "no note may be lost at the boundary");
    assert!(notes.iter().all(|n| n.start_ticks == 0), "time clamps at zero");
    assert_eq!(notes.iter().map(|n| n.pitch).max(), Some(127), "pitch clamps at 127");
}

#[test]
fn resizing_below_zero_leaves_a_representable_note() {
    let mut s = session(vec![note(60, 0, 100, 100)]);
    let tx = edits::resize_notes(&s, 0, &[0], -500).unwrap();
    s.apply(tx).unwrap();
    assert_eq!(s.tracks()[0].notes[0].duration_ticks, 1, "zero-length is unrepresentable in SMF");
}

#[test]
fn velocity_is_clamped_into_the_legal_range() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    s.apply(edits::set_velocity(&s.clone(), 0, &[0], 0).unwrap()).unwrap();
    // Velocity 0 would be read back as a note-off, so it must never be stored.
    assert_eq!(s.tracks()[0].notes[0].velocity, 1);
}

