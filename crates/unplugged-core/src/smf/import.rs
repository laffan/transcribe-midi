//! A whole file the user dragged in, as tracks they can choose between.
//!
//! Distinct from [`read`](super::read): that reads one of our own storage files, this
//! reads an arbitrary SMF whose PPQ, track count and naming are all somebody else's
//! decision. [`rescale_ppq`] is the part that matters — 96, 384 and 960 are all common,
//! and landing the material at the wrong tempo silently would be worse than failing.

use std::collections::HashMap;
use std::path::Path;

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks};

use super::read::clamp_ticks;
use super::tempo::micros_per_quarter_to_bpm;

/// One track's worth of imported material.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedTrack {
    pub name: Option<String>,
    pub notes: Vec<Note>,
}

/// Everything an imported file tells us.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedFile {
    pub tracks: Vec<ImportedTrack>,
    pub ppq: Option<u16>,
    pub tempo_bpm: Option<f64>,
    pub time_signature: Option<crate::model::TimeSignature>,
}

impl ImportedFile {
    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(|t| t.notes.len()).sum()
    }
}

/// Parse an SMF for import, keeping each chunk as a separate track.
///
/// Format 0 files yield a single track. Format 1 files keep their chunks, minus any that
/// carry no notes — the conductor track becomes tempo and time-signature metadata rather
/// than an empty track cluttering the project.
pub fn parse_smf_for_import(bytes: &[u8], path_for_errors: &Path) -> Result<ImportedFile> {
    let smf = Smf::parse(bytes).map_err(|source| CoreError::MidiParse {
        path: path_for_errors.to_path_buf(),
        source,
    })?;

    let ppq = match smf.header.timing {
        Timing::Metrical(ppq) => Some(ppq.as_int()),
        Timing::Timecode(..) => None,
    };

    let mut tempo_bpm = None;
    let mut time_signature = None;
    let mut tracks = Vec::new();

    for chunk in &smf.tracks {
        let mut name = None;
        let mut abs: u64 = 0;
        let mut pending: HashMap<(u8, u8), Vec<(u64, u8)>> = HashMap::new();
        let mut notes = Vec::new();

        for event in chunk {
            abs += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(bytes)) => {
                    if name.is_none() {
                        let text = String::from_utf8_lossy(bytes).trim().to_string();
                        if !text.is_empty() {
                            name = Some(text);
                        }
                    }
                }
                // First tempo and time signature win. A tempo map is out of scope for
                // v1 — the model has a single tempo — so later changes are dropped
                // rather than silently averaged.
                TrackEventKind::Meta(MetaMessage::Tempo(micros)) => {
                    tempo_bpm.get_or_insert_with(|| micros_per_quarter_to_bpm(micros.as_int()));
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(numerator, denominator_pow2, ..)) => {
                    if time_signature.is_none() {
                        let denominator = 1u32 << denominator_pow2.min(6);
                        time_signature =
                            crate::model::TimeSignature::new(numerator, denominator as u8).ok();
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let channel = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            pending
                                .entry((channel, key.as_int()))
                                .or_default()
                                .push((abs, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            let queue = pending.entry((channel, key.as_int())).or_default();
                            if queue.is_empty() {
                                continue;
                            }
                            let (start, velocity) = queue.remove(0);
                            notes.push(Note {
                                pitch: key.as_int(),
                                start_ticks: clamp_ticks(start),
                                duration_ticks: clamp_ticks(abs.saturating_sub(start).max(1)),
                                velocity,
                                channel,
                            });
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        for ((channel, pitch), queue) in pending {
            for (start, velocity) in queue {
                notes.push(Note {
                    pitch,
                    start_ticks: clamp_ticks(start),
                    duration_ticks: clamp_ticks(abs.saturating_sub(start).max(1)),
                    velocity,
                    channel,
                });
            }
        }

        if notes.is_empty() {
            continue; // conductor track, or an empty chunk
        }
        notes.sort_by_key(Note::order_key);
        tracks.push(ImportedTrack { name, notes });
    }

    Ok(ImportedFile { tracks, ppq, tempo_bpm, time_signature })
}

/// Rescale note times from one PPQ to another.
///
/// Imported files routinely use a different resolution from the project (96, 192, 960
/// are all common). Without this the material would land at the wrong tempo — silently,
/// which is worse than failing.
pub fn rescale_ppq(notes: &mut [Note], from_ppq: u16, to_ppq: u16) {
    if from_ppq == to_ppq || from_ppq == 0 || to_ppq == 0 {
        return;
    }
    let ratio = to_ppq as f64 / from_ppq as f64;
    for note in notes.iter_mut() {
        note.start_ticks = (note.start_ticks as f64 * ratio).round() as Ticks;
        // Never round a note away to nothing.
        note.duration_ticks = ((note.duration_ticks as f64 * ratio).round() as Ticks).max(1);
    }
}
