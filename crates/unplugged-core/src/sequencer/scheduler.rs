//! The cursor, the loop, the count-in and the metronome.
//!
//! Everything reachable from [`Sequencer::render`] runs on the audio thread: it
//! allocates nothing, locks nothing and never panics. The buffers it needs are sized at
//! construction, and output is appended to a caller-owned `Vec` that the caller
//! preallocated — which is why `render` takes an `out` parameter instead of returning.

use crate::model::Ticks;

use super::event::{EventKind, RenderedEvent, TimelineEvent};
use super::timeline::Timeline;
use super::{
    MAX_SOUNDING, METRONOME_BEAT_PITCH, METRONOME_DOWNBEAT_PITCH, METRONOME_TRACK,
    METRONOME_VELOCITY,
};

/// How long a click sounds, in ticks. Short enough to read as percussive at any tempo.
const METRONOME_CLICK_TICKS: f64 = 30.0;

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
