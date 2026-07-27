//! Note boundaries, before and after they are placed in a render buffer.
//!
//! [`TimelineEvent`] is positioned in musical time and lives in a [`Timeline`](super::Timeline);
//! [`RenderedEvent`] is the same boundary once the scheduler has resolved it to a sample
//! offset inside the buffer currently being filled. Keeping them as separate types is
//! what stops a tick from ever being handed to the audio backend as a frame count.

use crate::model::Ticks;

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
    pub(super) fn order_key(&self) -> (Ticks, u8, u8, u8) {
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
