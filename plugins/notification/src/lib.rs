// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Send message notifications (brief auto-expiring OS window element) to your user. Can also be used with the Notification Web API.

#![doc(
    html_logo_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png",
    html_favicon_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png"
)]

use serde::Serialize;
#[cfg(mobile)]
use tauri::plugin::PluginHandle;
#[cfg(desktop)]
use tauri::AppHandle;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use models::*;
pub use tauri::plugin::PermissionState;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

// new: Windows-specific modules
#[cfg(windows)]
pub(crate) mod windows;

pub use error::{Error, Result};

#[cfg(desktop)]
pub use desktop::Notification;
#[cfg(mobile)]
pub use mobile::Notification;

/// The notification builder.
#[derive(Debug)]
pub struct NotificationBuilder<R: Runtime> {
    #[cfg(desktop)]
    app: AppHandle<R>,
    #[cfg(mobile)]
    handle: PluginHandle<R>,
    pub(crate) data: NotificationData,
}

impl<R: Runtime> NotificationBuilder<R> {
    #[cfg(desktop)]
    fn new(app: AppHandle<R>) -> Self {
        Self {
            app,
            data: Default::default(),
        }
    }

    #[cfg(mobile)]
    fn new(handle: PluginHandle<R>) -> Self {
        Self {
            handle,
            data: Default::default(),
        }
    }

    /// Sets the notification identifier.
    pub fn id(mut self, id: i32) -> Self {
        self.data.id = id;
        self
    }

    /// Identifier of the {@link Channel} that deliveres this notification.
    ///
    /// If the channel does not exist, the notification won't fire.
    /// Make sure the channel exists with {@link listChannels} and {@link createChannel}.
    pub fn channel_id(mut self, id: impl Into<String>) -> Self {
        self.data.channel_id.replace(id.into());
        self
    }

    /// Sets the notification title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.data.title.replace(title.into());
        self
    }

    /// Sets the notification body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.data.body.replace(body.into());
        self
    }

    /// Schedule this notification to fire on a later time or a fixed interval.
    pub fn schedule(mut self, schedule: Schedule) -> Self {
        self.data.schedule.replace(schedule);
        self
    }

    /// Multiline text.
    /// Changes the notification style to big text.
    /// Cannot be used with `inboxLines`.
    pub fn large_body(mut self, large_body: impl Into<String>) -> Self {
        self.data.large_body.replace(large_body.into());
        self
    }

    /// Detail text for the notification with `largeBody`, `inboxLines` or `groupSummary`.
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.data.summary.replace(summary.into());
        self
    }

    /// Defines an action type for this notification.
    pub fn action_type_id(mut self, action_type_id: impl Into<String>) -> Self {
        self.data.action_type_id.replace(action_type_id.into());
        self
    }

    /// Identifier used to group multiple notifications.
    ///
    /// <https://developer.apple.com/documentation/usernotifications/unmutablenotificationcontent/1649872-threadidentifier>
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.data.group.replace(group.into());
        self
    }

    /// Instructs the system that this notification is the summary of a group on Android.
    pub fn group_summary(mut self) -> Self {
        self.data.group_summary = true;
        self
    }

    /// The sound resource name for the notification.
    pub fn sound(mut self, sound: impl Into<String>) -> Self {
        self.data.sound.replace(sound.into());
        self
    }

    /// Append an inbox line to the notification.
    /// Changes the notification style to inbox.
    /// Cannot be used with `largeBody`.
    ///
    /// Only supports up to 5 lines.
    pub fn inbox_line(mut self, line: impl Into<String>) -> Self {
        self.data.inbox_lines.push(line.into());
        self
    }

    /// Notification icon.
    ///
    /// On Android the icon must be placed in the app's `res/drawable` folder.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.data.icon.replace(icon.into());
        self
    }

    /// Notification large icon (Android).
    ///
    /// The icon must be placed in the app's `res/drawable` folder.
    pub fn large_icon(mut self, large_icon: impl Into<String>) -> Self {
        self.data.large_icon.replace(large_icon.into());
        self
    }

    /// Icon color on Android.
    pub fn icon_color(mut self, icon_color: impl Into<String>) -> Self {
        self.data.icon_color.replace(icon_color.into());
        self
    }

    /// Append an attachment to the notification.
    pub fn attachment(mut self, attachment: Attachment) -> Self {
        self.data.attachments.push(attachment);
        self
    }

    /// Adds an extra payload to store in the notification.
    pub fn extra(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        self.data
            .extra
            .insert(key.into(), serde_json::to_value(value).unwrap());
        self
    }

    /// If true, the notification cannot be dismissed by the user on Android.
    ///
    /// An application service must manage the dismissal of the notification.
    /// It is typically used to indicate a background task that is pending (e.g. a file download)
    /// or the user is engaged with (e.g. playing music).
    pub fn ongoing(mut self) -> Self {
        self.data.ongoing = true;
        self
    }

    /// Automatically cancel the notification when the user clicks on it.
    pub fn auto_cancel(mut self) -> Self {
        self.data.auto_cancel = true;
        self
    }

    /// Changes the notification presentation to be silent on iOS (no badge, no sound, not listed).
    pub fn silent(mut self) -> Self {
        self.data.silent = true;
        self
    }

    // new: Windows builder methods

    /// Notification tag for WinRT history API (Windows 10+ only).
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.data.tag.replace(tag.into());
        self
    }

    /// Hero image at top of toast (Windows 10+ only).
    pub fn hero_image(mut self, src: impl Into<String>) -> Self {
        self.data.hero_image.replace(src.into());
        self
    }

    /// Add an interactive action button (Windows 10+ only).
    pub fn windows_action(mut self, action: WindowsAction) -> Self {
        self.data.windows_actions.push(action);
        self
    }

    /// Add a text or selection input (Windows 10+ only).
    pub fn windows_input(mut self, input: WindowsInput) -> Self {
        self.data.windows_inputs.push(input);
        self
    }

    /// Attach a progress bar (Windows 10 build 19041+ only).
    pub fn progress(mut self, progress: WindowsProgress) -> Self {
        self.data.progress.replace(progress);
        self
    }

    /// Auto-remove after N milliseconds (Windows 10+ only).
    pub fn expiry_ms(mut self, ms: u64) -> Self {
        self.data.expiry_ms = Some(ms);
        self
    }

    /// Set notification scenario (Windows 10+ only).
    pub fn scenario(mut self, scenario: WindowsScenario) -> Self {
        self.data.scenario.replace(scenario);
        self
    }

    /// Set delivery priority (Windows 10+ only).
    pub fn priority(mut self, priority: WindowsPriority) -> Self {
        self.data.priority.replace(priority);
        self
    }

    /// Remove from Action Center on reboot (Windows 11+ only).
    pub fn expires_on_reboot(mut self) -> Self {
        self.data.expires_on_reboot = true;
        self
    }

    /// Enable background COM activation (Windows 8+ only).
    pub fn background_activation(mut self) -> Self {
        self.data.background_activation = true;
        self
    }
}

/// Extensions to [`tauri::App`], [`tauri::AppHandle`], [`tauri::WebviewWindow`], [`tauri::Webview`] and [`tauri::Window`] to access the notification APIs.
pub trait NotificationExt<R: Runtime> {
    fn notification(&self) -> &Notification<R>;
}

impl<R: Runtime, T: Manager<R>> crate::NotificationExt<R> for T {
    fn notification(&self) -> &Notification<R> {
        self.state::<Notification<R>>().inner()
    }
}

/// Initializes the plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R, PluginConfig> {
    Builder::<R, PluginConfig>::new("notification")
        .invoke_handler(tauri::generate_handler![
            commands::notify,
            commands::request_permission,
            commands::is_permission_granted,
            // new: notification management
            commands::clear_notification,
            commands::clear_notification_group,
            commands::clear_all_notifications,
            // new: Notification Listener
            commands::request_listener_access,
            commands::get_listener_access_status,
            commands::get_active_notifications,
        ])
        .js_init_script(include_str!("init-iife.js").replace(
            "__TEMPLATE_windows__",
            if cfg!(windows) { "true" } else { "false" },
        ))
        .setup(|app, api| {
            // new: read plugin config and register COM activator on Windows
            let config: PluginConfig = api.config().clone();
            #[cfg(windows)]
            if let Some(ref guid) = config.com_server_guid {
                if let Err(e) = crate::windows::com_activator::register(guid) {
                    log::warn!("[notification] COM activator registration failed: {e}");
                }
            }

            #[cfg(mobile)]
            let notification = mobile::init(app, api)?;
            #[cfg(desktop)]
            let notification = desktop::init(app, api)?;
            app.manage(notification);
            Ok(())
        })
        .on_event(|_app, event| {
            // Unregister the COM activator when the app exits to release
            // the class object registration cleanly.
            if let tauri::RunEvent::Exit = event {
                #[cfg(windows)]
                crate::windows::com_activator::unregister();
            }
        })
        .build()
}
