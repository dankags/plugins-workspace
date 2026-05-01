// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use tauri::{command, plugin::PermissionState, AppHandle, Runtime, State};

#[cfg(windows)]
use crate::windows_platform::registry_installer::RegistryConfig;
use crate::{Notification, NotificationData, Result};

#[command]
pub(crate) async fn is_permission_granted<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
) -> Result<Option<bool>> {
    let state = notification.permission_state()?;
    match state {
        PermissionState::Granted => Ok(Some(true)),
        PermissionState::Denied => Ok(Some(false)),
        PermissionState::Prompt | PermissionState::PromptWithRationale => Ok(None),
    }
}

#[command]
pub(crate) async fn request_permission<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
) -> Result<PermissionState> {
    notification.request_permission()
}

#[command]
pub(crate) async fn notify<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
    options: NotificationData,
) -> Result<()> {
    let mut builder = notification.builder();
    builder.data = options;
    builder.show()
}

// ── new: Windows notification management ─────────────────────────────────────

#[command]
pub(crate) async fn clear_notification<R: Runtime>(
    app: AppHandle<R>,
    tag: String,
    group: String,
) -> Result<()> {
    #[cfg(windows)]
    {
        let identifier = app.config().identifier.clone();
        crate::windows_platform::notification_listener::remove_notification(
            &identifier,
            &tag,
            &group,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (app, tag, group);
        Ok(())
    }
}

#[command]
pub(crate) async fn clear_notification_group<R: Runtime>(
    app: AppHandle<R>,
    group: String,
) -> Result<()> {
    #[cfg(windows)]
    {
        let identifier = app.config().identifier.clone();
        crate::windows_platform::notification_listener::remove_notification_group(
            &identifier,
            &group,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (app, group);
        Ok(())
    }
}

#[command]
pub(crate) async fn clear_all_notifications<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    #[cfg(windows)]
    {
        let identifier = app.config().identifier.clone();
        crate::windows_platform::notification_listener::remove_all_notifications(&identifier)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Ok(())
    }
}

// ── new: Notification Listener ────────────────────────────────────────────────

#[command]
pub(crate) async fn request_listener_access<R: Runtime>(
    _app: AppHandle<R>,
) -> Result<crate::models::ListenerAccessStatus> {
    #[cfg(windows)]
    {
        crate::windows_platform::notification_listener::request_access().await
    }
    #[cfg(not(windows))]
    {
        Ok(crate::models::ListenerAccessStatus::NotSupported)
    }
}

#[command]
pub(crate) async fn get_listener_access_status<R: Runtime>(
    _app: AppHandle<R>,
) -> Result<crate::models::ListenerAccessStatus> {
    #[cfg(windows)]
    {
        crate::windows_platform::notification_listener::get_access_status()
    }
    #[cfg(not(windows))]
    {
        Ok(crate::models::ListenerAccessStatus::NotSupported)
    }
}

#[command]
pub(crate) async fn get_active_notifications<R: Runtime>(
    _app: AppHandle<R>,
) -> Result<Vec<crate::models::WinActiveNotification>> {
    #[cfg(windows)]
    {
        crate::windows_platform::notification_listener::get_all_notifications().await
    }
    #[cfg(not(windows))]
    {
        Ok(vec![])
    }
}

// ── new: Windows uninstall / cleanup ─────────────────────────────────────────

/// Remove the registry entries (COM server + AUMID) written during install.
///
/// Call this from your app's uninstaller — not on normal exit.
/// Requires `comServerGuid` to be present in the plugin config.
#[tauri::command]
pub async fn uninstall_notification_registration<R: Runtime>(
    app: AppHandle<R>,
) -> std::result::Result<(), String> {
    #[cfg(windows)]
    {
        use tauri::Manager;

        let config = app.state::<crate::PluginConfig>().inner().clone();

        let aumid = app.config().identifier.clone();

        let guid_str = match config.com_server_guid {
            Some(ref g) => g.clone(),
            None => {
                log::warn!("[notification] uninstall called but no comServerGuid configured");
                return Ok(());
            }
        };

        crate::windows_platform::registry_installer::uninstall(&aumid, &guid_str)
            .map_err(|e| e.to_string())?;

        log::info!(
            "[notification] registry keys removed for aumid={} guid={}",
            aumid,
            guid_str
        );
    }

    Ok(())
}

/// Remove the Start Menu shortcut for this app.
///
/// ```typescript
/// await invoke('plugin:notification|remove_notification_shortcut');
/// ```
#[tauri::command]
pub async fn remove_notification_shortcut<R: Runtime>(
    app: AppHandle<R>,
) -> std::result::Result<(), String> {
    #[cfg(windows)]
    {
        let display_name = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| app.config().identifier.clone());

        crate::windows_platform::shortcut_creator::remove(&display_name)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn uninstall(aumid: String, guid: String) -> crate::Result<()> {
    uninstall_key(&format!("Software\\Classes\\CLSID\\{}", guid))?;
    uninstall_key(&format!("Software\\Classes\\AppUserModelId\\{}", aumid))?;
    log::info!(
        "[notification] registry uninstalled — AUMID={} COM={}",
        aumid,
        guid
    );
    Ok(())
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
