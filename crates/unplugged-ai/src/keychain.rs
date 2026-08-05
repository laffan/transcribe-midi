//! The API key store.
//!
//! The spec is unambiguous: the key lives in the platform Keychain, not in
//! `project.json`, not in localStorage, and never in the webview. This module is the
//! only thing in the app that can read it, and the value it returns is held for the
//! lifetime of one HTTPS request and then dropped.
//!
//! On Apple targets this calls into `swift/UnpluggedAudio/Sources/UnpluggedPlatform/
//! Keychain.swift` over the same C ABI the rest of the platform surface uses. Off-Apple
//! there is no Keychain and there is no substitute worth pretending is one, so the store
//! is process-local and says so — the browser preview has no AI anyway.

/// Keychain account name for the Anthropic key.
pub const ANTHROPIC_ACCOUNT: &str = "anthropic-api-key";

#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    #[error("the API key contains a character that cannot be stored")]
    Malformed,

    #[error("could not write to the Keychain (status {0})")]
    Write(i32),

    #[error("could not remove the key from the Keychain (status {0})")]
    Delete(i32),

    #[error("no API key has been set")]
    Missing,
}

pub type Result<T> = std::result::Result<T, KeychainError>;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod platform {
    use super::{KeychainError, Result};
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    extern "C" {
        fn unplugged_platform_keychain_set(
            account: *const c_char,
            secret: *const c_char,
            hint: *const c_char,
        ) -> i32;
        fn unplugged_platform_keychain_get(account: *const c_char) -> *mut c_char;
        fn unplugged_platform_keychain_has(account: *const c_char) -> i32;
        fn unplugged_platform_keychain_hint(account: *const c_char) -> *mut c_char;
        fn unplugged_platform_keychain_delete(account: *const c_char) -> i32;
        fn unplugged_platform_string_free(pointer: *mut c_char);
    }

    fn c(text: &str) -> Result<CString> {
        CString::new(text).map_err(|_| KeychainError::Malformed)
    }

    /// Copy a string out of Swift and free it there. Freed through Swift's `free`, not
    /// Rust's allocator — `CString::from_raw` here would be a cross-allocator free.
    ///
    /// # Safety
    /// `pointer` must be NULL or a `strdup`'d NUL-terminated string owned by Swift.
    unsafe fn take(pointer: *mut c_char) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        let value = CStr::from_ptr(pointer).to_string_lossy().into_owned();
        unplugged_platform_string_free(pointer);
        Some(value)
    }

    pub fn set(account: &str, secret: &str, hint: &str) -> Result<()> {
        let account = c(account)?;
        let secret = c(secret)?;
        let hint = c(hint)?;
        match unsafe {
            unplugged_platform_keychain_set(account.as_ptr(), secret.as_ptr(), hint.as_ptr())
        } {
            0 => Ok(()),
            status => Err(KeychainError::Write(status)),
        }
    }

    /// Existence, from the item's attributes. Never releases the secret, so it never
    /// raises the Keychain's "wants to use your confidential information" dialog.
    pub fn has(account: &str) -> bool {
        let Ok(account) = c(account) else { return false };
        unsafe { unplugged_platform_keychain_has(account.as_ptr()) == 1 }
    }

    /// The hint stored beside the secret. Attributes only, for the same reason.
    pub fn hint(account: &str) -> Option<String> {
        let account = c(account).ok()?;
        // SAFETY: the returned pointer is NULL or a `strdup`'d string, taken and freed
        // before this returns.
        unsafe { take(unplugged_platform_keychain_hint(account.as_ptr())) }
    }

    /// The secret itself.
    ///
    /// **The only call in the app that decrypts the key**, and therefore the only one
    /// that can prompt. Reached from exactly two places, both immediately before an
    /// HTTPS request.
    pub fn get(account: &str) -> Result<String> {
        let account = c(account)?;
        // SAFETY: the pointer is either NULL or a `strdup`'d NUL-terminated string,
        // taken and freed before this returns.
        unsafe { take(unplugged_platform_keychain_get(account.as_ptr())) }
            .ok_or(KeychainError::Missing)
    }

    pub fn delete(account: &str) -> Result<()> {
        let account = c(account)?;
        match unsafe { unplugged_platform_keychain_delete(account.as_ptr()) } {
            0 => Ok(()),
            status => Err(KeychainError::Delete(status)),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod platform {
    use super::{KeychainError, Result};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// Process-local, deliberately not persisted.
    ///
    /// Writing a key to a file off-Apple would be a security regression dressed up as a
    /// convenience: the whole point of the requirement is that the key is never at rest
    /// in the clear. A dev host restarting and forgetting the key is the correct
    /// behaviour, not a bug to work around.
    fn store() -> &'static Mutex<HashMap<String, String>> {
        static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        STORE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub fn set(account: &str, secret: &str, _hint: &str) -> Result<()> {
        let mut guard = store().lock().map_err(|_| KeychainError::Write(-1))?;
        guard.insert(account.to_string(), secret.to_string());
        Ok(())
    }

    pub fn get(account: &str) -> Result<String> {
        let guard = store().lock().map_err(|_| KeychainError::Missing)?;
        guard.get(account).cloned().ok_or(KeychainError::Missing)
    }

    /// There is no dialog to avoid here, and no attributes to store a hint in, so both
    /// of these read the map. The behaviour the caller sees is identical.
    pub fn has(account: &str) -> bool {
        store()
            .lock()
            .map(|guard| guard.contains_key(account))
            .unwrap_or(false)
    }

    pub fn hint(account: &str) -> Option<String> {
        let guard = store().lock().ok()?;
        guard.get(account).map(|key| super::hint_of(key))
    }

    pub fn delete(account: &str) -> Result<()> {
        let mut guard = store().lock().map_err(|_| KeychainError::Delete(-1))?;
        guard.remove(account);
        Ok(())
    }
}

/// Store the Anthropic API key.
///
/// Validated here rather than in the platform layer so the rule is the same everywhere:
/// an API key is printable ASCII, and anything else is a paste accident — most often a
/// stray newline, occasionally an embedded NUL that would silently truncate the key on
/// the way through the C ABI and leave the user with an authentication error they cannot
/// explain.
pub fn set_api_key(key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_graphic()) {
        return Err(KeychainError::Malformed);
    }
    platform::set(ANTHROPIC_ACCOUNT, key, &hint_of(key))
}

/// The last four characters of a key, as the settings panel shows them.
///
/// Never more: this string crosses into the webview, and the requirement is that the key
/// does not. Four characters is enough to tell two keys apart and useless to anyone who
/// intercepts it — which is also why it is safe to keep as a Keychain *attribute*, where
/// it can be read without decrypting anything.
fn hint_of(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{tail}")
}

/// Read the Anthropic API key. The returned `String` should be dropped promptly.
pub fn api_key() -> Result<String> {
    platform::get(ANTHROPIC_ACCOUNT)
}

pub fn clear_api_key() -> Result<()> {
    platform::delete(ANTHROPIC_ACCOUNT)
}

/// Whether a key is set.
///
/// Asks whether the item exists rather than reading it. It used to read it, which is why
/// opening the app raised "Unplugged wants to use your confidential information" — the
/// prompt bar asks this on mount, so the app decrypted the user's API key at launch
/// before being asked to do anything with it.
pub fn has_api_key() -> bool {
    platform::has(ANTHROPIC_ACCOUNT)
}

/// True when this build can actually persist a key across launches.
///
/// Surfaced in the UI rather than hidden: a dev-host user who types a key into the
/// browser preview should be told it will not survive a restart.
pub fn is_persistent() -> bool {
    cfg!(any(target_os = "macos", target_os = "ios"))
}

/// The last four characters of the stored key, for the settings panel.
///
/// Read from the item's attributes, where [`set_api_key`] put them, rather than derived
/// from the secret. `None` means either that no key is set or that it was stored by a
/// build from before hints existed; the panel shows "stored" for both, which is worth far
/// more than decrypting a key to recover four characters of it.
pub fn key_hint() -> Option<String> {
    platform::hint(ANTHROPIC_ACCOUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_key_is_refused() {
        assert!(set_api_key("   ").is_err());
    }

    #[test]
    fn a_key_with_a_nul_byte_is_refused() {
        assert!(set_api_key("sk-ant\0evil").is_err());
    }

    #[test]
    fn a_hint_is_the_last_four_characters_and_nothing_more() {
        assert_eq!(hint_of("sk-ant-api03-EXAMPLE-abcd"), "…abcd");
        // A key shorter than the hint must not panic or reveal proportionally more.
        assert_eq!(hint_of("ab"), "…ab");
        assert_eq!(hint_of(""), "…");
    }

    /// The invariant that this whole module exists to hold, checked where the compiler
    /// cannot: **only the read path asks the Keychain for the secret.**
    ///
    /// Nothing fails if this drifts. The app simply starts decrypting the user's API key
    /// again on launch, and the only symptom is a system dialog that looks like it comes
    /// from macOS rather than from a line of our code — which is exactly how the original
    /// bug survived.
    #[test]
    fn only_the_read_path_asks_the_keychain_for_the_secret() {
        let swift = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../swift/UnpluggedAudio/Sources/UnpluggedPlatform/Keychain.swift"),
        )
        .expect("Keychain.swift moved — this test needs its new path");

        // Ignore the file's own prose about the rule; only the code counts.
        let code: String = swift
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            code.matches("kSecReturnData").count(),
            1,
            "kSecReturnData appears more than once in Keychain.swift — something other \
             than the read path is decrypting the key, which prompts the user"
        );

        let getter = code
            .find("func unplugged_platform_keychain_get")
            .expect("the read path is gone");
        let returns_data = code.find("kSecReturnData").expect("checked above");
        let next_entry = code[getter..]
            .find("@_cdecl")
            .map(|at| getter + at)
            .unwrap_or(code.len());

        assert!(
            (getter..next_entry).contains(&returns_data),
            "kSecReturnData is outside unplugged_platform_keychain_get"
        );

        for silent in ["unplugged_platform_keychain_has", "unplugged_platform_keychain_hint"] {
            assert!(code.contains(silent), "{silent} is missing — status would have to read the key");
        }
    }

    /// One test for the whole round trip, because the off-Apple store is process-global
    /// and parallel tests would race on it.
    #[test]
    fn set_read_hint_and_clear() {
        set_api_key("sk-ant-api03-EXAMPLE-not-a-real-key-abcd").unwrap();
        assert!(has_api_key());
        assert_eq!(key_hint().as_deref(), Some("…abcd"));

        clear_api_key().unwrap();
        assert!(!has_api_key());
        assert_eq!(key_hint(), None);
        // Clearing a key that is not there is not an error — the settings panel calls
        // this to mean "make sure there is no key", and it should be idempotent.
        assert!(clear_api_key().is_ok());
    }
}
