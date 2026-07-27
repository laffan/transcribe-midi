//! The gesture builders in `edits`: quantize and paste.

use super::*;

#[test]
fn quantize_snaps_to_the_nearest_grid_line_in_both_directions() {
    let mut s = session(vec![note(60, 10, 480, 100), note(64, 230, 480, 100)]);
    let tx = edits::quantize(&s, 0, &[0, 1], 240).unwrap();
    s.apply(tx).unwrap();

    let starts: Vec<u32> = s.tracks()[0].notes.iter().map(|n| n.start_ticks).collect();
    assert_eq!(starts, vec![0, 240], "10 rounds down, 230 rounds up");
}

#[test]
fn quantize_with_a_zero_grid_is_a_no_op_rather_than_a_divide_by_zero() {
    let mut s = session(vec![note(60, 10, 480, 100)]);
    let tx = edits::quantize(&s, 0, &[0], 0).unwrap();
    s.apply(tx).unwrap();
    assert_eq!(s.tracks()[0].notes[0].start_ticks, 10);
}

#[test]
fn paste_preserves_relative_timing_within_the_group() {
    let mut s = session(vec![]);
    let clipboard = vec![note(60, 1000, 240, 100), note(64, 1240, 240, 100)];

    s.apply(edits::paste(0, &clipboard, 0)).unwrap();

    let starts: Vec<u32> = s.tracks()[0].notes.iter().map(|n| n.start_ticks).collect();
    assert_eq!(starts, vec![0, 240], "the 240-tick gap must survive the move to zero");
}

#[test]
fn pasting_an_empty_clipboard_does_nothing() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    s.apply(edits::paste(0, &[], 480)).unwrap();
    assert_eq!(s.tracks()[0].notes.len(), 1);
    assert!(!s.can_undo());
}

