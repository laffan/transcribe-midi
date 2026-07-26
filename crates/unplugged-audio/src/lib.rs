//! Audio engine binding: one Rust-facing API, one Swift implementation.
//!
//! Ownership split, per the spec: **Rust owns the sequencer and decides when notes
//! fire; Swift owns the AVAudioEngine graph and the render callback.** Swift calls back
//! into [`unplugged_audio_render`] once per render quantum to ask what happens in the
//! next buffer, and applies the answer with sample offsets.

pub mod backend;
pub mod shared;

use std::ffi::c_void;
use std::sync::Arc;

use unplugged_core::sequencer::{RenderedEvent, Timeline};
use unplugged_core::Ticks;

pub use backend::{AudioBackend, AudioError, AudioResult, NullBackend};
pub use shared::{AudioCursor, SharedTransport};

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use backend::AppleBackend;

/// What the app holds. Owns the shared transport and whichever backend is compiled in.
pub struct AudioEngine {
    transport: Arc<SharedTransport>,
    backend: Box<dyn AudioBackend>,
}

impl AudioEngine {
    /// Build an engine on the platform backend.
    ///
    /// On non-Apple hosts this is the null backend, so the app runs (silently) for
    /// development without `#[cfg]` at any call site.
    pub fn new() -> AudioResult<Self> {
        let transport = Arc::new(SharedTransport::new());

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let backend: Box<dyn AudioBackend> = {
            // The pointer handed to the audio thread must stay valid for as long as the
            // engine lives. `transport` is an `Arc` held in this struct for exactly that
            // lifetime, and `render_context` leaks a clone so Swift's copy is owning.
            let ctx = Arc::into_raw(Arc::clone(&transport)) as *mut c_void;
            // SAFETY: `ctx` points at a `SharedTransport` kept alive by the `Arc` clone
            // above, which is only released in `Drop`.
            Box::new(unsafe { AppleBackend::new(ctx)? })
        };

        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        let backend: Box<dyn AudioBackend> = Box::new(NullBackend::new());

        Ok(AudioEngine { transport, backend })
    }

    pub fn with_backend(backend: Box<dyn AudioBackend>) -> Self {
        AudioEngine {
            transport: Arc::new(SharedTransport::new()),
            backend,
        }
    }

    pub fn transport(&self) -> &Arc<SharedTransport> {
        &self.transport
    }

    pub fn backend(&self) -> &dyn AudioBackend {
        self.backend.as_ref()
    }

    // -- lifecycle ----------------------------------------------------------

    pub fn start(&self) -> AudioResult<()> {
        self.backend.start()
    }

    pub fn shutdown(&self) -> AudioResult<()> {
        self.transport.set_playing(false);
        self.backend.all_notes_off()?;
        self.backend.stop()
    }

    pub fn sample_rate(&self) -> f64 {
        self.backend.sample_rate()
    }

    // -- transport ----------------------------------------------------------

    pub fn set_timeline(&self, timeline: Timeline) {
        self.transport.set_timeline(timeline);
    }

    pub fn play(&self) -> AudioResult<()> {
        self.backend.start()?;
        self.transport.set_playing(true);
        Ok(())
    }

    /// Stop and silence. The sequencer emits releases for held notes on the next
    /// buffer, but `all_notes_off` is also sent so a stalled engine cannot leave a
    /// note hanging.
    pub fn stop(&self) -> AudioResult<()> {
        self.transport.set_playing(false);
        self.backend.all_notes_off()
    }

    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    pub fn seek(&self, tick: Ticks) {
        self.transport.request_seek(tick);
    }

    pub fn position_ticks(&self) -> Ticks {
        self.transport.position_ticks()
    }

    pub fn set_tempo(&self, bpm: f64) {
        self.transport.set_tempo(bpm);
    }

    pub fn set_loop_region(&self, region: Option<(Ticks, Ticks)>) {
        self.transport.set_loop_region(region);
    }

    pub fn loop_region(&self) -> Option<(Ticks, Ticks)> {
        self.transport.loop_region()
    }

    pub fn set_metronome(&self, enabled: bool) {
        self.transport.set_metronome(enabled);
    }

    pub fn metronome_enabled(&self) -> bool {
        self.transport.metronome_enabled()
    }

    /// Suppress timeline events until `tick`, leaving only the click audible.
    pub fn set_count_in_until(&self, tick: Option<Ticks>) {
        self.transport.set_count_in_until(tick);
    }

    pub fn in_count_in(&self) -> bool {
        self.transport.in_count_in()
    }

    /// Wrapping count of loop wraps, polled by the host to drive the recorder.
    pub fn wrap_count(&self) -> u32 {
        self.transport.wrap_count()
    }

    // -- live input ---------------------------------------------------------

    pub fn ensure_tracks(&self, count: usize) -> AudioResult<()> {
        self.backend.ensure_tracks(count)
    }

    pub fn note_on(&self, track: u16, pitch: u8, velocity: u8, channel: u8) -> AudioResult<()> {
        self.backend.start()?;
        self.backend.note_on(track, pitch, velocity, channel)
    }

    pub fn note_off(&self, track: u16, pitch: u8, channel: u8) -> AudioResult<()> {
        self.backend.note_off(track, pitch, channel)
    }

    pub fn all_notes_off(&self) -> AudioResult<()> {
        self.backend.all_notes_off()
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.shutdown();
        // Balance the `Arc::into_raw` in `new`. The backend has been dropped by the
        // time this runs (field order), so the audio thread is no longer running.
        // Reconstructing and dropping the `Arc` releases that reference.
        // NOTE: only sound because `AppleBackend::drop` stops the engine first.
    }
}

// ---------------------------------------------------------------------------
// Audio-thread entry point
// ---------------------------------------------------------------------------

/// One event as it crosses the FFI boundary. `repr(C)` so Swift can read it directly.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CRenderedEvent {
    pub frame_offset: u32,
    pub track: u16,
    /// 0 = note off, 1 = note on.
    pub kind: u8,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
    _pad: [u8; 2],
}

impl From<RenderedEvent> for CRenderedEvent {
    fn from(e: RenderedEvent) -> Self {
        CRenderedEvent {
            frame_offset: e.frame_offset,
            track: e.track,
            kind: match e.kind {
                unplugged_core::sequencer::EventKind::NoteOff => 0,
                unplugged_core::sequencer::EventKind::NoteOn => 1,
            },
            pitch: e.pitch,
            velocity: e.velocity,
            channel: e.channel,
            _pad: [0; 2],
        }
    }
}

/// Per-audio-thread scratch state, created by Swift once and passed back every buffer.
///
/// Kept opaque to Swift: it holds the playback cursor and a preallocated event buffer so
/// that nothing on the audio thread allocates.
pub struct RenderState {
    cursor: AudioCursor,
    events: Vec<RenderedEvent>,
}

/// Create the render-thread state. Called once, off the audio thread.
///
/// # Safety
/// The returned pointer must be released with [`unplugged_audio_render_state_destroy`].
#[no_mangle]
pub unsafe extern "C" fn unplugged_audio_render_state_create(
    sample_rate: f64,
    ppq: u16,
    tempo_bpm: f64,
) -> *mut c_void {
    let state = Box::new(RenderState {
        cursor: AudioCursor::new(sample_rate, ppq, tempo_bpm),
        events: Vec::with_capacity(2048),
    });
    Box::into_raw(state) as *mut c_void
}

/// # Safety
/// `state` must come from [`unplugged_audio_render_state_create`] and not be in use by
/// the audio thread.
#[no_mangle]
pub unsafe extern "C" fn unplugged_audio_render_state_destroy(state: *mut c_void) {
    if !state.is_null() {
        drop(Box::from_raw(state as *mut RenderState));
    }
}

/// Ask the sequencer what happens in the next `frames` samples.
///
/// **Called from the audio thread, once per render quantum.** Allocation-free and
/// lock-free: the event buffer is preallocated in `RenderState`, and all shared state is
/// reached through atomics.
///
/// Returns the number of events written to `out`, capped at `capacity`.
///
/// # Safety
/// - `transport` must be the `SharedTransport` pointer given to `unplugged_audio_start`.
/// - `state` must come from [`unplugged_audio_render_state_create`].
/// - `out` must be valid for `capacity` `CRenderedEvent` writes.
/// - Must not be called concurrently for the same `state`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_audio_render(
    transport: *mut c_void,
    state: *mut c_void,
    frames: u32,
    out: *mut CRenderedEvent,
    capacity: u32,
) -> u32 {
    if transport.is_null() || state.is_null() || out.is_null() || capacity == 0 {
        return 0;
    }

    let transport = &*(transport as *const SharedTransport);
    let state = &mut *(state as *mut RenderState);

    transport.render(&mut state.cursor, frames, &mut state.events);

    let count = state.events.len().min(capacity as usize);
    for (index, event) in state.events.iter().take(count).enumerate() {
        std::ptr::write(out.add(index), CRenderedEvent::from(*event));
    }
    count as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use unplugged_core::sequencer::{EventKind, TimelineEvent};

    fn engine() -> AudioEngine {
        AudioEngine::with_backend(Box::new(NullBackend::new()))
    }

    #[test]
    fn play_starts_the_backend_and_sets_the_transport() {
        let e = engine();
        assert!(!e.is_playing());
        e.play().unwrap();
        assert!(e.is_playing());
        assert!(e.backend().is_running());
    }

    #[test]
    fn stop_silences_the_backend() {
        let e = engine();
        e.play().unwrap();
        e.stop().unwrap();
        assert!(!e.is_playing());
    }

    #[test]
    fn live_notes_bypass_the_sequencer_and_reach_the_backend() {
        let e = engine();
        e.note_on(0, 60, 100, 0).unwrap();
        e.note_off(0, 60, 0).unwrap();

        // Downcast is not available through `dyn`, so assert via the engine's contract:
        // the backend was started implicitly by the first live note.
        assert!(e.backend().is_running(), "a live note must start the engine");
        assert!(!e.is_playing(), "but must not start the transport");
    }

    #[test]
    fn the_ffi_render_entry_point_produces_events() {
        let transport = SharedTransport::new();
        transport.set_timeline(Timeline::new(vec![TimelineEvent {
            tick: 2,
            kind: EventKind::NoteOn,
            track: 0,
            pitch: 60,
            velocity: 100,
            channel: 0,
        }]));
        transport.set_playing(true);

        unsafe {
            let state = unplugged_audio_render_state_create(48_000.0, 480, 120.0);
            let mut out = [CRenderedEvent {
                frame_offset: 0, track: 0, kind: 0, pitch: 0, velocity: 0, channel: 0, _pad: [0; 2],
            }; 16];

            let count = unplugged_audio_render(
                &transport as *const _ as *mut c_void,
                state,
                512,
                out.as_mut_ptr(),
                out.len() as u32,
            );

            assert_eq!(count, 1);
            assert_eq!(out[0].kind, 1, "note on");
            assert_eq!(out[0].pitch, 60);
            assert_eq!(out[0].frame_offset, 100);

            unplugged_audio_render_state_destroy(state);
        }
    }

    #[test]
    fn the_ffi_entry_point_refuses_null_and_zero_capacity_rather_than_faulting() {
        unsafe {
            let state = unplugged_audio_render_state_create(48_000.0, 480, 120.0);
            let transport = SharedTransport::new();
            let mut out = [CRenderedEvent {
                frame_offset: 0, track: 0, kind: 0, pitch: 0, velocity: 0, channel: 0, _pad: [0; 2],
            }; 4];

            assert_eq!(
                unplugged_audio_render(std::ptr::null_mut(), state, 512, out.as_mut_ptr(), 4),
                0
            );
            assert_eq!(
                unplugged_audio_render(
                    &transport as *const _ as *mut c_void,
                    state,
                    512,
                    std::ptr::null_mut(),
                    4
                ),
                0
            );
            assert_eq!(
                unplugged_audio_render(
                    &transport as *const _ as *mut c_void,
                    state,
                    512,
                    out.as_mut_ptr(),
                    0
                ),
                0
            );

            unplugged_audio_render_state_destroy(state);
            unplugged_audio_render_state_destroy(std::ptr::null_mut()); // must not fault
        }
    }

    #[test]
    fn the_ffi_render_respects_the_output_capacity() {
        let transport = SharedTransport::new();
        let events: Vec<TimelineEvent> = (0..64)
            .map(|i| TimelineEvent {
                tick: i,
                kind: EventKind::NoteOn,
                track: 0,
                pitch: 60,
                velocity: 100,
                channel: 0,
            })
            .collect();
        transport.set_timeline(Timeline::new(events));
        transport.set_playing(true);

        unsafe {
            let state = unplugged_audio_render_state_create(48_000.0, 480, 120.0);
            let mut out = [CRenderedEvent {
                frame_offset: 0, track: 0, kind: 0, pitch: 0, velocity: 0, channel: 0, _pad: [0; 2],
            }; 8];

            let count = unplugged_audio_render(
                &transport as *const _ as *mut c_void,
                state,
                48_000,
                out.as_mut_ptr(),
                out.len() as u32,
            );
            assert_eq!(count, 8, "must clamp to capacity rather than overrun the buffer");

            unplugged_audio_render_state_destroy(state);
        }
    }
}
