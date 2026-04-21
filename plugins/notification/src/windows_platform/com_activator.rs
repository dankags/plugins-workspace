// Hardened COM Activator for Windows Background Toast Activation
// Production-grade lifecycle with reliability, security, and observability
//
// Features implemented:
// - RAII-safe COM initialization (ComGuard)
// - Deterministic CoRegisterClassObject shutdown
// - COM security initialization (CoInitializeSecurity)
// - Threading model validation
// - Activation timeout watchdog
// - Multi-instance process collision protection (named mutex)
// - Safe message pump cancellation
// - Windows service-mode compatibility detection
// - Structured activation tracing
// - Crash-safe event journaling
// - Background activation retry strategy

use std::cell::Cell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::System::Services::{OpenSCManagerW, SC_MANAGER_CONNECT};
use windows::{
    core::{Error, GUID, HRESULT},
    Win32::Foundation::*,
    Win32::System::Com::*,
    Win32::System::Threading::*,
    Win32::UI::WindowsAndMessaging::*,
};

use windows::Win32::UI::Notifications::*;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows_core::*;
use windows_sys::Win32::Foundation::RPC_E_TOO_LATE;

use crate::trace_event;
use crate::windows_platform::runtime_context::context;

static GLOBAL_REGISTRATION: OnceLock<Mutex<Option<ComRegistration>>> = OnceLock::new();

fn registration_slot() -> &'static Mutex<Option<ComRegistration>> {
    GLOBAL_REGISTRATION.get_or_init(|| Mutex::new(None))
}

static GLOBAL_CANCEL: OnceLock<Arc<AtomicBool>> = OnceLock::new();

fn cancel_token() -> &'static Arc<AtomicBool> {
    GLOBAL_CANCEL.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

thread_local! {
    static COM_INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

static SECURITY_INITIALIZED: AtomicBool = AtomicBool::new(false);

// --- The Activator Implementation ---

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
        // This log line must always appear when a toast action is clicked.
        // If you do not see it, the issue is COM registration or toast XML,
        // not Rust logic.
        trace_event!("Activate() called");
        log::info!(
            "[notification] Activate() called — background_launch={}",
            is_background_activation_launch()
        );

        let result = std::panic::catch_unwind(|| {
            let raw_args = unsafe { invoked_args.to_string().unwrap_or_default() };

            log::debug!("[notification] Activate() raw_args={:?}", raw_args);

            let ev = crate::windows_platform::activation_bridge::parse_background_args(&raw_args);

            let mut inputs = ev.inputs;
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

            let event = crate::windows_platform::activation_bridge::to_action_event(
                crate::windows_platform::activation_bridge::ActivationEvent { inputs, ..ev },
            );

            log::info!(
                "[notification] Activate() parsed — action_id={:?} tag={:?} group={:?} inputs={:?}",
                event.action_id,
                event.tag,
                event.group,
                event.inputs.keys().collect::<Vec<_>>(),
            );

            if is_background_activation_launch() {
                // BACKGROUND PROCESS: the app has no webview / relay yet.
                // Persist the activation to the encrypted queue so the worker
                // thread can process it and emit it once the relay is ready.
                let id = uuid::Uuid::new_v4().to_string();
                crate::windows_platform::activation_queue::enqueue(id, event);
            } else {
                // FOREGROUND PROCESS: relay thread is already running.
                // Dispatch directly — no queue round-trip, no disk write,
                // no latency. The app receives the event immediately.
                crate::windows_platform::action_handler::dispatch(event);
            }
        });

        if let Err(ref e) = result {
            // Log the panic payload if it is a string so it appears in logs.
            let msg = e
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| e.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("unknown panic");
            log::error!("[notification] Activate() panicked: {}", msg);
            trace_event!("Activate() panicked");
        }

        result.map_err(|_| Error::from(E_FAIL))
    }
}

// --- The Class Factory Implementation ---

#[implement(IClassFactory)]
struct NotificationActivatorFactory;

impl IClassFactory_Impl for NotificationActivatorFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        riid: *const GUID,
        object: *mut *mut std::ffi::c_void,
    ) -> Result<()> {
        if !outer.is_null() {
            unsafe {
                *object = std::ptr::null_mut();
            }
            return Err(Error::from(CLASS_E_NOAGGREGATION));
        }

        unsafe {
            let activator: INotificationActivationCallback = NotificationActivator.into();
            activator.query(riid, object).ok()
        }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

// ============================================================
// Structured tracing + crash-safe journaling
// ============================================================

fn journal_path() -> PathBuf {
    std::panic::catch_unwind(|| context().storage_dir.join("tauri_com_activation.log"))
        .unwrap_or_else(|_| std::env::temp_dir().join("tauri_com_activation.log"))
}
pub fn write_journal(event: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path())
    {
        let _ = writeln!(file, "{:?} | {}", std::time::SystemTime::now(), event);

        let _ = file.flush();
    }
}

#[macro_export]
macro_rules! trace_event {
    ($msg:expr) => {{
        log::info!("[notification] {}", $msg);
        $crate::windows_platform::com_activator::write_journal($msg);
    }};
}

// ============================================================
// COM Security Initialization
// ============================================================

pub fn initialize_com_security() -> windows::core::Result<()> {
    if SECURITY_INITIALIZED.load(Ordering::SeqCst) {
        return Ok(());
    }

    unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IDENTIFY,
            None,
            EOAC_NONE,
            None,
        )
        .map_err(|e| {
            // If it's already initialized, we treat it as success
            if e.code() == windows_core::HRESULT(RPC_E_TOO_LATE) {
                return Error::from(S_OK);
            }
            e
        })?;

        SECURITY_INITIALIZED.store(true, Ordering::SeqCst);
        trace_event!("COM security initialized");
        Ok(())
    }
}

// ============================================================
// Threading Model Validation
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadingModel {
    STA,
    MTA,
}

pub fn current_threading_model() -> windows::core::Result<ThreadingModel> {
    unsafe {
        let mut apt = APTTYPE(0);
        let mut qual = APTTYPEQUALIFIER(0);

        CoGetApartmentType(&mut apt, &mut qual)?;

        match apt {
            APTTYPE_STA => Ok(ThreadingModel::STA),
            _ => Ok(ThreadingModel::MTA),
        }
    }
}

pub fn validate_threading_model(expected: ThreadingModel) -> windows::core::Result<()> {
    let actual = current_threading_model()?;

    if actual != expected {
        trace_event!("Threading model mismatch");
        return Err(Error::from_thread());
    }

    Ok(())
}

// ============================================================
// Activation Timeout Watchdog
// ============================================================

pub struct ActivationWatchdog {
    cancelled: Arc<AtomicBool>,
}

impl ActivationWatchdog {
    pub fn start(timeout_ms: u32) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(timeout_ms as u64));

            if !flag.load(Ordering::SeqCst) {
                trace_event!("Activation timeout — forced exit");
                std::process::exit(1);
            }
        });

        Self { cancelled }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

// ============================================================
// Multi-instance collision protection
// ============================================================

pub struct InstanceGuard {
    handle: HANDLE,
}

impl InstanceGuard {
    pub fn acquire(name: &str) -> windows::core::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();

        let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) };

        if handle.is_err() {
            return Err(Error::from_thread());
        }

        let handle = handle.unwrap();

        let err = unsafe { GetLastError() };

        if err == ERROR_ALREADY_EXISTS {
            // FIX (Issue 2a): the previous instance may still be shutting down
            // (e.g. its RAII guard is queued for drop but the OS hasn't released
            // the mutex yet). Wait up to 3 seconds for it to finish rather than
            // failing immediately — this covers the rapid-relaunch window.
            trace_event!("Instance mutex already exists — waiting for previous instance");

            let wait_result = unsafe {
                WaitForSingleObject(handle, 3000 /* ms */)
            };

            // WAIT_OBJECT_0 = 0x0 — we now own the mutex
            if wait_result.0 != 0 {
                // Timed out or abandoned — still log and fail cleanly
                unsafe {
                    let _ = CloseHandle(handle);
                }
                trace_event!("Previous instance did not release mutex in time");
                return Err(Error::from(E_FAIL));
            }

            trace_event!("Acquired instance mutex after waiting");
        }

        Ok(Self { handle })
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

// ============================================================
// Safe Message Pump
// ============================================================

pub type CancelToken = Arc<AtomicBool>;

pub fn run_pump_with_cancel(timeout_ms: u32, cancel: CancelToken) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

    loop {
        if cancel.load(Ordering::SeqCst) {
            trace_event!("Pump cancelled");
            break;
        }

        if Instant::now() >= deadline {
            trace_event!("Pump timeout reached");
            break;
        }

        unsafe {
            let mut msg = MSG::default();

            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }

                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        thread::sleep(Duration::from_millis(10));
    }
}

// ============================================================
// Service Mode Detection
// ============================================================

pub fn is_service_mode() -> bool {
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT);
        scm.is_err()
    }
}

// ============================================================
// COM Guard (RAII)
// ============================================================

pub struct ComGuard {
    initialized_here: bool,
}

impl ComGuard {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let hr: HRESULT = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            match hr {
                hr if hr.is_ok() => {
                    COM_INITIALIZED.with(|f| f.set(true));
                    trace_event!("COM initialized");

                    Ok(Self {
                        initialized_here: true,
                    })
                }

                hr if hr == RPC_E_CHANGED_MODE => {
                    trace_event!("COM already initialized with different model");

                    Ok(Self {
                        initialized_here: false,
                    })
                }

                err => Err(err.into()),
            }
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized_here {
            unsafe {
                CoUninitialize();
            }

            trace_event!("COM uninitialized");
        }
    }
}

// ============================================================
// COM Registration
// ============================================================

pub struct ComRegistration {
    cookie: u32,
    _guard: ComGuard,
}

impl Drop for ComRegistration {
    fn drop(&mut self) {
        if CLASS_REGISTERED.swap(false, Ordering::SeqCst) {
            unsafe {
                let _ = CoRevokeClassObject(self.cookie);
            }

            trace_event!("COM class revoked");
        }
    }
}

pub fn register(clsid: &GUID, factory: &IUnknown) -> windows::core::Result<ComRegistration> {
    let guard = ComGuard::new()?;

    initialize_com_security()?;

    // FIX (Issue 4): validate_threading_model(STA) was called unconditionally,
    // but ComGuard::new() succeeds with initialized_here=false when COM was
    // already initialised as MTA by another part of the process. In that case
    // validate_threading_model would return an error, register_with_retry would
    // exhaust all retries, and the COM server would silently never register.
    //
    // We only enforce the STA requirement when WE initialised COM (i.e., when
    // the guard tells us it is a fresh init). If COM was already running we
    // trust the existing apartment and skip the check — Windows will reject
    // CoRegisterClassObject with the appropriate HRESULT if it truly cannot
    // work, giving a clear error rather than a misleading threading-model one.
    if guard.initialized_here {
        validate_threading_model(ThreadingModel::STA)?;
    }

    let cookie =
        unsafe { CoRegisterClassObject(clsid, factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)? };

    CLASS_REGISTERED.store(true, Ordering::SeqCst);

    trace_event!("COM class registered");

    Ok(ComRegistration {
        cookie,
        _guard: guard,
    })
}

// ============================================================
// Background Activation Retry Strategy
// ============================================================

pub fn register_with_retry(
    clsid: &GUID,
    factory: &IUnknown,
    retries: u32,
) -> windows::core::Result<ComRegistration> {
    let mut attempt = 0;

    loop {
        match register(clsid, factory) {
            Ok(reg) => return Ok(reg),

            Err(err) => {
                attempt += 1;

                trace_event!("Registration failed — retrying");

                if attempt >= retries {
                    trace_event!("Registration retries exhausted");
                    return Err(err);
                }

                thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

// ============================================================
// GUID parsing
// ============================================================

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

// ============================================================
// Launch detection
// ============================================================

pub fn is_background_activation_launch() -> bool {
    std::env::args().any(|a| a == "----BackgroundActivated")
}

// ============================================================
// Full hardened activation entrypoint
// ============================================================

pub fn run_background_activation(clsid: &GUID, factory: &IUnknown) -> windows::core::Result<()> {
    if !is_background_activation_launch() {
        return Ok(());
    }

    trace_event!("Background activation detected");

    // ------------------------------------------------------------
    // Prevent multi-instance collision
    // ------------------------------------------------------------

    let _instance = InstanceGuard::acquire("Global\\Tauri.Notification.COM")?;

    // ------------------------------------------------------------
    // Start watchdog
    // ------------------------------------------------------------

    let watchdog = ActivationWatchdog::start(5000);

    // ------------------------------------------------------------
    // Register COM
    // ------------------------------------------------------------

    let registration = register_with_retry(clsid, factory, 3)?;

    {
        let mut slot = registration_slot().lock().unwrap();

        *slot = Some(registration);
    }

    trace_event!("COM registration stored in global slot");

    // ------------------------------------------------------------
    // Run message pump
    // ------------------------------------------------------------

    let cancel = cancel_token().clone();
    cancel.store(false, Ordering::SeqCst);

    run_pump_with_cancel(5000, cancel.clone());

    // ------------------------------------------------------------
    // Cleanup coordination
    // ------------------------------------------------------------

    cancel.store(true, Ordering::SeqCst);

    watchdog.cancel();

    trace_event!("Activation completed");

    Ok(())
}

// Inside your lib.rs or main.rs setup
pub fn run_background_activation_loop(clsid: &GUID) -> windows::core::Result<()> {
    // 1. Create the factory instance
    let factory: IUnknown = NotificationActivatorFactory.into();

    // 2. Pass it to your existing hardened runner
    run_background_activation(clsid, &factory)
}

/// Register the COM class object for the **foreground** process.
///
/// The foreground app is already running — its own Tauri/tao event loop keeps
/// the process alive, so no blocking message pump is needed.  We only need
/// `CoRegisterClassObject` so that Windows can deliver `Activate()` when the
/// user clicks a `activationType="background"` toast action button while the
/// app is in the foreground.
///
/// The registration is stored in the global slot and revoked by
/// `plugin_unregister()` on `RunEvent::Exit`, identical to the background path.
///
/// # Why this was missing
///
/// `run_background_activation` returns immediately when
/// `is_background_activation_launch()` is false, so calling it from the
/// foreground path never reached `CoRegisterClassObject`.  Without registration,
/// Windows had nowhere to deliver `Activate()` and toast action buttons were
/// silently ignored while the app was running.
pub fn register_foreground(clsid: &GUID) -> windows::core::Result<()> {
    if is_background_activation_launch() {
        // This function is only for the foreground path.
        return Ok(());
    }

    trace_event!("Foreground COM registration starting");

    let factory: IUnknown = NotificationActivatorFactory.into();

    let registration = register_with_retry(clsid, &factory, 3)?;

    {
        let mut slot = registration_slot().lock().unwrap();
        *slot = Some(registration);
    }

    trace_event!("Foreground COM registration stored in global slot");

    Ok(())
}

pub fn plugin_unregister() -> windows::core::Result<()> {
    trace_event!("Plugin unregister initiated");

    // ------------------------------------------------------------
    // 1. Stop message pump
    // ------------------------------------------------------------

    if let Some(cancel) = GLOBAL_CANCEL.get() {
        cancel.store(true, Ordering::SeqCst);

        trace_event!("Message pump cancellation requested");
    }

    // ------------------------------------------------------------
    // 2. Flush activation queue safely
    // ------------------------------------------------------------

    if let Err(e) = crate::windows_platform::activation_queue::flush() {
        trace_event!("Queue flush failed during shutdown");
        log::error!("[notification] queue flush error: {}", e);
    } else {
        trace_event!("Queue flushed successfully");
    }

    // ------------------------------------------------------------
    // 3. Stop worker thread
    // ------------------------------------------------------------

    crate::windows_platform::activation_queue::shutdown_worker();

    trace_event!("Worker shutdown requested");

    // ------------------------------------------------------------
    // 4. Revoke COM class registration
    // ------------------------------------------------------------

    {
        let mut slot = registration_slot().lock().unwrap();

        if let Some(registration) = slot.take() {
            drop(registration);

            trace_event!("COM registration dropped");
        } else {
            trace_event!("No active COM registration");
        }
    }

    // ------------------------------------------------------------
    // 5. Final state reset
    // ------------------------------------------------------------

    CLASS_REGISTERED.store(false, Ordering::SeqCst);

    trace_event!("Plugin unregister completed");

    Ok(())
}
