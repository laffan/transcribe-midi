//! Tests for SMF conversion, grouped by the direction data is moving.

mod import;
mod rescale;
mod round_trip;
mod type1;

use std::path::PathBuf;

use midly::num::{u15, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use super::*;
use crate::model::{
    InstrumentRef, Note, Project, ProjectManifest, TimeSignature, Track, TrackMeta, DEFAULT_PPQ,
    SCHEMA_VERSION,
};

fn p() -> PathBuf {
    PathBuf::from("test.mid")
}

fn note(pitch: u8, start: u32, dur: u32, vel: u8, ch: u8) -> Note {
    Note::new(pitch, start, dur, vel, ch).unwrap()
}

fn track(name: &str, notes: Vec<Note>) -> Track {
    Track {
        meta: TrackMeta {
            id: name.into(), name: name.into(), channel: 0,
            instrument: InstrumentRef::BuiltInSampler, muted: false, soloed: false,
            color: "#fff".into(), key_hint: None,
        },
        notes,
        ppq: DEFAULT_PPQ,
    }
}

fn project(tracks: Vec<Track>, tempo: f64, ts: TimeSignature) -> Project {
    Project {
        manifest: ProjectManifest {
            schema_version: SCHEMA_VERSION,
            id: "p".into(),
            name: "Test Project".into(),
            tempo_bpm: tempo,
            time_signature: ts,
            ppq: DEFAULT_PPQ,
            tracks: tracks.iter().map(|t| t.meta.clone()).collect(),
            created_at_ms: 0,
            modified_at_ms: 0,
        },
        tracks,
    }
}
