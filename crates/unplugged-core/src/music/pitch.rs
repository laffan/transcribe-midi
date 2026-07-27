//! Pitch classes and note names, in both directions.
//!
//! Parsing accepts what a person or a model would write — flats, sharps, either case —
//! while display always spells sharps. The asymmetry is deliberate: the spelling we emit
//! is never fed back through the parser, so there is no round trip to preserve.

use super::OCTAVE;

/// Sharp spelling. Used for display only — the model is never asked to parse this back.
pub(super) const SHARP_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

// ---------------------------------------------------------------------------
// Pitch names
// ---------------------------------------------------------------------------

/// Parse a pitch class: `C`, `f#`, `Bb`, `Ebb`, `G##`.
///
/// Accepts any number of accidentals because a model asked for "the flat seventh of Cb"
/// will occasionally produce a double flat, and wrapping is harmless.
pub fn parse_pitch_class(text: &str) -> Option<u8> {
    let text = text.trim();
    let mut chars = text.chars();
    let letter = chars.next()?.to_ascii_uppercase();

    let base: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };

    let mut value = base;
    for c in chars {
        match c {
            '#' | '♯' => value += 1,
            'b' | 'B' | '♭' => value -= 1,
            // A trailing octave digit is handled by `parse_pitch`, not here.
            _ => return None,
        }
    }

    Some(value.rem_euclid(OCTAVE) as u8)
}

/// Parse an absolute pitch: `C4` is 60, matching the piano roll's labelling.
///
/// Scientific pitch notation has no universal middle-C octave; this codebase uses the
/// MIDI-standard C4 = 60 convention throughout, and the piano roll agrees.
pub fn parse_pitch(text: &str) -> Option<u8> {
    let text = text.trim();
    let split = text
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit() || *c == '-')
        .map(|(i, _)| i)?;
    let (name, octave) = text.split_at(split);

    let class = parse_pitch_class(name)? as i32;
    let octave: i32 = octave.parse().ok()?;
    let value = (octave + 1) * OCTAVE + class;

    (0..=127).contains(&value).then_some(value as u8)
}

/// Render a MIDI pitch as `C4`. Always sharp-spelled.
pub fn pitch_name(pitch: u8) -> String {
    let class = (pitch % 12) as usize;
    let octave = (pitch / 12) as i32 - 1;
    format!("{}{}", SHARP_NAMES[class], octave)
}
