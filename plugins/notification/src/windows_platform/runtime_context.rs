use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

#[derive(Debug)]
pub struct ActivationContext {
    pub app_name: String,
    pub guid: String,
    pub storage_dir: PathBuf,
}

// was OnceLock<ActivationContext> which panics on the second
// call to init_context() within the same process (all test cases share statics).
// Changed to OnceLock<Mutex<Option<...>>> so that init_context() can overwrite
// the value freely — including between test runs — without panicking.
static CONTEXT: OnceLock<Mutex<Option<ActivationContext>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<ActivationContext>> {
    CONTEXT.get_or_init(|| Mutex::new(None))
}

/// Initialize (or reinitialize) the activation context.
///
/// Safe to call multiple times — subsequent calls overwrite the previous value.
/// This is required for test isolation where each test calls `setup()`.
pub fn init_context(app_name: String, guid: String, storage_dir: PathBuf) {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(ActivationContext {
        app_name,
        guid,
        storage_dir,
    });
}

/// Borrow the activation context for the duration of the guard's lifetime.
///
/// Panics if `init_context` has not been called yet.
pub fn context() -> impl std::ops::Deref<Target = ActivationContext> + 'static {
    ContextGuard(slot().lock().unwrap_or_else(|e| e.into_inner()))
}

// ── Internal guard wrapper ────────────────────────────────────────────────

struct ContextGuard(MutexGuard<'static, Option<ActivationContext>>);

impl std::ops::Deref for ContextGuard {
    type Target = ActivationContext;
    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("ActivationContext not initialized — call init_context() first")
    }
}
