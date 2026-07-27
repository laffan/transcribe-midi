//! The note list stays sorted, and identical notes stay distinguishable.

use super::*;

#[test]
fn moving_a_note_past_another_keeps_the_list_sorted() {
    // This is the case that breaks an in-place assignment implementation: note 0
    // moves after note 1, so its sort position changes.
    let mut s = session(vec![note(60, 0, 480, 100), note(64, 480, 480, 90)]);

    let tx = edits::move_notes(&s, 0, &[0], 960, 0).unwrap();
    s.apply(tx).unwrap();

    assert!(s.tracks()[0].is_sorted(), "list must remain sorted after a reordering move");
    assert_eq!(pitches(&s), vec![64, 60]);

    s.undo().unwrap();
    assert_eq!(pitches(&s), vec![60, 64]);
    assert!(s.tracks()[0].is_sorted());
}

#[test]
fn transposing_across_another_note_at_the_same_tick_keeps_sorting() {
    // Same start tick, so ordering is decided by pitch.
    let mut s = session(vec![note(60, 0, 480, 100), note(64, 0, 480, 90)]);
    let tx = edits::move_notes(&s, 0, &[0], 0, 12).unwrap(); // 60 -> 72
    s.apply(tx).unwrap();

    assert!(s.tracks()[0].is_sorted());
    assert_eq!(pitches(&s), vec![64, 72]);
}

#[test]
fn undo_restores_exactly_after_a_multi_note_move() {
    let original = vec![note(60, 0, 480, 100), note(64, 480, 240, 90), note(67, 960, 120, 80)];
    let mut s = session(original.clone());

    let tx = edits::move_notes(&s, 0, &[0, 1, 2], 240, 3).unwrap();
    s.apply(tx).unwrap();
    assert_ne!(s.tracks()[0].notes, original);

    s.undo().unwrap();
    assert_eq!(s.tracks()[0].notes, original, "undo must be exact, not approximate");
}

#[test]
fn identical_notes_are_tracked_separately() {
    // Two notes that compare equal must not collapse into one on undo.
    let mut s = session(vec![note(60, 0, 480, 100), note(60, 0, 480, 100)]);
    assert_eq!(s.tracks()[0].notes.len(), 2);

    s.apply(edits::delete_notes(0, vec![0])).unwrap();
    assert_eq!(s.tracks()[0].notes.len(), 1);

    s.undo().unwrap();
    assert_eq!(s.tracks()[0].notes.len(), 2, "both duplicates must come back");
}

