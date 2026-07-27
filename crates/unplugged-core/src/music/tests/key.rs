//! Key parsing, membership, snapping, and diatonic steps.

use super::*;

#[test]
fn parses_keys() {
    assert_eq!(Key::parse("C major"), Some(Key::new(0, Scale::Major)));
    assert_eq!(Key::parse("f# minor"), Some(Key::new(6, Scale::NaturalMinor)));
    assert_eq!(Key::parse("Eb dorian"), Some(Key::new(3, Scale::Dorian)));
    assert_eq!(Key::parse("A"), Some(Key::new(9, Scale::Major)), "bare tonic is major");
    assert_eq!(Key::parse("D harmonic minor"), Some(Key::new(2, Scale::HarmonicMinor)));
    assert_eq!(Key::parse("Q major"), None);
}

#[test]
fn c_major_has_the_white_notes() {
    let key = Key::new(0, Scale::Major);
    assert_eq!(key.pitch_classes(), vec![0, 2, 4, 5, 7, 9, 11]);
    assert!(key.contains(60));
    assert!(!key.contains(61));
}

#[test]
fn snapping_finds_the_nearest_scale_tone() {
    let key = Key::new(0, Scale::Major);
    assert_eq!(key.snap(60), 60, "already in key");
    assert_eq!(key.snap(61), 60, "C# down to C");
    assert_eq!(key.snap(66), 65, "F# ties down to F");
    assert_eq!(key.snap(70), 69, "Bb down to A");
}

#[test]
fn snapping_is_idempotent() {
    for scale in [Scale::Major, Scale::MinorPentatonic, Scale::WholeTone, Scale::Blues] {
        let key = Key::new(7, scale);
        for pitch in 0u8..=127 {
            let once = key.snap(pitch);
            assert_eq!(key.snap(once), once, "{:?} at {pitch}", scale);
            assert!(key.contains(once), "{:?} at {pitch} landed off-scale", scale);
        }
    }
}

#[test]
fn snapping_stays_close() {
    let key = Key::new(0, Scale::Major);
    for pitch in 0u8..=127 {
        let snapped = key.snap(pitch) as i32;
        assert!((snapped - pitch as i32).abs() <= 1, "moved too far from {pitch}");
    }
}

#[test]
fn diatonic_transposition_walks_the_scale() {
    let key = Key::new(0, Scale::Major);
    // A third above C is E, a third above D is F — the interval is not constant.
    assert_eq!(key.transpose_degrees(60, 2), 64);
    assert_eq!(key.transpose_degrees(62, 2), 65);
    // A full octave is seven degrees.
    assert_eq!(key.transpose_degrees(60, 7), 72);
    assert_eq!(key.transpose_degrees(60, -7), 48);
    assert_eq!(key.transpose_degrees(60, -1), 59, "down a degree crosses the octave");
}

#[test]
fn diatonic_transposition_snaps_chromatic_input() {
    let key = Key::new(0, Scale::Major);
    // C# is not in C major; it snaps to C first, then moves a third.
    assert_eq!(key.transpose_degrees(61, 2), 64);
}

