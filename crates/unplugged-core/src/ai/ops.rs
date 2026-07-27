//! Tools that change notes the workspace already has.
//!
//! Selection, pitch and time operations: every one of them maps over the current
//! selection and leaves the note count alone (`delete_notes` excepted, which only ever
//! shrinks it). Tools that invent new notes are in [`super::generate`].

use std::collections::HashSet;

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks};
use crate::music::{self, Key};

use super::rng::Rng;
use super::tools::{
    FitToScale, Humanize, InsertNotes, QuantizeArgs, SelectNotes, SetDuration, SetVelocityArgs,
    TransposeToKey,
};
use super::workspace::{NoteId, Workspace};

impl Workspace {
    pub(super) fn select(&mut self, args: &SelectNotes) -> Result<String> {
        let everything = args.all.unwrap_or(false)
            || (args.start_ticks.is_none()
                && args.end_ticks.is_none()
                && args.pitch_min.is_none()
                && args.pitch_max.is_none());

        self.selection = self
            .entries
            .iter()
            .filter(|entry| {
                if everything {
                    return true;
                }
                let note = &entry.note;
                // A note is inside a time range if it *starts* inside it. Overlap-based
                // selection would pull in a whole note sounding through bar 3 when the
                // user asked for bar 3, which is not what "the notes in bar 3" means.
                args.start_ticks.is_none_or(|t| note.start_ticks >= t)
                    && args.end_ticks.is_none_or(|t| note.start_ticks < t)
                    && args.pitch_min.is_none_or(|p| note.pitch >= p)
                    && args.pitch_max.is_none_or(|p| note.pitch <= p)
            })
            .map(|entry| entry.id)
            .collect();

        if self.selection.is_empty() {
            return Ok(format!(
                "Nothing matched. The track has {} notes.",
                self.entries.len()
            ));
        }
        Ok(self.describe("Selected", self.selection.len()))
    }

    pub(super) fn transpose(&mut self, semitones: i32) -> Result<String> {
        let count = self.map_selected(|note| {
            note.pitch = (note.pitch as i32 + semitones).clamp(0, 127) as u8;
        })?;
        Ok(self.describe("Transposed", count))
    }

    pub(super) fn transpose_to_key(&mut self, args: &TransposeToKey) -> Result<String> {
        let from = Key::parse(&args.from)
            .ok_or_else(|| CoreError::Invalid(format!("\"{}\" is not a key", args.from)))?;
        let to = Key::parse(&args.to)
            .ok_or_else(|| CoreError::Invalid(format!("\"{}\" is not a key", args.to)))?;

        // Shortest path between the tonics, so C→B goes down a semitone rather than up
        // eleven and dragging the part into another register.
        let raw = to.tonic as i32 - from.tonic as i32;
        let shift = (raw + 6).rem_euclid(music::OCTAVE) - 6;

        let count = self.map_selected(|note| {
            let moved = (note.pitch as i32 + shift).clamp(0, 127) as u8;
            note.pitch = to.snap(moved);
        })?;

        Ok(format!(
            "{} (shifted {shift:+} semitones and fitted to {}).",
            self.describe("Transposed", count),
            to.name()
        ))
    }

    pub(super) fn fit_to_scale(&mut self, args: &FitToScale) -> Result<String> {
        let key = self.key_or(Some(&args.key))?;
        let mut moved = 0usize;
        let count = self.map_selected(|note| {
            let snapped = key.snap(note.pitch);
            if snapped != note.pitch {
                moved += 1;
            }
            note.pitch = snapped;
        })?;
        Ok(format!(
            "{} ({moved} were off-scale).",
            self.describe("Fitted", count)
        ))
    }

    pub(super) fn quantize(&mut self, args: &QuantizeArgs) -> Result<String> {
        let grid = self.context.division_ticks(&args.grid)? as i64;
        let strength = args.strength.unwrap_or(1.0).clamp(0.0, 1.0);
        let durations = args.durations.unwrap_or(false);

        let snap = move |value: i64| -> i64 {
            let target = ((value as f64 / grid as f64).round() as i64) * grid;
            value + (((target - value) as f64) * strength).round() as i64
        };

        let count = self.map_selected(|note| {
            note.start_ticks = snap(note.start_ticks as i64).max(0) as Ticks;
            if durations {
                let end = snap(note.end_ticks() as i64);
                // A note quantized to zero length is unrepresentable in SMF, so the
                // floor is one grid step rather than one tick — a 64th-note stub where a
                // 16th was intended is not a useful result either.
                note.duration_ticks = (end - note.start_ticks as i64).max(grid) as Ticks;
            }
        })?;

        Ok(self.describe("Quantized", count))
    }

    pub(super) fn humanize(&mut self, args: &Humanize) -> Result<String> {
        let timing = args.timing_ticks.unwrap_or(self.context.ppq as u32 / 32) as i64;
        let velocity = args.velocity.unwrap_or(10) as i64;
        let mut rng = Rng::new(args.seed.unwrap_or(0x5EED_1234_ABCD_0001));

        let count = self.map_selected(|note| {
            if timing > 0 {
                let offset = rng.range(-timing, timing);
                note.start_ticks = (note.start_ticks as i64 + offset).max(0) as Ticks;
            }
            if velocity > 0 {
                let offset = rng.range(-velocity, velocity);
                note.velocity = (note.velocity as i64 + offset).clamp(1, 127) as u8;
            }
        })?;

        Ok(self.describe("Humanized", count))
    }

    pub(super) fn set_velocity(&mut self, args: &SetVelocityArgs) -> Result<String> {
        if args.velocity.is_none() && args.scale.is_none() {
            return Err(CoreError::Invalid(
                "set_velocity needs either velocity or scale".into(),
            ));
        }

        let positions = self.require_selection()?;
        let span = positions.len().saturating_sub(1).max(1) as f64;

        for (rank, &index) in positions.iter().enumerate() {
            let note = &mut self.entries[index].note;
            let mut value = note.velocity as f64;

            if let Some(target) = args.velocity {
                value = match args.ramp_to {
                    // Ranked by musical order, which `positions` already is.
                    Some(end) => {
                        target as f64 + (end as f64 - target as f64) * (rank as f64 / span)
                    }
                    None => target as f64,
                };
            }
            if let Some(scale) = args.scale {
                value *= scale;
            }
            note.velocity = (value.round() as i64).clamp(1, 127) as u8;
        }

        Ok(self.describe("Set the velocity of", positions.len()))
    }

    pub(super) fn set_duration(&mut self, args: &SetDuration) -> Result<String> {
        if args.division.is_none() && args.scale.is_none() && args.legato != Some(true) {
            return Err(CoreError::Invalid(
                "set_duration needs a division, a scale factor, or legato".into(),
            ));
        }

        let fixed = match &args.division {
            Some(text) => Some(self.context.division_ticks(text)?),
            None => None,
        };

        let positions = self.require_selection()?;

        if args.legato == Some(true) {
            // Reach to the next selected note's start. The last note keeps its length —
            // there is nothing to reach to, and stretching it to the track end would be
            // a surprise.
            for window in 0..positions.len().saturating_sub(1) {
                let start = self.entries[positions[window]].note.start_ticks;
                let next = self.entries[positions[window + 1]].note.start_ticks;
                if next > start {
                    self.entries[positions[window]].note.duration_ticks = next - start;
                }
            }
        }

        for &index in &positions {
            let note = &mut self.entries[index].note;
            if let Some(ticks) = fixed {
                note.duration_ticks = ticks;
            }
            if let Some(scale) = args.scale {
                note.duration_ticks =
                    ((note.duration_ticks as f64 * scale).round() as i64).clamp(1, Ticks::MAX as i64)
                        as Ticks;
            }
        }

        Ok(self.describe("Set the length of", positions.len()))
    }

    pub(super) fn insert_notes(&mut self, args: &InsertNotes) -> Result<String> {
        if args.notes.is_empty() {
            return Err(CoreError::Invalid("insert_notes was given no notes".into()));
        }
        self.guard_capacity(args.notes.len())?;

        let channel = self.context.channel;
        // Built first so a bad note anywhere in the batch fails before anything is
        // added — a half-inserted phrase is worse than none.
        let mut built = Vec::with_capacity(args.notes.len());
        for spec in &args.notes {
            built.push(Note::new(
                spec.pitch,
                spec.start_ticks,
                spec.duration_ticks.max(1),
                spec.velocity.unwrap_or(96).clamp(1, 127),
                channel,
            )?);
        }

        let count = built.len();
        let added: Vec<NoteId> = built.into_iter().map(|note| self.add(note)).collect();
        self.resort();
        self.selection = added;
        Ok(self.describe("Inserted", count))
    }

    pub(super) fn delete_notes(&mut self) -> Result<String> {
        let doomed: HashSet<NoteId> = self.selected_ids().into_iter().collect();
        if doomed.is_empty() {
            return Err(CoreError::Invalid("nothing is selected to delete".into()));
        }

        self.entries.retain(|entry| !doomed.contains(&entry.id));
        self.selection.clear();
        Ok(format!(
            "Deleted {} notes. Nothing is selected now; the track has {} notes.",
            doomed.len(),
            self.entries.len()
        ))
    }
}
