// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Plugin-managed notification sound playback via rodio.
//!
//! # What this does
//!
//! When the caller sets `.sound("my_sound.wav")` on a notification, this
//! module resolves the file from the Tauri resource directory, decodes it
//! with `rodio`, and plays it in a dedicated background thread — completely
//! transparent to the caller.
//!
//! The caller never needs to touch rodio directly. The plugin handles:
//!   - Resource path resolution (works in dev and in a production install)
//!   - Output stream + decoder lifecycle
//!   - Background thread so playback never blocks the notification dispatch
//!   - Silent flag — `silent = true` skips playback entirely
//!   - Graceful fallback — if the file cannot be found or decoded, a warning
//!     is logged and notification dispatch continues normally
//!
//! # Resource setup (tauri.conf.json)
//!
//! Add your sound files to `bundle.resources` so Tauri copies them into the
//! installed app's resource directory:
//!
//! ```json
//! {
//!   "bundle": {
//!     "resources": ["sounds/*"]
//!   }
//! }
//! ```
//!
//! Then in your Rust/JS notification builder:
//!
//! ```rust
//! notification()
//!   .builder()
//!   .title("New message")
//!   .sound("notification.wav")   // filename only — resolved from resources
//!   .show()?;
//! ```
//!
//! # Sound name resolution order
//!
//! 1. `data.silent == true` → skip entirely, no sound.
//! 2. `data.sound` is `None` → no custom sound (OS default still plays via
//!    the toast XML's `<audio>` element on Windows).
//! 3. `data.sound` == `"silent"` (case-insensitive) → skip entirely.
//! 4. Sound name ends with `.wav`, `.mp3`, `.ogg`, `.flac` → treated as a
//!    filename; resolved from resources.
//! 5. Otherwise → treated as a ms-winsoundevent name (Windows only, handled
//!    by the toast XML builder, not this module).
//!
//! # Platform notes
//!
//! - **Windows**: Custom `.wav` files are played here via rodio. The toast XML
//!   emits `<audio silent="true"/>` when a custom sound name is detected
//!   (see `xml_builder::build_audio`) so Windows does not *also* play its own
//!   sound. ms-winsoundevent names are left to the toast XML entirely.
//! - **macOS / Linux**: rodio is used directly; there is no toast XML layer.
//! - **Mobile**: this module compiles to a no-op.

use std::io::Cursor;
use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

// ── Public API ────────────────────────────────────────────────────────────────

/// Attempt to play the notification sound described by `sound_name`.
///
/// - Resolves the file from the app's resource directory.
/// - Decodes and plays via rodio in a detached background thread.
/// - Never blocks the caller; never panics — errors are logged as warnings.
/// - Returns immediately; the sound plays asynchronously.
///
/// Call this from `NotificationBuilder::show()` before or after firing the
/// toast, on all desktop platforms.
pub fn play<R: Runtime>(app: &AppHandle<R>, sound_name: &str, silent: bool) {
    if silent {
        return;
    }

    // Normalise: strip leading/trailing whitespace
    let name = sound_name.trim();

    if name.eq_ignore_ascii_case("silent") || name.is_empty() {
        return;
    }

    // ms-winsoundevent names have no extension and no path separator —
    // let the toast XML handle them; rodio cannot play them.
    if !is_file_sound(name) {
        return;
    }

    match resolve_resource(app, name) {
        Some(path) => spawn_playback(path),
        None => {
            log::warn!(
                "[notification] sound file {:?} not found in resources — skipping playback",
                name
            );
        }
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Returns `true` if the sound name should be treated as a file (has a
/// known audio extension), `false` if it looks like a ms-winsoundevent name.
fn is_file_sound(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".wav")
        || lower.ends_with(".mp3")
        || lower.ends_with(".ogg")
        || lower.ends_with(".flac")
}

/// Resolve a resource-relative filename to an absolute path using Tauri's
/// resource resolver.  Returns `None` if the file does not exist.
fn resolve_resource<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<PathBuf> {
    // Tauri's resource_dir() returns the directory where bundled resources live.
    // In dev this is the project root; in production it is the installed
    // resource directory (next to the executable on Windows, inside .app on macOS).
    let resource_dir = app.path().resource_dir().ok()?;

    // Try the name as-is first (works if caller already gives "sounds/foo.wav")
    let direct = resource_dir.join(name);
    if direct.exists() {
        return Some(direct);
    }

    // Also try "sounds/<name>" as a conventional subdirectory
    let in_sounds = resource_dir.join("sounds").join(name);
    if in_sounds.exists() {
        return Some(in_sounds);
    }

    None
}

/// Spawn a detached OS thread to load and play the file at `path`.
///
/// The thread owns the `OutputStream` (which must stay alive for the
/// duration of playback) and the `Sink` (which drives the decoder).
/// When playback finishes the thread exits and both are dropped cleanly.
fn spawn_playback(path: PathBuf) {
    std::thread::Builder::new()
        .name("notification-sound".to_string())
        .spawn(move || {
            if let Err(e) = play_file(&path) {
                log::warn!("[notification] sound playback failed for {:?}: {}", path, e);
            }
        })
        // If the thread cannot be spawned (extreme resource exhaustion) we log
        // and continue — notification still shows, just without sound.
        .unwrap_or_else(|e| {
            log::warn!("[notification] failed to spawn sound thread: {}", e);
            // Return a dummy JoinHandle — we don't join it anyway
            std::thread::spawn(|| {})
        });
}

/// Load the file at `path`, build a rodio output stream + sink, play the
/// file to completion, and return.
///
/// This runs entirely on the background thread spawned by `spawn_playback`.
fn play_file(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Read the whole file into memory so we don't hold a file descriptor open
    // for the entire playback duration (important on Windows where the path
    // may be inside a locked installer or a network share).
    let bytes = std::fs::read(path)?;

    // Build the output stream. OutputStreamBuilder selects the default output
    // device automatically. The stream must stay alive while the sink plays.
    let stream = rodio::OutputStreamBuilder::open_default_stream()?;
    let sink = rodio::Sink::connect_new(&stream.mixer());

    // Decode from the in-memory buffer.
    let cursor = Cursor::new(bytes);
    let decoder = rodio::Decoder::new(cursor)?;

    sink.append(decoder);

    // Block this thread until playback finishes, then return so the thread exits
    // and the stream is dropped. sleep_until_end() is the correct API —
    // it does not busy-wait.
    sink.sleep_until_end();

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_file_sound ─────────────────────────────────────────────────────

    #[test]
    fn wav_is_file_sound() {
        assert!(is_file_sound("notification.wav"));
        assert!(is_file_sound("ALERT.WAV")); // case-insensitive
    }

    #[test]
    fn mp3_ogg_flac_are_file_sounds() {
        assert!(is_file_sound("ding.mp3"));
        assert!(is_file_sound("ping.ogg"));
        assert!(is_file_sound("chime.flac"));
    }

    #[test]
    fn ms_winsound_event_is_not_file_sound() {
        assert!(!is_file_sound("Notification.Default"));
        assert!(!is_file_sound("Notification.Looping.Alarm"));
        assert!(!is_file_sound("Mail"));
    }

    #[test]
    fn empty_string_is_not_file_sound() {
        assert!(!is_file_sound(""));
    }

    #[test]
    fn extensionless_name_is_not_file_sound() {
        assert!(!is_file_sound("mysound"));
    }

    // ── Guard logic (no AppHandle needed) ────────────────────────────────
    // These test the early-return branches that run before resolve_resource().

    // We cannot call play() in unit tests because we have no AppHandle, but we
    // can verify the guard conditions by testing is_file_sound() and the
    // silent / "silent" string checks directly.

    #[test]
    fn silent_string_skips_file_sound_check() {
        // "silent" is caught before is_file_sound() is evaluated
        assert!(!is_file_sound("silent")); // sanity: not treated as a file
                                           // Also check case variants
        assert!(!is_file_sound("SILENT"));
    }

    #[test]
    fn subdirectory_path_is_file_sound_when_it_has_extension() {
        assert!(is_file_sound("sounds/notification.wav"));
        assert!(is_file_sound("sounds/alert.mp3"));
    }
}
