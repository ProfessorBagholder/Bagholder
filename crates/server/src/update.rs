//! The in-app update: the hourly check against the latest GitHub release, the
//! install of that release's prebuilt archive for this platform (or a pull,
//! for a git checkout), and the supervisor that restarts the server into the
//! new version and puts the previous one back when it does not start.
//!
//! A release carries one archive per platform, `bagholder-vX.Y.Z-<target>.tar.gz`
//! (`.zip` on Windows) with its `.sha256` beside it, holding the `bagholder`
//! executable and the page's static files.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::app::{app, f, log, now_iso, now_unix, parse_instant, spawn, truthy, APP_VERSION, REPO};

pub const RESTART_CODE: i32 = 3;
/// A restarted server alive this long is a good update.
pub const UPDATE_HEALTHY_SEC: u64 = 20;
pub const UPDATE_MAX_BYTES: usize = 50 * 1024 * 1024;
pub const UPDATE_CHECK_HOURS: f64 = 1.0;
pub const UPDATES_OFF_MESSAGE: &str = "This copy is updated with docker compose pull; a new release is a new image.";

pub fn repo_url() -> String {
    format!("https://github.com/{}", REPO)
}

pub fn release_url() -> String {
    format!("https://api.github.com/repos/{}/releases/latest", REPO)
}

/// Where a container copy's header sends the user for a new release.
pub fn image_page() -> String {
    format!("{}/pkgs/container/bagholder", repo_url())
}

/// `BAGHOLDER_NO_UPDATE`: a copy updated some other way (the container).
pub fn updates_off() -> bool {
    !std::env::var("BAGHOLDER_NO_UPDATE").unwrap_or_default().trim().is_empty()
}

/// The platform this binary was built for, as the release names its archive.
pub fn target_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "macos" => format!("{}-apple-darwin", arch),
        "windows" => format!("{}-pc-windows-msvc", arch),
        "linux" => format!("{}-unknown-linux-gnu", arch),
        other => format!("{}-{}", arch, other),
    }
}

fn archive_ext() -> &'static str {
    if cfg!(windows) { "zip" } else { "tar.gz" }
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "bagholder.exe" } else { "bagholder" }
}

/// The folder the running copy lives in: the executable's own.
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| app().root.clone())
}

/// 'v1.2.3' -> (1, 2, 3).
pub fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let t = tag.trim();
    let t = t.strip_prefix('v').unwrap_or(t);
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

/// Tests stand in for GitHub here: `(calls, answer)`, `None` answering as offline.
#[cfg(test)]
pub static FAKE_RELEASE: std::sync::Mutex<Option<(usize, Option<Value>)>> = std::sync::Mutex::new(None);

/// The latest release as GitHub describes it, `None` when it cannot be read.
fn fetch_release() -> Option<Value> {
    #[cfg(test)]
    {
        if let Some((calls, answer)) = FAKE_RELEASE.lock().unwrap().as_mut() {
            *calls += 1;
            return answer.clone();
        }
        return None;
    }
    #[allow(unreachable_code)]
    {
        let ua = format!("Bagholder/{}", APP_VERSION);
        let got = bagholder_market::client::request(
            "GET",
            &release_url(),
            &[("Accept", "application/vnd.github+json"), ("User-Agent", &ua)],
            None,
            Duration::from_secs(30),
        );
        got.ok().and_then(|r| serde_json::from_slice(&r.body).ok())
    }
}

/// The latest release against APP_VERSION, the
/// record stored in meta. Never fails.
pub fn check_for_update() -> Value {
    let mut record = json!({"checkedAt": now_iso(), "ok": false, "latest": "", "url": format!("{}/releases/latest", repo_url()), "updateAvailable": false});
    let rel = fetch_release();
    if let Some(rel) = rel {
        if let Some(latest) = parse_version(&f(&rel, "tag_name")) {
            let tag = f(&rel, "tag_name");
            let html = f(&rel, "html_url");
            record["ok"] = json!(true);
            record["latest"] = json!(tag);
            if !html.is_empty() {
                record["url"] = json!(html);
            }
            let available = Some(latest) > parse_version(APP_VERSION);
            record["updateAvailable"] = json!(available);
            record["assets"] = release_assets(&rel);
            if available {
                if let Ok(c) = app().open() {
                    crate::notify::emit(
                        &c,
                        "updates",
                        &format!("update:{}", tag),
                        &format!("Bagholder {} is available", tag),
                        if updates_off() { "Pull the new image." } else { "Update from the header." },
                        None,
                    );
                }
            }
        }
    }
    if let Ok(c) = app().open() {
        let _ = bagholder_store::tables::set_meta(&c, "update_check", &bagholder_store::tables::json_text(&record));
    }
    record
}

/// The last check's record, {} when none.
pub fn update_status() -> Value {
    let raw = app().open().ok().and_then(|c| bagholder_store::tables::get_meta(&c, "update_check", "").ok()).unwrap_or_default();
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) if v.is_object() => v,
        _ => json!({}),
    }
}

/// {archive, sha} download URLs of this platform's
/// archive and its .sha256, when the release carries both.
pub fn release_assets(rel: &Value) -> Value {
    let tag = f(rel, "tag_name");
    let stem = format!("bagholder-{}-{}.{}", tag, target_triple(), archive_ext());
    let mut archive = String::new();
    let mut sha = String::new();
    for a in rel.get("assets").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let name = f(&a, "name");
        if name == stem {
            archive = f(&a, "browser_download_url");
        } else if name == format!("{}.sha256", stem) {
            sha = f(&a, "browser_download_url");
        }
    }
    if !archive.is_empty() && !sha.is_empty() {
        json!({"archive": archive, "sha": sha})
    } else {
        json!({})
    }
}

// --------------------------------------------------------------------------
// in-app update
// --------------------------------------------------------------------------

fn which(cmd: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| {
        let p = d.join(cmd);
        p.is_file() || (cfg!(windows) && d.join(format!("{}.exe", cmd)).is_file())
    })
}

/// 'git' when this copy is a git checkout with git on
/// the path, else 'release'.
pub fn update_mode() -> &'static str {
    if app().root.join(".git").exists() && which("git") { "git" } else { "release" }
}

fn git(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git").args(args).current_dir(&app().root).stdin(Stdio::null()).output()
}

/// A clean tree on master.
pub fn git_update_ready() -> (bool, String) {
    let status = match git(&["status", "--porcelain"]) { Ok(o) => o, Err(e) => return (false, format!("git: {}", e)) };
    if !String::from_utf8_lossy(&status.stdout).trim().is_empty() {
        return (false, "This copy is a git checkout with local changes; pull it yourself.".into());
    }
    let head = match git(&["rev-parse", "--abbrev-ref", "HEAD"]) { Ok(o) => o, Err(e) => return (false, format!("git: {}", e)) };
    if String::from_utf8_lossy(&head.stdout).trim() != "master" {
        return (false, "This copy is a git checkout on another branch; pull it yourself.".into());
    }
    (true, String::new())
}

pub fn can_update(rec: Option<&Value>) -> bool {
    let owned;
    let rec = match rec {
        Some(r) => r,
        None => {
            owned = update_status();
            &owned
        }
    };
    if !truthy(rec.get("updateAvailable")) || updates_off() {
        return false;
    }
    if update_mode() == "git" {
        return git_update_ready().0;
    }
    truthy(rec.get("assets"))
}

fn download(url: &str, dest: &Path, max_bytes: usize) -> Result<(), String> {
    let ua = format!("Bagholder/{}", APP_VERSION);
    let resp = bagholder_market::client::request("GET", url, &[("User-Agent", &ua), ("Accept", "application/octet-stream")], None, Duration::from_secs(120))
        .map_err(|e| format!("{:?}", e))?;
    if resp.body.len() > max_bytes {
        return Err("release archive is larger than expected".into());
    }
    std::fs::write(dest, &resp.body).map_err(|e| e.to_string())
}

fn rel_name(p: &Path, root: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    Some(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for e in entries.flatten() {
        let p = e.path();
        let meta = match std::fs::symlink_metadata(&p) { Ok(m) => m, Err(_) => continue };
        if meta.file_type().is_symlink() {
            // a link could point anywhere: only regular files are installed
            let _ = std::fs::remove_file(&p);
        } else if meta.is_dir() {
            walk(&p, root, out);
        } else if meta.is_file() {
            if let Some(n) = rel_name(&p, root) {
                out.push(n);
            }
        }
    }
}

/// The archive's regular files into `staging`,
/// refusing an archive with any entry that would land outside it. Returns the
/// relative paths written.
fn extract_release(archive: &Path, staging: &Path) -> Result<Vec<String>, String> {
    let list_flag = if cfg!(windows) { "-tf" } else { "-tzf" };
    let listed = Command::new("tar").arg(list_flag).arg(archive).stdin(Stdio::null()).output().map_err(|e| format!("tar: {}", e))?;
    if !listed.status.success() {
        return Err("release archive could not be read".into());
    }
    for name in String::from_utf8_lossy(&listed.stdout).lines() {
        let name = name.trim();
        if name.starts_with('/') || name.starts_with('\\') || name.contains(':') || name.split(['/', '\\']).any(|c| c == "..") {
            return Err(format!("release archive has an unsafe entry: {}", name));
        }
    }
    let x_flag = if cfg!(windows) { "-xf" } else { "-xzf" };
    let out = Command::new("tar").arg(x_flag).arg(archive).arg("-C").arg(staging).stdin(Stdio::null()).output().map_err(|e| format!("tar: {}", e))?;
    if !out.status.success() {
        return Err(format!("release archive could not be unpacked: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let mut written = Vec::new();
    walk(staging, staging, &mut written);
    written.sort();
    if written.is_empty() {
        return Err("release archive is empty".into());
    }
    Ok(written)
}

/// The new executable is there and runs, and says
/// it is the version being installed.
fn check_binary(staging: &Path, names: &[String], tag: &str) -> Result<(), String> {
    if !names.iter().any(|n| n == exe_name()) {
        return Err(format!("release archive has no {}", exe_name()));
    }
    let exe = staging.join(exe_name());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    let mut child = Command::new(&exe)
        .arg("--version")
        .env("BAGHOLDER_NO_BROWSER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("the new version does not run: {}", e))?;
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the new version did not answer --version".into());
            }
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let want = tag.trim_start_matches('v');
    if !out.status.success() || !text.contains(want) {
        return Err(format!("the new version answered {} rather than {}", text.trim(), want));
    }
    Ok(())
}

/// Put `src` at `dest` by a rename within the destination's folder, so the
/// file is never half written. On Windows a running executable cannot be
/// replaced, only renamed aside, so the file in place moves to `.old` first.
fn put_in_place(src: &Path, dest: &Path, copy: bool) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_file_name(format!("{}.new", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    if copy {
        std::fs::copy(src, &tmp)?;
    } else if std::fs::rename(src, &tmp).is_err() {
        // another filesystem: a copy, then the same rename
        std::fs::copy(src, &tmp)?;
    }
    if cfg!(windows) && dest.exists() {
        let old = dest.with_file_name(format!("{}.old", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(dest, &old)?;
    }
    std::fs::rename(&tmp, dest)
}

/// The current copies kept under HOME/previous,
/// the new files put in place, the marker the supervisor watches left.
fn install_files(staging: &Path, names: &[String], tag: &str) -> Result<(), String> {
    let home = &app().home;
    let dir = app_dir();
    let previous = home.join("previous");
    if previous.exists() {
        std::fs::remove_dir_all(&previous).map_err(|e| e.to_string())?;
    }
    for name in names {
        let cur = dir.join(name);
        if cur.exists() {
            let keep = previous.join(name);
            if let Some(p) = keep.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&cur, &keep).map_err(|e| e.to_string())?;
        }
    }
    for name in names {
        if let Err(e) = put_in_place(&staging.join(name), &dir.join(name), false) {
            // a replace failed part way: every file goes back to its previous copy
            rollback();
            return Err(e.to_string());
        }
    }
    std::fs::write(home.join("update-pending"), tag).map_err(|e| e.to_string())
}

/// The previous copies put back.
pub fn rollback() -> bool {
    rollback_in(&app().home, &app_dir())
}

fn rollback_in(home: &Path, dir: &Path) -> bool {
    let previous = home.join("previous");
    if !previous.exists() {
        return false;
    }
    let mut names = Vec::new();
    walk(&previous, &previous, &mut names);
    for name in names {
        let _ = put_in_place(&previous.join(&name), &dir.join(&name), true);
    }
    let _ = std::fs::remove_dir_all(&previous);
    true
}

/// Finish the response in flight, then stop
/// serving so the supervisor restarts the server.
pub fn request_restart() {
    app().exit_code.store(RESTART_CODE, std::sync::atomic::Ordering::SeqCst);
    spawn("bagholder-restart", || {
        std::thread::sleep(Duration::from_millis(500));
        app().stop.store(true, std::sync::atomic::Ordering::SeqCst);
    });
}

fn set_updating(msg: &str) {
    app().state.lock().unwrap().updating = msg.to_string();
}

fn sha256_hex(data: &[u8]) -> String {
    openssl::sha::sha256(data).iter().map(|b| format!("{:02x}", b)).collect()
}

fn install_release(tag: &str, rec: &Value) -> Result<(), String> {
    let assets = rec.get("assets").cloned().unwrap_or(json!({}));
    if !truthy(Some(&assets)) {
        return Err("This release has no downloadable archive.".into());
    }
    set_updating(&format!("Downloading {}…", tag));
    let home = app().home.clone();
    let staging = home.join("staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let archive = home.join(format!("bagholder-{}.{}", tag, archive_ext()));
    let sha_file = home.join("release.sha256");
    download(&f(&assets, "archive"), &archive, UPDATE_MAX_BYTES)?;
    download(&f(&assets, "sha"), &sha_file, 4096)?;
    let want = std::fs::read_to_string(&sha_file).map_err(|e| e.to_string())?.split_whitespace().next().unwrap_or("").trim().to_lowercase();
    let got = sha256_hex(&std::fs::read(&archive).map_err(|e| e.to_string())?);
    if want != got {
        return Err("The download did not match the release's checksum.".into());
    }
    set_updating(&format!("Installing {}…", tag));
    let names = extract_release(&archive, &staging)?;
    check_binary(&staging, &names, tag)?;
    install_files(&staging, &names, tag)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_file(&sha_file);
    Ok(())
}

fn pull(tag: &str) -> Result<(), String> {
    set_updating(&format!("Updating to {}…", tag));
    let (ok, why) = git_update_ready();
    if !ok {
        return Err(why);
    }
    let before = git(&["rev-parse", "HEAD"]).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let r = git(&["pull", "--ff-only"]).map_err(|e| e.to_string())?;
    if !r.status.success() {
        let msg = { let e = String::from_utf8_lossy(&r.stderr).to_string(); if e.is_empty() { String::from_utf8_lossy(&r.stdout).to_string() } else { e } };
        return Err(format!("git pull failed: {}", msg.trim().chars().take(200).collect::<String>()));
    }
    // a checkout runs what it builds: the new sources are built before the restart,
    // and a build that fails puts the previous commit back
    set_updating(&format!("Building {}…", tag));
    let built = Command::new("cargo").args(["build", "--release", "--bins"]).current_dir(&app().root).stdin(Stdio::null()).output();
    if !built.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        if !before.is_empty() {
            let _ = git(&["reset", "--hard", &before]);
        }
        let msg = built.map(|o| String::from_utf8_lossy(&o.stderr).to_string()).unwrap_or_else(|e| e.to_string());
        let last = msg.trim().lines().last().unwrap_or("").chars().take(200).collect::<String>();
        return Err(format!("the new version did not build: {}", last));
    }
    std::fs::write(app().home.join("update-pending"), tag).map_err(|e| e.to_string())
}

/// Bring this copy to `tag`, then restart. Never
/// fails; a failure lands in the state's update error and nothing is changed.
pub fn perform_update(tag: &str, rec: &Value) {
    let done = if update_mode() == "git" { pull(tag) } else { install_release(tag, rec) };
    match done {
        Ok(()) => {
            set_updating("Restarting…");
            log(&format!("bagholder update: {} installed, restarting", tag));
            request_restart();
        }
        Err(e) => {
            {
                let mut st = app().state.lock().unwrap();
                st.updating.clear();
                st.update_error = format!("Update failed: {}", e);
            }
            log(&format!("bagholder update failed: {}", e));
        }
    }
}

/// Begin the update the page asked for, in the
/// background.
pub fn start_update() -> Value {
    if updates_off() {
        return json!({"ok": false, "error": UPDATES_OFF_MESSAGE});
    }
    let rec = update_status();
    {
        let mut st = app().state.lock().unwrap();
        if !st.updating.is_empty() {
            return json!({"ok": true});
        }
        if st.syncing {
            return json!({"ok": false, "error": "Wait for the sync to finish, then update."});
        }
        if !truthy(rec.get("updateAvailable")) || !truthy(rec.get("latest")) {
            return json!({"ok": false, "error": "No update to install."});
        }
        drop(st);
        if !can_update(Some(&rec)) {
            let why = if update_mode() == "git" { git_update_ready().1 } else { "This release has no downloadable archive.".to_string() };
            return json!({"ok": false, "error": why});
        }
        st = app().state.lock().unwrap();
        if !st.updating.is_empty() {
            return json!({"ok": true});
        }
        st.update_error.clear();
        st.updating = format!("Updating to {}…", f(&rec, "latest"));
    }
    let tag = f(&rec, "latest");
    spawn("bagholder-update", move || perform_update(&tag, &rec));
    json!({"ok": true})
}

/// Run the server as a child and start it again whenever
/// it exits asking to be (an update). A restarted server that dies within
/// `healthy_sec` of an update gets the previous files put back and is started
/// once more. `home` is the data folder the update marker lives in.
pub fn supervise(home: &Path, healthy_sec: u64) -> i32 {
    // the path is taken once: an update renames a new file over it, and the
    // running process's own idea of its path can go stale after that
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log(&format!("bagholder: cannot find the executable to supervise: {}", e));
            return 1;
        }
    };
    let dir = exe.canonicalize().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let marker = home.join("update-pending");
    loop {
        let mut child = match Command::new(&exe).args(&args).env("BAGHOLDER_CHILD", "1").spawn() {
            Ok(c) => c,
            Err(e) => {
                log(&format!("bagholder: the server could not be started: {}", e));
                return 1;
            }
        };
        let mut pending = marker.exists();
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) => {}
                Err(_) => break None,
            }
            if pending && started.elapsed() >= Duration::from_secs(healthy_sec) {
                // alive past the window: the update took
                let _ = std::fs::remove_file(&marker);
                let _ = std::fs::remove_dir_all(home.join("previous"));
                pending = false;
                break child.wait().ok();
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        let code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        if code == RESTART_CODE {
            continue;
        }
        if pending && code != 0 {
            let _ = std::fs::remove_file(&marker);
            if rollback_in(home, &dir) {
                log("bagholder update: the new version did not start; the previous one is back");
                continue;
            }
        }
        return code;
    }
}

/// At most hourly.
pub fn check_for_update_if_due() -> Value {
    let rec = update_status();
    if let Some(last) = parse_instant(&f(&rec, "checkedAt")) {
        if now_unix() - last < UPDATE_CHECK_HOURS * 3600.0 {
            return rec;
        }
    }
    check_for_update()
}
