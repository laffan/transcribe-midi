//! Transposition, scale fitting, inversion and harmony.

use super::*;

#[test]
fn transpose_clamps_rather_than_wrapping() {
    let mut workspace = workspace(vec![note(120, 0, 480)]);
    workspace
        .apply(&call(json!({"name": "transpose", "input": {"semitones": 24}})))
        .unwrap();
    assert_eq!(workspace.notes()[0].pitch, 127);
}

#[test]
fn transpose_to_key_takes_the_shorter_direction() {
    let mut workspace = workspace(vec![note(60, 0, 480)]);
    workspace
        .apply(&call(json!({
            "name": "transpose_to_key",
            "input": {"from": "C major", "to": "B major"},
        })))
        .unwrap();
    // Down one semitone, not up eleven.
    assert_eq!(workspace.notes()[0].pitch, 59);
}

#[test]
fn fit_to_scale_leaves_in_key_notes_alone() {
    let mut workspace = workspace(vec![note(60, 0, 480), note(61, 480, 480)]);
    workspace
        .apply(&call(json!({"name": "fit_to_scale", "input": {"key": "C major"}})))
        .unwrap();
    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(notes[0].pitch, 60);
    assert_eq!(notes[1].pitch, 60, "C# folded onto C");
}

#[test]
fn invert_mirrors_around_the_lowest_note_by_default() {
    let mut workspace =
        workspace(vec![note(60, 0, 480), note(64, 480, 480), note(67, 960, 480)]);
    workspace.apply(&call(json!({"name": "invert", "input": {}}))).unwrap();

    let mut by_start = workspace.notes();
    by_start.sort_by_key(|n| n.start_ticks);
    // Around 60: 60 stays, 64 → 56, 67 → 53.
    assert_eq!(
        by_start.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        vec![60, 56, 53]
    );
}

#[test]
fn harmonize_adds_a_diatonic_third() {
    let mut workspace = workspace(vec![note(60, 0, 480), note(62, 480, 480)]);
    workspace.apply(&call(json!({"name": "harmonize", "input": {}}))).unwrap();

    let mut pitches: Vec<u8> = workspace.notes().iter().map(|n| n.pitch).collect();
    pitches.sort_unstable();
    // C+E and D+F — the third is major above C and minor above D, which is the whole
    // point of harmonising in the key rather than by a fixed interval.
    assert_eq!(pitches, vec![60, 62, 64, 65]);
}

#[test]
fn harmonize_below_with_a_fixed_interval() {
    let mut workspace = workspace(vec![note(60, 0, 480)]);
    workspace
        .apply(&call(json!({"name": "harmonize", "input": {"semitones": -12}})))
        .unwrap();
    let mut pitches: Vec<u8> = workspace.notes().iter().map(|n| n.pitch).collect();
    pitches.sort_unstable();
    assert_eq!(pitches, vec![48, 60]);
}

#[test]
fn harmonize_without_a_key_anywhere_is_an_error() {
    let mut context = context();
    context.key = None;
    let mut workspace = Workspace::new(context, &[note(60, 0, 480)], &[]);
    let error = workspace
        .apply(&call(json!({"name": "harmonize", "input": {}})))
        .unwrap_err();
    assert!(error.to_string().contains("no key"), "{error}");
}

