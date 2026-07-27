//! Flattening a project into a timeline, honouring mute and solo.

use super::*;

#[test]
fn from_project_emits_a_pair_per_note() {
    let p = project(vec![track(
        "t1",
        vec![Note::new(60, 0, 480, 100, 0).unwrap(), Note::new(64, 480, 480, 90, 0).unwrap()],
        false,
        false,
    )]);
    let timeline = Timeline::from_project(&p);
    assert_eq!(timeline.events().len(), 4);
    assert_eq!(timeline.length_ticks(), 960);
}

#[test]
fn muted_tracks_are_excluded() {
    let p = project(vec![
        track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], false, false),
        track("b", vec![Note::new(72, 0, 480, 100, 0).unwrap()], true, false),
    ]);
    let timeline = Timeline::from_project(&p);
    assert!(timeline.events().iter().all(|e| e.pitch == 60));
}

#[test]
fn solo_overrides_mute_on_other_tracks() {
    // Track b is soloed; track a is not muted but must still fall silent.
    let p = project(vec![
        track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], false, false),
        track("b", vec![Note::new(72, 0, 480, 100, 0).unwrap()], false, true),
    ]);
    let timeline = Timeline::from_project(&p);
    assert!(!timeline.is_empty());
    assert!(
        timeline.events().iter().all(|e| e.pitch == 72),
        "only the soloed track should sound"
    );
}

#[test]
fn a_track_that_is_both_muted_and_soloed_still_sounds() {
    // Solo wins, matching every DAW's behaviour.
    let p = project(vec![track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], true, true)]);
    assert!(!Timeline::from_project(&p).is_empty());
}

#[test]
fn track_index_is_carried_through_so_events_can_be_routed() {
    let p = project(vec![
        track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], false, false),
        track("b", vec![Note::new(72, 0, 480, 100, 1).unwrap()], false, false),
    ]);
    let timeline = Timeline::from_project(&p);
    let second: Vec<_> = timeline.events().iter().filter(|e| e.track == 1).collect();
    assert_eq!(second.len(), 2);
    assert!(second.iter().all(|e| e.pitch == 72 && e.channel == 1));
}

