//! Microphone capture, for Phase 7's transcription.
//!
//! Same shape as the playback binding — one Swift implementation behind a C ABI, a null
//! path everywhere else — so callers never need a `#[cfg]`. Nothing here interprets the
//! samples; that is `unplugged-transcribe`'s job.
//!
//! **No audio is written to disk and no take is kept.** The spec puts recorded audio
//! tracks out of scope and says the microphone exists only to feed transcription, so the
//! samples live in one buffer, are analysed, and are dropped.

use crate::backend::{AudioError, AudioResult};

/// Ceiling on a single take, in seconds.
///
/// Matches the Swift side's own limit. Two minutes is far longer than the phrase this
/// feature is for, and bounds what a recording left running can consume.
pub const MAX_CAPTURE_SECONDS: f64 = 120.0;

/// A microphone input, drained into a growing buffer.
pub trait CaptureBackend: Send + Sync {
    /// Begin capturing. Idempotent.
    fn start(&self) -> AudioResult<()>;
    fn stop(&self);
    /// Move any newly captured samples into `destination`. Returns how many were added.
    fn drain(&self, destination: &mut Vec<f32>) -> usize;
    /// Hardware sample rate. Zero until capture has started.
    fn sample_rate(&self) -> f64;
    /// Peak level since the last call, 0–1. Reading resets it.
    fn take_peak(&self) -> f32;
}

/// Distinguished so the UI can offer to open Settings instead of showing an error.
///
/// A denied microphone is not a fault; it is a decision the user made and can revisit,
/// and it is the only capture failure with an obvious next step.
pub fn is_permission_error(error: &AudioError) -> bool {
    error.0.contains("microphone access")
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use super::*;
    use std::ffi::{c_void, CStr};
    use std::os::raw::c_char;

    // Implemented in swift/UnpluggedAudio/Sources/UnpluggedAudio/Capture.swift.
    extern "C" {
        fn unplugged_capture_create() -> *mut c_void;
        fn unplugged_capture_destroy(handle: *mut c_void);
        fn unplugged_capture_start(handle: *mut c_void) -> i32;
        fn unplugged_capture_stop(handle: *mut c_void) -> i32;
        fn unplugged_capture_drain(handle: *mut c_void, out: *mut f32, capacity: u32) -> u32;
        fn unplugged_capture_sample_rate(handle: *mut c_void) -> f64;
        fn unplugged_capture_take_peak(handle: *mut c_void) -> f32;
        fn unplugged_capture_last_error(handle: *mut c_void) -> *mut c_char;
        // Shared with the Phase 5 platform target: the string comes from `strdup`, so it
        // must be released by `free` rather than by Rust's allocator.
        fn unplugged_platform_string_free(pointer: *mut c_char);
    }

    /// How many samples to pull per drain. About a second at 48 kHz.
    const DRAIN_CHUNK: usize = 48_000;

    pub struct AppleCapture {
        handle: *mut c_void,
    }

    // SAFETY: the Swift object guards its buffer with its own lock, and `handle` is an
    // opaque pointer that is never dereferenced in Rust.
    unsafe impl Send for AppleCapture {}
    unsafe impl Sync for AppleCapture {}

    impl AppleCapture {
        pub fn new() -> AudioResult<Self> {
            let handle = unsafe { unplugged_capture_create() };
            if handle.is_null() {
                return Err(AudioError("could not create the microphone input".into()));
            }
            Ok(AppleCapture { handle })
        }

        fn last_error(&self) -> Option<String> {
            // SAFETY: the pointer is either NULL or a `strdup`'d string; it is copied and
            // freed here before this returns.
            unsafe {
                let pointer = unplugged_capture_last_error(self.handle);
                if pointer.is_null() {
                    return None;
                }
                let message = CStr::from_ptr(pointer).to_string_lossy().into_owned();
                unplugged_platform_string_free(pointer);
                Some(message)
            }
        }
    }

    impl Drop for AppleCapture {
        fn drop(&mut self) {
            unsafe {
                unplugged_capture_stop(self.handle);
                unplugged_capture_destroy(self.handle);
            }
        }
    }

    impl CaptureBackend for AppleCapture {
        fn start(&self) -> AudioResult<()> {
            match unsafe { unplugged_capture_start(self.handle) } {
                0 => Ok(()),
                // The wording matters: `is_permission_error` matches on it, and the UI
                // branches on that to offer the Settings route.
                2 => Err(AudioError(
                    "microphone access has not been granted — allow it in System Settings"
                        .into(),
                )),
                code => Err(AudioError(self.last_error().unwrap_or_else(|| {
                    format!("could not start the microphone (code {code})")
                }))),
            }
        }

        fn stop(&self) {
            unsafe {
                unplugged_capture_stop(self.handle);
            }
        }

        fn drain(&self, destination: &mut Vec<f32>) -> usize {
            let mut total = 0;
            loop {
                let base = destination.len();
                destination.resize(base + DRAIN_CHUNK, 0.0);

                // SAFETY: the pointer addresses `DRAIN_CHUNK` initialised f32s that this
                // Vec owns, and Swift writes at most that many.
                let written = unsafe {
                    unplugged_capture_drain(
                        self.handle,
                        destination[base..].as_mut_ptr(),
                        DRAIN_CHUNK as u32,
                    )
                } as usize;

                destination.truncate(base + written);
                total += written;

                // A short read means the queue is empty; a full one means there may be
                // more waiting.
                if written < DRAIN_CHUNK {
                    return total;
                }
            }
        }

        fn sample_rate(&self) -> f64 {
            unsafe { unplugged_capture_sample_rate(self.handle) }
        }

        fn take_peak(&self) -> f32 {
            unsafe { unplugged_capture_take_peak(self.handle) }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::AppleCapture;

/// Stands in off-Apple so the workspace builds and the Rust logic stays testable.
///
/// It reports no samples rather than synthesising any: a fake microphone that produced a
/// tone would make the transcription UI look like it worked on a machine where it cannot.
#[derive(Default)]
pub struct NullCapture;

impl NullCapture {
    pub fn new() -> Self {
        NullCapture
    }
}

impl CaptureBackend for NullCapture {
    fn start(&self) -> AudioResult<()> {
        Err(AudioError(
            "microphone capture needs the macOS or iOS build".into(),
        ))
    }
    fn stop(&self) {}
    fn drain(&self, _destination: &mut Vec<f32>) -> usize {
        0
    }
    fn sample_rate(&self) -> f64 {
        0.0
    }
    fn take_peak(&self) -> f32 {
        0.0
    }
}

/// Build the capture backend for this platform.
pub fn new_capture() -> Box<dyn CaptureBackend> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        match AppleCapture::new() {
            Ok(capture) => Box::new(capture),
            // Falling back rather than failing: the app should still open, and the error
            // surfaces when the user actually presses record.
            Err(_) => Box::new(NullCapture::new()),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        Box::new(NullCapture::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_null_capture_reports_nothing_rather_than_faking_it() {
        let capture = NullCapture::new();
        assert!(capture.start().is_err());
        assert_eq!(capture.sample_rate(), 0.0);
        assert_eq!(capture.take_peak(), 0.0);

        let mut buffer = Vec::new();
        assert_eq!(capture.drain(&mut buffer), 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_denied_microphone_is_told_apart_from_a_broken_one() {
        assert!(is_permission_error(&AudioError(
            "microphone access has not been granted — allow it in System Settings".into()
        )));
        assert!(!is_permission_error(&AudioError(
            "no microphone input is available".into()
        )));
    }
}
