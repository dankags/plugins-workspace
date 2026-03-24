// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Builds the adaptive toast XML payload from `NotificationData`.
//!
//! The XML spec is documented at:
//! <https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/adaptive-interactive-toasts>
//!
//! All feature gating goes through `WindowsVersion` — the builder never
//! hard-codes version comparisons directly.

use super::version::WindowsVersion;
use crate::models::{
    NotificationData, WindowsActionPlacement, WindowsActionType, WindowsInputType, WindowsScenario,
};

/// Build the complete `<toast>` XML string for the given notification data
/// and target Windows version.
///
/// Features not supported by `ver` are silently omitted — the result is
/// always a valid toast XML document for that version.
pub fn build(data: &NotificationData, ver: WindowsVersion) -> crate::Result<String> {
    let mut xml = String::with_capacity(1024);

    // ── <toast> root ──────────────────────────────────────────────────────
    xml.push_str("<toast");
    if ver.has_scenario() {
        if let Some(ref s) = data.scenario {
            let scenario_str = match s {
                WindowsScenario::Alarm => "alarm",
                WindowsScenario::Reminder => "reminder",
                WindowsScenario::IncomingCall => "incomingCall",
                WindowsScenario::Urgent if ver.has_urgent() => "urgent",
                // Graceful fallback: Urgent → reminder on Win10
                WindowsScenario::Urgent => "reminder",
                WindowsScenario::Default => "default",
            };
            xml.push_str(&format!(" scenario=\"{scenario_str}\""));
        }
    }
    // duration hint — looping audio for alarm/reminder
    if ver.has_scenario() {
        if let Some(ref s) = data.scenario {
            let is_looping = matches!(s, WindowsScenario::Alarm | WindowsScenario::Reminder);
            if is_looping {
                xml.push_str(" duration=\"long\"");
            }
        }
    }
    // Encode tag and group into the launch args so Activate() can recover them
    let mut launch_parts = Vec::new();
    if let Some(ref tag) = data.tag {
        launch_parts.push(format!("tag={}", esc(tag)));
    }
    if let Some(ref group) = data.group {
        launch_parts.push(format!("group={}", esc(group)));
    }
    if !launch_parts.is_empty() {
        xml.push_str(&format!(" launch=\"{}\"", launch_parts.join("&amp;")));
    }
    xml.push('>');

    // ── <visual> ──────────────────────────────────────────────────────────
    xml.push_str("<visual><binding template=\"ToastGeneric\">");

    // Hero image (Win10+)
    if ver.has_hero_image() {
        if let Some(ref src) = data.hero_image {
            xml.push_str(&format!("<image placement=\"hero\" src=\"{}\"/>", esc(src)));
        }
    }

    // App logo override — uses the `icon` field (Win10+)
    if ver.has_logo_override() {
        if let Some(ref src) = data.icon {
            xml.push_str(&format!(
                "<image placement=\"appLogoOverride\" src=\"{}\" hint-crop=\"circle\"/>",
                esc(src)
            ));
        }
    }

    // Title text
    if let Some(ref t) = data.title {
        xml.push_str(&format!("<text>{}</text>", esc(t)));
    }

    // Body text
    if let Some(ref b) = data.body {
        xml.push_str(&format!("<text>{}</text>", esc(b)));
    }

    // Large body (shows as a second text line — truncated on small displays)
    if let Some(ref lb) = data.large_body {
        xml.push_str(&format!("<text hint-maxLines=\"5\">{}</text>", esc(lb)));
    }

    // Progress bar (Win10 2004+)
    if ver.has_progress() {
        if let Some(ref p) = data.progress {
            let val_attr = if p.value < 0.0 {
                "indeterminate".to_string()
            } else {
                format!("{:.4}", p.value.clamp(0.0, 1.0))
            };
            let title_attr = p.title.as_deref().unwrap_or("");
            let status_attr = p.status.as_deref().unwrap_or("");
            let vstr_attr = p.value_string.as_deref().unwrap_or("");
            xml.push_str(&format!(
                "<progress title=\"{}\" value=\"{val_attr}\" \
                 valueStringOverride=\"{}\" status=\"{}\"/>",
                esc(title_attr),
                esc(vstr_attr),
                esc(status_attr),
            ));
        }
    }

    xml.push_str("</binding></visual>");

    // ── <audio> ───────────────────────────────────────────────────────────
    // Win8+ supports <audio>; Win7 uses a different mechanism entirely.
    if ver.has_winrt_toast() {
        let is_looping = ver.has_scenario()
            && matches!(
                data.scenario,
                Some(WindowsScenario::Alarm) | Some(WindowsScenario::Reminder)
            );
        xml.push_str(&build_audio(data, is_looping));
    }

    // ── <actions> (Win10+) ────────────────────────────────────────────────
    // WinRT requires inputs to come before action buttons in the XML.
    if ver.has_actions() && (!data.windows_inputs.is_empty() || !data.windows_actions.is_empty()) {
        xml.push_str("<actions>");

        // Inputs
        for input in &data.windows_inputs {
            match input.input_type {
                WindowsInputType::Text => {
                    xml.push_str(&format!(
                        "<input id=\"{}\" type=\"text\" placeHolderContent=\"{}\"/>",
                        esc(&input.id),
                        esc(input.placeholder.as_deref().unwrap_or(""))
                    ));
                }
                WindowsInputType::Selection if ver.has_selection_input() => {
                    let default_attr = input
                        .default_selection
                        .as_ref()
                        .map(|d| format!(" defaultInput=\"{}\"", esc(d)))
                        .unwrap_or_default();
                    xml.push_str(&format!(
                        "<input id=\"{}\" type=\"selection\"{default_attr}>",
                        esc(&input.id)
                    ));
                    for item in &input.selections {
                        xml.push_str(&format!(
                            "<selection id=\"{}\" content=\"{}\"/>",
                            esc(&item.id),
                            esc(&item.content)
                        ));
                    }
                    xml.push_str("</input>");
                }
                // Selection not yet available — render as text input
                WindowsInputType::Selection => {
                    xml.push_str(&format!(
                        "<input id=\"{}\" type=\"text\" placeHolderContent=\"{}\"/>",
                        esc(&input.id),
                        esc(input.placeholder.as_deref().unwrap_or(""))
                    ));
                }
            }
        }

        // Action buttons
        for action in &data.windows_actions {
            let activation = match action.action_type {
                WindowsActionType::Background => "background",
                WindowsActionType::Protocol => "protocol",
                WindowsActionType::Foreground => "foreground",
            };

            // Protocol actions use the URI verbatim; others encode as key=value
            // so Activate() can recover action_id, tag, and group.
            // Non-protocol actions always use action.id directly, even if a
            // protocol field happens to be set on the action.
            let args = if action.action_type != WindowsActionType::Protocol {
                let mut parts = vec![format!("action={}", esc(&action.id))];
                if let Some(ref tag) = data.tag {
                    parts.push(format!("tag={}", esc(tag)));
                }
                if let Some(ref group) = data.group {
                    parts.push(format!("group={}", esc(group)));
                }
                // Join with XML-encoded & so the attribute value stays valid.
                parts.join("&amp;")
            } else {
                action.protocol.as_deref().unwrap_or(&action.id).to_string()
            };

            let icon_attr = action
                .icon
                .as_ref()
                .map(|i| format!(" imageUri=\"{}\"", esc(i)))
                .unwrap_or_default();

            let placement_attr = match action.placement {
                WindowsActionPlacement::ContextMenu => " placement=\"contextMenu\"",
                WindowsActionPlacement::Default => "",
            };

            // Inline button bound to a specific input (quick-reply pattern)
            let hint_input_id = action
                .input_id
                .as_ref()
                .map(|id| format!(" hint-inputId=\"{}\"", esc(id)))
                .unwrap_or_default();

            xml.push_str(&format!(
                "<action content=\"{}\" arguments=\"{}\" \
                 activationType=\"{activation}\"{icon_attr}{placement_attr}{hint_input_id}/>",
                esc(&action.label),
                // Non-protocol args are pre-escaped (esc applied per-part, & → &amp;).
                // Protocol URIs are passed through esc here for attribute safety.
                if action.action_type == WindowsActionType::Protocol {
                    esc(&args)
                } else {
                    args
                },
            ));
        }

        xml.push_str("</actions>");
    }

    xml.push_str("</toast>");
    Ok(xml)
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Minimal XML escaping — prevents malformed payloads from user-supplied text.
pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Build the `<audio>` element from the notification's sound / silent fields.
/// `is_looping` should be true for alarm and reminder scenarios, which use
/// `duration="long"` and require the audio to loop for the duration.
fn build_audio(data: &NotificationData, is_looping: bool) -> String {
    if data.silent {
        return "<audio silent=\"true\"/>".to_string();
    }

    let loop_attr = if is_looping { "true" } else { "false" };

    match &data.sound {
        None => {
            // For looping scenarios with no explicit sound, emit a looping
            // alarm sound so the audio persists with the duration="long" toast.
            if is_looping {
                format!(
                    "<audio src=\"ms-winsoundevent:Notification.Looping.Alarm\" loop=\"{loop_attr}\"/>"
                )
            } else {
                String::new()
            }
        }
        Some(sound) => {
            // Treat the string "silent" as a silent flag
            if sound.eq_ignore_ascii_case("silent") {
                return "<audio silent=\"true\"/>".to_string();
            }
            if sound.ends_with(".wav") || sound.contains('\\') || sound.contains('/') {
                format!("<audio src=\"{}\" loop=\"{loop_attr}\"/>", esc(sound))
            } else {
                format!(
                    "<audio src=\"ms-winsoundevent:Notification.{}\" loop=\"{loop_attr}\"/>",
                    esc(sound)
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        NotificationData, WindowsAction, WindowsActionPlacement, WindowsActionType, WindowsInput,
        WindowsInputType, WindowsProgress, WindowsScenario, WindowsSelectionItem,
    };

    // ── Test-data helpers ─────────────────────────────────────────────────

    fn data(title: &str, body: &str) -> NotificationData {
        NotificationData {
            title: Some(title.into()),
            body: Some(body.into()),
            ..Default::default()
        }
    }

    fn bg_action(id: &str, label: &str) -> WindowsAction {
        WindowsAction {
            id: id.into(),
            label: label.into(),
            action_type: WindowsActionType::Background,
            protocol: None,
            icon: None,
            placement: WindowsActionPlacement::Default,
            input_id: None,
        }
    }

    fn text_input(id: &str, placeholder: &str) -> WindowsInput {
        WindowsInput {
            id: id.into(),
            placeholder: Some(placeholder.into()),
            input_type: WindowsInputType::Text,
            selections: vec![],
            default_selection: None,
        }
    }

    fn selection_input(id: &str) -> WindowsInput {
        WindowsInput {
            id: id.into(),
            placeholder: None,
            input_type: WindowsInputType::Selection,
            selections: vec![
                WindowsSelectionItem {
                    id: "opt_a".into(),
                    content: "Option A".into(),
                },
                WindowsSelectionItem {
                    id: "opt_b".into(),
                    content: "Option B".into(),
                },
            ],
            default_selection: Some("opt_a".into()),
        }
    }

    // ── esc() ─────────────────────────────────────────────────────────────

    #[test]
    fn esc_ampersand() {
        assert_eq!(esc("a & b"), "a &amp; b");
    }
    #[test]
    fn esc_less_than() {
        assert_eq!(esc("a < b"), "a &lt; b");
    }
    #[test]
    fn esc_greater_than() {
        assert_eq!(esc("a > b"), "a &gt; b");
    }
    #[test]
    fn esc_double_quote() {
        assert_eq!(esc(r#"a "b" c"#), r#"a &quot;b&quot; c"#);
    }
    #[test]
    fn esc_single_quote() {
        assert_eq!(esc("it's"), "it&apos;s");
    }
    #[test]
    fn esc_plain() {
        assert_eq!(esc("hello"), "hello");
    }

    #[test]
    fn esc_all_special_chars_together() {
        assert_eq!(
            esc(r#"<a>&"b"c'</a>"#),
            "&lt;a&gt;&amp;&quot;b&quot;c&apos;&lt;/a&gt;"
        );
    }

    #[test]
    fn esc_empty_string() {
        assert_eq!(esc(""), "");
    }

    // ── Structural shell — valid for every version ────────────────────────

    #[test]
    fn every_version_produces_valid_toast_shell() {
        let d = data("Title", "Body");
        let versions = [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
            WindowsVersion::Win10,
            WindowsVersion::Win11,
        ];
        for v in versions {
            let xml = build(&d, v).unwrap();
            assert!(
                xml.starts_with("<toast"),
                "{}: must start with <toast",
                v.display_name()
            );
            assert!(
                xml.ends_with("</toast>"),
                "{}: must end with </toast>",
                v.display_name()
            );
            assert!(
                xml.contains("<visual>"),
                "{}: missing <visual>",
                v.display_name()
            );
            assert!(
                xml.contains("</visual>"),
                "{}: missing </visual>",
                v.display_name()
            );
        }
    }

    #[test]
    fn toast_generic_binding_template_always_present() {
        let xml = build(&data("t", "b"), WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"template="ToastGeneric""#));
    }

    #[test]
    fn title_and_body_emitted_as_text_elements() {
        let xml = build(&data("Hello", "World"), WindowsVersion::Win10).unwrap();
        assert!(xml.contains("<text>Hello</text>"));
        assert!(xml.contains("<text>World</text>"));
    }

    #[test]
    fn title_and_body_are_xml_escaped() {
        let xml = build(&data("5 < 10", r#"Use & "quotes""#), WindowsVersion::Win10).unwrap();
        assert!(xml.contains("5 &lt; 10"));
        assert!(xml.contains("Use &amp; &quot;quotes&quot;"));
    }

    #[test]
    fn missing_title_omits_text_element() {
        let d = NotificationData {
            body: Some("body only".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert_eq!(xml.matches("<text>").count(), 1);
    }

    #[test]
    fn large_body_uses_hint_max_lines() {
        let d = NotificationData {
            large_body: Some("Long description here.".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"hint-maxLines="5""#));
        assert!(xml.contains("Long description here."));
    }

    // ── Audio ─────────────────────────────────────────────────────────────

    #[test]
    fn silent_flag_emits_silent_audio() {
        let d = NotificationData {
            silent: true,
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"<audio silent="true"/>"#));
    }

    #[test]
    fn silent_string_emits_silent_audio() {
        let d = NotificationData {
            sound: Some("silent".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"<audio silent="true"/>"#));
    }

    #[test]
    fn named_sound_uses_ms_winsoundevent() {
        let d = NotificationData {
            sound: Some("Mail".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("ms-winsoundevent:Notification.Mail"));
        assert!(xml.contains(r#"loop="false""#));
    }

    #[test]
    fn no_sound_field_produces_no_audio_element() {
        let xml = build(&data("t", "b"), WindowsVersion::Win10).unwrap();
        assert!(
            !xml.contains("<audio"),
            "must not emit <audio> when sound is None"
        );
    }

    #[test]
    fn win7_produces_no_audio_element() {
        // Win7 uses tray balloon — no XML audio element
        let d = NotificationData {
            sound: Some("Mail".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win7).unwrap();
        assert!(!xml.contains("<audio"), "Win7 must not emit <audio>");
    }

    #[test]
    fn alarm_scenario_loops_audio() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Alarm),
            sound: Some("Mail".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"loop="true""#), "alarm must loop audio");
    }

    #[test]
    fn reminder_scenario_loops_audio() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Reminder),
            sound: Some("Reminder".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"loop="true""#), "reminder must loop audio");
    }

    #[test]
    fn alarm_with_no_sound_emits_looping_default() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Alarm),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(
            xml.contains("<audio"),
            "alarm with no sound must emit audio element"
        );
        assert!(
            xml.contains("Looping.Alarm"),
            "must use looping alarm sound"
        );
        assert!(xml.contains(r#"loop="true""#));
    }

    #[test]
    fn incoming_call_scenario_does_not_loop_audio() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::IncomingCall),
            sound: Some("Call".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(
            xml.contains(r#"loop="false""#),
            "incomingCall must not loop audio"
        );
    }

    #[test]
    fn no_scenario_does_not_loop_audio() {
        let d = NotificationData {
            sound: Some("Mail".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"loop="false""#));
    }

    // ── Hero image ────────────────────────────────────────────────────────

    #[test]
    fn hero_image_emitted_on_win10() {
        let d = NotificationData {
            hero_image: Some("C:\\hero.png".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"placement="hero""#));
        assert!(xml.contains("hero.png"));
    }

    #[test]
    fn hero_image_suppressed_below_win10pre() {
        let d = NotificationData {
            hero_image: Some("C:\\hero.png".into()),
            ..Default::default()
        };
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
        ] {
            let xml = build(&d, v).unwrap();
            assert!(
                !xml.contains(r#"placement="hero""#),
                "{} must not have hero image",
                v.display_name()
            );
        }
    }

    // ── Logo override ─────────────────────────────────────────────────────

    #[test]
    fn logo_override_emitted_from_win81() {
        let d = NotificationData {
            icon: Some("C:\\icon.png".into()),
            ..Default::default()
        };
        for v in [
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
            WindowsVersion::Win10,
            WindowsVersion::Win11,
        ] {
            let xml = build(&d, v).unwrap();
            assert!(
                xml.contains(r#"placement="appLogoOverride""#),
                "{} must have appLogoOverride",
                v.display_name()
            );
            assert!(
                xml.contains(r#"hint-crop="circle""#),
                "{} must have hint-crop",
                v.display_name()
            );
        }
    }

    #[test]
    fn logo_override_suppressed_on_win7_and_win8() {
        let d = NotificationData {
            icon: Some("C:\\icon.png".into()),
            ..Default::default()
        };
        for v in [WindowsVersion::Win7, WindowsVersion::Win8] {
            let xml = build(&d, v).unwrap();
            assert!(
                !xml.contains(r#"placement="appLogoOverride""#),
                "{} must not have appLogoOverride",
                v.display_name()
            );
        }
    }

    // ── Progress bar ──────────────────────────────────────────────────────

    fn prog(value: f32) -> WindowsProgress {
        WindowsProgress {
            value,
            title: Some("Uploading".into()),
            status: Some("In progress".into()),
            value_string: Some("50%".into()),
        }
    }

    #[test]
    fn progress_bar_emitted_on_win10_2004() {
        let d = NotificationData {
            progress: Some(prog(0.5)),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("<progress"), "must contain <progress>");
        assert!(xml.contains("0.5000"));
        assert!(xml.contains("In progress"));
        assert!(xml.contains("Uploading"));
        assert!(xml.contains("50%"));
    }

    #[test]
    fn progress_value_negative_means_indeterminate() {
        let d = NotificationData {
            progress: Some(prog(-1.0)),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"value="indeterminate""#));
    }

    #[test]
    fn progress_value_clamped_above_one() {
        let d = NotificationData {
            progress: Some(prog(2.0)),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("1.0000"), "value > 1 must be clamped to 1");
    }

    #[test]
    fn progress_value_clamped_at_zero() {
        let d = NotificationData {
            progress: Some(prog(0.0)),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("0.0000"));
    }

    #[test]
    fn progress_suppressed_on_win10_pre19041() {
        let d = NotificationData {
            progress: Some(prog(0.5)),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10Pre19041).unwrap();
        assert!(
            !xml.contains("<progress"),
            "Win10Pre must not emit <progress>"
        );
    }

    // ── Scenario ──────────────────────────────────────────────────────────

    #[test]
    fn alarm_scenario_sets_long_duration() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Alarm),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"scenario="alarm""#));
        assert!(xml.contains(r#"duration="long""#));
    }

    #[test]
    fn reminder_scenario_sets_long_duration() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Reminder),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"scenario="reminder""#));
        assert!(xml.contains(r#"duration="long""#));
    }

    #[test]
    fn incoming_call_no_long_duration() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::IncomingCall),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"scenario="incomingCall""#));
        assert!(!xml.contains(r#"duration="long""#));
    }

    #[test]
    fn default_scenario_emits_default_attribute() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Default),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"scenario="default""#));
    }

    #[test]
    fn urgent_scenario_emitted_on_win11() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Urgent),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win11).unwrap();
        assert!(xml.contains(r#"scenario="urgent""#));
    }

    #[test]
    fn urgent_scenario_falls_back_to_reminder_on_win10() {
        // On Win10, Urgent is not supported — must degrade to reminder silently
        let d = NotificationData {
            scenario: Some(WindowsScenario::Urgent),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(
            xml.contains(r#"scenario="reminder""#),
            "Urgent must fall back to reminder on Win10"
        );
        assert!(!xml.contains(r#"scenario="urgent""#));
    }

    #[test]
    fn scenario_entirely_suppressed_below_win10pre() {
        let d = NotificationData {
            scenario: Some(WindowsScenario::Alarm),
            ..Default::default()
        };
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
        ] {
            let xml = build(&d, v).unwrap();
            assert!(
                !xml.contains("scenario="),
                "{} must not emit scenario attribute",
                v.display_name()
            );
        }
    }

    // ── Action buttons ────────────────────────────────────────────────────

    #[test]
    fn background_action_emits_correct_attributes() {
        let mut d = data("t", "b");
        d.windows_actions = vec![bg_action("dismiss", "Dismiss")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("<actions>"), "must open <actions>");
        assert!(xml.contains("</actions>"), "must close </actions>");
        assert!(xml.contains(r#"content="Dismiss""#));
        // Arguments are now encoded as key=value
        assert!(
            xml.contains("action=dismiss"),
            "action id must be encoded as action=<id>"
        );
        assert!(xml.contains(r#"activationType="background""#));
    }

    #[test]
    fn foreground_action_type_emitted() {
        let mut d = data("t", "b");
        d.windows_actions = vec![WindowsAction {
            action_type: WindowsActionType::Foreground,
            ..bg_action("open", "Open")
        }];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"activationType="foreground""#));
    }

    #[test]
    fn protocol_action_uses_uri_as_argument() {
        let mut d = data("t", "b");
        d.windows_actions = vec![WindowsAction {
            action_type: WindowsActionType::Protocol,
            protocol: Some("https://example.com".into()),
            ..bg_action("link", "Visit")
        }];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"activationType="protocol""#));
        assert!(
            xml.contains("https://example.com"),
            "URI must be the argument for protocol actions"
        );
        // Protocol actions pass the URI verbatim — no action= encoding
        assert!(
            !xml.contains("action=link"),
            "protocol action must not use key=value encoding"
        );
    }

    #[test]
    fn context_menu_placement_emitted() {
        let mut d = data("t", "b");
        d.windows_actions = vec![WindowsAction {
            placement: WindowsActionPlacement::ContextMenu,
            ..bg_action("snooze", "Snooze")
        }];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"placement="contextMenu""#));
    }

    #[test]
    fn action_icon_emitted_as_image_uri() {
        let mut d = data("t", "b");
        d.windows_actions = vec![WindowsAction {
            icon: Some("C:\\icons\\reply.png".into()),
            ..bg_action("reply", "Reply")
        }];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("imageUri="));
        assert!(xml.contains("reply.png"));
    }

    #[test]
    fn multiple_actions_all_appear_in_xml() {
        let mut d = data("t", "b");
        d.windows_actions = vec![
            bg_action("yes", "Yes"),
            bg_action("no", "No"),
            bg_action("maybe", "Maybe"),
        ];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"content="Yes""#));
        assert!(xml.contains(r#"content="No""#));
        assert!(xml.contains(r#"content="Maybe""#));
        // 3 <action ...> elements
        assert_eq!(xml.matches("<action ").count(), 3);
    }

    #[test]
    fn actions_suppressed_below_win10pre() {
        let mut d = data("t", "b");
        d.windows_actions = vec![bg_action("dismiss", "Dismiss")];
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
        ] {
            let xml = build(&d, v).unwrap();
            assert!(
                !xml.contains("<actions>"),
                "{} must not emit <actions>",
                v.display_name()
            );
        }
    }

    #[test]
    fn no_actions_block_when_nothing_to_emit() {
        let xml = build(&data("t", "b"), WindowsVersion::Win10).unwrap();
        assert!(
            !xml.contains("<actions>"),
            "empty <actions> must not appear"
        );
    }

    // ── Text input ────────────────────────────────────────────────────────

    #[test]
    fn text_input_emitted_with_id_and_placeholder() {
        let mut d = data("t", "b");
        d.windows_inputs = vec![text_input("reply_box", "Type a reply...")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"type="text""#));
        assert!(xml.contains(r#"id="reply_box""#));
        assert!(xml.contains("Type a reply..."));
    }

    #[test]
    fn inline_reply_button_sets_hint_input_id() {
        let mut d = data("t", "b");
        d.windows_inputs = vec![text_input("box", "Reply...")];
        let mut btn = bg_action("send", "Send");
        btn.input_id = Some("box".into());
        d.windows_actions = vec![btn];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"hint-inputId="box""#));
    }

    #[test]
    fn inputs_precede_actions_in_xml() {
        // WinRT schema requires inputs before action buttons
        let mut d = data("t", "b");
        d.windows_inputs = vec![text_input("box", "Reply...")];
        let mut btn = bg_action("send", "Send");
        btn.input_id = Some("box".into());
        d.windows_actions = vec![btn];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        let input_pos = xml.find(r#"type="text""#).expect("input element missing");
        let action_pos = xml
            .find(r#"content="Send""#)
            .expect("action element missing");
        assert!(
            input_pos < action_pos,
            "input element must precede action element in XML"
        );
    }

    // ── Selection input ───────────────────────────────────────────────────

    #[test]
    fn selection_input_emitted_with_items_on_win10() {
        let mut d = data("t", "b");
        d.windows_inputs = vec![selection_input("sel")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains(r#"type="selection""#));
        assert!(xml.contains(r#"defaultInput="opt_a""#));
        assert!(xml.contains(r#"id="opt_a" content="Option A""#));
        assert!(xml.contains(r#"id="opt_b" content="Option B""#));
    }

    #[test]
    fn selection_input_degrades_to_text_when_not_supported() {
        // has_selection_input() is true from Win10Pre19041 onward, so we need
        // a version that has actions (so the <actions> block is emitted at all)
        // but does NOT yet have selection — that is Win81 and below.
        // Win81 does not have has_actions() so the entire <actions> block is
        // suppressed there; the fallback text path only fires when
        // has_actions() == true && has_selection_input() == false, which never
        // occurs in the current version matrix (both gates share the same
        // Win10Pre19041 boundary).
        //
        // The degradation arm therefore can only be exercised synthetically.
        // We verify instead that Win10Pre19041 correctly emits the FULL
        // selection element (not the degraded form), confirming the gate works.
        let mut d = data("t", "b");
        d.windows_inputs = vec![selection_input("sel")];
        let xml = build(&d, WindowsVersion::Win10Pre19041).unwrap();
        assert!(
            xml.contains(r#"type="selection""#),
            "Win10Pre19041 must emit full selection input"
        );
        assert!(
            xml.contains(r#"id="opt_a""#),
            "selection items must be present"
        );
    }

    // ── XML well-formedness ───────────────────────────────────────────────

    #[test]
    fn actions_block_has_matching_open_and_close_tags() {
        let mut d = data("t", "b");
        d.windows_actions = vec![bg_action("a", "A")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert_eq!(xml.matches("<actions>").count(), 1);
        assert_eq!(xml.matches("</actions>").count(), 1);
    }

    #[test]
    fn visual_binding_tags_are_balanced() {
        let xml = build(&data("t", "b"), WindowsVersion::Win10).unwrap();
        assert_eq!(xml.matches("<visual>").count(), 1);
        assert_eq!(xml.matches("</visual>").count(), 1);
        assert_eq!(xml.matches("<binding").count(), 1);
        assert_eq!(xml.matches("</binding>").count(), 1);
    }

    // ── launch attribute — tag/group round-trip ───────────────────────────

    #[test]
    fn launch_attribute_absent_when_no_tag_or_group() {
        let xml = build(&data("t", "b"), WindowsVersion::Win10).unwrap();
        assert!(
            !xml.contains("launch="),
            "launch attribute must not appear when tag and group are both None"
        );
    }

    #[test]
    fn launch_attribute_encodes_tag_only() {
        let d = NotificationData {
            title: Some("t".into()),
            tag: Some("msg-123".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("launch="), "launch attribute must be present");
        assert!(xml.contains("tag=msg-123"), "tag must be encoded in launch");
        assert!(!xml.contains("group="), "group must not appear when None");
    }

    #[test]
    fn launch_attribute_encodes_group_only() {
        let d = NotificationData {
            title: Some("t".into()),
            group: Some("chat".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("launch="), "launch attribute must be present");
        assert!(
            xml.contains("group=chat"),
            "group must be encoded in launch"
        );
        assert!(!xml.contains("tag="), "tag must not appear when None");
    }

    #[test]
    fn launch_attribute_encodes_both_tag_and_group() {
        let d = NotificationData {
            title: Some("t".into()),
            tag: Some("msg-123".into()),
            group: Some("chat".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("tag=msg-123"), "tag must be in launch");
        assert!(xml.contains("group=chat"), "group must be in launch");
    }

    #[test]
    fn launch_tag_group_are_xml_escaped() {
        let d = NotificationData {
            title: Some("t".into()),
            tag: Some("a&b".into()),
            group: Some("<grp>".into()),
            ..Default::default()
        };
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("tag=a&amp;b"), "tag value must be XML-escaped");
        assert!(
            xml.contains("group=&lt;grp&gt;"),
            "group value must be XML-escaped"
        );
    }

    // ── action arguments encode tag/group ────────────────────────────────

    #[test]
    fn action_arguments_include_tag_and_group_when_set() {
        let mut d = NotificationData {
            title: Some("t".into()),
            tag: Some("msg-123".into()),
            group: Some("chat".into()),
            ..Default::default()
        };
        d.windows_actions = vec![bg_action("reply", "Reply")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(
            xml.contains("action=reply"),
            "action id must be in arguments"
        );
        // Parts are joined with &amp; and each value is esc()-ed
        assert!(
            xml.contains("tag=msg-123"),
            "tag must be in action arguments"
        );
        assert!(
            xml.contains("group=chat"),
            "group must be in action arguments"
        );
        // Separator must be &amp; not raw & to keep the XML attribute valid
        assert!(
            xml.contains("action=reply&amp;tag="),
            "parts must be joined with &amp;"
        );
    }

    #[test]
    fn action_arguments_omit_tag_group_when_absent() {
        let mut d = data("t", "b");
        d.windows_actions = vec![bg_action("dismiss", "Dismiss")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        assert!(xml.contains("action=dismiss"));
        // No tag= or group= should appear anywhere in the arguments
        let args_start = xml.find("arguments=").expect("arguments attr missing");
        let args_end = xml[args_start..].find('"').unwrap() + args_start;
        let args_end = xml[args_end + 1..].find('"').unwrap() + args_end + 1;
        let args_slice = &xml[args_start..=args_end];
        assert!(
            !args_slice.contains("tag="),
            "tag must not appear when None"
        );
        assert!(
            !args_slice.contains("group="),
            "group must not appear when None"
        );
    }

    #[test]
    fn protocol_action_arguments_never_encoded_with_tag_group() {
        // Protocol actions pass URI verbatim — tag/group are NOT appended
        let mut d = NotificationData {
            title: Some("t".into()),
            tag: Some("msg-123".into()),
            group: Some("chat".into()),
            ..Default::default()
        };
        d.windows_actions = vec![WindowsAction {
            action_type: WindowsActionType::Protocol,
            protocol: Some("https://example.com".into()),
            ..bg_action("link", "Visit")
        }];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        // The URI must appear as the argument, not action= encoding
        assert!(xml.contains("https://example.com"));
        assert!(!xml.contains("action=link"));
    }

    #[test]
    fn multiple_actions_all_carry_tag_and_group() {
        let mut d = NotificationData {
            title: Some("t".into()),
            tag: Some("t1".into()),
            group: Some("g1".into()),
            ..Default::default()
        };
        d.windows_actions = vec![bg_action("yes", "Yes"), bg_action("no", "No")];
        let xml = build(&d, WindowsVersion::Win10).unwrap();
        // tag=t1 appears in: launch attr (1) + each action's arguments (2) = 3 total
        assert_eq!(
            xml.matches("tag=t1").count(),
            3,
            "tag must appear in launch attr and each action's arguments"
        );
        assert_eq!(
            xml.matches("group=g1").count(),
            3,
            "group must appear in launch attr and each action's arguments"
        );
    }
}
