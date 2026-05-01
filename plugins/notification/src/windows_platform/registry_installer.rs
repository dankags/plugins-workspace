// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Registry installation and cleanup for Windows toast COM activation.
//!
//! # Keys written by this module
//!
//! Every key is under `HKCU` (current user) — no admin rights required.
//!
//! ```text
//! HKCU\Software\Classes\AppUserModelId\{aumid}
//!     DisplayName     REG_SZ  "My App"
//!     IconUri         REG_SZ  "C:\...\icon.ico"
//!     CustomActivator REG_SZ  "{GUID}"
//!
//! HKCU\Software\Classes\CLSID\{GUID}
//!     (Default)       REG_SZ  "My App"
//!
//! HKCU\Software\Classes\CLSID\{GUID}\LocalServer32
//!     (Default)       REG_SZ  "C:\...\myapp.exe"
//! ```
//!
//! # Cleanup
//!
//! Call [`uninstall`] to remove every key above. This should be called from:
//! 1. The `uninstall_notification_registration` Tauri command (user-triggered)
//! 2. A WiX / NSIS uninstall action (installer-triggered) — see
//!    `wix/notification_cleanup.wxs` in this repository

use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS},
    RegKey,
};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RegistryConfig {
    /// COM server GUID, with or without braces.
    pub com_server_guid: String,
    /// App User Model ID — must match `tauri.conf.json` identifier.
    pub aumid: String,
    /// Display name shown in Windows Settings → Apps.
    pub display_name: String,
    /// Absolute path to the app icon (`.ico` preferred). Must NOT start with `\\?\`.
    pub icon_path: Option<String>,
    /// Absolute path to the app executable. Defaults to the current executable.
    pub exe_path: Option<String>,
}

// ── Normalisation helpers ─────────────────────────────────────────────────────

/// Ensure a GUID string has surrounding braces: `{XXXXXXXX-...}`.
fn with_braces(guid: &str) -> String {
    let s = guid.trim();
    if s.starts_with('{') {
        s.to_string()
    } else {
        format!("{{{s}}}")
    }
}

/// Resolve the executable path, falling back to the current process.
fn resolve_exe(exe_path: Option<&str>) -> String {
    exe_path.map(|s| s.to_string()).unwrap_or_else(|| {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

// ── Install ───────────────────────────────────────────────────────────────────

/// Write all registry keys required for COM-activated toast notifications.
///
/// Idempotent — safe to call on every app startup. Existing correct values are
/// overwritten in place (no harm, no duplicate entries).
///
/// # What gets written
///
/// ```text
/// HKCU\Software\Classes\AppUserModelId\{aumid}
///     DisplayName     = "My App"
///     IconUri         = "C:\...\icon.ico"   (if icon_path provided)
///     CustomActivator = "{GUID}"
///
/// HKCU\Software\Classes\CLSID\{GUID}
///     (Default)       = "My App"
///
/// HKCU\Software\Classes\CLSID\{GUID}\LocalServer32
///     (Default)       = "C:\...\myapp.exe"
/// ```
pub fn install(config: &RegistryConfig) -> crate::Result<()> {
    let guid = with_braces(&config.com_server_guid);
    let exe = resolve_exe(config.exe_path.as_deref());
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // ── AUMID key ─────────────────────────────────────────────────────────────
    let aumid_path = format!("Software\\Classes\\AppUserModelId\\{}", config.aumid);
    let (aumid_key, _) = hkcu
        .create_subkey(&aumid_path)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    aumid_key
        .set_value("DisplayName", &config.display_name)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    if let Some(ref icon) = config.icon_path {
        aumid_key
            .set_value("IconUri", icon)
            .map_err(|e| crate::Error::Windows(e.to_string()))?;
    }

    // CustomActivator — the value that links the AUMID to the COM server.
    // Without this Windows shows toasts but never calls Activate().
    aumid_key
        .set_value("CustomActivator", &guid)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    log::info!(
        "[notification] registry: HKCU\\{}  CustomActivator={}",
        aumid_path,
        guid
    );

    // ── CLSID key ─────────────────────────────────────────────────────────────
    let clsid_path = format!("Software\\Classes\\CLSID\\{}", guid);
    let (clsid_key, _) = hkcu
        .create_subkey(&clsid_path)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    clsid_key
        .set_value("", &config.display_name)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    // ── LocalServer32 ─────────────────────────────────────────────────────────
    let server_path = format!("{}\\LocalServer32", clsid_path);
    let (server_key, _) = hkcu
        .create_subkey(&server_path)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    server_key
        .set_value("", &exe)
        .map_err(|e| crate::Error::Windows(e.to_string()))?;

    log::info!(
        "[notification] registry: HKCU\\{}  exe={}",
        server_path,
        exe
    );

    Ok(())
}

// ── CustomActivator (standalone write) ────────────────────────────────────────

/// Write only the `CustomActivator` value to the AUMID key.
///
/// Called separately from `install()` so it can be updated independently if
/// the GUID changes (e.g. after a major version bump).
pub fn write_custom_activator(aumid: &str, guid: &str) {
    let guid_with_braces = with_braces(guid);
    let key_path = format!("Software\\Classes\\AppUserModelId\\{aumid}");

    match RegKey::predef(HKEY_CURRENT_USER).create_subkey(&key_path) {
        Ok((key, _)) => match key.set_value("CustomActivator", &guid_with_braces) {
            Ok(_) => log::info!(
                "[notification] CustomActivator → HKCU\\{}  =  {}",
                key_path,
                guid_with_braces
            ),
            Err(e) => log::error!(
                "[notification] failed to write CustomActivator to HKCU\\{}: {}",
                key_path,
                e
            ),
        },
        Err(e) => log::error!(
            "[notification] failed to open/create HKCU\\{}: {}",
            key_path,
            e
        ),
    }
}

// ── Uninstall ─────────────────────────────────────────────────────────────────

/// Remove every registry key written by [`install`] and [`write_custom_activator`].
///
/// # Keys removed
///
/// ```text
/// HKCU\Software\Classes\AppUserModelId\{aumid}        (entire key tree)
/// HKCU\Software\Classes\CLSID\{GUID}                  (entire key tree)
/// ```
///
/// Each deletion is attempted independently — a failure on one key does not
/// prevent the others from being cleaned up. All errors are logged.
///
/// # When to call this
///
/// - From the `uninstall_notification_registration` Tauri command so users can
///   trigger it from your app's settings screen.
/// - From the WiX/NSIS uninstaller action (see `wix/notification_cleanup.wxs`).
/// - Automatically on next startup if the exe path in the registry no longer
///   points to an existing file (stale install detection).
///
/// # Idempotent
///
/// Safe to call even if the keys were never written or were already deleted.
/// `winreg` returns success when deleting a non-existent key.
pub fn uninstall(aumid: &str, guid: &str) -> crate::Result<()> {
    let guid_with_braces = with_braces(guid);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // ── 1. Remove AUMID key (DisplayName, IconUri, CustomActivator) ───────────
    //
    // delete_subkey_all removes the key AND all its values and child keys
    // recursively, so we do not need to delete individual values first.
    let aumid_path = format!("Software\\Classes\\AppUserModelId\\{aumid}");

    match hkcu.open_subkey_with_flags("Software\\Classes\\AppUserModelId", KEY_ALL_ACCESS) {
        Ok(parent) => match parent.delete_subkey_all(aumid) {
            Ok(_) => log::info!("[notification] uninstall: removed HKCU\\{}", aumid_path),
            Err(e) if is_not_found(&e) => log::debug!(
                "[notification] uninstall: HKCU\\{} already absent",
                aumid_path
            ),
            Err(e) => log::error!(
                "[notification] uninstall: failed to remove HKCU\\{}: {}",
                aumid_path,
                e
            ),
        },
        Err(e) if is_not_found(&e) => {
            log::debug!("[notification] uninstall: AppUserModelId parent key absent");
        }
        Err(e) => log::error!(
            "[notification] uninstall: failed to open AppUserModelId parent: {}",
            e
        ),
    }

    // ── 2. Remove CLSID key (LocalServer32 + default value) ──────────────────
    //
    // The CLSID key has a child key (LocalServer32), so we use delete_subkey_all
    // which handles the entire tree recursively.
    let clsid_parent_path = "Software\\Classes\\CLSID";
    let clsid_name = guid_with_braces.as_str(); // e.g. "{F3A7B2C1-...}"

    match hkcu.open_subkey_with_flags(clsid_parent_path, KEY_ALL_ACCESS) {
        Ok(parent) => match parent.delete_subkey_all(clsid_name) {
            Ok(_) => log::info!(
                "[notification] uninstall: removed HKCU\\{}\\{}",
                clsid_parent_path,
                clsid_name
            ),
            Err(e) if is_not_found(&e) => log::debug!(
                "[notification] uninstall: HKCU\\{}\\{} already absent",
                clsid_parent_path,
                clsid_name
            ),
            Err(e) => log::error!(
                "[notification] uninstall: failed to remove HKCU\\{}\\{}: {}",
                clsid_parent_path,
                clsid_name,
                e
            ),
        },
        Err(e) if is_not_found(&e) => {
            log::debug!("[notification] uninstall: CLSID parent key absent");
        }
        Err(e) => log::error!(
            "[notification] uninstall: failed to open CLSID parent: {}",
            e
        ),
    }

    log::info!("[notification] uninstall: registry cleanup complete");
    Ok(())
}

/// Returns `true` if the error is a "key/value not found" error.
/// Used to treat missing keys as already-clean rather than as errors.
fn is_not_found(e: &std::io::Error) -> bool {
    // ERROR_FILE_NOT_FOUND (2) or ERROR_PATH_NOT_FOUND (3)
    matches!(e.raw_os_error(), Some(2) | Some(3))
}

// ── Stale install detection ───────────────────────────────────────────────────

/// Check whether the `LocalServer32` path in the registry still points to a
/// file that exists on disk.
///
/// Returns `true` if the registration is stale (exe missing or key absent).
/// Call this on startup to auto-clean orphaned registrations from a previous
/// unclean uninstall.
pub fn is_registration_stale(guid: &str) -> bool {
    let guid_with_braces = with_braces(guid);
    let server_path = format!(
        "Software\\Classes\\CLSID\\{}\\LocalServer32",
        guid_with_braces
    );

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    match hkcu.open_subkey(&server_path) {
        Ok(key) => {
            let exe: String = key.get_value("").unwrap_or_default();
            if exe.is_empty() {
                return true;
            }
            // Strip \\?\ prefix that may have been written by an older version
            let clean_exe = if let Some(stripped) = exe.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                exe
            };
            !std::path::Path::new(&clean_exe).exists()
        }
        Err(_) => true, // key absent = stale
    }
}
