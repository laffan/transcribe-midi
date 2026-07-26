//! Phase 5 platform surface: share sheet, pasteboard and drag-out.
//!
//! Same shape as the audio binding — one Swift implementation behind a C ABI, a null
//! path everywhere else — so callers never need a `#[cfg]`.

use crate::error::{CommandError, CommandResult};

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use std::ffi::CString;
    use std::os::raw::c_char;

    extern "C" {
        fn unplugged_platform_copy_file(path: *const c_char) -> i32;
        #[cfg(target_os = "ios")]
        fn unplugged_platform_share_file(path: *const c_char) -> i32;
        #[cfg(target_os = "macos")]
        fn unplugged_platform_begin_file_drag(path: *const c_char) -> i32;
    }

    fn c_path(path: &str) -> Result<CString, String> {
        CString::new(path).map_err(|_| "path contains an interior NUL byte".to_string())
    }

    pub fn copy_file(path: &str) -> Result<(), String> {
        let c = c_path(path)?;
        match unsafe { unplugged_platform_copy_file(c.as_ptr()) } {
            0 => Ok(()),
            code => Err(format!("could not copy to the pasteboard (code {code})")),
        }
    }

    #[cfg(target_os = "ios")]
    pub fn share_file(path: &str) -> Result<(), String> {
        let c = c_path(path)?;
        match unsafe { unplugged_platform_share_file(c.as_ptr()) } {
            0 => Ok(()),
            2 => Err("no window is available to present the share sheet".to_string()),
            code => Err(format!("could not present the share sheet (code {code})")),
        }
    }

    #[cfg(not(target_os = "ios"))]
    pub fn share_file(_path: &str) -> Result<(), String> {
        Err("the share sheet is iOS only — use Export on macOS".to_string())
    }

    #[cfg(target_os = "macos")]
    pub fn begin_file_drag(path: &str) -> Result<(), String> {
        let c = c_path(path)?;
        match unsafe { unplugged_platform_begin_file_drag(c.as_ptr()) } {
            0 => Ok(()),
            1 => Err("the staged file no longer exists".to_string()),
            2 => Err("no window is available to drag from".to_string()),
            // The AppKit drag has to be attached to a real event; if the pointer gesture
            // has already finished there is nothing to attach it to.
            3 => Err("no active mouse event to start the drag from".to_string()),
            code => Err(format!("could not start the drag (code {code})")),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn begin_file_drag(_path: &str) -> Result<(), String> {
        Err("drag-out is macOS only — use Share on iOS".to_string())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod apple {
    pub fn copy_file(_path: &str) -> Result<(), String> {
        Err("the pasteboard is not available on this platform".to_string())
    }
    pub fn share_file(_path: &str) -> Result<(), String> {
        Err("sharing is not available on this platform".to_string())
    }
    pub fn begin_file_drag(_path: &str) -> Result<(), String> {
        Err("drag-out is not available on this platform".to_string())
    }
}

/// Put a staged `.mid` on the pasteboard.
///
/// The UI states plainly what this is for: it pastes a *file* into Finder or Files. It
/// does not paste notes into Logic's piano roll — that clipboard format is proprietary
/// and undocumented, and the spec explicitly rules out reverse-engineering it.
#[tauri::command]
pub fn copy_file_to_pasteboard(path: String) -> CommandResult<()> {
    apple::copy_file(&path).map_err(CommandError::from)
}

/// Present the iOS share sheet for a staged file (UTI `public.midi-audio`).
#[tauri::command]
pub fn share_file(path: String) -> CommandResult<()> {
    apple::share_file(&path).map_err(CommandError::from)
}

/// Begin a native macOS drag of a staged file, so it can be dropped into Logic Pro.
#[tauri::command]
pub fn begin_file_drag(path: String) -> CommandResult<()> {
    apple::begin_file_drag(&path).map_err(CommandError::from)
}

/// What the current platform can actually do, so the UI shows only real affordances
/// rather than buttons that fail when pressed.
#[tauri::command]
pub fn platform_capabilities() -> CommandResult<PlatformCapabilities> {
    Ok(PlatformCapabilities {
        share_sheet: cfg!(target_os = "ios"),
        drag_out: cfg!(target_os = "macos"),
        pasteboard: cfg!(any(target_os = "macos", target_os = "ios")),
        save_dialog: true,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct PlatformCapabilities {
    pub share_sheet: bool,
    pub drag_out: bool,
    pub pasteboard: bool,
    pub save_dialog: bool,
}
