//! With Agents on and a tunnel chosen, a copy of the app started again starts the
//! tunnel again, so an update or a restart never cuts every agent off
//! (`docs/plans/agents-from-anywhere.md` §5; brief 80 §5.1). The built server runs
//! twice on one home, each time ending with a child of this test; the tunnel is a
//! stand-in `ngrok` found first on PATH. HOME is a folder of the test's own, so a
//! real ngrok found where it is installed has no credentials and opens nothing.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What each server ends with, and the servers: every one ended however the test
/// ends, a failed assertion too, and each stand-in tunnel with them.
#[derive(Default)]
struct Started {
    children: Vec<Child>,
    tunnels: Vec<i32>,
}

impl Drop for Started {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
        for t in &self.tunnels {
            // SAFETY: a signal to a stand-in tunnel this test's server started
            unsafe { libc::kill(*t, libc::SIGKILL) };
        }
    }
}

fn running(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat.rsplit_once(") ").is_none_or(|(_, rest)| !rest.starts_with('Z')),
        Err(_) => true,
    }
}

/// One request to the page's own listener, as the page makes it.
fn ask(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let body = body.unwrap_or("");
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Bagholder: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut text = String::new();
    s.read_to_string(&mut text).unwrap();
    let status = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let (_, rest) = text.split_once("\r\n\r\n").unwrap_or(("", ""));
    (status, rest.to_string())
}

/// The server on `home`, ending with a new child of this test: its port, once it
/// listens.
fn serve(home: &Path, user_home: &Path, bin: &Path, started: &mut Started) -> u16 {
    let owner = Command::new("sleep").arg("600").spawn().unwrap();
    let root = user_home.join("root");
    std::fs::create_dir_all(root.join("web/dist")).unwrap();
    std::fs::write(root.join("web/dist/index.html"), "<!doctype html><title>Bagholder</title>").unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let mut child = Command::new(env!("CARGO_BIN_EXE_bagholder"))
        .env_clear()
        .env("PATH", path)
        .env("HOME", user_home)
        .env("BAGHOLDER_HOME", home)
        .env("BAGHOLDER_ROOT", &root)
        .env("BAGHOLDER_PORT", port.to_string())
        .env("BAGHOLDER_CHILD", "1")
        .env("BAGHOLDER_NO_BROWSER", "1")
        .env("BAGHOLDER_DRY_ORDERS", "1")
        .env("BAGHOLDER_OFFLINE", "1")
        .env("BAGHOLDER_EXIT_WITH", owner.id().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    assert!(out.by_ref().map_while(Result::ok).any(|l| l.contains("http://127.0.0.1:")), "the server listened");
    std::thread::spawn(move || out.for_each(drop));
    started.children.push(owner);
    started.children.push(child);
    port
}

/// Ends the newest server: its owner exits, and the server stops through its own stop.
fn stop(started: &mut Started) {
    let mut server = started.children.pop().unwrap();
    let mut owner = started.children.pop().unwrap();
    owner.kill().unwrap();
    owner.wait().unwrap();
    assert!(server.wait().unwrap().success(), "it stopped through its own stop");
}

#[test]
fn a_tunnel_chosen_with_agents_on_starts_again_with_the_app() {
    let (home, user_home, bin) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // the stand-in tunnel says its own process id, then waits
    let ngrok = bin.path().join("ngrok");
    std::fs::write(&ngrok, "#!/bin/sh\necho \"stand-in tunnel $$\"\nexec sleep 600\n").unwrap();
    std::fs::set_permissions(&ngrok, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut started = Started::default();

    // Agents on and a tunnel chosen, never started by hand
    let port = serve(home.path(), user_home.path(), bin.path(), &mut started);
    assert_eq!(ask(port, "POST", "/api/agents/switch", Some(r#"{"on":true}"#)).0, 200);
    let (code, said) = ask(port, "POST", "/api/agents/tunnel", Some(r#"{"provider":"ngrok","name":"agents.example.com"}"#));
    assert_eq!(code, 200, "{said}");
    stop(&mut started);

    // started again: the tunnel with it, no one asking
    let port = serve(home.path(), user_home.path(), bin.path(), &mut started);
    let until = Instant::now() + Duration::from_secs(30);
    let tunnel = loop {
        let (_, status) = ask(port, "GET", "/api/status", None);
        let v: serde_json::Value = serde_json::from_str(&status).unwrap_or_default();
        let lines: Vec<String> = v["agents"]["tunnel"]["lines"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        if let Some(pid) = lines.iter().find_map(|l| l.strip_prefix("stand-in tunnel ").and_then(|p| p.trim().parse::<i32>().ok())) {
            assert_eq!(v["agents"]["tunnel"]["running"], "ngrok", "{v}");
            break pid;
        }
        assert!(Instant::now() < until, "no tunnel started again: {v}");
        std::thread::sleep(Duration::from_millis(200));
    };
    started.tunnels.push(tunnel);
    assert!(running(tunnel));
    // and it ends with the app
    stop(&mut started);
    let until = Instant::now() + Duration::from_secs(10);
    while running(tunnel) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(tunnel), "the tunnel outlived the app");
}
