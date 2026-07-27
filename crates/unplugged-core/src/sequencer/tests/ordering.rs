//! What happens when two events share a tick.

use super::*;

#[test]
fn note_off_precedes_note_on_at_the_same_tick() {
    // Otherwise a repeated note is cut off by the previous note's release.
    let timeline = Timeline::new(vec![
        ev(480, EventKind::NoteOn, 60),
        ev(480, EventKind::NoteOff, 60),
    ]);
    assert_eq!(timeline.events()[0].kind, EventKind::NoteOff);
    assert_eq!(timeline.events()[1].kind, EventKind::NoteOn);
}

