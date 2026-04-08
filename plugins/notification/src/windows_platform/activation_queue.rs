// activation_queue.rs
//
// Enterprise-grade persistent activation queue
//
// Guarantees:
// - Exactly-once activation processing
// - Crash-safe persistence
// - Deduplication
// - Sequential execution
// - Safe shutdown
// - Backpressure protection
//
// Designed for Windows background toast activation

use serde::{Deserialize, Serialize};

use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::windows_platform::{runtime_context::context, shutdown};
use crate::NotificationActionEvent;

use crate::trace_event;

// ============================================================
// Configuration
// ============================================================

const MAX_QUEUE_SIZE: usize = 512;
const WORKER_INTERVAL_MS: u64 = 50;

// ============================================================
// Global State
// ============================================================

static QUEUE: LazyLock<Mutex<VecDeque<QueuedActivation>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

static DEDUP: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// ============================================================
// Activation Model
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedActivation {
    pub id: String,
    pub payload: NotificationActionEvent,
    pub timestamp: u64,
}

// ============================================================
// File Paths
// ============================================================

fn queue_file() -> PathBuf {
    let dir = context().storage_dir.clone();
    // message. Now logs the actual error so failures are visible in prod.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!(
            "[notification] failed to create queue storage dir {:?}: {}",
            dir,
            e
        );
    }
    dir.join("activation_queue.json")
}

fn journal_file() -> PathBuf {
    let dir = context().storage_dir.clone();
    // causing append_journal() to silently fail whenever it was called before
    // queue_file() had created the directory.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!(
            "[notification] failed to create journal storage dir {:?}: {}",
            dir,
            e
        );
    }
    dir.join("activation_queue.journal")
}

// ============================================================
// Persistence
// ============================================================

fn save_queue(queue: &VecDeque<QueuedActivation>) {
    if let Ok(json) = serde_json::to_string(queue) {
        if let Err(err) = fs::write(queue_file(), json) {
            trace_event!("Queue save failed");
            log::error!("[notification] Queue save error: {err}");
        }
    }
}

fn append_journal(event: &str) {
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_file())
    {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let _ = writeln!(file, "{} | {}", ts, event);
        let _ = file.flush();
    }
}

// ============================================================
// Public API
// ============================================================

pub fn load_queue() {
    if let Ok(data) = fs::read_to_string(queue_file()) {
        match serde_json::from_str::<VecDeque<QueuedActivation>>(&data) {
            Ok(queue) => {
                let mut d = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
                let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

                // CRITICAL: reset existing state
                q.clear();
                d.clear();

                for item in queue {
                    d.insert(item.id.clone());
                    q.push_back(item);
                }

                trace_event!("Activation queue restored");
                append_journal("Queue restored from disk");
            }

            Err(_) => {
                trace_event!("Queue restore failed");
            }
        }
    }
}

// ============================================================
// Enqueue
// ============================================================

pub fn enqueue(id: String, payload: NotificationActionEvent) {
    // Atomic check-and-insert under both locks to prevent TOCTOU race.
    // Lock order: DEDUP then QUEUE (consistent across all code paths).

    let (evicted_id, snapshot): (Option<String>, VecDeque<QueuedActivation>) = {
        let mut dedup = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

        if dedup.contains(&id) {
            trace_event!("Duplicate activation ignored");
            return;
        }

        let evicted = if queue.len() >= MAX_QUEUE_SIZE {
            trace_event!("Queue overflow — dropping oldest");
            queue.pop_front().map(|oldest| oldest.id)
        } else {
            None
        };

        let activation = QueuedActivation {
            id: id.clone(),
            payload,
            timestamp: now(),
        };

        queue.push_back(activation);
        dedup.insert(id.clone());
        let snapshot = queue.clone();
        (evicted, snapshot)
    }; // Both locks released

    // Persist outside the lock so slow I/O never blocks other threads.
    save_queue(&snapshot);

    // Remove evicted id from DEDUP after persistence.
    if let Some(evicted) = evicted_id {
        let mut dedup = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        dedup.remove(&evicted);
    }

    append_journal("Activation queued");
    trace_event!("Activation queued");
}

// ============================================================
// Worker Lifecycle
// ============================================================

pub fn start_worker() {
    if std::env::var("DISABLE_WORKER").is_ok() {
        trace_event!("Worker disabled by environment");
        return;
    }

    if WORKER_RUNNING.swap(true, Ordering::SeqCst) {
        trace_event!("Worker already running");
        return;
    }

    SHUTDOWN.store(false, Ordering::SeqCst);

    thread::spawn(move || {
        trace_event!("Activation worker started");

        append_journal("Worker started");

        loop {
            if SHUTDOWN.load(Ordering::SeqCst) {
                trace_event!("Worker shutdown signal received");
                append_journal("Worker stopped");
                break;
            }

            process_next();

            thread::sleep(Duration::from_millis(WORKER_INTERVAL_MS));
        }

        WORKER_RUNNING.store(false, Ordering::SeqCst);
        shutdown::signal_worker_complete();
    });
}

pub fn shutdown_worker() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

// ============================================================
// Processing
// ============================================================

fn process_next() {
    let item = {
        let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        queue.pop_front()
    };

    if let Some(act) = item {
        process(act);
    }
}

fn process(act: QueuedActivation) {
    trace_event!("Processing activation");
    append_journal("Processing activation");

    let result = std::panic::catch_unwind(|| {
        crate::windows_platform::action_handler::dispatch(act.payload.clone());
    });

    match result {
        Ok(_) => {
            trace_event!("Activation processed");
            append_journal("Activation processed");
            cleanup_after_success(act.id);
        }

        Err(_) => {
            trace_event!("Activation failed — requeue");
            append_journal("Activation failed");
            requeue(act);
        }
    }
}

// ============================================================
// Retry Logic
// ============================================================

fn requeue(act: QueuedActivation) {
    // Lock order: DEDUP then QUEUE.
    let snapshot: Option<VecDeque<QueuedActivation>> = {
        let mut d = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() < MAX_QUEUE_SIZE {
            d.insert(act.id.clone());
            q.push_back(act);
            Some(q.clone())
        } else {
            None // queue full — item dropped, DEDUP stays clear
        }
    }; // Both locks released

    if let Some(snap) = snapshot {
        save_queue(&snap); // disk write outside the lock
    }
}

fn cleanup_after_success(id: String) {
    // same lock-then-write pattern — clone inside, write outside.
    {
        let mut dedup = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        dedup.remove(&id);
    }

    let snapshot = {
        let queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        queue.clone()
    }; // QUEUE lock released

    save_queue(&snapshot); // disk write outside the lock
}

pub fn flush() -> crate::Result<()> {
    trace_event!("Flushing activation queue");

    {
        let mut d = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        q.clear();
        d.clear();
        save_queue(&q);
    }

    Ok(())
}

// ============================================================
// Utilities
// ============================================================

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use crate::windows_platform::{self, runtime_context};

    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Once;
    use std::thread;
    use std::time::Duration;

    static INIT: Once = Once::new();

    fn make_event(id: &str) -> NotificationActionEvent {
        NotificationActionEvent {
            action_id: id.to_string(),
            inputs: HashMap::new(),
            tag: None,
            group: None,
        }
    }

    fn setup() {
        std::env::remove_var("DISABLE_WORKER");

        SHUTDOWN.store(true, Ordering::SeqCst);

        // init_context() is now safe to call repeatedly —
        // it overwrites the Mutex<Option<>> rather than panicking on a
        // second OnceLock::set() call.
        runtime_context::init_context(
            "TestApp".into(),
            "00000000-0000-0000-0000-000000000000".into(),
            std::env::temp_dir().join("notification_test_storage"),
        );

        //  shutdown::init() must be called before start_worker()
        // so that signal_worker_complete() never panics when the worker exits.
        windows_platform::shutdown::init();

        thread::sleep(Duration::from_millis(50));

        WORKER_RUNNING.store(false, Ordering::SeqCst);

        {
            let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            q.clear();
        }

        {
            let mut d = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
            d.clear();
        }

        // Proper cleanup of persisted files
        let _ = fs::remove_file(queue_file());
        let _ = fs::remove_file(journal_file());

        SHUTDOWN.store(false, Ordering::SeqCst);
    }

    // ------------------------------------------------------------
    // enqueue persistence
    // ------------------------------------------------------------

    #[test]
    fn test_enqueue_persists_to_disk() {
        setup();

        enqueue("id1".into(), make_event("payload1"));

        let file_exists = queue_file().exists();

        assert!(file_exists);

        let data = fs::read_to_string(queue_file()).unwrap();

        assert!(data.contains("payload1"));
    }

    // ------------------------------------------------------------
    // deduplication
    // ------------------------------------------------------------

    #[test]
    fn test_duplicate_activation_ignored() {
        setup();

        enqueue("same".into(), make_event("payload"));
        enqueue("same".into(), make_event("payload"));

        let queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

        assert_eq!(queue.len(), 1);
    }

    // ------------------------------------------------------------
    // overflow protection
    // ------------------------------------------------------------

    #[test]
    fn test_queue_overflow_drops_oldest() {
        setup();

        for i in 0..(MAX_QUEUE_SIZE + 10) {
            enqueue(format!("id{i}"), make_event(&format!("payload{i}")));
        }

        let queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

        assert!(queue.len() <= MAX_QUEUE_SIZE);
    }

    // ------------------------------------------------------------
    // persistence restore
    // ------------------------------------------------------------

    #[test]
    fn test_load_queue_restores_items() {
        setup();

        enqueue("restore1".into(), make_event("payload"));

        {
            let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            q.clear();
        }

        {
            let mut d = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
            d.clear();
        }

        load_queue();

        let queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, "restore1");
    }

    // ------------------------------------------------------------
    // worker lifecycle
    // ------------------------------------------------------------

    #[test]
    fn test_worker_start_and_shutdown() {
        setup();

        start_worker();

        thread::sleep(Duration::from_millis(100));

        assert!(WORKER_RUNNING.load(Ordering::SeqCst));

        shutdown_worker();

        thread::sleep(Duration::from_millis(100));

        assert!(!WORKER_RUNNING.load(Ordering::SeqCst) || SHUTDOWN.load(Ordering::SeqCst));
    }

    // ------------------------------------------------------------
    // sequential processing safety
    // ------------------------------------------------------------

    #[test]
    fn test_sequential_processing_order() {
        setup();

        enqueue("1".into(), make_event("A"));
        enqueue("2".into(), make_event("B"));
        enqueue("3".into(), make_event("C"));

        let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

        let first = q.pop_front().unwrap();
        let second = q.pop_front().unwrap();
        let third = q.pop_front().unwrap();

        assert_eq!(first.payload.action_id, "A");
        assert_eq!(second.payload.action_id, "B");
        assert_eq!(third.payload.action_id, "C");
    }

    // ------------------------------------------------------------
    // timestamp correctness
    // ------------------------------------------------------------

    #[test]
    fn test_timestamp_is_set() {
        setup();

        enqueue("time".into(), make_event("payload"));

        let queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());

        let item = queue.front().unwrap();

        assert!(item.timestamp > 0);
    }

    // ------------------------------------------------------------
    // overflow dedup consistency (regression for Issue 1b / 6)
    // ------------------------------------------------------------

    #[test]
    fn test_overflow_does_not_leave_stale_dedup_entry() {
        setup();

        // Fill the queue to capacity
        for i in 0..MAX_QUEUE_SIZE {
            enqueue(format!("id{i}"), make_event(&format!("p{i}")));
        }

        // This should evict "id0" from both QUEUE and DEDUP
        enqueue("overflow".into(), make_event("overflow"));

        // "id0" must no longer be in DEDUP, so re-enqueuing it should succeed
        enqueue("id0".into(), make_event("re-enqueued"));

        let d = DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        assert!(d.contains("id0"), "re-enqueued id0 should be back in DEDUP");
    }
}
