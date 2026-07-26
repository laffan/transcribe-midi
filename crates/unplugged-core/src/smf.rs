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
}
