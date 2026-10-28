//! With Agents on, a tunnel the person started starts again with the app, so an
//! update or a restart never cuts every agent off (`docs/plans/agents-from-anywhere.md`
//! §5; brief 80 §5.1); one they stopped stays stopped; and a copy of the book
//! started for some work (`BAGHOLDER_EXIT_WITH`) starts none (brief 80's
//! re-review, 1 and 2). The built server runs on one home, more than once; the
//! tunnel is a stand-in `ngrok` found first on PATH. HOME is a folder of the
//! test's own, so a real ngrok found where it is installed has no credentials and
//! opens nothing.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The servers started and what each ends with, and the stand-in tunnels: every
/// one ended however the test ends, a failed assertion too.
#[derive(Default)]
struct Started {
    servers: Vec<(Child, Option<Child>)>,
    tunnels: Vec<i32>,
}

impl Drop for Started {
    fn drop(&mut self) {
        for (server, owner) in &mut self.servers {
            let _ = server.kill();
            let _ = server.wait();
            if let Some(o) = owner {
                let _ = o.kill();
                let _ = o.wait();
            }
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

/// The folders a test runs in: the app's home, the user's, and the stand-in's.
struct Homes {
    home: tempfile::TempDir,
    user: tempfile::TempDir,
    bin: tempfile::TempDir,
}

impl Homes {
    fn new() -> Homes {
        let h = Homes { home: tempfile::tempdir().unwrap(), user: tempfile::tempdir().unwrap(), bin: tempfile::tempdir().unwrap() };
        // the stand-in tunnel says its own process id, then waits
        let ngrok = h.bin.path().join("ngrok");
        std::fs::write(&ngrok, "#!/bin/sh\necho \"stand-in tunnel $$\"\nexec sleep 600\n").unwrap();
        std::fs::set_permissions(&ngrok, std::fs::Permissions::from_mode(0o755)).unwrap();
        h
    }
}

/// The server on the homes: as the person runs it, or, `copy`, as a copy started
/// for some work, ending with a child of this test. Its port, once it listens.
fn serve(h: &Homes, copy: bool, started: &mut Started) -> u16 {
    let root = h.user.path().join("root");
    std::fs::create_dir_all(root.join("web/dist")).unwrap();
    std::fs::write(root.join("web/dist/index.html"), "<!doctype html><title>Bagholder</title>").unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let path = format!("{}:{}", h.bin.path().display(), std::env::var("PATH").unwrap_or_default());
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bagholder"));
    cmd.env_clear()
        .env("PATH", path)
        .env("HOME", h.user.path())
        .env("BAGHOLDER_HOME", h.home.path())
        .env("BAGHOLDER_ROOT", &root)
        .env("BAGHOLDER_PORT", port.to_string())
        .env("BAGHOLDER_CHILD", "1")
        .env("BAGHOLDER_NO_BROWSER", "1")
        .env("BAGHOLDER_DRY_ORDERS", "1")
        .env("BAGHOLDER_OFFLINE", "1");
    let owner = copy.then(|| Command::new("sleep").arg("600").spawn().unwrap());
    if let Some(o) = &owner {
        cmd.env("BAGHOLDER_EXIT_WITH", o.id().to_string());
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    assert!(out.by_ref().map_while(Result::ok).any(|l| l.contains("http://127.0.0.1:")), "the server listened");
    std::thread::spawn(move || out.for_each(drop));
    started.servers.push((child, owner));
    port
}

/// Ends the newest server through its own stop: the stop a person sends, or its
/// owner ending for a copy.
fn stop(started: &mut Started) {
    let (mut server, owner) = started.servers.pop().unwrap();
    match owner {
        Some(mut o) => {
            o.kill().unwrap();
            o.wait().unwrap();
        }
        // SAFETY: the stop signal to the server this test started
        None => assert_eq!(unsafe { libc::kill(server.id() as i32, libc::SIGTERM) }, 0),
    }
    assert!(server.wait().unwrap().success(), "it stopped through its own stop");
}

/// The stand-in tunnel's process id, once the server says it runs; none within
/// `within`.
fn tunnel_of(port: u16, within: Duration) -> Option<i32> {
    let until = Instant::now() + within;
    loop {
        let (_, status) = ask(port, "GET", "/api/status", None);
        let v: serde_json::Value = serde_json::from_str(&status).unwrap_or_default();
        let lines: Vec<String> = v["agents"]["tunnel"]["lines"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        if let Some(pid) = lines.iter().find_map(|l| l.strip_prefix("stand-in tunnel ").and_then(|p| p.trim().parse::<i32>().ok())) {
            assert_eq!(v["agents"]["tunnel"]["running"], "ngrok", "{v}");
            return Some(pid);
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Agents on, a tunnel chosen, and started from the panel: its process id.
fn started_by_the_person(port: u16, started: &mut Started) -> i32 {
    assert_eq!(ask(port, "POST", "/api/agents/switch", Some(r#"{"on":true}"#)).0, 200);
    let (code, said) = ask(port, "POST", "/api/agents/tunnel", Some(r#"{"provider":"ngrok","name":"agents.example.com"}"#));
    assert_eq!(code, 200, "{said}");
    let (code, said) = ask(port, "POST", "/api/agents/tunnel/start", None);
    assert_eq!((code, said.contains("\"ok\":true")), (200, true), "{said}");
    let pid = tunnel_of(port, Duration::from_secs(30)).expect("the tunnel started");
    started.tunnels.push(pid);
    pid
}

/// Waits for a tunnel to end with the app that ran it.
fn ended(pid: i32) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    !running(pid)
}

#[test]
fn a_tunnel_running_when_the_app_stopped_starts_again_with_it() {
    let h = Homes::new();
    let mut started = Started::default();
    let port = serve(&h, false, &mut started);
    let first = started_by_the_person(port, &mut started);
    stop(&mut started);
    assert!(ended(first), "the tunnel outlived the app");

    // started again: the tunnel with it, no one asking
    let port = serve(&h, false, &mut started);
    let again = tunnel_of(port, Duration::from_secs(30)).expect("no tunnel started again");
    started.tunnels.push(again);
    // and it ends with the app
    stop(&mut started);
    assert!(ended(again), "the tunnel outlived the app");
}

#[test]
fn a_tunnel_the_person_stopped_stays_stopped_across_a_restart() {
    let h = Homes::new();
    let mut started = Started::default();
    let port = serve(&h, false, &mut started);
    let first = started_by_the_person(port, &mut started);
    assert_eq!(ask(port, "POST", "/api/agents/tunnel/stop", None).0, 200);
    assert!(ended(first), "the tunnel stopped");
    stop(&mut started);

    let port = serve(&h, false, &mut started);
    assert_eq!(tunnel_of(port, Duration::from_secs(3)), None, "a tunnel the person stopped started again");
    stop(&mut started);
}

/// Clearing the agents with a tunnel running stops it: nothing is left to answer
/// an agent (brief 80's re-review, 8).
#[test]
fn clearing_the_agents_stops_their_running_tunnel() {
    let h = Homes::new();
    let mut started = Started::default();
    let port = serve(&h, false, &mut started);
    let tunnel = started_by_the_person(port, &mut started);
    let (code, said) = ask(port, "POST", "/api/data/clear", Some(r#"{"kinds":["agents"]}"#));
    assert_eq!(code, 200, "{said}");
    assert!(ended(tunnel), "the tunnel outlived Clear");
    stop(&mut started);
}

#[test]
fn a_copy_of_the_book_never_starts_the_person_s_tunnel() {
    let h = Homes::new();
    let mut started = Started::default();
    let port = serve(&h, false, &mut started);
    let first = started_by_the_person(port, &mut started);
    stop(&mut started);
    assert!(ended(first), "the tunnel outlived the app");

    // the same book, as a copy started for some work
    let port = serve(&h, true, &mut started);
    assert_eq!(tunnel_of(port, Duration::from_secs(3)), None, "a copy started the person's tunnel");
    stop(&mut started);
}
