//! No step at start that reads the stored data stops the server (#516): a service
//! manager would only start it again into the same failure, with no page and no
//! update button. For a failure planted in each such step, the built server is
//! started on a home of its own: it listens, `/` serves the page, and the header
//! names the failure. And a start opens no store but the book and the market
//! cache (brief 75, test 9).

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
    started_by(Command::new(env!("CARGO_BIN_EXE_bagholder")), home, user_home, &[])
}

/// The server started by `command` (the server itself, or a tracer running it),
/// with `extra` variables beside the test's own.
fn started_by(mut command: Command, home: &Path, user_home: &Path, extra: &[(&str, String)]) -> Server {
    let root = user_home.join("root");
    std::fs::create_dir_all(root.join("web/dist")).unwrap();
    std::fs::write(root.join("web/dist/index.html"), PAGE).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = command
        .env_clear()
        .envs(extra.iter().map(|(k, v)| (*k, v.as_str())))
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

/// The header as the page has it: the `status` document's first message on a new
/// stream, as a page opening (or reloaded) receives it.
fn streamed(server: &Server) -> serde_json::Value {
    let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
    write!(s, "GET /api/events?zone=UTC&docs=%7B%22status%22%3A%7B%7D%7D HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Bagholder: 1\r\nAccept: text/event-stream\r\n\r\n", server.port).unwrap();
    let mut seen = String::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = s.read(&mut buf).expect("the stream sends the header");
        assert!(n > 0, "the stream ended before it sent the header: {seen}");
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        // each message is a frame of `event:` and `data:` lines
        for frame in seen.split("\n\n") {
            if !frame.lines().any(|l| l.trim() == "event: snapshot") {
                continue;
            }
            let Some(data) = frame.lines().find_map(|l| l.strip_prefix("data: ")) else { continue };
            let Ok(m) = serde_json::from_str::<serde_json::Value>(data) else { continue };
            if m["doc"] == "status" {
                return m["data"].clone();
            }
        }
    }
}

/// The header as `/api/status` answers it.
fn header(server: &Server) -> serde_json::Value {
    let (status, body) = get(server.port, "/api/status");
    assert!(status.contains(" 200 "), "the header is answered whatever failed: {status} {body}");
    serde_json::from_str(&body).unwrap()
}

/// The page is served, the header is answered whole (its version and its update
/// fields, so the update button works), and its error line is exactly `said`:
/// one sentence, naming what failed.
fn up_saying(server: &Server, said: &str) {
    let (status, page) = get(server.port, "/");
    assert!(status.contains(" 200 "), "the page: {status}");
    assert!(page.contains("the page"), "the page is the one the server was given: {page}");
    // read twice: a reader's failure is said once it fails again, so a reader
    // adding a sentence of its own would show by the second read
    header(server);
    let h = header(server);
    assert_eq!(h["ok"], true, "{h}");
    assert!(h["version"].as_str().is_some_and(|v| !v.is_empty()), "the version: {h}");
    for field in ["latestVersion", "updateAvailable", "canUpdate", "updateUrl"] {
        assert!(h.get(field).is_some(), "the update field {field}: {h}");
    }
    assert_eq!(h["error"], said, "the whole error line");
}

#[test]
fn a_book_this_build_cannot_open_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(home.path().join("book.db"), NOT_A_DATABASE).unwrap();
    up_saying(&started(home.path(), user.path()), "The book could not be opened: file is not a database.");
}

#[test]
fn a_market_cache_this_build_cannot_open_is_said_and_the_server_stays_up() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(home.path().join("market.db"), NOT_A_DATABASE).unwrap();
    let server = started(home.path(), user.path());
    up_saying(&server, "The market cache could not be opened: file is not a database.");
    // read again, it says the same: one failure, one wording, whichever reader met it
    assert_eq!(header(&server)["error"], "The market cache could not be opened: file is not a database.");
    // its versions, which only the cache holds, are absent, never made up
    let h = header(&server);
    assert_eq!((&h["dataVersion"], &h["coreVersion"]), (&serde_json::Value::Null, &serde_json::Value::Null));
}

/// The stores a start opens: every file the server and what it runs open, by
/// the system calls that open one, read from the trace `strace` keeps of the
/// built server. A store is a file SQLite keeps (a database, its write-ahead log,
/// its shared memory, its journal); each must be the book's or the market cache's.
/// Any other SQLite file beside them, in the data folder and in the user's
/// `.bagholder` folder, is never opened and is left byte for byte as it was.
#[cfg(target_os = "linux")]
#[test]
fn a_start_opens_only_the_book_and_the_market_cache() {
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let unrelated = ["unrelated.db", "store.db", "other.sqlite"];
    let mut bytes = Vec::new();
    for dir in [home.path().to_path_buf(), user.path().join(".bagholder")] {
        std::fs::create_dir_all(&dir).unwrap();
        for name in unrelated {
            rusqlite::Connection::open(dir.join(name)).unwrap().execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('k', 'v');").unwrap();
            bytes.push((dir.join(name), std::fs::read(dir.join(name)).unwrap()));
        }
    }
    let trace = user.path().join("opened.trace");
    // the server stops when this ends (`BAGHOLDER_EXIT_WITH`), and the tracer with it
    let mut work = Command::new("sleep").arg("600").spawn().expect("sleep runs");
    let mut tracer = Command::new("strace");
    tracer.args(["-f", "-qq", "-e", "trace=open,openat,openat2,creat", "-o"]).arg(&trace).arg(env!("CARGO_BIN_EXE_bagholder"));
    let mut server = started_by(tracer, home.path(), user.path(), &[("BAGHOLDER_EXIT_WITH", work.id().to_string())]);
    // a page opening: the header on its stream, and a view of the book
    streamed(&server);
    get(server.port, "/api/view?key=book&params=%7B%7D");
    work.kill().unwrap();
    work.wait().unwrap();
    let done = server.child.wait().unwrap();
    assert!(done.success(), "the server and its tracer stop with the work they were started for: {done}");
    let text = std::fs::read_to_string(&trace).unwrap();
    // each opened path, as the call names it: the first quoted argument
    let opened: std::collections::BTreeSet<String> = text.lines().filter(|l| l.contains("open") || l.contains("creat(")).filter_map(|l| l.split('"').nth(1)).map(str::to_string).collect();
    assert!(opened.iter().any(|p| p.ends_with("/book.db")), "the trace holds the start's opens: {opened:?}");
    let store = |p: &str| [".db", ".sqlite", "-wal", "-shm", "-journal"].iter().any(|e| p.ends_with(e));
    let book = home.path().join("book.db").to_string_lossy().into_owned();
    let cache = home.path().join("market.db").to_string_lossy().into_owned();
    let allowed: Vec<String> = [&book, &cache].iter().flat_map(|f| ["", "-wal", "-shm", "-journal"].map(|s| format!("{f}{s}"))).collect();
    let others: Vec<&String> = opened.iter().filter(|p| store(p) && !allowed.contains(p)).collect();
    assert!(others.is_empty(), "a start opened a store that is neither the book nor the market cache: {others:?}");
    for (path, was) in &bytes {
        assert_eq!(&std::fs::read(path).unwrap(), was, "{} is byte for byte as it was", path.display());
    }
    // the start made only the book, the cache, their journals and snapshots
    let made: Vec<String> = std::fs::read_dir(home.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| store(n) && !unrelated.contains(&n.as_str())).collect();
    assert!(made.iter().all(|n| n.starts_with("book.db") || n.starts_with("market.db")), "{made:?}");
    for dir in [home.path().to_path_buf(), user.path().join(".bagholder")] {
        for name in unrelated {
            assert!(!opened.iter().any(|p| p.ends_with(&format!("/{name}"))), "{name} in {} was opened", dir.display());
            assert!(!dir.join(format!("{name}-shm")).exists() && !dir.join(format!("{name}-journal")).exists(), "{name} was read by SQLite");
        }
    }
}

/// Whether the figure path is open: `/api/view` refuses with "the figures are not
/// open" until it is (a book with no zone stated yet answers otherwise).
fn figures_open(server: &Server) -> bool {
    !get(server.port, "/api/view?key=book&params=%7B%7D").1.contains("the figures are not open")
}

/// A store mended while the server runs opens without a restart: the header the
/// page next receives on its stream no longer says it, and the figures open
/// (briefs 50, 2 and 55). Read from the stream, never from `/api/status`, which
/// the page does not ask. For the market
/// cache (its file a folder, then put back) and for the book (a file no database
/// reads, then taken away).
#[test]
fn a_store_mended_while_the_server_runs_opens_at_the_next_header_read() {
    // the market cache
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    drop(started(home.path(), user.path()));
    std::fs::rename(home.path().join("market.db"), home.path().join("market.db.kept")).unwrap();
    std::fs::create_dir(home.path().join("market.db")).unwrap();
    let server = started(home.path(), user.path());
    let said = streamed(&server)["error"].as_str().unwrap().to_string();
    assert!(said.starts_with("The market cache could not be opened"), "{said}");
    assert!(!figures_open(&server), "nothing is built without its cache");
    std::fs::remove_dir(home.path().join("market.db")).unwrap();
    std::fs::rename(home.path().join("market.db.kept"), home.path().join("market.db")).unwrap();
    assert_eq!(streamed(&server)["error"], "", "the next header read opens it");
    assert!(figures_open(&server));
    drop(server);
    // the book
    let (home, user) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(home.path().join("book.db"), NOT_A_DATABASE).unwrap();
    let server = started(home.path(), user.path());
    assert_eq!(streamed(&server)["error"], "The book could not be opened: file is not a database.");
    assert!(!figures_open(&server));
    std::fs::remove_file(home.path().join("book.db")).unwrap();
    assert_eq!(streamed(&server)["error"], "", "the next header read opens it");
    assert!(figures_open(&server));
}
