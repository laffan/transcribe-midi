//! Bytes back into a note list.
//!
//! The input is a file some other program wrote, so nothing here trusts its structure:
//! an unterminated note is kept rather than dropped, a note-on with velocity 0 is read as
//! the note-off it means, and SMPTE timing is refused outright rather than reinterpreted
//! as ticks.

use std::collections::HashMap;
use std::path::Path;

use midly::{MidiMessage, Smf, Timing, TrackEventKind};

use crate::error::{CoreError, Result};
use crate::model::{Note, Ticks};

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

pub(super) fn clamp_ticks(value: u64) -> Ticks {
    value.min(Ticks::MAX as u64) as Ticks
}
