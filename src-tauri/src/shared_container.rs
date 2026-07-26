//! Where projects live, so the app and the plugin agree about it.
//!
//! A sandboxed app and a sandboxed app extension get **separate containers**. The AUv3
//! cannot read what the app wrote unless the two are pointed at the same directory, and
//! the failure when they are not is silent: the plugin shows an empty project list, which
//! is indistinguishable from never having run the app.
//!
//! There are two ways to point them at the same directory, and which is available depends
//! only on how the build was signed:
//!
//! 1. **An App Group.** What ships. Needs a paid team and a provisioning profile — App
//!    Groups cannot be registered on a free Apple ID at all.
//! 2. **A fixed path under the home directory**, which an ad-hoc-signed extension reaches
//!    through a read-only sandbox exception. No account needed, which is what makes local
//!    development possible.
//!
//! The app prefers the group and falls back to the fixed path. Both are readable by the
//! plugin; the app's own per-application directory, which is what earlier builds used, is
//! not, so it is only ever the last resort.

use std::path::{Path, PathBuf};

/// Must match `plugin/Support/UnpluggedAU-Signed.entitlements` and
/// `src-tauri/Entitlements-Signed.plist`.
///
/// Only read on Apple targets; kept unconditional so the three places that must agree are
/// greppable from one another rather than hidden behind a `cfg`.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub const APP_GROUP: &str = "group.com.unplugged.daw";

/// The home-relative directory both sides use when there is no App Group.
///
/// Must match `homeRelativeDataDirectory()` in `plugin/UnpluggedAU/UnpluggedAudioUnit.swift`
/// and the sandbox exception in `plugin/Support/UnpluggedAU.entitlements`. A test holds all
/// three to it, because disagreement is silent.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const HOME_RELATIVE_DIR: &str = "Library/Application Support/Unplugged";

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

/// `~/Library/Application Support/Unplugged`, macOS only.
///
/// `HOME` rather than the real account home on purpose: if this process *is* sandboxed,
/// `HOME` is its container and the real home is unwritable, so writing where we can is the
/// only correct answer. That case is the one an App Group exists to solve, and
/// [`Sharing::HomeDirectory`] says as much rather than promising more than it can deliver.
///
/// Not iOS: every process there is sandboxed and there is no shared home to fall back to,
/// so the group is the only arrangement that works.
#[cfg(target_os = "macos")]
fn home_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(HOME_RELATIVE_DIR))
}

#[cfg(not(target_os = "macos"))]
fn home_data_dir() -> Option<PathBuf> {
    None
}

/// How the plugin can — or cannot — reach what the app writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sharing {
    /// The App Group container. Works for a sandboxed app and a sandboxed extension.
    AppGroup,
    /// The fixed home-relative path, reached by the plugin's read-only sandbox exception.
    /// Works for an ad-hoc-signed build, which is every build without a paid team.
    HomeDirectory,
    /// The app's own directory. The plugin has never heard of it.
    Private,
}

/// Decide where data lives, and report whether the plugin will be able to see it.
pub struct DataLocation {
    pub dir: PathBuf,
    pub sharing: Sharing,
    /// Set when projects were carried over from a directory an earlier build used.
    pub migrated_from: Option<PathBuf>,
}

/// Resolve the data directory, carrying existing projects across if needed.
///
/// `fallback` is the app's own directory — what every build before the plugin used.
pub fn resolve(fallback: PathBuf) -> DataLocation {
    resolve_from(fallback, group_container(), home_data_dir())
}

/// The decision, separated from the platform lookups so every branch is testable on any
/// host — including the two the CI machine can never reach.
fn resolve_from(
    fallback: PathBuf,
    group: Option<PathBuf>,
    home: Option<PathBuf>,
) -> DataLocation {
    for (candidate, sharing) in
        [(group, Sharing::AppGroup), (home, Sharing::HomeDirectory)]
    {
        let Some(dir) = candidate else { continue };
        // A directory that cannot be created is not a location. Trying the next one beats
        // reporting a shared directory that no write will ever land in.
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        // Carry projects over the first time. **Copied, not moved**: this runs on a
        // machine holding the only copy of someone's work, and a failed move is
        // unrecoverable while a failed copy costs disk. The original is left in place
        // deliberately — it can be removed once the shared location has proved itself.
        let migrated_from = migrate_projects(&fallback, &dir);
        return DataLocation { dir, sharing, migrated_from };
    }

    DataLocation { dir: fallback, sharing: Sharing::Private, migrated_from: None }
}

fn migrate_projects(from: &Path, to: &Path) -> Option<PathBuf> {
    // Migrating a directory onto itself would be a no-op at best and a recursive copy at
    // worst; it happens whenever the app is already running from the shared location.
    if from == to {
        return None;
    }

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
    fn the_group_container_wins_when_there_is_one() {
        let root = temp("group-wins");
        let location = resolve_from(
            root.join("own"),
            Some(root.join("group")),
            Some(root.join("home")),
        );
        assert_eq!(location.dir, root.join("group"));
        assert_eq!(location.sharing, Sharing::AppGroup);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn without_a_group_the_home_directory_is_still_shared() {
        // The ad-hoc-signed case, which is every build without a paid Apple team. The
        // plugin reads this path through a sandbox exception, so it is not a degraded
        // mode — it is the one local development runs in.
        let root = temp("home");
        let location = resolve_from(root.join("own"), None, Some(root.join("home")));
        assert_eq!(location.dir, root.join("home"));
        assert_eq!(location.sharing, Sharing::HomeDirectory);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn with_neither_the_app_keeps_its_own_directory_and_says_so() {
        // Every non-Apple host, and iOS without the group entitlement.
        let fallback = temp("fallback");
        let location = resolve_from(fallback.clone(), None, None);
        assert_eq!(location.dir, fallback);
        assert_eq!(location.sharing, Sharing::Private);
        std::fs::remove_dir_all(&fallback).ok();
    }

    #[test]
    fn a_location_that_cannot_be_created_is_skipped_rather_than_reported() {
        // A file where the group container should be: creating the directory fails, and
        // claiming the plugin can see it would be a lie the app never gets to correct.
        let root = temp("uncreatable");
        std::fs::write(root.join("blocked"), b"not a directory").unwrap();

        let location = resolve_from(
            root.join("own"),
            Some(root.join("blocked/inside")),
            Some(root.join("home")),
        );
        assert_eq!(location.dir, root.join("home"));
        assert_eq!(location.sharing, Sharing::HomeDirectory);
        std::fs::remove_dir_all(&root).ok();
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

    #[test]
    fn the_three_places_that_name_the_shared_path_agree() {
        // The reason this is a test and not a comment: if the extension's entitlement,
        // its Swift, and this file drift apart, nothing fails. The app writes to one path,
        // the plugin reads another, and the only symptom is an empty project list in Logic
        // — which is exactly what "the app has never run" looks like.
        let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("plugin");

        let entitlements =
            std::fs::read_to_string(plugin.join("Support/UnpluggedAU.entitlements")).unwrap();
        assert!(
            entitlements.contains(&format!("<string>/{HOME_RELATIVE_DIR}/</string>")),
            "the sandbox exception does not name /{HOME_RELATIVE_DIR}/ — the plugin will \
             be denied the directory this app writes to"
        );

        let swift =
            std::fs::read_to_string(plugin.join("UnpluggedAU/UnpluggedAudioUnit.swift")).unwrap();
        assert!(
            swift.contains(&format!("\"/{HOME_RELATIVE_DIR}\"")),
            "UnpluggedAudioUnit.swift does not build /{HOME_RELATIVE_DIR} — it will read \
             a directory this app never writes to"
        );
    }

    #[test]
    fn a_directory_is_never_migrated_onto_itself() {
        // Which is what happens on every launch after the first, once the app's own
        // directory *is* the shared one. Copying a tree into itself would recurse.
        let root = temp("self");
        std::fs::create_dir_all(root.join("projects/song")).unwrap();
        std::fs::write(root.join("projects/song/project.json"), b"{}").unwrap();

        assert_eq!(migrate_projects(&root, &root), None);
        assert!(root.join("projects/song/project.json").is_file());
        std::fs::remove_dir_all(&root).ok();
    }
}
