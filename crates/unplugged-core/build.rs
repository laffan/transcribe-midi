//! Bakes the build's identity in, for `build_info.rs`.
//!
//! Every value degrades to something usable rather than failing: a build from a source
//! tarball with no git, or on a machine without the git binary, still compiles and still
//! reports *something*. A build script that can break the build over a cosmetic string is
//! a bad trade.

use std::process::Command;

fn main() {
    // Re-run when HEAD moves. `.git/HEAD` covers commits and branch switches; the index
    // covers staging, which is the cheapest proxy for "the tree might now be dirty".
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-env-changed=UNPLUGGED_BUILT_AT");

    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into());

    // `--quiet` exits non-zero when there *are* changes, which is the signal we want.
    // Absence of git is reported as clean rather than dirty: claiming a build is modified
    // when we simply cannot tell would make the marker meaningless.
    let dirty = match Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
    {
        Ok(status) => !status.success(),
        Err(_) => false,
    };

    // Reproducible builds can pin this; otherwise it is now. `SOURCE_DATE_EPOCH` is the
    // convention, so it is honoured.
    let built_at = std::env::var("UNPLUGGED_BUILT_AT")
        .ok()
        .or_else(|| std::env::var("SOURCE_DATE_EPOCH").ok().and_then(epoch_to_rfc3339))
        .unwrap_or_else(now_rfc3339);

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".into());

    println!("cargo:rustc-env=UNPLUGGED_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=UNPLUGGED_GIT_DIRTY={}", u8::from(dirty));
    println!("cargo:rustc-env=UNPLUGGED_BUILT_AT={built_at}");
    println!("cargo:rustc-env=UNPLUGGED_PROFILE={profile}");
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn now_rfc3339() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch(seconds)
}

fn epoch_to_rfc3339(raw: String) -> Option<String> {
    raw.trim().parse::<u64>().ok().map(format_epoch)
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Hand-rolled because a build script should not pull `chrono` into the dependency tree
/// of the one crate that is meant to have almost none. Proleptic Gregorian, which is
/// correct for every date this will ever see.
fn format_epoch(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    let (hour, minute, second) = (rest / 3600, (rest % 3600) / 60, rest % 60);

    // Civil-from-days, shifting the epoch to 0000-03-01 so leap days land at the end of
    // the cycle and the month arithmetic has no special cases.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
