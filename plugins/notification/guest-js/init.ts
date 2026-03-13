// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import type { PermissionState } from '@tauri-apps/api/core'
import { invoke } from '@tauri-apps/api/core'
import type {
  ListenerAccessStatus,
  NotificationActionEvent,
  Options,
  WinActiveNotification
} from './index'
;(function () {
  let permissionSettable = false
  let permissionValue = 'default'

  // ── Permission helpers ────────────────────────────────────────────────────

  async function isPermissionGranted(): Promise<boolean> {
    // @ts-expect-error __TEMPLATE_windows__ is replaced by Rust before injection
    if (window.Notification.permission !== 'default' || __TEMPLATE_windows__) {
      return await Promise.resolve(window.Notification.permission === 'granted')
    }
    return await invoke('plugin:notification|is_permission_granted')
  }

  function setNotificationPermission(value: NotificationPermission): void {
    permissionSettable = true
    // @ts-expect-error we can actually set this value on the webview
    window.Notification.permission = value
    permissionSettable = false
  }

  async function requestPermission(): Promise<PermissionState> {
    return await invoke<PermissionState>(
      'plugin:notification|request_permission'
    ).then((permission) => {
      setNotificationPermission(
        permission === 'prompt' || permission === 'prompt-with-rationale'
          ? 'default'
          : permission
      )
      return permission
    })
  }

  // ── Core send ─────────────────────────────────────────────────────────────

  async function sendNotification(options: string | Options): Promise<void> {
    if (typeof options === 'object') {
      Object.freeze(options)
    }
    await invoke('plugin:notification|notify', {
      options: typeof options === 'string' ? { title: options } : options
    })
  }

  // ── Notification history (Win 10+) ────────────────────────────────────────

  /**
   * Remove a single notification by tag + group from the Action Center.
   * No-op on non-Windows or Windows < 10.
   */
  async function clearNotification(tag: string, group: string): Promise<void> {
    await invoke('plugin:notification|clear_notification', { tag, group })
  }

  /**
   * Remove all notifications in a group from the Action Center.
   * No-op on non-Windows or Windows < 10.
   */
  async function clearNotificationGroup(group: string): Promise<void> {
    await invoke('plugin:notification|clear_notification_group', { group })
  }

  /**
   * Remove all notifications posted by this app from the Action Center.
   * No-op on non-Windows or Windows < 10.
   */
  async function clearAllNotifications(): Promise<void> {
    await invoke('plugin:notification|clear_all_notifications')
  }

  // ── Notification Listener (MSIX + Win 10+ only) ───────────────────────────

  /**
   * Request access to read all notifications in the Windows Action Center.
   *
   * Shows a system permission prompt the first time.
   * Always returns `'notSupported'` outside of MSIX-packaged builds on Win 10+.
   */
  async function requestListenerAccess(): Promise<ListenerAccessStatus> {
    return await invoke<ListenerAccessStatus>(
      'plugin:notification|request_listener_access'
    )
  }

  /**
   * Get the current Notification Listener access status without prompting.
   */
  async function getListenerAccessStatus(): Promise<ListenerAccessStatus> {
    return await invoke<ListenerAccessStatus>(
      'plugin:notification|get_listener_access_status'
    )
  }

  /**
   * Return all toast notifications currently in the Windows Action Center,
   * including those from other apps.
   *
   * Requires `ListenerAccessStatus.Allowed`. Returns `[]` on non-Windows,
   * Windows < 10, or when access has not been granted.
   */
  async function getActiveNotifications(): Promise<WinActiveNotification[]> {
    return await invoke<WinActiveNotification[]>(
      'plugin:notification|get_active_notifications'
    )
  }

  // ── Action event listener ─────────────────────────────────────────────────

  /**
   * Listen for notification action events fired when the user interacts
   * with a toast (button click, body tap, or inline reply).
   *
   * Returns an unlisten function — call it to stop listening.
   *
   * @example
   * ```ts
   * const unlisten = await onNotificationAction((event) => {
   *   if (event.actionId === 'reply') {
   *     sendMessage(event.inputs['reply_box'])
   *   }
   *   // body tap: actionId === ''
   * })
   * ```
   */
  async function onNotificationAction(
    handler: (event: NotificationActionEvent) => void
  ): Promise<() => void> {
    const { listen } = await import('@tauri-apps/api/event')
    return listen<NotificationActionEvent>('notification://action', (event) =>
      handler(event.payload)
    )
  }

  // ── window.Notification override ─────────────────────────────────────────

  // @ts-expect-error we replace the browser Notification API with our own
  window.Notification = function (
    title: string,
    options?: NotificationOptions
  ) {
    const opts = options ?? {}
    void sendNotification(Object.assign(opts, { title }) as Options)
  }

  // @ts-expect-error tauri does not have sync IPC
  window.Notification.requestPermission = requestPermission

  Object.defineProperty(window.Notification, 'permission', {
    enumerable: true,
    get: () => permissionValue,
    set: (v) => {
      if (!permissionSettable) {
        throw new Error('Readonly property')
      }
      permissionValue = v as string
    }
  })

  // ── Expose extended API on window.__TAURI_NOTIFICATION__ ─────────────────
  //
  // All Windows-specific helpers are exposed here so they can be called from
  // anywhere in the frontend without importing from the plugin module.
  //
  // Usage:
  //   window.__TAURI_NOTIFICATION__.sendNotification({ title: 'Hi', windowsActions: [...] })
  //   window.__TAURI_NOTIFICATION__.onNotificationAction(handler)
  //   window.__TAURI_NOTIFICATION__.clearAllNotifications()

  // @ts-expect-error extending window
  window.__TAURI_NOTIFICATION__ = {
    /** Send a notification. Accepts the full Options object including all Windows-specific fields. */
    sendNotification,

    /** Clear a single toast by tag + group (Win 10+). */
    clearNotification,

    /** Clear all toasts in a group (Win 10+). */
    clearNotificationGroup,

    /** Clear all toasts from this app (Win 10+). */
    clearAllNotifications,

    /** Request Notification Listener access (MSIX + Win 10+ only). */
    requestListenerAccess,

    /** Get current Notification Listener access status. */
    getListenerAccessStatus,

    /** Get all active notifications from the Action Center (MSIX + Win 10+ only). */
    getActiveNotifications,

    /**
     * Subscribe to notification action events (button clicks, body taps, inline replies).
     * Returns an unlisten function.
     */
    onNotificationAction
  }

  // ── Init: sync permission state ───────────────────────────────────────────

  void isPermissionGranted().then(function (response) {
    if (response === null) {
      setNotificationPermission('default')
    } else {
      setNotificationPermission(response ? 'granted' : 'denied')
    }
  })
})()

// ─────────────────────────────────────────────────────────────────────────────
// Type declarations for window extensions
// ─────────────────────────────────────────────────────────────────────────────

declare global {
  interface Window {
    /**
     * Extended Tauri notification API.
     * Exposes all Windows-specific features alongside the core send/clear API.
     */
    __TAURI_NOTIFICATION__: {
      sendNotification(options: string | Options): Promise<void>
      clearNotification(tag: string, group: string): Promise<void>
      clearNotificationGroup(group: string): Promise<void>
      clearAllNotifications(): Promise<void>
      requestListenerAccess(): Promise<ListenerAccessStatus>
      getListenerAccessStatus(): Promise<ListenerAccessStatus>
      getActiveNotifications(): Promise<WinActiveNotification[]>
      onNotificationAction(
        handler: (event: NotificationActionEvent) => void
      ): Promise<() => void>
    }
  }
}
