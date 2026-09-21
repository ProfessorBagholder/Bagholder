//! A test run never touches the person's own data folder. One test class once
//! reached ~/.bagholder without a temporary home and wrote a made-up headline
//! into the live database. `HOME` points at a temporary folder here, so the
//! "real" folder these tests are refused is never the person's.
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn fake_home() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());
    std::fs::create_dir_all(dir.path().join(".bagholder")).unwrap();
    (g, dir)
}

#[test]
fn test_the_real_folder_is_refused_to_a_test_run_and_a_temporary_one_is_not() {
    let (_g, home) = fake_home();
    assert!(bagholder_store::guard_home(&home.path().join(".bagholder")).is_err());
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(bagholder_store::guard_home(tmp.path()).unwrap(), tmp.path());
}

#[test]
fn test_a_test_that_forgets_its_home_fails_instead_of_opening_the_live_database() {
    let (_g, home) = fake_home();
    assert!(bagholder_store::connect(&home.path().join(".bagholder")).is_err());
    assert!(!home.path().join(".bagholder").join("bagholder.db").exists());
    let tmp = tempfile::tempdir().unwrap();
    assert!(bagholder_store::connect(tmp.path()).is_ok());
}
