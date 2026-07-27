//! Tests for the command layer, grouped by the invariant under test.

mod basics;
mod clamping;
mod gestures;
mod history;
mod ordering;
mod transactions;

use super::*;
use crate::model::{InstrumentRef, Note, Track, TrackMeta, DEFAULT_PPQ};

fn note(pitch: u8, start: u32, dur: u32, vel: u8) -> Note {
    Note::new(pitch, start, dur, vel, 0).unwrap()
}

fn session(notes: Vec<Note>) -> EditSession {
    let mut track = Track::new(
        TrackMeta {
            id: "t".into(), name: "T".into(), channel: 0,
            instrument: InstrumentRef::BuiltInSampler, muted: false, soloed: false,
            color: "#fff".into(), key_hint: None,
        },
        DEFAULT_PPQ,
    );
    track.notes = notes;
    track.sort_notes();
    EditSession::new(vec![track])
}

fn pitches(s: &EditSession) -> Vec<u8> {
    s.tracks()[0].notes.iter().map(|n| n.pitch).collect()
}
