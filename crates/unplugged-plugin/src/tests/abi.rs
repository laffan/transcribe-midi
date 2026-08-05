//! Null everywhere. Every entry point must return a fallback, not crash the host.

use super::*;
use crate::abi::*;

#[test]
fn the_c_abi_survives_null_everywhere() {
    // Every one of these is reachable from Swift, and any of them crashing takes the
    // host down with the user's session.
    unsafe {
        assert!(unplugged_plugin_create(std::ptr::null()).is_null());
        unplugged_plugin_destroy(std::ptr::null_mut());
        unplugged_plugin_string_free(std::ptr::null_mut());
        assert!(unplugged_plugin_projects_json(std::ptr::null_mut()).is_null());
        assert_eq!(unplugged_plugin_open(std::ptr::null_mut(), std::ptr::null()), -1);
        assert!(unplugged_plugin_last_error(std::ptr::null_mut()).is_null());
        unplugged_plugin_prepare(std::ptr::null_mut(), 48_000.0);
        assert_eq!(unplugged_plugin_track_count(std::ptr::null_mut()), 0);
        assert!(unplugged_plugin_state_json(std::ptr::null_mut()).is_null());
        assert_eq!(
            unplugged_plugin_set_state_json(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
        assert_eq!(
            unplugged_plugin_render(std::ptr::null_mut(), 0.0, 120.0, true, 512, std::ptr::null_mut(), 0),
            0
        );

        // The one that always works: the build stamp does not need an instance,
        // because the view shows it before anything is loaded.
        let info = unplugged_plugin_build_info_json();
        assert!(!info.is_null());
        let text = CStr::from_ptr(info).to_string_lossy().into_owned();
        assert!(text.contains("version"), "{text}");
        unplugged_plugin_string_free(info);
    }
}
