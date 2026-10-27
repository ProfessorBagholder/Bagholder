//! `bagholder tunnel-guard` (`docs/plans/agents-from-anywhere.md`, §4.4): the
//! tunnel Bagholder starts ends when the server it was started for ends, however
//! that server ends, so no tunnel process is left serving. The built binary guards
//! a stand-in tunnel for a stand-in server, both children of this test.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Whether `pid` still runs: a process ended but not yet reaped (Linux's `Z`) has ended.
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

#[test]
fn stopping_the_server_stops_the_tunnel() {
    let mut server = Command::new("sleep").arg("600").spawn().unwrap();
    // the stand-in tunnel says its own process id, then waits
    let mut guard = Command::new(env!("CARGO_BIN_EXE_bagholder"))
        .args(["tunnel-guard", "--", "sh", "-c", "echo $$; exec sleep 600"])
        .env("BAGHOLDER_EXIT_WITH", server.id().to_string())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(guard.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let tunnel: i32 = line.trim().parse().unwrap();
    assert!(running(tunnel), "the tunnel runs while the server does");
    // the server ends without a word to the guard
    server.kill().unwrap();
    server.wait().unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while running(tunnel) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(tunnel), "the tunnel outlived the server it was started for");
    guard.wait().unwrap();
}
