//! Moving material between two PPQ values without moving it in musical time.

use super::*;

#[test]
fn rescaling_ppq_preserves_musical_position() {
    // A quarter note at 96 ppq becomes a quarter note at 480 ppq.
    let mut notes = vec![note(60, 96, 96, 100, 0), note(64, 192, 48, 100, 0)];
    rescale_ppq(&mut notes, 96, 480);

    assert_eq!(notes[0].start_ticks, 480);
    assert_eq!(notes[0].duration_ticks, 480);
    assert_eq!(notes[1].start_ticks, 960);
    assert_eq!(notes[1].duration_ticks, 240);
}

#[test]
fn rescaling_down_never_annihilates_a_short_note() {
    // A 1-tick note at 960 ppq would round to 0 at 96 ppq, which SMF cannot express.
    let mut notes = vec![note(60, 0, 1, 100, 0)];
    rescale_ppq(&mut notes, 960, 96);
    assert_eq!(notes[0].duration_ticks, 1);
}

#[test]
fn rescaling_to_the_same_ppq_is_a_no_op() {
    let original = vec![note(60, 137, 499, 100, 0)];
    let mut notes = original.clone();
    rescale_ppq(&mut notes, 480, 480);
    assert_eq!(notes, original);

    // Zero must not divide by zero or zero everything out.
    rescale_ppq(&mut notes, 0, 480);
    rescale_ppq(&mut notes, 480, 0);
    assert_eq!(notes, original);
}
