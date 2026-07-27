//! The diff, and the round trip through the command layer that makes it one undo step.

use super::*;

#[test]
fn an_untouched_workspace_produces_no_transaction() {
    let workspace = workspace(vec![note(60, 0, 480)]);
    assert!(workspace.diff().is_empty());
    assert!(workspace.to_transaction(0, "AI edit").is_empty());
}

#[test]
fn the_diff_separates_added_removed_and_changed() {
    let mut workspace = workspace(vec![note(60, 0, 480), note(62, 480, 480)]);

    // Change one note...
    workspace
        .apply(&call(json!({
            "name": "select_notes",
            "input": {"pitch_min": 60, "pitch_max": 60},
        })))
        .unwrap();
    workspace
        .apply(&call(json!({"name": "transpose", "input": {"semitones": 2}})))
        .unwrap();

    // ...remove the other...
    workspace
        .apply(&call(json!({
            "name": "select_notes",
            "input": {"start_ticks": 480, "end_ticks": 960},
        })))
        .unwrap();
    workspace.apply(&call(json!({"name": "delete_notes", "input": {}}))).unwrap();

    // ...and add a third.
    workspace
        .apply(&call(json!({
            "name": "insert_notes",
            "input": {"notes": [{"pitch": 67, "start_ticks": 960, "duration_ticks": 480}]},
        })))
        .unwrap();

    let diff = workspace.diff();
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.changed.len(), 1);
    assert_eq!(diff.changed[0].before.pitch, 60);
    assert_eq!(diff.changed[0].after.pitch, 62);
    assert_eq!(diff.summary(), "1 added, 1 removed, 1 changed");
}

/// The property that matters: whatever the tools did, applying the transaction to a
/// real session must reproduce the workspace exactly, and undoing it must restore
/// the original — in one step, however many tools ran.
fn assert_round_trip(source: Vec<Note>, calls: &[serde_json::Value]) {
    let mut workspace = Workspace::new(context(), &source, &[]);
    for value in calls {
        workspace.apply(&call(value.clone())).unwrap();
    }

    let mut track = Track::new(
        TrackMeta {
            id: "t".into(),
            name: "T".into(),
            channel: 0,
            instrument: InstrumentRef::BuiltInSampler,
            muted: false,
            soloed: false,
            color: "#fff".into(),
            key_hint: None,
        },
        DEFAULT_PPQ,
    );
    track.notes = source.clone();

    let mut session = EditSession::new(vec![track]);
    let transaction = workspace.to_transaction(0, "AI edit");
    let expected = {
        let mut notes = workspace.notes();
        notes.sort_by_key(Note::order_key);
        notes
    };

    session.apply(transaction).unwrap();
    assert_eq!(
        session.tracks()[0].notes,
        expected,
        "the commit did not match the preview"
    );

    session.undo().unwrap();
    assert_eq!(
        session.tracks()[0].notes,
        source,
        "undo did not restore the original"
    );
    assert!(!session.can_undo(), "the whole edit must be a single undo step");
}

#[test]
fn a_single_tool_round_trips() {
    assert_round_trip(
        vec![note(60, 0, 480), note(64, 480, 480)],
        &[json!({"name": "transpose", "input": {"semitones": 7}})],
    );
}

#[test]
fn a_long_chain_of_tools_round_trips_as_one_undo_step() {
    assert_round_trip(
        vec![note(60, 13, 470), note(64, 500, 460), note(67, 950, 500)],
        &[
            json!({"name": "quantize", "input": {"grid": "1/8"}}),
            json!({"name": "harmonize", "input": {"degrees": 2, "key": "C major"}}),
            json!({"name": "set_velocity", "input": {"velocity": 60, "ramp_to": 120}}),
            json!({"name": "duplicate", "input": {"offset_bars": 1}}),
            json!({"name": "transpose", "input": {"semitones": -5}}),
            json!({"name": "humanize", "input": {"timing_ticks": 12, "seed": 99}}),
        ],
    );
}

#[test]
fn deleting_everything_round_trips() {
    assert_round_trip(
        vec![note(60, 0, 480), note(64, 480, 480)],
        &[json!({"name": "delete_notes", "input": {}})],
    );
}

#[test]
fn generating_into_an_empty_track_round_trips() {
    assert_round_trip(
        vec![],
        &[
            json!({
                "name": "insert_chord_progression",
                "input": {"progression": "i - VI - III - VII", "key": "A minor"},
            }),
            json!({"name": "arpeggiate", "input": {"division": "1/16", "pattern": "up_down"}}),
        ],
    );
}

#[test]
fn a_transposition_that_reorders_notes_round_trips() {
    // Moving the lower note above the upper one changes sort position, which is the
    // case that makes index-based selection unsafe.
    let source = vec![note(60, 0, 480), note(62, 0, 480)];
    let mut workspace = Workspace::new(context(), &source, &[0]);
    workspace
        .apply(&call(json!({"name": "transpose", "input": {"semitones": 10}})))
        .unwrap();

    let diff = workspace.diff();
    assert_eq!(diff.changed.len(), 1);
    assert_eq!(diff.changed[0].after.pitch, 70);
    assert!(diff.added.is_empty() && diff.removed.is_empty());
}

#[test]
fn the_notes_table_is_truncated() {
    let notes: Vec<Note> = (0..10).map(|i| note(60, i * 480, 480)).collect();
    let table = notes_table(&notes, &context(), 3);
    assert!(table.contains("and 7 more notes"), "{table}");
    assert_eq!(table.lines().count(), 5, "header, three rows, the elision");
}
