//! The AI tool surface: what a model is allowed to do to a track, and nothing else.
//!
//! Phase 6's model never sees a `Command` and never touches an [`EditSession`]. It calls
//! the tools defined here against a [`Workspace`] — a scratch copy of one track — and
//! when the loop finishes, the difference between the scratch copy and the original is
//! turned into **one** transaction that the user accepts or rejects. That gives three
//! things the spec asks for, structurally rather than by discipline:
//!
//! * every AI edit goes through the same command layer as every other edit;
//! * the whole conversation is a single undo step, however many tools were called;
//! * a preview diff exists for free, because the diff is how the transaction is built.
//!
//! Everything here is pure and deterministic — including `humanize`, which takes a seed.
//! A model's output is unpredictable enough without the code under it also being random.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::{Command, Transaction};
use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks, TimeSignature};
use crate::music::{self, Chord, Key};

/// Most notes one tool call may add. A model that asks for more has misunderstood the
/// request, and the error tells it so rather than locking the UI up drawing them.
pub const MAX_NOTES_PER_CALL: usize = 512;

/// Ceiling on the working copy. Well beyond any hand-edited track.
pub const MAX_WORKSPACE_NOTES: usize = 20_000;

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tool calls
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tool schemas
// ---------------------------------------------------------------------------

/// JSON schemas for the tool list sent to the API.
///
/// Written out rather than derived: the `description` strings are the actual interface
/// the model programs against, and they need to be readable by a person tuning them.
pub fn tool_definitions(context: &AiContext) -> Vec<serde_json::Value> {
    let ppq = context.ppq;
    let bar = context.bar_ticks();
    let timing =
        format!("Times are in ticks; {ppq} ticks is a quarter note and {bar} ticks is one bar.");

    let object = |properties: serde_json::Value, required: Vec<&str>| {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    };

    vec![
        json!({
            "name": "select_notes",
            "description": format!(
                "Choose which notes later tools act on. Filters combine: a call with \
                 both a time range and a pitch range selects only notes inside both. \
                 Call with no arguments, or all=true, to select the whole track. The \
                 selection starts as whatever the user had selected in the editor. {timing}"
            ),
            "input_schema": object(json!({
                "all": {"type": "boolean", "description": "Select every note in the track."},
                "start_ticks": {"type": "integer", "minimum": 0},
                "end_ticks": {"type": "integer", "minimum": 0},
                "pitch_min": {"type": "integer", "minimum": 0, "maximum": 127},
                "pitch_max": {"type": "integer", "minimum": 0, "maximum": 127},
            }), vec![]),
        }),
        json!({
            "name": "transpose",
            "description": "Shift the selected notes by a fixed number of semitones. \
                            Positive is up. Notes are clamped to the MIDI range.",
            "input_schema": object(json!({
                "semitones": {"type": "integer", "minimum": -127, "maximum": 127},
            }), vec!["semitones"]),
        }),
        json!({
            "name": "transpose_to_key",
            "description": "Move the selection from one key to another, shifting by the \
                            shorter direction and then fitting any note that lands \
                            outside the new key onto its nearest scale tone. Keys look \
                            like \"C major\", \"f# minor\", \"Eb dorian\".",
            "input_schema": object(json!({
                "from": {"type": "string"},
                "to": {"type": "string"},
            }), vec!["from", "to"]),
        }),
        json!({
            "name": "fit_to_scale",
            "description": "Move every selected note that is outside the key to its \
                            nearest scale tone, leaving in-key notes untouched. Ties \
                            resolve downward.",
            "input_schema": object(json!({
                "key": {"type": "string", "description": "For example \"D minor\"."},
            }), vec!["key"]),
        }),
        json!({
            "name": "quantize",
            "description": format!(
                "Snap the selection to a grid. Note values are written \"1/8\", \"1/16\", \
                 \"1/8t\" for triplets, \"1/8.\" for dotted; a bare number is a tick \
                 count. Strength below 1.0 moves notes part of the way, which keeps a \
                 performance feeling played. {timing}"
            ),
            "input_schema": object(json!({
                "grid": {"type": "string"},
                "strength": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                "durations": {"type": "boolean", "description": "Also snap note ends."},
            }), vec!["grid"]),
        }),
        json!({
            "name": "humanize",
            "description": "Add small random offsets to timing and velocity so a \
                            programmed part sounds played. Deterministic: the same seed \
                            gives the same result, and notes never move before tick 0.",
            "input_schema": object(json!({
                "timing_ticks": {"type": "integer", "minimum": 0, "maximum": 960},
                "velocity": {"type": "integer", "minimum": 0, "maximum": 64},
                "seed": {"type": "integer", "minimum": 0},
            }), vec![]),
        }),
        json!({
            "name": "set_velocity",
            "description": "Set the selection's velocity outright, scale it by a factor, \
                            or ramp it: give both velocity and ramp_to for a crescendo \
                            across the selection in time order.",
            "input_schema": object(json!({
                "velocity": {"type": "integer", "minimum": 1, "maximum": 127},
                "scale": {"type": "number", "minimum": 0.01, "maximum": 10.0},
                "ramp_to": {"type": "integer", "minimum": 1, "maximum": 127},
            }), vec![]),
        }),
        json!({
            "name": "set_duration",
            "description": format!(
                "Change how long the selected notes sound. Give a note value, a scale \
                 factor, or legato=true to stretch each note to the start of the next. \
                 {timing}"
            ),
            "input_schema": object(json!({
                "division": {"type": "string", "description": "For example \"1/8\"."},
                "scale": {"type": "number", "minimum": 0.01, "maximum": 32.0},
                "legato": {"type": "boolean"},
            }), vec![]),
        }),
        json!({
            "name": "insert_notes",
            "description": format!(
                "Add notes to the track. The new notes become the selection, so a \
                 following tool call operates on them. {timing} Middle C is pitch 60. \
                 At most {MAX_NOTES_PER_CALL} notes per call."
            ),
            "input_schema": object(json!({
                "notes": {
                    "type": "array",
                    "items": object(json!({
                        "pitch": {"type": "integer", "minimum": 0, "maximum": 127},
                        "start_ticks": {"type": "integer", "minimum": 0},
                        "duration_ticks": {"type": "integer", "minimum": 1},
                        "velocity": {"type": "integer", "minimum": 1, "maximum": 127},
                    }), vec!["pitch", "start_ticks", "duration_ticks"]),
                },
            }), vec!["notes"]),
        }),
        json!({
            "name": "delete_notes",
            "description": "Remove the selected notes.",
            "input_schema": object(json!({}), vec![]),
        }),
        json!({
            "name": "duplicate",
            "description": "Copy the selection later in time. Give an offset in ticks or \
                            bars; with no offset the copy lands immediately after the \
                            selection ends. The copies become the selection.",
            "input_schema": object(json!({
                "offset_ticks": {"type": "integer", "minimum": 0},
                "offset_bars": {"type": "number", "minimum": 0.0},
                "count": {"type": "integer", "minimum": 1, "maximum": 64},
                "transpose": {"type": "integer", "minimum": -127, "maximum": 127},
            }), vec![]),
        }),
        json!({
            "name": "invert",
            "description": "Mirror the selection's pitches around an axis, so rising \
                            intervals fall by the same amount. Defaults to the lowest \
                            selected note as the axis.",
            "input_schema": object(json!({
                "axis_pitch": {"type": "integer", "minimum": 0, "maximum": 127},
            }), vec![]),
        }),
        json!({
            "name": "retrograde",
            "description": "Reverse the selection in time within its own span. Pitches \
                            are unchanged; the last note becomes the first.",
            "input_schema": object(json!({}), vec![]),
        }),
        json!({
            "name": "arpeggiate",
            "description": "Break each chord in the selection into single notes, one per \
                            step, filling the chord's original length. Patterns: up, \
                            down, up_down, down_up, as_played.",
            "input_schema": object(json!({
                "division": {"type": "string", "description": "Step length, e.g. \"1/16\"."},
                "pattern": {
                    "type": "string",
                    "enum": ["up", "down", "up_down", "down_up", "as_played"],
                },
                "gate": {"type": "number", "minimum": 0.05, "maximum": 1.0},
            }), vec!["division"]),
        }),
        json!({
            "name": "harmonize",
            "description": "Add a second voice above or below each selected note. \
                            Diatonic by default — degrees=2 is a third in the key, \
                            degrees=-2 a third below. Use semitones for a fixed \
                            parallel interval instead. Both voices become the selection.",
            "input_schema": object(json!({
                "degrees": {"type": "integer", "minimum": -14, "maximum": 14},
                "semitones": {"type": "integer", "minimum": -36, "maximum": 36},
                "key": {"type": "string"},
            }), vec![]),
        }),
        json!({
            "name": "insert_chord_progression",
            "description": format!(
                "Write block chords into the track. The progression may be roman \
                 numerals (\"ii - V7 - I\") or chord symbols (\"Am | F | C | G\"), and \
                 roman numerals need a key. {timing} The chords become the selection, so \
                 arpeggiate can follow."
            ),
            "input_schema": object(json!({
                "progression": {"type": "string"},
                "key": {"type": "string"},
                "bars_per_chord": {"type": "number", "minimum": 0.125, "maximum": 16.0},
                "start_ticks": {"type": "integer", "minimum": 0},
                "octave": {"type": "integer", "minimum": -1, "maximum": 8},
            }), vec!["progression"]),
        }),
    ]
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

/// Identity for a note inside a workspace.
///
/// Notes have no id in the domain model — they are addressed by index — but indices
/// shift the moment anything is transposed into a new sort position, and a selection
/// that silently drifted onto different notes between two tool calls would be a very
/// hard bug to see. Ids below `original.len()` are original notes, at their original
/// index; anything higher was created during this conversation.
type NoteId = u64;

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: NoteId,
    note: Note,
}

/// A scratch copy of one track that tools operate on.
#[derive(Debug, Clone)]
pub struct Workspace {
    context: AiContext,
    original: Vec<Note>,
    entries: Vec<Entry>,
    selection: Vec<NoteId>,
    next_id: NoteId,
}

impl Workspace {
    /// `selection` is the editor's current selection, as indices into `notes`.
    pub fn new(context: AiContext, notes: &[Note], selection: &[usize]) -> Self {
        let entries: Vec<Entry> = notes
            .iter()
            .enumerate()
            .map(|(index, note)| Entry { id: index as NoteId, note: *note })
            .collect();

        // An empty editor selection means "the whole track": a prompt like "quantize
        // this" with nothing selected should act on everything, which is what every
        // other editor does and what the user plainly means.
        let selection: Vec<NoteId> = if selection.is_empty() {
            entries.iter().map(|e| e.id).collect()
        } else {
            selection
                .iter()
                .filter(|&&index| index < entries.len())
                .map(|&index| index as NoteId)
                .collect()
        };

        Workspace {
            context,
            original: notes.to_vec(),
            next_id: entries.len() as NoteId,
            entries,
            selection,
        }
    }

    pub fn context(&self) -> &AiContext {
        &self.context
    }

    pub fn notes(&self) -> Vec<Note> {
        self.entries.iter().map(|e| e.note).collect()
    }

    pub fn selection_len(&self) -> usize {
        self.selection.len()
    }

    /// Run one tool call, returning the text handed back to the model.
    pub fn apply(&mut self, call: &ToolCall) -> Result<String> {
        match call {
            ToolCall::SelectNotes(args) => self.select(args),
            ToolCall::Transpose(args) => self.transpose(args.semitones),
            ToolCall::TransposeToKey(args) => self.transpose_to_key(args),
            ToolCall::FitToScale(args) => self.fit_to_scale(args),
            ToolCall::Quantize(args) => self.quantize(args),
            ToolCall::Humanize(args) => self.humanize(args),
            ToolCall::SetVelocity(args) => self.set_velocity(args),
            ToolCall::SetDuration(args) => self.set_duration(args),
            ToolCall::InsertNotes(args) => self.insert_notes(args),
            ToolCall::DeleteNotes(_) => self.delete_notes(),
            ToolCall::Duplicate(args) => self.duplicate(args),
            ToolCall::Invert(args) => self.invert(args),
            ToolCall::Retrograde(_) => self.retrograde(),
            ToolCall::Arpeggiate(args) => self.arpeggiate(args),
            ToolCall::Harmonize(args) => self.harmonize(args),
            ToolCall::InsertChordProgression(args) => self.insert_progression(args),
        }
    }

    // -- internals ----------------------------------------------------------

    fn resort(&mut self) {
        // Stable, so two notes at the same start and pitch keep their relative order and
        // their ids stay put — otherwise the diff would report spurious changes.
        self.entries.sort_by_key(|e| e.note.order_key());
    }

    fn selected_ids(&self) -> Vec<NoteId> {
        let live: HashSet<NoteId> = self.entries.iter().map(|e| e.id).collect();
        self.selection
            .iter()
            .copied()
            .filter(|id| live.contains(id))
            .collect()
    }

    /// Positions of the selected notes, in musical order.
    fn selected_positions(&self) -> Vec<usize> {
        let selected: HashSet<NoteId> = self.selection.iter().copied().collect();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| selected.contains(&e.id))
            .map(|(index, _)| index)
            .collect()
    }

    fn require_selection(&self) -> Result<Vec<usize>> {
        let positions = self.selected_positions();
        if positions.is_empty() {
            return Err(CoreError::Invalid(
                "nothing is selected — call select_notes first".into(),
            ));
        }
        Ok(positions)
    }

    /// Edit every selected note in place, then restore sort order.
    fn map_selected(&mut self, mut f: impl FnMut(&mut Note)) -> Result<usize> {
        let positions = self.require_selection()?;
        for index in &positions {
            f(&mut self.entries[*index].note);
        }
        self.resort();
        Ok(positions.len())
    }

    fn add(&mut self, note: Note) -> NoteId {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Entry { id, note });
        id
    }

    fn guard_capacity(&self, adding: usize) -> Result<()> {
        if adding > MAX_NOTES_PER_CALL {
            return Err(CoreError::Invalid(format!(
                "that would add {adding} notes; at most {MAX_NOTES_PER_CALL} per call"
            )));
        }
        if self.entries.len() + adding > MAX_WORKSPACE_NOTES {
            return Err(CoreError::Invalid(format!(
                "the track would exceed {MAX_WORKSPACE_NOTES} notes"
            )));
        }
        Ok(())
    }

    fn key_or(&self, given: Option<&String>) -> Result<Key> {
        if let Some(text) = given {
            return Key::parse(text)
                .ok_or_else(|| CoreError::Invalid(format!("\"{text}\" is not a key")));
        }
        self.context.key.ok_or_else(|| {
            CoreError::Invalid(
                "no key given and the track has no key set — pass one, e.g. \"C major\"".into(),
            )
        })
    }

    /// Describe the selection, so the model can see what it just did.
    fn describe(&self, verb: &str, count: usize) -> String {
        let positions = self.selected_positions();
        if positions.is_empty() {
            return format!("{verb} {count} notes. Nothing is selected now.");
        }

        let notes: Vec<Note> = positions.iter().map(|&i| self.entries[i].note).collect();
        let low = notes.iter().map(|n| n.pitch).min().unwrap_or(0);
        let high = notes.iter().map(|n| n.pitch).max().unwrap_or(0);
        let start = notes.iter().map(|n| n.start_ticks).min().unwrap_or(0);
        let end = notes.iter().map(Note::end_ticks).max().unwrap_or(0);

        format!(
            "{verb} {count} notes. Selection is now {} notes, {}–{}, bars {}–{}. Track has {} notes.",
            notes.len(),
            music::pitch_name(low),
            music::pitch_name(high),
            self.context.position_label(start),
            self.context.position_label(end),
            self.entries.len(),
        )
    }

    // -- tools --------------------------------------------------------------

    fn select(&mut self, args: &SelectNotes) -> Result<String> {
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

    fn transpose(&mut self, semitones: i32) -> Result<String> {
        let count = self.map_selected(|note| {
            note.pitch = (note.pitch as i32 + semitones).clamp(0, 127) as u8;
        })?;
        Ok(self.describe("Transposed", count))
    }

    fn transpose_to_key(&mut self, args: &TransposeToKey) -> Result<String> {
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

    fn fit_to_scale(&mut self, args: &FitToScale) -> Result<String> {
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

    fn quantize(&mut self, args: &QuantizeArgs) -> Result<String> {
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

    fn humanize(&mut self, args: &Humanize) -> Result<String> {
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

    fn set_velocity(&mut self, args: &SetVelocityArgs) -> Result<String> {
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

    fn set_duration(&mut self, args: &SetDuration) -> Result<String> {
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

    fn insert_notes(&mut self, args: &InsertNotes) -> Result<String> {
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

    fn delete_notes(&mut self) -> Result<String> {
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

    fn duplicate(&mut self, args: &Duplicate) -> Result<String> {
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

    fn invert(&mut self, args: &Invert) -> Result<String> {
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

    fn retrograde(&mut self) -> Result<String> {
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

    fn arpeggiate(&mut self, args: &Arpeggiate) -> Result<String> {
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

    fn harmonize(&mut self, args: &Harmonize) -> Result<String> {
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

    fn insert_progression(&mut self, args: &InsertChordProgression) -> Result<String> {
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

    // -- committing ---------------------------------------------------------

    /// True when this note is not what it was at the same id in the original.
    fn is_dirty(&self, entry: &Entry) -> bool {
        match self.original.get(entry.id as usize) {
            Some(original) => *original != entry.note,
            None => true,
        }
    }

    /// What changed, for the preview the user accepts or rejects.
    pub fn diff(&self) -> NoteDiff {
        let mut added = Vec::new();
        let mut changed = Vec::new();
        let surviving: HashMap<NoteId, Note> = self
            .entries
            .iter()
            .filter(|e| (e.id as usize) < self.original.len())
            .map(|e| (e.id, e.note))
            .collect();

        for entry in &self.entries {
            match self.original.get(entry.id as usize) {
                Some(before) if *before != entry.note => changed.push(NoteChange {
                    before: *before,
                    after: entry.note,
                }),
                Some(_) => {}
                None => added.push(entry.note),
            }
        }

        let removed: Vec<Note> = self
            .original
            .iter()
            .enumerate()
            .filter(|(index, _)| !surviving.contains_key(&(*index as NoteId)))
            .map(|(_, note)| *note)
            .collect();

        added.sort_by_key(Note::order_key);
        changed.sort_by_key(|c| c.after.order_key());

        NoteDiff { added, removed, changed }
    }

    /// Turn the whole conversation into one transaction against `track`.
    ///
    /// Expressed as a delete followed by an insert rather than a `Replace`: `Replace`
    /// indices refer to the pre-command state, and a transaction that both replaced and
    /// deleted would need its indices to survive the reordering the replace itself
    /// causes. Delete-then-insert has no such coupling and is exactly equivalent.
    pub fn to_transaction(&self, track: usize, label: impl Into<String>) -> Transaction {
        let label = label.into();
        if self.diff().is_empty() {
            return Transaction::new(label, Vec::new());
        }

        let dirty: HashSet<NoteId> = self
            .entries
            .iter()
            .filter(|entry| self.is_dirty(entry))
            .map(|entry| entry.id)
            .collect();
        let surviving: HashSet<NoteId> = self.entries.iter().map(|e| e.id).collect();

        let mut commands = Vec::new();

        // Every original note that was removed *or* modified comes out; the modified
        // ones go back in below with their new values.
        let doomed: Vec<usize> = (0..self.original.len())
            .filter(|index| {
                let id = *index as NoteId;
                !surviving.contains(&id) || dirty.contains(&id)
            })
            .collect();
        if !doomed.is_empty() {
            commands.push(Command::Delete { track, indices: doomed });
        }

        let mut fresh: Vec<Note> = self
            .entries
            .iter()
            .filter(|entry| self.is_dirty(entry))
            .map(|entry| entry.note)
            .collect();
        fresh.sort_by_key(Note::order_key);
        if !fresh.is_empty() {
            commands.push(Command::Insert { track, notes: fresh });
        }

        Transaction::new(label, commands)
    }
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteChange {
    pub before: Note,
    pub after: Note,
}

/// The preview the user sees before anything is committed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteDiff {
    pub added: Vec<Note>,
    pub removed: Vec<Note>,
    pub changed: Vec<NoteChange>,
}

impl NoteDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// One line for the console and the accept/reject prompt.
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "No change".into();
        }
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!("{} added", self.added.len()));
        }
        if !self.removed.is_empty() {
            parts.push(format!("{} removed", self.removed.len()));
        }
        if !self.changed.is_empty() {
            parts.push(format!("{} changed", self.changed.len()));
        }
        parts.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------

/// xorshift64*. Not cryptographic and not trying to be — it exists so `humanize` gives
/// the same answer twice, which matters far more here than statistical quality.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift, so it is never allowed.
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[low, high]`.
    fn range(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        let span = (high - low + 1) as u64;
        low + (self.next_u64() % span) as i64
    }
}

// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::EditSession;
    use crate::model::{InstrumentRef, Track, TrackMeta, DEFAULT_PPQ};

    fn context() -> AiContext {
        AiContext {
            ppq: DEFAULT_PPQ,
            time_signature: TimeSignature::new(4, 4).unwrap(),
            tempo_bpm: 120.0,
            channel: 0,
            key: Some(Key::new(0, music::Scale::Major)),
        }
    }

    fn note(pitch: u8, start: u32, duration: u32) -> Note {
        Note::new(pitch, start, duration, 96, 0).unwrap()
    }

    fn workspace(notes: Vec<Note>) -> Workspace {
        Workspace::new(context(), &notes, &[])
    }

    fn call(json: serde_json::Value) -> ToolCall {
        serde_json::from_value(json).expect("tool call should deserialise")
    }

    // -- plumbing -----------------------------------------------------------

    #[test]
    fn every_tool_definition_deserialises_as_a_call() {
        // The schema list and the `ToolCall` enum are two descriptions of the same
        // interface. If a name is added to one and not the other, the model gets a tool
        // it cannot use and the failure only shows up mid-conversation, at runtime.
        let definitions = tool_definitions(&context());
        assert_eq!(definitions.len(), 16, "the spec lists sixteen tools");

        for definition in definitions {
            let name = definition["name"].as_str().unwrap().to_string();
            let schema = &definition["input_schema"];

            // Build a minimal input from the schema's required fields.
            let mut input = serde_json::Map::new();
            for required in schema["required"].as_array().unwrap() {
                let field = required.as_str().unwrap();
                let kind = schema["properties"][field]["type"].as_str().unwrap();
                input.insert(
                    field.to_string(),
                    match kind {
                        "integer" | "number" => json!(1),
                        "boolean" => json!(true),
                        "array" => json!([{"pitch": 60, "start_ticks": 0, "duration_ticks": 120}]),
                        _ => json!("1/8"),
                    },
                );
            }

            let value = json!({"name": name, "input": input});
            assert!(
                serde_json::from_value::<ToolCall>(value.clone()).is_ok(),
                "tool {name} has no matching ToolCall variant: {value}"
            );
        }
    }

    #[test]
    fn an_unknown_tool_is_refused_at_the_boundary() {
        let value = json!({"name": "delete_the_project", "input": {}});
        assert!(serde_json::from_value::<ToolCall>(value).is_err());
    }

    #[test]
    fn division_parsing() {
        let context = context();
        assert_eq!(context.division_ticks("1/4").unwrap(), 480);
        assert_eq!(context.division_ticks("1/8").unwrap(), 240);
        assert_eq!(context.division_ticks("1/16").unwrap(), 120);
        assert_eq!(context.division_ticks("1/8t").unwrap(), 160, "triplet eighth");
        assert_eq!(context.division_ticks("1/8.").unwrap(), 360, "dotted eighth");
        assert_eq!(context.division_ticks("8").unwrap(), 8, "bare ticks");
        assert!(context.division_ticks("1/5").is_err());
        assert!(context.division_ticks("nonsense").is_err());
        assert!(context.division_ticks("0").is_err());
    }

    #[test]
    fn bar_and_beat_labels_count_from_one() {
        let context = context();
        assert_eq!(context.position_label(0), "1.1");
        assert_eq!(context.position_label(480), "1.2");
        assert_eq!(context.position_label(1920), "2.1");
    }

    // -- selection ----------------------------------------------------------

    #[test]
    fn an_empty_editor_selection_means_the_whole_track() {
        let workspace = Workspace::new(context(), &[note(60, 0, 480), note(64, 480, 480)], &[]);
        assert_eq!(workspace.selection_len(), 2);
    }

    #[test]
    fn the_editor_selection_is_respected() {
        let workspace = Workspace::new(context(), &[note(60, 0, 480), note(64, 480, 480)], &[1]);
        assert_eq!(workspace.selection_len(), 1);
    }

    #[test]
    fn selection_filters_combine() {
        let mut workspace = workspace(vec![
            note(60, 0, 480),
            note(72, 480, 480),
            note(64, 1920, 480),
        ]);

        workspace
            .apply(&call(json!({
                "name": "select_notes",
                "input": {"start_ticks": 0, "end_ticks": 960, "pitch_min": 65},
            })))
            .unwrap();

        assert_eq!(workspace.selection_len(), 1, "only the high note in bar 1");
        workspace
            .apply(&call(json!({"name": "transpose", "input": {"semitones": 1}})))
            .unwrap();

        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(notes[1].pitch, 73);
        assert_eq!(notes[0].pitch, 60, "the others are untouched");
    }

    #[test]
    fn selecting_by_time_uses_note_starts() {
        // A whole note in bar 1 is not "a note in bar 2" just because it is still
        // sounding there.
        let mut workspace = workspace(vec![note(60, 0, 1920 * 2)]);
        workspace
            .apply(&call(json!({
                "name": "select_notes",
                "input": {"start_ticks": 1920, "end_ticks": 3840},
            })))
            .unwrap();
        assert_eq!(workspace.selection_len(), 0);
    }

    // -- pitch tools --------------------------------------------------------

    #[test]
    fn transpose_clamps_rather_than_wrapping() {
        let mut workspace = workspace(vec![note(120, 0, 480)]);
        workspace
            .apply(&call(json!({"name": "transpose", "input": {"semitones": 24}})))
            .unwrap();
        assert_eq!(workspace.notes()[0].pitch, 127);
    }

    #[test]
    fn transpose_to_key_takes_the_shorter_direction() {
        let mut workspace = workspace(vec![note(60, 0, 480)]);
        workspace
            .apply(&call(json!({
                "name": "transpose_to_key",
                "input": {"from": "C major", "to": "B major"},
            })))
            .unwrap();
        // Down one semitone, not up eleven.
        assert_eq!(workspace.notes()[0].pitch, 59);
    }

    #[test]
    fn fit_to_scale_leaves_in_key_notes_alone() {
        let mut workspace = workspace(vec![note(60, 0, 480), note(61, 480, 480)]);
        workspace
            .apply(&call(json!({"name": "fit_to_scale", "input": {"key": "C major"}})))
            .unwrap();
        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(notes[0].pitch, 60);
        assert_eq!(notes[1].pitch, 60, "C# folded onto C");
    }

    #[test]
    fn invert_mirrors_around_the_lowest_note_by_default() {
        let mut workspace =
            workspace(vec![note(60, 0, 480), note(64, 480, 480), note(67, 960, 480)]);
        workspace.apply(&call(json!({"name": "invert", "input": {}}))).unwrap();

        let mut by_start = workspace.notes();
        by_start.sort_by_key(|n| n.start_ticks);
        // Around 60: 60 stays, 64 → 56, 67 → 53.
        assert_eq!(
            by_start.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            vec![60, 56, 53]
        );
    }

    #[test]
    fn harmonize_adds_a_diatonic_third() {
        let mut workspace = workspace(vec![note(60, 0, 480), note(62, 480, 480)]);
        workspace.apply(&call(json!({"name": "harmonize", "input": {}}))).unwrap();

        let mut pitches: Vec<u8> = workspace.notes().iter().map(|n| n.pitch).collect();
        pitches.sort_unstable();
        // C+E and D+F — the third is major above C and minor above D, which is the whole
        // point of harmonising in the key rather than by a fixed interval.
        assert_eq!(pitches, vec![60, 62, 64, 65]);
    }

    #[test]
    fn harmonize_below_with_a_fixed_interval() {
        let mut workspace = workspace(vec![note(60, 0, 480)]);
        workspace
            .apply(&call(json!({"name": "harmonize", "input": {"semitones": -12}})))
            .unwrap();
        let mut pitches: Vec<u8> = workspace.notes().iter().map(|n| n.pitch).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, vec![48, 60]);
    }

    #[test]
    fn harmonize_without_a_key_anywhere_is_an_error() {
        let mut context = context();
        context.key = None;
        let mut workspace = Workspace::new(context, &[note(60, 0, 480)], &[]);
        let error = workspace
            .apply(&call(json!({"name": "harmonize", "input": {}})))
            .unwrap_err();
        assert!(error.to_string().contains("no key"), "{error}");
    }

    // -- time tools ---------------------------------------------------------

    #[test]
    fn quantize_snaps_to_the_grid() {
        let mut workspace = workspace(vec![note(60, 13, 480), note(62, 235, 480)]);
        workspace
            .apply(&call(json!({"name": "quantize", "input": {"grid": "1/8"}})))
            .unwrap();
        let mut starts: Vec<u32> = workspace.notes().iter().map(|n| n.start_ticks).collect();
        starts.sort_unstable();
        assert_eq!(starts, vec![0, 240]);
    }

    #[test]
    fn partial_quantize_moves_part_of_the_way() {
        let mut workspace = workspace(vec![note(60, 100, 480)]);
        workspace
            .apply(&call(json!({
                "name": "quantize",
                "input": {"grid": "1/4", "strength": 0.5},
            })))
            .unwrap();
        assert_eq!(workspace.notes()[0].start_ticks, 50, "halfway back to 0");
    }

    #[test]
    fn humanize_is_deterministic_and_stays_in_range() {
        let source = vec![note(60, 0, 480), note(62, 480, 480), note(64, 960, 480)];
        let mut a = workspace(source.clone());
        let mut b = workspace(source);
        let tool = call(json!({
            "name": "humanize",
            "input": {"timing_ticks": 30, "velocity": 20, "seed": 7},
        }));

        a.apply(&tool).unwrap();
        b.apply(&tool).unwrap();
        assert_eq!(a.notes(), b.notes(), "same seed, same result");

        for note in a.notes() {
            assert!((1..=127).contains(&note.velocity));
            assert!(note.duration_ticks > 0);
        }
    }

    #[test]
    fn humanize_never_moves_a_note_before_zero() {
        let mut workspace = workspace(vec![note(60, 2, 480)]);
        workspace
            .apply(&call(json!({
                "name": "humanize",
                "input": {"timing_ticks": 200, "seed": 3},
            })))
            .unwrap();
        // `start_ticks` is unsigned, so a negative result would have wrapped to two
        // billion rather than failing — which is exactly why this is asserted.
        assert!(workspace.notes()[0].start_ticks < 480);
    }

    #[test]
    fn retrograde_reverses_within_the_span() {
        let mut workspace = workspace(vec![
            note(60, 0, 480),
            note(62, 480, 480),
            note(64, 960, 960),
        ]);
        workspace.apply(&call(json!({"name": "retrograde", "input": {}}))).unwrap();

        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(notes[0].pitch, 64, "the last note is now first");
        assert_eq!(notes[0].start_ticks, 0);
        assert_eq!(notes[2].pitch, 60);
        assert_eq!(notes[2].end_ticks(), 1920, "the span is preserved");
    }

    #[test]
    fn retrograde_twice_is_the_identity() {
        let source = vec![note(60, 0, 480), note(62, 480, 240), note(64, 960, 960)];
        let mut workspace = workspace(source.clone());
        let tool = call(json!({"name": "retrograde", "input": {}}));
        workspace.apply(&tool).unwrap();
        workspace.apply(&tool).unwrap();
        assert_eq!(workspace.notes(), source);
    }

    #[test]
    fn legato_reaches_the_next_note() {
        let mut workspace = workspace(vec![note(60, 0, 100), note(62, 480, 100)]);
        workspace
            .apply(&call(json!({"name": "set_duration", "input": {"legato": true}})))
            .unwrap();
        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(notes[0].duration_ticks, 480);
        assert_eq!(notes[1].duration_ticks, 100, "the last note is left alone");
    }

    #[test]
    fn set_velocity_ramps_across_the_selection() {
        let mut workspace =
            workspace(vec![note(60, 0, 480), note(62, 480, 480), note(64, 960, 480)]);
        workspace
            .apply(&call(json!({
                "name": "set_velocity",
                "input": {"velocity": 40, "ramp_to": 100},
            })))
            .unwrap();
        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(
            notes.iter().map(|n| n.velocity).collect::<Vec<_>>(),
            vec![40, 70, 100]
        );
    }

    #[test]
    fn set_velocity_with_no_arguments_is_refused() {
        let mut workspace = workspace(vec![note(60, 0, 480)]);
        assert!(workspace
            .apply(&call(json!({"name": "set_velocity", "input": {}})))
            .is_err());
    }

    // -- generative tools ---------------------------------------------------

    #[test]
    fn duplicate_defaults_to_the_next_bar() {
        let mut workspace = workspace(vec![note(60, 0, 480)]);
        workspace.apply(&call(json!({"name": "duplicate", "input": {}}))).unwrap();

        let mut starts: Vec<u32> = workspace.notes().iter().map(|n| n.start_ticks).collect();
        starts.sort_unstable();
        assert_eq!(starts, vec![0, 1920]);
    }

    #[test]
    fn duplicate_transposes_each_copy_further() {
        let mut workspace = workspace(vec![note(60, 0, 480)]);
        workspace
            .apply(&call(json!({
                "name": "duplicate",
                "input": {"offset_bars": 1, "count": 2, "transpose": 12},
            })))
            .unwrap();

        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            vec![60, 72, 84]
        );
    }

    #[test]
    fn duplicate_leaves_the_copies_selected() {
        let mut workspace = workspace(vec![note(60, 0, 480)]);
        workspace
            .apply(&call(json!({"name": "duplicate", "input": {"offset_bars": 1}})))
            .unwrap();
        // Chaining is the point: duplicate then transpose should move only the copy.
        workspace
            .apply(&call(json!({"name": "transpose", "input": {"semitones": 5}})))
            .unwrap();

        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(notes[0].pitch, 60, "the original is untouched");
        assert_eq!(notes[1].pitch, 65);
    }

    #[test]
    fn arpeggiate_breaks_a_chord_into_steps() {
        // A C major triad lasting one bar.
        let mut workspace = workspace(vec![
            note(60, 0, 1920),
            note(64, 0, 1920),
            note(67, 0, 1920),
        ]);
        workspace
            .apply(&call(json!({
                "name": "arpeggiate",
                "input": {"division": "1/8", "pattern": "up"},
            })))
            .unwrap();

        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(notes.len(), 8, "eight eighth notes in a 4/4 bar");
        assert_eq!(
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            vec![60, 64, 67, 60, 64, 67, 60, 64],
            "the pattern cycles"
        );
        assert!(
            notes.iter().all(|n| n.duration_ticks < 240),
            "gated shorter than the step"
        );
    }

    #[test]
    fn arpeggiate_up_down_does_not_repeat_the_turning_points() {
        let mut workspace = workspace(vec![note(60, 0, 960), note(64, 0, 960), note(67, 0, 960)]);
        workspace
            .apply(&call(json!({
                "name": "arpeggiate",
                "input": {"division": "1/8", "pattern": "up_down"},
            })))
            .unwrap();

        let mut notes = workspace.notes();
        notes.sort_by_key(|n| n.start_ticks);
        assert_eq!(
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            vec![60, 64, 67, 64]
        );
    }

    #[test]
    fn a_bad_arpeggio_pattern_is_refused() {
        let mut workspace = workspace(vec![note(60, 0, 960)]);
        assert!(workspace
            .apply(&call(json!({
                "name": "arpeggiate",
                "input": {"division": "1/8", "pattern": "sideways"},
            })))
            .is_err());
    }

    #[test]
    fn chord_progressions_land_on_bar_lines() {
        let mut workspace = workspace(vec![]);
        let summary = workspace
            .apply(&call(json!({
                "name": "insert_chord_progression",
                "input": {"progression": "I - V - vi - IV", "key": "C major"},
            })))
            .unwrap();

        let notes = workspace.notes();
        assert_eq!(notes.len(), 12, "four triads");

        let mut starts: Vec<u32> = notes.iter().map(|n| n.start_ticks).collect();
        starts.dedup();
        assert_eq!(starts, vec![0, 1920, 3840, 5760]);
        assert!(summary.contains('C') && summary.contains('G'), "{summary}");
    }

    #[test]
    fn a_progression_can_be_arpeggiated_afterwards() {
        let mut workspace = workspace(vec![]);
        workspace
            .apply(&call(json!({
                "name": "insert_chord_progression",
                "input": {"progression": "ii - V - I", "key": "C major", "bars_per_chord": 1},
            })))
            .unwrap();
        workspace
            .apply(&call(json!({"name": "arpeggiate", "input": {"division": "1/8"}})))
            .unwrap();

        assert_eq!(workspace.notes().len(), 24, "three bars of eighths");
    }

    #[test]
    fn insert_notes_is_bounded() {
        let mut workspace = workspace(vec![]);
        let many: Vec<serde_json::Value> = (0..MAX_NOTES_PER_CALL + 1)
            .map(|i| json!({"pitch": 60, "start_ticks": i * 10, "duration_ticks": 5}))
            .collect();

        let error = workspace
            .apply(&call(json!({"name": "insert_notes", "input": {"notes": many}})))
            .unwrap_err();
        assert!(error.to_string().contains("at most"), "{error}");
        assert!(workspace.notes().is_empty(), "nothing was added");
    }

    #[test]
    fn a_tool_that_fails_leaves_the_workspace_alone() {
        let source = vec![note(60, 0, 480)];
        let mut workspace = workspace(source.clone());
        assert!(workspace
            .apply(&call(json!({"name": "quantize", "input": {"grid": "1/7"}})))
            .is_err());
        assert_eq!(workspace.notes(), source);
    }

    // -- diff and commit ----------------------------------------------------

    #[test]
    fn an_untouched_workspace_produces_no_transaction() {
        let workspace = workspace(vec![note(60, 0, 480)]);
        assert!(workspace.diff().is_empty());
        assert!(workspace.to_transaction(0, "AI edit").is_empty());
    }

    #[test]
    fn the_diff_separates_added_removed_and_changed() {
        let mut workspace = workspace(vec![note(60, 0, 480), note(62, 480, 480)]);

        // Change one note...
        workspace
            .apply(&call(json!({
                "name": "select_notes",
                "input": {"pitch_min": 60, "pitch_max": 60},
            })))
            .unwrap();
        workspace
            .apply(&call(json!({"name": "transpose", "input": {"semitones": 2}})))
            .unwrap();

        // ...remove the other...
        workspace
            .apply(&call(json!({
                "name": "select_notes",
                "input": {"start_ticks": 480, "end_ticks": 960},
            })))
            .unwrap();
        workspace.apply(&call(json!({"name": "delete_notes", "input": {}}))).unwrap();

        // ...and add a third.
        workspace
            .apply(&call(json!({
                "name": "insert_notes",
                "input": {"notes": [{"pitch": 67, "start_ticks": 960, "duration_ticks": 480}]},
            })))
            .unwrap();

        let diff = workspace.diff();
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].before.pitch, 60);
        assert_eq!(diff.changed[0].after.pitch, 62);
        assert_eq!(diff.summary(), "1 added, 1 removed, 1 changed");
    }

    /// The property that matters: whatever the tools did, applying the transaction to a
    /// real session must reproduce the workspace exactly, and undoing it must restore
    /// the original — in one step, however many tools ran.
    fn assert_round_trip(source: Vec<Note>, calls: &[serde_json::Value]) {
        let mut workspace = Workspace::new(context(), &source, &[]);
        for value in calls {
            workspace.apply(&call(value.clone())).unwrap();
        }

        let mut track = Track::new(
            TrackMeta {
                id: "t".into(),
                name: "T".into(),
                channel: 0,
                instrument: InstrumentRef::BuiltInSampler,
                muted: false,
                soloed: false,
                color: "#fff".into(),
                key_hint: None,
            },
            DEFAULT_PPQ,
        );
        track.notes = source.clone();

        let mut session = EditSession::new(vec![track]);
        let transaction = workspace.to_transaction(0, "AI edit");
        let expected = {
            let mut notes = workspace.notes();
            notes.sort_by_key(Note::order_key);
            notes
        };

        session.apply(transaction).unwrap();
        assert_eq!(
            session.tracks()[0].notes,
            expected,
            "the commit did not match the preview"
        );

        session.undo().unwrap();
        assert_eq!(
            session.tracks()[0].notes,
            source,
            "undo did not restore the original"
        );
        assert!(!session.can_undo(), "the whole edit must be a single undo step");
    }

    #[test]
    fn a_single_tool_round_trips() {
        assert_round_trip(
            vec![note(60, 0, 480), note(64, 480, 480)],
            &[json!({"name": "transpose", "input": {"semitones": 7}})],
        );
    }

    #[test]
    fn a_long_chain_of_tools_round_trips_as_one_undo_step() {
        assert_round_trip(
            vec![note(60, 13, 470), note(64, 500, 460), note(67, 950, 500)],
            &[
                json!({"name": "quantize", "input": {"grid": "1/8"}}),
                json!({"name": "harmonize", "input": {"degrees": 2, "key": "C major"}}),
                json!({"name": "set_velocity", "input": {"velocity": 60, "ramp_to": 120}}),
                json!({"name": "duplicate", "input": {"offset_bars": 1}}),
                json!({"name": "transpose", "input": {"semitones": -5}}),
                json!({"name": "humanize", "input": {"timing_ticks": 12, "seed": 99}}),
            ],
        );
    }

    #[test]
    fn deleting_everything_round_trips() {
        assert_round_trip(
            vec![note(60, 0, 480), note(64, 480, 480)],
            &[json!({"name": "delete_notes", "input": {}})],
        );
    }

    #[test]
    fn generating_into_an_empty_track_round_trips() {
        assert_round_trip(
            vec![],
            &[
                json!({
                    "name": "insert_chord_progression",
                    "input": {"progression": "i - VI - III - VII", "key": "A minor"},
                }),
                json!({"name": "arpeggiate", "input": {"division": "1/16", "pattern": "up_down"}}),
            ],
        );
    }

    #[test]
    fn a_transposition_that_reorders_notes_round_trips() {
        // Moving the lower note above the upper one changes sort position, which is the
        // case that makes index-based selection unsafe.
        let source = vec![note(60, 0, 480), note(62, 0, 480)];
        let mut workspace = Workspace::new(context(), &source, &[0]);
        workspace
            .apply(&call(json!({"name": "transpose", "input": {"semitones": 10}})))
            .unwrap();

        let diff = workspace.diff();
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].after.pitch, 70);
        assert!(diff.added.is_empty() && diff.removed.is_empty());
    }

    #[test]
    fn the_notes_table_is_truncated() {
        let notes: Vec<Note> = (0..10).map(|i| note(60, i * 480, 480)).collect();
        let table = notes_table(&notes, &context(), 3);
        assert!(table.contains("and 7 more notes"), "{table}");
        assert_eq!(table.lines().count(), 5, "header, three rows, the elision");
    }
}
