//! Shared harness for the ported server tests: one app() on one temp home,
//! and one lock every test touching global state takes.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::app::App;

static LOCK: Mutex<()> = Mutex::new(());
static APP: OnceLock<Arc<App>> = OnceLock::new();

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

/// Serialize and make sure the shared test app exists.
pub fn guard() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    APP.get_or_init(|| App::new(home(), root, "127.0.0.1".into()));
    g
}

/// The one shared test app, set up by `guard()`.
pub fn app() -> Arc<App> {
    APP.get().expect("guard() sets up the app").clone()
}

/// The same app as a genuine `'static` reference: for a local `db()`/`conn()`
/// test helper that returns a pooled connection borrowed from it (the pool
/// ties a `Pooled<'_>` to the `&App` that opened it, and only a `'static`
/// reference lets that claim `Pooled<'static>` as the old global did).
pub fn app_ref() -> &'static App {
    APP.get().expect("guard() sets up the app")
}
