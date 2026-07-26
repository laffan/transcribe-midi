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
        fn unplugged_platform_keychain_set(account: *const c_char, secret: *const c_char) -> i32;
        fn unplugged_platform_keychain_get(account: *const c_char) -> *mut c_char;
        fn unplugged_platform_keychain_delete(account: *const c_char) -> i32;
        fn unplugged_platform_string_free(pointer: *mut c_char);
    }

    fn c(text: &str) -> Result<CString> {
        CString::new(text).map_err(|_| KeychainError::Malformed)
    }

    pub fn set(account: &str, secret: &str) -> Result<()> {
        let account = c(account)?;
        let secret = c(secret)?;
        match unsafe { unplugged_platform_keychain_set(account.as_ptr(), secret.as_ptr()) } {
            0 => Ok(()),
            status => Err(KeychainError::Write(status)),
        }
    }

    pub fn get(account: &str) -> Result<String> {
        let account = c(account)?;
        // SAFETY: the pointer is either NULL or a `strdup`'d NUL-terminated string, and
        // it is copied and freed before this function returns. It is freed through the
        // Swift side's `free`, not Rust's allocator — `CString::from_raw` here would be
        // a cross-allocator free.
        unsafe {
            let pointer = unplugged_platform_keychain_get(account.as_ptr());
            if pointer.is_null() {
                return Err(KeychainError::Missing);
            }
            let value = CStr::from_ptr(pointer).to_string_lossy().into_owned();
            unplugged_platform_string_free(pointer);
            Ok(value)
        }
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

    pub fn set(account: &str, secret: &str) -> Result<()> {
        let mut guard = store().lock().map_err(|_| KeychainError::Write(-1))?;
        guard.insert(account.to_string(), secret.to_string());
        Ok(())
    }

    pub fn get(account: &str) -> Result<String> {
        let guard = store().lock().map_err(|_| KeychainError::Missing)?;
        guard.get(account).cloned().ok_or(KeychainError::Missing)
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
    platform::set(ANTHROPIC_ACCOUNT, key)
}

/// Read the Anthropic API key. The returned `String` should be dropped promptly.
pub fn api_key() -> Result<String> {
    platform::get(ANTHROPIC_ACCOUNT)
}

pub fn clear_api_key() -> Result<()> {
    platform::delete(ANTHROPIC_ACCOUNT)
}

pub fn has_api_key() -> bool {
    api_key().is_ok()
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
/// Never the whole key: this string crosses into the webview, and the requirement is
/// that the key does not. Four characters is enough to tell two keys apart and useless
/// to anyone who intercepts it.
pub fn key_hint() -> Option<String> {
    let key = api_key().ok()?;
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    Some(format!("…{tail}"))
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
