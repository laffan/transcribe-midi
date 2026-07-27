//! A transaction is all-or-nothing, and undoes as one unit.

use super::*;

#[test]
fn a_multi_command_transaction_undoes_as_one_unit() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    let tx = Transaction::new(
        "Harmonize",
        vec![
            Command::Insert { track: 0, notes: vec![note(64, 0, 480, 100)] },
            Command::Insert { track: 0, notes: vec![note(67, 0, 480, 100)] },
        ],
    );
    s.apply(tx).unwrap();
    assert_eq!(pitches(&s), vec![60, 64, 67]);

    s.undo().unwrap();
    assert_eq!(pitches(&s), vec![60], "one undo must revert the whole transaction");
}

#[test]
fn an_invalid_command_leaves_the_session_untouched() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    let before = s.tracks()[0].notes.clone();

    // Second command is invalid; the first must not be applied either.
    let tx = Transaction::new(
        "Half bad",
        vec![
            Command::Insert { track: 0, notes: vec![note(64, 0, 480, 100)] },
            Command::Delete { track: 0, indices: vec![999] },
        ],
    );
    assert!(s.apply(tx).is_err());
    assert_eq!(s.tracks()[0].notes, before, "a rejected transaction must not partially apply");
    assert!(!s.can_undo(), "and must not create a history entry");
}

#[test]
fn replace_rejects_mismatched_index_and_note_counts() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    let tx = Transaction::single(
        "Bad replace",
        Command::Replace { track: 0, indices: vec![0], notes: vec![] },
    );
    assert!(s.apply(tx).is_err());
}

#[test]
fn an_unknown_track_is_an_error() {
    let mut s = session(vec![note(60, 0, 480, 100)]);
    assert!(s.apply(edits::insert_note(9, note(60, 0, 480, 100))).is_err());
}

