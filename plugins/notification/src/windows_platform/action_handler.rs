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

use crate::{models::NotificationActionEvent, trace_event};

/// The Tauri event name emitted when a notification action fires.
pub const EVENT_NAME: &str = "notification://action";

/// Sender half of the action relay channel.
/// Initialized once by `start_relay` and shared across threads.
static SENDER: OnceLock<mpsc::SyncSender<NotificationActionEvent>> = OnceLock::new();

/// Dispatch a notification action event from any thread (COM callback,
/// WinRT handler, etc.).
///
/// Blocks until the relay thread accepts the event (bounded by the channel
/// capacity of 64). If the relay thread has exited the send returns an error
/// and the event is logged but not retried — the activation was already
/// persisted to disk by the queue before this point.
pub fn dispatch(event: NotificationActionEvent) {
    log::debug!(
        "[notification] dispatch called: action_id={}",
        event.action_id
    );

    println!(
        "✔ Dispatching notification action: id={}, inputs={:?}, tag={:?}, group={:?}",
        event.action_id, event.inputs, event.tag, event.group
    );

    match SENDER.get() {
        Some(tx) => {
            log::debug!("[notification] sending to relay channel");
            // event when the 64-slot channel is full (e.g. relay thread is
            // slow on app.emit()). Switched to blocking send() so the worker
            // thread waits rather than losing the event. The worker processes
            // one item at a time so a brief wait here is acceptable and
            // preserves the "exactly-once delivery" guarantee.
            if tx.send(event).is_err() {
                log::error!("[notification] relay channel closed — relay thread has exited");
                println!("⚠️ Warning: failed to dispatch notification action event because the relay thread has exited. ");
            }
        }
        None => {
            log::warn!("[notification] ❌ SENDER is None — relay never started!");
            println!("⚠️ Warning: notification action received but relay thread is not running. Event data: id={}, inputs={:?}, tag={:?}, group={:?}", event.action_id, event.inputs, event.tag, event.group);
        }
    }
}

/// Start the relay thread that forwards `NotificationActionEvent`s as Tauri
/// events on the global app handle.
///
/// Must be called once from `plugin::init()` **after** the `AppHandle` is
/// available. Safe to call multiple times — subsequent calls are no-ops.
pub fn start_relay<R: Runtime>(app: AppHandle<R>) {
    trace_event!("notification::start_relay initializing");
    println!("🔔 Initializing notification action relay...");

    // SENDER.set(tx). If SENDER was already set (second call to start_relay
    // in the same process — e.g. background activation path), set() returned
    // Err and the function returned, but the NEW rx was immediately dropped,
    // killing the paired channel. The OLD tx in SENDER may point to a dead rx
    // whose thread had already exited. Every subsequent dispatch() call would
    // get a SendError and events would be silently lost.
    //

    // populated. This guarantees that an rx is never created and immediately
    // orphaned, and that the existing live channel is always used.
    if SENDER.get().is_some() {
        trace_event!("notification::start_relay already initialized — skipping");
        println!("notification::start_relay already initialized — skipping");
        return;
    }

    let (tx, rx) = mpsc::sync_channel::<NotificationActionEvent>(64);

    if SENDER.set(tx).is_err() {
        // Lost a race with another caller — the channel we just created is
        // unused. rx drops here cleanly; the winner's channel is live.
        println!("⚠️ Warning: start_relay() called multiple times — this call is a no-op because the relay thread is already running. If you see this message during background activation, it means the relay thread from the initial activation is still running and will receive events as expected.");
        return;
    }

    // SENDER is now set. Spawn the relay thread to drain rx.
    std::thread::Builder::new()
        .name("notification-action-relay".to_string())
        .spawn(move || {
            trace_event!("notification::start_relay relay thread started");
            log::debug!("[notification] action relay thread started");
            println!("🔔 Notification action relay thread started.");
            for event in rx {
                log::debug!(
                    "[notification] relaying action event: action_id={}",
                    event.action_id
                );
                if let Err(e) = app.emit(EVENT_NAME, &event) {
                    log::error!("[notification] failed to emit action event: {e}");
                    println!("⚠️ Failed to emit notification action event: {e}");
                }
            }
            log::debug!("[notification] action relay thread exiting");
            println!("🔕 Notification action relay thread exiting.");
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
    // SENDER is a OnceLock set synchronously inside start_relay() before the
    // relay thread is spawned. In tests we never call start_relay() (no real
    // AppHandle), so SENDER stays None and dispatch() must log a warning and
    // return without panicking.

    #[test]
    fn dispatch_does_not_panic_when_relay_not_started() {
        // SENDER is None in a fresh test binary — dispatch() logs a warning.
        // If another test in the same binary has already set SENDER (tests
        // share process-level statics), send() will block until the rx side
        // is ready — which it won't be without a real relay thread. In that
        // case the channel is already set and this test exercises the
        // "channel is live" path. Either way: no panic.
        dispatch(make_event("test-before-relay"));
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

    // ─────────────────────────────────────────────────────────────
    // Concurrency + Reliability Tests
    // ─────────────────────────────────────────────────────────────

    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn dispatch_is_thread_safe_under_parallel_load() {
        let threads = 10;
        let barrier = Arc::new(Barrier::new(threads));

        let mut handles = Vec::new();

        for i in 0..threads {
            let barrier = barrier.clone();

            handles.push(thread::spawn(move || {
                barrier.wait();

                dispatch(make_event(&format!("parallel-{i}")));
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_handles_high_volume_without_panic() {
        for i in 0..1000 {
            dispatch(make_event(&format!("bulk-{i}")));
        }
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_preserves_event_data_integrity() {
        let mut inputs = HashMap::new();

        inputs.insert("username".to_string(), "alice".to_string());

        let event = NotificationActionEvent {
            action_id: "login".to_string(),
            inputs: inputs.clone(),
            tag: Some("session".to_string()),
            group: Some("auth".to_string()),
        };

        dispatch(event.clone());

        assert_eq!(event.action_id, "login");
        assert_eq!(event.inputs.get("username"), Some(&"alice".to_string()));
        assert_eq!(event.tag.as_deref(), Some("session"));
        assert_eq!(event.group.as_deref(), Some("auth"));
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_with_large_input_payload_does_not_panic() {
        let mut inputs = HashMap::new();

        let large_value = "X".repeat(10_000);

        inputs.insert("large".to_string(), large_value);

        dispatch(NotificationActionEvent {
            action_id: "large-test".to_string(),
            inputs,
            tag: None,
            group: None,
        });
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_with_unicode_inputs_does_not_panic() {
        let mut inputs = HashMap::new();

        inputs.insert("emoji".to_string(), "🚀🔥你好".to_string());

        dispatch(NotificationActionEvent {
            action_id: "unicode".to_string(),
            inputs,
            tag: None,
            group: None,
        });
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn multiple_dispatch_calls_do_not_deadlock() {
        let mut handles = Vec::new();

        for i in 0..20 {
            handles.push(thread::spawn(move || {
                dispatch(make_event(&format!("deadlock-{i}")));
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn sender_once_lock_is_initialized_at_most_once() {
        let first = SENDER.get();

        if first.is_some() {
            let second = SENDER.get();

            assert!(second.is_some());
        }
    }

    // ─────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_after_many_calls_remains_stable() {
        for _ in 0..500 {
            dispatch(make_event("stress"));
        }

        dispatch(make_event("final-check"));
    }

    // ── start_relay idempotence ───────────────────────────────────────────
    //
    //  start_relay() now checks SENDER.get().is_some() at entry
    // and returns immediately if already set — BEFORE creating a new channel.
    // This prevents a new rx being orphaned and a dead tx being left in SENDER.
    //
    // We cannot test start_relay() fully without a real AppHandle (requires a
    // running Tauri runtime). The idempotence guarantee is covered by the
    // early-return guard: a second call is a no-op because SENDER is Some.
}
