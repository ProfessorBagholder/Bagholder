//! The first word the server is started with: no argument starts the app, a
//! command it carries runs, and any other word is refused the one same way, with
//! exit 2, never taken for a start of the app. A developer's command a build does
//! not carry (`demo`, `pull-broker`: features no release enables) is refused
//! exactly as a word nobody knows, so a release build names neither.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The server run with `args` on a folder of its own, offline: its exit code and
/// what it said on standard error. A run that has not ended within a minute (a
/// server started) is stopped and fails the test.
fn run(args: &[&str]) -> (Option<i32>, String) {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bagholder"))
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("SystemRoot", std::env::var_os("SystemRoot").unwrap_or_default())
        .env("BAGHOLDER_HOME", home.path().join("data"))
        .env("BAGHOLDER_PORT", "0")
        .env("BAGHOLDER_CHILD", "1")
        .env("BAGHOLDER_NO_BROWSER", "1")
        .env("BAGHOLDER_DRY_ORDERS", "1")
        .env("BAGHOLDER_OFFLINE", "1")
        .env("BAGHOLDER_EXIT_WITH", std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("bagholder {args:?} did not end: it was taken for a start of the app");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut said = String::new();
    child.stderr.take().unwrap().read_to_string(&mut said).unwrap();
    assert!(!home.path().join("data").exists(), "bagholder {args:?} wrote a data folder");
    (status.code(), said)
}

#[test]
fn every_word_that_is_no_command_is_refused_the_same_way_and_starts_nothing() {
    let (code, said) = run(&["frobnicate"]);
    assert_eq!(code, Some(2), "{said}");
    assert!(said.contains("not a command this build runs"), "{said}");
    assert!(!said.contains("frobnicate"), "the refusal is one message, whatever the word: {said}");
    // a flag nobody knows, and a word with more after it, the same
    assert_eq!(run(&["--frobnicate"]), (code, said.clone()));
    assert_eq!(run(&["frobnicate", "x"]), (code, said.clone()));
    // a developer's command this build does not carry is no different from a typo
    #[cfg(not(feature = "demo"))]
    assert_eq!(run(&["demo", "x"]), (code, said.clone()));
    #[cfg(not(feature = "pull-broker"))]
    assert_eq!(run(&["pull-broker", "x"]), (code, said.clone()));
}

#[test]
fn the_version_is_still_said_whatever_comes_with_it() {
    let out = Command::new(env!("CARGO_BIN_EXE_bagholder")).arg("--version").env("BAGHOLDER_EXIT_WITH", std::process::id().to_string()).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("bagholder "));
}
