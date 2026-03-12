// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{collections::HashMap, fmt::Display};

use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};

use url::Url;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    id: String,
    url: Url,
}

impl Attachment {
    pub fn new(id: impl Into<String>, url: Url) -> Self {
        Self { id: id.into(), url }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleInterval {
    pub year: Option<u8>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    pub weekday: Option<u8>,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
}

#[derive(Debug)]
pub enum ScheduleEvery {
    Year,
    Month,
    TwoWeeks,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

impl Display for ScheduleEvery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Year => "year",
                Self::Month => "month",
                Self::TwoWeeks => "twoWeeks",
                Self::Week => "week",
                Self::Day => "day",
                Self::Hour => "hour",
                Self::Minute => "minute",
                Self::Second => "second",
            }
        )
    }
}

impl Serialize for ScheduleEvery {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

impl<'de> Deserialize<'de> for ScheduleEvery {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "year" => Ok(Self::Year),
            "month" => Ok(Self::Month),
            "twoweeks" => Ok(Self::TwoWeeks),
            "week" => Ok(Self::Week),
            "day" => Ok(Self::Day),
            "hour" => Ok(Self::Hour),
            "minute" => Ok(Self::Minute),
            "second" => Ok(Self::Second),
            _ => Err(DeError::custom(format!("unknown every kind '{s}'"))),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Schedule {
    #[serde(rename_all = "camelCase")]
    At {
        #[serde(
            serialize_with = "iso8601::serialize",
            deserialize_with = "time::serde::iso8601::deserialize"
        )]
        date: time::OffsetDateTime,
        #[serde(default)]
        repeating: bool,
        #[serde(default)]
        allow_while_idle: bool,
    },
    #[serde(rename_all = "camelCase")]
    Interval {
        interval: ScheduleInterval,
        #[serde(default)]
        allow_while_idle: bool,
    },
    #[serde(rename_all = "camelCase")]
    Every {
        interval: ScheduleEvery,
        count: u8,
        #[serde(default)]
        allow_while_idle: bool,
    },
}

// custom ISO-8601 serialization that does not use 6 digits for years.
mod iso8601 {
    use serde::{ser::Error as _, Serialize, Serializer};
    use time::{
        format_description::well_known::iso8601::{Config, EncodedConfig},
        format_description::well_known::Iso8601,
        OffsetDateTime,
    };

    const SERDE_CONFIG: EncodedConfig = Config::DEFAULT.encode();

    pub fn serialize<S: Serializer>(
        datetime: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        datetime
            .format(&Iso8601::<SERDE_CONFIG>)
            .map_err(S::Error::custom)?
            .serialize(serializer)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationData {
    #[serde(default = "default_id")]
    pub(crate) id: i32,
    pub(crate) channel_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) schedule: Option<Schedule>,
    pub(crate) large_body: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) action_type_id: Option<String>,
    pub(crate) group: Option<String>,
    #[serde(default)]
    pub(crate) group_summary: bool,
    pub(crate) sound: Option<String>,
    #[serde(default)]
    pub(crate) inbox_lines: Vec<String>,
    pub(crate) icon: Option<String>,
    pub(crate) large_icon: Option<String>,
    pub(crate) icon_color: Option<String>,
    #[serde(default)]
    pub(crate) attachments: Vec<Attachment>,
    #[serde(default)]
    pub(crate) extra: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub(crate) ongoing: bool,
    #[serde(default)]
    pub(crate) auto_cancel: bool,
    #[serde(default)]
    pub(crate) silent: bool,

    // new: Windows 10+ fields
    /// Notification tag — composite key with `group` for WinRT history API. Win10+ only.
    pub(crate) tag: Option<String>,
    /// Hero image at the top of the toast. Win10+ only.
    pub(crate) hero_image: Option<String>,
    /// Interactive action buttons. Win10+ only.
    #[serde(default)]
    pub(crate) windows_actions: Vec<WindowsAction>,
    /// Text/selection inputs. Win10+ only.
    #[serde(default)]
    pub(crate) windows_inputs: Vec<WindowsInput>,
    /// Progress bar. Win10 build 19041+ only.
    pub(crate) progress: Option<WindowsProgress>,
    /// Auto-remove after N milliseconds. Win10+ only.
    pub(crate) expiry_ms: Option<u64>,
    /// System-level presentation mode. Win10+ only.
    pub(crate) scenario: Option<WindowsScenario>,

    // new: Windows 11+ fields
    /// Delivery priority. `Urgent` requires Win11+.
    pub(crate) priority: Option<WindowsPriority>,
    /// Remove from Action Center on reboot. Win11+ only.
    #[serde(default)]
    pub(crate) expires_on_reboot: bool,

    // new: background COM activation
    /// Enable background COM activation. Win8+ only.
    #[serde(default)]
    pub(crate) background_activation: bool,
}

fn default_id() -> i32 {
    rand::random()
}

impl Default for NotificationData {
    fn default() -> Self {
        Self {
            id: default_id(),
            channel_id: None,
            title: None,
            body: None,
            schedule: None,
            large_body: None,
            summary: None,
            action_type_id: None,
            group: None,
            group_summary: false,
            sound: None,
            inbox_lines: Vec::new(),
            icon: None,
            large_icon: None,
            icon_color: None,
            attachments: Vec::new(),
            extra: Default::default(),
            ongoing: false,
            auto_cancel: false,
            silent: false,
            tag: None,
            hero_image: None,
            windows_actions: Vec::new(),
            windows_inputs: Vec::new(),
            progress: None,
            expiry_ms: None,
            scenario: None,
            priority: None,
            expires_on_reboot: false,
            background_activation: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingNotification {
    id: i32,
    title: Option<String>,
    body: Option<String>,
    schedule: Schedule,
}

impl PendingNotification {
    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveNotification {
    id: i32,
    tag: Option<String>,
    title: Option<String>,
    body: Option<String>,
    group: Option<String>,
    #[serde(default)]
    group_summary: bool,
    #[serde(default)]
    data: HashMap<String, String>,
    #[serde(default)]
    extra: HashMap<String, serde_json::Value>,
    #[serde(default)]
    attachments: Vec<Attachment>,
    action_type_id: Option<String>,
    schedule: Option<Schedule>,
    sound: Option<String>,
}

impl ActiveNotification {
    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    pub fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }

    pub fn group_summary(&self) -> bool {
        self.group_summary
    }

    pub fn data(&self) -> &HashMap<String, String> {
        &self.data
    }

    pub fn extra(&self) -> &HashMap<String, serde_json::Value> {
        &self.extra
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    pub fn action_type_id(&self) -> Option<&str> {
        self.action_type_id.as_deref()
    }

    pub fn schedule(&self) -> Option<&Schedule> {
        self.schedule.as_ref()
    }

    pub fn sound(&self) -> Option<&str> {
        self.sound.as_deref()
    }
}

#[cfg(mobile)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionType {
    id: String,
    actions: Vec<Action>,
    hidden_previews_body_placeholder: Option<String>,
    custom_dismiss_action: bool,
    allow_in_car_play: bool,
    hidden_previews_show_title: bool,
    hidden_previews_show_subtitle: bool,
}

#[cfg(mobile)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    id: String,
    title: String,
    requires_authentication: bool,
    foreground: bool,
    destructive: bool,
    input: bool,
    input_button_title: Option<String>,
    input_placeholder: Option<String>,
}

#[cfg(target_os = "android")]
pub use android::*;

#[cfg(target_os = "android")]
mod android {
    use serde::{Deserialize, Serialize};
    use serde_repr::{Deserialize_repr, Serialize_repr};

    #[derive(Debug, Clone, Copy, Serialize_repr, Deserialize_repr)]
    #[repr(u8)]
    pub enum Importance {
        None = 0,
        Min = 1,
        Low = 2,
        Default = 3,
        High = 4,
    }

    impl Default for Importance {
        fn default() -> Self {
            Self::Default
        }
    }

    #[derive(Debug, Clone, Copy, Serialize_repr, Deserialize_repr)]
    #[repr(i8)]
    pub enum Visibility {
        Secret = -1,
        Private = 0,
        Public = 1,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Channel {
        id: String,
        name: String,
        description: Option<String>,
        sound: Option<String>,
        lights: bool,
        light_color: Option<String>,
        vibration: bool,
        importance: Importance,
        visibility: Option<Visibility>,
    }

    #[derive(Debug)]
    pub struct ChannelBuilder(Channel);

    impl Channel {
        pub fn builder(id: impl Into<String>, name: impl Into<String>) -> ChannelBuilder {
            ChannelBuilder(Self {
                id: id.into(),
                name: name.into(),
                description: None,
                sound: None,
                lights: false,
                light_color: None,
                vibration: false,
                importance: Default::default(),
                visibility: None,
            })
        }

        pub fn id(&self) -> &str {
            &self.id
        }

        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn description(&self) -> Option<&str> {
            self.description.as_deref()
        }

        pub fn sound(&self) -> Option<&str> {
            self.sound.as_deref()
        }

        pub fn lights(&self) -> bool {
            self.lights
        }

        pub fn light_color(&self) -> Option<&str> {
            self.light_color.as_deref()
        }

        pub fn vibration(&self) -> bool {
            self.vibration
        }

        pub fn importance(&self) -> Importance {
            self.importance
        }

        pub fn visibility(&self) -> Option<Visibility> {
            self.visibility
        }
    }

    impl ChannelBuilder {
        pub fn description(mut self, description: impl Into<String>) -> Self {
            self.0.description.replace(description.into());
            self
        }

        pub fn sound(mut self, sound: impl Into<String>) -> Self {
            self.0.sound.replace(sound.into());
            self
        }

        pub fn lights(mut self, lights: bool) -> Self {
            self.0.lights = lights;
            self
        }

        pub fn light_color(mut self, color: impl Into<String>) -> Self {
            self.0.light_color.replace(color.into());
            self
        }

        pub fn vibration(mut self, vibration: bool) -> Self {
            self.0.vibration = vibration;
            self
        }

        pub fn importance(mut self, importance: Importance) -> Self {
            self.0.importance = importance;
            self
        }

        pub fn visibility(mut self, visibility: Visibility) -> Self {
            self.0.visibility.replace(visibility);
            self
        }

        pub fn build(self) -> Channel {
            self.0
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// New: Windows-specific types
// All types below are new additions — nothing above this line was changed
// beyond the new fields added to NotificationData and its Default impl.
// ════════════════════════════════════════════════════════════════════════════

/// An interactive button on a Windows 10+ toast notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsAction {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub action_type: WindowsActionType,
    pub protocol: Option<String>,
    pub icon: Option<String>,
    #[serde(default)]
    pub placement: WindowsActionPlacement,
    pub input_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindowsActionType {
    #[default]
    Foreground,
    Background,
    Protocol,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowsActionPlacement {
    #[default]
    Default,
    ContextMenu,
}

/// A text or selection input embedded in a Windows 10+ toast notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsInput {
    pub id: String,
    pub placeholder: Option<String>,
    #[serde(default)]
    pub input_type: WindowsInputType,
    #[serde(default)]
    pub selections: Vec<WindowsSelectionItem>,
    pub default_selection: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowsInputType {
    #[default]
    Text,
    Selection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSelectionItem {
    pub id: String,
    pub content: String,
}

/// Progress bar state for a long-running operation notification (Win10 19041+).
/// Re-send with same tag + group to update the bar in place.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsProgress {
    /// 0.0–1.0, or negative for an indeterminate (animated) bar.
    pub value: f32,
    pub title: Option<String>,
    pub status: Option<String>,
    pub value_string: Option<String>,
}

/// System-level notification presentation mode (Win10+).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowsScenario {
    Default,
    Alarm,
    Reminder,
    IncomingCall,
    /// Win11 build 22000+ only; falls back to Reminder on Win10.
    Urgent,
}

/// Notification delivery priority (Win10+).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowsPriority {
    Default,
    High,
    /// Win11 build 22000+ only; falls back to High on Win10.
    Urgent,
}

/// Payload of the `notification://action` Tauri event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationActionEvent {
    pub action_id: String,
    pub inputs: std::collections::HashMap<String, String>,
    pub tag: Option<String>,
    pub group: Option<String>,
}

/// A notification currently visible in the Windows Action Center.
/// Returned by get_active_notifications. MSIX + Win10+ only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinActiveNotification {
    pub id: u32,
    pub tag: Option<String>,
    pub group: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub app_id: Option<String>,
}

/// Notification Listener access permission status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ListenerAccessStatus {
    Allowed,
    Denied,
    Unspecified,
    NotSupported,
}

/// Plugin config from tauri.conf.json → plugins → notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    /// Stable GUID for COM background activation.
    pub com_server_guid: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PluginConfig serde ────────────────────────────────────────────────

    #[test]
    fn plugin_config_empty_object_gives_none_guid() {
        let cfg: PluginConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.com_server_guid.is_none());
    }

    #[test]
    fn plugin_config_parses_com_server_guid() {
        let cfg: PluginConfig =
            serde_json::from_str(r#"{"comServerGuid":"A3B4C5D6-E7F8-9012-ABCD-EF0123456789"}"#)
                .unwrap();
        assert_eq!(
            cfg.com_server_guid.as_deref(),
            Some("A3B4C5D6-E7F8-9012-ABCD-EF0123456789")
        );
    }

    #[test]
    fn plugin_config_null_guid_is_none() {
        let cfg: PluginConfig = serde_json::from_str(r#"{"comServerGuid":null}"#).unwrap();
        assert!(cfg.com_server_guid.is_none());
    }

    #[test]
    fn plugin_config_round_trips() {
        let original = PluginConfig {
            com_server_guid: Some("GUID-XYZ".to_string()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: PluginConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(original.com_server_guid, restored.com_server_guid);
    }

    // ── NotificationData default ──────────────────────────────────────────

    #[test]
    fn notification_data_default_has_sensible_values() {
        let d = NotificationData::default();
        assert!(d.title.is_none());
        assert!(d.body.is_none());
        assert!(d.windows_actions.is_empty());
        assert!(d.windows_inputs.is_empty());
        assert!(d.progress.is_none());
        assert!(!d.silent);
        assert!(!d.expires_on_reboot);
        assert!(!d.background_activation);
    }

    // ── NotificationActionEvent serde ────────────────────────────────────

    #[test]
    fn action_event_round_trips() {
        let mut inputs = HashMap::new();
        inputs.insert("reply_box".to_string(), "Hello".to_string());

        let ev = NotificationActionEvent {
            action_id: "reply".to_string(),
            inputs,
            tag: Some("msg-1".to_string()),
            group: Some("chat".to_string()),
        };

        let json = serde_json::to_string(&ev).unwrap();
        let restored: NotificationActionEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.action_id, "reply");
        assert_eq!(
            restored.inputs.get("reply_box").map(|s| s.as_str()),
            Some("Hello")
        );
        assert_eq!(restored.tag.as_deref(), Some("msg-1"));
        assert_eq!(restored.group.as_deref(), Some("chat"));
    }

    #[test]
    fn action_event_missing_optional_fields_deserializes() {
        let json = r#"{"actionId":"tap","inputs":{}}"#;
        let ev: NotificationActionEvent = serde_json::from_str(json).unwrap();
        assert_eq!(ev.action_id, "tap");
        assert!(ev.tag.is_none());
        assert!(ev.group.is_none());
    }

    // ── WinActiveNotification serde ───────────────────────────────────────

    #[test]
    fn win_active_notification_round_trips() {
        let n = WinActiveNotification {
            id: 42,
            tag: Some("tag-1".to_string()),
            group: Some("grp-1".to_string()),
            title: Some("Hello".to_string()),
            body: Some("World".to_string()),
            app_id: Some("com.example.app".to_string()),
        };
        let json = serde_json::to_string(&n).unwrap();
        let r: WinActiveNotification = serde_json::from_str(&json).unwrap();
        assert_eq!(r.id, 42);
        assert_eq!(r.title.as_deref(), Some("Hello"));
        assert_eq!(r.app_id.as_deref(), Some("com.example.app"));
    }

    // ── ListenerAccessStatus serde ────────────────────────────────────────

    #[test]
    fn listener_access_status_round_trips_all_variants() {
        for status in [
            ListenerAccessStatus::Allowed,
            ListenerAccessStatus::Denied,
            ListenerAccessStatus::Unspecified,
            ListenerAccessStatus::NotSupported,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let r: ListenerAccessStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(r, status);
        }
    }

    // ── WindowsAction serde ───────────────────────────────────────────────

    #[test]
    fn windows_action_type_defaults_to_foreground() {
        let json = r#"{"id":"a","label":"A","placement":"default"}"#;
        let a: WindowsAction = serde_json::from_str(json).unwrap();
        assert_eq!(a.action_type, WindowsActionType::Foreground);
    }

    #[test]
    fn windows_action_background_type_parses() {
        let json = r#"{"id":"a","label":"A","actionType":"background","placement":"default"}"#;
        let a: WindowsAction = serde_json::from_str(json).unwrap();
        assert_eq!(a.action_type, WindowsActionType::Background);
    }

    // ── WindowsProgress serde ─────────────────────────────────────────────

    #[test]
    fn windows_progress_round_trips() {
        let p = WindowsProgress {
            value: 0.75,
            title: Some("Uploading".to_string()),
            status: Some("In progress".to_string()),
            value_string: Some("75%".to_string()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let r: WindowsProgress = serde_json::from_str(&json).unwrap();
        assert!((r.value - 0.75).abs() < f32::EPSILON);
        assert_eq!(r.title.as_deref(), Some("Uploading"));
    }

    // ── WindowsScenario serde ─────────────────────────────────────────────

    #[test]
    fn windows_scenario_all_variants_round_trip() {
        let variants = ["default", "alarm", "reminder", "incomingCall", "urgent"];
        for s in variants {
            let json = format!(r#""{s}""#);
            let v: WindowsScenario = serde_json::from_str(&json).unwrap();
            let back = serde_json::to_string(&v).unwrap();
            assert_eq!(back, json, "round-trip failed for scenario '{s}'");
        }
    }
}
