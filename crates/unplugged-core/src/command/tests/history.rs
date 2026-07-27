//! History is bounded, and stays consistent over a long undo/redo run.

use super::*;

#[test]
fn history_is_bounded() {
    let mut s = session(vec![]);
    for i in 0..(MAX_HISTORY + 50) {
        s.apply(edits::insert_note(0, note(60, i as u32 * 10, 5, 100))).unwrap();
    }
    assert_eq!(s.undo_stack.len(), MAX_HISTORY, "history must not grow without bound");
    assert_eq!(s.undo_labels.len(), MAX_HISTORY, "labels must stay in step with the stack");
}

#[test]
fn a_long_undo_redo_run_stays_consistent() {
    let original = vec![note(60, 0, 480, 100), note(64, 480, 480, 90)];
    let mut s = session(original.clone());

    for i in 0..20 {
        let tx = edits::move_notes(&s, 0, &[0], 24, if i % 2 == 0 { 1 } else { -1 }).unwrap();
        s.apply(tx).unwrap();
    }
    for _ in 0..20 {
        s.undo().unwrap();
    }
    assert_eq!(s.tracks()[0].notes, original, "20 undos must land exactly on the start state");

    for _ in 0..20 {
        s.redo().unwrap();
    }
    for _ in 0..20 {
        s.undo().unwrap();
    }
    assert_eq!(s.tracks()[0].notes, original, "and again after a full redo/undo cycle");
}
