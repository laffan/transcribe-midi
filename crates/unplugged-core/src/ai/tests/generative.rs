//! Tools that add notes: duplication, arpeggios, progressions, and their bounds.

use super::*;

#[test]
fn duplicate_defaults_to_the_next_bar() {
    let mut workspace = workspace(vec![note(60, 0, 480)]);
    workspace.apply(&call(json!({"name": "duplicate", "input": {}}))).unwrap();

    let mut starts: Vec<u32> = workspace.notes().iter().map(|n| n.start_ticks).collect();
    starts.sort_unstable();
    assert_eq!(starts, vec![0, 1920]);
}

#[test]
fn duplicate_transposes_each_copy_further() {
    let mut workspace = workspace(vec![note(60, 0, 480)]);
    workspace
        .apply(&call(json!({
            "name": "duplicate",
            "input": {"offset_bars": 1, "count": 2, "transpose": 12},
        })))
        .unwrap();

    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(
        notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        vec![60, 72, 84]
    );
}

#[test]
fn duplicate_leaves_the_copies_selected() {
    let mut workspace = workspace(vec![note(60, 0, 480)]);
    workspace
        .apply(&call(json!({"name": "duplicate", "input": {"offset_bars": 1}})))
        .unwrap();
    // Chaining is the point: duplicate then transpose should move only the copy.
    workspace
        .apply(&call(json!({"name": "transpose", "input": {"semitones": 5}})))
        .unwrap();

    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(notes[0].pitch, 60, "the original is untouched");
    assert_eq!(notes[1].pitch, 65);
}

#[test]
fn arpeggiate_breaks_a_chord_into_steps() {
    // A C major triad lasting one bar.
    let mut workspace = workspace(vec![
        note(60, 0, 1920),
        note(64, 0, 1920),
        note(67, 0, 1920),
    ]);
    workspace
        .apply(&call(json!({
            "name": "arpeggiate",
            "input": {"division": "1/8", "pattern": "up"},
        })))
        .unwrap();

    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(notes.len(), 8, "eight eighth notes in a 4/4 bar");
    assert_eq!(
        notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        vec![60, 64, 67, 60, 64, 67, 60, 64],
        "the pattern cycles"
    );
    assert!(
        notes.iter().all(|n| n.duration_ticks < 240),
        "gated shorter than the step"
    );
}

#[test]
fn arpeggiate_up_down_does_not_repeat_the_turning_points() {
    let mut workspace = workspace(vec![note(60, 0, 960), note(64, 0, 960), note(67, 0, 960)]);
    workspace
        .apply(&call(json!({
            "name": "arpeggiate",
            "input": {"division": "1/8", "pattern": "up_down"},
        })))
        .unwrap();

    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(
        notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        vec![60, 64, 67, 64]
    );
}

#[test]
fn a_bad_arpeggio_pattern_is_refused() {
    let mut workspace = workspace(vec![note(60, 0, 960)]);
    assert!(workspace
        .apply(&call(json!({
            "name": "arpeggiate",
            "input": {"division": "1/8", "pattern": "sideways"},
        })))
        .is_err());
}

#[test]
fn chord_progressions_land_on_bar_lines() {
    let mut workspace = workspace(vec![]);
    let summary = workspace
        .apply(&call(json!({
            "name": "insert_chord_progression",
            "input": {"progression": "I - V - vi - IV", "key": "C major"},
        })))
        .unwrap();

    let notes = workspace.notes();
    assert_eq!(notes.len(), 12, "four triads");

    let mut starts: Vec<u32> = notes.iter().map(|n| n.start_ticks).collect();
    starts.dedup();
    assert_eq!(starts, vec![0, 1920, 3840, 5760]);
    assert!(summary.contains('C') && summary.contains('G'), "{summary}");
}

#[test]
fn a_progression_can_be_arpeggiated_afterwards() {
    let mut workspace = workspace(vec![]);
    workspace
        .apply(&call(json!({
            "name": "insert_chord_progression",
            "input": {"progression": "ii - V - I", "key": "C major", "bars_per_chord": 1},
        })))
        .unwrap();
    workspace
        .apply(&call(json!({"name": "arpeggiate", "input": {"division": "1/8"}})))
        .unwrap();

    assert_eq!(workspace.notes().len(), 24, "three bars of eighths");
}

#[test]
fn insert_notes_is_bounded() {
    let mut workspace = workspace(vec![]);
    let many: Vec<serde_json::Value> = (0..MAX_NOTES_PER_CALL + 1)
        .map(|i| json!({"pitch": 60, "start_ticks": i * 10, "duration_ticks": 5}))
        .collect();

    let error = workspace
        .apply(&call(json!({"name": "insert_notes", "input": {"notes": many}})))
        .unwrap_err();
    assert!(error.to_string().contains("at most"), "{error}");
    assert!(workspace.notes().is_empty(), "nothing was added");
}

#[test]
fn a_tool_that_fails_leaves_the_workspace_alone() {
    let source = vec![note(60, 0, 480)];
    let mut workspace = workspace(source.clone());
    assert!(workspace
        .apply(&call(json!({"name": "quantize", "input": {"grid": "1/7"}})))
        .is_err());
    assert_eq!(workspace.notes(), source);
}

