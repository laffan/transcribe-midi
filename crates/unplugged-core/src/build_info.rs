//! Which build is this?
//!
//! A question that is trivial in a standalone app — quit it, look at the window — and
//! genuinely hard in a plugin. Logic caches Audio Unit scans aggressively, keeps the
//! extension alive in a separate hosting process, and will happily run a copy you
//! replaced ten minutes ago. Without a stamp visible *inside the plugin window* there is
//! no way to tell a fix that did not work from a fix that was never loaded.
//!
//! So the stamp is baked in at compile time and surfaced everywhere the UI appears.
//!
//! The values come from `build.rs` through environment variables. They are deliberately
//! forgiving: a build from a tarball with no git history still produces a usable answer
//! rather than failing to compile.

use serde::Serialize;

/// Everything needed to identify a build, in one payload.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BuildInfo {
    /// Semantic version from `Cargo.toml`.
    pub version: &'static str,
    /// Short commit hash, or `"unknown"` outside a git checkout.
    pub commit: &'static str,
    /// True when the working tree had uncommitted changes at build time.
    ///
    /// The single most useful field here. "Did my edit make it in?" is the question being
    /// asked, and a clean hash that matches the last commit answers it wrongly when the
    /// build was made from a dirty tree.
    pub dirty: bool,
    /// RFC 3339 UTC timestamp of the build.
    pub built_at: &'static str,
    /// `debug` or `release`.
    pub profile: &'static str,
}

impl BuildInfo {
    pub const fn get() -> BuildInfo {
        BuildInfo {
            version: env!("CARGO_PKG_VERSION"),
            commit: env!("UNPLUGGED_GIT_COMMIT"),
            dirty: matches!(env!("UNPLUGGED_GIT_DIRTY").as_bytes(), b"1"),
            built_at: env!("UNPLUGGED_BUILT_AT"),
            profile: env!("UNPLUGGED_PROFILE"),
        }
    }

    /// One line, short enough for a plugin's title bar.
    ///
    /// The `+` on a dirty build is the important character: it is the difference between
    /// "this is commit abc1234" and "this is *something like* commit abc1234".
    pub fn short(&self) -> String {
        format!(
            "{} ({}{})",
            self.version,
            self.commit,
            if self.dirty { "+" } else { "" }
        )
    }

    /// Everything, for an About panel or a bug report.
    pub fn long(&self) -> String {
        format!(
            "Unplugged {} · {}{} · {} · built {}",
            self.version,
            self.commit,
            if self.dirty { " (modified)" } else { "" },
            self.profile,
            self.built_at,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_stamps_itself() {
        let info = BuildInfo::get();
        assert!(!info.version.is_empty());
        assert!(!info.commit.is_empty());
        assert!(!info.built_at.is_empty());
        assert!(matches!(info.profile, "debug" | "release"));
    }

    #[test]
    fn a_dirty_build_says_so() {
        // Constructed rather than read, so the assertion is about the formatting rather
        // than about whatever state this particular checkout happens to be in.
        let clean = BuildInfo {
            version: "0.1.0",
            commit: "abc1234",
            dirty: false,
            built_at: "2026-01-01T00:00:00Z",
            profile: "release",
        };
        let dirty = BuildInfo { dirty: true, ..clean.clone() };

        assert_eq!(clean.short(), "0.1.0 (abc1234)");
        assert_eq!(dirty.short(), "0.1.0 (abc1234+)");
        assert!(dirty.long().contains("(modified)"));
        assert!(!clean.long().contains("modified"));
    }
}
