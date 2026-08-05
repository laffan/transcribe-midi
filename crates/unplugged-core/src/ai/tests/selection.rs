//! What a tool acts on when the editor has, and has not, made a selection.

use super::*;

#[test]
fn an_empty_editor_selection_means_the_whole_track() {
    let workspace = Workspace::new(context(), &[note(60, 0, 480), note(64, 480, 480)], &[]);
    assert_eq!(workspace.selection_len(), 2);
}

#[test]
fn the_editor_selection_is_respected() {
    let workspace = Workspace::new(context(), &[note(60, 0, 480), note(64, 480, 480)], &[1]);
    assert_eq!(workspace.selection_len(), 1);
}

#[test]
fn selection_filters_combine() {
    let mut workspace = workspace(vec![
        note(60, 0, 480),
        note(72, 480, 480),
        note(64, 1920, 480),
    ]);

    workspace
        .apply(&call(json!({
            "name": "select_notes",
            "input": {"start_ticks": 0, "end_ticks": 960, "pitch_min": 65},
        })))
        .unwrap();

    assert_eq!(workspace.selection_len(), 1, "only the high note in bar 1");
    workspace
        .apply(&call(json!({"name": "transpose", "input": {"semitones": 1}})))
        .unwrap();

    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(notes[1].pitch, 73);
    assert_eq!(notes[0].pitch, 60, "the others are untouched");
}

#[test]
fn selecting_by_time_uses_note_starts() {
    // A whole note in bar 1 is not "a note in bar 2" just because it is still
    // sounding there.
    let mut workspace = workspace(vec![note(60, 0, 1920 * 2)]);
    workspace
        .apply(&call(json!({
            "name": "select_notes",
            "input": {"start_ticks": 1920, "end_ticks": 3840},
        })))
        .unwrap();
    assert_eq!(workspace.selection_len(), 0);
}

