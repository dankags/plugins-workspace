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
    let coord = COORDINATOR
        .get()
        .expect("ShutdownCoordinator not initialized");

    let mut done = coord.completed.lock().unwrap();
    *done = true;

    coord.condvar.notify_all();
}

pub fn wait_for_completion(timeout: Duration) -> bool {
    let coord = COORDINATOR
        .get()
        .expect("ShutdownCoordinator not initialized");

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

    // if let Err(e) = crate::windows_platform::com_activator::plugin_unregister() {
    //     log::error!("[notification] COM unregister failed: {}", e);
    // }

    log::debug!("[notification] background shutdown complete");

    std::process::exit(0);
}
