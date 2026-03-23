// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Runtime Windows version detection and per-version feature gates.
//!
//! The `WindowsVersion` enum models every major Windows release that has a
//! meaningfully different notification API surface.  All feature-gate methods
//! return `false` for unknown/future versions conservatively — but the
//! fallthrough arm in `current()` maps any future 10.0 build to `Win10`,
//! meaning new Windows releases will always get the full Win10 feature set
//! unless we add a new variant.

/// Every Windows release tier relevant to the notification API surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WindowsVersion {
    /// Windows 7 (NT 6.1) — tray balloon via `win7_notifications`.
    Win7,
    /// Windows 8 (NT 6.2) — basic WinRT toast, no adaptive template.
    Win8,
    /// Windows 8.1 (NT 6.3) — basic WinRT toast, `ToastGeneric` available.
    Win81,
    /// Windows 10 RTM → 1909 (builds 10240–19039).
    /// Full adaptive toast XML, actions, inputs, hero image, tag/group.
    /// No progress bar (added in build 15063 but we gate at 19041 for safety).
    Win10Pre19041,
    /// Windows 10 2004+ (build 19041+).
    /// Adds stable progress bar, data-binding updates.
    Win10,
    /// Windows 11 (build 22000+).
    /// Adds Urgent scenario, Urgent priority, `ExpiresOnReboot`.
    Win11,
}

#[allow(dead_code)]
impl WindowsVersion {
    /// Detect the running Windows version at runtime.
    ///
    /// Uses `windows_version::OsVersion` which reads the real version from
    /// `RtlGetVersion` — unlike `GetVersionEx` this is not shimmed by the
    /// compatibility layer, so it always returns the true OS version.
    pub fn current() -> Self {
        let v = windows_version::OsVersion::current();
        match (v.major, v.minor, v.build) {
            // Windows 7
            (6, 1, _) => Self::Win7,
            // Windows 8
            (6, 2, _) => Self::Win8,
            // Windows 8.1
            (6, 3, _) => Self::Win81,
            // Windows 11 (must come before Win10 — same major.minor)
            (10, 0, b) if b >= 22000 => Self::Win11,
            // Windows 10 2004+
            (10, 0, b) if b >= 19041 => Self::Win10,
            // Windows 10 RTM – 1909
            (10, 0, _) => Self::Win10Pre19041,
            // Future Windows: treat as Win10 to get full feature set
            _ => Self::Win10,
        }
    }

    // ── Feature gates ────────────────────────────────────────────────────
    //
    // Each method answers one question: "is this feature safe to use on the
    // currently running version?"  Call sites gate XML construction and WinRT
    // API calls through these rather than sprinkling version comparisons
    // everywhere.

    /// WinRT `ToastNotificationManager` is available.
    /// False only on Windows 7 (uses tray balloons instead).
    #[inline]
    pub fn has_winrt_toast(&self) -> bool {
        *self >= Self::Win8
    }

    /// `ToastGeneric` template with arbitrary text/image layout.
    /// Available from Windows 8.1; Win8 only has legacy named templates.
    #[inline]
    pub fn has_toast_generic(&self) -> bool {
        *self >= Self::Win81
    }

    /// `<action>` elements (interactive buttons) in toast XML.
    #[inline]
    pub fn has_actions(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `<input type="text">` quick-reply input.
    #[inline]
    pub fn has_text_input(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `<input type="selection">` dropdown input.
    #[inline]
    pub fn has_selection_input(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `<image placement="hero">` hero banner image.
    #[inline]
    pub fn has_hero_image(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `<image placement="appLogoOverride">` custom app logo on the toast.
    #[inline]
    pub fn has_logo_override(&self) -> bool {
        *self >= Self::Win81
    }

    /// `ToastNotification::SetTag` / `SetGroup` and the history management
    /// API (`ToastNotificationManager::History`).
    #[inline]
    pub fn has_tag_group(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `ToastNotification::SetExpirationTime`.
    #[inline]
    pub fn has_expiry(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `scenario` attribute on `<toast>` element
    /// (`alarm`, `reminder`, `incomingCall`).
    #[inline]
    pub fn has_scenario(&self) -> bool {
        *self >= Self::Win10Pre19041
    }

    /// `<progress>` element for in-toast progress bars.
    /// Technically added in build 15063 but we require 19041 for the stable
    /// data-binding update path.
    #[inline]
    pub fn has_progress(&self) -> bool {
        *self >= Self::Win10
    }

    /// `scenario="urgent"` and `ToastNotificationPriority::High` used as
    /// Urgent (proper Urgent enum value arrives in Windows App SDK, not WinRT).
    #[inline]
    pub fn has_urgent(&self) -> bool {
        *self >= Self::Win11
    }

    /// `ToastNotification::ExpiresOnReboot` property.
    #[inline]
    pub fn has_expires_on_reboot(&self) -> bool {
        *self >= Self::Win11
    }

    /// `UserNotificationListener` — reading other apps' notifications.
    /// Works on Win10+ but only for MSIX-packaged apps.
    #[inline]
    pub fn has_notification_listener(&self) -> bool {
        *self >= Self::Win10
    }

    /// COM-based background activation via `INotificationActivationCallback`.
    /// Available Win8+; on Win7 there is no toast to activate from.
    #[inline]
    pub fn has_background_activation(&self) -> bool {
        *self >= Self::Win8
    }

    /// Human-readable label for logging / diagnostics.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Win7 => "Windows 7",
            Self::Win8 => "Windows 8",
            Self::Win81 => "Windows 8.1",
            Self::Win10Pre19041 => "Windows 10 (pre-2004)",
            Self::Win10 => "Windows 10 (2004+)",
            Self::Win11 => "Windows 11",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WindowsVersion;

    // Every version in ascending order — used in several tests below.
    const ALL: [WindowsVersion; 6] = [
        WindowsVersion::Win7,
        WindowsVersion::Win8,
        WindowsVersion::Win81,
        WindowsVersion::Win10Pre19041,
        WindowsVersion::Win10,
        WindowsVersion::Win11,
    ];

    // ── Ordering ─────────────────────────────────────────────────────────

    #[test]
    fn ordering_is_strictly_ascending() {
        for w in ALL.windows(2) {
            assert!(
                w[0] < w[1],
                "{} must be < {}",
                w[0].display_name(),
                w[1].display_name()
            );
        }
    }

    #[test]
    fn equality_holds() {
        for v in ALL {
            assert_eq!(v, v);
        }
    }

    // ── display_name ─────────────────────────────────────────────────────

    #[test]
    fn display_names_are_non_empty() {
        for v in ALL {
            assert!(
                !v.display_name().is_empty(),
                "{v:?} must have non-empty display name"
            );
        }
    }

    #[test]
    fn display_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for v in ALL {
            assert!(
                seen.insert(v.display_name()),
                "duplicate display_name for {v:?}"
            );
        }
    }

    // ── has_winrt_toast — boundary: Win8 ─────────────────────────────────

    #[test]
    fn win7_has_no_winrt_toast() {
        assert!(!WindowsVersion::Win7.has_winrt_toast());
    }

    #[test]
    fn win8_and_above_have_winrt_toast() {
        for v in ALL.iter().skip(1) {
            assert!(
                v.has_winrt_toast(),
                "{} should have WinRT toast",
                v.display_name()
            );
        }
    }

    // ── has_toast_generic — boundary: Win81 ──────────────────────────────

    #[test]
    fn toast_generic_absent_on_win7_and_win8() {
        assert!(!WindowsVersion::Win7.has_toast_generic());
        assert!(!WindowsVersion::Win8.has_toast_generic());
    }

    #[test]
    fn toast_generic_present_from_win81() {
        for v in [
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
            WindowsVersion::Win10,
            WindowsVersion::Win11,
        ] {
            assert!(v.has_toast_generic(), "{}", v.display_name());
        }
    }

    // ── has_logo_override — boundary: Win81 ──────────────────────────────

    #[test]
    fn logo_override_absent_on_win7_and_win8() {
        assert!(!WindowsVersion::Win7.has_logo_override());
        assert!(!WindowsVersion::Win8.has_logo_override());
    }

    #[test]
    fn logo_override_present_from_win81() {
        for v in [
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
            WindowsVersion::Win10,
            WindowsVersion::Win11,
        ] {
            assert!(v.has_logo_override(), "{}", v.display_name());
        }
    }

    // ── Rich feature cluster — boundary: Win10Pre19041 ───────────────────
    //    has_actions, has_text_input, has_selection_input, has_hero_image,
    //    has_tag_group, has_expiry, has_scenario

    #[test]
    fn rich_features_absent_below_win10pre19041() {
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
        ] {
            assert!(!v.has_actions(), "{}: has_actions", v.display_name());
            assert!(!v.has_text_input(), "{}: has_text_input", v.display_name());
            assert!(
                !v.has_selection_input(),
                "{}: has_selection_input",
                v.display_name()
            );
            assert!(!v.has_hero_image(), "{}: has_hero_image", v.display_name());
            assert!(!v.has_tag_group(), "{}: has_tag_group", v.display_name());
            assert!(!v.has_expiry(), "{}: has_expiry", v.display_name());
            assert!(!v.has_scenario(), "{}: has_scenario", v.display_name());
        }
    }

    #[test]
    fn rich_features_present_from_win10pre19041() {
        for v in [
            WindowsVersion::Win10Pre19041,
            WindowsVersion::Win10,
            WindowsVersion::Win11,
        ] {
            assert!(v.has_actions(), "{}: has_actions", v.display_name());
            assert!(v.has_text_input(), "{}: has_text_input", v.display_name());
            assert!(
                v.has_selection_input(),
                "{}: has_selection_input",
                v.display_name()
            );
            assert!(v.has_hero_image(), "{}: has_hero_image", v.display_name());
            assert!(v.has_tag_group(), "{}: has_tag_group", v.display_name());
            assert!(v.has_expiry(), "{}: has_expiry", v.display_name());
            assert!(v.has_scenario(), "{}: has_scenario", v.display_name());
        }
    }

    // ── has_progress — boundary: Win10 (build 19041) ─────────────────────

    #[test]
    fn progress_absent_below_win10() {
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
        ] {
            assert!(
                !v.has_progress(),
                "{}: must not have progress",
                v.display_name()
            );
        }
    }

    #[test]
    fn progress_present_from_win10() {
        assert!(WindowsVersion::Win10.has_progress());
        assert!(WindowsVersion::Win11.has_progress());
    }

    // ── has_notification_listener — boundary: Win10 (same as progress) ───

    #[test]
    fn notification_listener_absent_below_win10() {
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
        ] {
            assert!(!v.has_notification_listener(), "{}", v.display_name());
        }
    }

    #[test]
    fn notification_listener_present_from_win10() {
        assert!(WindowsVersion::Win10.has_notification_listener());
        assert!(WindowsVersion::Win11.has_notification_listener());
    }

    // ── Win11-exclusive features ──────────────────────────────────────────

    #[test]
    fn urgent_and_expires_on_reboot_absent_on_win10_and_below() {
        for v in [
            WindowsVersion::Win7,
            WindowsVersion::Win8,
            WindowsVersion::Win81,
            WindowsVersion::Win10Pre19041,
            WindowsVersion::Win10,
        ] {
            assert!(!v.has_urgent(), "{}: has_urgent", v.display_name());
            assert!(
                !v.has_expires_on_reboot(),
                "{}: has_expires_on_reboot",
                v.display_name()
            );
        }
    }

    #[test]
    fn win11_has_urgent_and_expires_on_reboot() {
        assert!(WindowsVersion::Win11.has_urgent());
        assert!(WindowsVersion::Win11.has_expires_on_reboot());
    }

    // ── has_background_activation — boundary: Win8 ───────────────────────

    #[test]
    fn background_activation_absent_on_win7() {
        assert!(!WindowsVersion::Win7.has_background_activation());
    }

    #[test]
    fn background_activation_present_from_win8() {
        for v in ALL.iter().skip(1) {
            assert!(v.has_background_activation(), "{}", v.display_name());
        }
    }
}
