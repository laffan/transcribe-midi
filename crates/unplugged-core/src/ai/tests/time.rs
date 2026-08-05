//! Quantize, humanize, retrograde, durations and velocities.

use super::*;

#[test]
fn quantize_snaps_to_the_grid() {
    let mut workspace = workspace(vec![note(60, 13, 480), note(62, 235, 480)]);
    workspace
        .apply(&call(json!({"name": "quantize", "input": {"grid": "1/8"}})))
        .unwrap();
    let mut starts: Vec<u32> = workspace.notes().iter().map(|n| n.start_ticks).collect();
    starts.sort_unstable();
    assert_eq!(starts, vec![0, 240]);
}

#[test]
fn partial_quantize_moves_part_of_the_way() {
    let mut workspace = workspace(vec![note(60, 100, 480)]);
    workspace
        .apply(&call(json!({
            "name": "quantize",
            "input": {"grid": "1/4", "strength": 0.5},
        })))
        .unwrap();
    assert_eq!(workspace.notes()[0].start_ticks, 50, "halfway back to 0");
}

#[test]
fn humanize_is_deterministic_and_stays_in_range() {
    let source = vec![note(60, 0, 480), note(62, 480, 480), note(64, 960, 480)];
    let mut a = workspace(source.clone());
    let mut b = workspace(source);
    let tool = call(json!({
        "name": "humanize",
        "input": {"timing_ticks": 30, "velocity": 20, "seed": 7},
    }));

    a.apply(&tool).unwrap();
    b.apply(&tool).unwrap();
    assert_eq!(a.notes(), b.notes(), "same seed, same result");

    for note in a.notes() {
        assert!((1..=127).contains(&note.velocity));
        assert!(note.duration_ticks > 0);
    }
}

#[test]
fn humanize_never_moves_a_note_before_zero() {
    let mut workspace = workspace(vec![note(60, 2, 480)]);
    workspace
        .apply(&call(json!({
            "name": "humanize",
            "input": {"timing_ticks": 200, "seed": 3},
        })))
        .unwrap();
    // `start_ticks` is unsigned, so a negative result would have wrapped to two
    // billion rather than failing — which is exactly why this is asserted.
    assert!(workspace.notes()[0].start_ticks < 480);
}

#[test]
fn retrograde_reverses_within_the_span() {
    let mut workspace = workspace(vec![
        note(60, 0, 480),
        note(62, 480, 480),
        note(64, 960, 960),
    ]);
    workspace.apply(&call(json!({"name": "retrograde", "input": {}}))).unwrap();

    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(notes[0].pitch, 64, "the last note is now first");
    assert_eq!(notes[0].start_ticks, 0);
    assert_eq!(notes[2].pitch, 60);
    assert_eq!(notes[2].end_ticks(), 1920, "the span is preserved");
}

#[test]
fn retrograde_twice_is_the_identity() {
    let source = vec![note(60, 0, 480), note(62, 480, 240), note(64, 960, 960)];
    let mut workspace = workspace(source.clone());
    let tool = call(json!({"name": "retrograde", "input": {}}));
    workspace.apply(&tool).unwrap();
    workspace.apply(&tool).unwrap();
    assert_eq!(workspace.notes(), source);
}

#[test]
fn legato_reaches_the_next_note() {
    let mut workspace = workspace(vec![note(60, 0, 100), note(62, 480, 100)]);
    workspace
        .apply(&call(json!({"name": "set_duration", "input": {"legato": true}})))
        .unwrap();
    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(notes[0].duration_ticks, 480);
    assert_eq!(notes[1].duration_ticks, 100, "the last note is left alone");
}

#[test]
fn set_velocity_ramps_across_the_selection() {
    let mut workspace =
        workspace(vec![note(60, 0, 480), note(62, 480, 480), note(64, 960, 480)]);
    workspace
        .apply(&call(json!({
            "name": "set_velocity",
            "input": {"velocity": 40, "ramp_to": 100},
        })))
        .unwrap();
    let mut notes = workspace.notes();
    notes.sort_by_key(|n| n.start_ticks);
    assert_eq!(
        notes.iter().map(|n| n.velocity).collect::<Vec<_>>(),
        vec![40, 70, 100]
    );
}

#[test]
fn set_velocity_with_no_arguments_is_refused() {
    let mut workspace = workspace(vec![note(60, 0, 480)]);
    assert!(workspace
        .apply(&call(json!({"name": "set_velocity", "input": {}})))
        .is_err());
}

