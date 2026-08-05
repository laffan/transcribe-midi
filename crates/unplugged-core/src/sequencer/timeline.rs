//! A project flattened into one sorted event list.
//!
//! Built on the control thread so the audio thread never walks the project structure,
//! never allocates and never has to reason about mute or solo.

use crate::model::{Project, Ticks};

use super::event::{EventKind, TimelineEvent};

/// The flattened, sorted note events for a whole project.
///
/// Built on the control thread and handed to the audio thread wholesale, so that the
/// audio thread never walks the project structure or allocates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    pub(super) events: Vec<TimelineEvent>,
    length_ticks: Ticks,
}

impl Timeline {
    pub fn new(mut events: Vec<TimelineEvent>) -> Self {
        events.sort_by_key(|e| e.order_key());
        let length_ticks = events.last().map(|e| e.tick).unwrap_or(0);
        Timeline { events, length_ticks }
    }

    /// Flatten a project, honouring mute and solo.
    pub fn from_project(project: &Project) -> Self {
        Timeline::from_tracks(&project.tracks)
    }

    /// Flatten a track list, honouring mute and solo.
    ///
    /// Solo wins over mute, as it does in every DAW: if anything is soloed, only soloed
    /// tracks sound, regardless of their mute flags.
    pub fn from_tracks(tracks: &[crate::model::Track]) -> Self {
        let any_soloed = tracks.iter().any(|t| t.meta.soloed);

        let mut events = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            let audible = if any_soloed { track.meta.soloed } else { !track.meta.muted };
            if !audible {
                continue;
            }

            let track_index = index as u16;
            for note in &track.notes {
                events.push(TimelineEvent {
                    tick: note.start_ticks,
                    kind: EventKind::NoteOn,
                    track: track_index,
                    pitch: note.pitch,
                    velocity: note.velocity,
                    channel: note.channel,
                });
                events.push(TimelineEvent {
                    tick: note.end_ticks(),
                    kind: EventKind::NoteOff,
                    track: track_index,
                    pitch: note.pitch,
                    velocity: 64,
                    channel: note.channel,
                });
            }
        }

        Timeline::new(events)
    }

    pub fn events(&self) -> &[TimelineEvent] {
        &self.events
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Tick of the last event — i.e. where playback would naturally end.
    pub fn length_ticks(&self) -> Ticks {
        self.length_ticks
    }

    /// Index of the first event at or after `tick`.
    pub(super) fn seek_index(&self, tick: Ticks) -> usize {
        self.events.partition_point(|e| e.tick < tick)
    }
}
