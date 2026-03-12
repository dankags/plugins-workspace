// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use tauri::{command, plugin::PermissionState, AppHandle, Runtime, State};

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
        crate::windows::notification_listener::remove_notification(&identifier, &tag, &group)
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
        crate::windows::notification_listener::remove_notification_group(&identifier, &group)
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
        crate::windows::notification_listener::remove_all_notifications(&identifier)
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
        crate::windows::notification_listener::request_access().await
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
        crate::windows::notification_listener::get_access_status()
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
        crate::windows::notification_listener::get_all_notifications().await
    }
    #[cfg(not(windows))]
    {
        Ok(vec![])
    }
}
