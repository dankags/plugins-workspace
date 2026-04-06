// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Windows Start Menu shortcut creator.
//!
//! Unpackaged apps on Windows 8+ must have a Start Menu shortcut with an
//! embedded **App User Model ID (AUMID)** and optionally a **COM activator
//! CLSID** (`ToastActivatorCLSID`) for the toast notification system to:
//!
//! - Route toast activations back to the correct app
//! - Show the app in Windows Notification settings
//! - Enable background COM activation for action buttons
//!
//! The shortcut is written to:
//! `%APPDATA%\Microsoft\Windows\Start Menu\Programs\<shortcut_name>.lnk`
//!
//! `create_or_update()` is idempotent — it is safe to call on every launch.
//! `remove()` should be called from the app uninstaller.
//!
//! # References
//! - <https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps#step-1-enable-notifications-for-your-app>
//! - Shell property keys: `PKEY_AppUserModel_ID` (pid 5) and
//!   `PKEY_AppUserModel_ToastActivatorCLSID` (pid 26)
//!

#[cfg(windows)]
use windows::Win32::{
    Foundation::PROPERTYKEY, System::Com::StructuredStorage::PropVariantClear,
    UI::Shell::PropertiesSystem::IPropertyStore,
};
use windows_core::GUID;
#[cfg(windows)]
use windows_core::HSTRING;

/// Configuration for shortcut creation.
#[derive(Debug, Clone)]
pub struct ShortcutConfig {
    /// Filename for the `.lnk` file, without extension.
    /// E.g. `"My App"` → `…\Start Menu\Programs\My App.lnk`
    pub shortcut_name: String,
    /// App User Model ID to embed in the shortcut.
    pub aumid: String,
    /// Optional COM activator GUID for background toast activation.
    /// Must match `RegistryConfig::com_server_guid`.
    pub com_server_guid: Option<String>,
    /// Target executable path. Defaults to the current process if `None`.
    pub exe_path: Option<String>,
}

// const VT_LPWSTR: u16 = 31;

/// Create or update the Start Menu shortcut with the embedded AUMID.
///
/// Idempotent — safe to call on every app launch.
#[cfg(windows)]
pub fn create_or_update(config: &ShortcutConfig) -> crate::Result<()> {
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        Win32::UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
    };

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let exe = resolve_exe(config)?;
    let lnk_path = shortcut_path(&config.shortcut_name)?;

    // Ensure the Programs directory exists
    if let Some(parent) = lnk_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::Error::Windows(format!("create Programs dir: {e}")))?;
    }

    unsafe {
        use windows_core::Interface;

        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| crate::Error::Windows(format!("CoCreateInstance IShellLink: {e}")))?;

        shell_link
            .SetPath(&HSTRING::from(exe.as_str()))
            .map_err(|e| crate::Error::Windows(format!("IShellLink::SetPath: {e}")))?;

        // Embed AUMID via IPropertyStore
        let prop_store: IPropertyStore = shell_link
            .cast()
            .map_err(|e| crate::Error::Windows(format!("cast to IPropertyStore: {e}")))?;

        // PKEY_AppUserModel_ID — fmtid {9F4C2855-...}, pid 5
        set_str_property(
            &prop_store,
            windows::core::GUID {
                data1: 0x9F4C2855,
                data2: 0x9F79,
                data3: 0x4B39,
                data4: [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3],
            },
            5,
            &config.aumid,
        )
        .map_err(|e| crate::Error::Windows(format!("set AUMID property: {e}")))?;

        // PKEY_AppUserModel_ToastActivatorCLSID — same fmtid, pid 26
        if let Some(ref guid_str) = config.com_server_guid {
            let guid = super::com_activator::parse_guid(guid_str)
                .map_err(|e| crate::Error::Windows(format!("parse COM GUID for shortcut: {e}")))?;

            set_guid_property(
                &prop_store,
                windows::core::GUID {
                    data1: 0x9F4C2855,
                    data2: 0x9F79,
                    data3: 0x4B39,
                    data4: [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3],
                },
                26,
                &guid,
            )
            .map_err(|e| crate::Error::Windows(format!("set ToastActivatorCLSID property: {e}")))?;
        }

        prop_store
            .Commit()
            .map_err(|e| crate::Error::Windows(format!("IPropertyStore::Commit: {e}")))?;

        // Save the .lnk file via IPersistFile
        let persist: IPersistFile = shell_link
            .cast()
            .map_err(|e| crate::Error::Windows(format!("cast to IPersistFile: {e}")))?;

        let lnk_wide: Vec<u16> = lnk_path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .collect();

        persist
            .Save(PCWSTR(lnk_wide.as_ptr()), true)
            .map_err(|e| crate::Error::Windows(format!("IPersistFile::Save: {e}")))?;
    }

    log::info!("[notification] shortcut created: {}", lnk_path.display());

    Ok(())
}

/// Delete the Start Menu shortcut. Silently succeeds if absent.
#[cfg(windows)]
pub fn remove(shortcut_name: &str) -> crate::Result<()> {
    let path = shortcut_path(shortcut_name)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| crate::Error::Windows(format!("remove shortcut: {e}")))?;
        log::info!("[notification] shortcut removed: {}", path.display());
    }
    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────

#[cfg(windows)]
fn resolve_exe(config: &ShortcutConfig) -> crate::Result<String> {
    match &config.exe_path {
        Some(p) => Ok(p.clone()),
        None => std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| crate::Error::Windows(format!("current_exe: {e}"))),
    }
}

/// Returns the full `.lnk` path under the user's Start Menu Programs folder.
#[cfg(windows)]
fn shortcut_path(name: &str) -> crate::Result<std::path::PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Programs, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

    let pwstr = unsafe {
        SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)
            .map_err(|e| crate::Error::Windows(format!("SHGetKnownFolderPath: {e}")))?
    };

    let dir = unsafe {
        let ptr = pwstr.0;
        let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
        std::slice::from_raw_parts(ptr, len)
    };

    let path: std::path::PathBuf = String::from_utf16_lossy(dir).into();
    Ok(path.join(format!("{}.lnk", name)))
}

/// Set a string property (AUMID) using the helper available in windows 0.62
#[cfg(windows)]
unsafe fn set_str_property(
    store: &IPropertyStore,
    fmtid: GUID,
    pid: u32,
    value: &str,
) -> crate::Result<()> {
    use crate::error::Error;
    use ::windows::Win32::System::Com::StructuredStorage::InitPropVariantFromStringAsVector;

    let key = PROPERTYKEY { fmtid, pid };

    let hstring = HSTRING::from(value);

    // This function does the proper initialization + allocation internally
    let mut pv = InitPropVariantFromStringAsVector(&hstring).map_err(Error::from)?;

    // InitPropVariantFromStringVector(windows_core::PWSTR(value)); // safe zero init

    let hr = store.SetValue(&key, &pv).map_err(Error::from);

    let _ = PropVariantClear(&mut pv); // Always clean up

    match &hr {
        Ok(_) => {
            log::debug!(
                "[notification] set string property pid {} to '{}'",
                pid,
                value
            );
        }
        Err(e) => {
            log::error!(
                "[notification] failed to set string property pid {}: {}",
                pid,
                e
            );
        }
    }

    hr
}

#[cfg(windows)]
fn set_guid_property(
    store: &IPropertyStore,
    fmtid: GUID,
    pid: u32,
    guid: &GUID,
) -> crate::Result<()> {
    use crate::error::Error;
    use windows::Win32::System::Com::StructuredStorage::{
        InitPropVariantFromCLSID, PropVariantClear,
    }; // your crate's error type

    let key = PROPERTYKEY { fmtid, pid };

    // Convert InitPropVariantFromCLSID errors into crate::Error
    unsafe {
        let mut pv = InitPropVariantFromCLSID(guid).map_err(Error::from)?;

        let set_result = store.SetValue(&key, &pv).map_err(Error::from);

        // Always clear the PROPVARIANT
        let _ = PropVariantClear(&mut pv);

        match &set_result {
            Ok(_) => {
                log::debug!("[notification] set GUID property pid {} to {:?}", pid, guid);
            }
            Err(e) => {
                log::error!(
                    "[notification] failed to set GUID property pid {}: {}",
                    pid,
                    e
                );
            }
        }

        set_result
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ShortcutConfig {
        ShortcutConfig {
            shortcut_name: "Tauri Notification Test".into(),
            aumid: "TauriTest.NotifPlugin".into(),
            com_server_guid: Some("{A1B2C3D4-E5F6-7890-ABCD-EF1234567890}".into()),
            exe_path: Some("C:\\test\\app.exe".into()),
        }
    }

    #[test]
    fn config_fields_accessible() {
        let c = test_config();
        assert_eq!(c.aumid, "TauriTest.NotifPlugin");
        assert!(c.com_server_guid.is_some());
    }

    #[test]
    fn config_without_guid() {
        let c = ShortcutConfig {
            com_server_guid: None,
            ..test_config()
        };
        assert!(c.com_server_guid.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn shortcut_path_ends_with_lnk_under_programs() {
        let p = shortcut_path("MyTestApp").unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("Programs"), "must be under Programs: {s}");
        assert!(s.ends_with("MyTestApp.lnk"), "must end with .lnk: {s}");
    }

    #[cfg(windows)]
    #[test]
    fn create_and_remove_round_trip() {
        // We use the ComGuard from your com_activator if available,
        // otherwise just be very careful with scoping:
        {
            use windows::Win32::System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
            };
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }

            let c = ShortcutConfig {
                shortcut_name: "TauriNotifPluginTest_ShortcutRoundTrip".into(),
                exe_path: Some(
                    std::env::current_exe()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                ),
                ..test_config()
            };

            // Run the logic
            let res = create_or_update(&c);
            assert!(res.is_ok(), "Shortcut creation failed: {:?}", res.err());

            let rem_res = remove(&c.shortcut_name);
            assert!(
                rem_res.is_ok(),
                "Shortcut removal failed: {:?}",
                rem_res.err()
            );

            unsafe {
                CoUninitialize();
            }
        }
    }
}
