// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Notification Listener API — read and monitor all notifications in the
//! Windows Action Center, including those from other apps.
//!
//! # Availability
//!
//! | Scenario                   | Available? |
//! |----------------------------|-----------|
//! | MSIX-packaged app, Win10+  | ✅ Yes    |
//! | NSIS-installed app, Win10+ | ❌ No — `RequestAccessAsync` returns `Denied` |
//! | Dev mode (cargo run)       | ❌ No     |
//! | Windows 7 / 8 / 8.1       | ❌ No     |

use super::version::WindowsVersion;
use crate::models::{ListenerAccessStatus, WinActiveNotification};

#[cfg(windows)]
use windows::{
    core::Interface,
    UI::Notifications::Management::{
        UserNotificationListener, UserNotificationListenerAccessStatus,
    },
    UI::Notifications::NotificationKinds,
    UI::Notifications::UserNotification,
};

/// Request access to the Notification Listener.
pub async fn request_access() -> crate::Result<ListenerAccessStatus> {
    #[cfg(windows)]
    {
        if !WindowsVersion::current().has_notification_listener() {
            return Ok(ListenerAccessStatus::NotSupported);
        }
        if !is_packaged() {
            log::debug!(
                "[notification] Notification Listener requires a packaged (MSIX) app. \
                 Returning NotSupported."
            );
            return Ok(ListenerAccessStatus::NotSupported);
        }

        let listener = UserNotificationListener::Current()
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        // Break the chain so rustc can infer the async output type at each step.
        let status: UserNotificationListenerAccessStatus = listener
            .RequestAccessAsync()
            .map_err(|e| crate::Error::Windows(e.to_string()))?
            .await
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        Ok(winrt_status_to_model(status))
    }
    #[cfg(not(windows))]
    {
        Ok(ListenerAccessStatus::NotSupported)
    }
}

/// Query the current Notification Listener access status without prompting.
pub fn get_access_status() -> crate::Result<ListenerAccessStatus> {
    #[cfg(windows)]
    {
        if !WindowsVersion::current().has_notification_listener() {
            return Ok(ListenerAccessStatus::NotSupported);
        }
        if !is_packaged() {
            return Ok(ListenerAccessStatus::NotSupported);
        }

        let listener = UserNotificationListener::Current()
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        let status: UserNotificationListenerAccessStatus = listener
            .GetAccessStatus()
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        Ok(winrt_status_to_model(status))
    }
    #[cfg(not(windows))]
    {
        Ok(ListenerAccessStatus::NotSupported)
    }
}

/// Return all toast notifications currently in the Action Center.
pub async fn get_all_notifications() -> crate::Result<Vec<WinActiveNotification>> {
    #[cfg(windows)]
    {
        if !WindowsVersion::current().has_notification_listener() {
            return Ok(vec![]);
        }
        if !is_packaged() {
            return Ok(vec![]);
        }

        let listener = UserNotificationListener::Current()
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        let status: UserNotificationListenerAccessStatus = listener
            .GetAccessStatus()
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        if status != UserNotificationListenerAccessStatus::Allowed {
            return Ok(vec![]);
        }

        // GetNotificationsAsync returns IAsyncOperation<IVectorView<UserNotification>>
        let notifications = listener
            .GetNotificationsAsync(NotificationKinds::Toast)
            .map_err(|e| crate::Error::Windows(e.to_string()))?
            .await
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        let count = notifications
            .Size()
            .map_err(|e| crate::Error::Windows(e.to_string()))?;

        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            // IVectorView<UserNotification>::GetAt — explicit type avoids inference failure
            let user_notif: UserNotification = notifications
                .GetAt(i)
                .map_err(|e| crate::Error::Windows(e.to_string()))?;

            let id = user_notif
                .Id()
                .map_err(|e| crate::Error::Windows(e.to_string()))?;

            let app_info: Option<windows::ApplicationModel::AppInfo> = user_notif
                .AppInfo()
                .ok()
                .and_then(|a| a.cast::<windows::ApplicationModel::AppInfo>().ok());
            let app_id = app_info
                .as_ref()
                .and_then(|a| a.AppUserModelId().ok())
                .map(|h| h.to_string());

            // Notification() returns the base Notification type; cast to
            // ToastNotification to access Tag, Group, and Content XML.
            let base_notif = user_notif
                .Notification()
                .map_err(|e| crate::Error::Windows(e.to_string()))?;

            let toast_notif = base_notif
                .cast::<windows::UI::Notifications::ToastNotification>()
                .ok();

            let tag = toast_notif
                .as_ref()
                .and_then(|n| n.Tag().ok())
                .map(|h| h.to_string())
                .filter(|s| !s.is_empty());

            let group = toast_notif
                .as_ref()
                .and_then(|n| n.Group().ok())
                .map(|h| h.to_string())
                .filter(|s| !s.is_empty());

            let (title, body) = toast_notif
                .as_ref()
                .map(extract_text_from_notification)
                .unwrap_or((None, None));

            result.push(WinActiveNotification {
                id,
                tag,
                group,
                title,
                body,
                app_id,
            });
        }

        Ok(result)
    }
    #[cfg(not(windows))]
    {
        Ok(vec![])
    }
}

/// Remove a notification by tag + group from the Action Center.
pub fn remove_notification(app_id: &str, tag: &str, group: &str) -> crate::Result<()> {
    #[cfg(windows)]
    {
        use windows::{core::HSTRING, UI::Notifications::ToastNotificationManager};

        if !WindowsVersion::current().has_tag_group() {
            return Ok(());
        }

        ToastNotificationManager::History()
            .map_err(|e| crate::Error::Windows(e.to_string()))?
            .RemoveGroupedTagWithId(
                &HSTRING::from(tag),
                &HSTRING::from(group),
                &HSTRING::from(app_id),
            )
            .map_err(|e| crate::Error::Windows(e.to_string()))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app_id, tag, group);
        Ok(())
    }
}

/// Remove all notifications in a group from the Action Center.
pub fn remove_notification_group(app_id: &str, group: &str) -> crate::Result<()> {
    #[cfg(windows)]
    {
        use windows::{core::HSTRING, UI::Notifications::ToastNotificationManager};

        if !WindowsVersion::current().has_tag_group() {
            return Ok(());
        }

        ToastNotificationManager::History()
            .map_err(|e| crate::Error::Windows(e.to_string()))?
            .RemoveGroupWithId(&HSTRING::from(group), &HSTRING::from(app_id))
            .map_err(|e| crate::Error::Windows(e.to_string()))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app_id, group);
        Ok(())
    }
}

/// Remove ALL notifications for this app from the Action Center.
pub fn remove_all_notifications(app_id: &str) -> crate::Result<()> {
    #[cfg(windows)]
    {
        use windows::{core::HSTRING, UI::Notifications::ToastNotificationManager};

        ToastNotificationManager::History()
            .map_err(|e| crate::Error::Windows(e.to_string()))?
            .ClearWithId(&HSTRING::from(app_id))
            .map_err(|e| crate::Error::Windows(e.to_string()))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = app_id;
        Ok(())
    }
}

// ── Private helpers ───────────────────────────────────────────────────────

/// Detect whether the current process has a package identity (MSIX).
#[cfg(windows)]
fn is_packaged() -> bool {
    use windows::core::PWSTR;
    use windows::Win32::Storage::Packaging::Appx::{
        GetCurrentPackageFullName, PACKAGE_FULL_NAME_MAX_LENGTH,
    };
    let mut len = PACKAGE_FULL_NAME_MAX_LENGTH;
    let mut buf = vec![0u16; len as usize];
    unsafe { GetCurrentPackageFullName(&mut len, Some(PWSTR(buf.as_mut_ptr()))) }.is_ok()
}

#[cfg(windows)]
fn winrt_status_to_model(s: UserNotificationListenerAccessStatus) -> ListenerAccessStatus {
    match s {
        UserNotificationListenerAccessStatus::Allowed => ListenerAccessStatus::Allowed,
        UserNotificationListenerAccessStatus::Denied => ListenerAccessStatus::Denied,
        UserNotificationListenerAccessStatus::Unspecified => ListenerAccessStatus::Unspecified,
        _ => ListenerAccessStatus::NotSupported,
    }
}

/// Extract title and body text from a ToastNotification's XML content.
#[cfg(windows)]
fn extract_text_from_notification(
    notif: &windows::UI::Notifications::ToastNotification,
) -> (Option<String>, Option<String>) {
    let xml = match notif.Content() {
        Ok(x) => x,
        Err(_) => return (None, None),
    };
    let nodes = match xml.GetElementsByTagName(&windows::core::HSTRING::from("text")) {
        Ok(n) => n,
        Err(_) => return (None, None),
    };
    let get = |i: u32| -> Option<String> {
        nodes
            .Item(i)
            .ok()
            .and_then(|n| n.InnerText().ok())
            .map(|h| h.to_string())
            .filter(|s| !s.is_empty())
    };
    (get(0), get(1))
}
