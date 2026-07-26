//! The in-memory domain model.
//!
//! Nothing in here touches the filesystem or any platform API — that keeps it testable
//! on any host, which matters because the Apple-target build cannot be exercised in CI.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Musical time, in ticks, relative to the start of the track.
///
/// `u32` at 480 PPQ overflows at roughly 51 days of 120bpm music. Not a concern.
pub type Ticks = u32;

/// Pulses per quarter note. 480 divides cleanly by 2, 3, 4, 5, 6 and 8, so every grid
/// the piano roll offers (including triplets, which Phase 8's notation view needs) lands
/// on an integer tick.
pub const DEFAULT_PPQ: u16 = 480;

pub const MIN_PPQ: u16 = 24;
pub const MAX_PPQ: u16 = 15360;

pub const MIN_TEMPO: f64 = 20.0;
pub const MAX_TEMPO: f64 = 300.0;
pub const DEFAULT_TEMPO: f64 = 120.0;

pub const MAX_NAME_LEN: usize = 128;

/// The manifest schema version written into `project.json`.
///
/// Present from the first release so that a future format change has somewhere to hook a
/// migration. Loading refuses anything newer than this rather than guessing.
pub const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Note
// ---------------------------------------------------------------------------

/// A single note. This is the atom the whole editor operates on.
///
/// Deliberately `Copy` and free of any id: notes are identified by position within a
/// track's ordered list. Phase 3's command layer addresses them by index, and an
/// identity field would be a second source of truth to keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// MIDI pitch, 0–127. 60 is middle C.
    pub pitch: u8,
    pub start_ticks: Ticks,
    /// Always greater than zero — a zero-length note is unrepresentable in SMF
    /// (note-on and note-off at the same tick is ambiguous) and meaningless in the UI.
    pub duration_ticks: Ticks,
    /// MIDI velocity, 1–127. Zero is not allowed: a note-on with velocity 0 *is* a
    /// note-off in the MIDI spec, so a zero-velocity note would silently vanish on save.
    pub velocity: u8,
    /// MIDI channel, 0–15.
    pub channel: u8,
}

impl Note {
    pub fn new(pitch: u8, start_ticks: Ticks, duration_ticks: Ticks, velocity: u8, channel: u8) -> Result<Self> {
        let note = Note { pitch, start_ticks, duration_ticks, velocity, channel };
        note.validate()?;
        Ok(note)
    }

    pub fn validate(&self) -> Result<()> {
        if self.pitch > 127 {
            return Err(CoreError::PitchOutOfRange(self.pitch as u16));
        }
        if self.channel > 15 {
            return Err(CoreError::ChannelOutOfRange(self.channel as u16));
        }
        if self.velocity == 0 || self.velocity > 127 {
            // Clamped rather than rejected on import; rejected when constructed directly.
            return Err(CoreError::PitchOutOfRange(self.velocity as u16));
        }
        if self.duration_ticks == 0 {
            return Err(CoreError::ZeroDuration);
        }
        Ok(())
    }

    #[inline]
    pub fn end_ticks(&self) -> Ticks {
        self.start_ticks.saturating_add(self.duration_ticks)
    }

    /// Sort key. Ordering by `(start, pitch)` gives a stable, musically sensible order
    /// and makes note lookup at a given tick a binary search.
    #[inline]
    pub fn order_key(&self) -> (Ticks, u8) {
        (self.start_ticks, self.pitch)
    }
}

// ---------------------------------------------------------------------------
// Time signature
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u8,
    /// Note value that gets the beat: 1, 2, 4, 8, 16, 32 or 64.
    pub denominator: u8,
}

impl TimeSignature {
    pub fn new(numerator: u8, denominator: u8) -> Result<Self> {
        let ts = TimeSignature { numerator, denominator };
        ts.validate()?;
        Ok(ts)
    }

    pub fn validate(&self) -> Result<()> {
        let denom_ok = matches!(self.denominator, 1 | 2 | 4 | 8 | 16 | 32 | 64);
        if self.numerator == 0 || self.numerator > 64 || !denom_ok {
            return Err(CoreError::InvalidTimeSignature(self.numerator, self.denominator));
        }
        Ok(())
    }

    /// Length of one bar in ticks at the given PPQ.
    pub fn bar_ticks(&self, ppq: u16) -> Ticks {
        // ppq is per quarter note; a unit of `denominator` is 4/denominator quarter notes.
        (ppq as u32 * 4 * self.numerator as u32) / self.denominator as u32
    }
}

impl Default for TimeSignature {
    fn default() -> Self {
        TimeSignature { numerator: 4, denominator: 4 }
    }
}

// ---------------------------------------------------------------------------
// Instrument
// ---------------------------------------------------------------------------

/// Which instrument renders a track.
///
/// Serialized tagged so Phase 9 can add an `AudioUnit { component_id, state }` variant
/// without invalidating projects written by earlier versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstrumentRef {
    /// The bundled `AVAudioUnitSampler`. Per the spec this is the test instrument, not
    /// a feature — it stays swappable.
    #[default]
    BuiltInSampler,
}

// ---------------------------------------------------------------------------
// Track
// ---------------------------------------------------------------------------

/// Per-track settings, stored in `project.json`. Note data lives in the track's `.mid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    pub id: String,
    pub name: String,
    /// Default MIDI channel for notes recorded or drawn into this track.
    pub channel: u8,
    #[serde(default)]
    pub instrument: InstrumentRef,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub soloed: bool,
    /// Hex colour used by the piano roll and track list.
    pub color: String,
    /// User-declared key hint. Phase 8 uses this for enharmonic spelling; the spec is
    /// explicit that key is declared, never inferred.
    #[serde(default)]
    pub key_hint: Option<String>,
}

impl TrackMeta {
    /// A fresh track for position `index`, with the defaults a new track gets.
    ///
    /// Lives here rather than in the store because tracks are now created in two places:
    /// on disk by `ProjectStore::add_track`, and in memory when an AI suggestion writes a
    /// new part. Two constructors would drift.
    pub fn for_index(index: usize) -> Self {
        TrackMeta {
            id: format!("track-{}", index + 1),
            name: format!("Track {}", index + 1),
            channel: (index % 16) as u8,
            instrument: InstrumentRef::BuiltInSampler,
            muted: false,
            soloed: false,
            color: color_for_index(index).to_string(),
            key_hint: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.channel > 15 {
            return Err(CoreError::ChannelOutOfRange(self.channel as u16));
        }
        Ok(())
    }

    /// Path of this track's SMF, relative to the project directory.
    pub fn relative_midi_path(&self) -> String {
        format!("tracks/{}.mid", self.id)
    }
}

/// A track with its notes loaded. This is what the editor manipulates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    #[serde(flatten)]
    pub meta: TrackMeta,
    /// Always sorted by `Note::order_key`. Use the mutators below rather than pushing
    /// directly, or call `sort_notes` afterwards.
    pub notes: Vec<Note>,
    /// Copy of the project PPQ, stamped on at load time.
    ///
    /// The spec lists PPQ as part of the track model, so it is exposed here. It is *not*
    /// independently editable: tracks in one project must share a PPQ or they will not
    /// play together, so `project.json` holds the canonical value.
    pub ppq: u16,
}

impl Track {
    pub fn new(meta: TrackMeta, ppq: u16) -> Self {
        Track { meta, notes: Vec::new(), ppq }
    }

    pub fn sort_notes(&mut self) {
        self.notes.sort_by_key(|n| n.order_key());
    }

    /// Insert preserving sort order.
    pub fn insert_note(&mut self, note: Note) -> Result<()> {
        note.validate()?;
        let idx = self
            .notes
            .partition_point(|n| n.order_key() <= note.order_key());
        self.notes.insert(idx, note);
        Ok(())
    }

    /// Last tick at which anything sounds. Zero for an empty track.
    pub fn length_ticks(&self) -> Ticks {
        self.notes.iter().map(|n| n.end_ticks()).max().unwrap_or(0)
    }

    pub fn is_sorted(&self) -> bool {
        self.notes.windows(2).all(|w| w[0].order_key() <= w[1].order_key())
    }
}

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

/// The contents of `project.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub schema_version: u32,
    /// Stable identifier and directory name. Does not change when the project is renamed.
    pub id: String,
    pub name: String,
    pub tempo_bpm: f64,
    pub time_signature: TimeSignature,
    pub ppq: u16,
    pub tracks: Vec<TrackMeta>,
    /// Milliseconds since the Unix epoch. Stored as a number rather than a formatted
    /// timestamp so there is no timezone or parsing ambiguity between Rust and the
    /// frontend — JS reads it straight into `new Date(ms)`.
    pub created_at_ms: u64,
    pub modified_at_ms: u64,
}

impl ProjectManifest {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(CoreError::EmptyName);
        }
        if self.name.chars().count() > MAX_NAME_LEN {
            return Err(CoreError::NameTooLong(self.name.chars().count()));
        }
        if !(MIN_TEMPO..=MAX_TEMPO).contains(&self.tempo_bpm) || !self.tempo_bpm.is_finite() {
            return Err(CoreError::TempoOutOfRange(self.tempo_bpm));
        }
        if !(MIN_PPQ..=MAX_PPQ).contains(&self.ppq) {
            return Err(CoreError::InvalidPpq(self.ppq));
        }
        self.time_signature.validate()?;
        for track in &self.tracks {
            track.validate()?;
        }
        Ok(())
    }

    pub fn track(&self, track_id: &str) -> Result<&TrackMeta> {
        self.tracks
            .iter()
            .find(|t| t.id == track_id)
            .ok_or_else(|| CoreError::TrackNotFound(track_id.to_string()))
    }
}

/// A fully loaded project: manifest plus every track's notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub manifest: ProjectManifest,
    pub tracks: Vec<Track>,
}

/// Lightweight row for the project picker — avoids parsing every SMF just to list projects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub tempo_bpm: f64,
    pub time_signature: TimeSignature,
    pub track_count: usize,
    pub created_at_ms: u64,
    pub modified_at_ms: u64,
}

/// A project directory that could not be read, reported alongside the ones that could.
///
/// One unparseable project must not make the whole picker unusable, so listing collects
/// these rather than failing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectListError {
    pub id: String,
    pub message: String,
}

/// Result of listing the projects directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectListing {
    pub projects: Vec<ProjectSummary>,
    pub errors: Vec<ProjectListError>,
}

impl From<&ProjectManifest> for ProjectSummary {
    fn from(m: &ProjectManifest) -> Self {
        ProjectSummary {
            id: m.id.clone(),
            name: m.name.clone(),
            tempo_bpm: m.tempo_bpm,
            time_signature: m.time_signature,
            track_count: m.tracks.len(),
            created_at_ms: m.created_at_ms,
            modified_at_ms: m.modified_at_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// Track colours
// ---------------------------------------------------------------------------

/// Cycled through as tracks are added, so a new project looks deliberate rather than
/// uniformly grey. Values are the accent ramp from the design tokens.
pub const TRACK_COLORS: [&str; 8] = [
    "#5b8dd9", "#d97757", "#6cb08a", "#c07ac0",
    "#d9a441", "#5aa8b0", "#b06a6a", "#8a86d9",
];

pub fn color_for_index(index: usize) -> &'static str {
    TRACK_COLORS[index % TRACK_COLORS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_rejects_out_of_range_values() {
        assert!(Note::new(128, 0, 480, 100, 0).is_err(), "pitch 128 must be rejected");
        assert!(Note::new(60, 0, 480, 100, 16).is_err(), "channel 16 must be rejected");
        assert!(Note::new(60, 0, 0, 100, 0).is_err(), "zero duration must be rejected");
        assert!(Note::new(60, 0, 480, 0, 0).is_err(), "zero velocity must be rejected");
        assert!(Note::new(127, 0, 1, 1, 15).is_ok(), "boundary values must be accepted");
    }

    #[test]
    fn insert_note_keeps_list_sorted() {
        let meta = TrackMeta {
            id: "t1".into(), name: "Track 1".into(), channel: 0,
            instrument: InstrumentRef::BuiltInSampler, muted: false, soloed: false,
            color: "#fff".into(), key_hint: None,
        };
        let mut track = Track::new(meta, DEFAULT_PPQ);

        for (pitch, start) in [(64, 960), (60, 0), (67, 480), (62, 0)] {
            track.insert_note(Note::new(pitch, start, 240, 100, 0).unwrap()).unwrap();
        }

        assert!(track.is_sorted());
        // (0,60), (0,62), (480,67), (960,64)
        assert_eq!(
            track.notes.iter().map(|n| (n.start_ticks, n.pitch)).collect::<Vec<_>>(),
            vec![(0, 60), (0, 62), (480, 67), (960, 64)]
        );
    }

    #[test]
    fn track_length_is_the_furthest_note_end_not_the_last_start() {
        let meta = TrackMeta {
            id: "t1".into(), name: "T".into(), channel: 0,
            instrument: InstrumentRef::BuiltInSampler, muted: false, soloed: false,
            color: "#fff".into(), key_hint: None,
        };
        let mut track = Track::new(meta, DEFAULT_PPQ);
        // A long note starting early outlasts a short note starting later.
        track.insert_note(Note::new(60, 0, 1920, 100, 0).unwrap()).unwrap();
        track.insert_note(Note::new(72, 480, 240, 100, 0).unwrap()).unwrap();
        assert_eq!(track.length_ticks(), 1920);
    }

    #[test]
    fn bar_length_accounts_for_the_denominator() {
        let ppq = 480;
        assert_eq!(TimeSignature::new(4, 4).unwrap().bar_ticks(ppq), 1920);
        assert_eq!(TimeSignature::new(3, 4).unwrap().bar_ticks(ppq), 1440);
        assert_eq!(TimeSignature::new(6, 8).unwrap().bar_ticks(ppq), 1440);
        assert_eq!(TimeSignature::new(7, 8).unwrap().bar_ticks(ppq), 1680);
    }

    #[test]
    fn time_signature_rejects_non_power_of_two_denominators() {
        assert!(TimeSignature::new(4, 6).is_err());
        assert!(TimeSignature::new(0, 4).is_err());
        assert!(TimeSignature::new(6, 8).is_ok());
    }
}
