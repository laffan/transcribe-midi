//! The complete set of operations a model may request, as types.
//!
//! Deserialisation is the gate: a tool name or argument that does not appear here cannot
//! reach note data at all, because [`ToolCall`] is the only way into
//! [`Workspace::apply`](super::Workspace::apply). Adding a capability means adding a
//! variant here, a schema in [`super::schema`], and an arm in the dispatch — three places
//! that the compiler makes you visit.

use serde::{Deserialize, Serialize};
use crate::model::Ticks;

/// Every operation a model may perform, and the complete set.
///
/// Deserialised straight from the API's `{"name": ..., "input": {...}}` tool-use block,
/// so an unknown tool name or a malformed argument fails at the boundary with a message
/// the loop can hand back to the model — there is no path from model output to note data
/// that does not pass through this type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "name", content = "input", rename_all = "snake_case")]
pub enum ToolCall {
    SelectNotes(SelectNotes),
    Transpose(Transpose),
    TransposeToKey(TransposeToKey),
    FitToScale(FitToScale),
    Quantize(QuantizeArgs),
    Humanize(Humanize),
    SetVelocity(SetVelocityArgs),
    SetDuration(SetDuration),
    InsertNotes(InsertNotes),
    DeleteNotes(Empty),
    Duplicate(Duplicate),
    Invert(Invert),
    Retrograde(Empty),
    Arpeggiate(Arpeggiate),
    Harmonize(Harmonize),
    InsertChordProgression(InsertChordProgression),
}

/// Tools that take no arguments still receive `{}`, and serde needs somewhere to put it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Empty {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectNotes {
    pub all: Option<bool>,
    pub start_ticks: Option<Ticks>,
    pub end_ticks: Option<Ticks>,
    pub pitch_min: Option<u8>,
    pub pitch_max: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transpose {
    pub semitones: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransposeToKey {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitToScale {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantizeArgs {
    pub grid: String,
    /// 0.0 leaves notes alone, 1.0 snaps them fully. Defaults to 1.0.
    pub strength: Option<f64>,
    /// Quantize note ends as well as starts.
    pub durations: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Humanize {
    /// Maximum timing displacement in either direction.
    pub timing_ticks: Option<u32>,
    /// Maximum velocity displacement in either direction.
    pub velocity: Option<u8>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SetVelocityArgs {
    pub velocity: Option<u8>,
    pub scale: Option<f64>,
    /// With `velocity`, ramps linearly from it to this across the selection.
    pub ramp_to: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SetDuration {
    pub division: Option<String>,
    pub scale: Option<f64>,
    /// Extend each note to the start of the next one in the selection.
    pub legato: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteSpec {
    pub pitch: u8,
    pub start_ticks: Ticks,
    pub duration_ticks: Ticks,
    pub velocity: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertNotes {
    pub notes: Vec<NoteSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Duplicate {
    pub offset_ticks: Option<Ticks>,
    pub offset_bars: Option<f64>,
    pub count: Option<u32>,
    pub transpose: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Invert {
    /// Pitch to mirror around. Defaults to the lowest note in the selection.
    pub axis_pitch: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arpeggiate {
    pub division: String,
    /// `up`, `down`, `up_down`, `down_up`, or `as_played`. Defaults to `up`.
    pub pattern: Option<String>,
    /// Fraction of each step the note sounds for. Defaults to 0.9.
    pub gate: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Harmonize {
    /// Diatonic interval in scale degrees — 2 is a third above.
    pub degrees: Option<i32>,
    /// Fixed chromatic interval. Overrides `degrees` when given.
    pub semitones: Option<i32>,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertChordProgression {
    pub progression: String,
    pub key: Option<String>,
    pub bars_per_chord: Option<f64>,
    pub start_ticks: Option<Ticks>,
    /// Octave the chord roots are voiced from. Defaults to 3, below a typical melody.
    pub octave: Option<i32>,
}
