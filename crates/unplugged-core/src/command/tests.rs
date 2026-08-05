//! Tests for the command layer — the spec for undo, redo and every gesture's inverse.
//!
//! Split from `command.rs` under the 700-line rule.

use super::*;
use crate::model::{InstrumentRef, TrackMeta, DEFAULT_PPQ};

fn note(pitch: u8, start: u32, dur: u32, vel: u8) -> Note {
    Note::new(pitch, start, dur, vel, 0).unwrap()
}

fn session(notes: Vec<Note>) -> EditSession {
    let mut track = Track::new(
        TrackMeta {
            id: "t".into(), name: "T".into(), channel: 0,
            instrument: InstrumentRef::BuiltInSampler, muted: false, soloed: false,
            color: "#fff".into(), key_hint: None,
        },
        DEFAULT_PPQ,
    );
    track.notes = notes;
    track.sort_notes();
    EditSession::new(vec![track])
}

fn pitches(s: &EditSession) -> Vec<u8> {
    s.tracks()[0].notes.iter().map(|n| n.pitch).collect()
}

// -- basics -------------------------------------------------------------

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

// -- ordering invariants ------------------------------------------------

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

// -- clamping -----------------------------------------------------------

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

// -- duplicates ---------------------------------------------------------

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

// -- transactions -------------------------------------------------------

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

// -- musical helpers ----------------------------------------------------

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

// -- history bounds -----------------------------------------------------

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

// -- join ---------------------------------------------------------------

#[test]
fn joining_spans_from_the_first_attack_to_the_last_release() {
    let mut s = session(vec![
        note(60, 0, 100, 100),
        note(62, 240, 100, 90),
        note(64, 480, 120, 80),
    ]);
    let tx = edits::join_notes(&s, 0, &[0, 1, 2]).unwrap();
    s.apply(tx).unwrap();

    assert_eq!(s.tracks()[0].notes.len(), 1);
    let joined = s.tracks()[0].notes[0];
    assert_eq!(joined.start_ticks, 0);
    assert_eq!(joined.duration_ticks, 600, "to the end of the last one");
    assert_eq!(joined.pitch, 60, "the note that started is the note you meant");
    assert_eq!(joined.velocity, 100);
}

#[test]
fn joining_closes_the_gaps_between_the_fragments() {
    // The point of the command: a held note the transcriber broke into three comes
    // back as one continuous note, not three that touch.
    let mut s = session(vec![note(60, 0, 90, 100), note(60, 120, 90, 100)]);
    s.apply(edits::join_notes(&s, 0, &[0, 1]).unwrap()).unwrap();
    assert_eq!(s.tracks()[0].notes[0].duration_ticks, 210);
}

#[test]
fn joining_is_one_undo_step() {
    let original = vec![note(60, 0, 100, 100), note(62, 240, 100, 90)];
    let mut s = session(original.clone());
    s.apply(edits::join_notes(&s, 0, &[0, 1]).unwrap()).unwrap();
    assert_eq!(s.tracks()[0].notes.len(), 1);

    s.undo().unwrap();
    assert_eq!(s.tracks()[0].notes, original, "both fragments come back together");
    s.redo().unwrap();
    assert_eq!(s.tracks()[0].notes.len(), 1);
}

#[test]
fn joining_ignores_notes_outside_the_selection() {
    let mut s = session(vec![
        note(60, 0, 100, 100),
        note(62, 240, 100, 90),
        note(67, 960, 100, 90),
    ]);
    s.apply(edits::join_notes(&s, 0, &[0, 1]).unwrap()).unwrap();

    let notes = &s.tracks()[0].notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1].pitch, 67, "the unselected note is untouched");
    assert_eq!(notes[1].start_ticks, 960);
}

#[test]
fn joining_fewer_than_two_notes_leaves_no_history_entry() {
    let mut s = session(vec![note(60, 0, 100, 100)]);
    s.apply(edits::join_notes(&s, 0, &[0]).unwrap()).unwrap();
    s.apply(edits::join_notes(&s, 0, &[]).unwrap()).unwrap();
    assert!(!s.can_undo(), "nothing happened, so there is nothing to undo");
    assert_eq!(s.tracks()[0].notes.len(), 1);
}

#[test]
fn joining_survives_indices_that_no_longer_exist() {
    // The selection lives in the webview and can be one edit behind.
    let mut s = session(vec![note(60, 0, 100, 100), note(62, 240, 100, 90)]);
    s.apply(edits::join_notes(&s, 0, &[0, 1, 9]).unwrap()).unwrap();
    assert_eq!(s.tracks()[0].notes.len(), 1);
}

#[test]
fn joining_a_selection_out_of_order_still_starts_at_the_earliest() {
    let mut s = session(vec![note(67, 480, 100, 70), note(60, 0, 100, 100)]);
    // The track is sorted, so index 0 is the C. Ask for them backwards anyway.
    s.apply(edits::join_notes(&s, 0, &[1, 0]).unwrap()).unwrap();
    let joined = s.tracks()[0].notes[0];
    assert_eq!(joined.pitch, 60);
    assert_eq!(joined.start_ticks, 0);
    assert_eq!(joined.duration_ticks, 580);
}
