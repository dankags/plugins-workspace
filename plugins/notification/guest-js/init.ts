// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import type { PermissionState } from '@tauri-apps/api/core'
import { invoke } from '@tauri-apps/api/core'
import type { Options } from './index'

// ── Type declarations for window extensions ───────────────────────────────────

interface NotificationActionPayload {
  actionId: string
  inputs: Record<string, string>
  tag?: string
  group?: string
}

interface TauriNotificationApi {
  sendNotification(options: string | Options): Promise<void>
  clearNotification(tag: string, group: string): Promise<void>
  clearNotificationGroup(group: string): Promise<void>
  clearAllNotifications(): Promise<void>
  requestListenerAccess(): Promise<string>
  getListenerAccessStatus(): Promise<string>
  getActiveNotifications(): Promise<unknown[]>
  onNotificationAction(
    handler: (event: NotificationActionPayload) => void
  ): Promise<() => void>
}

// __TAURI_INTERNALS__.listen is always available in the webview.
// We only use it for `listen` because importing from @tauri-apps/api/event
// would make rollup treat this as a code-splitting build, which is incompatible
// with the iife output format required for init scripts.
interface TauriInternals {
  listen<T>(
    event: string,
    handler: (event: { payload: T }) => void
  ): Promise<() => void>
}

declare global {
  interface Window {
    __TAURI_INTERNALS__: TauriInternals
    __TAURI_NOTIFICATION__: TauriNotificationApi
    // Set by this IIFE after __TEMPLATE_windows__ is resolved by Rust.
    // index.ts reads this instead of referencing __TEMPLATE_windows__ directly,
    // because index.ts is bundled by Next.js which does not perform the Rust
    // string-replace and would throw ReferenceError.
    __TAURI_NOTIFICATION_WINDOWS__: boolean
  }
}

// ── IIFE ──────────────────────────────────────────────────────────────────────

;(function () {
  console.log('NOTIFICATION INIT IIFE LOADED')
  let permissionSettable = false
  let permissionValue = 'default'

  console.log('WINDOWS TEMPLATE VALUE:', '__TEMPLATE_windows__')

  // ── Permission helpers (unchanged from upstream) ──────────────────────

  async function isPermissionGranted(): Promise<boolean> {
    // @ts-expect-error __TEMPLATE_windows__ will be replaced in rust before it's injected.
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

  // ── Core send ─────────────────────────────────────────────────────────

  async function sendNotification(options: string | Options): Promise<void> {
    if (typeof options === 'object') {
      Object.freeze(options)
    }
    await invoke('plugin:notification|notify', {
      options: typeof options === 'string' ? { title: options } : options
    })
  }

  // ── Notification history (Win 10+) ────────────────────────────────────

  async function clearNotification(tag: string, group: string): Promise<void> {
    await invoke<void>('plugin:notification|clear_notification', { tag, group })
  }

  async function clearNotificationGroup(group: string): Promise<void> {
    await invoke<void>('plugin:notification|clear_notification_group', {
      group
    })
  }

  async function clearAllNotifications(): Promise<void> {
    await invoke<void>('plugin:notification|clear_all_notifications')
  }

  // ── Notification Listener (MSIX + Win 10+ only) ───────────────────────

  async function requestListenerAccess(): Promise<string> {
    return invoke<string>('plugin:notification|request_listener_access')
  }

  async function getListenerAccessStatus(): Promise<string> {
    return invoke<string>('plugin:notification|get_listener_access_status')
  }

  async function getActiveNotifications(): Promise<unknown[]> {
    return invoke<unknown[]>('plugin:notification|get_active_notifications')
  }

  // ── Action event listener ─────────────────────────────────────────────
  // Uses __TAURI_INTERNALS__.listen directly instead of importing from
  // @tauri-apps/api/event — importing that module would break the iife build.

  async function onNotificationAction(
    handler: (event: NotificationActionPayload) => void
  ): Promise<() => void> {
    return window.__TAURI_INTERNALS__.listen<NotificationActionPayload>(
      'notification://action',
      (e) => handler(e.payload)
    )
  }

  // ── window.Notification override (unchanged from upstream) ────────────

  // @ts-expect-error unfortunately we can't implement the whole type, so we overwrite it with our own version
  window.Notification = function (title, options) {
    // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
    const opts = options || {}
    void sendNotification(
      // eslint-disable-next-line @typescript-eslint/no-unsafe-argument
      Object.assign(opts, {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
        title
      })
    )
  }

  // @ts-expect-error tauri does not have sync IPC :(
  window.Notification.requestPermission = requestPermission

  Object.defineProperty(window.Notification, 'permission', {
    enumerable: true,
    get: () => permissionValue,
    set: (v) => {
      if (!permissionSettable) {
        throw new Error('Readonly property')
      }
      // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment
      permissionValue = v
    }
  })

  // ── Expose extended API on window.__TAURI_NOTIFICATION__ ─────────────

  window.__TAURI_NOTIFICATION__ = {
    sendNotification,
    clearNotification,
    clearNotificationGroup,
    clearAllNotifications,
    requestListenerAccess,
    getListenerAccessStatus,
    getActiveNotifications,
    onNotificationAction
  }

  // ── Expose Windows flag for index.ts ──────────────────────────────────
  // index.ts is bundled by Next.js which does not perform the Rust
  // string-replace on __TEMPLATE_windows__, so it cannot reference that
  // variable directly. We resolve it here (inside the IIFE that Rust DOES
  // process) and write the boolean result onto window so index.ts can read it.

  // ── Init: sync permission state on load (unchanged from upstream) ─────

  void isPermissionGranted().then(function (response) {
    if (response === null) {
      setNotificationPermission('default')
    } else {
      setNotificationPermission(response ? 'granted' : 'denied')
    }
  })

  // @ts-expect-error __TEMPLATE_windows__ is replaced by Rust before injection
  window.__TAURI_NOTIFICATION_WINDOWS__ = !!__TEMPLATE_windows__
})()
