//! A tonic plus a scale, and the questions worth asking of one.
//!
//! `snap` and `transpose_degrees` are the two that matter: snapping is how "fit to E
//! minor" is implemented, and stepping by degrees is how "up a third" means a third *in
//! the key* rather than four semitones.

use serde::{Deserialize, Serialize};

use super::pitch::{parse_pitch_class, SHARP_NAMES};
use super::scale::Scale;
use super::OCTAVE;

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
