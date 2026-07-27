//! The one struct that crosses the boundary by layout rather than by name.
//!
//! Field order and padding must match `CRenderedEvent` in
//! `plugin/Support/UnpluggedPluginFFI.h` exactly — the C side reads these bytes, not a
//! description of them, so a reordered field is a silent wrong-note bug rather than a
//! compile error. The invariants table in README-TECHNICAL.md lists every place that has
//! to agree.

use unplugged_core::sequencer::RenderedEvent;

/// Mirrors `CRenderedEvent` in `unplugged-audio`, and the C header the Swift side reads.
/// Field order and padding must match exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CRenderedEvent {
    pub frame_offset: u32,
    pub track: u16,
    /// 0 = note off, 1 = note on.
    pub kind: u8,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
    pub _pad: [u8; 2],
}

impl From<RenderedEvent> for CRenderedEvent {
    fn from(event: RenderedEvent) -> Self {
        use unplugged_core::sequencer::EventKind;
        CRenderedEvent {
            frame_offset: event.frame_offset,
            track: event.track,
            kind: match event.kind {
                EventKind::NoteOn => 1,
                EventKind::NoteOff => 0,
            },
            pitch: event.pitch,
            velocity: event.velocity,
            channel: event.channel,
            _pad: [0; 2],
        }
    }
}
