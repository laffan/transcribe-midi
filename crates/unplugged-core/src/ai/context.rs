//! What the model is told about the project, and how it is told.
//!
//! The model talks in bars and beats because that is how a musician describes an edit;
//! everything below the API boundary counts ticks. [`AiContext`] is the only place that
//! conversion lives, so a prompt and the notes it acts on can never disagree about where
//! bar 3 starts.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks, TimeSignature};
use crate::music::{self, Key};

/// What the model needs to know about the project to talk in bars and beats.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AiContext {
    pub ppq: u16,
    pub time_signature: TimeSignature,
    pub tempo_bpm: f64,
    /// Default channel for notes the model creates.
    pub channel: u8,
    /// The track's key hint, if it has one. Tools that need a key and were not given
    /// one fall back to this before failing.
    pub key: Option<Key>,
}

impl AiContext {
    pub fn bar_ticks(&self) -> Ticks {
        self.time_signature.bar_ticks(self.ppq)
    }

    /// Resolve a note-value string against this project's PPQ.
    ///
    /// Accepts `1/4`, `1/8t` (triplet), `1/8.` (dotted), or a bare tick count. Bare
    /// ticks are allowed because the model is told the PPQ and sometimes it is simply
    /// easier for it to say `320`.
    pub fn division_ticks(&self, text: &str) -> Result<Ticks> {
        let text = text.trim().to_ascii_lowercase();
        if text.is_empty() {
            return Err(CoreError::Invalid("no note value given".into()));
        }

        if let Ok(ticks) = text.parse::<u32>() {
            return if ticks == 0 {
                Err(CoreError::Invalid("a note value of 0 ticks is meaningless".into()))
            } else {
                Ok(ticks)
            };
        }

        let mut body = text.as_str();
        let mut triplet = false;
        let mut dotted = false;
        loop {
            if let Some(rest) = body.strip_suffix('t') {
                triplet = true;
                body = rest;
            } else if let Some(rest) = body.strip_suffix('.') {
                dotted = true;
                body = rest;
            } else {
                break;
            }
        }

        let denominator: u32 = body
            .strip_prefix("1/")
            .unwrap_or(body)
            .parse()
            .map_err(|_| CoreError::Invalid(format!("\"{text}\" is not a note value")))?;

        if denominator == 0 || !denominator.is_power_of_two() || denominator > 64 {
            return Err(CoreError::Invalid(format!(
                "\"{text}\" is not a note value — use 1/1 to 1/64, optionally with t or ."
            )));
        }

        // A whole note is four quarters; everything else divides down from there.
        let whole = self.ppq as u32 * 4;
        let mut ticks = whole / denominator;
        if dotted {
            ticks = ticks * 3 / 2;
        }
        if triplet {
            ticks = ticks * 2 / 3;
        }

        if ticks == 0 {
            Err(CoreError::Invalid(format!(
                "\"{text}\" is shorter than one tick at {} PPQ",
                self.ppq
            )))
        } else {
            Ok(ticks)
        }
    }

    /// Human-readable bar:beat for a tick position, 1-based as musicians count.
    pub fn position_label(&self, tick: Ticks) -> String {
        let bar_ticks = self.bar_ticks().max(1);
        let beat_ticks = (self.ppq as u32 * 4 / self.time_signature.denominator as u32).max(1);
        let bar = tick / bar_ticks;
        let beat = (tick % bar_ticks) / beat_ticks;
        format!("{}.{}", bar + 1, beat + 1)
    }
}

/// Render notes as a compact table for the model's context.
///
/// Sent instead of JSON: it is a fraction of the tokens for the same information, and a
/// model reads a bar-numbered table more reliably than a list of raw tick counts.
pub fn notes_table(notes: &[Note], context: &AiContext, limit: usize) -> String {
    if notes.is_empty() {
        return "The track is empty.".into();
    }

    let mut lines =
        vec!["index | pitch | bar.beat | start_ticks | length | velocity".to_string()];
    for (index, note) in notes.iter().take(limit).enumerate() {
        lines.push(format!(
            "{index} | {} | {} | {} | {} | {}",
            music::pitch_name(note.pitch),
            context.position_label(note.start_ticks),
            note.start_ticks,
            note.duration_ticks,
            note.velocity,
        ));
    }
    if notes.len() > limit {
        lines.push(format!("… and {} more notes not shown.", notes.len() - limit));
    }
    lines.join("\n")
}
