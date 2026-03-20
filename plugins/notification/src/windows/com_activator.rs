// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! COM server registration for background notification activation.
//!
//! We deliberately avoid the `windows` crate's `#[implement]` / `#[interface]`
//! proc-macros here because they expand to code that references `windows_core`
//! by crate name, which breaks when transitive dependencies (notify-rust,
//! win7-notifications) pull in a different version of `windows-core`.
//!
//! Instead we implement the COM vtables by hand using only `windows-sys` raw
//! FFI — no proc-macros, no version conflicts.
//!
//! INotificationActivationCallback IID: {53E31837-6600-4A81-9395-75CFFE746F94}
//! https://learn.microsoft.com/en-us/windows/win32/api/notificationactivationcallback

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::{
    Foundation::{CLASS_E_NOAGGREGATION, E_NOINTERFACE, S_OK},
    System::Com::{
        CoInitializeEx, CoRegisterClassObject, CoRevokeClassObject, CLSCTX_LOCAL_SERVER,
        COINIT_MULTITHREADED, REGCLS_MULTIPLEUSE,
    },
};

use crate::windows::action_handler;

type Bool = windows_sys::core::BOOL;

/// Token returned by `CoRegisterClassObject`; kept alive for the process lifetime.
static COM_REGISTRATION_TOKEN: OnceLock<Result<Mutex<u32>, String>> = OnceLock::new();

// ── GUIDs ─────────────────────────────────────────────────────────────────

/// INotificationActivationCallback IID
const IID_NOTIFICATION_ACTIVATION_CALLBACK: Guid = Guid {
    data1: 0x53E31837,
    data2: 0x6600,
    data3: 0x4A81,
    data4: [0x93, 0x95, 0x75, 0xCF, 0xFE, 0x74, 0x6F, 0x94],
};

/// IUnknown IID
const IID_IUNKNOWN: Guid = Guid {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

/// IClassFactory IID
const IID_ICLASS_FACTORY: Guid = Guid {
    data1: 0x00000001,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

// windows-sys GUID type alias
type Guid = windows_sys::core::GUID;
type Hresult = windows_sys::core::HRESULT;

// ── Key/value pair for input fields ───────────────────────────────────────

#[repr(C)]
struct NotificationUserInputData {
    key: *const u16,   // PCWSTR
    value: *const u16, // PCWSTR
}

// ── INotificationActivationCallback vtable (manual) ───────────────────────

#[repr(C)]
struct INotificationActivationCallbackVtbl {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // INotificationActivationCallback
    activate: unsafe extern "system" fn(
        *mut c_void,
        *const u16, // AppUserModelId  (LPCWSTR)
        *const u16, // InvokedArgs     (LPCWSTR)
        *const NotificationUserInputData,
        u32, // Count
    ) -> Hresult,
}

#[repr(C)]
struct NotificationActivator {
    vtbl: *const INotificationActivationCallbackVtbl,
    ref_count: std::sync::atomic::AtomicU32,
}

static NOTIFICATION_ACTIVATOR_VTBL: INotificationActivationCallbackVtbl =
    INotificationActivationCallbackVtbl {
        query_interface: activator_query_interface,
        add_ref: activator_add_ref,
        release: activator_release,
        activate: activator_activate,
    };

unsafe extern "system" fn activator_query_interface(
    this: *mut c_void,
    riid: *const Guid,
    object: *mut *mut c_void,
) -> Hresult {
    let riid = &*riid;
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_NOTIFICATION_ACTIVATION_CALLBACK) {
        activator_add_ref(this);
        *object = this;
        S_OK
    } else {
        *object = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn activator_add_ref(this: *mut c_void) -> u32 {
    let obj = &*(this as *const NotificationActivator);
    obj.ref_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1
}

unsafe extern "system" fn activator_release(this: *mut c_void) -> u32 {
    let obj = &*(this as *const NotificationActivator);
    let prev = obj
        .ref_count
        .fetch_sub(1, std::sync::atomic::Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        drop(Box::from_raw(this as *mut NotificationActivator));
    }
    prev - 1
}

unsafe extern "system" fn activator_activate(
    _this: *mut c_void,
    _app_user_model_id: *const u16,
    invoked_args: *const u16,
    data: *const NotificationUserInputData,
    count: u32,
) -> Hresult {
    let result = std::panic::catch_unwind(|| {
        let action_id = if invoked_args.is_null() {
            String::new()
        } else {
            pcwstr_to_string(invoked_args)
        };

        let mut inputs = std::collections::HashMap::new();
        if !data.is_null() && count > 0 {
            let slice = std::slice::from_raw_parts(data, count as usize);
            for item in slice {
                let key = if item.key.is_null() {
                    String::new()
                } else {
                    pcwstr_to_string(item.key)
                };
                let val = if item.value.is_null() {
                    String::new()
                } else {
                    pcwstr_to_string(item.value)
                };
                inputs.insert(key, val);
            }
        }

        action_handler::dispatch(crate::models::NotificationActionEvent {
            action_id,
            inputs,
            tag: None,
            group: None,
        });
    });

    if result.is_err() {
        log::error!("[notification] panic in COM activator callback");
        return windows_sys::Win32::Foundation::E_FAIL;
    }

    S_OK
}

fn new_activator() -> *mut c_void {
    let obj = Box::new(NotificationActivator {
        vtbl: &NOTIFICATION_ACTIVATOR_VTBL,
        ref_count: std::sync::atomic::AtomicU32::new(1),
    });
    Box::into_raw(obj) as *mut c_void
}

// ── IClassFactory vtable (manual) ─────────────────────────────────────────

#[repr(C)]
struct IClassFactoryVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    create_instance: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const Guid,
        *mut *mut c_void,
    ) -> Hresult,
    lock_server: unsafe extern "system" fn(*mut c_void, Bool) -> Hresult,
}

#[repr(C)]
struct NotificationActivatorFactory {
    vtbl: *const IClassFactoryVtbl,
}

static FACTORY_VTBL: IClassFactoryVtbl = IClassFactoryVtbl {
    query_interface: factory_query_interface,
    add_ref: factory_add_ref,
    release: factory_release,
    create_instance: factory_create_instance,
    lock_server: factory_lock_server,
};

// The factory is a process-lifetime singleton — static storage, no heap alloc.
static mut FACTORY_INSTANCE: NotificationActivatorFactory = NotificationActivatorFactory {
    vtbl: &FACTORY_VTBL,
};

unsafe extern "system" fn factory_query_interface(
    this: *mut c_void,
    riid: *const Guid,
    object: *mut *mut c_void,
) -> Hresult {
    let riid = &*riid;
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_ICLASS_FACTORY) {
        *object = this;
        S_OK
    } else {
        *object = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn factory_add_ref(_this: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn factory_release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn factory_create_instance(
    _this: *mut c_void,
    outer: *mut c_void,
    riid: *const Guid,
    object: *mut *mut c_void,
) -> Hresult {
    if !outer.is_null() {
        *object = std::ptr::null_mut();
        return CLASS_E_NOAGGREGATION;
    }
    let activator = new_activator();
    let riid_ref = &*riid;
    if guid_eq(riid_ref, &IID_IUNKNOWN) || guid_eq(riid_ref, &IID_NOTIFICATION_ACTIVATION_CALLBACK)
    {
        *object = activator;
        S_OK
    } else {
        // Release the just-created object
        activator_release(activator);
        *object = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn factory_lock_server(_this: *mut c_void, _lock: Bool) -> Hresult {
    S_OK
}

// ── Public API ────────────────────────────────────────────────────────────

/// Register the COM activator factory. Idempotent — safe to call multiple times.
pub fn register(guid_str: &str) -> crate::Result<()> {
    let guid =
        parse_guid(guid_str).map_err(|_| crate::Error::InvalidComGuid(guid_str.to_string()))?;

    COM_REGISTRATION_TOKEN
        .get_or_init(|| unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32);

            let factory_ptr = std::ptr::addr_of_mut!(FACTORY_INSTANCE) as *mut c_void;

            let mut token: u32 = 0;
            let hr = CoRegisterClassObject(
                &guid,
                factory_ptr,
                CLSCTX_LOCAL_SERVER,
                REGCLS_MULTIPLEUSE as u32,
                &mut token,
            );

            if hr != S_OK {
                return Err(format!("CoRegisterClassObject failed: HRESULT {hr:#010x}"));
            }

            Ok(Mutex::new(token))
        })
        .as_ref()
        .map_err(|e| crate::Error::Windows(e.clone()))?;

    log::debug!("[notification] COM activator registered (GUID={guid_str})");
    Ok(())
}

/// Unregister the COM factory.
pub fn unregister() {
    if let Some(Ok(m)) = COM_REGISTRATION_TOKEN.get() {
        if let Ok(token) = m.lock() {
            unsafe {
                CoRevokeClassObject(*token);
            }
        }
    }
}
/// Returns `true` if the process was launched by Windows for background activation.
pub fn is_background_activation_launch() -> bool {
    std::env::args().any(|a| a == "----BackgroundActivated")
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn guid_eq(a: &Guid, b: &Guid) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

/// Convert a null-terminated UTF-16 string pointer to a Rust `String`.
unsafe fn pcwstr_to_string(p: *const u16) -> String {
    let mut len = 0;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

fn parse_guid(s: &str) -> Result<Guid, ()> {
    let s = s.trim_matches(|c| c == '{' || c == '}');
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return Err(());
    }

    let expected = [8usize, 4, 4, 4, 12];
    for (part, exp) in parts.iter().zip(expected.iter()) {
        if part.len() != *exp {
            return Err(());
        }
    }

    let data1 = u32::from_str_radix(parts[0], 16).map_err(|_| ())?;
    let data2 = u16::from_str_radix(parts[1], 16).map_err(|_| ())?;
    let data3 = u16::from_str_radix(parts[2], 16).map_err(|_| ())?;

    let d4_str = format!("{}{}", parts[3], parts[4]);
    if d4_str.len() != 16 {
        return Err(());
    }
    let mut data4 = [0u8; 8];
    for i in 0..8 {
        data4[i] = u8::from_str_radix(&d4_str[i * 2..i * 2 + 2], 16).map_err(|_| ())?;
    }

    Ok(Guid {
        data1,
        data2,
        data3,
        data4,
    })
}

#[cfg(test)]
mod extended_tests {
    use super::*;

    // ── parse_guid: valid inputs ──────────────────────────────────────────

    #[test]
    fn parse_guid_uppercase_and_lowercase_hex() {
        // Both casings must be accepted
        parse_guid("a3b4c5d6-e7f8-9012-abcd-ef0123456789").unwrap();
        parse_guid("A3B4C5D6-E7F8-9012-ABCD-EF0123456789").unwrap();
    }

    #[test]
    fn parse_guid_all_zeros() {
        let g = parse_guid("00000000-0000-0000-0000-000000000000").unwrap();
        assert_eq!(g.data1, 0);
        assert_eq!(g.data2, 0);
        assert_eq!(g.data3, 0);
        assert_eq!(g.data4, [0u8; 8]);
    }

    #[test]
    fn parse_guid_all_fs() {
        let g = parse_guid("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF").unwrap();
        assert_eq!(g.data1, 0xFFFF_FFFF);
        assert_eq!(g.data2, 0xFFFF);
        assert_eq!(g.data3, 0xFFFF);
        assert_eq!(g.data4, [0xFF; 8]);
    }

    #[test]
    fn parse_guid_data_fields_are_correct() {
        // {12345678-1234-5678-9ABC-DEF012345678}
        let g = parse_guid("12345678-1234-5678-9ABC-DEF012345678").unwrap();
        assert_eq!(g.data1, 0x1234_5678);
        assert_eq!(g.data2, 0x1234);
        assert_eq!(g.data3, 0x5678);
        assert_eq!(g.data4[0], 0x9A);
        assert_eq!(g.data4[1], 0xBC);
        assert_eq!(g.data4[2], 0xDE);
        assert_eq!(g.data4[3], 0xF0);
    }

    #[test]
    fn parse_guid_braces_stripped() {
        let plain = parse_guid("A3B4C5D6-E7F8-9012-ABCD-EF0123456789").unwrap();
        let braced = parse_guid("{A3B4C5D6-E7F8-9012-ABCD-EF0123456789}").unwrap();
        assert_eq!(plain.data1, braced.data1);
        assert_eq!(plain.data4, braced.data4);
    }

    // ── parse_guid: invalid inputs ────────────────────────────────────────

    #[test]
    fn parse_guid_empty_string() {
        assert!(parse_guid("").is_err());
    }

    #[test]
    fn parse_guid_too_few_groups() {
        assert!(parse_guid("AAAAAAAA-BBBB-CCCC-DDDD").is_err());
    }

    #[test]
    fn parse_guid_too_many_groups() {
        assert!(parse_guid("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE-FF").is_err());
    }

    #[test]
    fn parse_guid_first_group_too_short() {
        assert!(parse_guid("A3B4C5D-E7F8-9012-ABCD-EF0123456789").is_err());
    }

    #[test]
    fn parse_guid_first_group_too_long() {
        assert!(parse_guid("A3B4C5D6E-E7F8-9012-ABCD-EF0123456789").is_err());
    }

    #[test]
    fn parse_guid_second_group_too_short() {
        assert!(parse_guid("A3B4C5D6-E7F-9012-ABCD-EF0123456789").is_err());
    }

    #[test]
    fn parse_guid_second_group_too_long() {
        assert!(parse_guid("A3B4C5D6-E7F89-9012-ABCD-EF0123456789").is_err());
    }

    #[test]
    fn parse_guid_last_group_too_short() {
        assert!(parse_guid("A3B4C5D6-E7F8-9012-ABCD-EF012345678").is_err());
    }

    #[test]
    fn parse_guid_last_group_too_long() {
        assert!(parse_guid("A3B4C5D6-E7F8-9012-ABCD-EF01234567890").is_err());
    }

    #[test]
    fn parse_guid_non_hex_in_first_group() {
        assert!(parse_guid("GGGGGGGG-E7F8-9012-ABCD-EF0123456789").is_err());
    }

    #[test]
    fn parse_guid_non_hex_in_last_group() {
        assert!(parse_guid("A3B4C5D6-E7F8-9012-ABCD-EF01234567ZZ").is_err());
    }

    #[test]
    fn parse_guid_whitespace_not_allowed() {
        assert!(parse_guid("A3B4C5D6 E7F8 9012 ABCD EF0123456789").is_err());
    }

    // ── is_background_activation_launch ──────────────────────────────────

    #[test]
    fn background_activation_absent_in_normal_test_process() {
        // The test binary is not launched by Windows for background activation,
        // so this must return false.
        assert!(!super::is_background_activation_launch());
    }

    // ── register: rejects invalid GUIDs before touching COM ──────────────
    // (Valid-GUID paths require a live COM server and cannot be unit-tested.)

    #[test]
    fn register_invalid_guid_returns_error() {
        let result = super::register("not-valid");
        assert!(result.is_err(), "invalid GUID must return Err");
        // Must be the InvalidComGuid variant, not a Windows API error
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::InvalidComGuid(_)
        ));
    }

    #[test]
    fn register_short_group_returns_error() {
        let result = super::register("A3B4C5D-E7F8-9012-ABCD-EF0123456789");
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::InvalidComGuid(_)
        ));
    }
}
