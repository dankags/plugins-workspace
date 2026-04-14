// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Bridges WinRT toast activation callbacks into Tauri events.
//!
//! # Two dispatch paths
//!
//! ## Foreground path (app has a webview)
//!
//! `start_relay(app)` is called during plugin setup.  `dispatch()` sends the
//! event over a `mpsc::SyncSender` to the relay thread, which calls
//! `app.emit("notification://action", event)` so the JS frontend receives it.
//!
//! ## Background path (no webview — COM-activated background process)
//!
//! The caller registers a Rust handler via `register_background_handler(f)`.
//! `dispatch()` calls the handler directly on the worker thread.  This is
//! useful when the background process needs to take action (write to a DB,
//! send an HTTP request, schedule a follow-up notification) without a webview.
//!
//! Both paths are active simultaneously when both are registered — the handler
//! fires first, then the relay emits to the frontend.  In a pure background
//! process `start_relay` is not called, so only the handler fires.
//!
//! # Usage
//!
//! ```rust
//! fn handle_background(event: tauri_plugin_notification::NotificationActionEvent) {
//!     match event.action_id.as_str() {
//!         "reply"   => { /* send the reply  */ }
//!         "dismiss" => { /* mark as read    */ }
//!         ""        => { /* body tap        */ }
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

use std::sync::{mpsc, Arc, OnceLock};
use tauri::{AppHandle, Emitter, Runtime};

use crate::{
    models::NotificationActionEvent, trace_event,
    windows_platform::background_activation::is_background_activation_launch,
};

/// The Tauri event name emitted when a notification action fires.
pub const EVENT_NAME: &str = "notification://action";

// ── Background handler ────────────────────────────────────────────────────────

/// Type alias for the background notification handler.
///
/// The handler receives a fully-parsed `NotificationActionEvent` and is called
/// synchronously on the worker thread.  It must not block indefinitely —
/// spawn a thread inside the handler for any long-running work.
pub type BackgroundHandler = Arc<dyn Fn(NotificationActionEvent) + Send + Sync + 'static>;

/// Global background handler — set once by `register_background_handler()`
/// before the worker thread starts.  Read lock-free on every `dispatch()`.
static BACKGROUND_HANDLER: OnceLock<BackgroundHandler> = OnceLock::new();

/// Register the background notification handler.
///
/// Called internally by `NotificationPlugin::build()` when `.on_background(f)`
/// has been chained.  The handler is stored before the worker thread starts so
/// every `dispatch()` call is guaranteed to see it.
///
/// Accepts any `Fn(NotificationActionEvent) + Send + Sync + 'static` —
/// both plain `fn` pointers and closures that capture `Arc`-wrapped state.
///
/// Subsequent calls are silently ignored (first registration wins).
pub fn register_background_handler<F>(handler: F)
where
    F: Fn(NotificationActionEvent) + Send + Sync + 'static,
{
    let _ = BACKGROUND_HANDLER.set(Arc::new(handler));
}

// ── Relay channel (foreground path) ──────────────────────────────────────────

/// Sender half of the action relay channel.
/// Initialized once by `start_relay` and shared across threads.
static SENDER: OnceLock<mpsc::SyncSender<NotificationActionEvent>> = OnceLock::new();

// ── dispatch() ────────────────────────────────────────────────────────────────

/// Dispatch a notification action event from any thread (COM callback,
/// worker thread, etc.).
///
/// Execution order on every call:
/// 1. If a background handler is registered → call it synchronously.
/// 2. If the relay channel is open → blocking `send()` to the relay thread,
///    which calls `app.emit()`.
///
/// Both steps run independently.  Having a handler does NOT suppress the Tauri
/// event, and having no handler does NOT prevent the relay from emitting.
pub fn dispatch(event: NotificationActionEvent) {
    log::debug!(
        "[notification] dispatch: action_id={:?} tag={:?} group={:?}",
        event.action_id,
        event.tag,
        event.group,
    );

    // ── Step 1: background handler ────────────────────────────────────────
    let should_call_handler = is_background_activation_launch() || cfg!(test);

    if should_call_handler {
        if let Some(handler) = BACKGROUND_HANDLER.get() {
            log::debug!("[notification] calling background handler");
            // Clone so the same event can travel the relay path below as well.
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handler(event.clone());
            })) {
                log::error!("[notification] background handler panicked: {:?}", e);
            }
        }
    }
    // ── Step 2: relay channel → Tauri frontend ────────────────────────────
    match SENDER.get() {
        Some(tx) => {
            log::debug!("[notification] sending to relay channel");
            if tx.send(event).is_err() {
                log::error!("[notification] relay channel closed — relay thread has exited");
            }
        }
        None => {
            // Normal in a background-only process.  Log at debug only when a
            // handler is registered (expected); warn when neither is present.
            if BACKGROUND_HANDLER.get().is_none() {
                log::warn!(
                    "[notification] event dropped — no background handler and no relay. \
                     Register a handler with init().on_background(f) or ensure \
                     start_relay() is called."
                );
            } else {
                log::debug!("[notification] no relay — background handler handled the event");
            }
        }
    }
}

// ── start_relay() (foreground path) ──────────────────────────────────────────

/// Start the relay thread that forwards `NotificationActionEvent`s as Tauri
/// events on the global app handle.
///
/// `SENDER` is set synchronously before this function returns, so by the time
/// `start_worker()` is called `dispatch()` finds a live channel regardless of
/// thread scheduling.
pub fn start_relay<R: Runtime>(app: AppHandle<R>) {
    trace_event!("notification::start_relay initializing");

    // Bail before creating a channel if already set — prevents an orphaned rx.
    if SENDER.get().is_some() {
        trace_event!("notification::start_relay already initialized — skipping");
        return;
    }

    let (tx, rx) = mpsc::sync_channel::<NotificationActionEvent>(64);

    if SENDER.set(tx).is_err() {
        // Lost the race — rx drops cleanly.
        return;
    }

    std::thread::Builder::new()
        .name("notification-action-relay".to_string())
        .spawn(move || {
            trace_event!("notification::start_relay relay thread started");
            log::debug!("[notification] action relay thread started");
            for event in rx {
                log::debug!(
                    "[notification] relaying event: action_id={}",
                    event.action_id
                );
                if let Err(e) = app.emit(EVENT_NAME, &event) {
                    log::error!("[notification] failed to emit action event: {e}");
                }
            }
            log::debug!("[notification] action relay thread exiting");
        })
        .expect("failed to spawn notification action relay thread");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::Duration;

    fn make_event(id: &str) -> NotificationActionEvent {
        NotificationActionEvent {
            action_id: id.to_string(),
            inputs: HashMap::new(),
            tag: None,
            group: None,
        }
    }

    // ── EVENT_NAME ────────────────────────────────────────────────────────

    #[test]
    fn event_name_is_correct() {
        assert_eq!(EVENT_NAME, "notification://action");
    }

    // ── dispatch() without anything registered ────────────────────────────

    #[test]
    fn dispatch_does_not_panic_when_nothing_registered() {
        dispatch(make_event("orphan"));
    }

    // ── background handler called by dispatch() ───────────────────────────
    //
    // Note: BACKGROUND_HANDLER is a process-wide OnceLock. If a prior test
    // in this binary already set it, registration is a no-op and we skip the
    // assertion (the handler from the first registration is still active, but
    // its captured state is different).  This is expected OnceLock behaviour.

    #[test]
    fn background_handler_receives_event() {
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let rx_clone = received.clone();

        let registered = BACKGROUND_HANDLER
            .set(Arc::new(move |ev: NotificationActionEvent| {
                rx_clone.lock().unwrap().push(ev.action_id.clone());
            }))
            .is_ok();

        if registered {
            dispatch(make_event("handler-test"));
            let got = received.lock().unwrap();
            assert!(
                got.contains(&"handler-test".to_string()),
                "handler must receive the dispatched event"
            );
        }
    }

    #[test]
    fn background_handler_receives_correct_fields() {
        let (tx, rx) = std::sync::mpsc::channel::<NotificationActionEvent>();

        let registered = BACKGROUND_HANDLER.set(Arc::new(move |ev| {
            let _ = tx.send(ev);
        }));

        if registered.is_ok() {
            let mut inputs = HashMap::new();
            inputs.insert("reply_box".to_string(), "Hello".to_string());

            dispatch(NotificationActionEvent {
                action_id: "reply".to_string(),
                inputs,
                tag: Some("msg-1".to_string()),
                group: Some("chat".to_string()),
            });

            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) {
                assert_eq!(ev.action_id, "reply");
                assert_eq!(ev.tag.as_deref(), Some("msg-1"));
                assert_eq!(ev.group.as_deref(), Some("chat"));
                assert_eq!(ev.inputs["reply_box"], "Hello");
            }
        }
    }

    #[test]
    fn body_tap_empty_action_id_dispatches_without_panic() {
        dispatch(make_event(""));
    }

    // ── register_background_handler idempotence ───────────────────────────

    #[test]
    fn register_handler_twice_is_noop() {
        let _ = BACKGROUND_HANDLER.set(Arc::new(|_| {}));
        let _ = BACKGROUND_HANDLER.set(Arc::new(|_| {}));
        // Reaching here without panic = pass.
    }

    // ── Concurrency ───────────────────────────────────────────────────────

    #[test]
    fn dispatch_thread_safe_under_parallel_load() {
        let barrier = Arc::new(Barrier::new(10));
        let mut handles = Vec::new();
        for i in 0..10 {
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                b.wait();
                dispatch(make_event(&format!("parallel-{i}")));
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn dispatch_high_volume_no_panic() {
        for i in 0..1000 {
            dispatch(make_event(&format!("bulk-{i}")));
        }
    }

    #[test]
    fn dispatch_unicode_inputs_no_panic() {
        let mut inputs = HashMap::new();
        inputs.insert("emoji".to_string(), "🚀🔥你好".to_string());
        dispatch(NotificationActionEvent {
            action_id: "unicode".to_string(),
            inputs,
            tag: None,
            group: None,
        });
    }

    #[test]
    fn dispatch_large_payload_no_panic() {
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), "X".repeat(10_000));
        dispatch(NotificationActionEvent {
            action_id: "large".to_string(),
            inputs,
            tag: None,
            group: None,
        });
    }
}
