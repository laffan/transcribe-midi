//! Where projects live, so the app and the plugin agree about it.
//!
//! A sandboxed app and a sandboxed app extension get **separate containers**. The AUv3
//! cannot read what the app wrote unless both go through an App Group, and the failure
//! when they do not is silent: the plugin shows an empty project list, which is
//! indistinguishable from never having run the app.
//!
//! So the app prefers the group container when one is available, and falls back to its
//! own directory when it is not — an unsigned build, or a profile without the entitlement.
//! The fallback is not a failure for the standalone; it only means the plugin has nothing
//! to play.

use std::path::{Path, PathBuf};

/// Must match `plugin/Support/UnpluggedAU.entitlements` and `src-tauri/Entitlements.plist`.
///
/// Only read on Apple targets; kept unconditional so the three places that must agree are
/// greppable from one another rather than hidden behind a `cfg`.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub const APP_GROUP: &str = "group.com.unplugged.daw";

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn group_container() -> Option<PathBuf> {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    extern "C" {
        fn unplugged_platform_group_container(group: *const c_char) -> *mut c_char;
        fn unplugged_platform_string_free(pointer: *mut c_char);
    }

    let group = CString::new(APP_GROUP).ok()?;
    // SAFETY: the pointer is either NULL or a `strdup`'d string, copied and freed here.
    unsafe {
        let pointer = unplugged_platform_group_container(group.as_ptr());
        if pointer.is_null() {
            return None;
        }
        let path = CStr::from_ptr(pointer).to_string_lossy().into_owned();
        unplugged_platform_string_free(pointer);
        (!path.is_empty()).then(|| PathBuf::from(path))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn group_container() -> Option<PathBuf> {
    None
}

/// Decide where data lives, and report whether the plugin will be able to see it.
pub struct DataLocation {
    pub dir: PathBuf,
    /// False when the group container was unavailable, so the plugin will find nothing.
    pub shared_with_plugin: bool,
    /// Set when projects were carried over from the app's own directory.
    pub migrated_from: Option<PathBuf>,
}

/// Resolve the data directory, carrying existing projects across if needed.
///
/// `fallback` is the app's own directory — what every build before the plugin used.
pub fn resolve(fallback: PathBuf) -> DataLocation {
    let Some(shared) = group_container() else {
        return DataLocation { dir: fallback, shared_with_plugin: false, migrated_from: None };
    };

    if std::fs::create_dir_all(&shared).is_err() {
        return DataLocation { dir: fallback, shared_with_plugin: false, migrated_from: None };
    }

    // Carry projects over the first time. **Copied, not moved**: this runs on a machine
    // holding the only copy of someone's work, and a failed move is unrecoverable while a
    // failed copy costs disk. The original is left in place deliberately — it can be
    // removed once the shared location has proved itself.
    let migrated_from = migrate_projects(&fallback, &shared);

    DataLocation { dir: shared, shared_with_plugin: true, migrated_from }
}

fn migrate_projects(from: &Path, to: &Path) -> Option<PathBuf> {
    let source = from.join("projects");
    let destination = to.join("projects");

    // Only ever into an empty destination. Merging two divergent project directories is a
    // conflict-resolution problem, and guessing at it would lose work.
    if !source.is_dir() || destination.exists() {
        return None;
    }

    copy_tree(&source, &destination).ok()?;
    Some(source)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("unplugged-container-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn without_a_group_container_the_app_keeps_its_own_directory() {
        // Which is what happens on every non-Apple host, and on an unsigned build.
        let fallback = temp("fallback");
        let location = resolve(fallback.clone());
        assert_eq!(location.dir, fallback);
        assert!(!location.shared_with_plugin);
        std::fs::remove_dir_all(&fallback).ok();
    }

    #[test]
    fn projects_are_copied_across_but_never_merged() {
        let root = temp("migrate");
        let old = root.join("old");
        let new = root.join("new");

        std::fs::create_dir_all(old.join("projects/song/tracks")).unwrap();
        std::fs::write(old.join("projects/song/project.json"), b"{}").unwrap();
        std::fs::write(old.join("projects/song/tracks/a.mid"), b"MThd").unwrap();

        assert_eq!(migrate_projects(&old, &new).as_deref(), Some(old.join("projects").as_path()));
        assert!(new.join("projects/song/project.json").is_file());
        assert!(new.join("projects/song/tracks/a.mid").is_file());
        // Copied, not moved: the original is the only copy of someone's work until the
        // new location has proved itself.
        assert!(old.join("projects/song/project.json").is_file());

        // A second run must not touch a destination that already has projects — merging
        // two divergent directories is a conflict-resolution problem, not a copy.
        std::fs::write(new.join("projects/song/project.json"), b"{\"edited\":true}").unwrap();
        assert_eq!(migrate_projects(&old, &new), None);
        assert_eq!(
            std::fs::read(new.join("projects/song/project.json")).unwrap(),
            b"{\"edited\":true}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nothing_to_migrate_is_not_an_error() {
        let root = temp("nothing");
        assert_eq!(migrate_projects(&root.join("old"), &root.join("new")), None);
        std::fs::remove_dir_all(&root).ok();
    }
}
