//! Note names in and out.

use super::*;

#[test]
fn parses_pitch_classes() {
    assert_eq!(parse_pitch_class("C"), Some(0));
    assert_eq!(parse_pitch_class("c"), Some(0));
    assert_eq!(parse_pitch_class("F#"), Some(6));
    assert_eq!(parse_pitch_class("Gb"), Some(6));
    assert_eq!(parse_pitch_class("Cb"), Some(11));
    assert_eq!(parse_pitch_class("B#"), Some(0));
    assert_eq!(parse_pitch_class("H"), None);
    assert_eq!(parse_pitch_class(""), None);
}

#[test]
fn middle_c_is_sixty() {
    assert_eq!(parse_pitch("C4"), Some(60));
    assert_eq!(parse_pitch("A4"), Some(69));
    assert_eq!(parse_pitch("C-1"), Some(0));
    assert_eq!(parse_pitch("G9"), Some(127));
    assert_eq!(parse_pitch("C10"), None, "past the MIDI range");
    assert_eq!(pitch_name(60), "C4");
    assert_eq!(pitch_name(69), "A4");
}

#[test]
fn pitch_names_round_trip() {
    for pitch in 0u8..=127 {
        assert_eq!(parse_pitch(&pitch_name(pitch)), Some(pitch));
    }
}

