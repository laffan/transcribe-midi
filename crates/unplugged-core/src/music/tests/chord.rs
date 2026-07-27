//! Chord symbols and voicing.

use super::*;

#[test]
fn parses_chord_symbols() {
    assert_eq!(Chord::parse("C"), Some(Chord::new(0, ChordQuality::Major)));
    assert_eq!(Chord::parse("Am"), Some(Chord::new(9, ChordQuality::Minor)));
    assert_eq!(Chord::parse("Bbmaj7"), Some(Chord::new(10, ChordQuality::Major7)));
    assert_eq!(Chord::parse("F#m7b5"), Some(Chord::new(6, ChordQuality::HalfDiminished7)));
    assert_eq!(Chord::parse("G7"), Some(Chord::new(7, ChordQuality::Dominant7)));
    assert_eq!(Chord::parse("Csus4"), Some(Chord::new(0, ChordQuality::Sus4)));
    assert_eq!(Chord::parse("Xyz"), None);
}

#[test]
fn voices_chords_upward_from_a_floor() {
    let c_major = Chord::new(0, ChordQuality::Major);
    assert_eq!(c_major.voice(60), vec![60, 64, 67]);
    assert_eq!(c_major.voice(61), vec![72, 76, 79], "next C at or above 61");

    let ninth = Chord::new(0, ChordQuality::Dominant9);
    assert_eq!(ninth.voice(60), vec![60, 64, 67, 70, 74]);
}

#[test]
fn drops_chord_tones_past_the_midi_range() {
    let high = Chord::new(0, ChordQuality::Dominant9);
    let voiced = high.voice(120);
    assert!(voiced.iter().all(|&p| p <= 127));
    assert!(voiced.len() < 5, "the upper extensions do not fit");
}

