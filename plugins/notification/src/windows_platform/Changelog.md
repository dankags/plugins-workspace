# Changelog — Windows notification enhancements

All changes relative to the upstream `tauri-apps/plugins-workspace` notification plugin.
Branch: `feat/notification-windows-rich-toasts`

---

## [Unreleased]

### Added — Windows platform

#### COM activation pipeline (new system)

- `windows_platform/com_activator.rs` — full COM server implementation
  - `INotificationActivationCallback` impl (`NotificationActivator`)
  - `IClassFactory` impl (`NotificationActivatorFactory`)
  - `ComGuard` — RAII `CoInitializeEx`/`CoUninitialize` on the correct STA thread
  - `ComRegistration` — RAII `CoRegisterClassObject`/`CoRevokeClassObject`
  - `InstanceGuard` — named Windows mutex prevents multiple background instances
  - `ActivationWatchdog` — hard-kills background process if `Activate()` never arrives
  - `register_foreground()` — dedicated STA thread for foreground COM registration
    (fixes "Registration retries exhausted" caused by Tauri's thread apartment conflict)
  - `write_journal()` — crash-safe append-only activation event log
  - `parse_guid()` — parses `{XXXXXXXX-...}` GUID strings
  - `is_background_activation_launch()` — detects `----BackgroundActivated` argv
  - `plugin_unregister()` — ordered teardown: cancel pump → flush queue → stop worker
    → wait for STA thread to self-clean (correct thread for `CoUninitialize`)
  - `run_background_activation_loop()` — background process entry point

- `windows_platform/activation_bridge.rs` — argument parsing and deep-link handler
  - `ActivationEvent` struct with `ActivationSource` (Background / Foreground / DeepLink)
  - `parse_background_args()` — parses COM `invoked_args` query string
  - `parse_foreground_args()` — parses foreground launch attribute string
  - `parse_activation_uri()` — parses protocol URL (`myapp://notification?...`)
  - `to_action_event()` — converts `ActivationEvent` to `NotificationActionEvent`
  - `register_deep_link_handler()` — wires `tauri-plugin-deep-link` events to
    `notification://action` for foreground path; handles both startup and live URLs
  - `unescape()` — decodes percent-encoding and XML entities
  - `extract_query()` — strips URI scheme/path, returns query string only

- `windows_platform/activation_queue.rs` — encrypted crash-safe activation queue
  - `QueueState` — single mutex owning both `VecDeque<QueuedActivation>` and
    `HashSet<String>` (eliminates all lock-order races)
  - `Condvar`-based worker wake (`WAKE.notify_one()` on every enqueue)
  - Separate `notification-persist` thread with 20ms debounce for coalesced writes
  - AES-256-GCM encryption: key = `SHA-256(guid)`, nonce = 12 random bytes per write
  - Atomic writes: temp file → `fsync` → `rename` (no corrupt state on crash)
  - `load_queue()` — decrypt + deserialize before acquiring any lock
  - `enqueue()` — three-phase no-nested-lock design
  - `flush()` — snapshot inside lock, write outside
  - `start_worker()` / `shutdown_worker()` — lifecycle management

- `windows_platform/action_handler.rs` — event relay and background handler
  - `BACKGROUND_HANDLER: OnceLock<Arc<dyn Fn(NotificationActionEvent)...>>`
  - `register_background_handler()` — called by `NotificationPlugin::build()`
  - `dispatch()` — calls background handler then sends to relay channel
    (both paths are independent; having a handler does not suppress the Tauri event)
  - `start_relay()` — sets `SENDER` synchronously before spawning relay thread
    (fixes startup race where worker could dispatch before `SENDER` was set)
  - Uses blocking `send()` instead of `try_send()` to prevent silent event drops
    under channel backpressure

- `windows_platform/runtime_context.rs`
  - `context()` now returns owned `ActivationContext` clone instead of `MutexGuard`
    (fixes CONTEXT mutex being held across blocking I/O in every caller)

- `windows_platform/shutdown.rs`
  - `signal_worker_complete()` and `wait_for_completion()` use `get_or_init`
    instead of `.get().expect()` — safe regardless of init call order

- `windows_platform/xml_builder.rs` — adaptive toast XML generator
  - `build()` — produces correct adaptive toast XML for Win8.1 through Win11
  - `activationType="background"` on `<toast>` root — emitted only when all actions
    are Background (prevents Foreground buttons from being overridden by root attribute)
  - `activation=bg/fg` hint encoded into every button's `arguments` — consumed by
    `Activate()` for routing, stripped by `activation_bridge` before reaching user code
  - `build_audio()` — emits `<audio silent="true"/>` for file sounds (prevents
    double-play when `rodio` handles playback)
  - Full action button support: `activationType`, `hint-inputId`, `placement`,
    `imageUri`, protocol URI
  - Selection input support with `defaultInput`
  - Progress bar element (Win10 19041+)
  - Hero image element
  - Scenario attribute with graceful fallback for Urgent on Win10
  - Duration="long" for looping alarm/reminder scenarios

- `windows_platform/version.rs` — runtime Windows version detection
  - `WindowsVersion` enum: Win7 / Win8 / Win8.1 / Win10Pre19041 / Win10 / Win11
  - Capability flag methods: `has_actions()`, `has_tag_group()`, `has_expiry()`,
    `has_progress()`, `has_selection_input()`, `has_urgent()`, `has_expires_on_reboot()`

- `windows_platform/sound_player.rs` — cross-platform bundled sound playback
  - `play()` — resolves sound file from Tauri resource directory, plays via `rodio`
    in a detached `notification-sound` thread
  - Respects `silent` flag and "silent" string value
  - Skips `ms-winsoundevent` names (handled by toast XML)
  - `is_file_sound()` — detects file sounds by extension
  - `resolve_resource()` — tries `resource_dir/name` then `resource_dir/sounds/name`

- `windows_platform/notification_listener.rs` — Action Center read API
  - `request_access()` / `get_access_status()` — MSIX only, returns `NotSupported`
    for non-packaged apps
  - `get_all_notifications()` — returns all toasts in Action Center
  - `remove_notification()` / `remove_notification_group()` / `remove_all_notifications()`
  - `is_packaged()` — detects MSIX packaging via `GetCurrentPackageFullName`

- `windows_platform/registry_installer.rs`
  - `install()` — writes `DisplayName`, `IconUri`, `LocalServer32` to registry
  - `write_custom_activator()` — writes `CustomActivator` to AUMID key
    (the missing value that prevented `Activate()` from ever being called)
  - `pick_notification_icon()` — prefers `.ico`, falls back to `.png`, strips
    `\\?\` prefix from `canonicalize()` output

- `windows_platform/shortcut_creator.rs` — Start Menu shortcut with AUMID
- `windows_platform/background_activation.rs` — `run_pump()` (legacy fallback),
  `parse_invoked_args()`, `is_background_activation_launch()`

- `windows_platform/mod.rs` — version-tiered `show()` dispatcher with sound dispatch

#### Public API additions (lib.rs / models.rs)

- `NotificationPlugin<R>` builder (returned by `init()`)
  - `.on_background(f)` — register Rust handler for background activations
  - `.build()` — finalize and return `TauriPlugin`
  - `From<NotificationPlugin<R>> for TauriPlugin<R, PluginConfig>` — allows
    `.plugin(tauri_plugin_notification::init())` without `.build()`

- `NotificationBuilder` — new Windows-specific methods:
  - `.tag(s)` — notification tag for history API
  - `.group(s)` — notification group for history API
  - `.hero_image(src)` — large image above title
  - `.windows_action(action)` — interactive button
  - `.windows_input(input)` — text or selection input
  - `.progress(progress)` — progress bar (Win10 19041+)
  - `.expiry_ms(ms)` — auto-dismiss after N milliseconds
  - `.scenario(s)` — alarm / reminder / incoming call / urgent
  - `.priority(p)` — notification delivery priority
  - `.expires_on_reboot()` — remove on next reboot (Win11+)
  - `.background_activation()` — force COM activation even without buttons

- New types in `models.rs`:
  - `WindowsAction` / `WindowsActionType` / `WindowsActionPlacement`
  - `WindowsInput` / `WindowsInputType` / `WindowsSelectionItem`
  - `WindowsProgress`
  - `WindowsScenario` / `WindowsPriority`
  - `NotificationActionEvent` — payload of `notification://action` Tauri event
  - `WinActiveNotification` — entry in Action Center (Notification Listener)
  - `ListenerAccessStatus`
  - `PluginConfig` with `com_server_guid`
  - `ActivationEvent` / `ActivationSource` in `activation_bridge`

### Fixed

- **`Activate()` never called in foreground process** — `run_background_activation`
  returned immediately for non-background launches, leaving no `CoRegisterClassObject`
  registration. Fixed by `register_foreground()` which spawns a dedicated STA thread.

- **`CustomActivator` registry value missing** — the most critical missing piece.
  Without it Windows displays toasts but silently discards all activations.
  Fixed by `write_custom_activator()` called in every foreground startup.

- **Threading model mismatch crashing COM registration** — `CoRegisterClassObject`
  was called from Tauri's main thread which has a different COM apartment. `validate_threading_model`
  rejected it and `ComGuard` uninitialised immediately, producing the confusing
  log `Registration failed The operation completed successfully.`. Fixed by
  `register_foreground()` which creates a fresh STA thread for COM registration.

- **Background process showing a window** — Tauri opens the main window during
  `setup()` regardless of launch mode. Fixed by calling `window.hide()` on all
  webview windows when `is_background_activation_launch()` is true.

- **`CoUninitialize` called from wrong thread** — `plugin_unregister()` was calling
  `drop(ComRegistration)` from the shutdown thread, which calls `CoUninitialize`.
  COM requires this to run on the STA thread that called `CoInitializeEx`. Fixed:
  `plugin_unregister()` sets `GLOBAL_CANCEL` and waits for the STA thread to
  self-clean.

- **Non-atomic dedup in `enqueue()`** — `QUEUE` and `DEDUP` were two separate
  mutexes. Two threads could both pass the dedup check before either inserted →
  duplicate activations. Fixed: single `QueueState` mutex owns both.

- **Lock-order inversion deadlock** — `enqueue()` locked DEDUP then QUEUE;
  `requeue()` locked QUEUE then DEDUP. Classic ABBA deadlock. Fixed: single lock.

- **I/O inside mutex (Bug D)** — `save_queue()` was called while holding the
  `QUEUE` lock across blocking `fsync` + `rename`. Fixed: clone snapshot inside
  lock, release, write outside.

- **`context()` holding CONTEXT lock across I/O** — `context()` returned a
  `MutexGuard` that callers held across `create_dir_all` + `fs::write`. Fixed:
  `context()` returns owned clone, lock released immediately.

- **Non-atomic file write** — `save_queue()` wrote directly to the final path.
  Crash mid-write produced a corrupt file. Fixed: temp file → `fsync` → `rename`.

- **Dead relay on second `start_relay()` call** — if called twice, the old code
  created a new `(tx, rx)` pair, `SENDER.set()` returned `Err`, the function
  returned but the new `rx` was immediately dropped — killing the channel. Fixed:
  early-return guard checks `SENDER.get().is_some()` before creating the channel.

- **Startup race between `SENDER` and `start_worker()`** — `start_relay()` spawned
  a thread that set `SENDER` asynchronously. If the worker processed a queued
  activation before that thread ran, `SENDER.get()` returned `None` and the event
  was dropped. Fixed: `SENDER.set()` happens synchronously before the relay thread
  is spawned.

- **`try_send()` silently dropping events** — `dispatch()` used `try_send()` which
  fails when the 64-slot channel is full. Fixed: blocking `send()`.

- **`panic` in `Activate()` silently swallowed** — `catch_unwind` returned `E_FAIL`
  with no log. Fixed: downcast the panic payload and log it at ERROR level.

- **`shutdown::init()` never called before `start_worker()`** — `signal_worker_complete()`
  used `.get().expect()` and panicked if `init()` hadn't run. Fixed: `get_or_init`
  in `signal_worker_complete()` + explicit `shutdown::init()` call before `start_worker()`.

- **`IconUri` written with `\\?\` prefix** — `std::fs::canonicalize` prepends
  `\\?\` on Windows. Registry `IconUri` must be a plain path. Fixed: `pick_notification_icon()`
  strips the prefix.

- **`activationType` on `<toast>` root overriding individual button types** — when
  `activationType="background"` was on the root, Foreground buttons were also routed
  through COM and didn't bring the app to focus. Fixed: root attribute only emitted
  when there are no Foreground actions.

- **`activation=bg/fg` hint not stripped from user-facing inputs** — the internal
  routing hint was leaking into `NotificationActionEvent.inputs`. Fixed: removed
  in `activation_bridge::parse_query()`.

- **`OnceLock` panic on second `init_context()` call in tests** — `OnceLock::set()`
  panics if called twice; all tests share process-level statics. Fixed:
  `OnceLock<Mutex<Option<ActivationContext>>>` allows overwrite.

- **`flush()` I/O inside lock** — `flush()` called `save_queue()` while holding
  the QUEUE lock. Fixed: same snapshot pattern as `enqueue()`.

- **`load_queue()` double-lock** — held QUEUE + DEDUP simultaneously. Fixed:
  parse file before acquiring any lock, single STATE lock for in-memory update.

- **`process::exit()` in `graceful_shutdown()` skipping RAII** — `InstanceGuard::drop()`
  and `ComGuard::drop()` never ran, leaking the named mutex and COM apartment.
  Fixed: removed `process::exit(0)`, return normally.

- **`register_with_retry` retrying on STA thread failure** — errors from
  `validate_threading_model` were incorrectly counted as transient registration
  failures. Fixed: threading validation only runs when `guard.initialized_here` is
  true; the dedicated STA thread always satisfies it.

- **`journal_file()` missing `create_dir_all`** — journal writes failed silently
  if called before `queue_file()` had created the directory. Fixed: both functions
  call `ensure_dir()`.

- **Duplicate `start_relay()` call from `desktop::init()`** — `desktop.rs` also
  called `start_relay()`, triggering the "already initialized — skipping" warning.
  Fixed: `desktop::init()` only constructs the `Notification` handle; all Windows
  initialization lives in `lib.rs` `setup()`.

- **`crate::windows_platform::sound_player` reference in `#[cfg(not(windows))]`
  block** — compile error because `windows_platform` does not exist on non-Windows.
  Fixed: `sound_player` exposed as a crate-level module under
  `#[cfg(all(desktop, not(windows)))]`.
