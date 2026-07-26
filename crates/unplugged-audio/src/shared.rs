//! State shared between the control thread and the audio thread.
//!
//! The audio thread must never block, allocate, or take a lock, so nothing here is a
//! `Mutex`. Note data is swapped in wholesale via `ArcSwap` (a lock-free atomic pointer
//! swap); transport commands travel as atomics.
//!
//! Commands that are *edges* rather than *states* — a seek is an event, not a
//! condition — carry a generation counter. The audio thread compares the generation it
//! last saw against the current one, so a seek is applied exactly once even though it
//! is transmitted through a plain atomic.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use unplugged_core::sequencer::{RenderedEvent, Sequencer, Timeline};
use unplugged_core::Ticks;

/// Sentinel for "no count-in". A real count-in target is a musical position, never
/// anywhere near `u32::MAX` ticks, so this is unambiguous.
const NO_COUNT_IN: u32 = u32::MAX;

/// Packs a generation into the high 32 bits and a payload into the low 32.
#[inline]
fn pack(generation: u32, payload: u32) -> u64 {
    ((generation as u64) << 32) | payload as u64
}

#[inline]
fn unpack(value: u64) -> (u32, u32) {
    ((value >> 32) as u32, value as u32)
}

/// Everything the audio thread reads and the control thread writes.
pub struct SharedTransport {
    timeline: ArcSwap<Timeline>,
    /// Bumped whenever `timeline` is replaced, so the audio thread knows to re-seat its
    /// cursor rather than diffing the event list.
    timeline_generation: AtomicU32,

    playing: AtomicBool,
    /// `(generation, tick)`.
    seek: AtomicU64,
    /// `f64::to_bits`. `f64` has no atomic type, and the value is only ever read and
    /// written whole, so the bit pattern is the natural carrier.
    tempo_bits: AtomicU64,
    /// `(start, end)`, both `u32`. `0` means looping is disabled — a loop cannot
    /// legitimately be zero-length, so the sentinel is unambiguous.
    loop_region: AtomicU64,

    /// Published by the audio thread for the UI to read. The frontend never drives
    /// timing; it only observes this.
    position_ticks: AtomicU32,
    sounding: AtomicU32,

    metronome: AtomicBool,
    /// Tick before which the timeline is silent. `u32::MAX` means no count-in.
    count_in_until: AtomicU32,
    /// Published by the audio thread so the UI can show the lead-in.
    in_count_in: AtomicBool,
    /// Wrapping count of loop wraps, so the host can drive the recorder's
    /// close-and-reopen without the audio thread calling into it.
    wrap_count: AtomicU32,
}

impl Default for SharedTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedTransport {
    pub fn new() -> Self {
        SharedTransport {
            timeline: ArcSwap::from_pointee(Timeline::default()),
            timeline_generation: AtomicU32::new(0),
            playing: AtomicBool::new(false),
            seek: AtomicU64::new(0),
            tempo_bits: AtomicU64::new(120.0f64.to_bits()),
            loop_region: AtomicU64::new(0),
            position_ticks: AtomicU32::new(0),
            sounding: AtomicU32::new(0),
            metronome: AtomicBool::new(false),
            count_in_until: AtomicU32::new(NO_COUNT_IN),
            in_count_in: AtomicBool::new(false),
            wrap_count: AtomicU32::new(0),
        }
    }

    // -- control thread -----------------------------------------------------

    pub fn set_timeline(&self, timeline: Timeline) {
        self.timeline.store(Arc::new(timeline));
        self.timeline_generation.fetch_add(1, Ordering::Release);
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Release);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    pub fn request_seek(&self, tick: Ticks) {
        let (generation, _) = unpack(self.seek.load(Ordering::Relaxed));
        self.seek.store(pack(generation.wrapping_add(1), tick), Ordering::Release);
    }

    pub fn set_tempo(&self, bpm: f64) {
        self.tempo_bits.store(bpm.to_bits(), Ordering::Release);
    }

    pub fn tempo(&self) -> f64 {
        f64::from_bits(self.tempo_bits.load(Ordering::Acquire))
    }

    pub fn set_loop_region(&self, region: Option<(Ticks, Ticks)>) {
        let packed = match region {
            Some((start, end)) if end > start => pack(start, end),
            _ => 0,
        };
        self.loop_region.store(packed, Ordering::Release);
    }

    pub fn loop_region(&self) -> Option<(Ticks, Ticks)> {
        match self.loop_region.load(Ordering::Acquire) {
            0 => None,
            packed => {
                let (start, end) = unpack(packed);
                Some((start, end))
            }
        }
    }

    pub fn set_metronome(&self, enabled: bool) {
        self.metronome.store(enabled, Ordering::Release);
    }

    pub fn metronome_enabled(&self) -> bool {
        self.metronome.load(Ordering::Acquire)
    }

    /// Suppress timeline events until `tick`, leaving only the click audible.
    pub fn set_count_in_until(&self, tick: Option<Ticks>) {
        self.count_in_until
            .store(tick.unwrap_or(NO_COUNT_IN), Ordering::Release);
    }

    /// True while the playhead is inside a count-in.
    pub fn in_count_in(&self) -> bool {
        self.in_count_in.load(Ordering::Acquire)
    }

    /// Wrapping count of loop wraps. Compare against a previously observed value.
    pub fn wrap_count(&self) -> u32 {
        self.wrap_count.load(Ordering::Acquire)
    }

    /// Playhead position, for the UI. Written by the audio thread.
    pub fn position_ticks(&self) -> Ticks {
        self.position_ticks.load(Ordering::Acquire)
    }

    pub fn sounding_count(&self) -> u32 {
        self.sounding.load(Ordering::Acquire)
    }

    // -- audio thread -------------------------------------------------------

    /// Apply anything the control thread has asked for, then render one buffer.
    ///
    /// `cursor` is owned exclusively by the audio thread. Called once per render
    /// quantum, and must not allocate — `out` is expected to be preallocated and is
    /// reused across calls.
    pub fn render(&self, cursor: &mut AudioCursor, frames: u32, out: &mut Vec<RenderedEvent>) {
        out.clear();

        // Tempo. Cheap to compare, and `set_tempo` preserves musical position.
        let tempo = self.tempo();
        if tempo != cursor.last_tempo && tempo.is_finite() && tempo > 0.0 {
            cursor.sequencer.set_tempo(tempo);
            cursor.last_tempo = tempo;
        }

        // Loop region.
        let region = self.loop_region();
        if region != cursor.last_loop {
            cursor.sequencer.set_loop_region(region);
            cursor.last_loop = region;
        }

        // Metronome and count-in.
        let metronome = self.metronome_enabled();
        if metronome != cursor.sequencer.metronome_enabled() {
            cursor.sequencer.set_metronome(metronome);
        }
        let count_in = match self.count_in_until.load(Ordering::Acquire) {
            NO_COUNT_IN => None,
            tick => Some(tick),
        };
        if count_in != cursor.last_count_in {
            cursor.sequencer.set_count_in_until(count_in);
            cursor.last_count_in = count_in;
        }

        // Note data. Only re-seated when the generation actually moved, so the common
        // case is a single relaxed load.
        let generation = self.timeline_generation.load(Ordering::Acquire);
        if generation != cursor.last_timeline_generation {
            // `ArcSwap::load` is lock-free; cloning the inner `Timeline` would not be,
            // so the sequencer takes it by value from the loaded snapshot.
            let snapshot = self.timeline.load();
            cursor.sequencer.set_timeline(Timeline::clone(&snapshot));
            cursor.last_timeline_generation = generation;
        }

        // Seek, applied at most once per issued request.
        let (seek_generation, tick) = unpack(self.seek.load(Ordering::Acquire));
        if seek_generation != cursor.last_seek_generation {
            cursor.sequencer.seek(tick, out);
            cursor.last_seek_generation = seek_generation;
        }

        // Transport. A stop must flush held notes, so it is handled as an edge.
        let playing = self.is_playing();
        if playing != cursor.sequencer.is_playing() {
            if playing {
                cursor.sequencer.play();
            } else {
                cursor.sequencer.stop(out);
            }
        }

        // `render` clears `out`, so anything already emitted above (seek/stop releases)
        // has to be preserved across the call.
        let carried = out.len();
        if carried == 0 {
            cursor.sequencer.render(frames, out);
        } else {
            cursor.scratch.clear();
            cursor.sequencer.render(frames, &mut cursor.scratch);
            out.extend_from_slice(&cursor.scratch);
        }

        self.position_ticks
            .store(cursor.sequencer.position_ticks(), Ordering::Release);
        self.in_count_in
            .store(cursor.sequencer.in_count_in(), Ordering::Release);
        self.wrap_count
            .store(cursor.sequencer.wrap_count(), Ordering::Release);
        self.sounding
            .store(cursor.sequencer.sounding_count() as u32, Ordering::Release);
    }
}

/// The audio thread's private playback state.
///
/// Deliberately not `Sync`: exactly one thread may own it, and that is enforced by
/// passing it as `&mut` from the single render entry point.
pub struct AudioCursor {
    sequencer: Sequencer,
    scratch: Vec<RenderedEvent>,
    last_tempo: f64,
    last_loop: Option<(Ticks, Ticks)>,
    last_timeline_generation: u32,
    last_seek_generation: u32,
    last_count_in: Option<Ticks>,
}

impl AudioCursor {
    pub fn new(sample_rate: f64, ppq: u16, tempo_bpm: f64) -> Self {
        AudioCursor {
            sequencer: Sequencer::new(sample_rate, ppq, tempo_bpm),
            // Preallocated: the audio thread must not allocate mid-render.
            scratch: Vec::with_capacity(1024),
            last_tempo: tempo_bpm,
            last_loop: None,
            last_timeline_generation: 0,
            last_seek_generation: 0,
            last_count_in: None,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sequencer.set_sample_rate(sample_rate);
    }

    pub fn set_ppq(&mut self, ppq: u16) {
        self.sequencer.set_ppq(ppq);
    }

    pub fn set_time_signature(&mut self, ts: unplugged_core::TimeSignature) {
        self.sequencer.set_time_signature(ts);
    }

    pub fn position_ticks(&self) -> Ticks {
        self.sequencer.position_ticks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unplugged_core::sequencer::{EventKind, TimelineEvent};

    const SR: f64 = 48_000.0;
    const PPQ: u16 = 480;

    fn ev(tick: Ticks, kind: EventKind, pitch: u8) -> TimelineEvent {
        TimelineEvent { tick, kind, track: 0, pitch, velocity: 100, channel: 0 }
    }

    fn setup() -> (SharedTransport, AudioCursor, Vec<RenderedEvent>) {
        (
            SharedTransport::new(),
            AudioCursor::new(SR, PPQ, 120.0),
            Vec::with_capacity(256),
        )
    }

    #[test]
    fn a_timeline_set_before_playback_is_picked_up_by_the_audio_thread() {
        let (shared, mut cursor, mut out) = setup();
        shared.set_timeline(Timeline::new(vec![ev(2, EventKind::NoteOn, 60)]));
        shared.set_playing(true);

        shared.render(&mut cursor, 512, &mut out);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].frame_offset, 100);
    }

    #[test]
    fn a_seek_is_applied_exactly_once() {
        let (shared, mut cursor, mut out) = setup();
        shared.set_timeline(Timeline::new(vec![ev(480, EventKind::NoteOn, 72)]));
        shared.set_playing(true);
        shared.request_seek(480);

        shared.render(&mut cursor, 64, &mut out);
        assert_eq!(out.len(), 1, "the note at the seek target fires");
        assert_eq!(out[0].pitch, 72);

        // A second render with no new request must not re-seek back to 480.
        shared.render(&mut cursor, 64, &mut out);
        assert!(out.is_empty(), "the seek must not repeat");
        assert!(shared.position_ticks() > 480);
    }

    #[test]
    fn stopping_flushes_held_notes_through_the_shared_layer() {
        let (shared, mut cursor, mut out) = setup();
        shared.set_timeline(Timeline::new(vec![
            ev(0, EventKind::NoteOn, 60),
            ev(9600, EventKind::NoteOff, 60),
        ]));
        shared.set_playing(true);
        shared.render(&mut cursor, 512, &mut out);
        assert_eq!(shared.sounding_count(), 1);

        shared.set_playing(false);
        shared.render(&mut cursor, 512, &mut out);

        assert!(
            out.iter().any(|e| e.kind == EventKind::NoteOff && e.pitch == 60),
            "the release must survive into the caller's buffer"
        );
        assert_eq!(shared.sounding_count(), 0);
    }

    #[test]
    fn events_emitted_during_a_seek_are_not_lost_to_the_render_that_follows() {
        // `Sequencer::render` clears its output buffer, so a naive implementation
        // discards the note-offs the seek just produced.
        let (shared, mut cursor, mut out) = setup();
        shared.set_timeline(Timeline::new(vec![
            ev(0, EventKind::NoteOn, 60),
            ev(9600, EventKind::NoteOff, 60),
            ev(500, EventKind::NoteOn, 80),
        ]));
        shared.set_playing(true);
        shared.render(&mut cursor, 512, &mut out);
        assert_eq!(shared.sounding_count(), 1, "note 60 is held");

        shared.request_seek(500);
        shared.render(&mut cursor, 512, &mut out);

        assert!(
            out.iter().any(|e| e.kind == EventKind::NoteOff && e.pitch == 60),
            "the seek's release must not be clobbered by the subsequent render"
        );
        assert!(
            out.iter().any(|e| e.kind == EventKind::NoteOn && e.pitch == 80),
            "and the note at the seek target must still fire"
        );
    }

    #[test]
    fn tempo_changes_reach_the_audio_thread() {
        let (shared, mut cursor, mut out) = setup();
        shared.set_playing(true);
        shared.render(&mut cursor, 4800, &mut out);
        assert_eq!(shared.position_ticks(), 96);

        shared.set_tempo(240.0);
        shared.render(&mut cursor, 4800, &mut out);
        // Twice the tempo covers twice the ticks in the same number of samples.
        assert_eq!(shared.position_ticks(), 96 + 192);
    }

    #[test]
    fn loop_region_round_trips_and_rejects_degenerate_input() {
        let shared = SharedTransport::new();
        assert_eq!(shared.loop_region(), None);

        shared.set_loop_region(Some((100, 500)));
        assert_eq!(shared.loop_region(), Some((100, 500)));

        shared.set_loop_region(Some((500, 500)));
        assert_eq!(shared.loop_region(), None, "zero-length must not become a real loop");

        shared.set_loop_region(Some((900, 100)));
        assert_eq!(shared.loop_region(), None, "inverted must not become a real loop");
    }

    #[test]
    fn replacing_the_timeline_while_playing_is_picked_up() {
        let (shared, mut cursor, mut out) = setup();
        shared.set_playing(true);
        shared.render(&mut cursor, 4800, &mut out); // to tick 96

        shared.set_timeline(Timeline::new(vec![ev(200, EventKind::NoteOn, 80)]));
        shared.render(&mut cursor, 48_000, &mut out);

        assert!(out.iter().any(|e| e.pitch == 80), "the new timeline must take effect");
    }

    #[test]
    fn generation_counters_survive_wrapping() {
        // u32 generations wrap; the comparison is equality, not ordering, so this is
        // fine — but only if `request_seek` uses wrapping arithmetic.
        let shared = SharedTransport::new();
        shared.seek.store(pack(u32::MAX, 0), Ordering::Release);
        shared.request_seek(123);
        let (generation, tick) = unpack(shared.seek.load(Ordering::Acquire));
        assert_eq!(generation, 0, "must wrap rather than overflow-panic");
        assert_eq!(tick, 123);
    }
}
