//! Tests for the timing core, grouped by the behaviour under test.
//!
//! The fixtures are shared; the assertions are not. Each group covers one thing the audio
//! thread must get right — sample placement, event ordering, loop wrapping, transport
//! changes, timeline construction, the click, and the count-in.

mod count_in;
mod looping;
mod metronome;
mod ordering;
mod timeline;
mod timing;
mod transport;

use super::*;
use crate::model::Ticks;
use crate::model::{
    InstrumentRef, Note, Project, ProjectManifest, TimeSignature, Track, TrackMeta, DEFAULT_PPQ,
    SCHEMA_VERSION,
};

const SR: f64 = 48_000.0;

fn seq() -> Sequencer {
    Sequencer::new(SR, DEFAULT_PPQ, 120.0)
}

fn ev(tick: Ticks, kind: EventKind, pitch: u8) -> TimelineEvent {
    TimelineEvent { tick, kind, track: 0, pitch, velocity: 100, channel: 0 }
}

fn track(id: &str, notes: Vec<Note>, muted: bool, soloed: bool) -> Track {
    Track {
        meta: TrackMeta {
            id: id.into(), name: id.into(), channel: 0,
            instrument: InstrumentRef::BuiltInSampler, muted, soloed,
            color: "#fff".into(), key_hint: None,
        },
        notes,
        ppq: DEFAULT_PPQ,
    }
}

fn project(tracks: Vec<Track>) -> Project {
    Project {
        manifest: ProjectManifest {
            schema_version: SCHEMA_VERSION,
            id: "p".into(), name: "P".into(),
            tempo_bpm: 120.0, time_signature: TimeSignature::default(),
            ppq: DEFAULT_PPQ,
            tracks: tracks.iter().map(|t| t.meta.clone()).collect(),
            created_at_ms: 0, modified_at_ms: 0,
        },
        tracks,
    }
}

fn clicks(out: &[RenderedEvent]) -> Vec<(u32, u8)> {
    out.iter()
        .filter(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOn)
        .map(|e| (e.frame_offset, e.pitch))
        .collect()
}
