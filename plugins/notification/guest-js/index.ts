// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

/**
 * Send toast notifications (brief auto-expiring OS window element) to your user.
 * Can also be used with the Notification Web API.
 *
 * ## Windows-specific features (Win 10+)
 * - Interactive action buttons (`windowsActions`)
 * - Inline text and selection inputs (`windowsInputs`)
 * - Hero images, app logo override (`heroImage`, `icon`)
 * - Progress bars (`progress`) — Win 10 build 19041+
 * - Scenarios: alarm, reminder, incomingCall, urgent (`scenario`)
 * - Notification history: tag/group, expiry, expiresOnReboot
 * - Background COM activation — set `comServerGuid` in tauri.conf.json
 *
 * @module
 */

import {
  addPluginListener,
  invoke,
  type PluginListener
} from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type { PermissionState } from '@tauri-apps/api/core'

// ─────────────────────────────────────────────────────────────────────────────
// Upstream types (unchanged from original)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Options to send a notification.
 * @since 2.0.0
 */
interface Options {
  /** The notification identifier to reference this object later. Must be a 32-bit integer. */
  id?: number
  /**
   * Identifier of the {@link Channel} that delivers this notification.
   * If the channel does not exist, the notification won't fire.
   */
  channelId?: string
  /** Notification title. */
  title: string
  /** Optional notification body. */
  body?: string
  /** Schedule this notification to fire on a later time or a fixed interval. */
  schedule?: Schedule
  /**
   * Multiline text. Changes the notification style to big text.
   * Cannot be used with `inboxLines`.
   */
  largeBody?: string
  /** Detail text for the notification with `largeBody`, `inboxLines` or `groupSummary`. */
  summary?: string
  /** Defines an action type for this notification. */
  actionTypeId?: string
  /**
   * Identifier used to group multiple notifications.
   * https://developer.apple.com/documentation/usernotifications/unmutablenotificationcontent/1649872-threadidentifier
   */
  group?: string
  /** Instructs the system that this notification is the summary of a group on Android. */
  groupSummary?: boolean
  /**
   * The sound resource name or file path for the notification.
   * - macOS: system sounds (e.g. "Ping") or sound files in the app bundle
   * - Linux: XDG theme sounds (e.g. "message-new-instant") or file paths
   * - Windows: suffix of a ms-winsoundevent name e.g. "Mail", "Reminder", "Looping.Alarm"
   * - Mobile: resource names
   */
  sound?: string
  /**
   * List of lines to add to the notification. Changes the style to inbox.
   * Cannot be used with `largeBody`. Only supports up to 5 lines.
   */
  inboxLines?: string[]
  /**
   * Notification icon.
   * On Android the icon must be placed in the app's `res/drawable` folder.
   * On Windows 8.1+, used as the `appLogoOverride` (circle-cropped, top-left).
   */
  icon?: string
  /** Notification large icon (Android). Must be in `res/drawable`. */
  largeIcon?: string
  /** Icon color on Android. */
  iconColor?: string
  /** Notification attachments. */
  attachments?: Attachment[]
  /** Extra payload to store in the notification. */
  extra?: Record<string, unknown>
  /**
   * If true, the notification cannot be dismissed by the user on Android.
   * Typically used for background tasks (e.g. a file download).
   */
  ongoing?: boolean
  /** Automatically cancel the notification when the user clicks on it. */
  autoCancel?: boolean
  /** Changes the notification presentation to be silent on iOS (no badge, no sound, not listed). */
  silent?: boolean
  /** Notification visibility (Android). */
  visibility?: Visibility
  /** Sets the number of items this notification represents on Android. */
  number?: number

  // ── Windows-specific fields ──────────────────────────────────────────────
  // These are ignored on iOS, Android, and macOS.

  /**
   * Notification tag for the WinRT history/update API (Windows 10+).
   * Combined with `group` to uniquely identify a toast for in-place updates.
   */
  tag?: string
  /**
   * Full-width hero image at the top of the toast (Windows 10+).
   * Provide a local file path or a `https://` URL.
   */
  heroImage?: string
  /**
   * Interactive action buttons (Windows 10+). Up to 5 buttons supported.
   * Clicks fire `notification://action`.
   */
  windowsActions?: WindowsAction[]
  /**
   * Text or selection inputs embedded in the toast (Windows 10+).
   * WinRT requires inputs to appear before buttons — the plugin handles this automatically.
   */
  windowsInputs?: WindowsInput[]
  /**
   * Progress bar state (Windows 10 build 19041+).
   * Re-send with the same `tag` + `group` to update the bar in-place.
   */
  progress?: WindowsProgress
  /** Auto-remove from the Action Center after this many milliseconds (Windows 10+). */
  expiryMs?: number
  /**
   * System-level presentation scenario (Windows 10+).
   * `'urgent'` requires Windows 11 — falls back to `'reminder'` on Win 10.
   */
  scenario?: WindowsScenario
  /**
   * Delivery priority (Windows 10+).
   * `'urgent'` requires Windows 11 — falls back to `'high'` on Win 10.
   */
  priority?: WindowsPriority
  /** Remove from the Action Center on next reboot (Windows 11+). */
  expiresOnReboot?: boolean
  /**
   * Enable background COM activation for this notification (Windows 8+).
   * Requires `comServerGuid` to be set in `tauri.conf.json`.
   */
  backgroundActivation?: boolean
}

interface ScheduleInterval {
  year?: number
  month?: number
  day?: number
  /**
   * 1 - Sunday, 2 - Monday, 3 - Tuesday, 4 - Wednesday,
   * 5 - Thursday, 6 - Friday, 7 - Saturday
   */
  weekday?: number
  hour?: number
  minute?: number
  second?: number
}

enum ScheduleEvery {
  Year = 'year',
  Month = 'month',
  TwoWeeks = 'twoWeeks',
  Week = 'week',
  Day = 'day',
  Hour = 'hour',
  Minute = 'minute',
  /** Not supported on iOS. */
  Second = 'second'
}

class Schedule {
  at: { date: Date; repeating: boolean; allowWhileIdle: boolean } | undefined
  interval: { interval: ScheduleInterval; allowWhileIdle: boolean } | undefined
  every:
    | { interval: ScheduleEvery; count: number; allowWhileIdle: boolean }
    | undefined

  static at(date: Date, repeating = false, allowWhileIdle = false): Schedule {
    return {
      at: { date, repeating, allowWhileIdle },
      interval: undefined,
      every: undefined
    }
  }

  static interval(
    interval: ScheduleInterval,
    allowWhileIdle = false
  ): Schedule {
    return {
      at: undefined,
      interval: { interval, allowWhileIdle },
      every: undefined
    }
  }

  static every(
    kind: ScheduleEvery,
    count: number,
    allowWhileIdle = false
  ): Schedule {
    return {
      at: undefined,
      interval: undefined,
      every: { interval: kind, count, allowWhileIdle }
    }
  }
}

/** Attachment of a notification. */
interface Attachment {
  /** Attachment identifier. */
  id: string
  /** Attachment URL. Accepts the `asset` and `file` protocols. */
  url: string
}

interface Action {
  id: string
  title: string
  requiresAuthentication?: boolean
  foreground?: boolean
  destructive?: boolean
  input?: boolean
  inputButtonTitle?: string
  inputPlaceholder?: string
}

interface ActionType {
  /** The identifier of this action type. */
  id: string
  /** The list of associated actions. */
  actions: Action[]
  hiddenPreviewsBodyPlaceholder?: string
  customDismissAction?: boolean
  allowInCarPlay?: boolean
  hiddenPreviewsShowTitle?: boolean
  hiddenPreviewsShowSubtitle?: boolean
}

interface PendingNotification {
  id: number
  title?: string
  body?: string
  schedule: Schedule
}

interface ActiveNotification {
  id: number
  tag?: string
  title?: string
  body?: string
  group?: string
  groupSummary: boolean
  data: Record<string, string>
  extra: Record<string, unknown>
  attachments: Attachment[]
  actionTypeId?: string
  schedule?: Schedule
  sound?: string
}

enum Importance {
  None = 0,
  Min,
  Low,
  Default,
  High
}

enum Visibility {
  Secret = -1,
  Private,
  Public
}

interface Channel {
  id: string
  name: string
  description?: string
  sound?: string
  lights?: boolean
  lightColor?: string
  vibration?: boolean
  importance?: Importance
  visibility?: Visibility
}

// ─────────────────────────────────────────────────────────────────────────────
// Windows-specific types (new — ignored on all other platforms)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * How a Windows toast action button activates the app.
 * - `foreground` — brings the app window to focus, then fires `notification://action`
 * - `background` — fires `notification://action` silently in the background
 * - `protocol`   — launches a URI; set `protocol` to the full URI string
 */
type WindowsActionType = 'foreground' | 'background' | 'protocol'

/**
 * Where the action button appears on the toast.
 * - `default`     — visible button row below the text
 * - `contextMenu` — hidden inside the right-click context menu
 */
type WindowsActionPlacement = 'default' | 'contextMenu'

/** An interactive button on a Windows 10+ toast notification. */
interface WindowsAction {
  /** Unique identifier — returned in `NotificationActionEvent.actionId`. */
  id: string
  /** Label shown on the button. */
  label: string
  /** Activation behaviour. Defaults to `'foreground'`. */
  actionType?: WindowsActionType
  /** URI to launch when `actionType` is `'protocol'`. */
  protocol?: string
  /** Optional icon image path shown on the button. */
  icon?: string
  /** Placement of the button. Defaults to `'default'`. */
  placement?: WindowsActionPlacement
  /** Bind this button to an input by input `id`. */
  inputId?: string
}

/** Input type for Windows toast inputs. */
type WindowsInputType = 'text' | 'selection'

/** A single option in a selection/dropdown input. */
interface WindowsSelectionItem {
  id: string
  content: string
}

/** A text or selection input embedded in a Windows 10+ toast notification. */
interface WindowsInput {
  id: string
  placeholder?: string
  inputType?: WindowsInputType
  selections?: WindowsSelectionItem[]
  defaultSelection?: string
}

/**
 * Progress bar state for a Windows 10 build 19041+ toast notification.
 * Re-send with the same `tag` + `group` to update the bar in-place.
 */
interface WindowsProgress {
  /** 0.0–1.0. Any negative value renders an indeterminate spinner. */
  value: number
  title?: string
  status?: string
  valueString?: string
}

/**
 * System-level notification presentation mode (Windows 10+).
 * `'urgent'` requires Windows 11 — falls back to `'reminder'` on Win 10.
 */
type WindowsScenario =
  | 'default'
  | 'alarm'
  | 'reminder'
  | 'incomingCall'
  | 'urgent'

/**
 * Delivery priority for Windows 10+ toast notifications.
 * `'urgent'` requires Windows 11 — falls back to `'high'` on Win 10.
 */
type WindowsPriority = 'default' | 'high' | 'urgent'

/**
 * Payload of the `notification://action` Tauri event (Windows only).
 * Fired on button click, body tap, or inline reply.
 */
interface NotificationActionEvent {
  /**
   * The `id` of the activated `WindowsAction`.
   * Empty string (`''`) = user tapped the toast body (no explicit button).
   */
  actionId: string
  /** Map of input id → value for any inputs attached to this notification. */
  inputs: Record<string, string>
  tag?: string
  group?: string
}

/**
 * A notification currently visible in the Windows Action Center.
 * Only available via the Notification Listener (MSIX + Win 10+).
 */
interface WinActiveNotification {
  id: number
  tag?: string
  group?: string
  title?: string
  body?: string
  appId?: string
}

/**
 * Access status for the Windows Notification Listener.
 * - `allowed`      — access granted
 * - `denied`       — user denied access
 * - `unspecified`  — not yet asked
 * - `notSupported` — not available (non-Windows, Win < 10, or non-MSIX app)
 */
type ListenerAccessStatus =
  | 'allowed'
  | 'denied'
  | 'unspecified'
  | 'notSupported'

// ─────────────────────────────────────────────────────────────────────────────
// Upstream functions (logic preserved exactly from original)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Checks if the permission to send notifications is granted.
 * @since 2.0.0
 */
async function isPermissionGranted(): Promise<boolean> {
  if (window.Notification.permission !== 'default') {
    return await Promise.resolve(window.Notification.permission === 'granted')
  }
  return await invoke('plugin:notification|is_permission_granted')
}

/**
 * Requests the permission to send notifications.
 * @since 2.0.0
 */
async function requestPermission(): Promise<NotificationPermission> {
  return await window.Notification.requestPermission()
}

/**
 * Sends a notification to the user.
 *
 * On Windows, routes through `invoke` directly so the full Options object
 * (including windowsActions, progress, etc.) reaches the Rust backend.
 * On all other platforms, routes through `window.Notification` exactly as
 * the original upstream code does — no behaviour change.
 *
 * @since 2.0.0
 */
function sendNotification(options: Options | string): void {
  // On Windows, we need to invoke directly so the full rich options
  // (windowsActions, windowsInputs, progress, etc.) are serialised and sent
  // to the Rust tier dispatch. window.Notification only passes title+body.
  //
  // __TEMPLATE_windows__ is replaced by Rust at injection time with
  // true or false, so this branch is resolved at runtime per platform.
  //
  // @ts-expect-error __TEMPLATE_windows__ is replaced by Rust before injection
  if (__TEMPLATE_windows__) {
    if (typeof options === 'string') {
      void invoke('plugin:notification|notify', { options: { title: options } })
    } else {
      const frozen = Object.freeze({ ...options })
      void invoke('plugin:notification|notify', { options: frozen })
    }
    return
  }

  // Original upstream path — used on macOS, Linux, iOS, Android
  if (typeof options === 'string') {
    new window.Notification(options)
  } else {
    new window.Notification(options.title, options)
  }
}

/**
 * Register actions that are performed when the user clicks on the notification.
 * @since 2.0.0
 */
async function registerActionTypes(types: ActionType[]): Promise<void> {
  await invoke('plugin:notification|register_action_types', { types })
}

/** Retrieves the list of pending notifications. @since 2.0.0 */
async function pending(): Promise<PendingNotification[]> {
  return await invoke('plugin:notification|get_pending')
}

/** Cancels the pending notifications with the given list of identifiers. @since 2.0.0 */
async function cancel(notifications: number[]): Promise<void> {
  await invoke('plugin:notification|cancel', { notifications })
}

/** Cancels all pending notifications. @since 2.0.0 */
async function cancelAll(): Promise<void> {
  await invoke('plugin:notification|cancel')
}

/** Retrieves the list of active notifications. @since 2.0.0 */
async function active(): Promise<ActiveNotification[]> {
  return await invoke('plugin:notification|get_active')
}

/** Removes the active notifications with the given list of identifiers. @since 2.0.0 */
async function removeActive(
  notifications: Array<{ id: number; tag?: string }>
): Promise<void> {
  await invoke('plugin:notification|remove_active', { notifications })
}

/** Removes all active notifications. @since 2.0.0 */
async function removeAllActive(): Promise<void> {
  await invoke('plugin:notification|remove_active')
}

/** Creates a notification channel (Android). @since 2.0.0 */
async function createChannel(channel: Channel): Promise<void> {
  await invoke('plugin:notification|create_channel', { ...channel })
}

/** Removes the channel with the given identifier (Android). @since 2.0.0 */
async function removeChannel(id: string): Promise<void> {
  await invoke('plugin:notification|delete_channel', { id })
}

/** Retrieves the list of notification channels (Android). @since 2.0.0 */
async function channels(): Promise<Channel[]> {
  return await invoke('plugin:notification|listChannels')
}

/** Listen for notifications received while the app is in the foreground. */
async function onNotificationReceived(
  cb: (notification: Options) => void
): Promise<PluginListener> {
  return await addPluginListener('notification', 'notification', cb)
}

/** Listen for notification action events (mobile). */
async function onAction(
  cb: (notification: Options) => void
): Promise<PluginListener> {
  return await addPluginListener('notification', 'actionPerformed', cb)
}

// ─────────────────────────────────────────────────────────────────────────────
// Windows-specific functions (new — no-op / not called on other platforms)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Remove a single notification from the Windows Action Center by tag + group.
 * No-op on non-Windows or Windows < 10.
 */
async function clearNotification(tag: string, group: string): Promise<void> {
  await invoke('plugin:notification|clear_notification', { tag, group })
}

/**
 * Remove all notifications in a group from the Windows Action Center.
 * No-op on non-Windows or Windows < 10.
 */
async function clearNotificationGroup(group: string): Promise<void> {
  await invoke('plugin:notification|clear_notification_group', { group })
}

/**
 * Remove all notifications posted by this app from the Windows Action Center.
 * No-op on non-Windows or Windows < 10.
 */
async function clearAllNotifications(): Promise<void> {
  await invoke('plugin:notification|clear_all_notifications')
}

/**
 * Request access to read all notifications in the Windows Action Center.
 * Always returns `'notSupported'` for non-MSIX apps and non-Windows platforms.
 */
async function requestListenerAccess(): Promise<ListenerAccessStatus> {
  return invoke('plugin:notification|request_listener_access')
}

/** Get the current Notification Listener access status without prompting. */
async function getListenerAccessStatus(): Promise<ListenerAccessStatus> {
  return invoke('plugin:notification|get_listener_access_status')
}

/**
 * Return all toast notifications currently visible in the Windows Action Center.
 * Returns `[]` on non-Windows, Windows < 10, or when access has not been granted.
 */
async function getActiveNotifications(): Promise<WinActiveNotification[]> {
  return invoke('plugin:notification|get_active_notifications')
}

/**
 * Listen for notification action events (Windows only).
 *
 * Fires on button click, body tap, or inline reply.
 * Returns an unlisten function.
 *
 * @example
 * ```ts
 * const unlisten = await onNotificationAction((event) => {
 *   if (event.actionId === 'reply') {
 *     sendMessage(event.inputs['reply_box'])
 *   }
 * })
 * unlisten()
 * ```
 */
async function onNotificationAction(
  handler: (event: NotificationActionEvent) => void
): Promise<UnlistenFn> {
  return listen<NotificationActionEvent>('notification://action', (event) =>
    handler(event.payload)
  )
}

// ─────────────────────────────────────────────────────────────────────────────
// Exports (matching upstream export shape exactly, with Windows additions)
// ─────────────────────────────────────────────────────────────────────────────

export type {
  Action,
  ActionType,
  ActiveNotification,
  Attachment,
  Channel,
  ListenerAccessStatus,
  NotificationActionEvent,
  Options,
  PendingNotification,
  ScheduleInterval,
  WinActiveNotification,
  // Windows-specific
  WindowsAction,
  WindowsActionPlacement,
  WindowsActionType,
  WindowsInput,
  WindowsInputType,
  WindowsPriority,
  WindowsProgress,
  WindowsScenario,
  WindowsSelectionItem
}

export {
  active,
  cancel,
  cancelAll,
  channels,
  clearAllNotifications,
  // Windows-specific
  clearNotification,
  clearNotificationGroup,
  createChannel,
  getActiveNotifications,
  getListenerAccessStatus,
  // Upstream
  Importance,
  isPermissionGranted,
  onAction,
  onNotificationAction,
  onNotificationReceived,
  pending,
  registerActionTypes,
  removeActive,
  removeAllActive,
  removeChannel,
  requestListenerAccess,
  requestPermission,
  Schedule,
  ScheduleEvery,
  sendNotification,
  Visibility
}
