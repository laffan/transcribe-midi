//! Tools that add notes the performance never contained.
//!
//! Duplication, inversion, retrograde, arpeggios, harmony lines and chord progressions.
//! Every one of them is bounded by [`Workspace::guard_capacity`] before it allocates, so
//! a model that asks for a thousand bars of arpeggio gets an error it can read rather
//! than a frozen editor.

use std::collections::{BTreeMap, HashSet};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks};
use crate::music::{self, Chord, Key};

use super::tools::{Arpeggiate, Duplicate, Harmonize, InsertChordProgression, Invert};
use super::workspace::{NoteId, Workspace};

impl Workspace {
    pub(super) fn duplicate(&mut self, args: &Duplicate) -> Result<String> {
        let positions = self.require_selection()?;
        let count = args.count.unwrap_or(1).clamp(1, 64) as usize;

        let source: Vec<Note> = positions.iter().map(|&i| self.entries[i].note).collect();
        let start = source.iter().map(|n| n.start_ticks).min().unwrap_or(0);
        let end = source.iter().map(Note::end_ticks).max().unwrap_or(0);

        let offset = match (args.offset_ticks, args.offset_bars) {
            (Some(ticks), _) => ticks,
            (None, Some(bars)) => (bars * self.context.bar_ticks() as f64).round().max(0.0) as Ticks,
            // With no offset, place the copy after the selection, rounded up to a bar
            // line. Stacking copies on top of the original is never the intent.
            (None, None) => {
                let bar = self.context.bar_ticks().max(1);
                let length = end.saturating_sub(start).max(1);
                length.div_ceil(bar) * bar
            }
        };

        if offset == 0 {
            return Err(CoreError::Invalid(
                "a duplicate at offset 0 would sit on top of the original".into(),
            ));
        }

        self.guard_capacity(source.len() * count)?;

        let transpose = args.transpose.unwrap_or(0);
        let mut added = Vec::with_capacity(source.len() * count);
        for copy in 1..=count {
            for note in &source {
                let mut copied = *note;
                copied.start_ticks = note.start_ticks.saturating_add(offset * copy as Ticks);
                copied.pitch = (note.pitch as i32 + transpose * copy as i32).clamp(0, 127) as u8;
                added.push(self.add(copied));
            }
        }

        self.resort();
        self.selection = added;
        Ok(self.describe("Duplicated into", source.len() * count))
    }

    pub(super) fn invert(&mut self, args: &Invert) -> Result<String> {
        let positions = self.require_selection()?;
        let axis = match args.axis_pitch {
            Some(pitch) => pitch as i32,
            None => positions
                .iter()
                .map(|&i| self.entries[i].note.pitch as i32)
                .min()
                .unwrap_or(60),
        };

        let count = self.map_selected(|note| {
            note.pitch = (2 * axis - note.pitch as i32).clamp(0, 127) as u8;
        })?;

        Ok(format!(
            "{} (around {}).",
            self.describe("Inverted", count),
            music::pitch_name(axis.clamp(0, 127) as u8)
        ))
    }

    pub(super) fn retrograde(&mut self) -> Result<String> {
        let positions = self.require_selection()?;
        let source: Vec<Note> = positions.iter().map(|&i| self.entries[i].note).collect();

        let start = source.iter().map(|n| n.start_ticks).min().unwrap_or(0);
        let end = source.iter().map(Note::end_ticks).max().unwrap_or(0);

        // Mirror each note about the selection's span: a note ending at the span's end
        // now starts at its beginning. Durations are preserved, which is what makes this
        // the musical retrograde rather than a reversal of the note list.
        for (rank, &index) in positions.iter().enumerate() {
            let note = source[rank];
            let mirrored = end.saturating_sub(note.end_ticks()).saturating_add(start);
            self.entries[index].note.start_ticks = mirrored;
        }

        self.resort();
        Ok(self.describe("Reversed", positions.len()))
    }

    pub(super) fn arpeggiate(&mut self, args: &Arpeggiate) -> Result<String> {
        let step = self.context.division_ticks(&args.division)?;
        let gate = args.gate.unwrap_or(0.9).clamp(0.05, 1.0);
        let pattern = args.pattern.as_deref().unwrap_or("up").to_ascii_lowercase();
        if !matches!(
            pattern.as_str(),
            "up" | "down" | "up_down" | "updown" | "down_up" | "downup" | "as_played"
        ) {
            return Err(CoreError::Invalid(format!(
                "\"{pattern}\" is not an arpeggio pattern"
            )));
        }

        let positions = self.require_selection()?;

        // Group by start tick: notes that begin together are the chord to break up.
        let mut chords: BTreeMap<Ticks, Vec<Note>> = BTreeMap::new();
        for &index in &positions {
            let note = self.entries[index].note;
            chords.entry(note.start_ticks).or_default().push(note);
        }

        let mut generated: Vec<Note> = Vec::new();
        for (start, mut chord) in chords {
            let span = chord.iter().map(Note::end_ticks).max().unwrap_or(start) - start;
            let steps = (span / step).max(1) as usize;

            chord.sort_by_key(|n| n.pitch);
            let order: Vec<Note> = match pattern.as_str() {
                "as_played" => chord.clone(),
                "down" => chord.iter().rev().copied().collect(),
                "up_down" | "updown" | "down_up" | "downup" => {
                    let mut ascending = chord.clone();
                    if pattern.starts_with("down") {
                        ascending.reverse();
                    }
                    let mut both = ascending.clone();
                    // Skip the turning points so the extremes are not played twice in a
                    // row — that is the difference between an arpeggio and a stutter.
                    both.extend(
                        ascending
                            .iter()
                            .rev()
                            .skip(1)
                            .take(ascending.len().saturating_sub(2))
                            .copied(),
                    );
                    both
                }
                _ => chord.clone(),
            };

            if order.is_empty() {
                continue;
            }

            for index in 0..steps {
                let source = order[index % order.len()];
                let at = start + step * index as Ticks;
                let duration = ((step as f64 * gate).round() as Ticks).max(1);
                generated.push(Note {
                    start_ticks: at,
                    duration_ticks: duration,
                    ..source
                });
            }
        }

        if generated.is_empty() {
            return Err(CoreError::Invalid("nothing to arpeggiate".into()));
        }
        self.guard_capacity(generated.len())?;

        let doomed: HashSet<NoteId> = self.selected_ids().into_iter().collect();
        self.entries.retain(|entry| !doomed.contains(&entry.id));

        let count = generated.len();
        let added: Vec<NoteId> = generated.into_iter().map(|note| self.add(note)).collect();
        self.resort();
        self.selection = added;

        Ok(self.describe("Arpeggiated into", count))
    }

    pub(super) fn harmonize(&mut self, args: &Harmonize) -> Result<String> {
        let positions = self.require_selection()?;
        let source: Vec<Note> = positions.iter().map(|&i| self.entries[i].note).collect();
        self.guard_capacity(source.len())?;

        let voice: Vec<Note> = if let Some(semitones) = args.semitones {
            source
                .iter()
                .map(|note| Note {
                    pitch: (note.pitch as i32 + semitones).clamp(0, 127) as u8,
                    ..*note
                })
                .collect()
        } else {
            let key = self.key_or(args.key.as_ref())?;
            // A third is two scale degrees, and that is the interval a musician means by
            // "harmonise it" far more often than any other, so it is the default.
            let degrees = args.degrees.unwrap_or(2);
            source
                .iter()
                .map(|note| Note {
                    pitch: key.transpose_degrees(note.pitch, degrees),
                    ..*note
                })
                .collect()
        };

        let mut selection = self.selected_ids();
        let count = voice.len();
        for note in voice {
            selection.push(self.add(note));
        }

        self.resort();
        self.selection = selection;
        Ok(self.describe("Harmonized", count))
    }

    pub(super) fn insert_progression(&mut self, args: &InsertChordProgression) -> Result<String> {
        // Chord symbols do not need a key, so a missing one is only fatal once a roman
        // numeral turns up — `parse_progression` reports that itself.
        let key = self
            .key_or(args.key.as_ref())
            .unwrap_or_else(|_| Key::new(0, music::Scale::Major));
        let chords: Vec<Chord> = music::parse_progression(&args.progression, &key)?;

        let bar = self.context.bar_ticks().max(1);
        let per_chord = ((args.bars_per_chord.unwrap_or(1.0).clamp(0.125, 16.0)) * bar as f64)
            .round()
            .max(1.0) as Ticks;

        let octave = args.octave.unwrap_or(3).clamp(-1, 8);
        // MIDI octave numbering puts C-1 at pitch 0, so C3 is (3 + 1) * 12 = 48.
        let floor = ((octave + 1) * 12).clamp(0, 127) as u8;
        let start = args.start_ticks.unwrap_or(0);

        let mut generated = Vec::new();
        for (index, chord) in chords.iter().enumerate() {
            let at = start + per_chord * index as Ticks;
            for pitch in chord.voice(floor) {
                generated.push(Note::new(pitch, at, per_chord, 84, self.context.channel)?);
            }
        }

        if generated.is_empty() {
            return Err(CoreError::Invalid("the progression produced no notes".into()));
        }
        self.guard_capacity(generated.len())?;

        let count = generated.len();
        let names: Vec<String> = chords.iter().map(Chord::name).collect();
        let added: Vec<NoteId> = generated.into_iter().map(|note| self.add(note)).collect();
        self.resort();
        self.selection = added;

        Ok(format!(
            "{} ({}).",
            self.describe("Inserted", count),
            names.join(" – ")
        ))
    }
}
