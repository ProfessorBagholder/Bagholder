//! Shared harness for the ported server tests: one app() on one temp home,
//! and one lock every test touching global state takes.
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

static LOCK: Mutex<()> = Mutex::new(());

pub fn home() -> PathBuf {
    static H: OnceLock<PathBuf> = OnceLock::new();
    H.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("bh-server-tests-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("BAGHOLDER_DRY_ORDERS", "1");
        d
    })
    .clone()
}

/// Serialize and make sure the global app exists.
pub fn guard() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    crate::app::init(home(), root, "127.0.0.1".into());
    g
}
