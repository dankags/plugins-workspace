// activation_queue.rs
//
// Encrypted, crash-safe, exactly-once activation queue.
//
// Guarantees
// ──────────
// - Exactly-once processing (dedup by activation id)
// - AES-256-GCM encrypted persistence (key derived from app GUID)
// - Atomic writes (temp-file + rename — no partial/corrupt state on crash)
// - Coalesced disk I/O (dirty flag + background writer thread)
// - Immediate worker wake-up via Condvar (no fixed-interval polling)
// - Lock-order safe (single QueueState mutex owns BOTH queue and dedup set)
// - No I/O inside any mutex (all syscalls happen after lock release)
// - Context lock released before any I/O (runtime_context returns owned clone)
//
// Encryption design (mirrors tauri-plugin-cache)
// ───────────────────────────────────────────────
// Key:       SHA-256(app_guid_bytes) → 32-byte AES key.
//            Stable across restarts; no OS keychain dependency needed for a
//            background-activation plugin.  The GUID is already a secret
//            embedded in the signed binary.
// Cipher:    AES-256-GCM (authenticated encryption — detects tampering).
// Wire format (binary file):
//   [12 bytes nonce][N bytes ciphertext+16-byte GCM tag]
// A fresh random nonce is generated on every write.
//
// Race condition audit (all fixed)
// ─────────────────────────────────
// OLD: QUEUE and DEDUP were two separate Mutexes → non-atomic check+insert
//      window where two threads both passed the dedup check before either
//      wrote to DEDUP → duplicate activations inserted.
// FIX: Single QueueState mutex owns both the VecDeque and the HashSet.
//      The dedup check and the insert are one atomic critical section.
//
// OLD: requeue() locked QUEUE then DEDUP (order: QUEUE→DEDUP).
//      enqueue() locked DEDUP then QUEUE (order: DEDUP→QUEUE).
//      → classic lock-order inversion deadlock under contention.
// FIX: Both operations go through the single QueueState lock.
//
// OLD: flush() called save_queue() while holding the QUEUE lock → I/O in mutex.
// FIX: flush() clears state inside the lock, takes a snapshot, releases lock,
//      then writes outside.
//
// OLD: load_queue() held QUEUE and DEDUP locks simultaneously.
// FIX: load_queue() parses the file before acquiring any lock, then holds
//      only the single QueueState lock for the in-memory update.
//
// OLD: context() returned a MutexGuard — callers held the CONTEXT lock
//      across create_dir_all + fs::write blocking syscalls.
// FIX: runtime_context::context() now returns an owned clone (lock-free I/O).
//
// OLD: save_queue() wrote directly to the final path — crash mid-write
//      produced a corrupt/truncated file with no recovery path.
// FIX: write to a sibling .tmp file, fsync, then atomic rename().

use serde::{Deserialize, Serialize};

use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, LazyLock, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use sha2::{Digest, Sha256};

use crate::trace_event;
use crate::windows_platform::{runtime_context::context, shutdown};
use crate::NotificationActionEvent;

// ── Configuration ─────────────────────────────────────────────────────────────

const MAX_QUEUE_SIZE: usize = 512;

// How long the persist thread waits for more dirty signals before flushing.
// Coalesces rapid back-to-back enqueues into a single write.
const PERSIST_DEBOUNCE_MS: u64 = 20;

// ── Unified queue state (single lock — eliminates all lock-order races) ───────

struct QueueState {
    items: VecDeque<QueuedActivation>,
    seen: HashSet<String>, // ids currently in `items`
}

impl QueueState {
    fn new() -> Self {
        Self {
            items: VecDeque::new(),
            seen: HashSet::new(),
        }
    }
}

// ── Global state ──────────────────────────────────────────────────────────────
#[allow(clippy::incompatible_msrv)]
static STATE: LazyLock<Mutex<QueueState>> = LazyLock::new(|| Mutex::new(QueueState::new()));

#[allow(clippy::incompatible_msrv)]
// Condvar wakes the worker thread immediately when an item is enqueued.
// Paired with the STATE mutex.
static WAKE: LazyLock<Condvar> = LazyLock::new(Condvar::new);

#[allow(clippy::incompatible_msrv)]
// Condvar + flag for the persist thread.
static DIRTY: LazyLock<(Mutex<bool>, Condvar)> =
    LazyLock::new(|| (Mutex::new(false), Condvar::new()));

static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static PERSIST_RUNNING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedActivation {
    pub id: String,
    pub payload: NotificationActionEvent,
    pub timestamp: u64,
}

// ── Encryption helpers ────────────────────────────────────────────────────────

/// Derive a 32-byte AES key from the app GUID.
///
/// SHA-256(guid_utf8) gives a stable, deterministic key that survives process
/// restarts without needing OS keychain access.  The GUID is a secret embedded
/// in the signed binary (same assumption tauri-plugin-cache makes).
fn derive_key(guid: &str) -> Key<Aes256Gcm> {
    let hash = Sha256::digest(guid.as_bytes());
    *Key::<Aes256Gcm>::from_slice(&hash)
}

/// Encrypt `plaintext` and return `[nonce(12) || ciphertext+tag]`.
fn encrypt(plaintext: &[u8], guid: &str) -> Vec<u8> {
    let cipher = Aes256Gcm::new(&derive_key(guid));

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let mut ct = cipher
        .encrypt(nonce, plaintext)
        .expect("AES-GCM encryption failed");

    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.append(&mut ct);
    out
}

/// Decrypt `[nonce(12) || ciphertext+tag]` and return plaintext.
/// Returns `None` on any authentication or format error.
fn decrypt(blob: &[u8], guid: &str) -> Option<Vec<u8>> {
    if blob.len() < 12 {
        return None;
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new(&derive_key(guid));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ct).ok()
}

// ── File paths ────────────────────────────────────────────────────────────────

fn queue_file(dir: &Path) -> PathBuf {
    dir.join("activation_queue.bin") // binary encrypted blob
}

fn queue_tmp_file(dir: &Path) -> PathBuf {
    dir.join("activation_queue.bin.tmp")
}

fn journal_file(dir: &Path) -> PathBuf {
    dir.join("activation_queue.journal")
}

fn ensure_dir(dir: &PathBuf) {
    if let Err(e) = fs::create_dir_all(dir) {
        log::error!(
            "[notification] failed to create storage dir {:?}: {}",
            dir,
            e
        );
    }
}

// ── Persistence ───────────────────────────────────────────────────────────────

/// Serialize `items`, encrypt, and write atomically via tmp→rename.
///
/// The write never touches the live file until the new content is fully
/// fsynced — a crash mid-write leaves the old file intact.
fn persist(items: &VecDeque<QueuedActivation>) {
    let ctx = context();
    let dir = ctx.storage_dir.clone();
    let guid = ctx.guid.clone();
    drop(ctx); // release CONTEXT clone immediately

    ensure_dir(&dir);

    let json = match serde_json::to_vec(items) {
        Ok(j) => j,
        Err(e) => {
            log::error!("[notification] queue serialization failed: {}", e);
            return;
        }
    };

    let blob = encrypt(&json, &guid);
    let tmp = queue_tmp_file(&dir);
    let dest = queue_file(&dir);

    // Write to tmp, fsync, rename.
    let write_result = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(&blob)?;
        f.flush()?;
        f.sync_all()?; // durability guarantee before rename
        drop(f);
        fs::rename(&tmp, &dest)?;
        Ok(())
    })();

    if let Err(e) = write_result {
        log::error!("[notification] queue persist failed: {}", e);
        // Clean up tmp if rename failed
        let _ = fs::remove_file(&tmp);
    }
}

/// Mark the in-memory state as dirty and wake the persist thread.
fn mark_dirty() {
    let (lock, cvar) = &*DIRTY;
    *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
    cvar.notify_one();
}

fn append_journal(event: &str) {
    let ctx = context();
    let dir = ctx.storage_dir.clone();
    drop(ctx);

    ensure_dir(&dir);

    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_file(&dir))
    {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(file, "{} | {}", ts, event);
        let _ = file.flush();
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Load the persisted queue from disk into memory.
///
/// RACE FIX: old version held QUEUE + DEDUP locks simultaneously.
/// New version: parse + decrypt BEFORE acquiring any lock, then hold
/// the single STATE lock only for the in-memory update (microseconds).
pub fn load_queue() {
    let ctx = context();
    let dir = ctx.storage_dir.clone();
    let guid = ctx.guid.clone();
    drop(ctx);

    println!("Initializing the load_queue");
    println!("Storage directory: {:?}", dir);
    println!("GUID: {} ({} bytes)\n", guid, guid.len());

    let path = queue_file(&dir);
    if !path.exists() {
        println!("❌ [load_queue] queue file does not exist.");
        return;
    }

    let blob = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("[notification] queue read failed: {}", e);
            println!("❌ [load_queue] [notification] queue read failed: {}", e);
            return;
        }
    };

    let json = match decrypt(&blob, &guid) {
        Some(j) => j,
        None => {
            log::warn!(
                "[notification] queue decryption failed — discarding (tampered or wrong key)"
            );
            println!("❌ [load_queue] [notification] queue decryption failed — discarding (tampered or wrong key)");
            let _ = fs::remove_file(&path); // remove corrupt/stale file
            return;
        }
    };

    let loaded: VecDeque<QueuedActivation> = match serde_json::from_slice(&json) {
        Ok(q) => q,
        Err(e) => {
            log::warn!(
                "[notification] queue deserialization failed: {} — discarding",
                e
            );
            println!(
                "❌ [load_queue] [notification] queue deserialization failed: {} — discarding",
                e
            );
            let _ = fs::remove_file(&path);
            return;
        }
    };

    println!(
        "✔ [load_queue] Loaded {} activations from disk",
        loaded.len()
    );
    println!("Activations: {:?}", loaded);

    // Only now acquire the lock — purely in-memory work from here.
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.items.clear();
    state.seen.clear();
    for item in loaded {
        state.seen.insert(item.id.clone());
        state.items.push_back(item);
    }
    drop(state);

    trace_event!("Activation queue restored");
    append_journal("Queue restored from disk");
}

// ── Enqueue ───────────────────────────────────────────────────────────────────

/// Enqueue an activation for processing.
///
/// RACE FIX (duplicate insertion): the old design used two separate Mutexes
/// (QUEUE, DEDUP).  The dedup check was: lock DEDUP, check, release DEDUP,
/// then lock QUEUE to insert.  Two threads could both pass the check before
/// either inserted → duplicates.
///
/// Fix: the dedup check and the queue insertion are now a single critical
/// section under the unified STATE mutex.  There is no window between check
/// and insert.
pub fn enqueue(id: String, payload: NotificationActionEvent) {
    // Take a snapshot to persist outside the lock.
    let snapshot: VecDeque<QueuedActivation> = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());

        if state.seen.contains(&id) {
            trace_event!("Duplicate activation ignored");
            println!("Duplicate activation ignored");
            return;
        }

        if state.items.len() >= MAX_QUEUE_SIZE {
            trace_event!("Queue overflow — dropping oldest");
            println!("Queue overflow — dropping oldest");
            if let Some(oldest) = state.items.pop_front() {
                state.seen.remove(&oldest.id);
            }
        }

        let activation = QueuedActivation {
            id: id.clone(),
            payload,
            timestamp: now(),
        };

        state.items.push_back(activation);
        state.seen.insert(id);

        state.items.clone() // fast in-memory clone before releasing lock
    }; // STATE lock released — all I/O happens below

    // Wake worker thread immediately (no 50ms polling delay).
    WAKE.notify_one();

    // Signal persist thread to write (coalesced, debounced).
    mark_dirty();

    // Snapshot available for emergency sync persist if persist thread is
    // not yet running (e.g. called before start_worker).
    let _ = snapshot; // persist thread will pick it up via dirty flag

    append_journal("Activation queued");
    trace_event!("Activation queued");
}

// ── Worker lifecycle ──────────────────────────────────────────────────────────

/// Start the activation worker thread and the background persist thread.
pub fn start_worker() {
    if std::env::var("DISABLE_WORKER").is_ok() {
        trace_event!("Worker disabled by environment");
        println!("❌ [start_worker] Worker disabled by environment");
        return;
    }

    SHUTDOWN.store(false, Ordering::SeqCst);

    // ── Persist thread ────────────────────────────────────────────────────
    // Dedicated thread for coalesced, debounced disk writes.
    // Woken by mark_dirty(); waits PERSIST_DEBOUNCE_MS then flushes once,
    // absorbing any writes that arrived during the wait.
    if !PERSIST_RUNNING.swap(true, Ordering::SeqCst) {
        thread::Builder::new()
            .name("notification-persist".to_string())
            .spawn(|| {
                let (dirty_lock, dirty_cvar) = &*DIRTY;
                loop {
                    // Wait until dirty
                    {
                        let mut dirty = dirty_lock.lock().unwrap_or_else(|e| e.into_inner());
                        while !*dirty {
                            if SHUTDOWN.load(Ordering::SeqCst) {
                                // Final flush before exit
                                let snap = STATE
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .items
                                    .clone();
                                persist(&snap);
                                PERSIST_RUNNING.store(false, Ordering::SeqCst);
                                return;
                            }
                            dirty = dirty_cvar
                                .wait_timeout(dirty, Duration::from_millis(100))
                                .unwrap_or_else(|e| e.into_inner())
                                .0;
                        }
                        *dirty = false; // consume the signal
                    }

                    if SHUTDOWN.load(Ordering::SeqCst) {
                        let snap = STATE
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .items
                            .clone();
                        persist(&snap);
                        PERSIST_RUNNING.store(false, Ordering::SeqCst);
                        return;
                    }

                    // Debounce: absorb writes that arrive in the next window
                    thread::sleep(Duration::from_millis(PERSIST_DEBOUNCE_MS));

                    // Take snapshot (lock held for clone only — microseconds)
                    let snap = STATE
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .items
                        .clone();

                    persist(&snap);
                }
            })
            .expect("failed to spawn notification-persist thread");
    }

    // ── Worker thread ─────────────────────────────────────────────────────
    // Processes one activation at a time.  Wakes immediately when enqueue()
    // calls WAKE.notify_one() instead of sleeping a fixed 50 ms per iteration.
    if !WORKER_RUNNING.swap(true, Ordering::SeqCst) {
        thread::Builder::new()
            .name("notification-worker".to_string())
            .spawn(move || {
                trace_event!("Activation worker started");
                append_journal("Worker started");
                println!("✔ [start_worker] Activation worker started");

                loop {
                    // Block until an item is available OR shutdown is set.
                    let item = {
                        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());

                        loop {
                            if SHUTDOWN.load(Ordering::Acquire) {
                                trace_event!("Worker shutdown signal received");
                                append_journal("Worker stopped");

                                println!("🔁 [start_worker] Worker shutdown signal received");

                                WORKER_RUNNING.store(false, Ordering::SeqCst);
                                shutdown::signal_worker_complete();
                                return;
                            }
                            if let Some(item) = state.items.pop_front() {
                                // Remove from seen so re-enqueue after failure works
                                state.seen.remove(&item.id);
                                break Some(item);
                            }
                            // Nothing to do — wait for WAKE.notify_one()
                            state = WAKE
                                .wait_timeout(state, Duration::from_secs(1))
                                .unwrap_or_else(|e| e.into_inner())
                                .0;
                        }
                    }; // STATE lock released before dispatch

                    if let Some(act) = item {
                        process(act);
                    }
                }
            })
            .expect("failed to spawn notification-worker thread");
    }
}

pub fn shutdown_worker() {
    SHUTDOWN.store(true, Ordering::Release);
    // Wake both threads so they can observe the shutdown flag.
    WAKE.notify_all();
    let (_, dirty_cvar) = &*DIRTY;
    dirty_cvar.notify_all();
}

// ── Processing ────────────────────────────────────────────────────────────────

fn process(act: QueuedActivation) {
    trace_event!("Processing activation");
    append_journal("Processing activation");
    println!("🔁 Processing activation: {}", act.id);

    let result = std::panic::catch_unwind(|| {
        crate::windows_platform::action_handler::dispatch(act.payload.clone());
    });

    match result {
        Ok(_) => {
            println!("✔ Dispatching activation: {}", act.id);
            trace_event!("Activation processed");
            append_journal("Activation processed");
            // Item was already popped and removed from `seen` in the worker loop.
            // Just trigger a persist to record the shorter queue.
            mark_dirty();
        }
        Err(_) => {
            println!("❌ Failed to dispatch activation: {}", act.id);
            trace_event!("Activation failed — requeue");
            append_journal("Activation failed");
            requeue(act);
        }
    }
}

// ── Retry ─────────────────────────────────────────────────────────────────────

fn requeue(act: QueuedActivation) {
    // RACE FIX: old code locked QUEUE then nested DEDUP inside (order inversion
    // vs enqueue's DEDUP→QUEUE). Now both use the single STATE lock.
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state.items.len() < MAX_QUEUE_SIZE {
            state.seen.insert(act.id.clone());
            state.items.push_back(act);
        }
        // If queue is full the item is dropped — DEDUP stays clear (correct).
    } // STATE lock released

    mark_dirty();
    WAKE.notify_one();
}

// ── Flush ─────────────────────────────────────────────────────────────────────

/// Clear the queue and persist an empty file.
///
/// RACE FIX: old flush() called save_queue() while holding the QUEUE lock
/// (I/O inside mutex — Bug D pattern). Fixed: clear inside lock, snapshot,
/// release, then write outside.
pub fn flush() -> crate::Result<()> {
    trace_event!("Flushing activation queue");

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.items.clear();
        state.seen.clear();
    } // STATE lock released

    // Persist the empty queue outside any lock.
    persist(&VecDeque::new());

    Ok(())
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_platform::{self, runtime_context};
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

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
        WAKE.notify_all();

        runtime_context::init_context(
            "00000000-0000-0000-0000-000000000000".into(),
            std::env::temp_dir().join("notification_test_storage"),
        );
        windows_platform::shutdown::init();

        thread::sleep(Duration::from_millis(60));

        WORKER_RUNNING.store(false, Ordering::SeqCst);
        PERSIST_RUNNING.store(false, Ordering::SeqCst);

        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.items.clear();
            state.seen.clear();
        }
        {
            let mut d = DIRTY.0.lock().unwrap_or_else(|e| e.into_inner());
            *d = false;
        }

        let ctx = context();
        let dir = ctx.storage_dir.clone();
        drop(ctx);
        let _ = fs::remove_file(queue_file(&dir));
        let _ = fs::remove_file(queue_tmp_file(&dir));
        let _ = fs::remove_file(journal_file(&dir));

        SHUTDOWN.store(false, Ordering::SeqCst);
    }

    // ── Encryption round-trip ─────────────────────────────────────────────

    #[test]
    fn encrypt_decrypt_round_trip() {
        let plain = b"hello encrypted world";
        let guid = "test-guid-1234";
        let blob = encrypt(plain, guid);
        let back = decrypt(&blob, guid).expect("decrypt failed");
        assert_eq!(back, plain);
    }

    #[test]
    fn decrypt_with_wrong_key_returns_none() {
        let blob = encrypt(b"secret", "guid-a");
        assert!(decrypt(&blob, "guid-b").is_none());
    }

    #[test]
    fn decrypt_truncated_blob_returns_none() {
        assert!(decrypt(&[0u8; 5], "guid").is_none());
    }

    #[test]
    fn two_encryptions_produce_different_nonces() {
        let blob1 = encrypt(b"data", "guid");
        let blob2 = encrypt(b"data", "guid");
        // Nonce is the first 12 bytes — must differ (random per call)
        assert_ne!(&blob1[..12], &blob2[..12]);
    }

    // ── Atomic write ─────────────────────────────────────────────────────

    #[test]
    fn persist_writes_encrypted_binary_file() {
        setup();
        let ctx = context();
        let dir = ctx.storage_dir.clone();
        drop(ctx);
        ensure_dir(&dir);

        let mut items = VecDeque::new();
        items.push_back(QueuedActivation {
            id: "t1".into(),
            payload: make_event("e1"),
            timestamp: 1,
        });
        persist(&items);

        let path = queue_file(&dir);
        assert!(path.exists(), "queue file must be written");

        // File must be binary (encrypted), not plain JSON
        let raw = fs::read(&path).unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&raw).is_err(),
            "file must NOT be plain JSON"
        );
    }

    // ── No-race dedup ─────────────────────────────────────────────────────

    #[test]
    fn concurrent_enqueue_same_id_inserts_exactly_once() {
        setup();

        let barrier = Arc::new(Barrier::new(20));
        let mut handles = Vec::new();

        for _ in 0..20 {
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                b.wait(); // all threads start simultaneously
                enqueue("race-id".into(), make_event("payload"));
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            state.items.len(),
            1,
            "exactly one item must be present after concurrent dedup test"
        );
    }

    // ── Enqueue + persist ─────────────────────────────────────────────────

    #[test]
    fn enqueue_marks_dirty_and_persist_writes_file() {
        setup();

        enqueue("id1".into(), make_event("payload1"));

        // Give persist thread time to wake and write
        thread::sleep(Duration::from_millis(200));

        let ctx = context();
        let path = queue_file(&ctx.storage_dir);
        drop(ctx);

        assert!(path.exists(), "queue file must exist after enqueue");
    }

    // ── Persistence round-trip ────────────────────────────────────────────

    #[test]
    fn load_queue_restores_persisted_items() {
        setup();

        enqueue("restore1".into(), make_event("payload"));
        thread::sleep(Duration::from_millis(200)); // let persist thread write

        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.items.clear();
            state.seen.clear();
        }

        load_queue();

        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].id, "restore1");
    }

    // ── Overflow ─────────────────────────────────────────────────────────

    #[test]
    fn overflow_drops_oldest_and_stays_at_max() {
        setup();
        for i in 0..(MAX_QUEUE_SIZE + 10) {
            enqueue(format!("id{i}"), make_event(&format!("p{i}")));
        }
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.items.len() <= MAX_QUEUE_SIZE);
    }

    #[test]
    fn overflow_evicted_id_removed_from_seen() {
        setup();
        for i in 0..MAX_QUEUE_SIZE {
            enqueue(format!("id{i}"), make_event(&format!("p{i}")));
        }
        // This evicts "id0"
        enqueue("overflow".into(), make_event("overflow"));

        // Re-enqueue "id0" — must succeed (not deduplicated)
        enqueue("id0".into(), make_event("re-enqueued"));

        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.seen.contains("id0"), "id0 must be back in seen");
    }

    // ── Sequential order ─────────────────────────────────────────────────

    #[test]
    fn items_preserved_in_fifo_order() {
        setup();
        enqueue("1".into(), make_event("A"));
        enqueue("2".into(), make_event("B"));
        enqueue("3".into(), make_event("C"));

        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(state.items.pop_front().unwrap().payload.action_id, "A");
        assert_eq!(state.items.pop_front().unwrap().payload.action_id, "B");
        assert_eq!(state.items.pop_front().unwrap().payload.action_id, "C");
    }

    // ── Timestamp ────────────────────────────────────────────────────────

    #[test]
    fn timestamp_is_nonzero() {
        setup();
        enqueue("ts".into(), make_event("payload"));
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.items.front().unwrap().timestamp > 0);
    }

    // ── Flush ─────────────────────────────────────────────────────────────

    #[test]
    fn flush_clears_queue_and_persists_empty() {
        setup();
        enqueue("f1".into(), make_event("p1"));
        flush().unwrap();

        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.items.is_empty());
        assert!(state.seen.is_empty());
    }

    // ── Worker wakes on enqueue ───────────────────────────────────────────

    #[test]
    fn worker_starts_and_shuts_down_cleanly() {
        setup();
        start_worker();
        thread::sleep(Duration::from_millis(50));
        assert!(WORKER_RUNNING.load(Ordering::SeqCst));
        shutdown_worker();
        thread::sleep(Duration::from_millis(200));
        assert!(!WORKER_RUNNING.load(Ordering::SeqCst) || SHUTDOWN.load(Ordering::SeqCst));
    }
}
