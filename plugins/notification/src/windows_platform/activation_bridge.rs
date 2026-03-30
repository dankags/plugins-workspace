// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Deep link + notification activation bridge.
//!
//! Normalises two activation paths into a single `ActivationEvent` that the
//! Tauri frontend receives via the `notification://action` event:
//!
//! ## Path A — Background activation
//!
//! User clicks a `activationType="background"` button. Windows spawns a new
//! process with `----BackgroundActivated`. The COM activator fires
//! `Activate()`, which calls `action_handler::dispatch()`, which emits
//! `notification://action` as a `NotificationActionEvent`.
//!
//! ## Path B — Foreground / deep-link activation
//!
//! User clicks a `activationType="foreground"` button or the toast body.
//! Windows re-focuses the running process (or re-launches it) and passes the
//! `launch` attribute value as a deep-link URI via `tauri-plugin-deep-link`.
//!
//! The URI format written by `xml_builder` is:
//! ```text
//! myapp://notification?action=reply&tag=msg-123&group=chat
//! ```
//! or, when Windows hands back the raw `launch` value:
//! ```text
//! action=reply&tag=msg-123&group=chat
//! ```
//!
//! Both are parsed by `parse_activation_uri`.
//!
//! ## Unified event
//!
//! Both paths emit a `notification://action` Tauri event carrying
//! `NotificationActionEvent` so the frontend handler is identical for both.
//! The `source` field (background / foreground / deep-link) lets callers
//! distinguish if needed.

use std::collections::HashMap;

use crate::models::NotificationActionEvent;

/// Extended activation event with source information.
///
/// The `source` field is the only addition over `NotificationActionEvent`.
/// The frontend `onNotificationAction` handler receives `NotificationActionEvent`
/// directly from `action_handler`; this type is for Rust-side callers that
/// need to know how the activation arrived.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationEvent {
    pub action_id: String,
    pub inputs: HashMap<String, String>,
    pub tag: Option<String>,
    pub group: Option<String>,
    pub source: ActivationSource,
}

/// How the activation reached the bridge.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivationSource {
    /// User clicked the toast body or a `activationType="foreground"` button.
    Foreground,
    /// User clicked a `activationType="background"` button (COM-activated).
    Background,
    /// Activation arrived via a deep-link URI (protocol handler).
    DeepLink,
}

impl From<NotificationActionEvent> for ActivationEvent {
    fn from(e: NotificationActionEvent) -> Self {
        Self {
            action_id: e.action_id,
            inputs: e.inputs,
            tag: e.tag,
            group: e.group,
            source: ActivationSource::Background,
        }
    }
}

/// Parse a notification activation URI or raw query string into an
/// `ActivationEvent`.
///
/// Accepts:
/// - Full URI: `myapp://notification?action=reply&tag=foo&group=bar`
/// - Raw query: `action=reply&tag=foo&group=bar`
/// - Plain action id: `dismiss`
pub fn parse_activation_uri(raw: &str) -> ActivationEvent {
    let query = extract_query(raw);
    parse_query(query, ActivationSource::DeepLink)
}

/// Parse a foreground launch argument string (from the `launch` attribute).
pub fn parse_foreground_args(raw: &str) -> ActivationEvent {
    parse_query(raw, ActivationSource::Foreground)
}

/// Parse a background COM `invoked_args` string.
pub fn parse_background_args(raw: &str) -> ActivationEvent {
    parse_query(raw, ActivationSource::Background)
}

/// Convert an `ActivationEvent` back into a `NotificationActionEvent` for
/// forwarding to `action_handler::dispatch()`.
pub fn to_action_event(e: ActivationEvent) -> NotificationActionEvent {
    NotificationActionEvent {
        action_id: e.action_id,
        inputs: e.inputs,
        tag: e.tag,
        group: e.group,
    }
}

/// Register a deep-link handler that forwards notification URIs as
/// `notification://action` Tauri events.
///
/// Call once from plugin `setup()` after the `AppHandle` is available,
/// alongside `action_handler::start_relay()`.
///
/// Requires the `tauri-plugin-deep-link` plugin to be registered in the app.
#[cfg(all(windows, feature = "deep-link"))]
pub fn register_deep_link_handler<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Emitter;
    use tauri_plugin_deep_link::DeepLinkExt;

    let app_handle = app.clone();

    // 1) Handle launch-via-notification (app was closed, OS spawned it with URL as argv)
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in &urls {
            let ev = parse_activation_uri(url.as_str());
            let action_ev = to_action_event(ev);
            if let Err(e) = app.emit(super::action_handler::EVENT_NAME, &action_ev) {
                log::error!("[notification] initial deep-link emit failed: {e}");
            }
        }
    }

    // 2) Handle notification clicks while app is already running
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            let ev = parse_activation_uri(url.as_str());
            let action_ev = to_action_event(ev);
            if let Err(e) = app_handle.emit(super::action_handler::EVENT_NAME, &action_ev) {
                log::error!("[notification] deep-link emit failed: {e}");
            }
        }
    });
}

// ── Private helpers ───────────────────────────────────────────────────────

/// Strip the URI scheme and path, returning only the query string portion.
fn extract_query(raw: &str) -> &str {
    if let Some(pos) = raw.find('?') {
        &raw[pos + 1..]
    } else if raw.contains("://") {
        // URI with no query params
        ""
    } else {
        raw
    }
}

fn parse_query(query: &str, source: ActivationSource) -> ActivationEvent {
    if query.is_empty() {
        return ActivationEvent {
            action_id: String::new(),
            inputs: HashMap::new(),
            tag: None,
            group: None,
            source,
        };
    }

    if !query.contains('=') {
        return ActivationEvent {
            action_id: unescape(query),
            inputs: HashMap::new(),
            tag: None,
            group: None,
            source,
        };
    }

    let mut map: HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let mut it = pair.splitn(2, '=');
            let k = unescape(it.next()?.trim());
            let v = unescape(it.next().unwrap_or("").trim());
            Some((k, v))
        })
        .collect();

    let action_id = map.remove("action").unwrap_or_default();
    let tag = map.remove("tag").filter(|s| !s.is_empty());
    let group = map.remove("group").filter(|s| !s.is_empty());
    // Remaining keys become inputs (e.g. inline text reply box values)
    let inputs = map;

    ActivationEvent {
        action_id,
        inputs,
        tag,
        group,
        source,
    }
}

/// Decode both XML entities (`&amp;`) and percent-encoding (`%20`).
fn unescape(s: &str) -> String {
    let pct = percent_decode(s);
    pct.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((h << 4 | l) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NotificationActionEvent;

    // ── parse_activation_uri ─────────────────────────────────────────────

    #[test]
    fn full_uri_parsed() {
        let e = parse_activation_uri("myapp://notification?action=reply&tag=msg-1&group=chat");
        assert_eq!(e.action_id, "reply");
        assert_eq!(e.tag.as_deref(), Some("msg-1"));
        assert_eq!(e.group.as_deref(), Some("chat"));
        assert_eq!(e.source, ActivationSource::DeepLink);
    }

    #[test]
    fn raw_query_string_parsed() {
        let e = parse_activation_uri("action=dismiss&tag=t1");
        assert_eq!(e.action_id, "dismiss");
        assert_eq!(e.tag.as_deref(), Some("t1"));
    }

    #[test]
    fn uri_without_query_gives_empty_action() {
        let e = parse_activation_uri("myapp://notification");
        assert!(e.action_id.is_empty());
        assert!(e.tag.is_none());
    }

    #[test]
    fn plain_action_id_parsed() {
        let e = parse_activation_uri("dismiss");
        assert_eq!(e.action_id, "dismiss");
    }

    #[test]
    fn empty_string_gives_empty_event() {
        let e = parse_activation_uri("");
        assert!(e.action_id.is_empty());
        assert!(e.tag.is_none());
        assert!(e.group.is_none());
    }

    // ── source tagging ───────────────────────────────────────────────────

    #[test]
    fn background_args_source() {
        let e = parse_background_args("action=snooze&tag=alarm-1");
        assert_eq!(e.source, ActivationSource::Background);
    }

    #[test]
    fn foreground_args_source() {
        let e = parse_foreground_args("action=open");
        assert_eq!(e.source, ActivationSource::Foreground);
    }

    // ── inputs ───────────────────────────────────────────────────────────

    #[test]
    fn extra_params_become_inputs() {
        let e = parse_activation_uri("action=send&tag=t1&replyBox=Hello+World");
        assert_eq!(e.action_id, "send");
        assert!(e.inputs.contains_key("replyBox"));
    }

    // ── unescape ─────────────────────────────────────────────────────────

    #[test]
    fn percent_encoded_space() {
        assert_eq!(unescape("hello%20world"), "hello world");
    }

    #[test]
    fn xml_entities_decoded() {
        assert_eq!(unescape("a&amp;b"), "a&b");
        assert_eq!(unescape("&lt;tag&gt;"), "<tag>");
    }

    // ── From<NotificationActionEvent> ────────────────────────────────────

    #[test]
    fn from_action_event_sets_background_source() {
        let e = NotificationActionEvent {
            action_id: "dismiss".into(),
            inputs: HashMap::new(),
            tag: Some("t1".into()),
            group: None,
        };
        let ae = ActivationEvent::from(e);
        assert_eq!(ae.action_id, "dismiss");
        assert_eq!(ae.source, ActivationSource::Background);
        assert_eq!(ae.tag.as_deref(), Some("t1"));
    }

    // ── serde ─────────────────────────────────────────────────────────────

    #[test]
    fn activation_event_serializes_camel_case() {
        let e = ActivationEvent {
            action_id: "reply".into(),
            inputs: HashMap::new(),
            tag: Some("t1".into()),
            group: None,
            source: ActivationSource::Background,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"actionId\""), "camelCase: {json}");
        assert!(json.contains("\"background\""), "source: {json}");
    }

    #[test]
    fn all_source_variants_serialize() {
        assert_eq!(
            serde_json::to_string(&ActivationSource::Foreground).unwrap(),
            "\"foreground\""
        );
        assert_eq!(
            serde_json::to_string(&ActivationSource::Background).unwrap(),
            "\"background\""
        );
        assert_eq!(
            serde_json::to_string(&ActivationSource::DeepLink).unwrap(),
            "\"deepLink\""
        );
    }

    // ── to_action_event ───────────────────────────────────────────────────

    #[test]
    fn to_action_event_round_trip() {
        let ae = ActivationEvent {
            action_id: "reply".into(),
            inputs: HashMap::new(),
            tag: Some("t1".into()),
            group: Some("g1".into()),
            source: ActivationSource::DeepLink,
        };
        let ev = to_action_event(ae);
        assert_eq!(ev.action_id, "reply");
        assert_eq!(ev.tag.as_deref(), Some("t1"));
        assert_eq!(ev.group.as_deref(), Some("g1"));
    }
}
