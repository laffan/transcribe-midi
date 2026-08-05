//! Turning a set of notes into something that can be played back off the timeline.
//!
//! The sequencer exists for the project: it is driven by the audio thread, owns one
//! transport, and is where playback belongs. Auditioning a *proposal* — the notes a
//! transcription is offering, before they are part of anything — cannot use it without
//! swapping the project's timeline out from under the transport and putting it back.
//!
//! So the boundaries are laid out in seconds instead, and a shell above this walks them
//! against a wall clock. That loses sample accuracy, which does not matter for a phrase
//! you are listening to in order to decide whether a note is wrong, and buys complete
//! independence from whatever the transport is doing.

use crate::Note;

/// One note boundary, placed in seconds from the moment playback starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuditionEvent {
    pub at_seconds: f64,
    /// True for a note-on, false for a note-off.
    pub on: bool,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

impl AuditionEvent {
    /// Sort key. A note-off sorts before a note-on at the same instant, so a note
    /// immediately followed by another of the same pitch retriggers instead of being
    /// silenced by the previous note's release arriving afterwards — the same rule the
    /// sequencer's timeline follows, for the same reason.
    fn order_key(&self) -> (f64, u8, u8, u8) {
        (self.at_seconds, u8::from(self.on), self.pitch, self.channel)
    }
}

/// How long the notes run for, in seconds. Zero when there is nothing to play.
pub fn span_seconds(notes: &[Note], ticks_per_second: f64) -> f64 {
    if ticks_per_second <= 0.0 {
        return 0.0;
    }
    notes
        .iter()
        .map(|note| f64::from(note.start_ticks + note.duration_ticks) / ticks_per_second)
        .fold(0.0, f64::max)
}

/// Place every note boundary in seconds relative to `from_seconds`.
///
/// A note already sounding at `from_seconds` starts immediately with whatever is left of
/// it, rather than being dropped: dropping it would make a phrase auditioned from its
/// middle sound like it has a hole where the held note was.
pub fn schedule(notes: &[Note], ticks_per_second: f64, from_seconds: f64) -> Vec<AuditionEvent> {
    if ticks_per_second <= 0.0 {
        return Vec::new();
    }
    let from = from_seconds.max(0.0);

    let mut events = Vec::with_capacity(notes.len() * 2);
    for note in notes {
        let start = f64::from(note.start_ticks) / ticks_per_second;
        let end = f64::from(note.start_ticks + note.duration_ticks) / ticks_per_second;
        if end <= from {
            continue;
        }

        events.push(AuditionEvent {
            at_seconds: (start - from).max(0.0),
            on: true,
            pitch: note.pitch,
            velocity: note.velocity,
            channel: note.channel,
        });
        events.push(AuditionEvent {
            at_seconds: end - from,
            on: false,
            pitch: note.pitch,
            velocity: note.velocity,
            channel: note.channel,
        });
    }

    // `f64` has no total order, so the comparison is written out rather than reached for
    // through `sort_by_key`. Every value here comes from a finite tick count, so the
    // fallback never runs — it is there so a NaN could only misorder, never panic.
    events.sort_by(|a, b| {
        a.order_key()
            .partial_cmp(&b.order_key())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 480 PPQ at 120 bpm — two beats a second, so a tick is a thousandth of a second.
    const TPS: f64 = 960.0;

    fn note(pitch: u8, start: u32, duration: u32) -> Note {
        Note::new(pitch, start, duration, 90, 0).unwrap()
    }

    #[test]
    fn each_note_becomes_an_on_and_an_off_in_seconds() {
        let events = schedule(&[note(60, 960, 480)], TPS, 0.0);
        assert_eq!(events.len(), 2);
        assert!(events[0].on);
        assert!((events[0].at_seconds - 1.0).abs() < 1e-9);
        assert!(!events[1].on);
        assert!((events[1].at_seconds - 1.5).abs() < 1e-9);
    }

    #[test]
    fn notes_that_finish_before_the_start_point_are_left_out() {
        let events = schedule(&[note(60, 0, 480), note(64, 960, 480)], TPS, 0.75);
        assert_eq!(events.len(), 2, "only the second note survives");
        assert_eq!(events[0].pitch, 64);
    }

    #[test]
    fn a_note_straddling_the_start_point_begins_immediately_with_what_is_left() {
        let events = schedule(&[note(60, 0, 1920)], TPS, 1.0);
        assert_eq!(events[0].at_seconds, 0.0, "it is already sounding");
        assert!((events[1].at_seconds - 1.0).abs() < 1e-9, "one second left of it");
    }

    #[test]
    fn a_release_sorts_before_a_retrigger_at_the_same_instant() {
        let events = schedule(&[note(60, 0, 480), note(60, 480, 480)], TPS, 0.0);
        let at_half = events.iter().filter(|e| e.at_seconds == 0.5).collect::<Vec<_>>();
        assert_eq!(at_half.len(), 2);
        assert!(!at_half[0].on, "the release comes first, or the note is cut short");
        assert!(at_half[1].on);
    }

    #[test]
    fn a_nonsense_tempo_schedules_nothing_rather_than_dividing_by_zero() {
        assert!(schedule(&[note(60, 0, 480)], 0.0, 0.0).is_empty());
        assert_eq!(span_seconds(&[note(60, 0, 480)], 0.0), 0.0);
    }

    #[test]
    fn the_span_reaches_the_end_of_the_last_note_not_its_start() {
        let span = span_seconds(&[note(60, 0, 480), note(64, 960, 1920)], TPS);
        assert!((span - 3.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_play_is_a_span_of_zero() {
        assert_eq!(span_seconds(&[], TPS), 0.0);
        assert!(schedule(&[], TPS, 0.0).is_empty());
    }
}
