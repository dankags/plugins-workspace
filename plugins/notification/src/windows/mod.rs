// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Windows-specific notification dispatch.
//!
//! Detects the runtime Windows version and routes to the appropriate tier:
//!
//! ```text
//! show() called
//!   └─ WindowsVersion::current()
//!        ├─ Win7          → tier::win7      (win7_notifications tray balloon)
//!        ├─ Win8          → tier::win8      (basic WinRT, legacy templates)
//!        ├─ Win8.1        → tier::win81     (basic WinRT, ToastGeneric)
//!        └─ Win10 / Win11 → tier::rich      (full adaptive toast XML, all features)
//! ```

pub mod action_handler;
pub mod com_activator;
pub mod notification_listener;
pub mod version;
pub mod xml_builder;

use crate::models::NotificationData;
use tauri::Runtime;
use version::WindowsVersion;

pub fn show<R: Runtime>(
    data: &NotificationData,
    identifier: &str,
    app: &tauri::AppHandle<R>,
) -> crate::Result<()> {
    let ver = WindowsVersion::current();
    log::debug!(
        "[notification] showing notification on {} (tag={:?})",
        ver.display_name(),
        data.tag
    );
    match ver {
        WindowsVersion::Win7 => tier::win7::show(data, app),
        WindowsVersion::Win8 => tier::win8::show(data, identifier),
        WindowsVersion::Win81 => tier::win81::show(data, identifier),
        WindowsVersion::Win10Pre19041 | WindowsVersion::Win10 | WindowsVersion::Win11 => {
            tier::rich::show(data, identifier, ver)
        }
    }
}

mod tier {

    // ── Tier 1: Windows 7 ─────────────────────────────────────────────────
    pub mod win7 {
        use crate::models::NotificationData;
        use tauri::Runtime;

        pub fn show<R: Runtime>(
            data: &NotificationData,
            app: &tauri::AppHandle<R>,
        ) -> crate::Result<()> {
            // Clone before closure — closure must be 'static, cannot borrow data.
            let title = data.title.clone();
            let body = data.body.clone();
            let app_clone = app.clone();
            app.run_on_main_thread(move || {
                let mut n = win7_notifications::Notification::new();
                if let Some(ref t) = title {
                    n.summary(t);
                }
                if let Some(ref b) = body {
                    n.body(b);
                }
                if let Some(icon) = app_clone.default_window_icon() {
                    n.icon(icon.rgba().to_vec(), icon.width(), icon.height());
                }
                if let Err(e) = n.show() {
                    log::warn!("[notification] win7 notification failed to show: {}", e);
                }
            })
            .map_err(|_| crate::Error::MainThread)?;
            Ok(())
        }
    }

    // ── Tier 2: Windows 8 ─────────────────────────────────────────────────
    pub mod win8 {
        use super::super::xml_builder::esc;
        use crate::models::NotificationData;
        use windows::{
            core::HSTRING,
            Data::Xml::Dom::XmlDocument,
            UI::Notifications::{ToastNotification, ToastNotificationManager},
        };

        pub fn show(data: &NotificationData, identifier: &str) -> crate::Result<()> {
            let xml_str = format!(
                "<toast><visual><binding template=\"ToastText02\">\
                 <text id=\"1\">{}</text><text id=\"2\">{}</text>\
                 </binding></visual></toast>",
                esc(data.title.as_deref().unwrap_or("")),
                esc(data.body.as_deref().unwrap_or(""))
            );
            fire_basic_toast(&xml_str, identifier)
        }

        pub(crate) fn fire_basic_toast(xml_str: &str, identifier: &str) -> crate::Result<()> {
            let doc = XmlDocument::new()?;
            doc.LoadXml(&HSTRING::from(xml_str))?;
            let toast = ToastNotification::CreateToastNotification(&doc)?;
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identifier))?
                .Show(&toast)?;
            Ok(())
        }
    }

    // ── Tier 3: Windows 8.1 ───────────────────────────────────────────────
    pub mod win81 {
        use super::super::{version::WindowsVersion, xml_builder::build};
        use crate::models::NotificationData;
        use windows::{
            core::HSTRING,
            Data::Xml::Dom::XmlDocument,
            UI::Notifications::{ToastNotification, ToastNotificationManager},
        };

        pub fn show(data: &NotificationData, identifier: &str) -> crate::Result<()> {
            let xml_str = build(data, WindowsVersion::Win81)?;
            let doc = XmlDocument::new()?;
            doc.LoadXml(&HSTRING::from(&xml_str))?;
            let toast = ToastNotification::CreateToastNotification(&doc)?;
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identifier))?
                .Show(&toast)?;
            Ok(())
        }
    }

    // ── Tier 4: Windows 10/11 — full rich toast ───────────────────────────
    //
    // Activation design: ALL activations (foreground button clicks, background
    // buttons, body tap) are routed through the COM activator
    // (INotificationActivationCallback). This avoids registering a
    // TypedEventHandler which requires windows_core types that conflict with
    // the versions pulled in by notify-rust / win7-notifications.
    //
    // Requirements:
    //   - The app must be registered with a COM GUID in tauri.conf.json
    //     under plugins.notification.comServerGuid.
    //   - All <action> elements in the XML must use activationType="background"
    //     or activationType="foreground" — both reach the COM activator when
    //     the app has a registered AUMID + COM server.
    pub mod rich {
        use super::super::{version::WindowsVersion, xml_builder::build};
        use crate::models::{NotificationData, WindowsPriority};
        use windows::{
            core::{Interface, HSTRING},
            Data::Xml::Dom::XmlDocument,
            Foundation::{DateTime, IReference, PropertyValue},
            UI::Notifications::{
                ToastNotification, ToastNotificationManager, ToastNotificationPriority,
            },
        };

        pub fn show(
            data: &NotificationData,
            identifier: &str,
            ver: WindowsVersion,
        ) -> crate::Result<()> {
            let xml_str = build(data, ver)?;
            let doc = XmlDocument::new()?;
            doc.LoadXml(&HSTRING::from(&xml_str))?;
            let toast = ToastNotification::CreateToastNotification(&doc)?;

            // Tag + Group
            if ver.has_tag_group() {
                if let Some(ref t) = data.tag {
                    toast.SetTag(&HSTRING::from(t))?;
                }
                if let Some(ref g) = data.group {
                    toast.SetGroup(&HSTRING::from(g))?;
                }
            }

            // Expiry — PropertyValue::CreateDateTime returns IInspectable;
            // cast to IReference<DateTime> via the Interface trait.
            if ver.has_expiry() {
                if let Some(ms) = data.expiry_ms {
                    match PropertyValue::CreateDateTime(ms_to_winrt_datetime(ms))
                        .and_then(|prop| prop.cast::<IReference<DateTime>>())
                        .and_then(|iref| toast.SetExpirationTime(&iref))
                    {
                        Ok(_) => {}
                        Err(e) => log::debug!("[notification] failed to set expiry: {}", e),
                    }
                }
            }

            // Expires on reboot (Win11+)
            if ver.has_expires_on_reboot() && data.expires_on_reboot {
                toast.SetExpiresOnReboot(true)?;
            }

            // Priority
            if let Some(ref p) = data.priority {
                toast.SetPriority(match p {
                    WindowsPriority::High | WindowsPriority::Urgent => {
                        ToastNotificationPriority::High
                    }
                    WindowsPriority::Default => ToastNotificationPriority::Default,
                })?;
            }

            // NOTE: We do NOT register a TypedEventHandler for Activated here.
            // All activations are handled by the COM activator registered in
            // com_activator::register(). The toast XML uses activationType=
            // "background" for all actions, which routes through COM.
            // Body-tap (no explicit action) also goes through COM when the app
            // has a registered AUMID + COM server GUID.

            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identifier))?
                .Show(&toast)?;
            Ok(())
        }

        fn ms_to_winrt_datetime(offset_ms: u64) -> DateTime {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            const WIN_EPOCH_OFFSET_MS: i64 = 11_644_473_600_000;
            DateTime {
                UniversalTime: (now_ms + WIN_EPOCH_OFFSET_MS + offset_ms as i64) * 10_000,
            }
        }
    }
}
