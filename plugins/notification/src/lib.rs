// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Send message notifications (brief auto-expiring OS window element) to your
//! user. Can also be used with the Notification Web API.
//!
//! # Background handler
//!
//! Register a Rust function to be called when a toast notification action
//! arrives in a background COM-activated process (or in the foreground
//! process, alongside the Tauri event):
//!
//! ```rust
//! fn handle_background(event: tauri_plugin_notification::NotificationActionEvent) {
//!     match event.action_id.as_str() {
//!         "reply"   => { /* send the reply  */ }
//!         "dismiss" => { /* mark as read    */ }
//!         ""        => { /* body / toast tap */ }
//!         _         => {}
//!     }
//! }
//!
//! tauri::Builder::default()
//!     .plugin(
//!         tauri_plugin_notification::init()
//!             .on_background(handle_background)
//!             .build()
//!     )
//!     .run(tauri::generate_context!())
//!     .expect("error running app");
//! ```
//!
//! Without `.on_background()`, the plugin still emits `notification://action`
//! Tauri events to the JS frontend as before.

#![doc(
    html_logo_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png",
    html_favicon_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png"
)]

use serde::Serialize;
use std::path::Path;
#[cfg(mobile)]
use tauri::plugin::PluginHandle;
#[cfg(desktop)]
use tauri::AppHandle;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};
#[cfg(windows)]
use windows::core::GUID;

pub use models::*;
pub use tauri::plugin::PermissionState;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

#[cfg(windows)]
pub(crate) mod windows_platform;

// sound_player is part of windows_platform on Windows (exposed via mod.rs).
// On non-Windows desktop platforms it lives at the crate root so desktop.rs
// can reference it as crate::sound_player without a platform guard.
#[cfg(all(desktop, not(windows)))]
pub(crate) mod sound_player;

pub use error::{Error, Result};

#[cfg(desktop)]
pub use desktop::Notification;
#[cfg(mobile)]
pub use mobile::Notification;

// ── NotificationBuilder (per-notification) ────────────────────────────────────

/// Builder for a single notification.  Obtain via
/// [`NotificationExt::notification`] → [`Notification::builder`].
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

    pub fn id(mut self, id: i32) -> Self {
        self.data.id = id;
        self
    }
    pub fn channel_id(mut self, id: impl Into<String>) -> Self {
        self.data.channel_id.replace(id.into());
        self
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.data.title.replace(title.into());
        self
    }
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.data.body.replace(body.into());
        self
    }
    pub fn schedule(mut self, schedule: Schedule) -> Self {
        self.data.schedule.replace(schedule);
        self
    }
    pub fn large_body(mut self, large_body: impl Into<String>) -> Self {
        self.data.large_body.replace(large_body.into());
        self
    }
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.data.summary.replace(summary.into());
        self
    }
    pub fn action_type_id(mut self, id: impl Into<String>) -> Self {
        self.data.action_type_id.replace(id.into());
        self
    }
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.data.group.replace(group.into());
        self
    }
    pub fn group_summary(mut self) -> Self {
        self.data.group_summary = true;
        self
    }
    pub fn sound(mut self, sound: impl Into<String>) -> Self {
        self.data.sound.replace(sound.into());
        self
    }
    pub fn inbox_line(mut self, line: impl Into<String>) -> Self {
        self.data.inbox_lines.push(line.into());
        self
    }
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.data.icon.replace(icon.into());
        self
    }
    pub fn large_icon(mut self, large_icon: impl Into<String>) -> Self {
        self.data.large_icon.replace(large_icon.into());
        self
    }
    pub fn icon_color(mut self, icon_color: impl Into<String>) -> Self {
        self.data.icon_color.replace(icon_color.into());
        self
    }
    pub fn attachment(mut self, attachment: Attachment) -> Self {
        self.data.attachments.push(attachment);
        self
    }
    pub fn extra(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        self.data
            .extra
            .insert(key.into(), serde_json::to_value(value).unwrap());
        self
    }
    pub fn ongoing(mut self) -> Self {
        self.data.ongoing = true;
        self
    }
    pub fn auto_cancel(mut self) -> Self {
        self.data.auto_cancel = true;
        self
    }
    pub fn silent(mut self) -> Self {
        self.data.silent = true;
        self
    }

    // Windows-specific builder methods
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.data.tag.replace(tag.into());
        self
    }
    pub fn hero_image(mut self, src: impl Into<String>) -> Self {
        self.data.hero_image.replace(src.into());
        self
    }
    pub fn windows_action(mut self, action: WindowsAction) -> Self {
        self.data.windows_actions.push(action);
        self
    }
    pub fn windows_input(mut self, input: WindowsInput) -> Self {
        self.data.windows_inputs.push(input);
        self
    }
    pub fn progress(mut self, progress: WindowsProgress) -> Self {
        self.data.progress.replace(progress);
        self
    }
    pub fn expiry_ms(mut self, ms: u64) -> Self {
        self.data.expiry_ms = Some(ms);
        self
    }
    pub fn scenario(mut self, scenario: WindowsScenario) -> Self {
        self.data.scenario.replace(scenario);
        self
    }
    pub fn priority(mut self, priority: WindowsPriority) -> Self {
        self.data.priority.replace(priority);
        self
    }
    pub fn expires_on_reboot(mut self) -> Self {
        self.data.expires_on_reboot = true;
        self
    }
    pub fn background_activation(mut self) -> Self {
        self.data.background_activation = true;
        self
    }
}

// ── NotificationPlugin (plugin-level builder) ─────────────────────────────────

/// Plugin-level builder returned by [`init()`].
///
/// Implements `Into<TauriPlugin>` so it can be passed directly to
/// `tauri::Builder::plugin()` without calling `.build()`.
///
/// `.on_background(f)` is fully optional and only meaningful on Windows.
/// On all other platforms the handler compiles but is never called.
///
/// ```rust
/// // Simplest — no handler required, no .build() required
/// .plugin(tauri_plugin_notification::init())
///
/// // With a background handler
/// .plugin(
///     tauri_plugin_notification::init()
///         .on_background(|event| println!("action: {}", event.action_id))
///         .build()
/// )
/// ```
pub struct NotificationPlugin<R: Runtime> {
    background_handler: Option<Box<dyn Fn(NotificationActionEvent) + Send + Sync + 'static>>,
    _runtime: std::marker::PhantomData<R>,
}

impl<R: Runtime> NotificationPlugin<R> {
    fn new() -> Self {
        Self {
            background_handler: None,
            _runtime: std::marker::PhantomData,
        }
    }

    /// Register a handler called for every notification action that arrives
    /// in the background COM-activated process (and, additionally, in the
    /// foreground process alongside the Tauri `notification://action` event).
    ///
    /// The handler receives a [`NotificationActionEvent`] with:
    /// - `action_id` — the button id you set on the `WindowsAction`, or `""`
    ///   for a body-tap / toast dismiss.
    /// - `inputs` — key-value map of text/selection input values.
    /// - `tag` / `group` — the notification's tag and group for routing.
    ///
    /// The handler is called synchronously on the worker thread.  For
    /// long-running work, spawn a thread inside the handler.
    ///
    /// Accepts any `Fn(NotificationActionEvent) + Send + Sync + 'static` —
    /// both plain function pointers and closures that capture `Arc` state.
    pub fn on_background<F>(mut self, handler: F) -> Self
    where
        F: Fn(NotificationActionEvent) + Send + Sync + 'static,
    {
        self.background_handler = Some(Box::new(handler));
        self
    }

    /// Finalize the plugin and return the `TauriPlugin`.
    ///
    /// Only needed when you have chained `.on_background(f)` — otherwise
    /// `NotificationPlugin` converts to `TauriPlugin` automatically via the
    /// `From` impl when passed to `tauri::Builder::plugin()`.
    pub fn build(self) -> TauriPlugin<R, PluginConfig> {
        // Register the background handler before any thread starts.
        if let Some(handler) = self.background_handler {
            #[cfg(windows)]
            windows_platform::action_handler::register_background_handler(handler);
            // On non-Windows platforms the handler is a no-op compile guard.
            #[cfg(not(windows))]
            let _ = handler;
        }

        build_tauri_plugin()
    }
}

/// Implement `From` so `NotificationPlugin` can be passed directly to
/// `tauri::Builder::plugin()` without an explicit `.build()` call.
///
/// Tauri's `.plugin()` accepts any `Into<TauriPlugin<R>>`, so this conversion
/// makes all three usage patterns valid:
///
/// ```rust
/// // 1. Simplest — no handler, no explicit build() (same ergonomics as original init())
/// .plugin(tauri_plugin_notification::init())
///
/// // 2. Explicit build — identical result to (1)
/// .plugin(tauri_plugin_notification::init().build())
///
/// // 3. With background handler — only pattern that requires .build()
/// .plugin(
///     tauri_plugin_notification::init()
///         .on_background(handle_notification)
///         .build()
/// )
/// ```
///
/// `.on_background()` is completely optional and only meaningful on Windows.
/// On all other platforms the handler is accepted by the type system but
/// silently ignored at runtime.
impl<R: Runtime> From<NotificationPlugin<R>> for TauriPlugin<R, PluginConfig> {
    fn from(plugin: NotificationPlugin<R>) -> Self {
        plugin.build()
    }
}

/// Create the plugin builder.
///
/// Returns a [`NotificationPlugin`] which implements `Into<TauriPlugin>`,
/// so it can be passed directly to `.plugin()` or chained with
/// `.on_background(f)` before calling `.build()`.
///
/// ```rust
/// // No handler needed — pass directly
/// .plugin(tauri_plugin_notification::init())
///
/// // With a background handler (Windows only, optional everywhere)
/// .plugin(
///     tauri_plugin_notification::init()
///         .on_background(|event| {
///             println!("action: {}", event.action_id);
///         })
///         .build()
/// )
/// ```
pub fn init<R: Runtime>() -> NotificationPlugin<R> {
    NotificationPlugin::new()
}

// ── Extensions ────────────────────────────────────────────────────────────────

/// Extensions to [`tauri::App`], [`tauri::AppHandle`], etc. to access the
/// notification APIs.
pub trait NotificationExt<R: Runtime> {
    fn notification(&self) -> &Notification<R>;
}

impl<R: Runtime, T: Manager<R>> crate::NotificationExt<R> for T {
    fn notification(&self) -> &Notification<R> {
        self.state::<Notification<R>>().inner()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

fn pick_icon(icons: &[String]) -> Option<String> {
    let mut png = None;

    for icon in icons {
        if icon.ends_with(".ico") {
            return Some(icon.clone());
        }
        if icon.ends_with(".png") && png.is_none() {
            png = Some(icon.clone());
        }
    }

    png
}

// ── Core plugin wiring ────────────────────────────────────────────────────────

/// Build and return the `TauriPlugin`.  Called by `NotificationPlugin::build()`.
fn build_tauri_plugin<R: Runtime>() -> TauriPlugin<R, PluginConfig> {
    Builder::<R, PluginConfig>::new("notification")
        .invoke_handler(tauri::generate_handler![
            commands::notify,
            commands::request_permission,
            commands::is_permission_granted,
            commands::clear_notification,
            commands::clear_notification_group,
            commands::clear_all_notifications,
            commands::request_listener_access,
            commands::get_listener_access_status,
            commands::get_active_notifications,
            commands::uninstall_notification_registration,
            commands::remove_notification_shortcut,
        ])
        .js_init_script(include_str!("init-iife.js").replace(
            "__TEMPLATE_windows__",
            if cfg!(windows) { "true" } else { "false" },
        ))
        .setup(|app, api| {
            #[cfg(windows)]
            {
                let config: PluginConfig = api.config().clone();
                app.manage(config.clone());


                let app_name = app
                    .config()
                    .product_name
                    .clone()
                    .unwrap_or_else(|| app.config().identifier.clone());

                let guid_str = config
                    .com_server_guid
                    .clone()
                    .unwrap_or_else(|| "default".into());

                // ── Storage migration ─────────────────────────────────────
                // Migrate from the old storage path layout if present.
                let base_dir = dirs::data_local_dir().ok_or_else(|| {
                    tauri::Error::Anyhow(anyhow::anyhow!(
                        "failed to locate local data directory"
                    ))
                })?;

                let old_storage_dir = base_dir
                    .join(format!("tauri-notification-{}", app.config().identifier))
                    .join(&app_name)
                    .join(guid_str.trim_start_matches('{').trim_end_matches('}'));

                let storage_dir = base_dir
                    .join(app.config().identifier.clone())
                    .join(&app_name)
                    .join(guid_str.trim_start_matches('{').trim_end_matches('}'));

                if old_storage_dir.exists() && !storage_dir.exists() {
                    if let Some(parent) = storage_dir.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            tauri::Error::Anyhow(anyhow::anyhow!(
                                "failed to create storage parent: {e}"
                            ))
                        })?;
                    }
                    std::fs::rename(&old_storage_dir, &storage_dir)
                        .or_else(|_| copy_dir_all(&old_storage_dir, &storage_dir))
                        .map_err(|e| {
                            tauri::Error::Anyhow(anyhow::anyhow!(
                                "failed to migrate notification storage: {e}"
                            ))
                        })?;
                    log::info!("[notification] storage migrated to {:?}", storage_dir);
                }

                std::fs::create_dir_all(&storage_dir).map_err(|e| {
                    tauri::Error::Anyhow(anyhow::anyhow!(
                        "failed to create notification storage directory: {e}"
                    ))
                })?;

                windows_platform::runtime_context::init_context(
                    app_name.clone(),
                    guid_str.clone(),
                    storage_dir,
                );

                // ── Launch mode detection ─────────────────────────────────
                let is_bg =
                    windows_platform::com_activator::is_background_activation_launch();

                log::debug!(
                    "[notification] launch mode: {}",
                    if is_bg { "background" } else { "foreground" }
                );

                // ── Core system init (both paths) ─────────────────────────
                // Order matters:
                //   1. load_queue   — restore persisted activations
                //   2. shutdown::init — must precede start_worker
                //   3. start_worker  — begins processing (uses BACKGROUND_HANDLER
                //                      which was registered in NotificationPlugin::build)
                //   4. start_relay   — sets SENDER before returning, so dispatch()
                //                      finds a live channel immediately
                //   5. COM register  — now safe to receive Activate() callbacks
                windows_platform::activation_queue::load_queue();
                windows_platform::shutdown::init();
                windows_platform::activation_queue::start_worker();

                // start_relay sets SENDER synchronously before returning —
                // any COM callback that fires after this point will find SENDER
                // populated. In background mode this relay emits to the Tauri
                // event bus; the background handler (step 3 above) also fires.
                windows_platform::action_handler::start_relay(app.clone());

                // ── COM registration ──────────────────────────────────────
                //
                // BACKGROUND path: run_background_activation_loop() contains
                // a blocking Win32 message pump. We spawn it on a dedicated
                // thread so setup() returns immediately and the exit watcher
                // can start watching before Activate() fires.
                //
                // FOREGROUND path: register_foreground() calls
                // CoRegisterClassObject and stores the result in the global
                // slot, then returns immediately. The Tauri/tao event loop
                // keeps the process alive — no pump needed. Without this
                // call the foreground process had no COM registration and
                // Windows silently dropped every toast action click.
                let guid: Option<GUID> = config
                    .com_server_guid
                    .as_deref()
                    .map(windows_platform::com_activator::parse_guid)
                    .transpose()
                    .map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!(e)))?;

                if let Some(guid) = guid {
                    if is_bg {
                        // BACKGROUND: pump on its own STA thread.
                        std::thread::Builder::new()
                            .name("notification-com-pump".to_string())
                            .spawn(move || {
                                match windows_platform::com_activator::run_background_activation_loop(&guid) {
                                    Ok(_) => log::debug!("[notification] COM background pump completed"),
                                    Err(e) => log::error!("[notification] COM background pump failed: {e}"),
                                }
                            })
                            .expect("failed to spawn notification-com-pump thread");
                    } else {
                        // FOREGROUND: register COM class object, no pump.
                        match windows_platform::com_activator::register_foreground(&guid) {
                            Ok(_) => log::debug!("[notification] COM registration active (foreground)"),
                            Err(e) => log::error!("[notification] COM registration failed: {e}"),
                        }
                    }
                }

                // ── Background process path ───────────────────────────────
                if is_bg {
                    log::debug!("[notification] background activation process started");
                    windows_platform::shutdown::spawn_background_exit_watcher(15);
                    return Ok(());
                }

                // ── Foreground-only initialization ────────────────────────
                log::debug!("[notification] foreground initialization");

                let aumid = app.config().identifier.clone();
                let display_name = app
                    .config()
                    .product_name
                    .clone()
                    .unwrap_or_else(|| aumid.clone());

                if let Some(ref guid_str) = config.com_server_guid {


                    let reg_config = windows_platform::registry_installer::RegistryConfig {
                        com_server_guid: guid_str.clone(),
                        aumid: aumid.clone(),
                        display_name: display_name.clone(),
                        icon_path: pick_icon(&app.config().bundle.icon),
                        exe_path: None,
                    };
                    println!("app icons: {:?}", app.config().bundle.icon);
                    println!("icons file_path: {:?}", pick_icon(&app.config().bundle.icon));

                    // if let Err(e) =
                    //     windows_platform::registry_installer::install(&reg_config)
                    // {
                    //     println!("[notification] Registry installation failed: {e}");
                    //     log::error!("[notification] Registry installation failed: {e}");
                    // }

                    match windows_platform::registry_installer::install(&reg_config) {
    Ok(_) => println!("Registry install OK"),
    Err(e) => println!("Registry install FAILED: {:?}", e),
}

                    let shortcut_config =
                        windows_platform::shortcut_creator::ShortcutConfig {
                            shortcut_name: display_name,
                            aumid,
                            com_server_guid: Some(guid_str.clone()),
                            exe_path: None,
                        };

                    if let Err(e) = windows_platform::shortcut_creator::create_or_update(
                        &shortcut_config,
                    ) {
                        println!("[notification] Shortcut creation failed: {e}");
                        log::warn!("[notification] Shortcut creation failed: {e}");
                    }
                }

                #[cfg(feature = "deep-link")]
                windows_platform::activation_bridge::register_deep_link_handler(app);
            }

            #[cfg(mobile)]
            let notification = mobile::init(app, api)?;
            #[cfg(desktop)]
            let notification = desktop::init(app, api)?;
            app.manage(notification);
            Ok(())
        })
        .on_event(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                #[cfg(windows)]
                {
                    if let Err(e) =
                        crate::windows_platform::com_activator::plugin_unregister()
                    {
                        log::error!("[notification] COM unregistration failed: {e}");
                    }
                    windows_platform::activation_queue::shutdown_worker();
                }
            }
        })
        .build()
}
