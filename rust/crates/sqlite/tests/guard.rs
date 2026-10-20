//! A test run never touches the person's own data folder. One test class once
//! reached ~/.bagholder without a temporary home and wrote a made-up headline
//! into the live data. `HOME` points at a temporary folder here, so the "real"
//! folder these tests are refused is never the person's.

#[test]
fn test_the_real_folder_is_refused_to_a_test_run_and_a_temporary_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());
    for name in [".bagholder", ".bagholder-rust"] {
        std::fs::create_dir_all(dir.path().join(name)).unwrap();
        assert!(bagholder_sqlite::guard_home(&dir.path().join(name)).is_err(), "{name}");
    }
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(bagholder_sqlite::guard_home(tmp.path()).unwrap(), tmp.path());
    assert!(bagholder_sqlite::running_tests());
}
