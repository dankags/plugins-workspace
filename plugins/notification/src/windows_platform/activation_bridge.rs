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
//!
//! ## Frontend usage
//!
//! ```ts
//! import { listen } from '@tauri-apps/api/event';
//!
//! await listen('notification://action', (event) => {
//!   const { actionId, tag, group, inputs, source } = event.payload;
//!
//!   if (actionId === '') {
//!     // Body-tap / toast dismissed — navigate using tag/group
//!     navigate(`/message/${tag}`);
//!   } else {
//!     // Button click — actionId is the id you set on the WindowsAction
//!     handleButton(actionId, inputs);
//!   }
//! });
//! ```

use std::collections::HashMap;

use crate::models::NotificationActionEvent;

// ── Public types ──────────────────────────────────────────────────────────────

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

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a notification activation URI or raw query string into an
/// `ActivationEvent`.
///
/// Accepts:
/// - Full URI:   `myapp://notification?action=reply&tag=foo&group=bar`
/// - Raw query:  `action=reply&tag=foo&group=bar`
/// - Plain id:   `dismiss`
/// - Empty:      `""` (body-tap with no tag/group)
pub fn parse_activation_uri(raw: &str) -> ActivationEvent {
    let query = extract_query(raw);
    parse_query(query, ActivationSource::DeepLink)
}

/// Parse a foreground launch argument string (from the `launch` attribute).
///
/// Called when Windows delivers the toast body-tap or foreground button click
/// through the deep-link path while the app is already running.
pub fn parse_foreground_args(raw: &str) -> ActivationEvent {
    parse_query(raw, ActivationSource::Foreground)
}

/// Parse a background COM `invoked_args` string.
///
/// Called by `com_activator::NotificationActivator_Impl::Activate()`.
/// Returns an `ActivationEvent` with `inputs` empty — the COM activator
/// merges `NOTIFICATION_USER_INPUT_DATA` on top before calling
/// `to_action_event()`.
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
/// Call once from plugin `setup()` (already done in `lib.rs`) after the
/// `AppHandle` is available, alongside `action_handler::start_relay()`.
///
/// Requires the `tauri-plugin-deep-link` plugin to be registered in the app
/// and the `deep-link` feature flag to be enabled in `Cargo.toml`.
///
/// # How foreground / body-tap activation works
///
/// When the user taps the toast body or a `activationType="foreground"` button
/// while the app is running, Windows re-focuses the process and delivers the
/// `launch` attribute value as a deep-link URL.  `xml_builder` encodes tag and
/// group into that attribute, so the round-trip is lossless.
///
/// When the app is NOT running, Windows re-launches it and passes the URL as
/// the first command-line argument; `tauri-plugin-deep-link` surfaces it via
/// `get_current()` on startup.  Both cases are handled below.
///
/// # tauri.conf.json setup
///
/// ```json
/// {
///   "plugins": {
///     "deep-link": {
///       "desktop": {
///         "schemes": ["myapp"]
///       }
///     }
///   }
/// }
/// ```
///
/// And in `xml_builder` / toast construction the `launch` attribute must be a
/// valid URI: `myapp://notification?tag=X&group=Y`.
// NOTE: cfg guard covers BOTH windows AND the feature flag.  The
// `tauri_plugin_deep_link` import is inside the function body so it is only
// compiled when both conditions are met.
#[cfg(all(windows, feature = "deep-link"))]
pub fn register_deep_link_handler<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Emitter;
    use tauri_plugin_deep_link::DeepLinkExt;

    let app_handle = app.clone();

    // 1) App was NOT running — Windows launched it with the URL as argv[1].
    //    tauri-plugin-deep-link surfaces it via get_current() at startup.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in &urls {
            let ev = parse_activation_uri(url.as_str());
            let action_ev = to_action_event(ev);
            if let Err(e) = app.emit(super::action_handler::EVENT_NAME, &action_ev) {
                log::error!("[notification] initial deep-link emit failed: {e}");
            }
        }
    }

    // 2) App IS already running — Windows re-focuses it and fires on_open_url.
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

// ── Private helpers ───────────────────────────────────────────────────────────

/// Strip the URI scheme and path, returning only the query string portion.
///
/// - `myapp://notification?action=reply` → `action=reply`
/// - `action=reply&tag=t`               → `action=reply&tag=t` (pass-through)
/// - `myapp://notification`             → `""` (no query)
fn extract_query(raw: &str) -> &str {
    if let Some(pos) = raw.find('?') {
        &raw[pos + 1..]
    } else if raw.contains("://") {
        // URI with no query params — nothing to parse
        ""
    } else {
        // Already a raw query string or plain action id
        raw
    }
}

/// Core parser: splits `key=value&…` pairs, extracts `action`/`tag`/`group`,
/// puts remaining keys into `inputs`.
///
/// Handles three input shapes:
/// - Empty string   → empty `ActivationEvent`
/// - No `=` at all → treated as a plain action id (e.g. `"dismiss"`)
/// - `key=value&…` → full parse with `unescape()` applied to every token
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

    // Plain action id with no key=value encoding (legacy / simple case)
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
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            // splitn(2) so values that contain '=' are not split further
            let mut it = pair.splitn(2, '=');
            let k = unescape(it.next()?.trim());
            let v = unescape(it.next().unwrap_or("").trim());
            Some((k, v))
        })
        .collect();

    let action_id = map.remove("action").unwrap_or_default();
    let tag = map.remove("tag").filter(|s| !s.is_empty());
    let group = map.remove("group").filter(|s| !s.is_empty());
    // Remaining keys are inline input values — text reply box contents
    // forwarded by the deep-link path (COM path merges them separately via
    // NOTIFICATION_USER_INPUT_DATA in com_activator.rs).
    let inputs = map;

    ActivationEvent {
        action_id,
        inputs,
        tag,
        group,
        source,
    }
}

/// Decode both percent-encoding (`%20`) and XML entities (`&amp;`) that may
/// survive in the argument string depending on which activation path
/// delivered them.
fn unescape(s: &str) -> String {
    let pct = percent_decode(s);
    pct.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Decode `%XX` percent-encoded byte sequences.
///
/// Non-ASCII bytes are cast to `char` (Latin-1 range). All keys and values
/// produced by `xml_builder` are ASCII; only user-supplied tag/group/action
/// values could be non-ASCII, and those are percent-encoded by the URI layer.
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

// ── Tests ─────────────────────────────────────────────────────────────────────

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

    // ── body-tap ─────────────────────────────────────────────────────────

    #[test]
    fn body_tap_background_produces_empty_action_id() {
        let ev = parse_background_args("tag=msg-123&group=chat");
        assert_eq!(ev.action_id, "", "body-tap must have empty action_id");
        assert_eq!(ev.tag.as_deref(), Some("msg-123"));
        assert_eq!(ev.group.as_deref(), Some("chat"));
    }

    #[test]
    fn body_tap_foreground_produces_empty_action_id() {
        let ev = parse_foreground_args("tag=msg-123");
        assert_eq!(ev.action_id, "");
        assert_eq!(ev.source, ActivationSource::Foreground);
    }

    // ── button click ─────────────────────────────────────────────────────

    #[test]
    fn button_click_recovers_action_id() {
        let ev = parse_background_args("action=reply&tag=msg-1&group=chat");
        assert_eq!(ev.action_id, "reply");
        assert_eq!(ev.tag.as_deref(), Some("msg-1"));
        assert_eq!(ev.group.as_deref(), Some("chat"));
    }

    #[test]
    fn missing_tag_and_group_are_none() {
        let ev = parse_background_args("action=dismiss");
        assert_eq!(ev.action_id, "dismiss");
        assert!(ev.tag.is_none());
        assert!(ev.group.is_none());
    }

    #[test]
    fn empty_args_gives_body_tap_with_no_tag_group() {
        let ev = parse_background_args("");
        assert_eq!(ev.action_id, "");
        assert!(ev.tag.is_none());
        assert!(ev.group.is_none());
    }

    // ── inputs ───────────────────────────────────────────────────────────

    #[test]
    fn inputs_are_empty_before_com_merge() {
        // COM path — inputs arrive via NOTIFICATION_USER_INPUT_DATA, not here
        let ev = parse_background_args("action=send&tag=t");
        assert!(ev.inputs.is_empty());
    }

    #[test]
    fn extra_params_become_inputs_on_deep_link_path() {
        // Deep-link path — inline reply text arrives in the URL query string
        let e = parse_activation_uri("action=send&tag=t1&replyBox=Hello+World");
        assert_eq!(e.action_id, "send");
        assert!(
            e.inputs.contains_key("replyBox"),
            "extra query params must land in inputs"
        );
    }

    // ── unescape / percent decode ─────────────────────────────────────────

    #[test]
    fn percent_encoded_space() {
        assert_eq!(unescape("hello%20world"), "hello world");
    }

    #[test]
    fn xml_entities_decoded() {
        assert_eq!(unescape("a&amp;b"), "a&b");
        assert_eq!(unescape("&lt;tag&gt;"), "<tag>");
    }

    #[test]
    fn value_containing_equals_not_split() {
        // splitn(2) must stop at first '='; value may contain further '='
        let e = parse_background_args("action=a=b=c");
        assert_eq!(e.action_id, "a=b=c");
    }

    #[test]
    fn empty_segments_skipped() {
        let e = parse_background_args("&action=tap&&tag=x&");
        assert_eq!(e.action_id, "tap");
        assert_eq!(e.tag.as_deref(), Some("x"));
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

    // ── Full round-trips ──────────────────────────────────────────────────

    #[test]
    fn full_round_trip_background_button_with_tag_and_group() {
        // xml_builder encodes: arguments="action=reply&amp;tag=msg-1&amp;group=chat"
        // Windows un-escapes &amp; → & before delivering to Activate()
        let windows_delivered = "action=reply&tag=msg-1&group=chat";

        let parsed = parse_background_args(windows_delivered);
        // Simulate COM activator merging NOTIFICATION_USER_INPUT_DATA
        let mut inputs = parsed.inputs.clone();
        inputs.insert("textBox".to_string(), "Hi there".to_string());

        let merged = ActivationEvent { inputs, ..parsed };
        let action = to_action_event(merged);

        assert_eq!(action.action_id, "reply");
        assert_eq!(action.tag.as_deref(), Some("msg-1"));
        assert_eq!(action.group.as_deref(), Some("chat"));
        assert_eq!(action.inputs["textBox"], "Hi there");
    }

    #[test]
    fn full_round_trip_deep_link_body_tap() {
        // xml_builder encodes launch="tag=msg-999"
        // tauri-plugin-deep-link surfaces: myapp://notification?tag=msg-999
        let url = "myapp://notification?tag=msg-999";
        let action = to_action_event(parse_activation_uri(url));
        assert_eq!(action.action_id, "", "body-tap must have empty action_id");
        assert_eq!(action.tag.as_deref(), Some("msg-999"));
        assert!(action.group.is_none());
    }

    #[test]
    fn full_round_trip_foreground_button() {
        let url = "myapp://notification?action=open&tag=alert-42";
        let ev = parse_activation_uri(url);
        assert_eq!(ev.source, ActivationSource::DeepLink);
        let action = to_action_event(ev);
        assert_eq!(action.action_id, "open");
        assert_eq!(action.tag.as_deref(), Some("alert-42"));
    }
}
