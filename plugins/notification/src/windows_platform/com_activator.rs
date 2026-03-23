use std::sync::{Mutex, OnceLock};

use windows::{
    core::*,
    Win32::{Foundation::*, System::Com::*, UI::Notifications::*},
};

static REGISTRATION_TOKEN: OnceLock<std::result::Result<Mutex<u32>, String>> = OnceLock::new();

//
// Activator
//

#[implement(INotificationActivationCallback)]
struct NotificationActivator;

impl INotificationActivationCallback_Impl for NotificationActivator_Impl {
    fn Activate(
        &self,
        _app_user_model_id: &PCWSTR,
        invoked_args: &PCWSTR,
        data: *const NOTIFICATION_USER_INPUT_DATA,
        count: u32,
    ) -> Result<()> {
        let result = std::panic::catch_unwind(|| {
            let raw_args = unsafe { invoked_args.to_string().unwrap_or_default() };

            // Parse "action=foo&tag=bar&group=baz" — also handles plain action ids
            // that don't use key=value encoding (e.g. protocol activations)
            let mut params: std::collections::HashMap<String, String> = raw_args
                .split('&')
                .filter_map(|pair| {
                    let mut it = pair.splitn(2, '=');
                    let key = it.next()?.to_string();
                    let val = it.next().unwrap_or("").to_string();
                    Some((key, val))
                })
                .collect();

            // If "action" key is present use it; otherwise the whole string is the id
            // (covers plain dismiss / body-click where launch attr has no key=value)
            let action_id = params
                .remove("action")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| raw_args.clone());

            let tag = params.remove("tag").filter(|s| !s.is_empty());
            let group = params.remove("group").filter(|s| !s.is_empty());

            // Collect user inputs from the data array
            let mut inputs = std::collections::HashMap::new();
            if !data.is_null() && count > 0 {
                unsafe {
                    let slice = std::slice::from_raw_parts(data, count as usize);
                    for item in slice {
                        let key = item.Key.to_string().unwrap_or_default();
                        let value = item.Value.to_string().unwrap_or_default();
                        inputs.insert(key, value);
                    }
                }
            }

            crate::windows_platform::action_handler::dispatch(
                crate::models::NotificationActionEvent {
                    action_id,
                    inputs,
                    tag,
                    group,
                },
            );
        });

        if result.is_err() {
            return Err(Error::from(E_FAIL));
        }

        Ok(())
    }
}

//
// Class Factory
//

#[implement(IClassFactory)]
struct NotificationActivatorFactory;

// FIX 1 (same): target NotificationActivatorFactory_Impl, not NotificationActivatorFactory
impl IClassFactory_Impl for NotificationActivatorFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>, // FIX 2: in windows-rs 0.58+, the outer param type
        // changed from Option<&IUnknown> to Ref<IUnknown>.
        // Ref<T> is defined in windows::core and represents
        // a nullable COM interface reference.
        riid: *const GUID,
        object: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        // FIX 2 cont'd: Ref<T> does not deref to Option — use .is_null() to check
        if !outer.is_null() {
            unsafe {
                *object = core::ptr::null_mut();
            }
            return Err(Error::from(CLASS_E_NOAGGREGATION));
        }

        let activator: INotificationActivationCallback = NotificationActivator.into();
        unsafe { activator.query(riid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

//
// Register
//

pub fn register(guid: &GUID) -> Result<()> {
    REGISTRATION_TOKEN
        .get_or_init(|| {
            // Inner closure is explicitly typed — all `?` inside here
            // propagate windows_core::Error, which is correct.
            let init = || -> Result<Mutex<u32>> {
                unsafe {
                    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;

                    let factory: IClassFactory = NotificationActivatorFactory.into();

                    let token = CoRegisterClassObject(
                        guid,
                        &factory,
                        CLSCTX_LOCAL_SERVER,
                        REGCLS_MULTIPLEUSE,
                    )?;

                    Ok(Mutex::new(token))
                }
            };

            // Only at THIS boundary do we convert to String
            init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Error::new(E_FAIL, e.as_str()))?;

    Ok(())
}

//
// Unregister
//

pub fn unregister() {
    if let Some(Ok(mutex)) = REGISTRATION_TOKEN.get() {
        if let Ok(token) = mutex.lock() {
            unsafe {
                let _ = CoRevokeClassObject(*token);
            }
        }
    }
}

//
// Background activation detection
//

pub fn is_background_activation_launch() -> bool {
    std::env::args().any(|a| a == "----BackgroundActivated")
}

/// Parse a GUID string in the standard registry format:
/// "{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}" or without braces.
pub fn parse_guid(s: &str) -> windows::core::Result<GUID> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');

    let parts: Vec<&str> = s.splitn(5, '-').collect();

    if parts.len() != 5 {
        return Err(Error::new(E_INVALIDARG, "Invalid GUID format"));
    }

    let data1 = u32::from_str_radix(parts[0], 16)
        .map_err(|_| Error::new(E_INVALIDARG, "Invalid GUID data1"))?;

    let data2 = u16::from_str_radix(parts[1], 16)
        .map_err(|_| Error::new(E_INVALIDARG, "Invalid GUID data2"))?;

    let data3 = u16::from_str_radix(parts[2], 16)
        .map_err(|_| Error::new(E_INVALIDARG, "Invalid GUID data3"))?;

    let d4_hex = format!("{}{}", parts[3], parts[4]);

    if d4_hex.len() != 16 {
        return Err(Error::new(E_INVALIDARG, "Invalid GUID data4 length"));
    }

    let mut data4 = [0u8; 8];
    for i in 0..8 {
        data4[i] = u8::from_str_radix(&d4_hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| Error::new(E_INVALIDARG, "Invalid GUID data4 byte"))?;
    }

    Ok(GUID {
        data1,
        data2,
        data3,
        data4,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_guid;
    use windows::core::GUID;

    // Helper to build expected GUIDs inline without parse_guid
    fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GUID {
        GUID {
            data1,
            data2,
            data3,
            data4,
        }
    }

    // -------------------------------------------------------------------------
    // parse_guid — valid inputs
    // -------------------------------------------------------------------------

    #[test]
    fn parse_guid_with_braces() {
        let g = parse_guid("{6D809377-6AF0-444B-8957-A3773F02200E}").unwrap();
        assert_eq!(
            g,
            guid(
                0x6D809377,
                0x6AF0,
                0x444B,
                [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
            )
        );
    }

    #[test]
    fn parse_guid_without_braces() {
        let g = parse_guid("6D809377-6AF0-444B-8957-A3773F02200E").unwrap();
        assert_eq!(
            g,
            guid(
                0x6D809377,
                0x6AF0,
                0x444B,
                [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
            )
        );
    }

    #[test]
    fn parse_guid_lowercase() {
        let g = parse_guid("{6d809377-6af0-444b-8957-a3773f02200e}").unwrap();
        assert_eq!(
            g,
            guid(
                0x6D809377,
                0x6AF0,
                0x444B,
                [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
            )
        );
    }

    #[test]
    fn parse_guid_mixed_case() {
        let g = parse_guid("{6D809377-6af0-444B-8957-A3773f02200e}").unwrap();
        assert_eq!(
            g,
            guid(
                0x6D809377,
                0x6AF0,
                0x444B,
                [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
            )
        );
    }

    #[test]
    fn parse_guid_all_zeros() {
        let g = parse_guid("{00000000-0000-0000-0000-000000000000}").unwrap();
        assert_eq!(g, guid(0, 0, 0, [0u8; 8]));
    }

    #[test]
    fn parse_guid_all_ff() {
        let g = parse_guid("{FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF}").unwrap();
        assert_eq!(g, guid(0xFFFFFFFF, 0xFFFF, 0xFFFF, [0xFF; 8],));
    }

    #[test]
    fn parse_guid_trims_whitespace() {
        let g = parse_guid("  {6D809377-6AF0-444B-8957-A3773F02200E}  ").unwrap();
        assert_eq!(g.data1, 0x6D809377);
    }

    // -------------------------------------------------------------------------
    // parse_guid — invalid inputs
    // -------------------------------------------------------------------------

    #[test]
    fn parse_guid_empty_string_fails() {
        assert!(parse_guid("").is_err());
    }

    #[test]
    fn parse_guid_too_few_segments_fails() {
        // Missing last segment
        assert!(parse_guid("{6D809377-6AF0-444B-8957}").is_err());
    }

    #[test]
    fn parse_guid_too_many_segments_fails() {
        assert!(parse_guid("{6D809377-6AF0-444B-8957-A3773F02200E-EXTRA}").is_err());
    }

    #[test]
    fn parse_guid_invalid_hex_in_data1_fails() {
        assert!(parse_guid("{GGGGGGGG-6AF0-444B-8957-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_invalid_hex_in_data2_fails() {
        assert!(parse_guid("{6D809377-ZZZZ-444B-8957-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_invalid_hex_in_data3_fails() {
        assert!(parse_guid("{6D809377-6AF0-XXXX-8957-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_invalid_hex_in_data4_fails() {
        assert!(parse_guid("{6D809377-6AF0-444B-ZZZZ-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_data4_too_short_fails() {
        // parts[3] is only 2 chars instead of 4 → d4_hex.len() != 16
        assert!(parse_guid("{6D809377-6AF0-444B-89-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_data4_too_long_fails() {
        assert!(parse_guid("{6D809377-6AF0-444B-895789-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_data1_overflow_fails() {
        // 9 hex digits — overflows u32
        assert!(parse_guid("{1FFFFFFFF-6AF0-444B-8957-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_data2_overflow_fails() {
        // 5 hex digits — overflows u16
        assert!(parse_guid("{6D809377-16AF0-444B-8957-A3773F02200E}").is_err());
    }

    #[test]
    fn parse_guid_no_dashes_fails() {
        assert!(parse_guid("6D8093776AF0444B8957A3773F02200E").is_err());
    }

    // -------------------------------------------------------------------------
    // parse_guid — round-trip / field correctness
    // -------------------------------------------------------------------------

    #[test]
    fn parse_guid_data4_bytes_are_correct() {
        // data4 = 89-57  +  A3-77-3F-02-20-0E
        let g = parse_guid("{6D809377-6AF0-444B-8957-A3773F02200E}").unwrap();
        assert_eq!(g.data4, [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E]);
    }

    #[test]
    fn parse_guid_two_identical_strings_produce_equal_guids() {
        let a = parse_guid("{6D809377-6AF0-444B-8957-A3773F02200E}").unwrap();
        let b = parse_guid("6D809377-6AF0-444B-8957-A3773F02200E").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn parse_guid_two_different_strings_produce_unequal_guids() {
        let a = parse_guid("{6D809377-6AF0-444B-8957-A3773F02200E}").unwrap();
        let b = parse_guid("{00000000-0000-0000-0000-000000000000}").unwrap();
        assert_ne!(a, b);
    }
}
