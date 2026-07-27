//! The JSON schemas advertised to the API.
//!
//! Split from [`super::tools`] because these strings are prose, not types: the
//! `description` fields are the actual interface a model programs against, and they get
//! tuned by reading them as documentation rather than as code.

use serde_json::json;

use super::context::AiContext;
use super::MAX_NOTES_PER_CALL;

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
