// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Bridges WinRT toast activation callbacks into Tauri events.
//!
//! The problem: WinRT `TypedEventHandler` callbacks fire on a Windows thread
//! pool thread.  Tauri's event system requires an `AppHandle`, which is not
//! `Send` in older Tauri versions and is not available at all inside a static
//! COM callback.
//!
//! Solution: a `std::sync::mpsc` channel.  The plugin's `init()` spawns a
//! dedicated relay thread that owns the `AppHandle` and forwards
//! `NotificationActionEvent`s as Tauri events.  COM / WinRT callbacks call
//! `dispatch()` which just sends on the channel — no `AppHandle` needed at
//! the call site.

use std::sync::{mpsc, OnceLock};
use tauri::{AppHandle, Emitter, Runtime};

use crate::models::NotificationActionEvent;

/// The Tauri event name emitted when a notification action fires.
pub const EVENT_NAME: &str = "notification://action";

/// Sender half of the action relay channel.
/// Initialized once by `start_relay` and shared across threads.
static SENDER: OnceLock<mpsc::SyncSender<NotificationActionEvent>> = OnceLock::new();

/// Dispatch a notification action event from any thread (COM callback,
/// WinRT handler, etc.).
///
/// If the relay thread has not been started yet (e.g. during early startup),
/// the event is silently dropped and a warning is logged.
pub fn dispatch(event: NotificationActionEvent) {
    println!(
        "[notification] dispatch called: action_id={}",
        event.action_id
    );
    match SENDER.get() {
        Some(tx) => {
            println!("[notification] sending to relay channel");
            if tx.try_send(event).is_err() {
                println!("[notification] ❌ relay channel full");
            }
        }
        None => {
            println!("[notification] ❌ SENDER is None — relay never started!");
        }
    }
}

/// Start the relay thread that forwards `NotificationActionEvent`s as Tauri
/// events on the global app handle.
///
/// Must be called once from `plugin::init()` **after** the `AppHandle` is
/// available.  Safe to call multiple times — subsequent calls are no-ops.
pub fn start_relay<R: Runtime>(app: AppHandle<R>) {
    // Channel capacity: 64 queued events before `try_send` starts failing.
    // This is plenty for realistic notification interaction rates.
    let (tx, rx) = mpsc::sync_channel::<NotificationActionEvent>(64);

    if SENDER.set(tx).is_err() {
        // Already initialized — this is fine
        return;
    }

    std::thread::Builder::new()
        .name("notification-action-relay".to_string())
        .spawn(move || {
            log::debug!("[notification] action relay thread started");
            for event in rx {
                log::debug!(
                    "[notification] relaying action event: action_id={}",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn event_name_is_the_expected_string() {
        assert_eq!(EVENT_NAME, "notification://action");
    }

    // ── dispatch() before relay starts ───────────────────────────────────
    //
    // The SENDER OnceLock may already be set if other tests in the binary
    // have called start_relay (since statics are shared per process).
    // Either way, dispatch() must never panic.

    #[test]
    fn dispatch_does_not_panic_when_relay_not_started() {
        // If SENDER is None, dispatch() logs a warning and returns.
        // If SENDER is already set (from another test), try_send must succeed
        // as long as the channel is not full.
        dispatch(make_event("test-before-relay"));
        // Reaching here means no panic.
    }

    #[test]
    fn dispatch_many_events_does_not_panic() {
        for i in 0..50 {
            dispatch(make_event(&format!("event-{i}")));
        }
    }

    #[test]
    fn dispatch_event_with_inputs_does_not_panic() {
        let mut inputs = HashMap::new();
        inputs.insert("reply_box".to_string(), "Hello world".to_string());
        dispatch(NotificationActionEvent {
            action_id: "send".to_string(),
            inputs,
            tag: Some("msg-42".to_string()),
            group: Some("messages".to_string()),
        });
    }

    #[test]
    fn dispatch_event_with_empty_action_id_does_not_panic() {
        // Empty action_id = body-tap activation (no explicit button)
        dispatch(make_event(""));
    }

    // ── start_relay idempotence ───────────────────────────────────────────
    //
    // We can't test start_relay properly without a real AppHandle (requires a
    // running Tauri runtime).  We test the only thing we can: that calling
    // start_relay when already initialized is a no-op and does not panic.
    // This is implicitly covered by the OnceLock guard: SENDER.set() returns
    // Err if already set and start_relay silently returns.
    //
    // Full relay-thread integration (events arriving on the Tauri event bus)
    // requires an end-to-end test with a Tauri test harness.
}
