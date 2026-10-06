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
/// How often GitHub is asked for the latest release. Each ask is conditional (its
/// ETag), so an unchanged release answers 304 with nothing in it; unauthenticated,
/// a 304 still counts against the sixty requests an hour GitHub allows an address
/// (observed 2026-10-06: `x-ratelimit-remaining` fell by one per 304), so every
/// two minutes is thirty an hour, half the allowance. A release is published by
/// `release.yml` the moment its files are all attached, so it is offered within
/// two minutes of being installable.
pub const UPDATE_EVERY: Duration = Duration::from_secs(2 * 60);
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
    crate::app::env_on("BAGHOLDER_NO_UPDATE")
}

/// The header's entry for the update check's own record.
const UPDATE_CHECK: &str = "update-check";

/// A file of the update's own removed: one already gone is removed; any other
/// refusal is the error.
fn remove_left(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{} could not be removed: {e}", path.display())),
    }
}

/// A folder of the update's own removed, as `remove_left` removes a file.
fn remove_left_dir(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{} could not be removed: {e}", path.display())),
    }
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
    /// Its size in bytes, as GitHub states it: what its download is held to.
    pub(crate) size: u64,
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
    /// Each one's size as GitHub states it; none in a record an earlier build kept.
    #[serde(default)]
    pub archive_bytes: Option<u64>,
    #[serde(default)]
    pub sha_bytes: Option<u64>,
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
    /// GitHub's ETag for the answer this record was made from: the next ask sends
    /// it, and an unchanged release answers 304.
    pub etag: String,
}

/// Tests stand in for GitHub here: `(calls, answer)`, `None` answering as offline.
/// A release's ETag is its tag and its assets' names, so a fake answers 304 to
/// the same release asked again, as GitHub does.
#[cfg(test)]
pub static FAKE_RELEASE: std::sync::Mutex<Option<(usize, Option<GithubRelease>)>> = std::sync::Mutex::new(None);

/// What GitHub answered for the latest release.
enum Fetched {
    /// The release, and the ETag to ask with next time.
    Release(GithubRelease, String),
    /// Unchanged since the ETag asked with (a 304).
    Same,
    Failed,
}

/// The latest release as GitHub describes it, asked conditionally with `etag`.
fn fetch_release(etag: &str) -> Fetched {
    #[cfg(test)]
    {
        if let Some((calls, answer)) = FAKE_RELEASE.lock().unwrap().as_mut() {
            *calls += 1;
            return match answer {
                Some(r) => {
                    let tag = format!("\"{}|{}\"", r.tag_name, r.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(","));
                    if !etag.is_empty() && etag == tag { Fetched::Same } else { Fetched::Release(r.clone(), tag) }
                }
                None => Fetched::Failed,
            };
        }
        return Fetched::Failed;
    }
    #[allow(unreachable_code)]
    {
        let ua = format!("Bagholder/{}", APP_VERSION);
        let mut headers = vec![("Accept", "application/vnd.github+json"), ("User-Agent", ua.as_str())];
        if !etag.is_empty() {
            headers.push(("If-None-Match", etag));
        }
        let Ok(r) = bagholder_net::client::request_any("GET", &release_url(), &headers, None, Duration::from_secs(30)) else { return Fetched::Failed };
        match r.status {
            304 => Fetched::Same,
            200 => match serde_json::from_slice(&r.body) {
                Ok(rel) => Fetched::Release(rel, r.headers.iter().find(|(k, _)| k == "etag").map(|(_, v)| v.clone()).unwrap_or_default()),
                Err(_) => Fetched::Failed,
            },
            _ => Fetched::Failed,
        }
    }
}

/// The latest release against APP_VERSION, the
/// record stored in meta. Never fails: a record that could not be kept is said
/// in the header until one is.
pub fn check_for_update(app: &Arc<App>) -> UpdateRecord {
    let mut record = UpdateRecord { checked_at: now_iso(), ok: false, latest: String::new(), url: format!("{}/releases/latest", repo_url()), update_available: false, assets: None, etag: String::new() };
    // the record kept is asked with its ETag: an unchanged release is that record, checked now
    let kept = update_status(app).ok().filter(|r| r.ok && !r.etag.is_empty());
    let rel = match fetch_release(kept.as_ref().map(|r| r.etag.as_str()).unwrap_or("")) {
        Fetched::Release(rel, etag) => {
            record.etag = etag;
            Some(rel)
        }
        Fetched::Same => {
            // the release is the one kept; whether it is newer is this build's to say
            // (a copy that has just updated runs it), so it is said again below
            record = UpdateRecord { checked_at: now_iso(), update_available: false, ..kept.expect("asked with its ETag") };
            None
        }
        Fetched::Failed => None,
    };
    if let Some(rel) = &rel {
        if parse_version(&rel.tag_name).is_some() {
            record.ok = true;
            record.latest = rel.tag_name.clone();
            if !rel.html_url.is_empty() {
                record.url = rel.html_url.clone();
            }
            record.assets = release_assets(rel);
        }
    }
    if let Some(latest) = parse_version(&record.latest).filter(|_| record.ok) {
        let tag = record.latest.clone();
        {
            // newer, and ready for this copy to take: a copy that installs archives
            // waits until this platform's archive and its checksum are attached (the
            // release is published before its archives are built), so nothing is
            // offered that cannot be pressed; the next check offers it
            let installable = updates_off() || update_mode(app) == "git" || record.assets.is_some();
            let available = Some(latest) > parse_version(APP_VERSION) && installable;
            record.update_available = available;
            if available {
                // a notice that could not be recorded is said in the header until one is
                crate::notify::tell(
                    app,
                    "updates",
                    &format!("update:{}", tag),
                    &format!("Bagholder {} is available", tag),
                    if updates_off() { "Pull the new image." } else { "Update from the header." },
                    None,
                );
            }
        }
    }
    let kept = app.cache().and_then(|c| bagholder_store::tables::set_meta(&c, CHECK_KEY, &bagholder_store::tables::json_text(&serde_json::to_value(&record).unwrap())));
    // the header shows what the check found as soon as it is kept
    app.events.signal();
    match kept {
        Ok(()) => crate::feeds::feed_answered(app, UPDATE_CHECK),
        Err(e) => crate::feeds::feed_failed(app, UPDATE_CHECK, format!("The update check could not be kept: {e}")),
    }
    record
}

/// The market cache's `meta` key the last check's record is kept under.
pub const CHECK_KEY: &str = "update_check";

/// The last check's record, default when none has been kept; one that cannot be
/// read is the error.
pub fn update_status(app: &Arc<App>) -> Result<UpdateRecord, String> {
    let raw = app.cache().and_then(|c| bagholder_store::tables::get_meta(&c, CHECK_KEY, "")).map_err(|e| format!("The update check could not be read: {e}"))?;
    if raw.is_empty() { return Ok(UpdateRecord::default()); }
    let mut rec: UpdateRecord = serde_json::from_str(&raw).map_err(|e| format!("The update check could not be read: {e}"))?;
    // a record kept by an earlier build offers what this build already runs: a copy
    // that has just updated reads it before its first check, and is offered nothing
    rec.update_available &= parse_version(&rec.latest) > parse_version(APP_VERSION);
    Ok(rec)
}

/// This platform's archive in the release `tag`: `bagholder-vX.Y.Z-rust-<target>.tar.gz`
/// (`.zip` on Windows).
pub fn archive_name(tag: &str) -> String {
    format!("bagholder-{}-rust-{}.{}", tag, target_triple(), archive_ext())
}

/// {archive, sha} download URLs of this platform's
/// archive and its .sha256, when the release carries both.
pub(crate) fn release_assets(rel: &GithubRelease) -> Option<ReleaseAssets> {
    let stem = archive_name(&rel.tag_name);
    let mut archive = String::new();
    let mut sha = String::new();
    let (mut archive_bytes, mut sha_bytes) = (None, None);
    for a in &rel.assets {
        if a.name == stem {
            archive = a.browser_download_url.clone();
            archive_bytes = Some(a.size);
        } else if a.name == format!("{}.sha256", stem) {
            sha = a.browser_download_url.clone();
            sha_bytes = Some(a.size);
        }
    }
    if !archive.is_empty() && !sha.is_empty() { Some(ReleaseAssets { archive, sha, archive_bytes, sha_bytes }) } else { None }
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

/// The program `cmd` on the PATH (on Windows also as `.exe` or `.cmd`, which is how npm is installed there).
fn find_program(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) { vec![format!("{cmd}.exe"), format!("{cmd}.cmd"), cmd.to_string()] } else { vec![cmd.to_string()] };
    std::env::split_paths(&path).find_map(|d| names.iter().map(|n| d.join(n)).find(|p| p.is_file()))
}

/// The last line a failed command wrote, to say why it failed.
fn last_line(out: &std::io::Result<std::process::Output>) -> String {
    let msg = match out {
        Ok(o) => {
            let e = String::from_utf8_lossy(&o.stderr).to_string();
            if e.trim().is_empty() { String::from_utf8_lossy(&o.stdout).to_string() } else { e }
        }
        Err(e) => e.to_string(),
    };
    msg.trim().lines().last().unwrap_or("").chars().take(200).collect()
}

/// Build what a checkout at `root` runs, from its sources as they are now: the
/// page (`npm ci`, then `npm run build` in `web/`), which the server's build
/// carries, then the server (`cargo build --release --bins` in `cargo_dir`).
/// `find` names each program's path. The first step that fails is the error,
/// and nothing after it runs.
pub(crate) fn build_checkout(root: &Path, cargo_dir: &Path, find: &dyn Fn(&str) -> Option<PathBuf>) -> Result<(), String> {
    let run = |program: &Path, args: &[&str], dir: &Path| {
        let out = Command::new(program).args(args).current_dir(dir).stdin(Stdio::null()).output();
        if out.as_ref().is_ok_and(|o| o.status.success()) { Ok(()) } else { Err(last_line(&out)) }
    };
    let web = root.join("web");
    if web.join("package.json").is_file() {
        let npm = find("npm").ok_or("the page could not be built: npm is not on the PATH (install Node.js)")?;
        for step in [&["ci"][..], &["run", "build"][..]] {
            run(&npm, step, &web).map_err(|e| format!("the page did not build: npm {} failed: {e}", step.join(" ")))?;
        }
    }
    let cargo = find("cargo").ok_or("the new version could not be built: cargo is not on the PATH")?;
    run(&cargo, &["build", "--release", "--bins"], cargo_dir).map_err(|e| format!("the new version did not build: {e}"))
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

pub fn can_update(app: &Arc<App>, rec: &UpdateRecord) -> bool {
    if !rec.update_available || updates_off() {
        return false;
    }
    if update_mode(app) == "git" {
        return git_update_ready(app).0;
    }
    rec.assets.is_some()
}

/// GitHub's hosts a release asset comes from: the API and the download host it
/// redirects to (`objects.githubusercontent.com`, `release-assets.githubusercontent.com`).
fn github_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ["github.com", "githubusercontent.com"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// A release asset streamed to disk as it arrives, no larger than `max_bytes`, the
/// header told how much of it has come (`Downloading v2.2.6… 34%`).
fn download(app: &Arc<App>, tag: &str, url: &str, dest: &Path, max_bytes: u64) -> Result<(), String> {
    let ua = format!("Bagholder/{}", APP_VERSION);
    let mut f = std::fs::File::create(dest).map_err(|e| format!("{} could not be written: {e}", dest.display()))?;
    let mut said: Option<u64> = None;
    let mut progress = |written: u64| {
        let pct = if max_bytes > 0 { written * 100 / max_bytes } else { 0 };
        if said != Some(pct) {
            said = Some(pct);
            set_updating(app, &format!("Downloading {tag}… {pct}%"));
        }
        true
    };
    bagholder_net::client::download(url, &[("User-Agent", &ua), ("Accept", "application/octet-stream")], Duration::from_secs(120), &mut f, max_bytes, &github_host, &mut progress).map_err(|e| e.to_string())?;
    Ok(())
}

fn rel_name(p: &Path, root: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    Some(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
}

/// The regular files under `dir`, by their names relative to `root`; a folder or
/// an entry that cannot be read is the error, never a file left out.
fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{} could not be read: {e}", dir.display()))?;
    for e in entries {
        let p = e.map_err(|e| format!("{} could not be read: {e}", dir.display()))?.path();
        let meta = std::fs::symlink_metadata(&p).map_err(|e| format!("{} could not be read: {e}", p.display()))?;
        if meta.file_type().is_symlink() {
            // a link could point anywhere: only regular files are installed
            remove_left(&p)?;
        } else if meta.is_dir() {
            walk(&p, root, out)?;
        } else if meta.is_file() {
            if let Some(n) = rel_name(&p, root) {
                out.push(n);
            }
        }
    }
    Ok(())
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
    walk(staging, staging, &mut written)?;
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
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("the new version could not be made runnable: {e}"))?;
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
                // one that has exited is killed without error: any refusal is said with the rest
                return Err(match child.kill().and_then(|()| child.wait()) {
                    Ok(_) => "the new version did not answer --version".into(),
                    Err(e) => format!("the new version did not answer --version, and could not be stopped: {e}"),
                });
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
        match std::fs::remove_file(&old) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
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
            return Err(match rollback(app) {
                Ok(_) => e.to_string(),
                Err(back) => format!("{e}; and the previous version could not be put back: {back}"),
            });
        }
    }
    write_pending(home, &Pending { tag: tag.to_string(), git: None, at: Some(jiff::Timestamp::now().to_string()) })
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
    /// When the update was put in place: a store snapshot taken since is the store
    /// as the update found it, before its migrations. None in a marker an earlier
    /// build wrote.
    #[serde(default)]
    pub at: Option<String>,
}

/// The stores a migration may change, by their files in the data folder.
const STORE_FILES: [&str; 3] = [bagholder_book::BOOK_FILE, crate::figures::CACHE_FILE, crate::figures::OLD_FILE];

/// The stores put back as the update found them, from the snapshots its migrations
/// took (`docs/plans/stage-money.md`, part D): a store the update migrated goes back
/// to the version the previous build reads. A store no snapshot covers was not
/// migrated and stays. Err when one cannot be put back: the previous build would
/// refuse that store, so nothing of it is put back either.
fn restore_stores(home: &Path, pending: &Pending) -> Result<(), String> {
    let Some(at) = &pending.at else { return Ok(()) };
    let at: jiff::Timestamp = at.parse().map_err(|e| format!("the update's marker names a time that does not read: {e}"))?;
    let dir = home.join("snapshots");
    for file in STORE_FILES {
        let stem = file.trim_end_matches(".db");
        let snap = bagholder_sqlite::migrate::snapshot_since(&dir, stem, at).map_err(|e| format!("the snapshots could not be read: {e}"))?;
        let Some(snap) = snap else { continue };
        let target = home.join(file);
        // the log beside the file belongs to the migrated version: it goes with it
        for side in ["-wal", "-shm"] {
            remove_left(&home.join(format!("{file}{side}")))?;
        }
        put_in_place(&snap, &target, true).map_err(|e| format!("{file} could not be put back as it was before the update: {e}"))?;
    }
    Ok(())
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

/// The marker as written; an earlier build wrote the bare tag. A marker that
/// cannot be read is the error.
fn read_pending(home: &Path) -> Result<Pending, String> {
    let text = std::fs::read_to_string(home.join("update-pending")).map_err(|e| format!("the update's marker could not be read: {e}"))?;
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| Pending { tag: text.trim().to_string(), git: None, at: None }))
}

/// Where a failed update is written for the server the supervisor starts next,
/// which says it in the header (`recall_failure`).
const FAILED_FILE: &str = "update-failed";

/// The failure the supervisor left, taken into the header's update error once:
/// the restarted server is the one that can say it.
pub fn recall_failure(app: &Arc<App>) {
    let path = app.home.join(FAILED_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            app.state.lock().unwrap().update_error = format!("What the last update came to could not be read: {e}");
            return;
        }
    };
    // said once: a failure left that cannot be taken away would be said again at every start
    let taken = remove_left(&path);
    let text = text.trim();
    let said = match taken {
        Ok(()) => text.to_string(),
        Err(e) => format!("{text} {e}").trim().to_string(),
    };
    if !said.is_empty() {
        app.state.lock().unwrap().update_error = said;
    }
}

/// The previous copies put back.
pub fn rollback(app: &Arc<App>) -> Result<bool, String> {
    rollback_in(&app.home, &app_dir(app))
}

/// The version before a failed update put back: a git checkout's commit first
/// (the page and sources are the checkout's), then the kept executables.
/// True when the previous executables are in place again, false when none were
/// kept; a part that could not be put back is the error, every other part put
/// back all the same.
fn restore_previous(home: &Path, dir: &Path, pending: &Pending) -> Result<bool, String> {
    // the stores first: a previous build put back over stores it cannot read would
    // not start either, so a store that cannot be put back keeps the new version
    if let Err(e) = restore_stores(home, pending) {
        return Err(format!("{e}; the new version is left in place"));
    }
    let mut failed = vec![];
    if let Some(g) = &pending.git {
        // the checkout named, whatever repository the environment points at
        let reset = Command::new("git").args(["reset", "--hard", &g.commit]).current_dir(&g.root).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").stdin(Stdio::null()).output();
        match reset {
            Ok(o) if o.status.success() => {}
            Ok(o) => failed.push(format!("the checkout could not go back to {}: {}", g.commit, String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => failed.push(format!("the checkout could not go back to {}: {}", g.commit, e)),
        }
    }
    let back = rollback_in(home, dir);
    if let Err(e) = &back {
        failed.push(e.clone());
    }
    if failed.is_empty() { back } else { Err(failed.join("; ")) }
}

fn rollback_in(home: &Path, dir: &Path) -> Result<bool, String> {
    bagholder_store::guard_home(home)?;
    let previous = home.join("previous");
    if !previous.exists() {
        return Ok(false);
    }
    let mut names = Vec::new();
    walk(&previous, &previous, &mut names)?;
    // every file is put back that can be; the kept copies stay while any could not be
    let failed: Vec<String> = names.iter().filter_map(|name| put_in_place(&previous.join(name), &dir.join(name), true).err().map(|e| format!("{name} could not be put back: {e}"))).collect();
    if !failed.is_empty() {
        return Err(failed.join("; "));
    }
    remove_left_dir(&previous)?;
    Ok(true)
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
    // each held to the size GitHub states for it: never a size of the app's choosing
    let stated = |b: Option<u64>| b.ok_or("The release does not state its download's size: check for the update again.".to_string());
    download(app, tag, &assets.archive, &archive, stated(assets.archive_bytes)?)?;
    let mut f = std::fs::File::create(&sha_file).map_err(|e| format!("{} could not be written: {e}", sha_file.display()))?;
    let ua = format!("Bagholder/{}", APP_VERSION);
    bagholder_net::client::download(&assets.sha, &[("User-Agent", &ua)], Duration::from_secs(120), &mut f, stated(assets.sha_bytes)?, &github_host, &mut |_| true).map_err(|e| e.to_string())?;
    let want = std::fs::read_to_string(&sha_file).map_err(|e| e.to_string())?.split_whitespace().next().unwrap_or("").trim().to_lowercase();
    let got = sha256_hex(&std::fs::read(&archive).map_err(|e| e.to_string())?);
    if want != got {
        return Err("The download did not match the release's checksum.".into());
    }
    set_updating(app, &format!("Installing {}…", tag));
    let names = extract_release(&archive, &staging)?;
    check_binary(&staging, &names, tag)?;
    // the download is done with before anything is put in place
    remove_left(&archive)?;
    remove_left(&sha_file)?;
    install_files(app, &staging, &names, tag)?;
    // the install's renames have emptied it; the next install removes it first and fails if it cannot
    match remove_left_dir(&staging) {
        Ok(()) => {}
        Err(_left) => {}
    }
    Ok(())
}

fn pull(app: &Arc<App>, tag: &str) -> Result<(), String> {
    set_updating(app, &format!("Updating to {}…", tag));
    let (ok, why) = git_update_ready(app);
    if !ok {
        return Err(why);
    }
    let before = git(app, &["rev-parse", "HEAD"]).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).map_err(|e| format!("git could not name the current commit: {e}"))?;
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
    // a checkout runs what it builds: the new sources, the page and the server that
    // carries it, are built before the restart, and a build that fails puts the
    // previous commit back
    set_updating(app, &format!("Building {}…", tag));
    if let Err(why) = build_checkout(&app.root, &cargo_dir(app), &find_program) {
        let mut back = vec![];
        match git(app, &["reset", "--hard", &before]) {
            Ok(o) if o.status.success() => {}
            Ok(o) => back.push(format!("the checkout could not go back to {before}: {}", String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => back.push(format!("the checkout could not go back to {before}: {e}")),
        }
        // a build that failed part way may have linked one executable already
        if let Err(e) = rollback(app) {
            back.push(e);
        }
        return Err(if back.is_empty() { why } else { format!("{why}; and the previous version could not be put back: {}", back.join("; ")) });
    }
    write_pending(&app.home, &Pending { tag: tag.to_string(), git: Some(GitRestore { root: app.root.clone(), commit: before }), at: Some(jiff::Timestamp::now().to_string()) })
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
    let rec = match update_status(app) {
        Ok(r) => r,
        Err(e) => return OkOr::err(e),
    };
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
        if !can_update(app, &rec) {
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
                // alive past the window: the update took. A marker left would have the
                // next crash in a window roll a good version back: the supervisor has no
                // header, so its console is where that is said
                // the snapshots the update's migrations took are only for putting it back:
                // once it took they go (brief 19, change 11)
                if let Err(e) = remove_left(&marker).and_then(|()| remove_left_dir(&home.join("previous"))).and_then(|()| remove_left_dir(&home.join("snapshots"))) {
                    log(&format!("bagholder update: the update took, but what it kept could not be cleared: {e}"));
                }
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
            // the marker goes, so the version put back is not taken for the update
            let cleared = remove_left(&marker);
            let what = match &update {
                Ok(u) if !u.tag.is_empty() => u.tag.clone(),
                _ => "the new version".to_string(),
            };
            let back = update.and_then(|u| restore_previous(home, dir, &u));
            let said = match (&back, &cleared) {
                (Ok(_), Ok(())) => format!("Update failed: {} did not start.", what),
                (Err(e), _) => format!("Update failed: {} did not start, and the previous version could not be put back: {}.", what, e),
                (Ok(_), Err(e)) => format!("Update failed: {} did not start. {}.", what, e),
            };
            // the server started next says it in the header; the supervisor's console is all that is left when it cannot be written
            if let Err(e) = std::fs::write(home.join(FAILED_FILE), &said) {
                log(&format!("bagholder update: {said} (and this could not be left for the server to say: {e})"));
            }
            if back == Ok(true) {
                log(&format!("bagholder update: {} did not start; the previous version is back", what));
                continue;
            }
            log(&format!("bagholder update: {said}"));
        }
        return code;
    }
}

/// At most hourly.
pub fn check_for_update_if_due(app: &Arc<App>) -> UpdateRecord {
    match update_status(app) {
        Ok(rec) => {
            if let Some(last) = parse_instant(&rec.checked_at) {
                if now_unix() - last < UPDATE_EVERY.as_secs_f64() {
                    return rec;
                }
            }
        }
        // a record that cannot be read is said until one is kept: the check is made again
        Err(e) => crate::feeds::feed_failed(app, UPDATE_CHECK, e),
    }
    check_for_update(app)
}

#[cfg(test)]
mod host_tests {
    #[test]
    fn a_release_asset_is_fetched_only_from_github_on_every_hop() {
        // the release page and the asset host it redirects to (observed 2026-10-04)
        for ok in ["github.com", "api.github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"] {
            assert!(super::github_host(ok), "{ok}");
        }
        for no in ["github.com.evil.io", "evilgithub.com", "githubusercontent.co"] {
            assert!(!super::github_host(no), "{no}");
        }
    }
}
