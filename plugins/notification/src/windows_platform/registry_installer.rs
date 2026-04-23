// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Windows registry installer for COM server and AUMID registration.
//!
//! Unpackaged (non-MSIX) apps on Windows 8+ must register two HKCU keys for
//! toast notifications to work correctly:
//!
//! 1. **COM server** — tells Windows where to find the executable when a
//!    background activation fires:
//!    ```text
//!    HKCU\Software\Classes\CLSID\{GUID}\LocalServer32 = "C:\path\to\app.exe"
//!    ```
//!
//! 2. **AUMID** — registers the App User Model ID so Windows knows which app
//!    owns a notification, enabling the Action Center and Notification settings:
//!    ```text
//!    HKCU\Software\Classes\AppUserModelId\{AUMID}
//!      DisplayName = "My App"
//!      IconUri     = "C:\path\to\icon.ico"   (optional)
//!    ```
//!
//! Both entries live in `HKCU` — no elevation required.
//!
//! `install()` is called from `lib.rs` setup on every launch. Idempotent.
//! `uninstall()` should be called from the app's uninstaller.
//!
//! # References
//! - <https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps>
//! - <https://learn.microsoft.com/en-us/windows/win32/com/localserver32>

/// Configuration for registry installation.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    /// COM server GUID in `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` format.
    pub com_server_guid: String,
    /// App User Model ID — typically `CompanyName.AppName`.
    pub aumid: String,
    /// Human-readable display name shown in Windows Notification settings.
    pub display_name: String,
    /// Optional path to the app icon (`.ico` or `.png`).
    pub icon_path: Option<String>,
    /// Path to the executable launched for background activation.
    /// If `None`, defaults to the current process executable.
    pub exe_path: Option<String>,
}

/// Install all registry entries required for toast notifications.
///
/// Idempotent — safe to call on every app launch. Missing keys are created;
/// existing correct values are left unchanged.
#[cfg(windows)]
pub fn install(config: &RegistryConfig) -> crate::Result<()> {
    let exe = resolve_exe(config)?;
    install_com_server(&config.com_server_guid, &exe)?;
    install_aumid(config)?;
    log::info!(
        "[notification] registry installed — AUMID={} COM={}",
        config.aumid,
        config.com_server_guid
    );
    Ok(())
}

/// Remove all registry entries written by `install()`.
///
/// Call from the app uninstaller. Silently succeeds if the keys are absent.
#[cfg(windows)]
pub fn uninstall(config: &RegistryConfig) -> crate::Result<()> {
    uninstall_key(&format!(
        "Software\\Classes\\CLSID\\{}",
        config.com_server_guid
    ))?;
    uninstall_key(&format!(
        "Software\\Classes\\AppUserModelId\\{}",
        config.aumid
    ))?;
    log::info!(
        "[notification] registry uninstalled — AUMID={} COM={}",
        config.aumid,
        config.com_server_guid
    );
    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────

#[cfg(windows)]
fn resolve_exe(config: &RegistryConfig) -> crate::Result<String> {
    match &config.exe_path {
        Some(p) => Ok(p.clone()),
        None => {
            let exe = std::env::current_exe()
                .map_err(|e| crate::Error::Windows(format!("current_exe: {e}")))?;

            let exe = exe
                .canonicalize()
                .map_err(|e| crate::Error::Windows(format!("canonicalize: {e}")))?;

            Ok(exe.to_string_lossy().into_owned())
        }
    }
}

#[cfg(windows)]
fn install_com_server(guid: &str, exe_path: &str) -> crate::Result<()> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{
                RegCloseKey, RegCreateKeyExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
                REG_CREATE_KEY_DISPOSITION, REG_OPTION_NON_VOLATILE,
            },
        },
    };

    println!("Software\\Classes\\CLSID\\{}\\LocalServer32", guid);
    let key_path = HSTRING::from(format!("Software\\Classes\\CLSID\\{}\\LocalServer32", guid));

    let mut hkey = HKEY::default();

    let mut disposition = REG_CREATE_KEY_DISPOSITION(0);

    unsafe {
        use crate::trace_event;

        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &key_path,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            Some(&mut disposition),
        );

        if status != ERROR_SUCCESS {
            return Err(crate::Error::Windows(format!(
                "RegCreateKeyExW failed: {status:?}"
            )));
        }

        trace_event!(&format!("Resolved exe path: {:?}", std::env::current_exe()));
        trace_event!(&format!("Using COM server exe path: {:?}", exe_path));
        trace_event!(&format!(
            "Setting registry value: {:?} = {:?}",
            key_path, exe_path
        ));

        set_reg_sz(&hkey, "", exe_path)?;

        let close_status = RegCloseKey(hkey);

        if close_status != ERROR_SUCCESS {
            return Err(crate::Error::Windows(format!(
                "RegCloseKey failed: {close_status:?}"
            )));
        }
    }

    Ok(())
}

#[cfg(windows)]
fn install_aumid(config: &RegistryConfig) -> crate::Result<()> {
    use windows::{
        core::HSTRING,
        Win32::System::Registry::{
            RegCreateKeyExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_CREATE_KEY_DISPOSITION,
            REG_OPTION_NON_VOLATILE,
        },
    };

    let key_path = HSTRING::from(format!(
        "Software\\Classes\\AppUserModelId\\{}",
        config.aumid
    ));
    let mut hkey = HKEY::default();
    let mut disposition = REG_CREATE_KEY_DISPOSITION(0);

    unsafe {
        use windows::Win32::{Foundation::ERROR_SUCCESS, System::Registry::RegCloseKey};

        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &key_path,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            Some(&mut disposition),
        );

        if status != ERROR_SUCCESS {
            return Err(crate::Error::Windows(format!(
                "RegCreateKeyExW failed: {status:?}"
            )));
        }

        set_reg_sz(&hkey, "DisplayName", &config.display_name)?;

        if let Some(ref icon) = config.icon_path {
            set_reg_sz(&hkey, "IconUri", icon)?;
        }

        let close_status = RegCloseKey(hkey);

        if close_status != ERROR_SUCCESS {
            return Err(crate::Error::Windows(format!(
                "RegCloseKey failed: {close_status:?}"
            )));
        }
    }

    Ok(())
}

#[cfg(windows)]
pub fn write_custom_activator(aumid: &str, guid: &str) {
    // Normalise: ensure braces are present

    // use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    use winreg::{enums::*, RegKey};
    let guid_with_braces = if guid.starts_with('{') {
        guid.to_string()
    } else {
        format!("{{{guid}}}")
    };

    let key_path = format!("Software\\Classes\\AppUserModelId\\{aumid}");

    match RegKey::predef(HKEY_CURRENT_USER).create_subkey(&key_path) {
        Ok((key, _disposition)) => match key.set_value("CustomActivator", &guid_with_braces) {
            Ok(_) => {
                log::info!(
                    "[notification] CustomActivator → HKCU\\{}  =  {}",
                    key_path,
                    guid_with_braces
                );
            }
            Err(e) => {
                log::error!(
                    "[notification] failed to write CustomActivator to HKCU\\{}: {}",
                    key_path,
                    e
                );
            }
        },
        Err(e) => {
            log::error!(
                "[notification] failed to open/create HKCU\\{}: {}",
                key_path,
                e
            );
        }
    }
}

/// Delete an HKCU key and all its subkeys. Silently succeeds if absent.
#[cfg(windows)]
fn uninstall_key(subkey: &str) -> crate::Result<()> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
            System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER},
        },
    };

    unsafe {
        let status = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(subkey));

        // Success → return
        if status == ERROR_SUCCESS {
            return Ok(());
        }

        // Key missing → allowed
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }

        #[warn(clippy::needless_return)]
        // Real failure
        return Err(crate::Error::Windows(format!(
            "RegDeleteTreeW({subkey}) failed: {status:?}"
        )));
    }
}

/// Write a `REG_SZ` value (UTF-16 string) to an open registry key.
#[cfg(windows)]
unsafe fn set_reg_sz(
    hkey: &windows::Win32::System::Registry::HKEY,
    value_name: &str,
    data: &str,
) -> crate::Result<()> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{RegSetValueExW, REG_SZ},
        },
    };

    let name = HSTRING::from(value_name);

    // UTF-16 + null terminator
    let mut wide: Vec<u16> = data.encode_utf16().collect();
    wide.push(0);

    let bytes = std::slice::from_raw_parts(
        wide.as_ptr() as *const u8,
        wide.len() * std::mem::size_of::<u16>(),
    );

    let status = RegSetValueExW(*hkey, &name, Some(0), REG_SZ, Some(bytes));

    if status != ERROR_SUCCESS {
        return Err(crate::Error::Windows(format!(
            "RegSetValueExW({value_name}) failed: {status:?}"
        )));
    }

    Ok(())
}

// ── No-op stubs for non-Windows ──────────────────────────────────────────

#[cfg(not(windows))]
pub fn install(_config: &RegistryConfig) -> crate::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn uninstall(_config: &RegistryConfig) -> crate::Result<()> {
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RegistryConfig {
        RegistryConfig {
            com_server_guid: "{A1B2C3D4-E5F6-7890-ABCD-EF1234567890}".into(),
            aumid: "TauriTest.NotifPlugin".into(),
            display_name: "Tauri Notification Test".into(),
            icon_path: None,
            exe_path: Some("C:\\test\\app.exe".into()),
        }
    }

    #[test]
    fn config_fields_accessible() {
        let c = test_config();
        assert_eq!(c.aumid, "TauriTest.NotifPlugin");
        assert_eq!(c.display_name, "Tauri Notification Test");
        assert!(c.icon_path.is_none());
    }

    #[test]
    fn config_with_icon_path() {
        let c = RegistryConfig {
            icon_path: Some("C:\\app\\icon.ico".into()),
            ..test_config()
        };
        assert_eq!(c.icon_path.as_deref(), Some("C:\\app\\icon.ico"));
    }

    // Windows-only integration tests that actually write/delete registry keys.
    #[cfg(windows)]
    mod registry_roundtrip {
        use super::*;

        #[test]
        fn install_and_uninstall_com_server() {
            let guid = "{FFFFFFFF-0000-0000-0000-000000000001}";
            install_com_server(guid, "C:\\test\\roundtrip.exe").expect("install_com_server failed");
            uninstall_key(&format!("Software\\Classes\\CLSID\\{}", guid))
                .expect("uninstall_key failed");
            // Second uninstall must not error (key already gone)
            uninstall_key(&format!("Software\\Classes\\CLSID\\{}", guid))
                .expect("second uninstall should be no-op");
        }

        #[test]
        fn install_and_uninstall_aumid() {
            let config = RegistryConfig {
                aumid: "TauriTest.NotifPlugin.RoundTrip1".into(),
                ..test_config()
            };
            install_aumid(&config).expect("install_aumid failed");
            uninstall_key(&format!(
                "Software\\Classes\\AppUserModelId\\{}",
                config.aumid
            ))
            .expect("uninstall_key failed");
        }

        #[test]
        fn full_install_uninstall() {
            let config = RegistryConfig {
                com_server_guid: "{FFFFFFFF-0000-0000-0000-000000000002}".into(),
                aumid: "TauriTest.NotifPlugin.RoundTrip2".into(),
                ..test_config()
            };
            install(&config).expect("install failed");
            uninstall(&config).expect("uninstall failed");
        }
    }
}
