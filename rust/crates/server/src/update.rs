//! The in-app update: the hourly check against the latest GitHub release, the
//! install of that release's prebuilt archive for this platform (or a pull,
//! for a git checkout), and the supervisor that restarts the server into the
//! new version and puts the previous one back when it does not start.
//!
//! A release carries one archive per platform, `bagholder-vX.Y.Z-rust-<target>.tar.gz`
//! (`.zip` on Windows) with its `.sha256` beside it, holding the `bagholder`
//! executable and the page's static files.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use std::sync::Arc;

use crate::app::{log, now_iso, now_unix, parse_instant, spawn, App, APP_VERSION, REPO};

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
pub fn app_dir(app: &Arc<App>) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| app.root.clone())
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

/// One asset attached to a GitHub release.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct GithubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

/// The GitHub release reply, the fields Bagholder reads; the rest of the
/// body is never kept.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) html_url: String,
    pub(crate) assets: Vec<GithubAsset>,
}

/// {archive, sha} download URLs of this platform's archive and its
/// .sha256, when the release carries both.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAssets {
    pub archive: String,
    pub sha: String,
}

/// The last check against GitHub, stored as `update_check` and echoed to the page.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateRecord {
    pub checked_at: String,
    pub ok: bool,
    pub latest: String,
    pub url: String,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<ReleaseAssets>,
}

/// Tests stand in for GitHub here: `(calls, answer)`, `None` answering as offline.
#[cfg(test)]
pub static FAKE_RELEASE: std::sync::Mutex<Option<(usize, Option<GithubRelease>)>> = std::sync::Mutex::new(None);

/// The latest release as GitHub describes it, `None` when it cannot be read.
fn fetch_release() -> Option<GithubRelease> {
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
        let got = bagholder_net::client::request(
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
pub fn check_for_update(app: &Arc<App>) -> UpdateRecord {
    let mut record = UpdateRecord { checked_at: now_iso(), ok: false, latest: String::new(), url: format!("{}/releases/latest", repo_url()), update_available: false, assets: None };
    let rel = fetch_release();
    if let Some(rel) = rel {
        if let Some(latest) = parse_version(&rel.tag_name) {
            let tag = rel.tag_name.clone();
            record.ok = true;
            record.latest = tag.clone();
            if !rel.html_url.is_empty() {
                record.url = rel.html_url.clone();
            }
            let available = Some(latest) > parse_version(APP_VERSION);
            record.update_available = available;
            record.assets = release_assets(&rel);
            if available {
                if let Ok(c) = app.open() {
                    crate::notify::emit(
                        app,
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
    if let Ok(c) = app.open() {
        let _ = bagholder_store::tables::set_meta(&c, "update_check", &bagholder_store::tables::json_text(&serde_json::to_value(&record).unwrap()));
    }
    record
}

/// The last check's record, default when none.
pub fn update_status(app: &Arc<App>) -> UpdateRecord {
    let raw = app.open().ok().and_then(|c| bagholder_store::tables::get_meta(&c, "update_check", "").ok()).unwrap_or_default();
    if raw.is_empty() { return UpdateRecord::default(); }
    serde_json::from_str(&raw).unwrap_or_default()
}

/// This platform's archive in the release `tag`: `bagholder-vX.Y.Z-rust-<target>.tar.gz`
/// (`.zip` on Windows), beside the Python app's `bagholder-vX.Y.Z-web.zip`.
pub fn archive_name(tag: &str) -> String {
    format!("bagholder-{}-rust-{}.{}", tag, target_triple(), archive_ext())
}

/// {archive, sha} download URLs of this platform's
/// archive and its .sha256, when the release carries both.
pub(crate) fn release_assets(rel: &GithubRelease) -> Option<ReleaseAssets> {
    let stem = archive_name(&rel.tag_name);
    let mut archive = String::new();
    let mut sha = String::new();
    for a in &rel.assets {
        if a.name == stem {
            archive = a.browser_download_url.clone();
        } else if a.name == format!("{}.sha256", stem) {
            sha = a.browser_download_url.clone();
        }
    }
    if !archive.is_empty() && !sha.is_empty() { Some(ReleaseAssets { archive, sha }) } else { None }
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

/// The Cargo workspace of a checkout: rust/ under the repository root.
pub fn cargo_dir(app: &Arc<App>) -> PathBuf {
    let nested = app.root.join("rust");
    if nested.join("Cargo.toml").is_file() { nested } else { app.root.clone() }
}

/// 'git' when this copy is a git checkout with git on
/// the path, else 'release'.
pub fn update_mode(app: &Arc<App>) -> &'static str {
    if app.root.join(".git").exists() && which("git") { "git" } else { "release" }
}

fn git(app: &Arc<App>, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git").args(args).current_dir(&app.root).stdin(Stdio::null()).output()
}

/// A clean tree on master.
pub fn git_update_ready(app: &Arc<App>) -> (bool, String) {
    let status = match git(app, &["status", "--porcelain"]) { Ok(o) => o, Err(e) => return (false, format!("git: {}", e)) };
    if !String::from_utf8_lossy(&status.stdout).trim().is_empty() {
        return (false, "This copy is a git checkout with local changes; pull it yourself.".into());
    }
    let head = match git(app, &["rev-parse", "--abbrev-ref", "HEAD"]) { Ok(o) => o, Err(e) => return (false, format!("git: {}", e)) };
    if String::from_utf8_lossy(&head.stdout).trim() != "master" {
        return (false, "This copy is a git checkout on another branch; pull it yourself.".into());
    }
    (true, String::new())
}

pub fn can_update(app: &Arc<App>, rec: Option<&UpdateRecord>) -> bool {
    let owned;
    let rec = match rec {
        Some(r) => r,
        None => {
            owned = update_status(app);
            &owned
        }
    };
    if !rec.update_available || updates_off() {
        return false;
    }
    if update_mode(app) == "git" {
        return git_update_ready(app).0;
    }
    rec.assets.is_some()
}

fn download(url: &str, dest: &Path, max_bytes: usize) -> Result<(), String> {
    let ua = format!("Bagholder/{}", APP_VERSION);
    let resp = bagholder_net::client::request("GET", url, &[("User-Agent", &ua), ("Accept", "application/octet-stream")], None, Duration::from_secs(120))
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

/// The current copies of `names` in `dir` kept under HOME/previous, replacing
/// whatever an earlier update left there.
fn keep_previous(home: &Path, dir: &Path, names: &[String]) -> Result<(), String> {
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
    Ok(())
}

/// The current copies kept under HOME/previous,
/// the new files put in place, the marker the supervisor watches left.
fn install_files(app: &Arc<App>, staging: &Path, names: &[String], tag: &str) -> Result<(), String> {
    let home = &app.home;
    bagholder_store::guard_home(home)?;
    let dir = app_dir(app);
    keep_previous(home, &dir, names)?;
    for name in names {
        if let Err(e) = put_in_place(&staging.join(name), &dir.join(name), false) {
            // a replace failed part way: every file goes back to its previous copy
            rollback(app);
            return Err(e.to_string());
        }
    }
    write_pending(home, &Pending { tag: tag.to_string(), git: None })
}

/// The marker an update leaves for the supervisor (HOME/update-pending): the
/// version going in and, for a git checkout, the commit to go back to. The
/// supervisor reads it when the new server dies inside the healthy window.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub(crate) struct Pending {
    pub tag: String,
    /// A git checkout's way back: its root and the commit before the pull.
    #[serde(default)]
    pub git: Option<GitRestore>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub(crate) struct GitRestore {
    pub root: PathBuf,
    pub commit: String,
}

pub(crate) fn write_pending(home: &Path, p: &Pending) -> Result<(), String> {
    let text = serde_json::to_string(p).map_err(|e| e.to_string())?;
    std::fs::write(home.join("update-pending"), text).map_err(|e| e.to_string())
}

/// The marker as written; an earlier build wrote the bare tag.
fn read_pending(home: &Path) -> Pending {
    let text = std::fs::read_to_string(home.join("update-pending")).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_else(|_| Pending { tag: text.trim().to_string(), git: None })
}

/// Where a failed update is written for the server the supervisor starts next,
/// which says it in the header (`recall_failure`).
const FAILED_FILE: &str = "update-failed";

/// The failure the supervisor left, taken into the header's update error once:
/// the restarted server is the one that can say it.
pub fn recall_failure(app: &Arc<App>) {
    let path = app.home.join(FAILED_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let _ = std::fs::remove_file(&path);
    let text = text.trim();
    if !text.is_empty() {
        app.state.lock().unwrap().update_error = text.to_string();
    }
}

/// The previous copies put back.
pub fn rollback(app: &Arc<App>) -> bool {
    rollback_in(&app.home, &app_dir(app))
}

/// The version before a failed update put back: a git checkout's commit first
/// (the page and sources are the checkout's), then the kept executables.
/// True when the previous executables are in place again.
fn restore_previous(home: &Path, dir: &Path, pending: &Pending) -> bool {
    if let Some(g) = &pending.git {
        // the checkout named, whatever repository the environment points at
        let reset = Command::new("git").args(["reset", "--hard", &g.commit]).current_dir(&g.root).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").stdin(Stdio::null()).output();
        match reset {
            Ok(o) if o.status.success() => {}
            Ok(o) => log(&format!("bagholder update: the checkout could not go back to {}: {}", g.commit, String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => log(&format!("bagholder update: the checkout could not go back to {}: {}", g.commit, e)),
        }
    }
    rollback_in(home, dir)
}

fn rollback_in(home: &Path, dir: &Path) -> bool {
    if bagholder_store::guard_home(home).is_err() {
        return false;
    }
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
pub fn request_restart(app: &Arc<App>) {
    app.exit_code.store(RESTART_CODE, std::sync::atomic::Ordering::SeqCst);
    let a = app.clone();
    spawn("bagholder-restart", move || {
        std::thread::sleep(Duration::from_millis(500));
        a.request_stop();
    });
}

fn set_updating(app: &Arc<App>, msg: &str) {
    app.state.lock().unwrap().updating = msg.to_string();
}

fn sha256_hex(data: &[u8]) -> String {
    openssl::sha::sha256(data).iter().map(|b| format!("{:02x}", b)).collect()
}

fn install_release(app: &Arc<App>, tag: &str, rec: &UpdateRecord) -> Result<(), String> {
    let Some(assets) = &rec.assets else { return Err("This release has no downloadable archive.".into()) };
    set_updating(app, &format!("Downloading {}…", tag));
    let home = app.home.clone();
    let staging = home.join("staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let archive = home.join(format!("bagholder-{}.{}", tag, archive_ext()));
    let sha_file = home.join("release.sha256");
    download(&assets.archive, &archive, UPDATE_MAX_BYTES)?;
    download(&assets.sha, &sha_file, 4096)?;
    let want = std::fs::read_to_string(&sha_file).map_err(|e| e.to_string())?.split_whitespace().next().unwrap_or("").trim().to_lowercase();
    let got = sha256_hex(&std::fs::read(&archive).map_err(|e| e.to_string())?);
    if want != got {
        return Err("The download did not match the release's checksum.".into());
    }
    set_updating(app, &format!("Installing {}…", tag));
    let names = extract_release(&archive, &staging)?;
    check_binary(&staging, &names, tag)?;
    install_files(app, &staging, &names, tag)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_file(&sha_file);
    Ok(())
}

fn pull(app: &Arc<App>, tag: &str) -> Result<(), String> {
    set_updating(app, &format!("Updating to {}…", tag));
    let (ok, why) = git_update_ready(app);
    if !ok {
        return Err(why);
    }
    let before = git(app, &["rev-parse", "HEAD"]).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if before.is_empty() {
        return Err("git could not name the current commit".into());
    }
    // the executables the build replaces are kept, so a new version that does not
    // start is put back without building the old one again
    let dir = app_dir(app);
    let exes = [exe_name().to_string(), if cfg!(windows) { "bagholder-browser.exe".to_string() } else { "bagholder-browser".to_string() }];
    keep_previous(&app.home, &dir, &exes)?;
    let r = git(app, &["pull", "--ff-only"]).map_err(|e| e.to_string())?;
    if !r.status.success() {
        let msg = { let e = String::from_utf8_lossy(&r.stderr).to_string(); if e.is_empty() { String::from_utf8_lossy(&r.stdout).to_string() } else { e } };
        return Err(format!("git pull failed: {}", msg.trim().chars().take(200).collect::<String>()));
    }
    // a checkout runs what it builds: the new sources are built before the restart,
    // and a build that fails puts the previous commit back
    set_updating(app, &format!("Building {}…", tag));
    let built = Command::new("cargo").args(["build", "--release", "--bins"]).current_dir(cargo_dir(app)).stdin(Stdio::null()).output();
    if !built.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        let _ = git(app, &["reset", "--hard", &before]);
        // a build that failed part way may have linked one executable already
        rollback(app);
        let msg = built.map(|o| String::from_utf8_lossy(&o.stderr).to_string()).unwrap_or_else(|e| e.to_string());
        let last = msg.trim().lines().last().unwrap_or("").chars().take(200).collect::<String>();
        return Err(format!("the new version did not build: {}", last));
    }
    write_pending(&app.home, &Pending { tag: tag.to_string(), git: Some(GitRestore { root: app.root.clone(), commit: before }) })
}

/// Bring this copy to `tag`, then restart. Never
/// fails; a failure lands in the state's update error and nothing is changed.
pub fn perform_update(app: &Arc<App>, tag: &str, rec: &UpdateRecord) {
    if let Err(e) = bagholder_store::guard_home(&app.home) {
        log(&format!("bagholder update: {}", e));
        return;
    }
    let done = if update_mode(app) == "git" { pull(app, tag) } else { install_release(app, tag, rec) };
    match done {
        Ok(()) => {
            set_updating(app, "Restarting…");
            log(&format!("bagholder update: {} installed, restarting", tag));
            request_restart(app);
        }
        Err(e) => {
            {
                let mut st = app.state.lock().unwrap();
                st.updating.clear();
                st.update_error = format!("Update failed: {}", e);
            }
            log(&format!("bagholder update failed: {}", e));
        }
    }
}

/// Begin the update the page asked for, in the
/// background.
pub fn start_update(app: &Arc<App>) -> crate::http::OkOr {
    use crate::http::OkOr;
    if updates_off() {
        return OkOr::err(UPDATES_OFF_MESSAGE);
    }
    let rec = update_status(app);
    {
        let mut st = app.state.lock().unwrap();
        if !st.updating.is_empty() {
            return OkOr::ok();
        }
        if st.syncing {
            return OkOr::err("Wait for the sync to finish, then update.");
        }
        if !rec.update_available || rec.latest.is_empty() {
            return OkOr::err("No update to install.");
        }
        drop(st);
        if !can_update(app, Some(&rec)) {
            let why = if update_mode(app) == "git" { git_update_ready(app).1 } else { "This release has no downloadable archive.".to_string() };
            return OkOr::err(why);
        }
        st = app.state.lock().unwrap();
        if !st.updating.is_empty() {
            return OkOr::ok();
        }
        st.update_error.clear();
        st.updating = format!("Updating to {}…", rec.latest);
    }
    let tag = rec.latest.clone();
    let a = app.clone();
    spawn("bagholder-update", move || perform_update(&a, &tag, &rec));
    OkOr::ok()
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
    supervise_child(home, &dir, healthy_sec, || {
        let mut c = Command::new(&exe);
        c.args(&args).env("BAGHOLDER_CHILD", "1");
        c
    })
}

/// The supervisor's loop over the child `command` makes, the copy it runs living
/// in `dir`. A child that dies inside the window after an update gets the
/// version before it back (the kept files, and a git checkout's commit), is
/// started again, and the failure is left for that server to say.
pub(crate) fn supervise_child(home: &Path, dir: &Path, healthy_sec: u64, command: impl Fn() -> Command) -> i32 {
    let marker = home.join("update-pending");
    loop {
        let mut child = match command().spawn() {
            Ok(c) => c,
            Err(e) => {
                log(&format!("bagholder: the server could not be started: {}", e));
                return 1;
            }
        };
        let mut pending = marker.exists();
        let started = Instant::now();
        // The child is simply waited for. Only in the window after an update is it
        // looked at while it runs, to tell "alive past the window" from "died in it";
        // the standard library has no wait with a deadline, so that window -- and
        // nothing after it -- is the one place the supervisor looks again.
        let status = loop {
            if !pending {
                break child.wait().ok();
            }
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) => {}
                Err(_) => break None,
            }
            if started.elapsed() >= Duration::from_secs(healthy_sec) {
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
            let update = read_pending(home);
            let _ = std::fs::remove_file(&marker);
            let back = restore_previous(home, dir, &update);
            let what = if update.tag.is_empty() { "the new version".to_string() } else { update.tag.clone() };
            let _ = std::fs::write(home.join(FAILED_FILE), format!("Update failed: {} did not start.", what));
            if back {
                log(&format!("bagholder update: {} did not start; the previous version is back", what));
                continue;
            }
            log(&format!("bagholder update: {} did not start, and no previous version was kept to go back to", what));
        }
        return code;
    }
}

/// At most hourly.
pub fn check_for_update_if_due(app: &Arc<App>) -> UpdateRecord {
    let rec = update_status(app);
    if let Some(last) = parse_instant(&rec.checked_at) {
        if now_unix() - last < UPDATE_CHECK_HOURS * 3600.0 {
            return rec;
        }
    }
    check_for_update(app)
}
