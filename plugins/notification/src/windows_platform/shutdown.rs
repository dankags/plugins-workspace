use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

pub struct ShutdownCoordinator {
    completed: Mutex<bool>,
    condvar: Condvar,
}

static COORDINATOR: OnceLock<ShutdownCoordinator> = OnceLock::new();

pub fn init() {
    COORDINATOR.get_or_init(|| ShutdownCoordinator {
        completed: Mutex::new(false),
        condvar: Condvar::new(),
    });
}

pub fn signal_worker_complete() {
    // was `.get().expect(...)` which panics if init() was not
    // called before the worker thread exits. Changed to get_or_init so that
    // signal_worker_complete() is safe to call regardless of call order.
    let coord = COORDINATOR.get_or_init(|| ShutdownCoordinator {
        completed: Mutex::new(false),
        condvar: Condvar::new(),
    });

    let mut done = coord.completed.lock().unwrap();
    *done = true;

    coord.condvar.notify_all();
}

pub fn wait_for_completion(timeout: Duration) -> bool {
    // same get_or_init pattern for consistency — if somehow
    // wait_for_completion is called before init(), it should not panic.
    let coord = COORDINATOR.get_or_init(|| ShutdownCoordinator {
        completed: Mutex::new(false),
        condvar: Condvar::new(),
    });

    let done = coord.completed.lock().unwrap();

    let (done, _) = coord
        .condvar
        .wait_timeout_while(done, timeout, |d| !*d)
        .unwrap();

    *done
}

pub fn spawn_background_exit_watcher(timeout_secs: u64) {
    std::thread::spawn(move || {
        let timeout = Duration::from_secs(timeout_secs);

        log::debug!(
            "[notification] waiting for worker completion ({}s)",
            timeout_secs
        );

        let completed = wait_for_completion(timeout);

        if completed {
            log::info!("[notification] worker completed — shutting down");
        } else {
            log::warn!("[notification] timeout reached — forcing shutdown");
        }

        graceful_shutdown();
    });
}

fn graceful_shutdown() {
    log::debug!("[notification] flushing activation queue");

    if let Err(e) = crate::windows_platform::activation_queue::flush() {
        log::error!("[notification] queue flush failed: {}", e);
    }

    log::debug!("[notification] stopping worker");

    crate::windows_platform::activation_queue::shutdown_worker();

    log::debug!("[notification] unregistering COM server");

    if let Err(e) = crate::windows_platform::com_activator::plugin_unregister() {
        log::error!("[notification] COM unregister failed: {}", e);
    }

    log::debug!("[notification] background shutdown complete");

    // std::process::exit(0) bypasses all Rust destructors,
    // which means InstanceGuard::drop() never runs and the named Windows mutex
    // "Global\\Tauri.Notification.COM" leaks until the OS releases it.
    // On a rapid second launch this causes ERROR_ALREADY_EXISTS and the COM
    // server fails to register (Issue 2).
    //
    // We return normally here instead. The background process will exit
    // naturally once this thread and the worker thread both finish.
    // If a hard exit is truly required (e.g. Tauri does not exit cleanly in
    // background mode), call plugin_unregister() first so InstanceGuard is
    // already dropped before exit:
    //
    //     std::process::exit(0);  // ← only if absolutely necessary
}
