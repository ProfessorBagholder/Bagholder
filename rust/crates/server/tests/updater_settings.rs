//! A container copy's updater settings (`docs/plans/docker-update-button.md`):
//! `BAGHOLDER_UPDATER_URL` without `BAGHOLDER_CONTAINER_NAME` refuses the start,
//! naming the setting, as a bad `BAGHOLDER_HOSTS` does; with all three, the copy
//! starts, makes the token file the updater reads, and offers the button. The built
//! server is started on a home of its own.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

fn server(home: &Path, user_home: &Path, port: u16, extra: &[(&str, String)]) -> Command {
    let root = user_home.join("root");
    std::fs::create_dir_all(root.join("web/dist")).unwrap();
    std::fs::write(root.join("web/dist/index.html"), "<!doctype html><title>Bagholder</title>").unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_bagholder"));
    c.env_clear()
        .envs(extra.iter().map(|(k, v)| (*k, v.as_str())))
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
        // as the image sets it: the app never replaces its own files
        .env("BAGHOLDER_NO_UPDATE", "1")
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(user_home.join("stderr.log")).unwrap());
    c
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn an_updater_without_the_container_name_refuses_the_start_naming_it() {
    let (home, user_home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let token = home.path().join("updater").join("token");
    let extra = [("BAGHOLDER_UPDATER_URL", "http://bagholder-updater:8080".to_string()), ("BAGHOLDER_UPDATER_TOKEN_FILE", token.display().to_string())];
    let status = server(home.path(), user_home.path(), free_port(), &extra).status().unwrap();
    assert!(!status.success(), "refused");
    let said = std::fs::read_to_string(user_home.path().join("stderr.log")).unwrap_or_default();
    let logs: String = std::fs::read_dir(home.path()).map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "log")).map(|e| std::fs::read_to_string(e.path()).unwrap_or_default()).collect()).unwrap_or_default();
    assert!(format!("{said}{logs}").contains("BAGHOLDER_CONTAINER_NAME is needed"), "it names the setting: {said}{logs}");
    assert!(!token.exists(), "nothing is made for a start that is refused");
}

#[test]
fn with_all_three_the_copy_makes_the_token_and_offers_the_button() {
    let (home, user_home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let token = home.path().join("updater").join("token");
    let port = free_port();
    let extra = [
        ("BAGHOLDER_UPDATER_URL", "http://bagholder-updater:8080".to_string()),
        ("BAGHOLDER_UPDATER_TOKEN_FILE", token.display().to_string()),
        ("BAGHOLDER_CONTAINER_NAME", "bagholder".to_string()),
    ];
    let mut child = server(home.path(), user_home.path(), port, &extra).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    assert!(out.by_ref().map_while(Result::ok).any(|l| l.contains(&format!("http://127.0.0.1:{port}"))), "the server listened");
    std::thread::spawn(move || out.for_each(drop));
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET /api/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Bagholder: 1\r\nConnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    #[expect(clippy::let_underscore_must_use, reason = "the server may have exited already")]
    let _ = child.kill();
    #[expect(clippy::let_underscore_must_use, reason = "reaping the child; its status answers nothing")]
    let _ = child.wait();
    assert!(answer.contains("\"updateBy\":\"container\""), "{answer}");
    assert!(std::fs::read_to_string(&token).unwrap().trim().len() >= 32, "a token of its own");
    #[cfg(unix)]
    assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&token).unwrap().permissions()) & 0o777, 0o600, "readable by its user alone");
}
