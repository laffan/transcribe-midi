//! Playing a captured take back.
//!
//! Monitoring, not a feature. You cannot judge a transcription you have not heard, and
//! until this existed the only way to check a take was to accept its notes and listen to
//! the sampler play them — which tells you what the *transcriber* heard, not what you
//! played. The take is still dropped once its notes are committed.

use crate::backend::{AudioError, AudioResult};

/// A loaded take that can be played from any point.
pub trait PreviewBackend: Send + Sync {
    /// Hand over samples. Replaces whatever was loaded.
    fn load(&self, samples: &[f32], sample_rate: f64) -> AudioResult<()>;
    fn play(&self, from_seconds: f64) -> AudioResult<()>;
    fn stop(&self);
    /// Position in seconds, or `None` when not playing.
    fn position(&self) -> Option<f64>;
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use super::*;
    use std::ffi::c_void;

    // Implemented in swift/UnpluggedAudio/Sources/UnpluggedAudio/Preview.swift.
    extern "C" {
        fn unplugged_preview_create() -> *mut c_void;
        fn unplugged_preview_destroy(handle: *mut c_void);
        fn unplugged_preview_load(
            handle: *mut c_void,
            samples: *const f32,
            count: u32,
            sample_rate: f64,
        ) -> i32;
        fn unplugged_preview_play(handle: *mut c_void, from_seconds: f64) -> i32;
        fn unplugged_preview_stop(handle: *mut c_void);
        fn unplugged_preview_position(handle: *mut c_void) -> f64;
    }

    pub struct ApplePreview {
        handle: *mut c_void,
    }

    // SAFETY: the Swift object guards its buffer with its own lock, and `handle` is an
    // opaque pointer never dereferenced in Rust.
    unsafe impl Send for ApplePreview {}
    unsafe impl Sync for ApplePreview {}

    impl ApplePreview {
        pub fn new() -> AudioResult<Self> {
            let handle = unsafe { unplugged_preview_create() };
            if handle.is_null() {
                return Err(AudioError("could not create the preview player".into()));
            }
            Ok(ApplePreview { handle })
        }
    }

    impl Drop for ApplePreview {
        fn drop(&mut self) {
            unsafe {
                unplugged_preview_stop(self.handle);
                unplugged_preview_destroy(self.handle);
            }
        }
    }

    impl PreviewBackend for ApplePreview {
        fn load(&self, samples: &[f32], sample_rate: f64) -> AudioResult<()> {
            if samples.is_empty() || sample_rate <= 0.0 {
                return Err(AudioError("there is nothing to play".into()));
            }
            // SAFETY: the pointer and length describe `samples`, which Swift copies
            // before this returns.
            match unsafe {
                unplugged_preview_load(
                    self.handle,
                    samples.as_ptr(),
                    samples.len() as u32,
                    sample_rate,
                )
            } {
                0 => Ok(()),
                code => Err(AudioError(format!("could not load the take (code {code})"))),
            }
        }

        fn play(&self, from_seconds: f64) -> AudioResult<()> {
            match unsafe { unplugged_preview_play(self.handle, from_seconds.max(0.0)) } {
                0 => Ok(()),
                2 => Err(AudioError("the preview player could not start".into())),
                code => Err(AudioError(format!("could not play the take (code {code})"))),
            }
        }

        fn stop(&self) {
            unsafe { unplugged_preview_stop(self.handle) }
        }

        fn position(&self) -> Option<f64> {
            let seconds = unsafe { unplugged_preview_position(self.handle) };
            (seconds >= 0.0).then_some(seconds)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::ApplePreview;

/// Stands in off-Apple. Refuses rather than pretending: a silent "playing" state would be
/// worse than being told the build cannot do it.
#[derive(Default)]
pub struct NullPreview;

impl NullPreview {
    pub fn new() -> Self {
        NullPreview
    }
}

impl PreviewBackend for NullPreview {
    fn load(&self, _samples: &[f32], _sample_rate: f64) -> AudioResult<()> {
        Ok(())
    }
    fn play(&self, _from_seconds: f64) -> AudioResult<()> {
        Err(AudioError(
            "playing a take back needs the macOS or iOS build".into(),
        ))
    }
    fn stop(&self) {}
    fn position(&self) -> Option<f64> {
        None
    }
}

pub fn new_preview() -> Box<dyn PreviewBackend> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        match ApplePreview::new() {
            Ok(preview) => Box::new(preview),
            // Falling back rather than failing: the app should still open, and the error
            // surfaces when the user presses play.
            Err(_) => Box::new(NullPreview::new()),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        Box::new(NullPreview::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_null_preview_refuses_rather_than_pretending() {
        let preview = NullPreview::new();
        // Loading succeeds so the caller's flow is not littered with `#[cfg]`; playing is
        // where the truth is told.
        assert!(preview.load(&[0.1, 0.2], 44100.0).is_ok());
        assert!(preview.play(0.0).is_err());
        assert_eq!(preview.position(), None);
    }
}
