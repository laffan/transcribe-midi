//! Platform dispatch for the audio graph.
//!
//! There is exactly one Swift implementation (`swift/UnpluggedAudio`), exposed through a
//! C ABI and linked into **both** the macOS and iOS binaries. Tauri's Swift plugin
//! mechanism is iOS-only, but we do not need it: it exists so JavaScript can call Swift,
//! and in this app nothing does — Rust owns the engine and the webview only ever talks
//! to Rust. Dropping it means macOS and iOS share one binding path instead of two.
//!
//! On non-Apple hosts a null backend stands in so the workspace still builds and the
//! Rust-side logic stays testable.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioError(pub String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AudioError {}

pub type AudioResult<T> = Result<T, AudioError>;

/// The stable Rust-facing surface. Every caller in the app sees only this.
pub trait AudioBackend: Send + Sync {
    /// Build the graph and start the engine. Idempotent.
    fn start(&self) -> AudioResult<()>;
    fn stop(&self) -> AudioResult<()>;
    fn is_running(&self) -> bool;

    /// Hardware sample rate. Not known until the engine has started.
    fn sample_rate(&self) -> f64;

    /// Ensure there are at least `count` instrument channels in the graph.
    fn ensure_tracks(&self, count: usize) -> AudioResult<()>;

    /// Play a note immediately — used by the on-screen keyboard and live MIDI input,
    /// which bypass the sequencer entirely.
    fn note_on(&self, track: u16, pitch: u8, velocity: u8, channel: u8) -> AudioResult<()>;
    fn note_off(&self, track: u16, pitch: u8, channel: u8) -> AudioResult<()>;
    fn all_notes_off(&self) -> AudioResult<()>;

    fn set_track_gain(&self, track: u16, gain: f32) -> AudioResult<()>;
}

// ---------------------------------------------------------------------------
// Apple (macOS + iOS) — one C ABI, one Swift implementation
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Implemented in swift/UnpluggedAudio/AudioGraph.swift via `@_cdecl`.
    extern "C" {
        fn unplugged_audio_create() -> *mut c_void;
        fn unplugged_audio_destroy(handle: *mut c_void);
        /// `render_ctx` is the `SharedTransport` pointer Swift hands back to
        /// `unplugged_audio_render` on the audio thread.
        fn unplugged_audio_start(handle: *mut c_void, render_ctx: *mut c_void) -> i32;
        fn unplugged_audio_stop(handle: *mut c_void) -> i32;
        fn unplugged_audio_sample_rate(handle: *mut c_void) -> f64;
        fn unplugged_audio_ensure_tracks(handle: *mut c_void, count: u32) -> i32;
        fn unplugged_audio_note_on(handle: *mut c_void, track: u16, pitch: u8, velocity: u8, channel: u8) -> i32;
        fn unplugged_audio_note_off(handle: *mut c_void, track: u16, pitch: u8, channel: u8) -> i32;
        fn unplugged_audio_all_notes_off(handle: *mut c_void) -> i32;
        fn unplugged_audio_set_track_gain(handle: *mut c_void, track: u16, gain: f32) -> i32;
        /// Copies the last error into `buf`; returns the number of bytes written.
        fn unplugged_audio_last_error(handle: *mut c_void, buf: *mut u8, cap: u32) -> u32;
    }

    pub struct AppleBackend {
        handle: *mut c_void,
        render_ctx: *mut c_void,
        running: AtomicBool,
    }

    // SAFETY: the Swift side guards its mutable graph state with its own serial queue,
    // and the two raw pointers are opaque handles that are never dereferenced in Rust.
    unsafe impl Send for AppleBackend {}
    unsafe impl Sync for AppleBackend {}

    impl AppleBackend {
        /// # Safety
        /// `render_ctx` must point to a `SharedTransport` that outlives this backend.
        /// It is handed to the audio thread and dereferenced there on every buffer.
        pub unsafe fn new(render_ctx: *mut c_void) -> AudioResult<Self> {
            let handle = unplugged_audio_create();
            if handle.is_null() {
                return Err(AudioError("could not create the audio graph".into()));
            }
            Ok(AppleBackend { handle, render_ctx, running: AtomicBool::new(false) })
        }

        fn check(&self, code: i32, what: &str) -> AudioResult<()> {
            if code == 0 {
                return Ok(());
            }
            let mut buf = [0u8; 512];
            let written = unsafe {
                unplugged_audio_last_error(self.handle, buf.as_mut_ptr(), buf.len() as u32)
            } as usize;
            let detail = String::from_utf8_lossy(&buf[..written.min(buf.len())]).into_owned();
            Err(AudioError(if detail.is_empty() {
                format!("{what} failed (code {code})")
            } else {
                format!("{what} failed: {detail}")
            }))
        }
    }

    impl Drop for AppleBackend {
        fn drop(&mut self) {
            unsafe {
                unplugged_audio_stop(self.handle);
                unplugged_audio_destroy(self.handle);
            }
        }
    }

    impl AudioBackend for AppleBackend {
        fn start(&self) -> AudioResult<()> {
            if self.running.load(Ordering::Acquire) {
                return Ok(());
            }
            let code = unsafe { unplugged_audio_start(self.handle, self.render_ctx) };
            self.check(code, "starting the audio engine")?;
            self.running.store(true, Ordering::Release);
            Ok(())
        }

        fn stop(&self) -> AudioResult<()> {
            if !self.running.load(Ordering::Acquire) {
                return Ok(());
            }
            let code = unsafe { unplugged_audio_stop(self.handle) };
            self.check(code, "stopping the audio engine")?;
            self.running.store(false, Ordering::Release);
            Ok(())
        }

        fn is_running(&self) -> bool {
            self.running.load(Ordering::Acquire)
        }

        fn sample_rate(&self) -> f64 {
            unsafe { unplugged_audio_sample_rate(self.handle) }
        }

        fn ensure_tracks(&self, count: usize) -> AudioResult<()> {
            let code = unsafe { unplugged_audio_ensure_tracks(self.handle, count as u32) };
            self.check(code, "allocating track instruments")
        }

        fn note_on(&self, track: u16, pitch: u8, velocity: u8, channel: u8) -> AudioResult<()> {
            let code = unsafe { unplugged_audio_note_on(self.handle, track, pitch, velocity, channel) };
            self.check(code, "note on")
        }

        fn note_off(&self, track: u16, pitch: u8, channel: u8) -> AudioResult<()> {
            let code = unsafe { unplugged_audio_note_off(self.handle, track, pitch, channel) };
            self.check(code, "note off")
        }

        fn all_notes_off(&self) -> AudioResult<()> {
            let code = unsafe { unplugged_audio_all_notes_off(self.handle) };
            self.check(code, "all notes off")
        }

        fn set_track_gain(&self, track: u16, gain: f32) -> AudioResult<()> {
            let code = unsafe { unplugged_audio_set_track_gain(self.handle, track, gain) };
            self.check(code, "setting track gain")
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::AppleBackend;

// ---------------------------------------------------------------------------
// Null backend — non-Apple hosts
// ---------------------------------------------------------------------------

/// Does nothing audible, but records what it was asked to do.
///
/// This is what makes the Rust half of the audio layer testable on a Linux CI box: the
/// command plumbing, track allocation and transport wiring are all exercised, and only
/// the actual sound production is absent.
pub struct NullBackend {
    running: std::sync::atomic::AtomicBool,
    pub log: std::sync::Mutex<Vec<String>>,
}

impl Default for NullBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NullBackend {
    pub fn new() -> Self {
        NullBackend {
            running: std::sync::atomic::AtomicBool::new(false),
            log: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn record(&self, entry: String) {
        if let Ok(mut log) = self.log.lock() {
            log.push(entry);
        }
    }

    pub fn entries(&self) -> Vec<String> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

impl AudioBackend for NullBackend {
    fn start(&self) -> AudioResult<()> {
        self.running.store(true, std::sync::atomic::Ordering::Release);
        self.record("start".into());
        Ok(())
    }

    fn stop(&self) -> AudioResult<()> {
        self.running.store(false, std::sync::atomic::Ordering::Release);
        self.record("stop".into());
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Acquire)
    }

    fn sample_rate(&self) -> f64 {
        48_000.0
    }

    fn ensure_tracks(&self, count: usize) -> AudioResult<()> {
        self.record(format!("ensure_tracks {count}"));
        Ok(())
    }

    fn note_on(&self, track: u16, pitch: u8, velocity: u8, channel: u8) -> AudioResult<()> {
        self.record(format!("note_on t{track} p{pitch} v{velocity} c{channel}"));
        Ok(())
    }

    fn note_off(&self, track: u16, pitch: u8, channel: u8) -> AudioResult<()> {
        self.record(format!("note_off t{track} p{pitch} c{channel}"));
        Ok(())
    }

    fn all_notes_off(&self) -> AudioResult<()> {
        self.record("all_notes_off".into());
        Ok(())
    }

    fn set_track_gain(&self, track: u16, gain: f32) -> AudioResult<()> {
        self.record(format!("set_track_gain t{track} g{gain}"));
        Ok(())
    }
}
