//! `BAGHOLDER_EXIT_WITH=<pid>` (brief 64, 5): a copy of the app started for some
//! work stops, through its own stop, when the process it names exits; and a
//! process already gone refuses the start. The built server is started on a home
//! of its own, with a child of this test as what it ends with.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// What the server ends with: a child that waits until it is killed.
fn starter() -> Child {
    #[cfg(unix)]
    let c = Command::new("sleep").arg("600").spawn();
    #[cfg(windows)]
    let c = Command::new("powershell").args(["-NoProfile", "-Command", "Start-Sleep -Seconds 600"]).spawn();
    c.unwrap()
}

fn server(home: &Path, user_home: &Path, exit_with: u32) -> Command {
    let root = user_home.join("root");
    std::fs::create_dir_all(root.join("web/dist")).unwrap();
    std::fs::write(root.join("web/dist/index.html"), "<!doctype html><title>Bagholder</title>").unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut c = Command::new(env!("CARGO_BIN_EXE_bagholder"));
    c.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", user_home)
        .env("USERPROFILE", user_home)
        .env("SystemRoot", std::env::var_os("SystemRoot").unwrap_or_default())
        .env("BAGHOLDER_HOME", home)
        .env("BAGHOLDER_ROOT", &root)
        .env("BAGHOLDER_PORT", port.to_string())
        .env("BAGHOLDER_CHILD", "1")
        .env("BAGHOLDER_NO_BROWSER", "1")
        .env("BAGHOLDER_DRY_ORDERS", "1")
        .env("BAGHOLDER_OFFLINE", "1")
        .env("BAGHOLDER_EXIT_WITH", exit_with.to_string())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(user_home.join("stderr.log")).unwrap());
    c
}

/// The server stops through its own stop once the process it was started for
/// exits: woken by the exit itself, it ends with success, and the data folder's
/// lock is free again.
#[test]
fn a_copy_stops_when_the_process_it_was_started_for_exits() {
    let (home, user_home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut owner = starter();
    let mut child = server(home.path(), user_home.path(), owner.id()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    // listening: the line it prints once bound
    assert!(out.by_ref().map_while(Result::ok).any(|l| l.contains("http://127.0.0.1:")), "the server listened");
    std::thread::spawn(move || out.for_each(drop));
    owner.kill().unwrap();
    owner.wait().unwrap();
    // nothing is killed here: the server is only waited for, and it must end by itself
    let status = child.wait().unwrap();
    assert!(status.success(), "it stopped through its own stop: {status}");
    let lock = std::fs::OpenOptions::new().read(true).write(true).open(home.path().join("bagholder.lock")).unwrap();
    assert!(lock.try_lock().is_ok(), "the data folder is free again");
}

/// A process already gone refuses the start, saying so: a copy is never started
/// for work that has ended.
#[test]
fn a_process_already_gone_refuses_the_start() {
    let (home, user_home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut gone = starter();
    let pid = gone.id();
    gone.kill().unwrap();
    gone.wait().unwrap();
    let status = server(home.path(), user_home.path(), pid).status().unwrap();
    assert!(!status.success(), "refused");
    let said = std::fs::read_to_string(home.path().join("bagholder.log")).unwrap_or_default() + &std::fs::read_to_string(user_home.path().join("stderr.log")).unwrap_or_default();
    let logs: String = std::fs::read_dir(home.path()).map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "log")).map(|e| std::fs::read_to_string(e.path()).unwrap_or_default()).collect()).unwrap_or_default();
    assert!(format!("{said}{logs}").contains("is not running"), "it says why: {said}{logs}");
}
