//! A note list out to bytes.
//!
//! Three shapes, one event builder underneath: format 0 for per-track storage (notes and
//! a name, nothing else), format 1 with a conductor track for project export, and a
//! standalone format 0 that carries its own tempo so a dragged-out track opens at the
//! right speed rather than defaulting to 120.

use std::path::Path;

use midly::num::{u15, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks};

use super::tempo::bpm_to_micros_per_quarter;

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
