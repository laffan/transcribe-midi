//! Chord qualities, the symbols they answer to, and how a chord is voiced.
//!
//! Voicing is separate from identity on purpose: a [`Chord`] is a root and a quality with
//! no octave, and `voice` is the only thing that decides where the notes actually land.

use serde::{Deserialize, Serialize};

use super::pitch::{parse_pitch_class, SHARP_NAMES};
use super::OCTAVE;

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
    pub(super) fn parse_suffix(text: &str) -> Option<ChordQuality> {
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
