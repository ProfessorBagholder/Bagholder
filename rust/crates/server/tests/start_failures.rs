//! No step at start that reads the stored data stops the server (#516): a service
//! manager would only start it again into the same failure, with no page and no
//! update button. For a failure planted in each such step, the built server is
//! started on a home of its own: it listens, `/` serves the page, and the header
//! names the failure.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// The page the server is given to serve, so `/` is checked without a built page.
const PAGE: &str = "<!doctype html><title>Bagholder</title>the page";

/// Bytes no SQLite reader takes for a database.
const NOT_A_DATABASE: &[u8] = b"this file is not a database, whatever its name says";

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        // the test's own child, stopped with the test
        #[expect(clippy::let_underscore_must_use, reason = "the server may have exited already")]
        let _ = self.child.kill();
        #[expect(clippy::let_underscore_must_use, reason = "reaping the child; its status answers nothing")]
        let _ = self.child.wait();
    }
}

/// The server started on `home`, once it says it listens (the line it prints when
/// bound), with `HOME` its own so no other app's data on this machine is read.
fn started(home: &Path, user_home: &Path) -> Server {
    let root = user_home.join("root");
    std::fs::create_dir_all(root.join("web/dist")).unwrap();
    std::fs::write(root.join("web/dist/index.html"), PAGE).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bagholder"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", user_home)
        .env("USERPROFILE", user_home)
        .env("BAGHOLDER_HOME", home)
        .env("BAGHOLDER_ROOT", &root)
        .env("BAGHOLDER_PORT", port.to_string())
        .env("BAGHOLDER_CHILD", "1")
        .env("BAGHOLDER_NO_BROWSER", "1")
        .env("BAGHOLDER_DRY_ORDERS", "1")
        .env("BAGHOLDER_OFFLINE", "1")
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(user_home.join("stderr.log")).unwrap())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut server = Server { child, port };
    while let Some(line) = out.next() {
        if line.unwrap().contains(&format!("http://127.0.0.1:{port}")) {
            // the rest of what it prints is read until it exits: a pipe closed under
            // it would stop it at its next line
            std::thread::spawn(move || out.for_each(drop));
            return server;
        }
    }
    let status = server.child.wait().unwrap();
    panic!("the server stopped before it listened ({status})");
}

/// One GET's status line and body.
fn get(port: u16, path: &str) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Bagholder: 1\r\nConnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap_or((&answer, ""));
    (head.lines().next().unwrap_or("").to_string(), body.to_string())
}

/// The page is served, and the header's error line names `said`.
fn up_saying(server: &Server, said: &str) {
    let (status, page) = get(server.port, "/");
    assert!(status.contains(" 200 "), "the page: {status}");
    assert!(page.contains("the page"), "the page is the one the server was given: {page}");
    // the header's error line; a market cache that cannot be read is said by the
    // status answer refusing with it, as every request that meets it does
    let (status, body) = get(server.port, "/api/status");
    assert!(status.contains(" 200 ") || status.contains(" 500 "), "the status: {status}");
    assert!(body.contains(said), "the header names the failure ({said}): {body}");
}

#[test]
fn a_book_this_build_cannot_open_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(home.path().join("book.db"), NOT_A_DATABASE).unwrap();
    up_saying(&started(home.path(), user.path()), "book could not be opened");
}

#[test]
fn a_market_cache_this_build_cannot_open_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(home.path().join("market.db"), NOT_A_DATABASE).unwrap();
    up_saying(&started(home.path(), user.path()), "market cache could not be opened");
}

#[test]
fn an_earlier_database_that_cannot_be_carried_into_a_new_book_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(home.path().join("bagholder.db"), NOT_A_DATABASE).unwrap();
    up_saying(&started(home.path(), user.path()), "could not be carried into the book");
    // no book was made beside it, so the next start tries again
    assert!(!home.path().join("book.db").exists());
}

#[test]
fn an_earlier_store_that_cannot_be_carried_beside_a_book_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // a book of its own first, from an ordinary start
    drop(started(home.path(), user.path()));
    std::fs::write(home.path().join("bagholder.db"), NOT_A_DATABASE).unwrap();
    up_saying(&started(home.path(), user.path()), "earlier store could not be opened");
}

#[test]
fn the_python_apps_database_that_cannot_be_carried_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(user.path().join(".bagholder")).unwrap();
    std::fs::write(user.path().join(".bagholder").join("bagholder.db"), NOT_A_DATABASE).unwrap();
    up_saying(&started(home.path(), user.path()), "earlier database could not be carried into the book");
}
