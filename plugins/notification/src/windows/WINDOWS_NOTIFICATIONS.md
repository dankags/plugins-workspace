# tauri-plugin-notification — Windows Rich Toasts Fork

**Branch:** `feat/notification-windows-rich-toasts`  
**Base:** `tauri-apps/plugins-workspace` v2 (plugin version 2.3.3)  
**Status:** 116/116 tests passing

---

## What this fork adds

The upstream plugin sends a basic `notify-rust` notification on Windows — no
actions, no inputs, no progress bars, no history management, and nothing that
works reliably on Windows 7. This fork replaces that single code path with a
**tiered dispatch system** that detects the runtime Windows version and uses the
richest API available:

| Tier | OS | Backend | Features |
|------|----|---------|---------|
| 1 | Windows 7 | `win7-notifications` tray balloon | Title, body, app icon |
| 2 | Windows 8 | WinRT `ToastText02` template | Title, body, silent audio |
| 3 | Windows 8.1 | WinRT `ToastGeneric` | + App logo override |
| 4 | Windows 10 (pre-2004) | Adaptive XML | + Actions, inputs, hero image, tag/group, expiry, scenario |
| 5 | Windows 10 2004+ | Adaptive XML | + Progress bar |
| 6 | Windows 11 | Adaptive XML | + `urgent` scenario, `ExpiresOnReboot` |

macOS, Linux, iOS, and Android are **completely unchanged** — all new code is
gated behind `#[cfg(windows)]`.

---

## Quick start

### Install from your fork

```toml
# Cargo.toml
[dependencies]
tauri-plugin-notification = {
    git = "https://github.com/YOUR_USERNAME/plugins-workspace",
    branch = "feat/notification-windows-rich-toasts",
    package = "tauri-plugin-notification"
}
```

```sh
cargo update -p tauri-plugin-notification
```

### Register the plugin

```rust
// src-tauri/src/main.rs
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Configure for background activation (Win10+)

Generate a stable UUID for your app (do this once, commit it):

```sh
# PowerShell
[System.Guid]::NewGuid().ToString().ToUpper()
```

Add it to `tauri.conf.json`:

```json
{
  "plugins": {
    "notification": {
      "comServerGuid": "YOUR-STABLE-GUID-HERE"
    }
  }
}
```

> Without a `comServerGuid`, toasts are shown but button clicks and body-taps
> won't fire the `notification://action` event. You only need this for
> interactive notifications.

---

## Sending notifications

### Basic toast (all platforms)

```rust
use tauri_plugin_notification::NotificationExt;

app.notification()
    .builder()
    .title("Hello")
    .body("This works on Win7 through Win11.")
    .show()?;
```

### Silent notification

```rust
app.notification()
    .builder()
    .title("Silent update")
    .body("No sound played.")
    .silent()
    .show()?;
```

### Named Windows sound

Pass the suffix of a `ms-winsoundevent` name:

```rust
app.notification()
    .builder()
    .title("Mail")
    .body("You have a new message.")
    .sound("Mail")   // → ms-winsoundevent:Notification.Mail
    .show()?;
```

Common values: `Default`, `Mail`, `IM`, `Reminder`, `Looping.Alarm`,
`Looping.Call`.

### Hero image (Win10+)

```rust
app.notification()
    .builder()
    .title("Photo shared")
    .hero_image("C:\\Users\\daniel\\Pictures\\preview.png")
    .show()?;
```

### App logo override (Win8.1+)

The `icon()` builder method doubles as the `appLogoOverride` on Windows — it
maps to the round crop in the top-left of the toast:

```rust
app.notification()
    .builder()
    .title("New message")
    .icon("C:\\path\\to\\avatar.png")
    .show()?;
```

### Large body (multi-line)

```rust
app.notification()
    .builder()
    .title("Release notes")
    .large_body("Version 2.1 ships with improved performance, dark mode support, \
                 and over 40 bug fixes. Read the full changelog at...")
    .show()?;
```

---

## Interactive notifications (Win10+)

### Action buttons

```rust
use tauri_plugin_notification::models::{WindowsAction, WindowsActionType, WindowsActionPlacement};

app.notification()
    .builder()
    .title("Build failed")
    .body("The CI pipeline reported 3 errors.")
    .windows_action(WindowsAction {
        id: "view_logs".into(),
        label: "View logs".into(),
        action_type: WindowsActionType::Foreground,  // brings app to focus
        ..Default::default()
    })
    .windows_action(WindowsAction {
        id: "dismiss".into(),
        label: "Dismiss".into(),
        action_type: WindowsActionType::Background,  // handled silently via COM
        ..Default::default()
    })
    .show()?;
```

**Action types:**

| Type | Behaviour |
|------|-----------|
| `Foreground` | Activates the app window, fires `notification://action` |
| `Background` | Fires `notification://action` without bringing app to foreground |
| `Protocol` | Launches a URI — set `protocol: Some("https://...")` |

**Context menu placement** (right-click menu on the toast):

```rust
WindowsAction {
    id: "unsubscribe".into(),
    label: "Turn off notifications".into(),
    action_type: WindowsActionType::Background,
    placement: WindowsActionPlacement::ContextMenu,
    ..Default::default()
}
```

### Text input (quick reply)

```rust
use tauri_plugin_notification::models::{WindowsInput, WindowsInputType};

app.notification()
    .builder()
    .title("New message from Alice")
    .body("Hey, are you free tonight?")
    .windows_input(WindowsInput {
        id: "reply_box".into(),
        placeholder: Some("Type a reply...".into()),
        input_type: WindowsInputType::Text,
        ..Default::default()
    })
    .windows_action(WindowsAction {
        id: "send".into(),
        label: "Send".into(),
        action_type: WindowsActionType::Background,
        input_id: Some("reply_box".into()),  // binds button to the input box
        ..Default::default()
    })
    .show()?;
```

The text the user typed arrives in the `notification://action` event payload
under `inputs["reply_box"]`.

### Selection / dropdown input (Win10+)

```rust
use tauri_plugin_notification::models::{WindowsInput, WindowsInputType, WindowsSelectionItem};

app.notification()
    .builder()
    .title("Snooze reminder")
    .windows_input(WindowsInput {
        id: "snooze_duration".into(),
        input_type: WindowsInputType::Selection,
        selections: vec![
            WindowsSelectionItem { id: "5".into(),  content: "5 minutes".into() },
            WindowsSelectionItem { id: "15".into(), content: "15 minutes".into() },
            WindowsSelectionItem { id: "60".into(), content: "1 hour".into() },
        ],
        default_selection: Some("15".into()),
        ..Default::default()
    })
    .windows_action(WindowsAction {
        id: "snooze".into(),
        label: "Snooze".into(),
        action_type: WindowsActionType::Background,
        input_id: Some("snooze_duration".into()),
        ..Default::default()
    })
    .show()?;
```

### Action with icon

```rust
WindowsAction {
    id: "reply".into(),
    label: "Reply".into(),
    action_type: WindowsActionType::Background,
    icon: Some("C:\\path\\to\\reply_icon.png".into()),
    ..Default::default()
}
```

---

## Progress bar (Win10 build 19041+)

```rust
use tauri_plugin_notification::models::WindowsProgress;

// Show initial progress
app.notification()
    .builder()
    .title("Uploading files")
    .tag("upload-job-1")
    .group("uploads")
    .progress(WindowsProgress {
        value: 0.0,
        title: Some("3 files".into()),
        status: Some("Starting...".into()),
        value_string: Some("0%".into()),
    })
    .show()?;

// Update in-place (same tag + group — replaces the existing toast)
app.notification()
    .builder()
    .title("Uploading files")
    .tag("upload-job-1")
    .group("uploads")
    .progress(WindowsProgress {
        value: 0.65,
        title: Some("3 files".into()),
        status: Some("Uploading...".into()),
        value_string: Some("65%".into()),
    })
    .show()?;
```

Set `value` to any negative number for an indeterminate (animated) bar:

```rust
WindowsProgress {
    value: -1.0,   // indeterminate spinner
    status: Some("Connecting...".into()),
    ..Default::default()
}
```

> Progress bars require the same `tag` + `group` on every update so Windows
> knows which toast to update in place. Without them each call creates a new
> toast.

---

## Scenarios (Win10+)

Scenarios change how Windows presents the toast — overriding Do Not Disturb
for critical alerts, looping audio for alarms, etc.

```rust
use tauri_plugin_notification::models::WindowsScenario;

// Alarm — looping audio, long duration, breaks through focus assist
app.notification()
    .builder()
    .title("Morning alarm")
    .scenario(WindowsScenario::Alarm)
    .show()?;

// Reminder
app.notification()
    .builder()
    .title("Meeting in 5 minutes")
    .scenario(WindowsScenario::Reminder)
    .show()?;

// Incoming call UI
app.notification()
    .builder()
    .title("Alice is calling...")
    .scenario(WindowsScenario::IncomingCall)
    .show()?;

// Urgent — Win11 only; falls back to Reminder on Win10 silently
app.notification()
    .builder()
    .title("Critical system alert")
    .scenario(WindowsScenario::Urgent)
    .show()?;
```

---

## Notification history — tag & group (Win10+)

Use `tag` + `group` to update or remove a specific toast from the Action Center
without clearing everything:

```rust
// Send a tagged notification
app.notification()
    .builder()
    .title("Download complete")
    .tag("dl-42")
    .group("downloads")
    .show()?;
```

Remove it programmatically via the Tauri commands (from JavaScript/TypeScript):

```ts
import { clearNotification, clearNotificationGroup, clearAllNotifications } from '@tauri-apps/plugin-notification';

await clearNotification("dl-42", "downloads");       // one notification
await clearNotificationGroup("downloads");            // all in group
await clearAllNotifications();                        // everything from this app
```

---

## Expiry (Win10+)

Auto-remove a notification from the Action Center after a timeout:

```rust
app.notification()
    .builder()
    .title("Limited-time offer")
    .expiry_ms(30_000)   // remove after 30 seconds
    .show()?;
```

For Win11, you can also clear on next reboot:

```rust
app.notification()
    .builder()
    .title("Restart required")
    .expires_on_reboot()
    .show()?;
```

---

## Listening for action events

Every button click, body tap, and inline reply fires a `notification://action`
Tauri event. Listen for it in your frontend:

```ts
import { listen } from '@tauri-apps/api/event';

await listen('notification://action', (event) => {
    const { actionId, inputs, tag, group } = event.payload;

    if (actionId === 'send') {
        const replyText = inputs['reply_box'];
        console.log('User replied:', replyText);
    }

    if (actionId === 'snooze') {
        const minutes = inputs['snooze_duration'];
        console.log('Snooze for', minutes, 'minutes');
    }

    // Body tap (no explicit button) — actionId is an empty string
    if (actionId === '') {
        console.log('Toast body was tapped');
    }
});
```

Or from Rust using the `Listener` trait:

```rust
use tauri::Listener;
use tauri_plugin_notification::models::NotificationActionEvent;

app.listen("notification://action", |event| {
    if let Ok(payload) = serde_json::from_str::<NotificationActionEvent>(&event.payload()) {
        println!("Action: {}", payload.action_id);
        for (key, val) in &payload.inputs {
            println!("  input {key} = {val}");
        }
    }
});
```

> **Note:** Action events only fire when a `comServerGuid` is configured in
> `tauri.conf.json` and the COM activator is registered. Without it, the toast
> still appears but clicks are not routed back to the app.

---

## Notification Listener (MSIX + Win10+ only)

The Notification Listener lets your app read **all** notifications from the
Action Center, including those from other apps. This requires:

1. Your app is **MSIX-packaged** (not NSIS-installed)
2. Running on **Windows 10+**
3. The user has granted permission

```ts
import {
    requestListenerAccess,
    getListenerAccessStatus,
    getActiveNotifications
} from '@tauri-apps/plugin-notification';

// Request permission (shows system prompt once)
const status = await requestListenerAccess();

if (status === 'allowed') {
    const notifications = await getActiveNotifications();
    for (const n of notifications) {
        console.log(`[${n.appId}] ${n.title}: ${n.body}`);
    }
}

// Check status without prompting
const current = await getListenerAccessStatus();
console.log('Access:', current);  // 'allowed' | 'denied' | 'unspecified' | 'notSupported'
```

> During development (`cargo tauri dev`), `requestListenerAccess` will always
> return `notSupported` — this is expected. It only works in a signed MSIX build.

---

## Architecture deep-dive

### Tier dispatch (`src/windows/mod.rs`)

```
show() called
  └── WindowsVersion::current()   ← reads real OS version via RtlGetVersion
        ├── Win7          → win7::show()   — win7-notifications tray balloon
        ├── Win8          → win8::show()   — WinRT ToastText02
        ├── Win8.1        → win81::show()  — WinRT ToastGeneric
        └── Win10–Win11   → rich::show()  — full adaptive XML + COM activation
```

`WindowsVersion::current()` uses the `windows-version` crate which calls
`RtlGetVersion` directly, bypassing the OS compatibility shim that causes
`GetVersionEx` to lie on Windows 10+.

### XML builder (`src/windows/xml_builder.rs`)

`xml_builder::build(data, version)` produces the adaptive toast XML string.
Every feature is gated through a `WindowsVersion` method — the builder never
hardcodes version comparisons inline. This means adding support for a future
Windows version only requires adding a variant to the enum and updating feature
gate boundaries.

Example output for a full Win10 toast with action and input:

```xml
<toast scenario="reminder" duration="long">
  <visual>
    <binding template="ToastGeneric">
      <image placement="appLogoOverride" src="..." hint-crop="circle"/>
      <text>Meeting in 5 minutes</text>
      <text>Project sync with the team</text>
    </binding>
  </visual>
  <audio src="ms-winsoundevent:Notification.Reminder" loop="false"/>
  <actions>
    <input id="snooze_duration" type="selection" defaultInput="5">
      <selection id="5" content="5 minutes"/>
      <selection id="15" content="15 minutes"/>
    </input>
    <action content="Snooze" arguments="snooze"
            activationType="background" hint-inputId="snooze_duration"/>
    <action content="Dismiss" arguments="dismiss"
            activationType="background"/>
  </actions>
</toast>
```

### COM activator (`src/windows/com_activator.rs`)

Background activation on Windows works through a registered COM class factory.
When a user clicks a toast action, Windows launches the process with
`----BackgroundActivated` and invokes `INotificationActivationCallback::Activate`
on the registered GUID.

We implement the COM vtables **entirely by hand** using `windows-sys` raw FFI,
not the `windows` crate's `#[implement]` proc-macro. The reason: `#[implement]`
expands to code referencing `windows_core` by crate name, which breaks at
compile time when `notify-rust` and `win7-notifications` pull in a different
semver-incompatible version of `windows-core`.

The hand-rolled vtable approach has zero runtime cost and no version conflict:

```
IClassFactory  (static singleton, no heap alloc)
  └── INotificationActivationCallback  (heap-allocated per activation)
        └── Activate() → action_handler::dispatch()
```

`parse_guid()` validates the GUID string before any COM call is made, returning
`Error::InvalidComGuid` immediately for malformed input.

### Action handler (`src/windows/action_handler.rs`)

COM activation callbacks fire on a Windows thread pool thread that has no access
to the Tauri `AppHandle`. The solution is a `std::sync::mpsc::SyncSender`:

```
COM thread           app thread
    │                    │
    │  dispatch(event)   │
    ├──────────────────► SENDER (OnceLock<SyncSender>)
    │                    │
    │           relay thread (owns AppHandle)
    │                    │
    │                    ├── app.emit("notification://action", event)
    │                    │
```

`start_relay()` is called once from `desktop::init()`. `dispatch()` is safe to
call from any thread at any time — if the relay hasn't started yet, the event
is dropped with a warning log rather than panicking.

### Version feature gates (`src/windows/version.rs`)

All 17 feature gate methods and their OS boundaries:

| Method | Boundary |
|--------|----------|
| `has_winrt_toast()` | Win8+ |
| `has_toast_generic()` | Win8.1+ |
| `has_logo_override()` | Win8.1+ |
| `has_background_activation()` | Win8+ |
| `has_actions()` | Win10Pre19041+ |
| `has_text_input()` | Win10Pre19041+ |
| `has_selection_input()` | Win10Pre19041+ |
| `has_hero_image()` | Win10Pre19041+ |
| `has_tag_group()` | Win10Pre19041+ |
| `has_expiry()` | Win10Pre19041+ |
| `has_scenario()` | Win10Pre19041+ |
| `has_notification_listener()` | Win10+ |
| `has_progress()` | Win10 (build 19041)+ |
| `has_urgent()` | Win11+ |
| `has_expires_on_reboot()` | Win11+ |

---

## Files changed vs. upstream

Six upstream files were surgically patched (minimal diff). Six new files were
added under `src/windows/`.

### Patched upstream files

| File | Change summary |
|------|---------------|
| `Cargo.toml` | Added `windows 0.61.2`, `windows-sys 0.61`, `win7-notifications`, `windows-version` dependencies; `windows7-compat` feature kept as no-op shim |
| `src/error.rs` | Added `Windows`, `InvalidComGuid`, `ComAlreadyRegistered`, `MainThread`, `ListenerNotAvailable` error variants; `From<windows::core::Error>` impl |
| `src/models.rs` | Added Windows-specific fields to `NotificationData`; added 9 new types: `WindowsAction`, `WindowsActionType`, `WindowsActionPlacement`, `WindowsInput`, `WindowsInputType`, `WindowsSelectionItem`, `WindowsProgress`, `WindowsScenario`, `WindowsPriority`, `NotificationActionEvent`, `WinActiveNotification`, `ListenerAccessStatus`, `PluginConfig` |
| `src/lib.rs` | Added 10 Windows builder methods to `NotificationBuilder`; added `PluginConfig` COM GUID setup in `init()`; added `on_event` handler to call `unregister()` on exit |
| `src/desktop.rs` | Added Windows dispatch in `NotificationBuilder::show()` behind `#[cfg(windows)]`; calls `action_handler::start_relay()` in `init()` |
| `src/commands.rs` | Added 6 new commands: `clear_notification`, `clear_notification_group`, `clear_all_notifications`, `request_listener_access`, `get_listener_access_status`, `get_active_notifications` |

### New files

| File | Purpose |
|------|---------|
| `src/windows/mod.rs` | Tier dispatch — detects version and routes to correct tier |
| `src/windows/version.rs` | `WindowsVersion` enum with all feature gate methods |
| `src/windows/xml_builder.rs` | Builds adaptive toast XML from `NotificationData` |
| `src/windows/com_activator.rs` | Manual COM vtable implementation for `INotificationActivationCallback` |
| `src/windows/action_handler.rs` | `mpsc` channel bridge from COM callbacks to Tauri events |
| `src/windows/notification_listener.rs` | `UserNotificationListener` API for reading Action Center |

---

## Registry setup for background activation

For the COM activator to work, the app's AUMID and COM CLSID must be registered.
The NSIS installer script must write these keys:

```
HKCU\SOFTWARE\Classes\AppUserModelId\{your.app.id}
  └── (Default) = ""

HKCU\SOFTWARE\Classes\CLSID\{YOUR-GUID}
  └── (Default) = "YourApp.NotificationActivator"

HKCU\SOFTWARE\Classes\CLSID\{YOUR-GUID}\LocalServer32
  └── (Default) = "C:\path\to\your\app.exe"
```

For a Tauri NSIS installer, add this to your `installer.nsi`:

```nsh
WriteRegStr HKCU "SOFTWARE\Classes\AppUserModelId\${IDENTIFIER}" "" ""
WriteRegStr HKCU "SOFTWARE\Classes\CLSID\${COM_GUID}" "" "${PRODUCTNAME} Notification Activator"
WriteRegStr HKCU "SOFTWARE\Classes\CLSID\${COM_GUID}\LocalServer32" "" "$INSTDIR\${MAINBINARYNAME}.exe"
```

Where `${IDENTIFIER}` matches `tauri.conf.json → identifier` and `${COM_GUID}`
matches `plugins.notification.comServerGuid`.

---

## Test suite

116 tests, all passing. Run with:

```sh
cargo test --target x86_64-pc-windows-msvc
```

Tests are inline `#[cfg(test)]` blocks in each module — no external test
harness required:

| Module | Tests | Coverage |
|--------|-------|---------|
| `windows/xml_builder.rs` | 50 | `esc()`, all XML features × all version tiers |
| `windows/version.rs` | 20 | All 17 feature gates, ordering, display names |
| `windows/com_activator.rs` | 24 | GUID parsing (valid + 12 error cases), `register()` error path |
| `windows/action_handler.rs` | 5 | Channel mechanics, `EVENT_NAME`, dispatch before relay |
| `error.rs` | 8 | Display messages, JSON serialization, `From<windows::core::Error>` |
| `models.rs` | 13 | Serde round-trips for all new types |

All tests are pure Rust — no WinRT runtime, no COM server, no live OS required.
The WinRT/COM paths (tier dispatch, `register()` success path, Notification
Listener) are integration-tested by running the built app on a real Windows
machine.

---

## Known limitations

| Limitation | Notes |
|------------|-------|
| Notification Listener requires MSIX | `RequestAccessAsync` returns `Denied` for NSIS-installed apps. No workaround — this is a Windows OS restriction. |
| COM activation requires registry keys | The NSIS installer must write the CLSID keys. See [Registry setup](#registry-setup-for-background-activation). |
| Background activation in dev mode | `cargo tauri dev` doesn't write registry keys. Test background activation in a `cargo tauri build` release build. |
| Progress bar requires Win10 build 19041+ | Silently omitted on older builds. |
| `Urgent` scenario requires Win11 | Falls back to `Reminder` on Win10 silently. |
| `UserNotificationListener` requires MSIX | Will always return `NotSupported` in dev mode or NSIS builds. |

---

## Contributing / upstreaming

The fork was written with upstreaming in mind. The diff against upstream is
intentionally surgical — only the minimum required lines were changed in each
existing file. New functionality lives entirely in the new `src/windows/`
submodule.

To open a PR against upstream `tauri-apps/plugins-workspace`:

1. Fork upstream
2. `git remote add upstream https://github.com/tauri-apps/plugins-workspace`
3. `git fetch upstream`
4. `git rebase upstream/v2`
5. Open a PR from `feat/notification-windows-rich-toasts` → `tauri-apps/plugins-workspace:v2`
