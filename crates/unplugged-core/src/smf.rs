//! Conversion between the in-memory note list and Standard MIDI Files.
//!
//! Per-track storage is SMF **format 0** (one track chunk) carrying only note events and
//! a track-name meta event. Tempo and time signature deliberately live in `project.json`
//! and are not duplicated here — see DECISIONS.md. Format 1 export with a conductor
//! track is Phase 5's job.

use std::collections::HashMap;
use std::path::Path;

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks};

/// Ordering of events that land on the same tick.
///
/// Note-offs must be written before note-ons so that a note immediately followed by
/// another of the same pitch retriggers instead of being cut short by the previous
/// note's off event arriving after the new on.
const ORDER_NOTE_OFF: u8 = 0;
const ORDER_NOTE_ON: u8 = 1;

/// Serialize a note list to SMF format 0 bytes.
pub fn notes_to_smf_bytes(notes: &[Note], ppq: u16, track_name: &str) -> Result<Vec<u8>> {
    // (absolute_tick, ordering, event)
    let mut timed: Vec<(Ticks, u8, TrackEventKind<'static>)> = Vec::with_capacity(notes.len() * 2);

    for note in notes {
        note.validate()?;
        let channel = u4::new(note.channel);
        timed.push((
            note.start_ticks,
            ORDER_NOTE_ON,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key: u7::new(note.pitch),
                    vel: u7::new(note.velocity),
                },
            },
        ));
        timed.push((
            note.end_ticks(),
            ORDER_NOTE_OFF,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff {
                    key: u7::new(note.pitch),
                    vel: u7::new(64),
                },
            },
        ));
    }

    timed.sort_by_key(|(tick, order, _)| (*tick, *order));

    let name_bytes = track_name.as_bytes().to_vec();
    let mut events: Vec<TrackEvent> = Vec::with_capacity(timed.len() + 2);
    events.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(&name_bytes)),
    });

    let mut prev_tick: Ticks = 0;
    for (tick, _, kind) in timed {
        events.push(TrackEvent {
            delta: u28::new(tick - prev_tick),
            kind,
        });
        prev_tick = tick;
    }

    events.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let header = Header::new(Format::SingleTrack, Timing::Metrical(u15::new(ppq)));
    let smf = Smf {
        header,
        tracks: vec![events],
    };

    let mut buf = Vec::new();
    smf.write_std(&mut buf)
        .map_err(|e| CoreError::io(Path::new("<in-memory MIDI buffer>"), e))?;
    Ok(buf)
}

/// Parse SMF bytes into a sorted note list, plus the file's PPQ if it declares one.
///
/// Accepts format 0 and format 1 (all tracks merged), which means this doubles as the
/// import path in Phase 5. SMPTE-timed files are rejected: the whole model is tick-based
/// against a musical PPQ, and silently reinterpreting frame-based timing as ticks would
/// produce nonsense.
pub fn smf_bytes_to_notes(bytes: &[u8], path_for_errors: &Path) -> Result<(Vec<Note>, Option<u16>)> {
    let smf = Smf::parse(bytes).map_err(|source| CoreError::MidiParse {
        path: path_for_errors.to_path_buf(),
        source,
    })?;

    let file_ppq = match smf.header.timing {
        Timing::Metrical(ppq) => Some(ppq.as_int()),
        Timing::Timecode(..) => None,
    };

    let mut notes = Vec::new();

    for track in &smf.tracks {
        let mut abs: u64 = 0;
        // (channel, pitch) -> queue of pending note-ons, so overlapping identical pitches
        // pair oldest-on with oldest-off rather than collapsing into one note.
        let mut pending: HashMap<(u8, u8), Vec<(u64, u8)>> = HashMap::new();

        for event in track {
            abs += event.delta.as_int() as u64;

            let TrackEventKind::Midi { channel, message } = event.kind else {
                continue;
            };
            let channel = channel.as_int();

            match message {
                // A note-on with velocity 0 is a note-off. This is not an edge case —
                // most real-world MIDI files use running status and express every
                // note-off this way.
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    pending
                        .entry((channel, key.as_int()))
                        .or_default()
                        .push((abs, vel.as_int()));
                }
                MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                    let queue = pending.entry((channel, key.as_int())).or_default();
                    if queue.is_empty() {
                        continue; // Note-off with no matching on; ignore.
                    }
                    let (start, velocity) = queue.remove(0);
                    let duration = abs.saturating_sub(start).max(1);
                    notes.push(Note {
                        pitch: key.as_int(),
                        start_ticks: clamp_ticks(start),
                        duration_ticks: clamp_ticks(duration),
                        velocity,
                        channel,
                    });
                }
                _ => {}
            }
        }

        // Notes still held when the track ends. Rather than discarding user data, close
        // them at the final event time (minimum one tick).
        for ((channel, pitch), queue) in pending {
            for (start, velocity) in queue {
                let duration = abs.saturating_sub(start).max(1);
                notes.push(Note {
                    pitch,
                    start_ticks: clamp_ticks(start),
                    duration_ticks: clamp_ticks(duration),
                    velocity,
                    channel,
                });
            }
        }
    }

    notes.sort_by_key(|n| n.order_key());
    Ok((notes, file_ppq))
}

fn clamp_ticks(value: u64) -> Ticks {
    value.min(Ticks::MAX as u64) as Ticks
}

/// Microseconds per quarter note, as SMF's tempo meta event expresses it.
pub fn bpm_to_micros_per_quarter(bpm: f64) -> u24 {
    let micros = (60_000_000.0 / bpm).round().clamp(1.0, 0xFF_FFFF as f64);
    u24::new(micros as u32)
}

/// The inverse, for import.
pub fn micros_per_quarter_to_bpm(micros: u32) -> f64 {
    if micros == 0 {
        return crate::model::DEFAULT_TEMPO;
    }
    60_000_000.0 / micros as f64
}

// ---------------------------------------------------------------------------
// Type 1 export
// ---------------------------------------------------------------------------

/// Serialize a whole project as SMF **format 1**, the interchange format the spec asks
/// for and the one Logic Pro and GarageBand expect.
///
/// Layout follows the convention every DAW assumes: track 0 is a conductor track
/// carrying only tempo, time signature and the project name, and each subsequent chunk
/// is one instrument track. Storage uses format 0 per track instead (see DECISIONS.md);
/// these are separate concerns and deliberately separate code paths.
pub fn project_to_smf_type1(project: &crate::model::Project) -> Result<Vec<u8>> {
    let manifest = &project.manifest;
    let ppq = manifest.ppq;

    // --- conductor track ---
    let project_name = manifest.name.as_bytes().to_vec();
    let time_signature = &manifest.time_signature;
    // SMF stores the denominator as a power of two: 4 becomes 2, 8 becomes 3.
    let denominator_pow2 = time_signature.denominator.trailing_zeros() as u8;

    let mut conductor = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(&project_name)),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(bpm_to_micros_per_quarter(
                manifest.tempo_bpm,
            ))),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(
                time_signature.numerator,
                denominator_pow2,
                24, // MIDI clocks per metronome click — the conventional default
                8,  // 32nd notes per quarter
            )),
        },
    ];
    conductor.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    // --- instrument tracks ---
    //
    // Names must outlive the borrowed `MetaMessage::TrackName`, so they are collected
    // up front rather than produced inside the loop.
    let track_names: Vec<Vec<u8>> = project
        .tracks
        .iter()
        .map(|t| t.meta.name.as_bytes().to_vec())
        .collect();

    let mut chunks: Vec<Vec<TrackEvent>> = Vec::with_capacity(project.tracks.len() + 1);
    chunks.push(conductor);

    for (track, name) in project.tracks.iter().zip(track_names.iter()) {
        chunks.push(note_events(&track.notes, name)?);
    }

    let header = Header::new(Format::Parallel, Timing::Metrical(u15::new(ppq)));
    let smf = Smf { header, tracks: chunks };

    let mut buf = Vec::new();
    smf.write_std(&mut buf)
        .map_err(|e| CoreError::io(Path::new("<in-memory MIDI buffer>"), e))?;
    Ok(buf)
}

/// Serialize a single track as format 0, with tempo and time signature included.
///
/// This is what drag-out and "export track" produce: a standalone file that opens at the
/// right tempo rather than defaulting to 120.
pub fn track_to_standalone_smf(
    track: &crate::model::Track,
    tempo_bpm: f64,
    time_signature: crate::model::TimeSignature,
    ppq: u16,
) -> Result<Vec<u8>> {
    let name = track.meta.name.as_bytes().to_vec();
    let denominator_pow2 = time_signature.denominator.trailing_zeros() as u8;

    let mut events = vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(&name)),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(bpm_to_micros_per_quarter(tempo_bpm))),
        },
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(
                time_signature.numerator,
                denominator_pow2,
                24,
                8,
            )),
        },
    ];

    // Reuse the note-event builder, dropping its own name and end-of-track events.
    let notes = note_events(&track.notes, &[])?;
    events.extend(notes.into_iter().skip(1));

    let header = Header::new(Format::SingleTrack, Timing::Metrical(u15::new(ppq)));
    let smf = Smf { header, tracks: vec![events] };

    let mut buf = Vec::new();
    smf.write_std(&mut buf)
        .map_err(|e| CoreError::io(Path::new("<in-memory MIDI buffer>"), e))?;
    Ok(buf)
}

/// Build a delta-encoded event list for one track: name, notes, end-of-track.
fn note_events<'a>(notes: &[Note], name: &'a [u8]) -> Result<Vec<TrackEvent<'a>>> {
    let mut timed: Vec<(Ticks, u8, TrackEventKind<'static>)> = Vec::with_capacity(notes.len() * 2);

    for note in notes {
        note.validate()?;
        let channel = u4::new(note.channel);
        timed.push((
            note.start_ticks,
            ORDER_NOTE_ON,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key: u7::new(note.pitch),
                    vel: u7::new(note.velocity),
                },
            },
        ));
        timed.push((
            note.end_ticks(),
            ORDER_NOTE_OFF,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff {
                    key: u7::new(note.pitch),
                    vel: u7::new(64),
                },
            },
        ));
    }

    timed.sort_by_key(|(tick, order, _)| (*tick, *order));

    let mut events = Vec::with_capacity(timed.len() + 2);
    events.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(name)),
    });

    let mut prev_tick: Ticks = 0;
    for (tick, _, kind) in timed {
        events.push(TrackEvent {
            delta: u28::new(tick - prev_tick),
            kind,
        });
        prev_tick = tick;
    }

    events.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    Ok(events)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DEFAULT_PPQ;
    use std::path::PathBuf;

    fn p() -> PathBuf {
        PathBuf::from("test.mid")
    }

    fn note(pitch: u8, start: u32, dur: u32, vel: u8, ch: u8) -> Note {
        Note::new(pitch, start, dur, vel, ch).unwrap()
    }

    #[test]
    fn round_trip_preserves_every_field() {
        // In canonical order — sorted by (start_ticks, pitch). Note that at tick 960
        // pitch 48 precedes pitch 67, and the two are on different channels.
        let original = vec![
            note(60, 0, 480, 100, 0),
            note(64, 480, 240, 80, 0),
            note(48, 960, 480, 1, 15),
            note(67, 960, 1920, 127, 3),
        ];

        let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Piano").unwrap();
        let (parsed, ppq) = smf_bytes_to_notes(&bytes, &p()).unwrap();

        assert_eq!(ppq, Some(DEFAULT_PPQ));
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_survives_an_empty_track() {
        let bytes = notes_to_smf_bytes(&[], DEFAULT_PPQ, "Empty").unwrap();
        let (parsed, ppq) = smf_bytes_to_notes(&bytes, &p()).unwrap();
        assert!(parsed.is_empty());
        assert_eq!(ppq, Some(DEFAULT_PPQ));
    }

    #[test]
    fn overlapping_notes_of_the_same_pitch_stay_separate() {
        // Two of the same pitch overlapping — the naive "one pending note per pitch"
        // implementation collapses these into one note and loses the second.
        let original = vec![note(60, 0, 960, 100, 0), note(60, 480, 960, 90, 0)];
        let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Overlap").unwrap();
        let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
        assert_eq!(parsed.len(), 2, "overlapping same-pitch notes must not merge");
        assert_eq!(parsed, original);
    }

    #[test]
    fn note_on_with_zero_velocity_is_treated_as_note_off() {
        // Hand-built: note-on 60 at tick 0, then note-on 60 vel 0 at tick 480.
        // Almost all real MIDI files in the wild express note-offs this way.
        let events = vec![
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent {
                delta: u28::new(480),
                kind: TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(0) },
                },
            },
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
        ];
        let smf = Smf {
            header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(DEFAULT_PPQ))),
            tracks: vec![events],
        };
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();

        let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
        assert_eq!(parsed, vec![note(60, 0, 480, 100, 0)]);
    }

    #[test]
    fn adjacent_same_pitch_notes_do_not_lose_their_retrigger() {
        // Note A ends exactly where note B begins. If the off/on ordering at that tick
        // were reversed, B would be cut to nothing on replay.
        let original = vec![note(60, 0, 480, 100, 0), note(60, 480, 480, 100, 0)];
        let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Adjacent").unwrap();
        let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn unterminated_note_is_kept_rather_than_dropped() {
        let events = vec![
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent { delta: u28::new(960), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
        ];
        let smf = Smf {
            header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(DEFAULT_PPQ))),
            tracks: vec![events],
        };
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();

        let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].duration_ticks, 960);
    }

    #[test]
    fn parsed_notes_come_back_sorted() {
        let original = vec![
            note(72, 1920, 240, 100, 0),
            note(60, 0, 240, 100, 0),
            note(64, 960, 240, 100, 0),
        ];
        let bytes = notes_to_smf_bytes(&original, DEFAULT_PPQ, "Sort").unwrap();
        let (parsed, _) = smf_bytes_to_notes(&bytes, &p()).unwrap();
        assert!(parsed.windows(2).all(|w| w[0].order_key() <= w[1].order_key()));
    }

    #[test]
    fn tempo_conversion_matches_the_midi_spec() {
        // 120 bpm is exactly 500000 microseconds per quarter note.
        assert_eq!(bpm_to_micros_per_quarter(120.0).as_int(), 500_000);
        assert_eq!(bpm_to_micros_per_quarter(60.0).as_int(), 1_000_000);
    }

    #[test]
    fn garbage_input_is_an_error_not_a_panic() {
        assert!(smf_bytes_to_notes(b"definitely not a midi file", &p()).is_err());
        assert!(smf_bytes_to_notes(&[], &p()).is_err());
    }

    // -- type 1 export -----------------------------------------------------

    use crate::model::{InstrumentRef, Project, ProjectManifest, TimeSignature, Track, TrackMeta};
    use crate::model::SCHEMA_VERSION;

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

    #[test]
    fn type1_export_has_a_conductor_track_plus_one_chunk_per_track() {
        let proj = project(
            vec![
                track("Piano", vec![note(60, 0, 480, 100, 0)]),
                track("Bass", vec![note(36, 0, 960, 90, 1)]),
            ],
            120.0,
            TimeSignature::default(),
        );

        let bytes = project_to_smf_type1(&proj).unwrap();
        let smf = Smf::parse(&bytes).unwrap();

        assert_eq!(smf.header.format, Format::Parallel, "must be SMF type 1");
        assert_eq!(smf.tracks.len(), 3, "conductor plus two instrument tracks");
    }

    #[test]
    fn the_conductor_track_carries_tempo_and_time_signature() {
        let proj = project(vec![track("T", vec![note(60, 0, 480, 100, 0)])], 96.0,
                        TimeSignature::new(6, 8).unwrap());
        let bytes = project_to_smf_type1(&proj).unwrap();
        let smf = Smf::parse(&bytes).unwrap();

        let mut tempo = None;
        let mut sig = None;
        for event in &smf.tracks[0] {
            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(micros)) => tempo = Some(micros.as_int()),
                TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, ..)) => sig = Some((n, d)),
                _ => {}
            }
        }

        assert_eq!(tempo, Some(bpm_to_micros_per_quarter(96.0).as_int()));
        // 6/8 — the denominator is stored as a power of two, so 8 becomes 3.
        assert_eq!(sig, Some((6, 3)));
    }

    #[test]
    fn a_type1_export_reimports_with_the_same_notes_tempo_and_signature() {
        let proj = project(
            vec![
                track("Piano", vec![note(60, 0, 480, 100, 0), note(64, 480, 240, 80, 0)]),
                track("Bass", vec![note(36, 0, 1920, 90, 1)]),
            ],
            140.0,
            TimeSignature::new(3, 4).unwrap(),
        );

        let bytes = project_to_smf_type1(&proj).unwrap();
        let imported = parse_smf_for_import(&bytes, &p()).unwrap();

        assert_eq!(imported.ppq, Some(DEFAULT_PPQ));
        assert_eq!(imported.time_signature, Some(TimeSignature::new(3, 4).unwrap()));
        assert!((imported.tempo_bpm.unwrap() - 140.0).abs() < 0.01);

        assert_eq!(imported.tracks.len(), 2, "the empty conductor track must not become a track");
        assert_eq!(imported.tracks[0].name.as_deref(), Some("Piano"));
        assert_eq!(imported.tracks[0].notes, vec![note(60, 0, 480, 100, 0), note(64, 480, 240, 80, 0)]);
        assert_eq!(imported.tracks[1].name.as_deref(), Some("Bass"));
        assert_eq!(imported.tracks[1].notes, vec![note(36, 0, 1920, 90, 1)]);
    }

    #[test]
    fn exporting_a_project_with_no_notes_still_produces_a_valid_file() {
        let proj = project(vec![track("Empty", vec![])], 120.0, TimeSignature::default());
        let bytes = project_to_smf_type1(&proj).unwrap();

        let smf = Smf::parse(&bytes).unwrap();
        assert_eq!(smf.tracks.len(), 2);
        assert!(parse_smf_for_import(&bytes, &p()).unwrap().tracks.is_empty());
    }

    #[test]
    fn a_standalone_track_export_carries_its_own_tempo() {
        // Drag-out and single-track export must open at the right tempo rather than
        // defaulting to 120.
        let t = track("Solo", vec![note(60, 0, 480, 100, 0)]);
        let bytes = track_to_standalone_smf(&t, 88.0, TimeSignature::new(5, 4).unwrap(), DEFAULT_PPQ).unwrap();

        let imported = parse_smf_for_import(&bytes, &p()).unwrap();
        assert!((imported.tempo_bpm.unwrap() - 88.0).abs() < 0.01);
        assert_eq!(imported.time_signature, Some(TimeSignature::new(5, 4).unwrap()));
        assert_eq!(imported.tracks.len(), 1);
        assert_eq!(imported.tracks[0].notes.len(), 1);
        assert_eq!(imported.tracks[0].name.as_deref(), Some("Solo"));
    }

    #[test]
    fn channels_survive_the_type1_round_trip() {
        let proj = project(
            vec![track("Multi", vec![
                note(60, 0, 240, 100, 0),
                note(62, 0, 240, 100, 9),
                note(64, 0, 240, 100, 15),
            ])],
            120.0,
            TimeSignature::default(),
        );
        let bytes = project_to_smf_type1(&proj).unwrap();
        let imported = parse_smf_for_import(&bytes, &p()).unwrap();

        let channels: Vec<u8> = imported.tracks[0].notes.iter().map(|n| n.channel).collect();
        assert_eq!(channels, vec![0, 9, 15]);
    }

    // -- import ------------------------------------------------------------

    #[test]
    fn a_format_0_file_imports_as_one_track() {
        let bytes = notes_to_smf_bytes(&[note(60, 0, 480, 100, 0)], DEFAULT_PPQ, "Single").unwrap();
        let imported = parse_smf_for_import(&bytes, &p()).unwrap();

        assert_eq!(imported.tracks.len(), 1);
        assert_eq!(imported.tracks[0].name.as_deref(), Some("Single"));
        assert_eq!(imported.note_count(), 1);
        // No tempo meta in a bare per-track file; the caller keeps the project tempo.
        assert_eq!(imported.tempo_bpm, None);
    }

    #[test]
    fn import_rejects_garbage_rather_than_producing_an_empty_project() {
        assert!(parse_smf_for_import(b"not a midi file", &p()).is_err());
        assert!(parse_smf_for_import(&[], &p()).is_err());
    }

    #[test]
    fn a_track_with_no_name_imports_without_one() {
        let bytes = notes_to_smf_bytes(&[note(60, 0, 480, 100, 0)], DEFAULT_PPQ, "").unwrap();
        let imported = parse_smf_for_import(&bytes, &p()).unwrap();
        assert_eq!(imported.tracks[0].name, None, "an empty name must not become Some(\"\")");
    }

    // -- ppq rescaling -----------------------------------------------------

    #[test]
    fn rescaling_ppq_preserves_musical_position() {
        // A quarter note at 96 ppq becomes a quarter note at 480 ppq.
        let mut notes = vec![note(60, 96, 96, 100, 0), note(64, 192, 48, 100, 0)];
        rescale_ppq(&mut notes, 96, 480);

        assert_eq!(notes[0].start_ticks, 480);
        assert_eq!(notes[0].duration_ticks, 480);
        assert_eq!(notes[1].start_ticks, 960);
        assert_eq!(notes[1].duration_ticks, 240);
    }

    #[test]
    fn rescaling_down_never_annihilates_a_short_note() {
        // A 1-tick note at 960 ppq would round to 0 at 96 ppq, which SMF cannot express.
        let mut notes = vec![note(60, 0, 1, 100, 0)];
        rescale_ppq(&mut notes, 960, 96);
        assert_eq!(notes[0].duration_ticks, 1);
    }

    #[test]
    fn rescaling_to_the_same_ppq_is_a_no_op() {
        let original = vec![note(60, 137, 499, 100, 0)];
        let mut notes = original.clone();
        rescale_ppq(&mut notes, 480, 480);
        assert_eq!(notes, original);

        // Zero must not divide by zero or zero everything out.
        rescale_ppq(&mut notes, 0, 480);
        rescale_ppq(&mut notes, 480, 0);
        assert_eq!(notes, original);
    }
}
