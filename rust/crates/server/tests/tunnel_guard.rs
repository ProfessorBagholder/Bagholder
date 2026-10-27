//! `bagholder tunnel-guard` (`docs/plans/agents-from-anywhere.md`, §4.4): the
//! tunnel Bagholder starts ends when the server it was started for ends, however
//! that server ends, and when the guard itself is told to stop, so no tunnel
//! process is left serving. The built binary guards a stand-in tunnel for a
//! stand-in server, both children of this test. The stand-in tunnel is a shell
//! with a child of its own: Linux ends the guard's direct child with the guard
//! whatever the guard does, never a process that child started, so only the
//! guard ending the whole group ends it.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
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

/// What a test started, ended however the test ends, a failed assertion too: a
/// guard that does not end the tunnel must not leave it running after the test.
struct Started {
    server: Child,
    tunnel: Option<i32>,
}

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(t) = self.tunnel.filter(|t| running(*t)) {
            // SAFETY: a signal to the stand-in tunnel this test's guard started
            unsafe { libc::kill(t, libc::SIGKILL) };
        }
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

fn server() -> Started {
    Started { server: Command::new("sleep").arg("600").spawn().unwrap(), tunnel: None }
}

/// The guard of a stand-in tunnel for `server`, and the process id of the child
/// the stand-in started, which it says, then waits on.
fn guarded(started: &mut Started) -> (Child, i32) {
    let server = &started.server;
    let mut guard = Command::new(env!("CARGO_BIN_EXE_bagholder"))
        .args(["tunnel-guard", "--", "sh", "-c", "sleep 600 & echo $!; wait"])
        .env("BAGHOLDER_EXIT_WITH", server.id().to_string())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(guard.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let tunnel: i32 = line.trim().parse().unwrap();
    started.tunnel = Some(tunnel);
    assert!(running(tunnel), "the tunnel runs while the server does");
    (guard, tunnel)
}

fn ends(tunnel: i32, what: &str) {
    let until = Instant::now() + Duration::from_secs(10);
    while running(tunnel) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(tunnel), "the tunnel outlived {what}");
}

#[test]
fn stopping_the_server_stops_the_tunnel() {
    let mut started = server();
    let (mut guard, tunnel) = guarded(&mut started);
    // the server ends without a word to the guard
    started.server.kill().unwrap();
    started.server.wait().unwrap();
    ends(tunnel, "the server it was started for");
    guard.wait().unwrap();
}

#[test]
fn a_stop_sent_to_the_guard_stops_the_tunnel() {
    for (signal, name) in [(libc::SIGTERM, "TERM"), (libc::SIGINT, "INT"), (libc::SIGHUP, "HUP")] {
        let mut started = server();
        let (mut guard, tunnel) = guarded(&mut started);
        // SAFETY: a signal to the guard this test started
        assert_eq!(unsafe { libc::kill(guard.id() as i32, signal) }, 0);
        ends(tunnel, &format!("a {name} sent to its guard"));
        guard.wait().unwrap();
    }
}
