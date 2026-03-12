// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use serde::{ser::Serializer, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[cfg(mobile)]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),

    // new: Windows-specific error variants
    #[error("Windows API error: {0}")]
    Windows(String),

    #[error(
        "Invalid COM server GUID: '{0}'. Expected format: XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
    )]
    InvalidComGuid(String),

    #[error("COM activator already registered")]
    ComAlreadyRegistered,

    #[error("Failed to dispatch to the main thread")]
    MainThread,

    #[error("Notification Listener is not available (requires MSIX packaging on Windows 10+)")]
    ListenerNotAvailable,
}

// new: bridge windows::core::Error on Windows builds
#[cfg(windows)]
impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Self::Windows(e.to_string())
    }
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Display messages ──────────────────────────────────────────────────

    #[test]
    fn windows_error_display_includes_message() {
        let e = Error::Windows("HRESULT 0x80004005".to_string());
        assert!(e.to_string().contains("HRESULT 0x80004005"));
    }

    #[test]
    fn invalid_com_guid_display_includes_the_guid() {
        let e = Error::InvalidComGuid("bad-guid".to_string());
        let msg = e.to_string();
        assert!(
            msg.contains("bad-guid"),
            "must echo the offending GUID: {msg}"
        );
        assert!(
            msg.contains("XXXXXXXX-XXXX"),
            "must show expected format: {msg}"
        );
    }

    #[test]
    fn com_already_registered_display_is_non_empty() {
        assert!(!Error::ComAlreadyRegistered.to_string().is_empty());
    }

    #[test]
    fn main_thread_error_display_is_non_empty() {
        assert!(!Error::MainThread.to_string().is_empty());
    }

    #[test]
    fn listener_not_available_display_mentions_msix() {
        let msg = Error::ListenerNotAvailable.to_string();
        assert!(
            msg.contains("MSIX"),
            "should mention MSIX in the message: {msg}"
        );
    }

    // ── Serialize ─────────────────────────────────────────────────────────

    #[test]
    fn error_serializes_to_its_display_string() {
        let e = Error::Windows("test error".to_string());
        let json = serde_json::to_string(&e).unwrap();
        // Serialized form is a JSON string of the Display output
        assert!(json.contains("test error"), "serialized: {json}");
    }

    #[test]
    fn invalid_com_guid_serializes_to_string() {
        let e = Error::InvalidComGuid("not-a-guid".to_string());
        let json = serde_json::to_string(&e).unwrap();
        // Must be a JSON string (starts/ends with `"`)
        assert!(
            json.starts_with('"') && json.ends_with('"'),
            "must serialize to JSON string: {json}"
        );
        assert!(json.contains("not-a-guid"));
    }

    // ── From<windows::core::Error> (Windows-only) ─────────────────────────

    #[cfg(windows)]
    #[test]
    fn from_windows_core_error_produces_windows_variant() {
        // Construct a windows::core::Error from a known HRESULT
        use windows::core::Error as WinError;
        let win_err = WinError::from_hresult(windows::Win32::Foundation::E_FAIL);
        let our_err = Error::from(win_err);
        assert!(matches!(our_err, Error::Windows(_)));
        assert!(!our_err.to_string().is_empty());
    }
}
