use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug)]
pub struct ActivationContext {
    pub app_name: String,
    pub guid: String,
    pub storage_dir: PathBuf,
}

static CONTEXT: OnceLock<ActivationContext> = OnceLock::new();

pub fn init_context(app_name: String, guid: String, storage_dir: PathBuf) {
    CONTEXT
        .set(ActivationContext {
            app_name,
            guid,
            storage_dir,
        })
        .expect("ActivationContext initialized twice");
}

pub fn context() -> &'static ActivationContext {
    CONTEXT.get().expect("ActivationContext not initialized")
}
