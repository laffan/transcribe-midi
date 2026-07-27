//! Roman numerals against a key, and whole progressions.

use super::*;

#[test]
fn roman_numerals_take_the_key_signature() {
    let c = Key::new(0, Scale::Major);
    assert_eq!(parse_roman("I", &c), Some(Chord::new(0, ChordQuality::Major)));
    assert_eq!(parse_roman("ii", &c), Some(Chord::new(2, ChordQuality::Minor)));
    assert_eq!(parse_roman("iii", &c), Some(Chord::new(4, ChordQuality::Minor)));
    assert_eq!(parse_roman("IV", &c), Some(Chord::new(5, ChordQuality::Major)));
    assert_eq!(parse_roman("V", &c), Some(Chord::new(7, ChordQuality::Major)));
    assert_eq!(parse_roman("vi", &c), Some(Chord::new(9, ChordQuality::Minor)));
    assert_eq!(
        parse_roman("vii", &c),
        Some(Chord::new(11, ChordQuality::Diminished)),
        "the leading-tone triad is diminished without being told"
    );
}

#[test]
fn roman_numerals_in_minor() {
    let a = Key::new(9, Scale::NaturalMinor);
    assert_eq!(parse_roman("i", &a), Some(Chord::new(9, ChordQuality::Minor)));
    assert_eq!(parse_roman("iv", &a), Some(Chord::new(2, ChordQuality::Minor)));
    assert_eq!(parse_roman("VI", &a), Some(Chord::new(5, ChordQuality::Major)));
    assert_eq!(
        parse_roman("ii", &a),
        Some(Chord::new(11, ChordQuality::Diminished)),
        "the supertonic triad in natural minor is diminished"
    );
}

#[test]
fn roman_numerals_take_accidentals_and_suffixes() {
    let c = Key::new(0, Scale::Major);
    assert_eq!(parse_roman("bVII", &c), Some(Chord::new(10, ChordQuality::Major)));
    assert_eq!(parse_roman("V7", &c), Some(Chord::new(7, ChordQuality::Dominant7)));
    assert_eq!(parse_roman("Imaj7", &c), Some(Chord::new(0, ChordQuality::Major7)));
}

#[test]
fn harmonic_minor_raises_the_dominant() {
    let a = Key::new(9, Scale::HarmonicMinor);
    assert_eq!(
        parse_roman("V", &a),
        Some(Chord::new(4, ChordQuality::Major)),
        "the raised seventh makes the dominant major"
    );
}

#[test]
fn parses_a_progression_of_either_notation() {
    let c = Key::new(0, Scale::Major);
    let progression = parse_progression("ii - V7 - I", &c).unwrap();
    assert_eq!(
        progression,
        vec![
            Chord::new(2, ChordQuality::Minor),
            Chord::new(7, ChordQuality::Dominant7),
            Chord::new(0, ChordQuality::Major),
        ]
    );

    let absolute = parse_progression("Am | F | C | G", &c).unwrap();
    assert_eq!(absolute.len(), 4);
    assert_eq!(absolute[0], Chord::new(9, ChordQuality::Minor));
    assert_eq!(absolute[3], Chord::new(7, ChordQuality::Major));
}

#[test]
fn a_bad_chord_fails_the_whole_progression() {
    let c = Key::new(0, Scale::Major);
    assert!(parse_progression("I - nonsense - V", &c).is_err());
    assert!(parse_progression("", &c).is_err());
}
