//! Scales, keys and chords.
//!
//! Phase 6's AI tools speak musically — "fit to E minor", "harmonise a third above",
//! "ii–V–I" — and something has to turn that into pitch numbers. That translation is
//! pure arithmetic with no I/O, so it lives here where it can be tested exhaustively
//! rather than inside a network client whose behaviour depends on a model's output.
//!
//! Everything is expressed in **pitch classes** (0–11, C = 0) plus an octave, because
//! every musical operation here is octave-invariant. Voicing decisions — which octave a
//! harmonised note actually lands in — are made by the caller.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Semitones per octave. Named because `% 12` on its own reads as a magic number.
pub const OCTAVE: i32 = 12;

/// Sharp spelling. Used for display only — the model is never asked to parse this back.
const SHARP_NAMES: [&str; 12] = [
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

// ---------------------------------------------------------------------------
// Scales
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    WholeTone,
    Chromatic,
}

impl Scale {
    /// Semitone offsets from the tonic, ascending, one octave.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            // Ascending melodic minor. The descending form is a performance convention,
            // not a pitch set, and encoding it here would make `contains` direction-
            // dependent for no benefit.
            Scale::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Blues => &[0, 3, 5, 6, 7, 10],
            Scale::WholeTone => &[0, 2, 4, 6, 8, 10],
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Scale::Major => "major",
            Scale::NaturalMinor => "minor",
            Scale::HarmonicMinor => "harmonic minor",
            Scale::MelodicMinor => "melodic minor",
            Scale::Dorian => "dorian",
            Scale::Phrygian => "phrygian",
            Scale::Lydian => "lydian",
            Scale::Mixolydian => "mixolydian",
            Scale::Locrian => "locrian",
            Scale::MajorPentatonic => "major pentatonic",
            Scale::MinorPentatonic => "minor pentatonic",
            Scale::Blues => "blues",
            Scale::WholeTone => "whole tone",
            Scale::Chromatic => "chromatic",
        }
    }

    /// Every name the tool layer accepts. Generous, because the model writes these.
    pub fn parse(text: &str) -> Option<Scale> {
        let normalised: String = text
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();

        Some(match normalised.as_str() {
            "major" | "maj" | "ionian" => Scale::Major,
            "minor" | "min" | "m" | "naturalminor" | "aeolian" => Scale::NaturalMinor,
            "harmonicminor" | "harmonic" => Scale::HarmonicMinor,
            "melodicminor" | "melodic" => Scale::MelodicMinor,
            "dorian" => Scale::Dorian,
            "phrygian" => Scale::Phrygian,
            "lydian" => Scale::Lydian,
            "mixolydian" | "mixo" => Scale::Mixolydian,
            "locrian" => Scale::Locrian,
            "majorpentatonic" | "pentatonic" | "pentatonicmajor" => Scale::MajorPentatonic,
            "minorpentatonic" | "pentatonicminor" => Scale::MinorPentatonic,
            "blues" => Scale::Blues,
            "wholetone" | "whole" => Scale::WholeTone,
            "chromatic" => Scale::Chromatic,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    /// Pitch class of the tonic, 0–11.
    pub tonic: u8,
    pub scale: Scale,
}

impl Key {
    pub fn new(tonic: u8, scale: Scale) -> Self {
        Key { tonic: tonic % 12, scale }
    }

    /// Parse `C major`, `f# minor`, `Eb dorian`, `A` (bare tonic implies major).
    pub fn parse(text: &str) -> Option<Key> {
        let text = text.trim();
        // Split at the first whitespace, so the tonic keeps its accidentals and the rest
        // — however many words it is — goes to the scale parser.
        let (tonic_text, scale_text) = match text.split_once(char::is_whitespace) {
            Some((a, b)) => (a, b.trim()),
            None => (text, ""),
        };

        let tonic = parse_pitch_class(tonic_text)?;
        let scale = if scale_text.is_empty() {
            Scale::Major
        } else {
            Scale::parse(scale_text)?
        };

        Some(Key::new(tonic, scale))
    }

    pub fn name(&self) -> String {
        format!("{} {}", SHARP_NAMES[self.tonic as usize], self.scale.name())
    }

    /// The pitch classes in this key, ascending from the tonic.
    pub fn pitch_classes(&self) -> Vec<u8> {
        self.scale
            .intervals()
            .iter()
            .map(|&i| ((self.tonic as i32 + i as i32).rem_euclid(OCTAVE)) as u8)
            .collect()
    }

    pub fn contains(&self, pitch: u8) -> bool {
        let class = pitch % 12;
        self.pitch_classes().contains(&class)
    }

    /// Nearest pitch in the key.
    ///
    /// Ties break **downward**: a note exactly between two scale tones (only possible in
    /// whole-tone and pentatonic contexts) resolves to the lower one every time, so the
    /// operation is deterministic and repeated application is idempotent.
    pub fn snap(&self, pitch: u8) -> u8 {
        if self.contains(pitch) {
            return pitch;
        }
        let classes = self.pitch_classes();
        let mut best = pitch as i32;
        let mut best_distance = i32::MAX;

        for &class in &classes {
            // Search the octave below, at and above so the nearest tone is found even
            // when it sits across an octave boundary from `pitch`.
            for octave in -1..=1 {
                let base = (pitch as i32 / OCTAVE + octave) * OCTAVE + class as i32;
                if !(0..=127).contains(&base) {
                    continue;
                }
                let distance = (base - pitch as i32).abs() * 2 + i32::from(base > pitch as i32);
                if distance < best_distance {
                    best_distance = distance;
                    best = base;
                }
            }
        }
        best.clamp(0, 127) as u8
    }

    /// Index of `pitch` within the scale, counting from the tonic, or `None` if the
    /// pitch is not in the key.
    pub fn degree_of(&self, pitch: u8) -> Option<usize> {
        let class = (pitch as i32 - self.tonic as i32).rem_euclid(OCTAVE) as u8;
        self.scale.intervals().iter().position(|&i| i == class)
    }

    /// Move a pitch by `steps` scale degrees, staying in the key.
    ///
    /// A pitch outside the key is snapped into it first, which is what makes
    /// `harmonize` well-defined for chromatic material rather than an error.
    pub fn transpose_degrees(&self, pitch: u8, steps: i32) -> u8 {
        let snapped = self.snap(pitch);
        let intervals = self.scale.intervals();
        let size = intervals.len() as i32;

        let Some(degree) = self.degree_of(snapped) else {
            return snapped;
        };

        let target = degree as i32 + steps;
        // Euclidean division keeps octave displacement correct for negative steps.
        let octave_shift = target.div_euclid(size);
        let index = target.rem_euclid(size) as usize;

        let base = snapped as i32 - intervals[degree] as i32;
        let result = base + intervals[index] as i32 + octave_shift * OCTAVE;
        result.clamp(0, 127) as u8
    }
}

// ---------------------------------------------------------------------------
// Chords
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    Major,
    Minor,
    Diminished,
    Augmented,
    Sus2,
    Sus4,
    Major6,
    Minor6,
    Major7,
    Minor7,
    Dominant7,
    MinorMajor7,
    HalfDiminished7,
    Diminished7,
    Dominant9,
    Major9,
    Minor9,
}

impl ChordQuality {
    /// Semitone offsets from the root, in close position.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            ChordQuality::Major => &[0, 4, 7],
            ChordQuality::Minor => &[0, 3, 7],
            ChordQuality::Diminished => &[0, 3, 6],
            ChordQuality::Augmented => &[0, 4, 8],
            ChordQuality::Sus2 => &[0, 2, 7],
            ChordQuality::Sus4 => &[0, 5, 7],
            ChordQuality::Major6 => &[0, 4, 7, 9],
            ChordQuality::Minor6 => &[0, 3, 7, 9],
            ChordQuality::Major7 => &[0, 4, 7, 11],
            ChordQuality::Minor7 => &[0, 3, 7, 10],
            ChordQuality::Dominant7 => &[0, 4, 7, 10],
            ChordQuality::MinorMajor7 => &[0, 3, 7, 11],
            ChordQuality::HalfDiminished7 => &[0, 3, 6, 10],
            ChordQuality::Diminished7 => &[0, 3, 6, 9],
            ChordQuality::Dominant9 => &[0, 4, 7, 10, 14],
            ChordQuality::Major9 => &[0, 4, 7, 11, 14],
            ChordQuality::Minor9 => &[0, 3, 7, 10, 14],
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            ChordQuality::Major => "",
            ChordQuality::Minor => "m",
            ChordQuality::Diminished => "dim",
            ChordQuality::Augmented => "aug",
            ChordQuality::Sus2 => "sus2",
            ChordQuality::Sus4 => "sus4",
            ChordQuality::Major6 => "6",
            ChordQuality::Minor6 => "m6",
            ChordQuality::Major7 => "maj7",
            ChordQuality::Minor7 => "m7",
            ChordQuality::Dominant7 => "7",
            ChordQuality::MinorMajor7 => "mMaj7",
            ChordQuality::HalfDiminished7 => "m7b5",
            ChordQuality::Diminished7 => "dim7",
            ChordQuality::Dominant9 => "9",
            ChordQuality::Major9 => "maj9",
            ChordQuality::Minor9 => "m9",
        }
    }

    /// Parse the part of a chord symbol after the root.
    ///
    /// Order matters: the longer spellings are matched before their prefixes, or `maj7`
    /// would match `m` and become a minor triad with junk left over.
    fn parse_suffix(text: &str) -> Option<ChordQuality> {
        let normalised: String = text
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect();
        let lower = normalised.to_ascii_lowercase();

        Some(match lower.as_str() {
            "" | "maj" | "major" | "m3" => ChordQuality::Major,
            "m" | "min" | "minor" => ChordQuality::Minor,
            "dim" | "o" | "°" => ChordQuality::Diminished,
            "aug" | "+" | "#5" => ChordQuality::Augmented,
            "sus2" => ChordQuality::Sus2,
            "sus" | "sus4" => ChordQuality::Sus4,
            "6" | "maj6" | "add6" => ChordQuality::Major6,
            "m6" | "min6" | "minor6" => ChordQuality::Minor6,
            "maj7" | "ma7" | "major7" | "Δ7" | "δ7" => ChordQuality::Major7,
            "m7" | "min7" | "minor7" => ChordQuality::Minor7,
            "7" | "dom7" | "dominant7" => ChordQuality::Dominant7,
            "mmaj7" | "minmaj7" | "mma7" => ChordQuality::MinorMajor7,
            "m7b5" | "ø" | "ø7" | "halfdim" | "halfdim7" => ChordQuality::HalfDiminished7,
            "dim7" | "o7" | "°7" => ChordQuality::Diminished7,
            "9" | "dom9" => ChordQuality::Dominant9,
            "maj9" | "major9" => ChordQuality::Major9,
            "m9" | "min9" | "minor9" => ChordQuality::Minor9,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chord {
    /// Pitch class of the root, 0–11.
    pub root: u8,
    pub quality: ChordQuality,
}

impl Chord {
    pub fn new(root: u8, quality: ChordQuality) -> Self {
        Chord { root: root % 12, quality }
    }

    /// Parse a chord symbol: `C`, `Am`, `F#m7b5`, `Bbmaj7`.
    pub fn parse(text: &str) -> Option<Chord> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }

        // The root is the leading letter plus any accidentals; a lone `b` after the
        // letter is ambiguous (`Cb` the note vs. a flat-five suffix), and reading it as
        // part of the root is the reading that matches how chord symbols are written.
        let mut split = 1;
        for (index, c) in text.char_indices().skip(1) {
            if matches!(c, '#' | '♯' | 'b' | '♭') {
                split = index + c.len_utf8();
            } else {
                break;
            }
        }
        let (root_text, suffix) = text.split_at(split);

        let root = parse_pitch_class(root_text)?;
        let quality = ChordQuality::parse_suffix(suffix)?;
        Some(Chord::new(root, quality))
    }

    pub fn name(&self) -> String {
        format!("{}{}", SHARP_NAMES[self.root as usize], self.quality.suffix())
    }

    /// Voice the chord upward from the lowest pitch at or above `from`.
    ///
    /// Notes past 127 are dropped rather than wrapped: a wrapped ninth would sound as a
    /// second below the root, which is not the chord that was asked for.
    pub fn voice(&self, from: u8) -> Vec<u8> {
        let base = {
            let class = self.root as i32;
            let mut candidate = (from as i32 / OCTAVE) * OCTAVE + class;
            if candidate < from as i32 {
                candidate += OCTAVE;
            }
            candidate
        };

        self.quality
            .intervals()
            .iter()
            .filter_map(|&i| {
                let pitch = base + i as i32;
                (0..=127).contains(&pitch).then_some(pitch as u8)
            })
            .collect()
    }
}

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

#[cfg(test)]
mod tests {
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
}
