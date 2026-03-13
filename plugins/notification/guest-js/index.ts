// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

/**
 * Send desktop and mobile notifications.
 *
 * ## Windows-specific features
 *
 * On Windows 10+, this plugin supports rich adaptive toasts:
 * - Interactive action buttons (`windowsActions`)
 * - Inline text and selection inputs (`windowsInputs`)
 * - Hero images, app logo override (`heroImage`, `icon`)
 * - Progress bars (`progress`) — Win 10 build 19041+
 * - Scenarios: alarm, reminder, incomingCall, urgent (`scenario`)
 * - Notification history: tag/group, expiry, ExpiresOnReboot
 * - Background COM activation — set `comServerGuid` in tauri.conf.json
 *
 * @module
 */

import type { PermissionState } from '@tauri-apps/api/core'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

// ─────────────────────────────────────────────────────────────────────────────
// Shared / cross-platform types
// ─────────────────────────────────────────────────────────────────────────────

export interface Attachment {
  id: string
  url: string
}

export interface ScheduleInterval {
  year?: number
  month?: number
  day?: number
  weekday?: number
  hour?: number
  minute?: number
  second?: number
}

export type ScheduleEvery =
  | 'year'
  | 'month'
  | 'twoWeeks'
  | 'week'
  | 'day'
  | 'hour'
  | 'minute'
  | 'second'

export type Schedule =
  | {
      kind: 'at'
      date: Date
      repeating?: boolean
      allowWhileIdle?: boolean
    }
  | {
      kind: 'interval'
      interval: ScheduleInterval
      allowWhileIdle?: boolean
    }
  | {
      kind: 'every'
      interval: ScheduleEvery
      count: number
      allowWhileIdle?: boolean
    }

// ─────────────────────────────────────────────────────────────────────────────
// Windows-specific types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * How a Windows toast action button activates the app.
 *
 * - `foreground` — brings the app window to focus, then fires `notification://action`
 * - `background` — fires `notification://action` silently in the background
 * - `protocol`   — launches a URI; set `protocol` to the full URI string
 */
export type WindowsActionType = 'foreground' | 'background' | 'protocol'

/**
 * Where the action button appears on the toast.
 *
 * - `default`     — visible button row below the text
 * - `contextMenu` — hidden inside the right-click context menu
 */
export type WindowsActionPlacement = 'default' | 'contextMenu'

/** An interactive button on a Windows 10+ toast notification. */
export interface WindowsAction {
  /** Unique identifier — returned in `NotificationActionEvent.actionId`. */
  id: string
  /** Label shown on the button. */
  label: string
  /** Activation behaviour. Defaults to `'foreground'`. */
  actionType?: WindowsActionType
  /**
   * URI to launch when `actionType` is `'protocol'`.
   * This becomes the action argument instead of `id`.
   */
  protocol?: string
  /** Optional icon image path shown on the button. */
  icon?: string
  /** Placement of the button. Defaults to `'default'`. */
  placement?: WindowsActionPlacement
  /**
   * Bind this button to an input by input `id`.
   * Creates a quick-reply / snooze button inline with the input field.
   */
  inputId?: string
}

/** Input type for Windows toast inputs. */
export type WindowsInputType = 'text' | 'selection'

/** A single option in a selection/dropdown input. */
export interface WindowsSelectionItem {
  /** Value returned in `NotificationActionEvent.inputs`. */
  id: string
  /** Label shown in the dropdown. */
  content: string
}

/** A text or selection input embedded in a Windows 10+ toast notification. */
export interface WindowsInput {
  /** Unique identifier — used as the key in `NotificationActionEvent.inputs`. */
  id: string
  /** Placeholder text shown in an empty text input. */
  placeholder?: string
  /** Input type. Defaults to `'text'`. */
  inputType?: WindowsInputType
  /** Options for a `'selection'` input. */
  selections?: WindowsSelectionItem[]
  /** Default selected item id for a `'selection'` input. */
  defaultSelection?: string
}

/**
 * Progress bar state for a Windows 10 build 19041+ toast notification.
 *
 * Re-send with the same `tag` + `group` to update the bar in-place
 * inside the Action Center without creating a new toast.
 */
export interface WindowsProgress {
  /**
   * Progress value between 0.0 and 1.0.
   * Any negative value renders an indeterminate (animated) spinner.
   */
  value: number
  /** Title text above the progress bar. */
  title?: string
  /** Status text below the bar (e.g. "Uploading…"). */
  status?: string
  /** Override string displayed in place of the percentage (e.g. "2 / 5 files"). */
  valueString?: string
}

/**
 * System-level notification presentation mode (Windows 10+).
 *
 * - `default`      — standard toast
 * - `alarm`        — looping audio, long duration, breaks focus-assist
 * - `reminder`     — looping audio, long duration, breaks focus-assist
 * - `incomingCall` — ringtone, incoming call UI
 * - `urgent`       — Win 11 only; falls back to `'reminder'` on Win 10
 */
export type WindowsScenario =
  | 'default'
  | 'alarm'
  | 'reminder'
  | 'incomingCall'
  | 'urgent'

/**
 * Delivery priority for Windows 10+ toast notifications.
 *
 * - `default` — normal priority
 * - `high`    — elevated priority
 * - `urgent`  — Win 11 only; falls back to `'high'` on Win 10
 */
export type WindowsPriority = 'default' | 'high' | 'urgent'

/**
 * Payload of the `notification://action` Tauri event.
 * Fired when the user clicks a button, taps the toast body, or submits an inline reply.
 */
export interface NotificationActionEvent {
  /**
   * The `id` of the `WindowsAction` that was activated.
   * Empty string (`''`) means the user tapped the toast body (no explicit button).
   */
  actionId: string
  /**
   * Map of input id → value for any inputs attached to this notification.
   * For text inputs the value is the typed string.
   * For selection inputs the value is the selected item's `id`.
   */
  inputs: Record<string, string>
  /** The `tag` of the notification, if set. */
  tag?: string
  /** The `group` of the notification, if set. */
  group?: string
}

/**
 * A notification currently visible in the Windows Action Center.
 * Only available via the Notification Listener (MSIX + Win 10+).
 */
export interface WinActiveNotification {
  /** System-assigned numeric identifier. */
  id: number
  tag?: string
  group?: string
  title?: string
  body?: string
  /** App User Model ID of the app that posted this notification. */
  appId?: string
}

/**
 * Access status for the Windows Notification Listener.
 *
 * - `allowed`      — access granted, `getActiveNotifications()` will work
 * - `denied`       — user denied access
 * - `unspecified`  — user has not been asked yet
 * - `notSupported` — not available (non-Windows, Windows < 10, or non-MSIX app)
 */
export type ListenerAccessStatus =
  | 'allowed'
  | 'denied'
  | 'unspecified'
  | 'notSupported'

// ─────────────────────────────────────────────────────────────────────────────
// Main Options type
// ─────────────────────────────────────────────────────────────────────────────

/** Options for sending a notification. */
export interface Options {
  /** Notification identifier. Auto-generated if not set. */
  id?: number
  /** Android channel id. */
  channelId?: string
  /** Notification title. */
  title: string
  /** Notification body text. */
  body?: string
  /** Schedule for a delayed or repeating notification. */
  schedule?: Schedule
  /**
   * Multi-line body text (changes style to "big text" on Android).
   * On Windows renders as a second text block with up to 5 lines.
   */
  largeBody?: string
  /** Summary text shown with `largeBody`, inbox lines, or group summaries. */
  summary?: string
  /** Action type identifier (mobile). */
  actionTypeId?: string
  /**
   * Notification group identifier.
   * On Windows 10+ used with `tag` for the WinRT History API.
   */
  group?: string
  /** Mark as group summary notification (Android). */
  groupSummary?: boolean
  /**
   * Sound to play.
   *
   * On Windows, pass the suffix of a `ms-winsoundevent` name:
   * `'Default'`, `'Mail'`, `'IM'`, `'Reminder'`, `'Looping.Alarm'`, etc.
   * Pass `'silent'` or use `silent: true` for no sound.
   */
  sound?: string
  /** Inbox-style lines (Android). Cannot be used with `largeBody`. */
  inboxLines?: string[]
  /**
   * Notification icon path.
   *
   * On Android, must be in `res/drawable`.
   * On Windows 8.1+, used as the `appLogoOverride` (circle-cropped, top-left).
   */
  icon?: string
  /** Large icon (Android only). Must be in `res/drawable`. */
  largeIcon?: string
  /** Icon tint color (Android). */
  iconColor?: string
  /** File attachments (iOS). */
  attachments?: Attachment[]
  /** Extra key-value payload stored with the notification. */
  extra?: Record<string, unknown>
  /** Persistent notification that cannot be dismissed (Android). */
  ongoing?: boolean
  /** Auto-dismiss when the user taps the notification. */
  autoCancel?: boolean
  /** Silent notification — no sound, no badge (iOS), no tray sound (Windows). */
  silent?: boolean

  // ── Windows-specific fields ──────────────────────────────────────────────

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
   * Interactive action buttons (Windows 10+).
   * Up to 5 buttons are supported. Clicks fire `notification://action`.
   */
  windowsActions?: WindowsAction[]
  /**
   * Text or selection inputs embedded in the toast (Windows 10+).
   * WinRT requires inputs to appear before buttons in the XML — the plugin
   * handles this automatically.
   */
  windowsInputs?: WindowsInput[]
  /**
   * Progress bar state (Windows 10 build 19041+).
   * Re-send with the same `tag` + `group` to update the bar in-place.
   */
  progress?: WindowsProgress
  /**
   * Auto-remove from the Action Center after this many milliseconds (Windows 10+).
   */
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
  /**
   * Remove from the Action Center on next reboot (Windows 11+).
   */
  expiresOnReboot?: boolean
  /**
   * Enable background COM activation for this notification (Windows 8+).
   * Requires `comServerGuid` to be set in `tauri.conf.json`.
   */
  backgroundActivation?: boolean
}

// ─────────────────────────────────────────────────────────────────────────────
// Exported functions
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Check if the user has granted permission to send notifications.
 *
 * @example
 * ```ts
 * if (await isPermissionGranted()) {
 *   await sendNotification({ title: 'Hi!' })
 * }
 * ```
 */
export async function isPermissionGranted(): Promise<boolean> {
  if (window.Notification.permission !== 'default') {
    return Promise.resolve(window.Notification.permission === 'granted')
  }
  return invoke('plugin:notification|is_permission_granted')
}

/**
 * Request permission to send notifications.
 */
export async function requestPermission(): Promise<PermissionState> {
  return invoke('plugin:notification|request_permission')
}

/**
 * Send a notification.
 *
 * @example
 * ```ts
 * // Simple
 * await sendNotification({ title: 'Hello', body: 'World' })
 *
 * // With Windows action buttons
 * await sendNotification({
 *   title: 'Build failed',
 *   body: '3 errors in main.rs',
 *   windowsActions: [
 *     { id: 'view', label: 'View logs', actionType: 'foreground' },
 *     { id: 'dismiss', label: 'Dismiss', actionType: 'background' },
 *   ]
 * })
 * ```
 */
export async function sendNotification(
  options: string | Options
): Promise<void> {
  if (typeof options === 'object') {
    Object.freeze(options)
  }
  await invoke('plugin:notification|notify', {
    options: typeof options === 'string' ? { title: options } : options
  })
}

// ── Notification history (Windows 10+) ────────────────────────────────────

/**
 * Remove a single notification from the Windows Action Center by tag + group.
 * No-op on non-Windows or Windows < 10.
 *
 * @example
 * ```ts
 * await clearNotification('download-42', 'downloads')
 * ```
 */
export async function clearNotification(
  tag: string,
  group: string
): Promise<void> {
  await invoke('plugin:notification|clear_notification', { tag, group })
}

/**
 * Remove all notifications in a group from the Windows Action Center.
 * No-op on non-Windows or Windows < 10.
 */
export async function clearNotificationGroup(group: string): Promise<void> {
  await invoke('plugin:notification|clear_notification_group', { group })
}

/**
 * Remove all notifications posted by this app from the Windows Action Center.
 * No-op on non-Windows or Windows < 10.
 */
export async function clearAllNotifications(): Promise<void> {
  await invoke('plugin:notification|clear_all_notifications')
}

// ── Notification Listener (MSIX + Windows 10+ only) ───────────────────────

/**
 * Request access to read all notifications in the Windows Action Center,
 * including those from other apps.
 *
 * Shows a system permission prompt the first time it is called.
 * Always returns `'notSupported'` for non-MSIX-packaged apps and on
 * non-Windows platforms.
 *
 * @example
 * ```ts
 * const status = await requestListenerAccess()
 * if (status === 'allowed') {
 *   const all = await getActiveNotifications()
 * }
 * ```
 */
export async function requestListenerAccess(): Promise<ListenerAccessStatus> {
  return invoke('plugin:notification|request_listener_access')
}

/**
 * Get the current Notification Listener access status without prompting.
 */
export async function getListenerAccessStatus(): Promise<ListenerAccessStatus> {
  return invoke('plugin:notification|get_listener_access_status')
}

/**
 * Return all toast notifications currently visible in the Windows Action Center,
 * including those from other apps.
 *
 * Requires `ListenerAccessStatus` to be `'allowed'`.
 * Returns `[]` on non-Windows, Windows < 10, or when access has not been granted.
 */
export async function getActiveNotifications(): Promise<
  WinActiveNotification[]
> {
  return invoke('plugin:notification|get_active_notifications')
}

// ── Action event listener ─────────────────────────────────────────────────

/**
 * Listen for notification action events.
 *
 * Fires when the user:
 * - Clicks an action button (`actionId` = the button's `id`)
 * - Taps the toast body (`actionId` = `''`)
 * - Submits an inline reply (`actionId` = button's `id`, `inputs` contains the text)
 *
 * Returns an unlisten function — call it to stop listening.
 *
 * @example
 * ```ts
 * const unlisten = await onNotificationAction((event) => {
 *   switch (event.actionId) {
 *     case 'reply':
 *       sendMessage(event.inputs['reply_box'])
 *       break
 *     case 'snooze':
 *       snoozeFor(parseInt(event.inputs['snooze_mins']))
 *       break
 *     case '':
 *       // body tap — open the app
 *       break
 *   }
 * })
 *
 * // Later, when you no longer need it:
 * unlisten()
 * ```
 */
export async function onNotificationAction(
  handler: (event: NotificationActionEvent) => void
): Promise<UnlistenFn> {
  return listen<NotificationActionEvent>('notification://action', (event) =>
    handler(event.payload)
  )
}
