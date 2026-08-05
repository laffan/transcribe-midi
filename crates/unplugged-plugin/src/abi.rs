//! The C entry points, and nothing but.
//!
//! Every one of them catches panics. Unwinding into Swift is undefined behaviour, and in a
//! plugin the process it would take down is the user's DAW — along with their unsaved
//! session. Every function here is therefore a thin wrapper: null-check, `guard`, delegate
//! to [`Plugin`](super::Plugin), convert the answer.
//!
//! Strings cross as `strdup`-style allocations freed by [`unplugged_plugin_string_free`];
//! every `create` has a `destroy`.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use unplugged_core::host_sync::HostTransport;
use unplugged_core::BuildInfo;

use super::event::CRenderedEvent;
use super::plugin::{Plugin, PluginState};


fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// Hand a `String` to C. Freed with `unplugged_plugin_string_free`.
fn to_c_string(text: String) -> *mut c_char {
    CString::new(text)
        .unwrap_or_else(|_| CString::new("").expect("an empty string has no NUL"))
        .into_raw()
}

unsafe fn plugin<'a>(handle: *mut std::ffi::c_void) -> Option<&'a mut Plugin> {
    (!handle.is_null()).then(|| &mut *(handle as *mut Plugin))
}

/// # Safety
/// `data_dir` must be a NUL-terminated UTF-8 path, or NULL.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_create(
    data_dir: *const c_char,
) -> *mut std::ffi::c_void {
    guard(std::ptr::null_mut(), || {
        if data_dir.is_null() {
            return std::ptr::null_mut();
        }
        let path = PathBuf::from(CStr::from_ptr(data_dir).to_string_lossy().into_owned());
        Box::into_raw(Box::new(Plugin::new(path))) as *mut std::ffi::c_void
    })
}

/// # Safety
/// `handle` must come from `unplugged_plugin_create` and not be in use by the audio thread.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_destroy(handle: *mut std::ffi::c_void) {
    guard((), || {
        if !handle.is_null() {
            drop(Box::from_raw(handle as *mut Plugin));
        }
    })
}

/// # Safety
/// `pointer` must come from one of the functions here that returns a string.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_string_free(pointer: *mut c_char) {
    guard((), || {
        if !pointer.is_null() {
            drop(CString::from_raw(pointer));
        }
    })
}

/// JSON array of the projects in the shared app-data directory.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_projects_json(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || match plugin(handle) {
        Some(plugin) => to_c_string(plugin.projects_json()),
        None => std::ptr::null_mut(),
    })
}

/// Open a project. Returns 0, or non-zero with a message from
/// `unplugged_plugin_last_error`.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`; `id` must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_open(
    handle: *mut std::ffi::c_void,
    id: *const c_char,
) -> i32 {
    guard(-1, || {
        let Some(plugin) = plugin(handle) else { return -1 };
        if id.is_null() {
            plugin.close();
            return 0;
        }
        let id = CStr::from_ptr(id).to_string_lossy().into_owned();
        match plugin.open(&id) {
            Ok(()) => 0,
            Err(error) => {
                plugin.record_error(error);
                1
            }
        }
    })
}

/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_last_error(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        match plugin(handle).and_then(Plugin::take_last_error) {
            Some(message) => to_c_string(message),
            None => std::ptr::null_mut(),
        }
    })
}

/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_prepare(
    handle: *mut std::ffi::c_void,
    sample_rate: f64,
) {
    guard((), || {
        if let Some(plugin) = plugin(handle) {
            plugin.prepare(sample_rate);
        }
    })
}

/// Number of tracks in the open project, so the view can size the channel list.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_track_count(handle: *mut std::ffi::c_void) -> u32 {
    guard(0, || plugin(handle).map_or(0, |p| p.track_count() as u32))
}

/// The host's `fullState`, as JSON.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_state_json(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || match plugin(handle) {
        Some(plugin) => to_c_string(
            serde_json::to_string(&plugin.state()).unwrap_or_else(|_| "{}".into()),
        ),
        None => std::ptr::null_mut(),
    })
}

/// Restore from the host's `fullState`. Never fails destructively.
///
/// # Safety
/// `handle` must come from `unplugged_plugin_create`; `json` must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_set_state_json(
    handle: *mut std::ffi::c_void,
    json: *const c_char,
) -> i32 {
    guard(-1, || {
        let Some(plugin) = plugin(handle) else { return -1 };
        if json.is_null() {
            plugin.close();
            return 0;
        }
        let raw = CStr::from_ptr(json).to_string_lossy().into_owned();
        // A session written by a newer build must not brick an older one; an unreadable
        // blob means "no project", not "refuse to load".
        let state: PluginState = serde_json::from_str(&raw).unwrap_or_default();
        plugin.set_state(state);
        0
    })
}

/// Version, commit and dirty flag, as JSON.
///
/// This is what the plugin's view shows. It is the only claim about which build is
/// running that comes from the running code rather than from what is on disk.
#[no_mangle]
pub extern "C" fn unplugged_plugin_build_info_json() -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        to_c_string(serde_json::to_string(&BuildInfo::get()).unwrap_or_else(|_| "{}".into()))
    })
}

/// One render block. **Called on the audio thread.**
///
/// # Safety
/// - `handle` must come from `unplugged_plugin_create` and not be used concurrently.
/// - `out` must be valid for `capacity` `CRenderedEvent` writes.
#[no_mangle]
pub unsafe extern "C" fn unplugged_plugin_render(
    handle: *mut std::ffi::c_void,
    host_beats: f64,
    tempo_bpm: f64,
    playing: bool,
    frames: u32,
    out: *mut CRenderedEvent,
    capacity: u32,
) -> u32 {
    guard(0, || {
        if out.is_null() || capacity == 0 {
            return 0;
        }
        let Some(plugin) = plugin(handle) else { return 0 };

        let slice = std::slice::from_raw_parts_mut(out, capacity as usize);
        plugin.render(
            HostTransport { beats: host_beats, tempo_bpm, playing },
            frames,
            slice,
        )
    })
}
