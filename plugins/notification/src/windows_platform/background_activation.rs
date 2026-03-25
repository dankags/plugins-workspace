// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Background activation entrypoint for COM-activated toast callbacks.
//!
//! When Windows launches the app via COM for a background toast activation
//! (the user clicked an action button with `activationType="background"`),
//! the process receives `----BackgroundActivated` as a command-line argument.
//!
//! This module provides:
//! - Detection of background activation launches (re-exported from `com_activator`
//!   for use by callers that should not depend on COM directly)
//! - Argument parsing from the `invoked_args` string passed to `Activate()`
//! - A blocking Win32 message pump that keeps the COM server alive until
//!   `INotificationActivationCallback::Activate` fires
//!
//! # Typical flow
//!
//! ```text
//! User clicks background action button on a toast
//!   └─ Windows spawns: myapp.exe ----BackgroundActivated
//!         └─ desktop::init() logs the background launch
//!               └─ lib.rs setup: com_activator::register() is called
//!                     └─ background_activation::run_pump(5000) keeps process alive
//!                           └─ Windows calls Activate()
//!                                 └─ action_handler::dispatch() sends event
//! ```

use std::collections::HashMap;

/// Returns `true` when this process was launched by Windows for a background
/// COM toast activation (`----BackgroundActivated` is present in argv).
pub fn is_background_activation_launch() -> bool {
    std::env::args().any(|a| a == "----BackgroundActivated")
}

/// Parsed payload from a toast activation `invoked_args` string.
///
/// Produced without needing a live Tauri runtime — useful during early startup
/// before the relay thread is started.
#[derive(Debug, Clone, Default)]
pub struct ActivationPayload {
    /// The action identifier from the toast XML `arguments` attribute.
    pub action_id: String,
    /// Toast tag encoded in the launch args, if present.
    pub tag: Option<String>,
    /// Toast group encoded in the launch args, if present.
    pub group: Option<String>,
    /// Any remaining key=value pairs not consumed as action/tag/group.
    pub extra: HashMap<String, String>,
}

/// Parse the `invoked_args` string passed to
/// `INotificationActivationCallback::Activate`.
///
/// Format produced by `xml_builder`: `action=<id>&tag=<tag>&group=<group>`
/// (parts joined with `&amp;` in XML, decoded to `&` by Windows before
/// passing to `Activate`).
///
/// Plain strings without `=` are treated as bare action ids.
pub fn parse_invoked_args(raw: &str) -> ActivationPayload {
    if raw.is_empty() {
        return ActivationPayload::default();
    }

    if !raw.contains('=') {
        return ActivationPayload {
            action_id: raw.to_string(),
            ..Default::default()
        };
    }

    let mut map: HashMap<String, String> = raw
        .split('&')
        .filter_map(|pair| {
            let mut it = pair.splitn(2, '=');
            let k = unescape_xml(it.next()?.trim());
            let v = unescape_xml(it.next().unwrap_or("").trim());
            Some((k, v))
        })
        .collect();

    let action_id = map.remove("action").unwrap_or_default();
    let tag = map.remove("tag").filter(|s| !s.is_empty());
    let group = map.remove("group").filter(|s| !s.is_empty());

    ActivationPayload {
        action_id,
        tag,
        group,
        extra: map,
    }
}

/// Pump the Win32 message loop for up to `timeout_ms` milliseconds.
///
/// Call this after `com_activator::register()` in a background activation
/// launch to keep the process alive until Windows delivers the COM callback.
/// A 5-second timeout (5000 ms) is appropriate for most cases.
#[cfg(windows)]
pub fn run_pump(timeout_ms: u32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG,
        PM_REMOVE, QS_ALLEVENTS, WM_QUIT,
    };

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);

    log::debug!(
        "[notification] background activation pump started (timeout={}ms)",
        timeout_ms
    );

    loop {
        if std::time::Instant::now() >= deadline {
            log::debug!("[notification] background activation pump timed out");
            break;
        }

        unsafe {
            MsgWaitForMultipleObjects(None, false, 200, QS_ALLEVENTS);

            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, Some(HWND::default()), 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    log::debug!("[notification] background activation pump received WM_QUIT");
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    log::debug!("[notification] background activation pump exiting");
}

#[cfg(not(windows))]
pub fn run_pump(_timeout_ms: u32) {}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_string() {
        let p = parse_invoked_args("");
        assert!(p.action_id.is_empty());
        assert!(p.tag.is_none());
        assert!(p.group.is_none());
    }

    #[test]
    fn parse_plain_action_id() {
        let p = parse_invoked_args("dismiss");
        assert_eq!(p.action_id, "dismiss");
        assert!(p.tag.is_none());
        assert!(p.group.is_none());
    }

    #[test]
    fn parse_action_key_only() {
        let p = parse_invoked_args("action=reply");
        assert_eq!(p.action_id, "reply");
    }

    #[test]
    fn parse_action_with_tag_and_group() {
        let p = parse_invoked_args("action=reply&tag=msg-123&group=chat");
        assert_eq!(p.action_id, "reply");
        assert_eq!(p.tag.as_deref(), Some("msg-123"));
        assert_eq!(p.group.as_deref(), Some("chat"));
    }

    #[test]
    fn parse_empty_tag_becomes_none() {
        let p = parse_invoked_args("action=snooze&tag=&group=");
        assert_eq!(p.action_id, "snooze");
        assert!(p.tag.is_none());
        assert!(p.group.is_none());
    }

    #[test]
    fn parse_extra_keys_go_to_extra_map() {
        let p = parse_invoked_args("action=send&tag=t1&replyBox=hello");
        assert_eq!(p.action_id, "send");
        assert_eq!(p.extra.get("replyBox").map(|s| s.as_str()), Some("hello"));
    }

    #[test]
    fn unescape_five_xml_entities() {
        assert_eq!(unescape_xml("&amp;&lt;&gt;&quot;&apos;"), "&<>\"'");
    }

    #[test]
    fn unescape_plain_unchanged() {
        assert_eq!(unescape_xml("hello"), "hello");
    }

    #[test]
    fn background_activation_absent_in_normal_test_process() {
        assert!(!is_background_activation_launch());
    }
}
