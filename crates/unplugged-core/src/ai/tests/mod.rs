//! Tests for the AI tool surface, grouped the way the module is.
//!
//! The fixtures live here because every group needs them; each group's assertions live in
//! its own file so no one of them grows past reading length.

mod commit;
mod generative;
mod pitch;
mod plumbing;
mod selection;
mod time;

use serde_json::json;

use super::*;
use crate::command::EditSession;
use crate::model::{
    InstrumentRef, Note, TimeSignature, Track, TrackMeta, DEFAULT_PPQ,
};
use crate::music::{self, Key};

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
