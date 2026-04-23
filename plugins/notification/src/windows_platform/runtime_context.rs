// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct ActivationContext {
    // pub app_name: String,
    pub guid: String,
    pub storage_dir: PathBuf,
}

// OnceLock<Mutex<Option<…>>> lets init_context() overwrite safely between
// test runs (each test calls setup()) without panicking on the second call.
static CONTEXT: OnceLock<Mutex<Option<ActivationContext>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<ActivationContext>> {
    CONTEXT.get_or_init(|| Mutex::new(None))
}

/// Initialize (or re-initialize) the activation context.
///
/// Safe to call multiple times — subsequent calls overwrite the value.
/// Required for test isolation where every test calls setup().
pub fn init_context(guid: String, storage_dir: PathBuf) {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) =
        Some(ActivationContext { guid, storage_dir });
}

/// Return a cheap clone of the context.
///
/// RACE FIX: the old design returned a MutexGuard (ContextGuard) that held
/// the CONTEXT lock for the entire duration of the caller's use. Any caller
/// that then did I/O (create_dir_all, fs::write) held the lock across a
/// blocking syscall, serialising every other thread that called context().
///
/// Fix: lock once, clone the small struct (three Strings + a PathBuf),
/// release the lock, and return the owned clone. I/O happens lock-free.
pub fn context() -> ActivationContext {
    slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("ActivationContext not initialized — call init_context() first")
}
