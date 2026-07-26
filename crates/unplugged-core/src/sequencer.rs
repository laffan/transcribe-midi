//! The playback engine's timing core.
//!
//! This module is deliberately pure: no atomics, no FFI, no platform types. The audio
//! thread owns a [`Sequencer`] and calls [`Sequencer::render`] once per render quantum;
//! everything about *when* a note fires is decided here.
//!
//! That purity is the point. The Swift audio graph cannot be compiled or run on a
//! non-Apple host, so if tick-to-sample conversion and loop wrapping lived over there
//! they would be untestable. Here they are covered by unit tests that run anywhere.

use crate::model::{Project, Ticks};

/// Maximum simultaneously-sounding notes tracked for all-notes-off purposes.
///
/// The audio thread must not allocate, so this is a hard, preallocated ceiling rather
/// than a growable list. 512 is far beyond what a MIDI track realistically sounds at
/// once; notes beyond it still play, they just are not tracked for the panic-off.
pub const MAX_SOUNDING: usize = 512;

/// Track index reserved for metronome clicks.
///
/// The audio backend routes this to its own dedicated sampler rather than a project
/// track, so the click is never affected by track mute, solo or gain — and never turns
/// up in an exported file.
pub const METRONOME_TRACK: u16 = u16::MAX;

/// Bar-start click. Higher than the off-beat so downbeats are unmistakable.
pub const METRONOME_DOWNBEAT_PITCH: u8 = 84;
pub const METRONOME_BEAT_PITCH: u8 = 76;
pub const METRONOME_VELOCITY: u8 = 110;

/// How long a click sounds, in ticks. Short enough to read as percussive at any tempo.
const METRONOME_CLICK_TICKS: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    NoteOff,
    NoteOn,
}

/// A note boundary at an absolute position on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineEvent {
    pub tick: Ticks,
    pub kind: EventKind,
    /// Index into the project's track list.
    pub track: u16,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

impl TimelineEvent {
    /// Sort key. `NoteOff` sorts before `NoteOn` at the same tick so that a note
    /// immediately followed by another of the same pitch retriggers rather than being
    /// silenced by the previous note's off arriving afterwards.
    fn order_key(&self) -> (Ticks, u8, u8, u8) {
        let kind_rank = match self.kind {
            EventKind::NoteOff => 0,
            EventKind::NoteOn => 1,
        };
        (self.tick, kind_rank, self.pitch, self.channel)
    }
}

/// An event placed within the current render buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderedEvent {
    /// Sample offset from the start of this buffer. This is what makes output
    /// sample-accurate rather than buffer-quantised.
    pub frame_offset: u32,
    pub kind: EventKind,
    pub track: u16,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

/// The flattened, sorted note events for a whole project.
///
/// Built on the control thread and handed to the audio thread wholesale, so that the
/// audio thread never walks the project structure or allocates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    events: Vec<TimelineEvent>,
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
    fn seek_index(&self, tick: Ticks) -> usize {
        self.events.partition_point(|e| e.tick < tick)
    }
}

// ---------------------------------------------------------------------------
// Sequencer
// ---------------------------------------------------------------------------

/// A note currently sounding, tracked so it can be silenced on stop, seek or loop wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sounding {
    track: u16,
    pitch: u8,
    channel: u8,
}

/// Owned and driven by the audio thread.
///
/// The cursor is kept in **samples**, not ticks: samples are what the audio clock
/// actually counts, and deriving ticks from samples (rather than the reverse) means
/// rounding error cannot accumulate across buffers.
#[derive(Debug, Clone)]
pub struct Sequencer {
    timeline: Timeline,
    ppq: u16,
    tempo_bpm: f64,
    sample_rate: f64,

    position_samples: f64,
    next_event: usize,
    playing: bool,

    loop_region: Option<(Ticks, Ticks)>,
    sounding: Vec<Sounding>,

    metronome_enabled: bool,
    /// Ticks per beat and beats per bar, from the project's time signature.
    beat_ticks: u32,
    beats_per_bar: u32,
    /// Absolute sample position at which the sounding click is released, and its pitch.
    click_off: Option<(f64, u8)>,

    /// While the playhead is before this tick, timeline events are suppressed and only
    /// the metronome sounds. `None` means no count-in is active.
    count_in_until: Option<Ticks>,

    /// Incremented on every loop wrap. The host polls this to tell the recorder to
    /// close and reopen held notes — the audio thread cannot call into it directly.
    wrap_count: u32,
}

impl Sequencer {
    pub fn new(sample_rate: f64, ppq: u16, tempo_bpm: f64) -> Self {
        Sequencer {
            timeline: Timeline::default(),
            ppq: ppq.max(1),
            tempo_bpm: tempo_bpm.max(1.0),
            sample_rate: sample_rate.max(1.0),
            position_samples: 0.0,
            next_event: 0,
            playing: false,
            loop_region: None,
            // Preallocated so that `push` on the audio thread never allocates.
            sounding: Vec::with_capacity(MAX_SOUNDING),
            metronome_enabled: false,
            beat_ticks: ppq.max(1) as u32,
            beats_per_bar: 4,
            click_off: None,
            count_in_until: None,
            wrap_count: 0,
        }
    }

    /// Set the metronome grid from the project's time signature.
    pub fn set_time_signature(&mut self, time_signature: crate::model::TimeSignature) {
        self.beat_ticks = ((self.ppq as u32 * 4) / time_signature.denominator.max(1) as u32).max(1);
        self.beats_per_bar = time_signature.numerator.max(1) as u32;
    }

    pub fn set_metronome(&mut self, enabled: bool) {
        self.metronome_enabled = enabled;
    }

    pub fn metronome_enabled(&self) -> bool {
        self.metronome_enabled
    }

    /// Suppress timeline events until `tick`, leaving only the metronome audible.
    pub fn set_count_in_until(&mut self, tick: Option<Ticks>) {
        self.count_in_until = tick;
    }

    /// True while the playhead is inside a count-in.
    pub fn in_count_in(&self) -> bool {
        matches!(self.count_in_until, Some(until) if self.position_ticks() < until)
    }

    // -- configuration ------------------------------------------------------

    pub fn set_timeline(&mut self, timeline: Timeline) {
        self.timeline = timeline;
        // The event list changed under us; re-derive the cursor from the position we
        // are actually at rather than trusting the old index.
        self.next_event = self.timeline.seek_index(self.position_ticks());
    }

    pub fn set_tempo(&mut self, bpm: f64) {
        if !bpm.is_finite() || bpm <= 0.0 {
            return;
        }
        // Preserve musical position across a tempo change: hold the tick fixed and
        // recompute the sample cursor. Holding samples fixed instead would make the
        // playhead jump on the timeline.
        let tick = self.position_ticks_f64();
        self.tempo_bpm = bpm;
        self.position_samples = tick * self.samples_per_tick();
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return;
        }
        let tick = self.position_ticks_f64();
        self.sample_rate = sample_rate;
        self.position_samples = tick * self.samples_per_tick();
    }

    pub fn set_ppq(&mut self, ppq: u16) {
        if ppq == 0 {
            return;
        }
        let tick = self.position_ticks_f64();
        self.ppq = ppq;
        self.position_samples = tick * self.samples_per_tick();
    }

    /// `None` disables looping. An empty or inverted region is rejected rather than
    /// silently accepted — a zero-length loop would spin forever.
    pub fn set_loop_region(&mut self, region: Option<(Ticks, Ticks)>) {
        self.loop_region = match region {
            Some((start, end)) if end > start => Some((start, end)),
            _ => None,
        };
    }

    pub fn loop_region(&self) -> Option<(Ticks, Ticks)> {
        self.loop_region
    }

    // -- transport ----------------------------------------------------------

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Stop and return the note-offs needed to silence anything still sounding.
    ///
    /// Returning them rather than dropping them is what stops a stuck note when the
    /// user hits stop mid-chord.
    pub fn stop(&mut self, out: &mut Vec<RenderedEvent>) {
        self.playing = false;
        self.flush_sounding(0, out);
    }

    /// Move the playhead. Silences anything sounding, since those notes' off events are
    /// no longer ahead of the cursor.
    pub fn seek(&mut self, tick: Ticks, out: &mut Vec<RenderedEvent>) {
        self.flush_sounding(0, out);
        self.position_samples = tick as f64 * self.samples_per_tick();
        self.next_event = self.timeline.seek_index(tick);
    }

    pub fn samples_per_tick(&self) -> f64 {
        // 60 s/min ÷ (beats/min × ticks/beat) = seconds per tick.
        (self.sample_rate * 60.0) / (self.tempo_bpm * self.ppq as f64)
    }

    fn position_ticks_f64(&self) -> f64 {
        self.position_samples / self.samples_per_tick()
    }

    pub fn position_ticks(&self) -> Ticks {
        let ticks = self.position_ticks_f64();
        if ticks <= 0.0 {
            0
        } else {
            ticks.min(Ticks::MAX as f64) as Ticks
        }
    }

    pub fn sounding_count(&self) -> usize {
        self.sounding.len()
    }

    /// Monotonic (wrapping) count of loop wraps performed so far.
    pub fn wrap_count(&self) -> u32 {
        self.wrap_count
    }

    // -- rendering ----------------------------------------------------------

    /// Produce every event that falls inside the next `frames` samples.
    ///
    /// `out` is cleared first and is expected to be a reused, preallocated buffer —
    /// the audio thread must not allocate.
    pub fn render(&mut self, frames: u32, out: &mut Vec<RenderedEvent>) {
        out.clear();
        if !self.playing || frames == 0 {
            return;
        }

        let spt = self.samples_per_tick();
        let mut remaining = frames;
        let mut buffer_offset: u32 = 0;

        while remaining > 0 {
            // How far can we go before the loop end forces a jump?
            let segment = match self.loop_region {
                Some((_, loop_end)) => {
                    let loop_end_samples = loop_end as f64 * spt;
                    let to_loop_end = loop_end_samples - self.position_samples;

                    if to_loop_end <= 0.0 {
                        // Already at or past the loop end (e.g. the region moved while
                        // playing). Wrap immediately rather than emitting anything.
                        self.wrap_to_loop_start(buffer_offset, out);
                        continue;
                    }

                    if to_loop_end < remaining as f64 {
                        // The wrap happens inside this buffer. Render up to it, then
                        // jump and carry on filling the same buffer.
                        to_loop_end.floor().max(0.0) as u32
                    } else {
                        remaining
                    }
                }
                None => remaining,
            };

            let segment = segment.min(remaining);
            self.emit_segment(segment, buffer_offset, out);

            self.position_samples += segment as f64;
            buffer_offset += segment;
            remaining -= segment;

            if remaining > 0 {
                // We stopped short, which only happens at a loop boundary.
                self.wrap_to_loop_start(buffer_offset, out);
            }
        }
    }

    /// Emit events falling in `[position, position + segment)`.
    fn emit_segment(&mut self, segment: u32, buffer_offset: u32, out: &mut Vec<RenderedEvent>) {
        if segment == 0 {
            return;
        }

        let spt = self.samples_per_tick();
        let window_start = self.position_samples;
        let window_end = window_start + segment as f64;

        // During a count-in only the click sounds. The cursor still advances past any
        // timeline events in the window, so playback picks up cleanly at the record
        // point rather than replaying the count-in bars' worth of notes all at once.
        let silent = self.count_in_active();

        while let Some(event) = self.timeline.events.get(self.next_event) {
            let event_samples = event.tick as f64 * spt;
            if event_samples >= window_end {
                break;
            }

            if !silent {
                // An event slightly behind the cursor (possible right after a seek lands
                // mid-tick) is emitted at offset 0 rather than dropped.
                let offset_in_segment = (event_samples - window_start).max(0.0) as u32;
                let frame_offset = buffer_offset + offset_in_segment.min(segment.saturating_sub(1));

                let event = *event;
                self.track_sounding(event);
                out.push(RenderedEvent {
                    frame_offset,
                    kind: event.kind,
                    track: event.track,
                    pitch: event.pitch,
                    velocity: event.velocity,
                    channel: event.channel,
                });
            }

            self.next_event += 1;
        }

        self.emit_metronome(segment, buffer_offset, out);
    }

    fn count_in_active(&self) -> bool {
        match self.count_in_until {
            Some(until) => self.position_samples < until as f64 * self.samples_per_tick(),
            None => false,
        }
    }

    /// Emit metronome clicks for beats falling inside this segment.
    ///
    /// Windows are half-open `[start, end)`, so a beat landing exactly on a segment
    /// boundary fires once — in the following segment — rather than twice. That matters
    /// because a loop wrap splits a buffer into two segments.
    fn emit_metronome(&mut self, segment: u32, buffer_offset: u32, out: &mut Vec<RenderedEvent>) {
        let window_start = self.position_samples;
        let window_end = window_start + segment as f64;
        let last_frame = segment.saturating_sub(1);

        // Release a click that started in an earlier buffer.
        if let Some((off_at, pitch)) = self.click_off {
            if off_at < window_end {
                let offset = ((off_at - window_start).max(0.0) as u32).min(last_frame);
                out.push(RenderedEvent {
                    frame_offset: buffer_offset + offset,
                    kind: EventKind::NoteOff,
                    track: METRONOME_TRACK,
                    pitch,
                    velocity: 64,
                    channel: 0,
                });
                self.click_off = None;
            }
        }

        if !self.metronome_enabled {
            return;
        }

        let spt = self.samples_per_tick();
        let beat_samples = self.beat_ticks as f64 * spt;
        // NaN would make the loop below never terminate, so the guard is written to
        // reject it explicitly rather than relying on a negated comparison.
        if !beat_samples.is_finite() || beat_samples <= 0.0 {
            return;
        }

        let mut beat = (window_start / beat_samples).ceil() as i64;
        loop {
            let beat_at = beat as f64 * beat_samples;
            if beat_at >= window_end {
                break;
            }
            if beat_at < window_start {
                beat += 1;
                continue;
            }

            let downbeat = beat.rem_euclid(self.beats_per_bar.max(1) as i64) == 0;
            let pitch = if downbeat { METRONOME_DOWNBEAT_PITCH } else { METRONOME_BEAT_PITCH };
            let offset = ((beat_at - window_start).max(0.0) as u32).min(last_frame);

            // A click still sounding from the previous beat is released first, so very
            // fast tempi cannot stack clicks on the dedicated sampler.
            if let Some((_, previous)) = self.click_off.take() {
                out.push(RenderedEvent {
                    frame_offset: buffer_offset + offset,
                    kind: EventKind::NoteOff,
                    track: METRONOME_TRACK,
                    pitch: previous,
                    velocity: 64,
                    channel: 0,
                });
            }

            out.push(RenderedEvent {
                frame_offset: buffer_offset + offset,
                kind: EventKind::NoteOn,
                track: METRONOME_TRACK,
                pitch,
                velocity: METRONOME_VELOCITY,
                channel: 0,
            });
            self.click_off = Some((beat_at + METRONOME_CLICK_TICKS * spt, pitch));

            beat += 1;
        }
    }

    fn wrap_to_loop_start(&mut self, buffer_offset: u32, out: &mut Vec<RenderedEvent>) {
        let Some((loop_start, _)) = self.loop_region else {
            return;
        };
        // Anything still held would hang across the jump, because its note-off lives
        // after the loop end and we are about to skip past it.
        self.flush_sounding(buffer_offset, out);
        self.wrap_count = self.wrap_count.wrapping_add(1);
        self.position_samples = loop_start as f64 * self.samples_per_tick();
        self.next_event = self.timeline.seek_index(loop_start);
    }

    fn track_sounding(&mut self, event: TimelineEvent) {
        match event.kind {
            EventKind::NoteOn => {
                if self.sounding.len() < MAX_SOUNDING {
                    self.sounding.push(Sounding {
                        track: event.track,
                        pitch: event.pitch,
                        channel: event.channel,
                    });
                }
            }
            EventKind::NoteOff => {
                if let Some(index) = self.sounding.iter().position(|s| {
                    s.track == event.track && s.pitch == event.pitch && s.channel == event.channel
                }) {
                    self.sounding.swap_remove(index);
                }
            }
        }
    }

    /// Emit a note-off for everything currently sounding and clear the list.
    ///
    /// Includes any metronome click still ringing — otherwise stopping the transport
    /// mid-click leaves the click hanging, which is the same stuck-note bug as for a
    /// held note, just more annoying.
    fn flush_sounding(&mut self, frame_offset: u32, out: &mut Vec<RenderedEvent>) {
        for s in self.sounding.drain(..) {
            out.push(RenderedEvent {
                frame_offset,
                kind: EventKind::NoteOff,
                track: s.track,
                pitch: s.pitch,
                velocity: 64,
                channel: s.channel,
            });
        }

        if let Some((_, pitch)) = self.click_off.take() {
            out.push(RenderedEvent {
                frame_offset,
                kind: EventKind::NoteOff,
                track: METRONOME_TRACK,
                pitch,
                velocity: 64,
                channel: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InstrumentRef, Note, ProjectManifest, TimeSignature, Track, TrackMeta};
    use crate::model::{DEFAULT_PPQ, SCHEMA_VERSION};

    const SR: f64 = 48_000.0;

    fn seq() -> Sequencer {
        Sequencer::new(SR, DEFAULT_PPQ, 120.0)
    }

    fn ev(tick: Ticks, kind: EventKind, pitch: u8) -> TimelineEvent {
        TimelineEvent { tick, kind, track: 0, pitch, velocity: 100, channel: 0 }
    }

    fn track(id: &str, notes: Vec<Note>, muted: bool, soloed: bool) -> Track {
        Track {
            meta: TrackMeta {
                id: id.into(), name: id.into(), channel: 0,
                instrument: InstrumentRef::BuiltInSampler, muted, soloed,
                color: "#fff".into(), key_hint: None,
            },
            notes,
            ppq: DEFAULT_PPQ,
        }
    }

    fn project(tracks: Vec<Track>) -> Project {
        Project {
            manifest: ProjectManifest {
                schema_version: SCHEMA_VERSION,
                id: "p".into(), name: "P".into(),
                tempo_bpm: 120.0, time_signature: TimeSignature::default(),
                ppq: DEFAULT_PPQ,
                tracks: tracks.iter().map(|t| t.meta.clone()).collect(),
                created_at_ms: 0, modified_at_ms: 0,
            },
            tracks,
        }
    }

    // -- timing ------------------------------------------------------------

    #[test]
    fn samples_per_tick_matches_the_arithmetic() {
        // 120 bpm, 480 ppq, 48 kHz: one beat is 0.5 s = 24000 samples,
        // so one tick is 24000 / 480 = 50 samples.
        assert_eq!(seq().samples_per_tick(), 50.0);
    }

    #[test]
    fn nothing_is_emitted_while_stopped() {
        let mut s = seq();
        s.set_timeline(Timeline::new(vec![ev(0, EventKind::NoteOn, 60)]));
        let mut out = Vec::new();
        s.render(512, &mut out);
        assert!(out.is_empty(), "a stopped sequencer must emit nothing");
        assert_eq!(s.position_ticks(), 0, "and must not advance");
    }

    #[test]
    fn events_land_on_their_exact_sample_offset() {
        let mut s = seq();
        // 50 samples per tick: tick 2 -> 100, tick 5 -> 250.
        s.set_timeline(Timeline::new(vec![
            ev(2, EventKind::NoteOn, 60),
            ev(5, EventKind::NoteOn, 64),
        ]));
        s.play();

        let mut out = Vec::new();
        s.render(512, &mut out);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].frame_offset, 100);
        assert_eq!(out[0].pitch, 60);
        assert_eq!(out[1].frame_offset, 250);
        assert_eq!(out[1].pitch, 64);
    }

    #[test]
    fn an_event_beyond_the_buffer_waits_for_the_next_one() {
        let mut s = seq();
        s.set_timeline(Timeline::new(vec![ev(20, EventKind::NoteOn, 60)])); // sample 1000
        s.play();

        let mut out = Vec::new();
        s.render(512, &mut out);
        assert!(out.is_empty(), "sample 1000 is outside the first 512-frame buffer");

        s.render(512, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].frame_offset, 1000 - 512, "offset is relative to this buffer");
    }

    #[test]
    fn an_event_exactly_on_a_buffer_boundary_is_not_lost_or_doubled() {
        let mut s = seq();
        // Tick 10 is sample 500. With 500-frame buffers it sits exactly on the seam.
        s.set_timeline(Timeline::new(vec![ev(10, EventKind::NoteOn, 60)]));
        s.play();

        let mut out = Vec::new();
        s.render(500, &mut out);
        assert!(out.is_empty(), "sample 500 is exclusive of the [0,500) window");

        s.render(500, &mut out);
        assert_eq!(out.len(), 1, "and must appear exactly once in the next buffer");
        assert_eq!(out[0].frame_offset, 0);
    }

    #[test]
    fn position_tracks_elapsed_samples() {
        let mut s = seq();
        s.play();
        let mut out = Vec::new();
        for _ in 0..10 {
            s.render(480, &mut out); // 4800 samples total = 96 ticks
        }
        assert_eq!(s.position_ticks(), 96);
    }

    // -- ordering ----------------------------------------------------------

    #[test]
    fn note_off_precedes_note_on_at_the_same_tick() {
        // Otherwise a repeated note is cut off by the previous note's release.
        let timeline = Timeline::new(vec![
            ev(480, EventKind::NoteOn, 60),
            ev(480, EventKind::NoteOff, 60),
        ]);
        assert_eq!(timeline.events()[0].kind, EventKind::NoteOff);
        assert_eq!(timeline.events()[1].kind, EventKind::NoteOn);
    }

    // -- looping -----------------------------------------------------------

    #[test]
    fn playback_wraps_at_the_loop_end_within_a_single_buffer() {
        let mut s = seq();
        // Loop ticks 0..10 (samples 0..500). Note at tick 2 (sample 100).
        s.set_timeline(Timeline::new(vec![ev(2, EventKind::NoteOn, 60)]));
        s.set_loop_region(Some((0, 10)));
        s.play();

        let mut out = Vec::new();
        s.render(1200, &mut out); // spans two full loops plus part of a third

        let ons: Vec<u32> = out
            .iter()
            .filter(|e| e.kind == EventKind::NoteOn)
            .map(|e| e.frame_offset)
            .collect();
        assert_eq!(ons, vec![100, 600, 1100], "the note must fire once per loop pass");
    }

    #[test]
    fn a_note_held_across_the_loop_end_is_released_rather_than_hanging() {
        let mut s = seq();
        // Note on at tick 2, off at tick 40 — but the loop ends at tick 10, so the
        // off event is never reached. Without an explicit release this note hangs.
        s.set_timeline(Timeline::new(vec![
            ev(2, EventKind::NoteOn, 60),
            ev(40, EventKind::NoteOff, 60),
        ]));
        s.set_loop_region(Some((0, 10)));
        s.play();

        let mut out = Vec::new();
        s.render(600, &mut out);

        let offs: Vec<&RenderedEvent> = out.iter().filter(|e| e.kind == EventKind::NoteOff).collect();
        assert_eq!(offs.len(), 1, "exactly one release at the wrap");
        assert_eq!(offs[0].pitch, 60);
        assert_eq!(offs[0].frame_offset, 500, "released at the loop boundary");
    }

    #[test]
    fn a_degenerate_loop_region_is_rejected_not_obeyed() {
        let mut s = seq();
        s.set_loop_region(Some((100, 100)));
        assert_eq!(s.loop_region(), None, "a zero-length loop would spin forever");
        s.set_loop_region(Some((200, 100)));
        assert_eq!(s.loop_region(), None, "an inverted loop is meaningless");
    }

    #[test]
    fn render_terminates_even_with_a_loop_shorter_than_the_buffer() {
        let mut s = seq();
        s.set_timeline(Timeline::new(vec![ev(0, EventKind::NoteOn, 60)]));
        s.set_loop_region(Some((0, 1))); // 50 samples, far shorter than the buffer
        s.play();

        let mut out = Vec::new();
        s.render(4096, &mut out); // must not hang
        assert!(out.len() > 1, "the loop should have wrapped many times");
    }

    // -- transport ---------------------------------------------------------

    #[test]
    fn stopping_mid_note_releases_it() {
        let mut s = seq();
        s.set_timeline(Timeline::new(vec![
            ev(0, EventKind::NoteOn, 60),
            ev(1000, EventKind::NoteOff, 60),
        ]));
        s.play();

        let mut out = Vec::new();
        s.render(512, &mut out);
        assert_eq!(s.sounding_count(), 1);

        s.stop(&mut out);
        assert!(!s.is_playing());
        assert_eq!(s.sounding_count(), 0);
        assert!(
            out.iter().any(|e| e.kind == EventKind::NoteOff && e.pitch == 60),
            "stop must release the held note"
        );
    }

    #[test]
    fn seeking_releases_held_notes_and_repositions_the_cursor() {
        let mut s = seq();
        s.set_timeline(Timeline::new(vec![
            ev(0, EventKind::NoteOn, 60),
            ev(1000, EventKind::NoteOff, 60),
            ev(480, EventKind::NoteOn, 72),
        ]));
        s.play();

        let mut out = Vec::new();
        s.render(512, &mut out);
        assert_eq!(s.sounding_count(), 1);

        s.seek(480, &mut out);
        assert_eq!(s.position_ticks(), 480);
        assert_eq!(s.sounding_count(), 0, "the held note must not survive the jump");

        out.clear();
        s.render(64, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pitch, 72, "playback resumes from the new position");
    }

    #[test]
    fn tempo_change_preserves_musical_position() {
        let mut s = seq();
        s.play();
        let mut out = Vec::new();
        s.render(4800, &mut out); // 96 ticks at 120 bpm
        assert_eq!(s.position_ticks(), 96);

        s.set_tempo(240.0);
        assert_eq!(s.position_ticks(), 96, "the playhead must not jump on the timeline");
        assert_eq!(s.samples_per_tick(), 25.0, "but ticks now pass twice as fast");
    }

    #[test]
    fn invalid_tempo_and_sample_rate_are_ignored_rather_than_poisoning_the_clock() {
        let mut s = seq();
        let before = s.samples_per_tick();
        s.set_tempo(0.0);
        s.set_tempo(-5.0);
        s.set_tempo(f64::NAN);
        s.set_sample_rate(0.0);
        s.set_ppq(0);
        assert_eq!(s.samples_per_tick(), before);
        assert!(s.samples_per_tick().is_finite());
    }

    // -- timeline construction ---------------------------------------------

    #[test]
    fn from_project_emits_a_pair_per_note() {
        let p = project(vec![track(
            "t1",
            vec![Note::new(60, 0, 480, 100, 0).unwrap(), Note::new(64, 480, 480, 90, 0).unwrap()],
            false,
            false,
        )]);
        let timeline = Timeline::from_project(&p);
        assert_eq!(timeline.events().len(), 4);
        assert_eq!(timeline.length_ticks(), 960);
    }

    #[test]
    fn muted_tracks_are_excluded() {
        let p = project(vec![
            track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], false, false),
            track("b", vec![Note::new(72, 0, 480, 100, 0).unwrap()], true, false),
        ]);
        let timeline = Timeline::from_project(&p);
        assert!(timeline.events().iter().all(|e| e.pitch == 60));
    }

    #[test]
    fn solo_overrides_mute_on_other_tracks() {
        // Track b is soloed; track a is not muted but must still fall silent.
        let p = project(vec![
            track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], false, false),
            track("b", vec![Note::new(72, 0, 480, 100, 0).unwrap()], false, true),
        ]);
        let timeline = Timeline::from_project(&p);
        assert!(!timeline.is_empty());
        assert!(
            timeline.events().iter().all(|e| e.pitch == 72),
            "only the soloed track should sound"
        );
    }

    #[test]
    fn a_track_that_is_both_muted_and_soloed_still_sounds() {
        // Solo wins, matching every DAW's behaviour.
        let p = project(vec![track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], true, true)]);
        assert!(!Timeline::from_project(&p).is_empty());
    }

    #[test]
    fn track_index_is_carried_through_so_events_can_be_routed() {
        let p = project(vec![
            track("a", vec![Note::new(60, 0, 480, 100, 0).unwrap()], false, false),
            track("b", vec![Note::new(72, 0, 480, 100, 1).unwrap()], false, false),
        ]);
        let timeline = Timeline::from_project(&p);
        let second: Vec<_> = timeline.events().iter().filter(|e| e.track == 1).collect();
        assert_eq!(second.len(), 2);
        assert!(second.iter().all(|e| e.pitch == 72 && e.channel == 1));
    }

    // -- metronome ---------------------------------------------------------

    fn clicks(out: &[RenderedEvent]) -> Vec<(u32, u8)> {
        out.iter()
            .filter(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOn)
            .map(|e| (e.frame_offset, e.pitch))
            .collect()
    }

    #[test]
    fn the_metronome_clicks_on_every_beat() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default()); // 4/4
        s.set_metronome(true);
        s.play();

        // 120 bpm, 480 ppq, 48 kHz: a beat is 24000 samples. One bar = 96000.
        let mut out = Vec::new();
        s.render(96_000, &mut out);

        let offsets: Vec<u32> = clicks(&out).iter().map(|(o, _)| *o).collect();
        assert_eq!(offsets, vec![0, 24_000, 48_000, 72_000]);
    }

    #[test]
    fn the_downbeat_is_a_different_pitch_from_the_other_beats() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default());
        s.set_metronome(true);
        s.play();

        let mut out = Vec::new();
        s.render(96_000, &mut out);

        let pitches: Vec<u8> = clicks(&out).iter().map(|(_, p)| *p).collect();
        assert_eq!(
            pitches,
            vec![
                METRONOME_DOWNBEAT_PITCH,
                METRONOME_BEAT_PITCH,
                METRONOME_BEAT_PITCH,
                METRONOME_BEAT_PITCH
            ]
        );
    }

    #[test]
    fn the_bar_length_follows_the_time_signature() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::new(3, 4).unwrap());
        s.set_metronome(true);
        s.play();

        let mut out = Vec::new();
        s.render(6 * 24_000, &mut out); // two bars of 3/4

        let downbeats: Vec<u32> = clicks(&out)
            .iter()
            .filter(|(_, p)| *p == METRONOME_DOWNBEAT_PITCH)
            .map(|(o, _)| *o)
            .collect();
        assert_eq!(downbeats, vec![0, 3 * 24_000], "a downbeat every three beats");
    }

    #[test]
    fn a_beat_on_a_buffer_boundary_clicks_exactly_once() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default());
        s.set_metronome(true);
        s.play();

        // Buffers of exactly one beat put every beat on a seam.
        let mut total = 0;
        let mut out = Vec::new();
        for _ in 0..4 {
            s.render(24_000, &mut out);
            total += clicks(&out).len();
        }
        assert_eq!(total, 4, "no beat may be doubled or dropped at a seam");
    }

    #[test]
    fn the_metronome_is_silent_when_disabled() {
        let mut s = seq();
        s.set_metronome(false);
        s.play();

        let mut out = Vec::new();
        s.render(96_000, &mut out);
        assert!(clicks(&out).is_empty());
    }

    #[test]
    fn every_click_is_released() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default());
        s.set_metronome(true);
        s.play();

        let mut ons = 0;
        let mut offs = 0;
        let mut out = Vec::new();
        for _ in 0..40 {
            s.render(4_800, &mut out); // 192000 samples total, two bars
            ons += out.iter().filter(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOn).count();
            offs += out.iter().filter(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOff).count();
        }
        assert!(ons >= 8, "expected at least two bars of clicks, got {ons}");
        assert_eq!(ons, offs, "every click must be released or the sampler stacks voices");
    }

    #[test]
    fn stopping_mid_click_releases_it() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default());
        s.set_metronome(true);
        s.play();

        let mut out = Vec::new();
        s.render(64, &mut out); // the click at beat 0 starts but has not ended
        assert_eq!(clicks(&out).len(), 1);

        s.stop(&mut out);
        assert!(
            out.iter().any(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOff),
            "a click ringing at stop must be released"
        );
    }

    #[test]
    fn clicks_keep_firing_across_a_loop_wrap() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default());
        s.set_metronome(true);
        s.set_loop_region(Some((0, 960))); // two beats
        s.play();

        let mut out = Vec::new();
        s.render(96_000, &mut out); // two full loop passes

        // Beats at 0 and 24000 within each 48000-sample pass.
        assert_eq!(clicks(&out).len(), 4, "the click must survive the wrap");
    }

    // -- count-in ----------------------------------------------------------

    #[test]
    fn count_in_suppresses_timeline_events_but_not_the_click() {
        let mut s = seq();
        s.set_time_signature(TimeSignature::default());
        s.set_metronome(true);
        // One bar of count-in: notes before tick 1920 must not sound.
        s.set_count_in_until(Some(1920));
        s.set_timeline(Timeline::new(vec![
            ev(0, EventKind::NoteOn, 60),     // inside the count-in
            ev(1920, EventKind::NoteOn, 72),  // at the record point
        ]));
        s.play();

        let mut out = Vec::new();
        s.render(96_000, &mut out); // exactly one bar

        assert!(
            !out.iter().any(|e| e.track != METRONOME_TRACK),
            "no timeline event may sound during the count-in"
        );
        assert_eq!(clicks(&out).len(), 4, "but the click must play");

        // Past the record point the timeline resumes.
        out.clear();
        s.render(4_800, &mut out);
        assert!(
            out.iter().any(|e| e.track != METRONOME_TRACK && e.pitch == 72),
            "playback must resume at the record point"
        );
    }

    #[test]
    fn count_in_reports_its_own_state() {
        let mut s = seq();
        s.set_count_in_until(Some(1920));
        s.play();
        assert!(s.in_count_in());

        let mut out = Vec::new();
        s.render(96_000, &mut out); // one bar, landing exactly on the record point
        assert!(!s.in_count_in(), "the count-in ends at the record point");

        s.set_count_in_until(None);
        assert!(!s.in_count_in());
    }

    #[test]
    fn replacing_the_timeline_mid_playback_keeps_the_cursor_consistent() {
        let mut s = seq();
        s.set_timeline(Timeline::new(vec![ev(0, EventKind::NoteOn, 60)]));
        s.play();
        let mut out = Vec::new();
        s.render(4800, &mut out); // now at tick 96

        // Swap in a timeline with an event before and after the current position.
        s.set_timeline(Timeline::new(vec![
            ev(10, EventKind::NoteOn, 50),  // behind the playhead
            ev(200, EventKind::NoteOn, 80), // ahead of it
        ]));

        out.clear();
        s.render(48_000, &mut out);
        let pitches: Vec<u8> = out.iter().map(|e| e.pitch).collect();
        assert_eq!(pitches, vec![80], "only the event ahead of the playhead should fire");
    }
}
