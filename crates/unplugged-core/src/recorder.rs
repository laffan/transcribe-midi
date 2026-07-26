//! Capture of live MIDI input into notes.
//!
//! Pure and platform-free, like [`crate::sequencer`]: the awkward parts of recording are
//! all timing decisions, and timing decisions are exactly what cannot be tested on a
//! machine without a MIDI interface. Port I/O lives in `crates/unplugged-midi`; what
//! happens to the events once they arrive lives here.
//!
//! **Every event is stamped by the caller with a tick derived from the audio clock**, per
//! the Phase 0 decision: `midir` does not populate timestamps on iOS, so trusting its
//! clock would record correctly on macOS and wrongly on device.

use std::collections::HashMap;

use crate::model::{Note, Ticks};

/// A note-on that has not yet been matched with its note-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    start_ticks: Ticks,
    velocity: u8,
}

/// Accumulates live input into notes.
///
/// Overdub is the default and only mode: captured notes are *added* to whatever is
/// already on the track. Replacing would be a destructive edit, and the command layer
/// makes adding-then-undoing cheap.
#[derive(Debug, Default)]
pub struct Recorder {
    /// `(channel, pitch)` → queue of unmatched note-ons.
    ///
    /// A queue rather than a single slot: a sustained chord voicing can restrike the
    /// same pitch before the first release arrives, and collapsing those loses a note.
    pending: HashMap<(u8, u8), Vec<Pending>>,
    captured: Vec<Note>,
    /// Grid to snap captured starts to, in ticks. Zero means no quantisation.
    quantize_ticks: u32,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Quantise note starts as they are captured ("transport quantize" in the inspector).
    ///
    /// Applied at capture rather than afterwards so the recorded take is what the user
    /// hears on the next loop pass. Zero disables it.
    pub fn set_quantize(&mut self, ticks: u32) {
        self.quantize_ticks = ticks;
    }

    pub fn is_empty(&self) -> bool {
        self.captured.is_empty() && self.pending.is_empty()
    }

    pub fn captured(&self) -> &[Note] {
        &self.captured
    }

    pub fn held_count(&self) -> usize {
        self.pending.values().map(Vec::len).sum()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.captured.clear();
    }

    // -----------------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------------

    pub fn note_on(&mut self, tick: Ticks, pitch: u8, velocity: u8, channel: u8) {
        // A note-on with velocity 0 is a note-off — most hardware sends releases this way.
        if velocity == 0 {
            self.note_off(tick, pitch, channel);
            return;
        }

        self.pending
            .entry((channel & 0x0F, pitch & 0x7F))
            .or_default()
            .push(Pending {
                start_ticks: tick,
                velocity: velocity.min(127),
            });
    }

    pub fn note_off(&mut self, tick: Ticks, pitch: u8, channel: u8) {
        let key = (channel & 0x0F, pitch & 0x7F);
        let Some(queue) = self.pending.get_mut(&key) else {
            return; // Release with no matching press — e.g. a key held before record began.
        };
        if queue.is_empty() {
            return;
        }

        // Oldest-first, so overlapping restrikes of one pitch nest correctly.
        let pending = queue.remove(0);
        if queue.is_empty() {
            self.pending.remove(&key);
        }

        self.push_note(key.1, pending, tick, key.0);
    }

    fn push_note(&mut self, pitch: u8, pending: Pending, end_tick: Ticks, channel: u8) {
        let start = if self.quantize_ticks > 0 {
            let grid = self.quantize_ticks as f64;
            ((pending.start_ticks as f64 / grid).round() * grid) as Ticks
        } else {
            pending.start_ticks
        };

        // A note must have non-zero length to be representable in SMF. Quantisation can
        // push the start past the end, so the duration is derived defensively.
        let duration = end_tick.saturating_sub(start).max(1);

        self.captured.push(Note {
            pitch,
            start_ticks: start,
            duration_ticks: duration,
            velocity: pending.velocity.max(1),
            channel,
        });
    }

    // -----------------------------------------------------------------------
    // Loop and stop
    // -----------------------------------------------------------------------

    /// Handle the transport wrapping from `loop_end` back to `loop_start`.
    ///
    /// Held notes are closed at the loop end and **reopened at the loop start**. Closing
    /// alone would drop the sound from the top of the next pass, which is wrong for a
    /// held pad or a sustained chord; reopening reproduces what the player is actually
    /// still holding. The cost is that one continuous press becomes one note per pass,
    /// which is both unavoidable in a bar-looped take and what the playback will sound
    /// like anyway.
    pub fn on_loop_wrap(&mut self, loop_end: Ticks, loop_start: Ticks) {
        let mut reopened: HashMap<(u8, u8), Vec<Pending>> = HashMap::new();

        for (key, queue) in std::mem::take(&mut self.pending) {
            for pending in queue {
                self.push_note(key.1, pending, loop_end, key.0);
                reopened.entry(key).or_default().push(Pending {
                    start_ticks: loop_start,
                    velocity: pending.velocity,
                });
            }
        }

        self.pending = reopened;
    }

    /// Stop recording and take everything captured, closing anything still held at `tick`.
    ///
    /// Returns notes sorted into the canonical order, ready to hand to the command layer.
    pub fn finish(&mut self, tick: Ticks) -> Vec<Note> {
        for (key, queue) in std::mem::take(&mut self.pending) {
            for pending in queue {
                self.push_note(key.1, pending, tick, key.0);
            }
        }

        let mut notes = std::mem::take(&mut self.captured);
        notes.sort_by_key(Note::order_key);
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, start: u32, dur: u32, vel: u8) -> Note {
        Note { pitch, start_ticks: start, duration_ticks: dur, velocity: vel, channel: 0 }
    }

    #[test]
    fn a_press_and_release_becomes_one_note() {
        let mut r = Recorder::new();
        r.note_on(0, 60, 100, 0);
        r.note_off(480, 60, 0);

        assert_eq!(r.finish(960), vec![note(60, 0, 480, 100)]);
    }

    #[test]
    fn note_on_with_zero_velocity_is_a_release() {
        // Most hardware sends releases this way rather than as note-off.
        let mut r = Recorder::new();
        r.note_on(0, 60, 100, 0);
        r.note_on(480, 60, 0, 0);

        assert_eq!(r.finish(960), vec![note(60, 0, 480, 100)]);
        assert_eq!(r.held_count(), 0);
    }

    #[test]
    fn a_chord_records_as_simultaneous_notes() {
        let mut r = Recorder::new();
        for pitch in [60, 64, 67] {
            r.note_on(0, pitch, 100, 0);
        }
        for pitch in [60, 64, 67] {
            r.note_off(480, pitch, 0);
        }

        let notes = r.finish(480);
        assert_eq!(notes.len(), 3);
        assert!(notes.iter().all(|n| n.start_ticks == 0 && n.duration_ticks == 480));
    }

    #[test]
    fn overlapping_restrikes_of_one_pitch_stay_separate() {
        // Press, press again, release, release — a trill or a sustained restrike. A
        // single-slot implementation loses one of these.
        let mut r = Recorder::new();
        r.note_on(0, 60, 100, 0);
        r.note_on(240, 60, 90, 0);
        r.note_off(480, 60, 0);
        r.note_off(720, 60, 0);

        let notes = r.finish(720);
        assert_eq!(notes.len(), 2, "both presses must survive");
        // Oldest-first pairing: the first press gets the first release.
        assert_eq!(notes[0], note(60, 0, 480, 100));
        assert_eq!(notes[1], note(60, 240, 480, 90));
    }

    #[test]
    fn a_note_still_held_at_stop_is_closed_rather_than_lost() {
        let mut r = Recorder::new();
        r.note_on(100, 72, 110, 0);
        assert_eq!(r.held_count(), 1);

        let notes = r.finish(1000);
        assert_eq!(notes, vec![note(72, 100, 900, 110)]);
        assert_eq!(r.held_count(), 0, "finish must drain the pending map");
    }

    #[test]
    fn a_release_with_no_matching_press_is_ignored() {
        // The player was already holding the key when recording started.
        let mut r = Recorder::new();
        r.note_off(480, 60, 0);
        assert!(r.finish(960).is_empty());
    }

    #[test]
    fn channels_are_kept_distinct() {
        let mut r = Recorder::new();
        r.note_on(0, 60, 100, 0);
        r.note_on(0, 60, 100, 5);
        r.note_off(480, 60, 5);
        r.note_off(960, 60, 0);

        let notes = r.finish(960);
        assert_eq!(notes.len(), 2);
        let ch5 = notes.iter().find(|n| n.channel == 5).unwrap();
        assert_eq!(ch5.duration_ticks, 480, "the channel-5 release must not close channel 0");
    }

    // -- looping ------------------------------------------------------------

    #[test]
    fn a_note_held_across_the_loop_point_is_closed_and_reopened() {
        let mut r = Recorder::new();
        r.note_on(1800, 60, 100, 0); // late in a 0..1920 loop
        r.on_loop_wrap(1920, 0);

        assert_eq!(r.held_count(), 1, "still physically held, so still pending");

        r.note_off(240, 60, 0); // released early in the next pass
        let notes = r.finish(480);

        assert_eq!(notes.len(), 2, "one note per pass");
        assert_eq!(notes[0], note(60, 0, 240, 100), "reopened at the loop start");
        assert_eq!(notes[1], note(60, 1800, 120, 100), "closed at the loop end");
    }

    #[test]
    fn loop_wrap_with_nothing_held_captures_nothing() {
        let mut r = Recorder::new();
        r.on_loop_wrap(1920, 0);
        assert!(r.finish(0).is_empty());
    }

    #[test]
    fn a_note_held_across_several_wraps_yields_one_note_per_pass() {
        let mut r = Recorder::new();
        r.note_on(0, 60, 100, 0);
        r.on_loop_wrap(1920, 0);
        r.on_loop_wrap(1920, 0);
        r.on_loop_wrap(1920, 0);

        let notes = r.finish(1920);
        assert_eq!(notes.len(), 4, "three wraps plus the final pass");
        assert!(notes.iter().all(|n| n.duration_ticks == 1920));
    }

    #[test]
    fn velocity_survives_a_loop_wrap() {
        let mut r = Recorder::new();
        r.note_on(1000, 64, 37, 0);
        r.on_loop_wrap(1920, 0);
        r.note_off(100, 64, 0);

        let notes = r.finish(200);
        assert!(notes.iter().all(|n| n.velocity == 37), "the reopened note keeps its velocity");
    }

    // -- quantisation -------------------------------------------------------

    #[test]
    fn quantize_snaps_starts_to_the_nearest_grid_line() {
        let mut r = Recorder::new();
        r.set_quantize(240);

        r.note_on(10, 60, 100, 0); // rounds down to 0
        r.note_off(500, 60, 0);
        r.note_on(230, 64, 100, 0); // rounds up to 240
        r.note_off(700, 64, 0);

        let notes = r.finish(700);
        assert_eq!(notes[0].start_ticks, 0);
        assert_eq!(notes[1].start_ticks, 240);
    }

    #[test]
    fn quantize_never_produces_a_zero_length_note() {
        // Snapping forward can push the start past the release.
        let mut r = Recorder::new();
        r.set_quantize(480);
        r.note_on(470, 60, 100, 0);
        r.note_off(475, 60, 0);

        let notes = r.finish(475);
        assert_eq!(notes[0].start_ticks, 480, "snapped forward, past the release");
        assert_eq!(notes[0].duration_ticks, 1, "duration must stay representable");
    }

    #[test]
    fn zero_quantize_leaves_timing_untouched() {
        let mut r = Recorder::new();
        r.set_quantize(0);
        r.note_on(137, 60, 100, 0);
        r.note_off(499, 60, 0);
        assert_eq!(r.finish(499)[0].start_ticks, 137);
    }

    // -- hygiene ------------------------------------------------------------

    #[test]
    fn captured_notes_come_back_sorted() {
        let mut r = Recorder::new();
        r.note_on(960, 72, 100, 0);
        r.note_off(1000, 72, 0);
        r.note_on(0, 60, 100, 0);
        r.note_off(100, 60, 0);
        r.note_on(480, 64, 100, 0);
        r.note_off(500, 64, 0);

        let notes = r.finish(1000);
        assert!(notes.windows(2).all(|w| w[0].order_key() <= w[1].order_key()));
    }

    #[test]
    fn every_captured_note_is_valid_for_the_model() {
        // Whatever comes out is handed straight to the command layer, which validates.
        let mut r = Recorder::new();
        r.set_quantize(240);
        r.note_on(5, 0, 1, 0); // lowest legal pitch and velocity
        r.note_off(6, 0, 0);
        r.note_on(10, 127, 127, 15); // highest legal everything
        r.on_loop_wrap(1920, 0);

        for note in r.finish(500) {
            assert!(note.validate().is_ok(), "invalid note produced: {note:?}");
        }
    }

    #[test]
    fn out_of_range_input_is_masked_rather_than_producing_invalid_notes() {
        // A malformed packet must not be able to create a note the model rejects.
        let mut r = Recorder::new();
        r.note_on(0, 200, 200, 99);
        r.note_off(480, 200, 99);

        let notes = r.finish(480);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].validate().is_ok());
        assert_eq!(notes[0].pitch, 200 & 0x7F);
        assert_eq!(notes[0].channel, 99 & 0x0F);
    }

    #[test]
    fn clear_discards_everything_including_held_notes() {
        let mut r = Recorder::new();
        r.note_on(0, 60, 100, 0);
        r.note_off(480, 60, 0);
        r.note_on(480, 64, 100, 0);

        r.clear();
        assert!(r.is_empty());
        assert!(r.finish(960).is_empty());
    }
}
