import Foundation
import Security

/// Phase 6: the Anthropic API key lives here and nowhere else.
///
/// The spec is explicit that the key must not be in `project.json`, not in localStorage
/// and never handed to the webview. The Keychain is the only store on either platform
/// that survives an app reinstall being the *user's* decision rather than ours, is
/// encrypted at rest, and is not readable by another app. Rust calls these three
/// functions and holds the key only for the lifetime of one HTTPS request.
///
/// `kSecClassGenericPassword` with a fixed service name, keyed by account, so a second
/// secret (a different provider, say) needs no new code.
///
/// Written in Swift rather than bound through a Rust keychain crate for the same reason
/// the audio graph is: this is one implementation that compiles for macOS and iOS
/// identically, and the Security framework API is genuinely the same on both. A Rust
/// crate would be a third-party bet on iOS support we cannot verify without a device.
///
/// **Only `unplugged_platform_keychain_get` asks for `kSecReturnData`, and it is the only
/// thing here that can raise "wants to use your confidential information".** macOS
/// consults an item's access control list when the *secret* is released, not when its
/// attributes are read — so "is a key set?" and "what are its last four characters?" are
/// answered from attributes and are silent. Answering them by reading the key, which is
/// what this file used to do, decrypted the user's API key at launch and put a dialog in
/// front of an app that had not been asked to do anything yet. There is a test in
/// `unplugged-ai` that reads this file and fails if `kSecReturnData` appears anywhere
/// else.

private let service = "com.unplugged.daw"

/// Shown in Keychain Access, where "com.unplugged.daw" alone tells the user nothing.
private let label = "Unplugged — Anthropic API key"

private func accountString(_ pointer: UnsafePointer<CChar>?) -> String? {
    guard let pointer else { return nil }
    let account = String(cString: pointer)
    return account.isEmpty ? nil : account
}

private func baseQuery(_ account: String) -> [String: Any] {
    [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
    ]
}

/// Store or replace a secret. Returns 0 on success, the `OSStatus` otherwise.
///
/// `hint` is stored as the item's comment — an attribute, not the secret — so the
/// settings panel can show which key is set without anything decrypting it. It is the
/// last four characters, which is already what crosses into the webview.
@_cdecl("unplugged_platform_keychain_set")
public func unplugged_platform_keychain_set(
    _ cAccount: UnsafePointer<CChar>?,
    _ cSecret: UnsafePointer<CChar>?,
    _ cHint: UnsafePointer<CChar>?
) -> Int32 {
    guard let account = accountString(cAccount), let cSecret else { return -1 }
    guard let data = String(cString: cSecret).data(using: .utf8) else { return -2 }

    // Delete first rather than branching on add-vs-update: `SecItemUpdate` fails when
    // there is nothing to update, and the two-call dance is the documented idiom.
    SecItemDelete(baseQuery(account) as CFDictionary)

    var query = baseQuery(account)
    query[kSecValueData as String] = data
    query[kSecAttrLabel as String] = label
    if let cHint {
        query[kSecAttrComment as String] = String(cString: cHint)
    }
    // The key is needed for a network call the user initiates, so it never has to be
    // readable while the device is locked. `AfterFirstUnlock` would be readable by a
    // background task; `WhenUnlocked` is the tighter class and costs nothing here.
    query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlocked

    let status = SecItemAdd(query as CFDictionary, nil)
    return status == errSecSuccess ? 0 : Int32(status)
}

/// Attributes only — never the secret. The query behind both silent lookups below.
private func attributes(_ account: String) -> [String: Any]? {
    var query = baseQuery(account)
    query[kSecReturnAttributes as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne

    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else { return nil }
    return item as? [String: Any]
}

/// Whether a secret exists. 1 for yes, 0 for no. Does not touch the secret.
@_cdecl("unplugged_platform_keychain_has")
public func unplugged_platform_keychain_has(_ cAccount: UnsafePointer<CChar>?) -> Int32 {
    guard let account = accountString(cAccount) else { return 0 }
    return attributes(account) == nil ? 0 : 1
}

/// The hint stored alongside a secret, or NULL when there is no item or no hint.
///
/// A key written by a build before hints existed has no comment: the caller gets NULL and
/// the panel says "stored" instead of the last four. That is the right trade — the
/// alternative is decrypting the key to recover four characters of it.
@_cdecl("unplugged_platform_keychain_hint")
public func unplugged_platform_keychain_hint(
    _ cAccount: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>? {
    guard let account = accountString(cAccount),
          let found = attributes(account),
          let comment = found[kSecAttrComment as String] as? String,
          !comment.isEmpty
    else {
        return nil
    }
    return strdup(comment)
}

/// Read a secret. Returns a malloc'd C string the caller must pass to
/// `unplugged_platform_string_free`, or NULL when there is no such item.
@_cdecl("unplugged_platform_keychain_get")
public func unplugged_platform_keychain_get(
    _ cAccount: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>? {
    guard let account = accountString(cAccount) else { return nil }

    var query = baseQuery(account)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne

    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
          let data = item as? Data,
          let secret = String(data: data, encoding: .utf8)
    else {
        return nil
    }

    return strdup(secret)
}

/// Remove a secret. Returns 0 when the item is gone, whether or not it existed.
@_cdecl("unplugged_platform_keychain_delete")
public func unplugged_platform_keychain_delete(_ cAccount: UnsafePointer<CChar>?) -> Int32 {
    guard let account = accountString(cAccount) else { return -1 }
    let status = SecItemDelete(baseQuery(account) as CFDictionary)
    return (status == errSecSuccess || status == errSecItemNotFound) ? 0 : Int32(status)
}

/// Free a string returned by `unplugged_platform_keychain_get`.
///
/// The allocation comes from `strdup`, so it must be released with `free` and not by
/// Rust's allocator — hence a dedicated entry point rather than `CString::from_raw`.
@_cdecl("unplugged_platform_string_free")
public func unplugged_platform_string_free(_ pointer: UnsafeMutablePointer<CChar>?) {
    guard let pointer else { return }
    free(pointer)
}
