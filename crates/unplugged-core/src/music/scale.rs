//! The scales the tools understand, as interval sets.
//!
//! Intervals are semitones from the tonic, ascending, and never include the octave — the
//! length of the slice is the number of degrees, which is what diatonic arithmetic counts
//! against.

use serde::{Deserialize, Serialize};

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
