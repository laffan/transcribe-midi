//! Roman numerals and progressions.
//!
//! Separate from [`chord`](super::chord) because a numeral is meaningless without a key:
//! `ii` is a chord only once you know what the tonic is, and which quality the scale
//! gives that degree.

use crate::error::{CoreError, Result};

use super::chord::{Chord, ChordQuality};
use super::key::Key;
use super::OCTAVE;

/// Parse a roman numeral in a key: `I`, `ii`, `V7`, `bVII`, `vii°`, `iv6`.
///
/// Case carries meaning — uppercase is major, lowercase is minor — and an explicit
/// suffix overrides it. An unqualified numeral takes the quality the scale gives that
/// degree, so `ii` in C major is D minor without anyone having to say so.
pub fn parse_roman(text: &str, key: &Key) -> Option<Chord> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // Leading accidental shifts the root off the diatonic degree.
    let mut chars = text.char_indices().peekable();
    let mut accidental = 0i32;
    let mut start = 0usize;
    while let Some(&(index, c)) = chars.peek() {
        match c {
            'b' | '♭' => {
                accidental -= 1;
                start = index + c.len_utf8();
                chars.next();
            }
            '#' | '♯' => {
                accidental += 1;
                start = index + c.len_utf8();
                chars.next();
            }
            _ => break,
        }
    }

    let rest = &text[start..];
    // Longest numeral first, or `iii` would match `ii` and leave a stray `i`.
    const NUMERALS: [(&str, usize); 7] = [
        ("vii", 6),
        ("iii", 2),
        ("vi", 5),
        ("iv", 3),
        ("ii", 1),
        ("v", 4),
        ("i", 0),
    ];

    let lower = rest.to_ascii_lowercase();
    let (numeral, degree) = NUMERALS
        .iter()
        .find(|(numeral, _)| lower.starts_with(numeral))
        .copied()?;

    let head = &rest[..numeral.len()];
    let suffix = &rest[numeral.len()..];
    let uppercase = head.chars().next()?.is_uppercase();

    // Diatonic root for this degree. Scales with fewer than seven notes cannot express
    // every numeral, so the request is refused rather than silently folded.
    let intervals = key.scale.intervals();
    let interval = *intervals.get(degree)?;
    let root = (key.tonic as i32 + interval as i32 + accidental).rem_euclid(OCTAVE) as u8;

    let quality = if !suffix.is_empty() {
        ChordQuality::parse_suffix(suffix)?
    } else if accidental != 0 {
        // An accidental moves the root off its scale degree, so the key's own harmony no
        // longer describes the chord. `bVII` in C major is a borrowed B♭ *major* triad,
        // not the diminished triad the seventh degree would otherwise give. Case is the
        // only signal left, and it is the signal the notation intends.
        if uppercase {
            ChordQuality::Major
        } else {
            ChordQuality::Minor
        }
    } else {
        diatonic_quality(key, degree, uppercase)
    };

    Some(Chord::new(root, quality))
}

/// Quality of the triad built on `degree` by stacking thirds within the key.
///
/// Derived rather than tabulated, so it stays correct for the modes and for harmonic
/// minor's raised seventh instead of only for major and natural minor.
fn diatonic_quality(key: &Key, degree: usize, uppercase: bool) -> ChordQuality {
    let intervals = key.scale.intervals();
    if intervals.len() < 7 {
        return if uppercase { ChordQuality::Major } else { ChordQuality::Minor };
    }

    let at = |offset: usize| -> i32 {
        let index = (degree + offset) % intervals.len();
        let wraps = (degree + offset) / intervals.len();
        intervals[index] as i32 + wraps as i32 * OCTAVE
    };

    let third = at(2) - at(0);
    let fifth = at(4) - at(0);

    match (third, fifth) {
        (3, 6) => ChordQuality::Diminished,
        (3, _) => ChordQuality::Minor,
        (4, 8) => ChordQuality::Augmented,
        (4, _) => ChordQuality::Major,
        // Anything else is not a stack of thirds (pentatonic degrees, whole tone).
        // Fall back to what the numeral's case asked for.
        _ => {
            if uppercase {
                ChordQuality::Major
            } else {
                ChordQuality::Minor
            }
        }
    }
}

/// Parse a whole progression: `ii - V7 - I` or `Am | F | C | G`.
///
/// Accepts roman numerals and absolute chord symbols in the same list, because models
/// mix them, and a progression that half-parses is worse than one that fails loudly.
pub fn parse_progression(text: &str, key: &Key) -> Result<Vec<Chord>> {
    let chords: Vec<&str> = text
        .split(['-', '|', ',', '–', '—'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if chords.is_empty() {
        return Err(CoreError::Invalid("empty chord progression".into()));
    }

    chords
        .into_iter()
        .map(|symbol| {
            parse_roman(symbol, key)
                .or_else(|| Chord::parse(symbol))
                .ok_or_else(|| CoreError::Invalid(format!("could not read the chord \"{symbol}\"")))
        })
        .collect()
}
