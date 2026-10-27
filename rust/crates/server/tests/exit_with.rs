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

/// A copy that stops leaves no local model it started: the summaries' model server
/// stops with it. A stand-in model (a script answering `/health` on the port it is
/// given) is in the data folder, asked for as the filings feed asks for it; then
/// the copy is stopped through its own stop.
#[cfg(unix)]
#[test]
fn a_copy_that_stops_leaves_no_local_model_running() {
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    let (home, user_home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let models = home.path().join("models");
    std::fs::create_dir_all(models.join("www")).unwrap();
    std::fs::write(models.join("www").join("health"), "ok").unwrap();
    let model = models.join("summarizer.llamafile");
    std::fs::write(&model, "#!/bin/sh\nport=\"\"\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"--port\" ] && port=\"$2\"; shift; done\necho $$ > \"$(dirname \"$0\")/pid\"\nexec python3 -m http.server \"$port\" --bind 127.0.0.1 --directory \"$(dirname \"$0\")/www\"\n").unwrap();
    std::fs::set_permissions(&model, std::fs::Permissions::from_mode(0o755)).unwrap();
    let sha: String = openssl::sha::sha256(&std::fs::read(&model).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
    let model_port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut owner = starter();
    let mut child = server(home.path(), user_home.path(), owner.id()).env("BAGHOLDER_LLAMAFILE_SHA256", &sha).env("BAGHOLDER_LLM_PORT", model_port.to_string()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let listening = out.by_ref().map_while(Result::ok).find(|l| l.contains("http://127.0.0.1:")).expect("the server listened");
    std::thread::spawn(move || out.for_each(drop));
    let port = listening.rsplit(':').next().unwrap().trim().trim_end_matches('/').to_string();
    // the filings feed asks whether summaries can be read, which brings the model up
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    write!(s, "GET /api/filings/feed?scope=all HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    let pid_file = models.join("pid");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !pid_file.exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let pid = std::fs::read_to_string(&pid_file).expect("the model was started").trim().to_string();
    let running = |pid: &str| Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    assert!(running(&pid), "the model runs while the copy does");
    owner.kill().unwrap();
    owner.wait().unwrap();
    assert!(child.wait().unwrap().success(), "it stopped through its own stop");
    let still = running(&pid);
    if still {
        // the test's own leftover goes, whatever the assertion says
        #[expect(clippy::let_underscore_must_use, reason = "cleaning up after a failure the assertion below reports")]
        let _ = Command::new("kill").arg(&pid).status();
    }
    assert!(!still, "no local model is left running after the copy stopped");
}
