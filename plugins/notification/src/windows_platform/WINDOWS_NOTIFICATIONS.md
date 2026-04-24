# Windows Toast Notifications — Developer Reference

A complete technical reference for the Windows toast notification system built into
`tauri-plugin-notification`. Covers everything from initial setup through advanced
usage patterns, threading model, encryption, and contribution guidelines.

---

## Table of Contents

1. [Overview](#overview)
2. [Prerequisites](#prerequisites)
3. [Configuration](#configuration)
4. [Quick Start](#quick-start)
5. [Notification Features](#notification-features)
   - [Basic Notifications](#basic-notifications)
   - [Action Buttons](#action-buttons)
   - [Inline Reply and Selection Inputs](#inline-reply-and-selection-inputs)
   - [Custom Sounds](#custom-sounds)
   - [Hero Images](#hero-images)
   - [Progress Bars](#progress-bars)
   - [Scenarios and Priority](#scenarios-and-priority)
   - [Expiry and Reboot Behaviour](#expiry-and-reboot-behaviour)
6. [Handling Activation Events](#handling-activation-events)
   - [Frontend (JavaScript / TypeScript)](#frontend-javascript--typescript)
   - [Rust Background Handler](#rust-background-handler)
   - [Activation Sources](#activation-sources)
7. [Background Activation](#background-activation)
   - [What Background Activation Is](#what-background-activation-is)
   - [How It Works End-to-End](#how-it-works-end-to-end)
   - [The Background Handler Pattern](#the-background-handler-pattern)
8. [Windows Version Tiers](#windows-version-tiers)
9. [Architecture Deep Dive](#architecture-deep-dive)
   - [Thread Model](#thread-model)
   - [COM Registration](#com-registration)
   - [Activation Queue](#activation-queue)
   - [Encryption Design](#encryption-design)
   - [Registry Requirements](#registry-requirements)
   - [Toast XML Generation](#toast-xml-generation)
10. [Troubleshooting](#troubleshooting)
11. [Contributing](#contributing)

---

## Overview

This plugin extends the upstream `tauri-plugin-notification` with a full Windows
toast notification stack. It supports every Windows version from Windows 7 through
Windows 11, routing to an appropriate implementation tier for each:

| Windows version        | Tier                              | Features                                       |
| ---------------------- | --------------------------------- | ---------------------------------------------- |
| Windows 7              | `win7_notifications` tray balloon | title, body, icon                              |
| Windows 8              | WinRT `ToastText02`               | title, body                                    |
| Windows 8.1            | WinRT `ToastGeneric`              | title, body, images                            |
| Windows 10 (pre-19041) | Full adaptive XML                 | actions, inputs, audio                         |
| Windows 10 (19041+)    | Full adaptive XML                 | all above + progress bar                       |
| Windows 11             | Full adaptive XML                 | all above + urgent scenario, expires-on-reboot |

The key addition over the upstream plugin is a complete **COM-based activation
pipeline** — every button click, inline reply, and toast body tap is reliably
delivered to your Rust and JavaScript code whether the app is running in the
foreground, running minimised, or completely closed.

---

## Prerequisites

### Cargo.toml

```toml
[dependencies]
tauri-plugin-notification = { path = "../plugins/notification", features = ["deep-link"] }

# Encryption for the activation queue
aes-gcm = "0.10"
sha2    = "0.10"

# Registry access for COM registration
winreg = "0.52"
```

The `deep-link` feature is required for foreground activation via protocol URLs
(body taps and `Foreground` buttons while the app is running).

### Cargo.toml — `[target.'cfg(windows)'.dependencies]`

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_System_Com",
    "Win32_System_Threading",
    "Win32_System_Services",
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_Notifications",
    "Win32_Storage_Packaging_Appx",
    "UI_Notifications",
    "UI_Notifications_Management",
    "Data_Xml_Dom",
    "Foundation",
    "ApplicationModel",
] }
uuid = { version = "1", features = ["v4"] }
rodio = "0.17"
```

### tauri.conf.json

```json
{
  "productName": "My App",
  "identifier": "com.company.myapp",
  "plugins": {
    "notification": {
      "comServerGuid": "{A3B4C5D6-E7F8-9012-ABCD-EF0123456789}"
    },
    "deep-link": {
      "desktop": {
        "schemes": ["myapp"]
      }
    }
  },
  "bundle": {
    "resources": ["sounds/*"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/icon.ico"]
  }
}
```

#### Generating a GUID

The `comServerGuid` must be a stable, unique GUID that never changes after your
app ships — it is the identity of your COM server in the Windows registry. Generate
it once and commit it:

```powershell
# PowerShell
[System.Guid]::NewGuid().ToString("B").ToUpper()
# Output: {A3B4C5D6-E7F8-9012-ABCD-EF0123456789}
```

```bash
# Linux / macOS
uuidgen | tr '[:lower:]' '[:upper:]' | sed 's/^/{/; s/$}/}'
```

### main.rs — plugin registration

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri_plugin_notification::init()
                // Optional: handle background activations in Rust
                .on_background(handle_notification_action)
                .build()
        )
        .run(tauri::generate_context!())
        .expect("error running app");
}

fn handle_notification_action(event: tauri_plugin_notification::NotificationActionEvent) {
    match event.action_id.as_str() {
        ""        => { /* body tap */ }
        "reply"   => { /* send reply: event.inputs["reply_box"] */ }
        "dismiss" => { /* mark as read */ }
        _         => {}
    }
}
```

**Plugin registration order matters** on Windows:

```
single-instance → deep-link → notification
```

`tauri-plugin-single-instance` with the `deep-link` feature enabled is what causes
`on_open_url` to fire on the already-running instance when a foreground activation
arrives. Without it, a second app process would be spawned instead of the URL being
delivered to the running one.

---

## Configuration

### PluginConfig

Configured under `plugins.notification` in `tauri.conf.json`:

| Field           | Type     | Required                  | Description                                                                               |
| --------------- | -------- | ------------------------- | ----------------------------------------------------------------------------------------- |
| `comServerGuid` | `string` | Yes (Windows rich toasts) | COM server GUID. Must be unique per app. Format: `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` |

Example:

```json
{
  "plugins": {
    "notification": {
      "comServerGuid": "{F3A7B2C1-9E4D-4A8F-B6C2-1D5E8F3A7B2C}"
    }
  }
}
```

Without `comServerGuid`, the plugin still works (Win7/Win8 tiers, basic Win10
notifications without action buttons), but COM activation is disabled — button
clicks will not fire `Activate()`.

---

## Quick Start

### Minimal notification

```rust
use tauri_plugin_notification::NotificationExt;

app.notification()
    .builder()
    .title("Hello")
    .body("This is a notification.")
    .show()?;
```

### Notification with a reply button

```rust
use tauri_plugin_notification::{
    NotificationExt, WindowsAction, WindowsActionType, WindowsInput, WindowsInputType,
};

app.notification()
    .builder()
    .title("New message from Alice")
    .body("Hey, are you free tonight?")
    .tag("msg-456")
    .group("messages")
    .windows_input(WindowsInput {
        id: "reply_box".into(),
        placeholder: Some("Type a reply...".into()),
        input_type: WindowsInputType::Text,
        ..Default::default()
    })
    .windows_action(WindowsAction {
        id: "reply".into(),
        label: "Reply".into(),
        action_type: WindowsActionType::Background, // silent, no window focus
        input_id: Some("reply_box".into()),
        ..Default::default()
    })
    .windows_action(WindowsAction {
        id: "open".into(),
        label: "Open".into(),
        action_type: WindowsActionType::Foreground, // brings app to focus
        ..Default::default()
    })
    .show()?;
```

### Listening in JavaScript

```typescript
import { listen } from '@tauri-apps/api/event'

await listen('notification://action', (event) => {
  const { actionId, inputs, tag, group } = event.payload

  if (actionId === '') {
    // Body tap
    console.log('Tapped notification, tag:', tag)
  } else if (actionId === 'reply') {
    const text = inputs['reply_box']
    sendMessage(tag, text)
  } else if (actionId === 'open') {
    openConversation(tag)
  }
})
```

---

## Notification Features

### Basic Notifications

```rust
app.notification()
    .builder()
    .title("Title text")
    .body("Body text")
    .large_body("Extended body shown when the toast is expanded. Supports multiple lines.")
    .summary("Summary line below large body")
    .show()?;
```

### Action Buttons

Action buttons appear at the bottom of the toast. There are three activation types:

| `WindowsActionType` | Behaviour              | When app is open                                    | When app is closed                      |
| ------------------- | ---------------------- | --------------------------------------------------- | --------------------------------------- |
| `Background`        | Silent COM activation  | `notification://action` fired, no focus change      | Background process handles it invisibly |
| `Foreground`        | COM activation + focus | `notification://action` fired, app brought to focus | App relaunched visibly                  |
| `Protocol`          | Opens a URI            | System opens the URI (browser, file manager, etc.)  | Same                                    |

```rust
use tauri_plugin_notification::{WindowsAction, WindowsActionType, WindowsActionPlacement};

// Standard button in the button row
.windows_action(WindowsAction {
    id: "snooze".into(),
    label: "Snooze 5 min".into(),
    action_type: WindowsActionType::Background,
    icon: Some("ms-appx:///Assets/snooze.png".into()), // optional icon
    ..Default::default()
})

// Button in the right-click context menu (not visible in button row)
.windows_action(WindowsAction {
    id: "mute".into(),
    label: "Mute conversation".into(),
    action_type: WindowsActionType::Background,
    placement: WindowsActionPlacement::ContextMenu,
    ..Default::default()
})

// Protocol button — opens a URL without going through Activate()
.windows_action(WindowsAction {
    id: "view-online".into(),
    label: "View online".into(),
    action_type: WindowsActionType::Protocol,
    protocol: Some("https://myapp.com/messages/123".into()),
    ..Default::default()
})
```

**Important:** Always use `WindowsActionType::Background` for buttons that capture
user input (reply text, selections). Only use `Foreground` when you explicitly want
the app window to appear.

### Inline Reply and Selection Inputs

Inputs must be declared **before** action buttons in the builder chain (this matches
WinRT XML ordering requirements).

#### Text input

```rust
use tauri_plugin_notification::{WindowsInput, WindowsInputType};

.windows_input(WindowsInput {
    id: "reply_box".into(),
    placeholder: Some("Type a reply...".into()),
    input_type: WindowsInputType::Text,
    ..Default::default()
})
// Bind the Send button to the input with hint-inputId
.windows_action(WindowsAction {
    id: "send".into(),
    label: "Send".into(),
    action_type: WindowsActionType::Background,
    input_id: Some("reply_box".into()), // renders inline next to text box
    ..Default::default()
})
```

#### Selection (dropdown)

```rust
use tauri_plugin_notification::{WindowsInput, WindowsInputType, WindowsSelectionItem};

.windows_input(WindowsInput {
    id: "priority".into(),
    placeholder: None,
    input_type: WindowsInputType::Selection,
    selections: vec![
        WindowsSelectionItem { id: "high".into(),   content: "High".into() },
        WindowsSelectionItem { id: "normal".into(), content: "Normal".into() },
        WindowsSelectionItem { id: "low".into(),    content: "Low".into() },
    ],
    default_selection: Some("normal".into()),
})
```

When the user submits, `inputs` in the event payload contains the values keyed by
input `id`:

```typescript
const replyText = event.payload.inputs['reply_box']
const priority = event.payload.inputs['priority'] // "high" | "normal" | "low"
```

### Custom Sounds

The plugin plays bundled sound files via `rodio` in a background thread, completely
transparent to the caller.

#### Setup

Add sound files to your `tauri.conf.json` bundle resources:

```json
{
  "bundle": {
    "resources": ["sounds/*"]
  }
}
```

#### Usage

```rust
// Play a bundled file (resolved from the Tauri resource directory)
.sound("notification.wav")
.sound("sounds/alert.mp3")   // also works with subdirectory prefix

// Use a Windows system sound (ms-winsoundevent, handled by toast XML)
.sound("Notification.Default")
.sound("Notification.Looping.Alarm")

// Silent — suppress all sound
.silent()
// Or equivalently:
.sound("silent")
```

#### Sound resolution order

1. `data.silent == true` → no sound played
2. `sound == "silent"` (case-insensitive) → no sound played
3. Sound name has `.wav`, `.mp3`, `.ogg`, or `.flac` extension → resolved from
   `resource_dir/name`, then `resource_dir/sounds/name`, played via `rodio`
4. Otherwise → treated as `ms-winsoundevent` name, handled by the toast XML audio
   element (Windows only)

When a custom file sound is detected, the toast XML emits `<audio silent="true"/>`
so Windows does not double-play its own sound on top of the `rodio` playback.

### Hero Images

A large image displayed at the top of the toast, above the title and body.

```rust
// Absolute path (local file)
.hero_image("C:\\Users\\Alice\\Pictures\\banner.jpg")

// ms-appx path (bundled resource in MSIX packages)
.hero_image("ms-appx:///Assets/hero.jpg")
```

Requires Windows 10+. Ignored on older versions.

### Progress Bars

Display a progress bar within the notification. Requires Windows 10 build 19041+.

```rust
use tauri_plugin_notification::WindowsProgress;

// Indeterminate (animated spinner) — negative value
.tag("download-123")
.group("downloads")
.progress(WindowsProgress {
    value: -1.0,
    title: Some("Downloading file.zip".into()),
    status: Some("Connecting...".into()),
    value_string: None,
})
.show()?;

// Later — update in place by sending with the same tag + group
app.notification()
    .builder()
    .tag("download-123")
    .group("downloads")
    .progress(WindowsProgress {
        value: 0.45,
        title: Some("Downloading file.zip".into()),
        status: Some("45% complete".into()),
        value_string: Some("45%".into()),
    })
    .show()?;
```

The `tag` + `group` combination identifies the notification to update. Windows
replaces the existing notification in place rather than showing a new one.

### Scenarios and Priority

#### Scenario

Controls the visual presentation and audio looping behaviour.

```rust
use tauri_plugin_notification::WindowsScenario;

.scenario(WindowsScenario::Alarm)        // looping audio, no auto-dismiss
.scenario(WindowsScenario::Reminder)     // looping audio, no auto-dismiss
.scenario(WindowsScenario::IncomingCall) // full-screen on locked screen
.scenario(WindowsScenario::Urgent)       // Win11 22000+ only, bypasses Focus Assist
.scenario(WindowsScenario::Default)      // standard behaviour
```

`Urgent` falls back to `Reminder` on Windows 10.

#### Priority

```rust
use tauri_plugin_notification::WindowsPriority;

.priority(WindowsPriority::High)    // high-priority delivery
.priority(WindowsPriority::Urgent)  // Win11 22000+ only
.priority(WindowsPriority::Default)
```

### Expiry and Reboot Behaviour

```rust
// Auto-dismiss from Action Center after 60 seconds
.expiry_ms(60_000)

// Remove from Action Center on next reboot (Win11+ only)
.expires_on_reboot()
```

---

## Handling Activation Events

### Frontend (JavaScript / TypeScript)

Every notification action — button click, inline send, body tap — emits a
`notification://action` event to the Tauri frontend. This works whether the app was
in the foreground or was relaunched from background activation.

```typescript
import { listen } from '@tauri-apps/api/event'

interface NotificationActionEvent {
  actionId: string // "" = body tap, otherwise the action button id
  inputs: Record<string, string> // text/selection input values, keyed by input id
  tag: string | null // notification tag for routing
  group: string | null // notification group
}

const unlisten = await listen<NotificationActionEvent>(
  'notification://action',
  (event) => {
    const { actionId, inputs, tag, group } = event.payload

    switch (actionId) {
      case '':
        // Body tap — navigate to the relevant content
        navigateTo(`/messages/${tag}`)
        break

      case 'reply':
        sendMessage({
          conversationId: tag,
          text: inputs['reply_box']
        })
        break

      case 'dismiss':
        markAsRead(tag)
        break

      case 'snooze':
        snoozeNotification(tag, inputs['snooze_duration'])
        break

      case 'open':
        // Foreground action — app is already focused
        openConversation(tag)
        break
    }
  }
)

// Call unlisten() when the component unmounts to avoid memory leaks
onDestroy(unlisten)
```

### Rust Background Handler

The `.on_background()` handler is called on the worker thread for every notification
action, both in the background process (when app was closed) and in the foreground
process alongside the Tauri event.

```rust
fn handle_notification(event: tauri_plugin_notification::NotificationActionEvent) {
    // action_id is "" for body taps, otherwise the button id you set
    match event.action_id.as_str() {
        "" => {
            // Body tap in background — app will be relaunched by Windows
            log::info!("Toast body tapped, tag={:?}", event.tag);
        }

        "reply" => {
            let text = event.inputs.get("reply_box").map(|s| s.as_str()).unwrap_or("");
            let tag  = event.tag.as_deref().unwrap_or("");

            // Send the reply without opening a window
            if let Err(e) = send_reply_silently(tag, text) {
                log::error!("Reply failed: {e}");
            }
        }

        "dismiss" => {
            mark_notification_read(event.tag.as_deref().unwrap_or(""));
        }

        _ => {
            log::warn!("Unknown action_id: {}", event.action_id);
        }
    }
}

// Register when initializing:
tauri_plugin_notification::init()
    .on_background(handle_notification)
    .build()
```

The handler receives `NotificationActionEvent` by value. It is called
synchronously on the worker thread — for long-running work, spawn a thread inside:

```rust
fn handle_notification(event: tauri_plugin_notification::NotificationActionEvent) {
    std::thread::spawn(move || {
        // long-running work here — does not block the worker
        heavy_operation(event);
    });
}
```

### Activation Sources

The activation event carries a `source` field (accessible in Rust via
`activation_bridge::ActivationEvent`) that identifies how the activation arrived:

| Source       | When                                                                   |
| ------------ | ---------------------------------------------------------------------- |
| `Background` | COM `Activate()` called — app was closed or used a `Background` button |
| `Foreground` | Deep-link URL — app was running, `Foreground` button clicked           |
| `DeepLink`   | Protocol URL via `tauri-plugin-deep-link`                              |

---

## Background Activation

### What Background Activation Is

Background activation is what happens when:

1. The app is **not running**
2. The user clicks a notification button with `activationType="background"`

Windows reads `LocalServer32` from the registry (`HKCU\Software\Classes\CLSID\{GUID}\LocalServer32`),
launches your app executable with `----BackgroundActivated` as a command-line argument,
and calls `INotificationActivationCallback::Activate()` on the registered COM server.

The critical requirement: **the background process must never show a window**. It
receives the activation, runs your handler, and exits silently. The user should not
see any UI flash.

### How It Works End-to-End

```
User clicks "Send Reply" (activationType="background") on a toast
while the app is CLOSED
  │
  ├─ Windows reads registry:
  │    HKCU\Software\Classes\AppUserModelId\com.company.myapp
  │      CustomActivator = {YOUR-GUID}
  │    HKCU\Software\Classes\CLSID\{YOUR-GUID}\LocalServer32
  │      (Default) = C:\...\myapp.exe
  │
  ├─ Windows launches: myapp.exe ----BackgroundActivated
  │
  ├─ Tauri setup() runs:
  │    load_queue()           — restore any crash-survived activations
  │    shutdown::init()
  │    start_worker()         — activation worker thread starts
  │    start_relay()          — relay thread starts (SENDER set synchronously)
  │    spawn("notification-com-pump") thread:
  │      CoInitializeEx(STA)
  │      CoRegisterClassObject({YOUR-GUID}, factory)
  │      run_pump_with_cancel(5000ms)  ← blocks, pumping Win32 messages
  │    hide all windows()    — no UI shown to user
  │    spawn_background_exit_watcher(15s)
  │    return Ok(())
  │
  ├─ Windows delivers COM callback to the pump thread:
  │    Activate(app_id, "action=reply&tag=msg-1&activation=bg", user_inputs)
  │      parse_background_args() → ActivationEvent { action_id="reply", ... }
  │      merge NOTIFICATION_USER_INPUT_DATA → inputs={"reply_box": "Hello"}
  │      enqueue(uuid, NotificationActionEvent)
  │        → AES-256-GCM encrypt → fsync → atomic rename to queue file
  │
  ├─ Worker thread wakes (Condvar):
  │    pop activation from queue
  │    dispatch(event)
  │      ├─ on_background handler(event)   ← YOUR RUST HANDLER FIRES HERE
  │      └─ relay.send(event)              ← no frontend, send drops
  │    mark_dirty() → persist thread writes empty queue
  │    signal_worker_complete()
  │
  ├─ Exit watcher observes worker_complete:
  │    graceful_shutdown()
  │      flush queue
  │      shutdown_worker()
  │      plugin_unregister() → CoRevokeClassObject (on STA thread) → CoUninitialize
  │
  └─ Process exits cleanly, user never saw a window
```

When the app **is running** and the user clicks a `Background` button:

```
User clicks "Send Reply" while the app IS running
  │
  ├─ Windows calls Activate() on the FOREGROUND process's registered COM server
  │    (register_foreground() called this CoRegisterClassObject on startup)
  │
  ├─ Activate() runs:
  │    is_background_activation_launch() == false
  │    → dispatch(event) directly (no queue, no disk write)
  │         ├─ on_background handler(event)     ← fires if registered
  │         └─ relay.send(event)
  │              └─ app.emit("notification://action", event) → JS frontend
  │
  └─ Frontend receives the event, app stays focused (or not, per action type)
```

### The Background Handler Pattern

The handler function signature is:

```
Fn(NotificationActionEvent) + Send + Sync + 'static
```

This means it can be:

#### Plain function pointer (simplest)

```rust
fn handle(event: NotificationActionEvent) { /* ... */ }

tauri_plugin_notification::init()
    .on_background(handle)
    .build()
```

#### Closure capturing shared state via Arc

```rust
let db = Arc::new(Database::open("myapp.db")?);
let config = Arc::new(AppConfig::load()?);

tauri_plugin_notification::init()
    .on_background({
        let db = db.clone();
        let config = config.clone();
        move |event| {
            match event.action_id.as_str() {
                "reply" => {
                    let text = event.inputs.get("reply_box").cloned().unwrap_or_default();
                    db.save_reply(&text, config.user_id());
                }
                _ => {}
            }
        }
    })
    .build()
```

#### Without a handler (JS-only mode)

```rust
// .on_background() is optional — omit it for JS-only activation handling
.plugin(tauri_plugin_notification::init())
// or equivalently:
.plugin(tauri_plugin_notification::init().build())
```

---

## Windows Version Tiers

The plugin detects the Windows version at runtime and routes to the appropriate
implementation:

```
show() called
  └─ WindowsVersion::current()
       ├─ Win7          → tier::win7  (win7_notifications tray balloon)
       ├─ Win8          → tier::win8  (WinRT ToastText02 template)
       ├─ Win8.1        → tier::win81 (WinRT ToastGeneric)
       └─ Win10/Win11   → tier::rich  (full adaptive toast XML)
```

The `tier::rich` path is where all the COM activation, inputs, custom sounds, and
progress bar features live. All other tiers degrade gracefully — they display what
their OS supports and ignore features that are unavailable.

### Feature availability by version

| Feature                   | Win7 | Win8 | Win8.1 | Win10 | Win10 19041+ | Win11 |
| ------------------------- | :--: | :--: | :----: | :---: | :----------: | :---: |
| title + body              |  ✅  |  ✅  |   ✅   |  ✅   |      ✅      |  ✅   |
| hero image                |  ❌  |  ❌  |   ✅   |  ✅   |      ✅      |  ✅   |
| action buttons            |  ❌  |  ❌  |   ❌   |  ✅   |      ✅      |  ✅   |
| text/selection inputs     |  ❌  |  ❌  |   ❌   |  ✅   |      ✅      |  ✅   |
| COM background activation |  ❌  |  ❌  |   ❌   |  ✅   |      ✅      |  ✅   |
| tag + group (history API) |  ❌  |  ❌  |   ❌   |  ✅   |      ✅      |  ✅   |
| expiry time               |  ❌  |  ❌  |   ❌   |  ✅   |      ✅      |  ✅   |
| progress bar              |  ❌  |  ❌  |   ❌   |  ❌   |      ✅      |  ✅   |
| `Urgent` scenario         |  ❌  |  ❌  |   ❌   |  ❌   |      ❌      |  ✅   |
| expires on reboot         |  ❌  |  ❌  |   ❌   |  ❌   |      ❌      |  ✅   |

---

## Architecture Deep Dive

### Thread Model

The plugin creates three permanent background threads and one short-lived one:

```
Main Tauri thread
  └─ setup()
       ├─ spawns: notification-worker
       ├─ spawns: notification-persist
       ├─ spawns: notification-action-relay
       └─ spawns: notification-com-sta (foreground) OR notification-com-pump (background)
```

#### `notification-worker`

Owned by `activation_queue`. Blocks on a `Condvar` waiting for items in the queue.
When `enqueue()` pushes an item it calls `WAKE.notify_one()` — the worker wakes
immediately, pops the item, and calls `action_handler::dispatch()`. No polling,
no fixed sleep interval.

#### `notification-persist`

Owned by `activation_queue`. Wakes when `mark_dirty()` is called (after every
`enqueue`). Waits `PERSIST_DEBOUNCE_MS` (20ms) to coalesce rapid back-to-back
enqueues, then takes one snapshot of the queue and writes it to disk outside any
lock. This separates the hot path (enqueue + wake worker) from the I/O path.

#### `notification-action-relay`

Owned by `action_handler`. Blocks on `mpsc::Receiver`. When `dispatch()` sends
an event, this thread calls `app.emit("notification://action", event)` to deliver
it to the Tauri JS frontend. The channel capacity is 64. If the relay is slow,
`dispatch()` blocks (bounded `send()`) rather than dropping events.

#### `notification-com-sta` (foreground process only)

Owned by `com_activator::register_foreground()`. Initialises a dedicated STA
(Single-Threaded Apartment), calls `CoRegisterClassObject`, then runs a
`PeekMessage`/`DispatchMessage` loop. The STA pump is required because Windows
delivers COM callbacks as messages to the STA thread's message queue — without
it, `Activate()` is never called. Exits when `GLOBAL_CANCEL` is set by
`plugin_unregister()` on `RunEvent::Exit`.

#### `notification-com-pump` (background process only)

Spawned for the background activation process. Calls
`run_background_activation_loop()` which runs the full background activation
pipeline: `InstanceGuard`, `ActivationWatchdog`, `CoRegisterClassObject`, then
`run_pump_with_cancel(5000ms)`. Exits after `Activate()` fires or the 5-second
timeout elapses.

### Cross-thread communication

```
COM callback (STA thread)
  │ dispatch(event)  [blocking send — never drops]
  ▼
mpsc::SyncSender ──────────────────────────────►  notification-action-relay
                                                       │ app.emit()
                                                       ▼
                                                  JS Frontend

COM callback (STA thread) [background process]
  │ enqueue(id, event)
  ▼
QueueState Mutex (single lock owns VecDeque + HashSet)
  │ WAKE.notify_one()
  ▼
notification-worker  ──► dispatch(event) ──► on_background handler
  │ mark_dirty()
  ▼
DIRTY Condvar
  │ (after 20ms debounce)
  ▼
notification-persist  ──► AES-256-GCM encrypt ──► fsync ──► atomic rename
```

### COM Registration

Windows requires four registry values for COM-activated toasts to work. The plugin
writes all four automatically on every foreground startup:

```
HKCU\Software\Classes\AppUserModelId\{aumid}
    DisplayName     REG_SZ  "My App"
    IconUri         REG_SZ  "C:\...\icons\icon.ico"   ← plain path, no \\?\ prefix
    CustomActivator REG_SZ  "{YOUR-GUID}"              ← the critical value

HKCU\Software\Classes\CLSID\{YOUR-GUID}\LocalServer32
    (Default)       REG_SZ  "C:\...\myapp.exe"
```

`CustomActivator` is the value that was historically missing from most implementations.
Without it, Windows displays the notification but never calls `Activate()` — every
button click is silently discarded.

`IconUri` must be a plain absolute path (`C:\...`). The `\\?\` extended-length path
prefix that `std::fs::canonicalize` adds is automatically stripped by
`pick_notification_icon()` in `lib.rs`.

#### Verify your registry

```powershell
reg query "HKCU\Software\Classes\AppUserModelId\com.company.myapp"
# Expected:
#   DisplayName     REG_SZ  My App
#   IconUri         REG_SZ  C:\...\icon.ico      ← no \\?\
#   CustomActivator REG_SZ  {YOUR-GUID}

reg query "HKCU\Software\Classes\CLSID\{YOUR-GUID}\LocalServer32"
# Expected:
#   (Default)       REG_SZ  C:\...\myapp.exe
```

### Activation Queue

The queue provides crash-safe, exactly-once delivery for background activations.

#### Why it exists

In the background activation process, `Activate()` fires on the STA thread and the
worker thread processes it. Between these two events the process could crash, the
watchdog could force-exit, or the system could be low on resources. The queue
persists the activation to disk before processing so that even if the process dies
mid-handler, the activation is not lost — it will be replayed on next launch.

The queue is not used in the foreground process. `Activate()` dispatches directly
to the relay channel, bypassing persistence entirely, because the foreground app
is already running and crash-safe persistence adds latency with no benefit.

#### Queue guarantees

- **Exactly-once**: dedup set (part of `QueueState`) prevents the same activation
  UUID from being processed twice, even across restarts
- **FIFO order**: `VecDeque` preserves insertion order; worker pops from the front
- **Backpressure**: max 512 items; oldest is evicted when full
- **Atomic writes**: temp file → `fsync` → `rename` — a crash mid-write leaves the
  previous file intact
- **No I/O under any lock**: all disk operations happen after the `QueueState` mutex
  is released

#### Encryption

```
Key derivation:   SHA-256(guid_utf8_bytes) → 32-byte AES-256 key
Cipher:           AES-256-GCM (authenticated encryption)
Nonce:            12 random bytes, freshly generated on every write
Wire format:      [12 bytes nonce][N bytes ciphertext + 16 bytes GCM auth tag]
```

The key is derived from the COM server GUID which is embedded in the signed
application binary — the same assumption `tauri-plugin-cache` makes. No OS
keychain access is required. If the file is tampered with, GCM authentication
fails, the file is deleted, and the queue starts fresh.

### Registry Requirements

The toast XML encodes the activation type into each button's `arguments` attribute:

```xml
<toast activationType="background" launch="tag=msg-1&amp;group=chat">
  <visual>...</visual>
  <actions>
    <action
      content="Send Reply"
      arguments="action=reply&amp;tag=msg-1&amp;activation=bg"
      activationType="background"
      hint-inputId="reply_box"/>
    <action
      content="Open"
      arguments="action=open&amp;tag=msg-1&amp;activation=fg"
      activationType="foreground"/>
  </actions>
</toast>
```

The `activation=bg` / `activation=fg` hint is written by `xml_builder` and consumed
by `Activate()` to determine whether to silently handle (bg) or bring the app to
focus (fg). It is stripped from the `inputs` map in `activation_bridge` before the
event reaches your code.

The `activationType="background"` on the `<toast>` root controls body tap behaviour
and is only emitted when there are **no Foreground actions** present. When a
Foreground action exists, the root attribute is omitted so each button's individual
`activationType` is respected.

### Toast XML Generation

The `xml_builder::build()` function produces adaptive toast XML from `NotificationData`.

Complete example of the XML produced for a full-featured notification:

```xml
<toast activationType="background"
       launch="tag=combined-test&amp;group=tests">
  <visual>
    <binding template="ToastGeneric">
      <image placement="hero" src="C:\Windows\Web\Wallpaper\img0.jpg"/>
      <text>Full Featured Toast</text>
      <text>This toast uses every Win10+ feature at once.</text>
      <text placement="attribution">tiktok-clone</text>
    </binding>
  </visual>
  <audio src="ms-winsoundevent:Notification.Default"/>
  <actions>
    <input id="reply" type="text" placeHolderContent="Type something..."/>
    <input id="category" type="selection" defaultInput="normal">
      <selection id="urgent"  content="Urgent"/>
      <selection id="normal"  content="Normal"/>
      <selection id="low"     content="Low priority"/>
    </input>
    <action content="Send Reply"
            arguments="action=send&amp;tag=combined-test&amp;group=tests&amp;activation=bg"
            activationType="background"
            hint-inputId="reply"/>
    <action content="Open"
            arguments="action=open&amp;tag=combined-test&amp;group=tests&amp;activation=fg"
            activationType="foreground"/>
    <action content="Mute conversation"
            arguments="action=mute&amp;tag=combined-test&amp;group=tests&amp;activation=bg"
            activationType="background"
            placement="contextMenu"/>
  </actions>
</toast>
```

---

## Troubleshooting

### `Activate()` is never called

Check in order:

1. **`CustomActivator` in registry**

   ```powershell
   reg query "HKCU\Software\Classes\AppUserModelId\com.company.myapp"
   ```

   You must see `CustomActivator = {YOUR-GUID}`. If absent, the plugin failed to
   write it — check startup logs for `[notification] CustomActivator →`.

2. **GUID mismatch**
   The GUID in `tauri.conf.json` (`comServerGuid`) must exactly match the GUID in
   the registry and in the toast XML. They are all derived from the same config value.

3. **`activationType` missing from buttons**
   All action buttons that should go through COM must have `action_type:
WindowsActionType::Background` (or `Foreground`). `Protocol` buttons do not call
   `Activate()`.

4. **COM registration failed at startup**
   Look for `[notification] COM registration active (foreground)` in your logs. If
   absent, `register_foreground()` failed — check for `Registration failed` entries
   in the journal file at `%LOCALAPPDATA%\{identifier}\{app}\{guid}\tauri_com_activation.log`.

5. **Toast fired with wrong AUMID**
   `CreateToastNotifierWithId` is called with `app.config().identifier`. This must
   match the AUMID registered in `AppUserModelId`. Verify both values are identical.

### Notification appears but buttons are missing

The `has_actions()` version check returns `false` — you are on Windows 8 or 8.1.
Action buttons require Windows 10+. The notification still shows but without the
`<actions>` block.

### Sound plays twice

A custom file sound (`.wav`, `.mp3`, etc.) is being played by both `rodio` and the
toast XML audio element. The `build_audio()` function should emit `<audio
silent="true"/>` when a file sound is detected. Verify the toast XML in logs
contains `<audio silent="true"/>` for file sounds.

### `IconUri` shows `\\?\` prefix

The `canonicalize()` call is producing an extended-length path before `pick_notification_icon()`
strips it. Check that `pick_notification_icon()` is being called and that the log
shows `[notification] resolved icon path: C:\...` (without `\\?\`). If the prefix
persists in the registry, delete the `IconUri` value manually:

```powershell
reg delete "HKCU\Software\Classes\AppUserModelId\com.company.myapp" /v IconUri /f
```

The next app launch will rewrite it with the correct value.

### Background process shows a window briefly

The window hiding in `lib.rs` setup runs after Tauri creates the window. If you see
a brief flash, add `"visible": false` to your window configuration in `tauri.conf.json`:

```json
{
  "windows": [
    {
      "label": "main",
      "visible": false
    }
  ]
}
```

The foreground path shows the window on next launch. The background path hides it
in `setup()` and never shows it again.

### `Registration retries exhausted` in journal

This was a threading model mismatch — fixed by `register_foreground()` spawning a
dedicated STA thread. If you still see this after the fix, the STA thread failed to
start. Check system resources and whether another process holds the named mutex
`Global\Tauri.Notification.COM`.

---

## Contributing

### Repository structure

```
plugins/notification/
├── src/
│   ├── lib.rs                    Plugin init, NotificationPlugin builder
│   ├── models.rs                 All public types (NotificationData, WindowsAction, etc.)
│   ├── error.rs                  Error enum with windows::core::Error bridge
│   ├── desktop.rs                Desktop show() path, non-Windows sound player call
│   ├── commands.rs               Tauri invoke_handler commands
│   └── windows_platform/
│       ├── mod.rs                Version detection, tier routing, sound dispatch
│       ├── xml_builder.rs        Adaptive toast XML generation
│       ├── version.rs            WindowsVersion detection and capability flags
│       ├── com_activator.rs      COM server, Activate(), register_foreground()
│       ├── activation_bridge.rs  URI / arg parsing, deep-link handler
│       ├── activation_queue.rs   Encrypted crash-safe queue, worker, persist threads
│       ├── action_handler.rs     SENDER, dispatch(), start_relay(), BACKGROUND_HANDLER
│       ├── runtime_context.rs    ActivationContext singleton (storage dir, GUID)
│       ├── shutdown.rs           ShutdownCoordinator, exit watcher
│       ├── background_activation.rs  is_background_activation_launch(), run_pump
│       ├── registry_installer.rs    Registry write helpers incl. write_custom_activator
│       ├── shortcut_creator.rs   Start Menu shortcut creation
│       ├── notification_listener.rs  Action Center read API (MSIX only)
│       └── sound_player.rs       rodio-based sound playback for all platforms
```

### Adding a new Windows feature

1. Add fields to `NotificationData` in `models.rs` with `#[serde(default)]`
2. Add the corresponding builder method to `NotificationBuilder` in `lib.rs`
3. Add a capability check to `WindowsVersion` in `version.rs` if needed
4. Consume the field in `xml_builder::build()` — check `ver.has_feature()` before emitting
5. Add a test to `xml_builder.rs` that verifies the XML output for the new feature
6. Document the feature in `WINDOWS_NOTIFICATIONS.md` with the version availability

### Testing

```bash
# Run all unit tests (no Windows required for most)
cargo test -p tauri-plugin-notification

# Run only the Windows-specific tests (requires Windows)
cargo test -p tauri-plugin-notification --target x86_64-pc-windows-msvc

# Test with all features enabled
cargo test -p tauri-plugin-notification --all-features
```

### Design principles

**Lock ordering**: the activation queue uses a single `QueueState` mutex that owns
both the `VecDeque` and the `HashSet`. No nested locks. Any new code that touches
queue state must go through `STATE.lock()` only.

**No I/O under any lock**: all disk operations happen after the `QueueState` mutex
guard is dropped. Snapshot the data inside the lock (`clone()`), release, then write.

**COM threading**: anything that calls `CoInitializeEx`, `CoRegisterClassObject`, or
`CoRevokeClassObject` must do so on a dedicated STA thread it fully owns. Never
call these functions from the Tauri main thread or a Tokio runtime thread.

**`CoUninitialize` must run on the same thread as `CoInitializeEx`**: the
`notification-com-sta` thread owns the `ComGuard` and must be the one to drop it.
`plugin_unregister()` sets `GLOBAL_CANCEL` and waits for the STA thread to self-clean
rather than forcibly dropping `ComRegistration` from the shutdown thread.

**Activation routing**: `Activate()` checks `is_background_activation_launch()`:

- `true` (background process) → `enqueue()` → worker → `dispatch()` → handler
- `false` (foreground process) → `dispatch()` directly → handler + relay

**`activation=bg/fg` hint**: encoded by `xml_builder` into every button's `arguments`
string. Read by `Activate()` to determine routing. Stripped by `activation_bridge`
before the event reaches user code.

### Submitting a pull request

1. Ensure `cargo test` passes on Windows
2. Verify the registry is written correctly after your change with `reg query`
3. Test both foreground (app open) and background (app closed) activation paths
4. Add or update the relevant section in `WINDOWS_NOTIFICATIONS.md`
5. Reference the Tauri issue or RFC that motivated the change in your PR description
