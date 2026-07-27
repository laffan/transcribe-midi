//! Apply, undo, redo, and what the history labels say.

use super::*;

#[test]
fn insert_then_undo_restores_the_original() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    s.apply(edits::insert_note(0, note(64, 480, 480, 100))).unwrap();
    assert_eq!(pitches(&s), vec![60, 64]);

    s.undo().unwrap();
    assert_eq!(pitches(&s), vec![60]);

    s.redo().unwrap();
    assert_eq!(pitches(&s), vec![60, 64]);
}

#[test]
fn delete_then_undo_restores_notes_in_the_right_places() {
    let mut s = session(vec![note(60, 0, 480, 100), note(64, 480, 480, 90), note(67, 960, 480, 80)]);
    s.apply(edits::delete_notes(0, vec![0, 2])).unwrap();
    assert_eq!(pitches(&s), vec![64]);

    s.undo().unwrap();
    assert_eq!(pitches(&s), vec![60, 64, 67], "order must be restored, not appended");
    assert!(s.tracks()[0].is_sorted());
}

#[test]
fn an_empty_transaction_leaves_no_history_entry() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    s.apply(Transaction::new("Nothing", Vec::new())).unwrap();
    assert!(!s.can_undo(), "a no-op drag must not create an undo step");
}

#[test]
fn applying_clears_the_redo_stack() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    s.apply(edits::insert_note(0, note(64, 480, 480, 100))).unwrap();
    s.undo().unwrap();
    assert!(s.can_redo());

    s.apply(edits::insert_note(0, note(67, 960, 480, 100))).unwrap();
    assert!(!s.can_redo(), "a new edit must invalidate the redo branch");
}

#[test]
fn undo_and_redo_report_nothing_when_the_stacks_are_empty() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    assert!(s.undo().unwrap().is_none());
    assert!(s.redo().unwrap().is_none());
}

#[test]
fn history_labels_track_the_stacks() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    assert_eq!(s.undo_label(), None);

    s.apply(edits::insert_note(0, note(64, 480, 480, 100))).unwrap();
    assert_eq!(s.undo_label(), Some("Insert note"));

    s.undo().unwrap();
    assert_eq!(s.redo_label(), Some("Insert note"));
    assert_eq!(s.undo_label(), None);
}

