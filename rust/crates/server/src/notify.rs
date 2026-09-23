//! Notifications: what Bagholder tells the person about while they are not
//! looking.
//!
//! Every event starts here, since the server is what keeps watching while the
//! page sits in a background tab or is closed. Each becomes one row, keyed so
//! the same event is never told twice. Where the computer has a desktop, the
//! server posts the row as the system's own notification under Bagholder's
//! name and icon: on a Mac through a small applet it builds for itself in the
//! home folder, on Windows through a toast registered under Bagholder's name,
//! on Linux through the desktop's notification service. Where it has none, the
//! page shows the row through the browser's Notification API. Every open page
//! keeps the history, fed by a stream of every row as it is made.

use rusqlite::{Connection, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use std::sync::Arc;

use bagholder_diff_derive::Diff;
use ts_rs::TS;

use crate::app::{log, now_iso, App};

pub const KINDS: [&str; 6] = ["fills", "problems", "connection", "updates", "releases", "disclosures"];
pub const RELEASE_SCOPES: [&str; 3] = ["releasesHeld", "releasesWatched", "releasesAll"];
pub const DISCLOSURE_SCOPES: [&str; 3] = ["disclosuresHeld", "disclosuresWatched", "disclosuresAll"];
pub const SETTINGS_KEY: &str = "notify_settings";
/// A comment on the stream this often keeps the connection through proxies
/// and sleeps.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// "browser": never post from this process; the page shows them.
pub const MODE_ENV: &str = "BAGHOLDER_NOTIFY";
pub const APP_NAME: &str = "Bagholder";
pub const MAC_BUNDLE_ID: &str = "com.bagholder.notifier";
pub const WATERMARK: &str = "notify_seen:";

pub fn setting_keys() -> Vec<&'static str> {
    let mut out = vec!["fills", "problems", "connection", "updates"];
    out.extend(RELEASE_SCOPES);
    out.extend(DISCLOSURE_SCOPES);
    out
}

fn url(app: &App) -> String {
    format!("http://127.0.0.1:{}/", *app.port.lock().unwrap())
}

fn icon(app: &App) -> PathBuf {
    app.root.join("favicon.png")
}

/// A JSON value that reads as a plain boolean, or as `false` for anything
/// else -- absent, `null`, a stray non-boolean a hand-edited store might
/// carry. Every switch was always read this leniently (`app::truthy`).
fn lenient_bool<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    Ok(matches!(Option::<Value>::deserialize(d)?, Some(Value::Bool(true))))
}

/// A JSON value that reads as `Some(bool)` only when it is a plain boolean;
/// anything else -- absent, `null`, a non-boolean -- reads as `None`, which a
/// patch leaves untouched. Unknown keys are already ignored by serde itself.
fn lenient_bool_patch<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<bool>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::Bool(b)) => Some(b),
        _ => None,
    })
}

/// One switch per notification kind or Releases/Disclosures scope, in
/// `setting_keys()` order. Every kind is off until turned on from the menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS, Diff)]
#[serde(rename_all = "camelCase", default)]
pub struct NotifySettings {
    #[serde(deserialize_with = "lenient_bool")]
    pub fills: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub problems: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub connection: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub updates: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub releases_held: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub releases_watched: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub releases_all: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub disclosures_held: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub disclosures_watched: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub disclosures_all: bool,
}

impl NotifySettings {
    fn get(&self, key: &str) -> bool {
        match key {
            "fills" => self.fills,
            "problems" => self.problems,
            "connection" => self.connection,
            "updates" => self.updates,
            "releasesHeld" => self.releases_held,
            "releasesWatched" => self.releases_watched,
            "releasesAll" => self.releases_all,
            "disclosuresHeld" => self.disclosures_held,
            "disclosuresWatched" => self.disclosures_watched,
            "disclosuresAll" => self.disclosures_all,
            _ => false,
        }
    }

    fn set(&mut self, key: &str, v: bool) {
        match key {
            "fills" => self.fills = v,
            "problems" => self.problems = v,
            "connection" => self.connection = v,
            "updates" => self.updates = v,
            "releasesHeld" => self.releases_held = v,
            "releasesWatched" => self.releases_watched = v,
            "releasesAll" => self.releases_all = v,
            "disclosuresHeld" => self.disclosures_held = v,
            "disclosuresWatched" => self.disclosures_watched = v,
            "disclosuresAll" => self.disclosures_all = v,
            _ => {}
        }
    }
}

/// `POST /api/notifications/settings`: the switches that changed, by name.
/// Unknown keys and non-booleans are ignored, as they always were.
#[derive(Clone, Copy, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct NotifySettingsPatch {
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub fills: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub problems: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub connection: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub updates: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub releases_held: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub releases_watched: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub releases_all: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub disclosures_held: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub disclosures_watched: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_patch")]
    #[ts(optional)]
    pub disclosures_all: Option<bool>,
}

impl NotifySettingsPatch {
    fn apply(&self, on: &mut NotifySettings) {
        for k in setting_keys() {
            if let Some(v) = self.get(k) {
                on.set(k, v);
            }
        }
    }

    fn get(&self, key: &str) -> Option<bool> {
        match key {
            "fills" => self.fills,
            "problems" => self.problems,
            "connection" => self.connection,
            "updates" => self.updates,
            "releasesHeld" => self.releases_held,
            "releasesWatched" => self.releases_watched,
            "releasesAll" => self.releases_all,
            "disclosuresHeld" => self.disclosures_held,
            "disclosuresWatched" => self.disclosures_watched,
            "disclosuresAll" => self.disclosures_all,
            _ => None,
        }
    }
}

/// The kinds, the native channel, and the unread count -- what the header's
/// stream carries, and `GET /api/notifications` besides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
pub struct NotifyStatus {
    #[serde(flatten)]
    #[ts(flatten)]
    pub settings: NotifySettings,
    pub native: String,
    pub unread: i64,
}

/// Every kind off until it is turned on from the menu.
pub fn settings(conn: &Connection) -> Result<NotifySettings> {
    let raw = bagholder_store::tables::get_meta(conn, SETTINGS_KEY, "")?;
    Ok(serde_json::from_str(if raw.is_empty() { "{}" } else { &raw }).unwrap_or_default())
}

/// Whether any Releases notification set is on.
pub fn any_release_scope(conn: &Connection) -> bool {
    !scopes(conn, &RELEASE_SCOPES, "releases").is_empty()
}

fn scopes(conn: &Connection, keys: &[&str], prefix: &str) -> Vec<String> {
    let on = settings(conn).unwrap_or_default();
    keys.iter().filter(|k| on.get(k)).map(|k| k[prefix.len()..].to_lowercase()).collect()
}

pub fn disclosure_scopes(conn: &Connection) -> Vec<String> {
    scopes(conn, &DISCLOSURE_SCOPES, "disclosures")
}

pub fn release_scopes(conn: &Connection) -> Vec<String> {
    scopes(conn, &RELEASE_SCOPES, "releases")
}

pub fn kind_on(conn: &Connection, kind: &str) -> bool {
    match kind {
        "disclosures" => !disclosure_scopes(conn).is_empty(),
        "releases" => !release_scopes(conn).is_empty(),
        k => settings(conn).map(|s| s.get(k)).unwrap_or(false),
    }
}

pub fn set_settings(conn: &Connection, patch: &NotifySettingsPatch) -> Result<NotifySettings> {
    let mut cur = settings(conn)?;
    patch.apply(&mut cur);
    bagholder_store::tables::set_meta(conn, SETTINGS_KEY, &bagholder_store::tables::json_text(&serde_json::to_value(&cur).unwrap()))?;
    Ok(cur)
}

/// The `notifications` document: the bell, as a page showing it is sent.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsDoc {
    pub rows: Vec<bagholder_store::feeds::Notification>,
    pub unread: i64,
}

/// The kinds, the native channel, and the unread count.
pub fn status(conn: &Connection) -> Result<NotifyStatus> {
    Ok(NotifyStatus { settings: settings(conn)?, native: native_channel(), unread: bagholder_store::feeds::unread_notifications(conn)? })
}

/// `GET /api/notifications`: the settings, kinds, the list and the unread
/// count -- everything the bell's panel needs, opened cold.
#[derive(Serialize, TS)]
pub struct NotificationsAnswer {
    pub ok: bool,
    pub settings: NotifyStatus,
    pub kinds: Vec<String>,
    pub rows: Vec<bagholder_store::feeds::Notification>,
    pub unread: i64,
}

/// `POST /api/notifications/settings`.
#[derive(Serialize, TS)]
pub struct NotifySettingsAnswer {
    pub ok: bool,
    pub settings: NotifyStatus,
}

/// `POST /api/notifications/test`.
#[derive(Serialize, TS)]
pub struct NotifyTestAnswer {
    pub ok: bool,
    pub id: i64,
}

/// `POST /api/notifications/read`.
#[derive(Serialize, TS)]
pub struct NotificationsReadAnswer {
    pub ok: bool,
    pub read: usize,
}

/// `POST /api/notifications/seen`.
#[derive(Serialize, TS)]
pub struct NotificationsSeenAnswer {
    pub ok: bool,
    pub seen: usize,
}

/// `POST /api/notifications/clear`.
#[derive(Serialize, TS)]
pub struct NotificationsClearAnswer {
    pub ok: bool,
    pub cleared: usize,
}

/// Notifications by id, for `read` and `seen`. For `read`, no ids at all
/// means every one.
#[derive(Clone, Debug, Default, Deserialize, TS)]
pub struct NotificationIds {
    pub ids: Option<Vec<i64>>,
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

pub fn native_channel() -> String {
    #[cfg(test)]
    if let Some(c) = test_hooks::CHANNEL.lock().unwrap().clone() {
        return c;
    }
    let mode = std::env::var(MODE_ENV).unwrap_or_default();
    let display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false) || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
    channel_for(&mode, std::env::consts::OS, &|n| which(n).is_some(), display)
}

/// The channel a system offers: its mode, its kind, the tools it has and
/// whether it has a desktop.
pub(crate) fn channel_for(mode: &str, os: &str, has: &dyn Fn(&str) -> bool, display: bool) -> String {
    let mode = mode.trim().to_lowercase();
    if ["browser", "off", "0", "none"].contains(&mode.as_str()) {
        return String::new();
    }
    if os == "macos" {
        return if has("osascript") { "mac".into() } else { String::new() };
    }
    if os == "windows" {
        return if has("powershell.exe") || has("pwsh.exe") || has("powershell") { "windows".into() } else { String::new() };
    }
    if has("notify-send") && display {
        return "linux".into();
    }
    String::new()
}

#[cfg(test)]
pub(crate) mod test_hooks {
    use std::sync::Mutex;
    /// A channel standing in for the system's.
    pub static CHANNEL: Mutex<Option<String>> = Mutex::new(None);
    /// While set, deliveries are recorded here and never reach the system.
    pub static DELIVERED: Mutex<Option<Vec<(String, String, String)>>> = Mutex::new(None);
    /// The heartbeat, in milliseconds, when set.
    pub static HEARTBEAT_MS: Mutex<Option<u64>> = Mutex::new(None);
}

fn heartbeat() -> Duration {
    #[cfg(test)]
    if let Some(ms) = *test_hooks::HEARTBEAT_MS.lock().unwrap() {
        return Duration::from_millis(ms);
    }
    HEARTBEAT
}

/// What a stream has that it has not shown before, and
/// nothing it held when it was first met.
///
/// Each stream carries a mark: the newest moment it has shown and the items it
/// showed at that moment. A stream met for the first time shows nothing and
/// its mark is set from it; after that it shows what is newer than the mark,
/// and what shares the mark's moment without having been shown.
pub fn fresh_since<T: Clone, A, I, S>(conn: &Connection, stream: &str, items: &[T], at: A, ident: I, seen: S) -> Vec<T>
where
    A: Fn(&T) -> String,
    I: Fn(&T) -> String,
    S: Fn(&T) -> bool,
{
    let key = format!("{}{}", WATERMARK, stream);
    let raw = bagholder_store::tables::get_meta(conn, &key, "").unwrap_or_default();
    let (mark, shown_raw) = match raw.split_once('|') { Some((a, b)) => (a.to_string(), b.to_string()), None => (raw.clone(), String::new()) };
    let shown: Vec<String> = shown_raw.split(',').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect();
    let stamped: Vec<(String, String, &T)> = items.iter().map(|i| (at(i), ident(i), i)).collect();
    let newest = stamped.iter().map(|(w, _, _)| w.clone()).max().unwrap_or_default();
    let remember = |top: &str| {
        let mut at_top: Vec<String> = stamped.iter().filter(|(w, _, _)| w == top).map(|(_, i, _)| i.clone()).collect();
        if top == mark {
            at_top.extend(shown.iter().cloned());
        }
        at_top.sort();
        at_top.dedup();
        let _ = bagholder_store::tables::set_meta(conn, &key, &format!("{}|{}", top, at_top.join(",")));
    };
    if raw.is_empty() {
        if !newest.is_empty() {
            remember(&newest);
        }
        return vec![];
    }
    let out: Vec<T> = stamped
        .iter()
        .filter(|(w, i, v)| (*w > mark || (*w == mark && !shown.contains(i))) && !seen(v))
        .map(|(_, _, v)| (*v).clone())
        .collect();
    if !out.is_empty() || newest > mark {
        remember(if newest > mark { &newest } else { &mark });
    }
    out
}

/// The id a stream item is known by when no other is given.
#[cfg(test)]
pub fn default_ident(i: &Value) -> String {
    match i {
        Value::Object(_) => crate::app::f(i, "id"),
        other => crate::app::s(Some(other)),
    }
}

/// This app's notification streams: the bell that wakes them to look again, and
/// the one thread that shows the system's own notifications, started with the
/// first (`enqueue`).
#[derive(Default)]
pub struct NotifyState {
    bell: (Mutex<u64>, Condvar),
    worker: OnceLock<Mutex<std::sync::mpsc::Sender<(String, String, String)>>>,
}

impl NotifyState {
    /// Wake every notification stream to look again: something was posted, or
    /// the app is stopping.
    pub fn wake_streams(&self) {
        let (m, c) = &self.bell;
        *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        c.notify_all();
    }
}

/// One notification, if its kind is on and this key has not
/// been told before.
pub fn emit(app: &Arc<App>, conn: &Connection, kind: &str, key: &str, title: &str, body: &str, extra: Option<bagholder_store::feeds::NotificationExtra>) -> Option<bagholder_store::feeds::Notification> {
    if !KINDS.contains(&kind) || !kind_on(conn, kind) {
        return None;
    }
    post(app, conn, kind, key, title, body, extra)
}

pub fn test_notification(app: &Arc<App>, conn: &Connection) -> Option<bagholder_store::feeds::Notification> {
    let stamp = {
        let now = crate::app::now_unix();
        let micros = ((now.fract()) * 1_000_000.0) as i64;
        format!("{}{:06}", now_iso().replace(['-', ':', 'T', 'Z'], ""), micros)
    };
    post(app, conn, "test", &format!("test:{}", stamp), APP_NAME, "Notifications reach you here.", None)
}

fn post(app: &Arc<App>, conn: &Connection, kind: &str, key: &str, title: &str, body: &str, extra: Option<bagholder_store::feeds::NotificationExtra>) -> Option<bagholder_store::feeds::Notification> {
    let channel = native_channel();
    // posted from here, the row is the server's own to show: seen from the start
    let row = bagholder_store::feeds::add_notification(conn, kind, key, title, body, extra.as_ref(), !channel.is_empty(), &now_iso()).ok()??;
    if !channel.is_empty() {
        enqueue(app.clone(), row.title.clone(), row.body.clone(), channel);
    }
    app.notify.wake_streams();
    Some(row)
}

fn enqueue(app: Arc<App>, title: String, body: String, chan: String) {
    let tx = app.notify.worker.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<(String, String, String)>();
        // the app holds the sender and the thread only a weak hold on the app: when
        // the app goes, the sender goes with it and the thread ends
        let weak = Arc::downgrade(&app);
        crate::app::spawn("bagholder-notify", move || {
            for (title, body, ch) in rx {
                let Some(app) = weak.upgrade() else { return };
                if !deliver(&app, &ch, &title, &body) {
                    log(&format!("bagholder notify: {} not shown ({})", title, ch));
                }
            }
        });
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap().send((title, body, chan));
}

/// Post one notification through the system.
pub fn deliver(app: &App, channel: &str, title: &str, body: &str) -> bool {
    #[cfg(test)]
    {
        if let Some(v) = test_hooks::DELIVERED.lock().unwrap().as_mut() {
            v.push((channel.to_string(), title.to_string(), body.to_string()));
            return true;
        }
        if !channel.is_empty() {
            panic!("a test reached the system's notifications");
        }
    }
    match channel {
        "mac" => mac_deliver(app, title, body),
        "windows" => windows_deliver(app, title, body),
        "linux" => linux_deliver(app, title, body),
        _ => false,
    }
}

// --- macOS: an applet of Bagholder's own ---------------------------------------

fn mac_script(app: &App) -> String {
    format!(
        "on run\n\tset t to system attribute \"BAGHOLDER_TITLE\"\n\tif t is \"\" then\n\t\topen location \"{}\"\n\telse\n\t\tdisplay notification (system attribute \"BAGHOLDER_BODY\") with title t\n\tend if\nend run\n",
        url(app)
    )
}

pub fn mac_app_path(app: &App) -> PathBuf {
    app.home.join(format!("{}.app", APP_NAME))
}

fn mac_stamp(app: &App) -> String {
    let mut h = openssl::sha::Sha1::new();
    h.update(mac_script(app).as_bytes());
    if let Ok(b) = std::fs::read(icon(app)) {
        h.update(&b);
    }
    h.finish().iter().map(|b| format!("{:02x}", b)).collect()
}

fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// The applet, built once and again whenever its script, the
/// app's address or the icon changes.
pub fn mac_app(app: &App) -> Option<PathBuf> {
    let appdir = mac_app_path(app);
    let stamp_file = appdir.join("Contents/Resources/bagholder.stamp");
    let want = mac_stamp(app);
    if appdir.join("Contents/MacOS/applet").exists() && std::fs::read_to_string(&stamp_file).ok().as_deref() == Some(want.as_str()) {
        return Some(appdir);
    }
    match mac_build(app, &appdir, &want) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("bagholder notify: the notifier app could not be built: {}", e));
            None
        }
    }
}

fn mac_build(app: &App, appdir: &Path, stamp: &str) -> std::io::Result<Option<PathBuf>> {
    if which("osacompile").is_none() {
        return Ok(None);
    }
    let work = std::env::temp_dir().join(format!("bagholder-notifier-{}", crate::app::uuid4()));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> std::io::Result<Option<PathBuf>> {
        let script = work.join("notifier.applescript");
        std::fs::write(&script, mac_script(app))?;
        let built = work.join(format!("{}.app", APP_NAME));
        let ws = |p: &Path| p.to_string_lossy().into_owned();
        if !run("osacompile", &["-o", &ws(&built), &ws(&script)]) {
            return Err(std::io::Error::other("osacompile failed"));
        }
        let plist = built.join("Contents/Info.plist");
        if !run("plutil", &["-replace", "CFBundleIdentifier", "-string", MAC_BUNDLE_ID, &ws(&plist)])
            || !run("plutil", &["-replace", "CFBundleDisplayName", "-string", APP_NAME, &ws(&plist)])
        {
            return Err(std::io::Error::other("plutil failed"));
        }
        if let Some(icns) = mac_icon(app, &work) {
            let res = built.join("Contents/Resources");
            std::fs::copy(&icns, res.join("applet.icns"))?;
            let car = res.join("Assets.car");
            if car.exists() {
                std::fs::remove_file(car)?;
            }
            run("plutil", &["-remove", "CFBundleIconName", &ws(&plist)]);
        }
        std::fs::write(built.join("Contents/Resources/bagholder.stamp"), stamp)?;
        if which("codesign").is_some() {
            // sealed last, with the stamp inside the seal
            run("codesign", &["--force", "--sign", "-", &ws(&built)]);
        }
        if appdir.exists() {
            std::fs::remove_dir_all(appdir)?;
        }
        if let Some(parent) = appdir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&built, appdir)?;
        Ok(Some(appdir.to_path_buf()))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn mac_icon(app: &App, work: &Path) -> Option<PathBuf> {
    let src = icon(app);
    if which("sips").is_none() || which("iconutil").is_none() || !src.exists() {
        return None;
    }
    let iconset = work.join("icon.iconset");
    std::fs::create_dir_all(&iconset).ok()?;
    let sizes: [(u32, &[&str]); 6] = [
        (16, &["icon_16x16.png"]),
        (32, &["icon_16x16@2x.png", "icon_32x32.png"]),
        (64, &["icon_32x32@2x.png"]),
        (128, &["icon_128x128.png"]),
        (256, &["icon_128x128@2x.png", "icon_256x256.png"]),
        (512, &["icon_256x256@2x.png", "icon_512x512.png"]),
    ];
    for (size, names) in sizes {
        let first = iconset.join(names[0]);
        let sz = size.to_string();
        if !run("sips", &["-z", &sz, &sz, &src.to_string_lossy(), "--out", &first.to_string_lossy()]) {
            return None;
        }
        for other in &names[1..] {
            std::fs::copy(&first, iconset.join(other)).ok()?;
        }
    }
    let icns = work.join("icon.icns");
    if !run("iconutil", &["-c", "icns", &iconset.to_string_lossy(), "-o", &icns.to_string_lossy()]) {
        return None;
    }
    if icns.exists() { Some(icns) } else { None }
}

fn mac_deliver(app: &App, title: &str, body: &str) -> bool {
    if let Some(appdir) = mac_app(app) {
        let out = mac_open_command(&appdir, title, body).output();
        match out {
            Ok(o) if o.status.success() => return true,
            Ok(o) => log(&format!("bagholder notify: the notifier app refused: {}", String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => log(&format!("bagholder notify: the notifier app refused: {}", e)),
        }
    }
    // without the app: the system's plain notification, under Script Editor's name
    mac_plain_command(title, body)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn mac_open_command(appdir: &Path, title: &str, body: &str) -> Command {
    let mut c = Command::new("open");
    c.args(["-n", "-W", "--env", &format!("BAGHOLDER_TITLE={}", title), "--env", &format!("BAGHOLDER_BODY={}", body)]).arg(appdir);
    c
}

fn mac_plain_command(title: &str, body: &str) -> Command {
    let mut c = Command::new("osascript");
    c.args(["-e", "display notification (system attribute \"BAGHOLDER_BODY\") with title (system attribute \"BAGHOLDER_TITLE\")"])
        .env("BAGHOLDER_TITLE", title)
        .env("BAGHOLDER_BODY", body);
    c
}

// --- Windows: a toast under an app id registered as Bagholder --------------------

const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml('<toast activationType="protocol" launch="__URL__"><visual><binding template="ToastGeneric"><text></text><text></text></binding></visual></toast>')
$t = $xml.GetElementsByTagName('text')
$t.Item(0).AppendChild($xml.CreateTextNode($env:BAGHOLDER_TITLE)) | Out-Null
$t.Item(1).AppendChild($xml.CreateTextNode($env:BAGHOLDER_BODY)) | Out-Null
$toast = New-Object Windows.UI.Notifications.ToastNotification $xml
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('__APP__').Show($toast)
"#;

pub fn windows_script(app: &App) -> String {
    WINDOWS_SCRIPT.replace("__URL__", &url(app)).replace("__APP__", APP_NAME)
}

fn windows_deliver(app: &App, title: &str, body: &str) -> bool {
    // the app id a toast is shown under, with Bagholder's name and icon, in the
    // person's own registry hive
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let key = format!("HKCU\\Software\\Classes\\AppUserModelId\\{}", APP_NAME);
        run("reg", &["add", &key, "/v", "DisplayName", "/t", "REG_SZ", "/d", APP_NAME, "/f"]);
        if icon(app).exists() {
            run("reg", &["add", &key, "/v", "IconUri", "/t", "REG_SZ", "/d", &icon(app).to_string_lossy(), "/f"]);
        }
    });
    let shell = if which("powershell.exe").is_some() || which("powershell").is_some() { "powershell" } else { "pwsh" };
    windows_command(shell, title, body, &windows_script(app))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn windows_command(shell: &str, title: &str, body: &str, script: &str) -> Command {
    let mut c = Command::new(shell);
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", script])
        .env("BAGHOLDER_TITLE", title)
        .env("BAGHOLDER_BODY", body);
    c
}

// --- Linux ------------------------------------------------------------------------

fn linux_deliver(app: &App, title: &str, body: &str) -> bool {
    linux_command(app, title, body).output().map(|o| o.status.success()).unwrap_or(false)
}

fn linux_command(app: &App, title: &str, body: &str) -> Command {
    let mut cmd = Command::new("notify-send");
    cmd.arg(format!("--app-name={}", APP_NAME));
    if icon(app).exists() {
        cmd.arg(format!("--icon={}", icon(app).to_string_lossy()));
    }
    cmd.args([title, body]);
    cmd
}

// --- the page's channel -------------------------------------------------------------

/// Every row made after `after` (or after the stream opens),
/// each once, with a comment between them every heartbeat. `write` answers
/// false when the reader has gone.
pub fn stream<W: FnMut(&str) -> bool>(app: &Arc<App>, after: Option<i64>, mut write: W) {
    let mut last = match after {
        Some(a) => a,
        None => app.open().ok().and_then(|c| bagholder_store::feeds::latest_notification_id(&c).ok()).unwrap_or(0),
    };
    if !write(": bagholder\n\n") {
        return;
    }
    while !app.stopping() {
        let rows = app.open().ok().and_then(|c| bagholder_store::feeds::list_notifications(&c, last, "", false, 50, false).ok()).unwrap_or_default();
        if rows.is_empty() {
            let (m, c) = &app.notify.bell;
            let g = m.lock().unwrap();
            let _ = c.wait_timeout(g, heartbeat());
            if app.stopping() {
                return;
            }
            let rows = app.open().ok().and_then(|c| bagholder_store::feeds::list_notifications(&c, last, "", false, 50, false).ok()).unwrap_or_default();
            if rows.is_empty() {
                if !write(": ping\n\n") {
                    return;
                }
                continue;
            }
            for r in rows {
                let id = r.id;
                last = last.max(id);
                if !write(&format!("id: {}\ndata: {}\n\n", id, bagholder_store::tables::json_text(&serde_json::to_value(&r).unwrap()))) {
                    return;
                }
            }
            continue;
        }
        for r in rows {
            let id = r.id;
            last = last.max(id);
            if !write(&format!("id: {}\ndata: {}\n\n", id, bagholder_store::tables::json_text(&serde_json::to_value(&r).unwrap()))) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::app::f;
    use serde_json::json;
    use bagholder_store::feeds as st;
    use std::ffi::OsStr;
    use std::sync::MutexGuard;

    static SERIAL: Mutex<()> = Mutex::new(());

    /// An app of these tests' own, on a home no other test writes to; each test
    /// starts its store empty, with every kind off, nothing posted on this machine.
    fn setup() -> (MutexGuard<'static, ()>, Arc<App>, bagholder_store::pool::Pooled<'static>) {
        static APP: std::sync::OnceLock<Arc<App>> = std::sync::OnceLock::new();
        let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(MODE_ENV, "browser");
        let app: &'static Arc<App> = APP.get_or_init(|| {
            let home = std::env::temp_dir().join(format!("bagholder-notify-tests-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            App::new(home, PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into())
        });
        let conn = app.open().unwrap();
        let app = app.clone();
        bagholder_store::relabel::ensure(&conn).unwrap();
        conn.execute("DELETE FROM notifications", []).unwrap();
        conn.execute("DELETE FROM meta WHERE key = ? OR key LIKE 'notify_seen:%'", [SETTINGS_KEY]).unwrap();
        // the streams' own memory too, so one test's releases are never another's history
        conn.execute_batch("DELETE FROM told; DELETE FROM news; DELETE FROM distributions; DELETE FROM watchlist").unwrap();
        *test_hooks::CHANNEL.lock().unwrap() = Some(String::new());
        *test_hooks::DELIVERED.lock().unwrap() = None;
        *test_hooks::HEARTBEAT_MS.lock().unwrap() = None;
        {
            let mut s = app.state.lock().unwrap();
            s.connected = false;
            s.error.clear();
            s.sync_fails = 0;
            s.sync_first_fail.clear();
        }
        (g, app, conn)
    }

    fn set(conn: &Connection, v: Value) -> NotifySettings {
        set_settings(conn, &serde_json::from_value(v).unwrap()).unwrap()
    }

    fn off() -> NotifySettings {
        NotifySettings::default()
    }

    /// A notification row, as the tests read it: its own JSON.
    fn v(r: &bagholder_store::feeds::Notification) -> Value {
        serde_json::to_value(r).unwrap()
    }

    fn list(conn: &Connection) -> Vec<Value> {
        st::list_notifications(conn, 0, "", false, 1000, false).unwrap().iter().map(v).collect()
    }

    fn id(x: &Value) -> i64 {
        x["id"].as_i64().unwrap()
    }

    fn idn(r: &bagholder_store::feeds::Notification) -> i64 {
        r.id
    }

    fn argv(c: &Command) -> Vec<String> {
        std::iter::once(c.get_program()).chain(c.get_args()).map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn env_of(c: &Command, k: &str) -> String {
        c.get_envs().find(|(n, _)| *n == OsStr::new(k)).and_then(|(_, v)| v).map(|v| v.to_string_lossy().into_owned()).unwrap_or_default()
    }

    #[test]
    fn test_every_kind_is_off_until_turned_on_and_the_settings_round_trip() {
        let (_g, _app, conn) = setup();
        assert_eq!(settings(&conn).unwrap(), off());
        let out = set(&conn, json!({"fills": true, "bogus": true, "updates": "yes"}));
        let mut want = off();
        want.fills = true;
        assert_eq!(out, want, "unknown keys and non-booleans are ignored");
        assert_eq!(settings(&conn).unwrap(), out);
        assert_eq!(status(&conn).unwrap(), NotifyStatus { settings: out, native: String::new(), unread: 0 }, "the kinds, the channel and the unread count");
        assert_eq!(channel_for("", "macos", &|_| true, false), "mac");
        assert_eq!(channel_for("", "windows", &|n| n == "powershell", false), "windows");
        assert_eq!(channel_for("", "linux", &|n| n == "notify-send", true), "linux");
        assert_eq!(channel_for("", "linux", &|_| false, false), "", "no desktop: the page is the channel");
        assert_eq!(channel_for("browser", "macos", &|_| true, true), "", "told to stand aside");
    }

    #[test]
    fn test_a_kind_that_is_off_is_not_told_and_a_key_is_told_once() {
        let (_g, app, conn) = setup();
        assert!(emit(&app, &conn, "fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75", None).is_none());
        set(&conn, json!({"fills": true}));
        let row = v(&emit(&app, &conn, "fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75", None).unwrap());
        assert_eq!((f(&row, "kind"), f(&row, "title"), f(&row, "body"), f(&row, "seenAt"), f(&row, "readAt")), ("fills".into(), "Order filled · QNC".into(), "Bought 5 at 1.75".into(), String::new(), String::new()));
        assert!(emit(&app, &conn, "fills", "order:1:filled", "Order filled · QNC", "again", None).is_none(), "the same event is never told twice");
        assert!(emit(&app, &conn, "bogus", "x", "t", "b", None).is_none(), "an unknown kind is nothing");
        assert!(emit(&app, &conn, "disclosures", "f1", "t", "b", None).is_none(), "no set of tickers chosen");
        set(&conn, json!({"disclosuresWatched": true}));
        assert_eq!(disclosure_scopes(&conn), vec!["watched".to_string()]);
        assert!(emit(&app, &conn, "disclosures", "f1", "t", "b", None).is_some(), "any set on: the kind is told");
        assert!(idn(&test_notification(&app, &conn).unwrap()) > id(&row), "the test goes out whatever the kinds say");
        assert_eq!(list(&conn).len(), 3);
    }

    #[test]
    fn test_seen_rows_are_not_listed_again_and_the_oldest_are_pruned() {
        let (_g, app, conn) = setup();
        set(&conn, json!({"fills": true}));
        let ids: Vec<i64> = (0..3).map(|i| idn(&emit(&app, &conn, "fills", &format!("k{}", i), "t", "b", None).unwrap())).collect();
        assert_eq!(st::mark_notifications_seen(&conn, &[ids[0]], &now_iso()).unwrap(), 1);
        assert_eq!(st::list_notifications(&conn, 0, "", true, 1000, false).unwrap().iter().map(idn).collect::<Vec<_>>(), ids[1..]);
        assert_eq!(st::list_notifications(&conn, ids[1], "", false, 1000, false).unwrap().iter().map(idn).collect::<Vec<_>>(), ids[2..]);
        // NOTIFICATIONS_KEPT is a constant of the store crate and cannot be lowered here: four rows stay
        emit(&app, &conn, "fills", "k9", "t", "b", None);
        let newest: Vec<String> = st::list_notifications(&conn, 0, "", false, 1000, true).unwrap().iter().map(|r| r.key.clone()).collect();
        assert_eq!(newest, ["k9", "k2", "k1", "k0"], "the history reads newest first");
        assert_eq!(st::unread_notifications(&conn).unwrap(), 4);
        assert_eq!(st::mark_notifications_read(&conn, Some(&[ids[2]]), &now_iso()).unwrap(), 1);
        assert_eq!(st::unread_notifications(&conn).unwrap(), 3);
        assert_eq!(st::mark_notifications_read(&conn, None, &now_iso()).unwrap(), 3, "no ids: every unread one");
        assert_eq!((st::unread_notifications(&conn).unwrap(), st::mark_notifications_read(&conn, None, &now_iso()).unwrap()), (0, 0));
        assert!(list(&conn).iter().all(|r| !f(r, "readAt").is_empty()));
        assert_eq!(st::clear_notifications(&conn).unwrap(), 4);
        assert_eq!((list(&conn), st::latest_notification_id(&conn).unwrap()), (vec![], 0));
    }

    /// Runs the stream on a thread; its chunks arrive on the channel, and it
    /// ends once `stop_after` data chunks have gone out.
    fn open_stream(app: &Arc<App>, after: Option<i64>, stop_after: usize) -> std::sync::mpsc::Receiver<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let app = app.clone();
        std::thread::spawn(move || {
            let mut rows = 0;
            stream(&app, after, |chunk| {
                if chunk.starts_with("id: ") {
                    rows += 1;
                }
                let _ = tx.send(chunk.to_string());
                rows < stop_after
            });
        });
        rx
    }

    fn next(rx: &std::sync::mpsc::Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(5)).expect("the stream said something")
    }

    fn next_row(rx: &std::sync::mpsc::Receiver<String>) -> String {
        loop {
            let c = next(rx);
            if c != ": ping\n\n" {
                return c;
            }
        }
    }

    #[test]
    fn test_the_stream_sends_every_row_after_the_id_the_page_brings_with_pings_between() {
        let (_g, app, conn) = setup();
        *test_hooks::HEARTBEAT_MS.lock().unwrap() = Some(50);
        set(&conn, json!({"fills": true}));
        let old = idn(&emit(&app, &conn, "fills", "old", "Old", "b", None).unwrap());
        st::mark_notifications_seen(&conn, &[old], &now_iso()).unwrap();
        let first = idn(&emit(&app, &conn, "fills", "first", "First", "b", None).unwrap());
        let rx = open_stream(&app, Some(old), 2);
        let (hello, row1, ping) = (next(&rx), next(&rx), next(&rx));
        let second = idn(&emit(&app, &conn, "fills", "second", "Second", "b", None).unwrap());
        let row2 = next_row(&rx);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the reader gone: the stream ends");
        assert_eq!(hello, ": bagholder\n\n");
        assert!(row1.starts_with(&format!("id: {}\ndata: ", first)) && row1.contains("\"title\": \"First\""), "{}", row1);
        assert_eq!(ping, ": ping\n\n", "nothing new by the heartbeat: a comment keeps the connection");
        assert!(row2.starts_with(&format!("id: {}\ndata: ", second)), "{}", row2);

        *test_hooks::CHANNEL.lock().unwrap() = Some("mac".into());
        *test_hooks::DELIVERED.lock().unwrap() = Some(vec![]);
        let rx = open_stream(&app, None, 1);
        assert_eq!((next(&rx), next(&rx)), (": bagholder\n\n".to_string(), ": ping\n\n".to_string()), "no id: only what is made after the stream opens");
        let third = idn(&emit(&app, &conn, "fills", "third", "Third", "b", None).unwrap());
        let chunk = next_row(&rx);
        assert!(chunk.starts_with(&format!("id: {}\n", third)) && chunk.contains("\"seenAt\": \"20"), "a row the server posts itself still reaches the history, already seen");
    }

    fn order() -> Value {
        json!({"id": "o1", "symbol": "QNC", "account": "🚀 Trading", "side": "BUY", "type": "LIMIT", "quantity": 5.0, "limitPrice": 1.75, "status": "pending", "role": "entry", "source": "bagholder", "securityId": "sec-1"})
    }

    fn with(base: &Value, patch: Value) -> Value {
        let mut o = base.as_object().unwrap().clone();
        o.extend(patch.as_object().unwrap().clone());
        Value::Object(o)
    }

    fn t4(a: &str, b: &str, c: &str, d: &str) -> Option<(String, String, String, String)> {
        Some((a.into(), b.into(), c.into(), d.into()))
    }

    #[test]
    fn test_a_fill_read_back_is_told_by_its_role() {
        // the rows as the tests spell them, read as what they are
        fn order_notice(o: &Value, upd: &Value) -> Option<(String, String, String, String)> {
            crate::orders::order_notice(&serde_json::from_value(o.clone()).unwrap(), &serde_json::from_value(upd.clone()).unwrap())
        }
        let o = order();
        assert_eq!(order_notice(&o, &json!({"status": "filled", "filledQty": 5.0, "avgFill": 1.75})), t4("fills", "order:o1:filled", "Order filled · QNC", "Bought 5 at 1.75 · 🚀 Trading"));
        let stop = with(&o, json!({"id": "o2", "side": "SELL", "type": "STOP", "stopPrice": 1.66, "role": "stop"}));
        assert_eq!(order_notice(&stop, &json!({"status": "filled", "filledQty": 5.0, "avgFill": 1.6374})), t4("fills", "order:o2:filled", "Stopped out · QNC", "Sold 5 at 1.64 · 🚀 Trading"));
        let target = with(&o, json!({"id": "o3", "side": "SELL", "limitPrice": 1.93, "role": "target"}));
        assert_eq!(order_notice(&target, &json!({"status": "filled", "filledQty": 5.0, "avgFill": 1.93})).unwrap().2, "Target hit · QNC");
        assert_eq!(order_notice(&with(&o, json!({"status": "filled"})), &json!({"status": "filled", "filledQty": 5.0, "avgFill": 1.75})), None, "read back filled again: nothing new");
        assert_eq!(order_notice(&with(&o, json!({"quantity": 100.0})), &json!({"status": "pending", "filledQty": 40.0, "avgFill": 64.5})), t4("fills", "order:o1:partial:40", "Partly filled · QNC", "40 of 100 at 64.50 · 🚀 Trading"));
        assert_eq!(order_notice(&with(&o, json!({"quantity": 100.0, "filledQty": 40.0})), &json!({"status": "pending", "filledQty": 40.0, "avgFill": 64.5})), None, "the same partial fill again");
    }

    #[test]
    fn test_problems_are_told_but_not_the_persons_own_cancel_nor_a_legs_expiry() {
        // the rows as the tests spell them, read as what they are
        fn order_notice(o: &Value, upd: &Value) -> Option<(String, String, String, String)> {
            crate::orders::order_notice(&serde_json::from_value(o.clone()).unwrap(), &serde_json::from_value(upd.clone()).unwrap())
        }
        let o = order();
        assert_eq!(order_notice(&o, &json!({"status": "rejected", "error": "Limit price has too many decimal places. Max allowed: 2"})),
            t4("problems", "order:o1:rejected", "Order rejected · QNC", "Buy 5 at 1.75 limit · Limit price has too many decimal places. Max allowed: 2"));
        assert_eq!(order_notice(&o, &json!({"status": "failed"})).unwrap().2, "Order not sent · QNC");
        assert_eq!(order_notice(&o, &json!({"status": "expired"})), t4("problems", "order:o1:expired", "Order expired · QNC", "Buy 5 at 1.75 limit · 🚀 Trading"));
        assert_eq!(order_notice(&o, &json!({"status": "cancelled"})), t4("problems", "order:o1:cancelled", "Order cancelled · QNC", "Buy 5 at 1.75 limit · 🚀 Trading"));
        assert_eq!(order_notice(&with(&o, json!({"status": "cancelling"})), &json!({"status": "cancelled"})), None, "a cancel asked for here is not told");
        let stop = with(&o, json!({"id": "o2", "side": "SELL", "type": "STOP", "stopPrice": 1.66, "role": "stop"}));
        assert_eq!(order_notice(&stop, &json!({"status": "expired"})), None, "a leg's expiry is the engine's to place again");
        assert_eq!(order_notice(&stop, &json!({"status": "cancelled"})), None);
        assert_eq!(order_notice(&stop, &json!({"status": "rejected", "error": "no shares"})).unwrap().3, "Sell 5 stop 1.66 · no shares");
        // `_order_words` and `_price_words` are private to orders.rs: read through a rejection's body
        let words = |ord: Value| order_notice(&ord, &json!({"status": "rejected", "error": "e"})).unwrap().3;
        assert_eq!(words(with(&o, json!({"type": "MARKET"}))), "Buy 5 at market · e");
        assert_eq!(words(with(&o, json!({"type": "STOP_LIMIT", "stopPrice": 1.6, "limitPrice": 1.55, "side": "SELL", "quantity": 2.5}))), "Sell 2.5 stop 1.60 · limit 1.55 · e");
        let prices: Vec<String> = [json!(1.6374), json!(0.625), json!(0.54), json!(12), Value::Null]
            .into_iter()
            .map(|p| words(with(&o, json!({"limitPrice": p}))).trim_start_matches("Buy 5 at ").trim_end_matches(" limit · e").to_string())
            .collect();
        assert_eq!(prices, ["1.64", "0.625", "0.54", "12.00", "—"]);
    }

    #[test]
    fn test_the_session_expiring_is_told_on_the_transition_and_a_failing_sync_on_the_third_time() {
        let (_g, app, conn) = setup();
        set(&conn, json!({"connection": true}));
        crate::session::note_session_expired(&app);
        assert_eq!(list(&conn), Vec::<Value>::new(), "not connected: nothing expired");
        app.state.lock().unwrap().connected = true;
        crate::session::note_session_expired(&app);
        crate::session::note_session_expired(&app);
        assert!(!app.state.lock().unwrap().connected);
        for _ in 0..4 {
            crate::session::note_sync_failed(&app, "Wealthsimple did not answer");
        }
        let got: Vec<(String, String)> = list(&conn).iter().map(|r| (f(r, "title"), f(r, "body"))).collect();
        assert_eq!(got, [("Sign in needed".to_string(), "The Wealthsimple session expired. Connect again from the menu.".to_string()), ("Sync failing".to_string(), "Wealthsimple did not answer".to_string())]);
    }

    #[test]
    fn test_posted_by_the_server_a_row_is_stored_seen_and_handed_to_the_system() {
        let (_g, app, conn) = setup();
        set(&conn, json!({"fills": true}));
        *test_hooks::CHANNEL.lock().unwrap() = Some("mac".into());
        *test_hooks::DELIVERED.lock().unwrap() = Some(vec![]);
        let row = emit(&app, &conn, "fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75", None).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && test_hooks::DELIVERED.lock().unwrap().as_ref().unwrap().is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(row.seen_at, "", "the server shows it: no page shows it too");
        assert_eq!(test_hooks::DELIVERED.lock().unwrap().clone().unwrap(), vec![("mac".to_string(), "Order filled · QNC".to_string(), "Bought 5 at 1.75".to_string())]);
        assert!(st::list_notifications(&conn, 0, "", true, 1000, false).unwrap().is_empty(), "nothing left for a page");
    }

    #[test]
    fn test_each_system_is_asked_in_its_own_words() {
        let (_g, app, _conn) = setup();
        *app.port.lock().unwrap() = 8799;
        let c = mac_open_command(Path::new("/x/Bagholder.app"), "Stopped out · QNC", "Sold 5 at 1.64");
        assert_eq!(argv(&c), ["open", "-n", "-W", "--env", "BAGHOLDER_TITLE=Stopped out · QNC", "--env", "BAGHOLDER_BODY=Sold 5 at 1.64", "/x/Bagholder.app"]);
        assert!(mac_script(&app).contains("open location \"http://127.0.0.1:8799/\""));
        let c = mac_plain_command("T", "B");
        assert_eq!((argv(&c)[0].as_str(), env_of(&c, "BAGHOLDER_TITLE").as_str(), env_of(&c, "BAGHOLDER_BODY").as_str()), ("osascript", "T", "B"), "without the applet, the system's plain notification");
        let c = windows_command("powershell.exe", "T", "B", &windows_script(&app));
        let a = argv(&c);
        assert_eq!((a[0].as_str(), &a[1..7], env_of(&c, "BAGHOLDER_TITLE").as_str()), ("powershell.exe", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden"].map(String::from)[..], "T"));
        assert!(a.last().unwrap().contains("CreateToastNotifier('Bagholder')"));
        assert!(a.last().unwrap().contains("launch=\"http://127.0.0.1:8799/\""), "a click on the toast opens the app");
        let a = argv(&linux_command(&app, "T", "B"));
        assert_eq!([&a[..2], &a[a.len() - 2..]].concat(), ["notify-send", "--app-name=Bagholder", "T", "B"]);
        assert!(a[2].starts_with("--icon="));
        assert!(!deliver(&app, "", "T", "B"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_the_mac_applet_is_built_once_under_bagholders_name_and_icon() {
        if !Path::new("/usr/bin/osacompile").exists() {
            return; // the applet is built with macOS's own tools
        }
        let (_g, app, _conn) = setup();
        *app.port.lock().unwrap() = 8799;
        let appdir = mac_app(&app).expect("built");
        assert_eq!(appdir, app.home.join("Bagholder.app"));
        let plist = Command::new("plutil").args(["-p"]).arg(appdir.join("Contents/Info.plist")).output().unwrap();
        let plist = String::from_utf8_lossy(&plist.stdout);
        assert!(plist.contains("\"CFBundleName\" => \"Bagholder\""), "{}", plist);
        assert!(plist.contains("\"CFBundleIdentifier\" => \"com.bagholder.notifier\""));
        assert!(!plist.contains("CFBundleIconName"), "the app's own icon file");
        assert!(std::fs::metadata(appdir.join("Contents/Resources/applet.icns")).unwrap().len() > 10000, "the icon built from the favicon");
        assert!(!appdir.join("Contents/Resources/Assets.car").exists());
        assert!(appdir.join("Contents/Resources/Scripts").exists() && mac_script(&app).contains("open location \"http://127.0.0.1:8799/\""));
        let stamp = std::fs::read_to_string(appdir.join("Contents/Resources/bagholder.stamp")).unwrap();
        let mtime = std::fs::metadata(appdir.join("Contents/Resources/bagholder.stamp")).unwrap().modified().unwrap();
        assert_eq!(mac_app(&app), Some(appdir.clone()));
        assert_eq!(std::fs::metadata(appdir.join("Contents/Resources/bagholder.stamp")).unwrap().modified().unwrap(), mtime, "already built: not built again");
        *app.port.lock().unwrap() = 8800;
        assert_ne!(mac_stamp(&app), stamp, "a new address means a new applet");
    }

    #[test]
    fn test_a_disclosure_is_told_by_the_documents_own_title_and_the_form_code_stands_in() {
        use crate::feeds::filings_notice;
        let (_g, app, _c) = setup();
        let rows = [filing_row("sec:1", "144", "Proposed sale of 40,000 shares by an officer")];
        assert_eq!(filings_notice(&app, "NBIS", &rows), ("New disclosure · NBIS".to_string(), "Proposed sale of 40,000 shares by an officer · SEC EDGAR".to_string()));
        // nothing could be read from it: the form stands in, in words where the app knows the form,
        // since "6-K" alone names the paperwork and not what happened. A row carrying an id would
        // be read over the network for a title first, which is not exercised here.
        assert_eq!(filings_notice(&app, "NBIS", &[filing_row("", "6-K", "")]).1, "Foreign issuer report (6-K) · SEC EDGAR");
        assert_eq!(filings_notice(&app, "NBIS", &[filing_row("", "144", "")]).1, "Notice of proposed sale (144) · SEC EDGAR");
        assert_eq!(filings_notice(&app, "NBIS", &[filing_row("", "40-F", "")]).1, "40-F · SEC EDGAR",
                   "a form the app has no words for keeps its code");
        let many: Vec<bagholder_store::feeds::Filing> = (0..4).map(|i| filing_row(&format!("sec:{}", i), "4", &format!("Insider report {}", i))).collect();
        assert_eq!(filings_notice(&app, "NBIS", &many), ("4 new disclosures · NBIS".to_string(), "Insider report 0, Insider report 1, Insider report 2 and more · SEC EDGAR".to_string()));
    }

    #[test]
    fn test_a_stream_met_for_the_first_time_shows_nothing_and_never_shows_its_past() {
        let (_g, _app, conn) = setup();
        let at = |i: &Value| f(i, "at");
        let fresh = |s: &str, items: &[Value]| -> Vec<String> { fresh_since(&conn, s, items, at, default_ident, |_| false).iter().map(|i| f(i, "id")).collect() };
        let held = vec![json!({"id": "a", "at": "2026-05-01"}), json!({"id": "b", "at": "2026-06-01"})];
        assert!(fresh("s1", &held).is_empty(), "met for the first time: nothing");
        assert_eq!(bagholder_store::tables::get_meta(&conn, "notify_seen:s1", "").unwrap(), "2026-06-01|b");
        assert!(fresh("s1", &held).is_empty(), "the same again: still nothing");
        let mut later = held.clone();
        later.push(json!({"id": "c", "at": "2026-07-01"}));
        assert_eq!(fresh("s1", &later), ["c"], "what comes after the mark");
        assert!(fresh("s1", &later).is_empty(), "and never again");
        let mut both = later.clone();
        both.push(json!({"id": "d", "at": "2026-07-01"}));
        both.push(json!({"id": "old", "at": "2026-02-01"}));
        assert_eq!(fresh("s1", &both), ["d"]);
        assert!(fresh("s1", &both).is_empty());
        assert!(fresh("s2", &held).is_empty());
        for k in ["notify_seen:s1", "notify_seen:s2"] {
            assert!(!bagholder_store::tables::get_meta(&conn, k, "").unwrap().is_empty());
        }
    }

        // -----------------------------------------------------------------------
        // what a notice carries, and one event as one notification
        // -----------------------------------------------------------------------

        /// The rows the store holds, newest first.
        fn posted(app: &Arc<App>) -> Vec<Value> {
            st::list_notifications(&app.open().unwrap(), 0, "", false, 50, true).unwrap().iter().map(v).collect()
        }

        /// A release as a wire hands it over.
        fn wire_release(id: &str, head: &str, url: &str, when: &str) -> bagholder_store::feeds::NewsItem {
            bagholder_store::feeds::NewsItem {
                id: id.into(), headline: head.into(), source: "Business Wire".into(), url: url.into(), published_at: when.into(),
                summary: String::new(), kind: bagholder_store::feeds::NewsKind::Release, via: bagholder_store::feeds::Feed::of_id(id).unwrap_or(bagholder_store::feeds::Feed::Tmx),
            }
        }

        /// A filed document, every field given, as a notice reads it.
        fn filing_full(id: &str, source: bagholder_store::feeds::Regulator, date: &str, form: &str, subject: &str, summary: &str, url: &str) -> bagholder_store::feeds::Filing {
            bagholder_store::feeds::Filing {
                doc: bagholder_store::feeds::FiledDocument {
                    id: id.into(), source, category: String::new(), profile_no: String::new(), issuer: String::new(),
                    form: form.into(), title: String::new(), date: date.into(), date_text: String::new(), size: String::new(), url: url.into(),
                },
                subject: subject.into(), summary: summary.into(), enriched_at: String::new(),
                enrich_version: None, enrich_final: false, fetched_at: String::new(),
            }
        }

        /// A filed document (SEC) with only its id, form code and subject given.
        fn filing_row(id: &str, form: &str, subject: &str) -> bagholder_store::feeds::Filing {
            filing_full(id, bagholder_store::feeds::Regulator::Sec, "", form, subject, "", "")
        }

        /// A news item, every field given, as a notice reads it.
        fn news_item(id: &str, headline: &str, summary: &str, url: &str, published_at: &str) -> bagholder_store::feeds::NewsItem {
            bagholder_store::feeds::NewsItem {
                id: id.into(), headline: headline.into(), source: String::new(), url: url.into(), published_at: published_at.into(),
                summary: summary.into(), kind: bagholder_store::feeds::NewsKind::Release,
                via: bagholder_store::feeds::Feed::of_id(id).unwrap_or(bagholder_store::feeds::Feed::Tmx),
            }
        }

    // ---------------------------------------------------------------------------
    // what a notice carries
    // ---------------------------------------------------------------------------

    #[test]
    fn test_a_distribution_release_carries_the_figures_and_a_way_to_read_it() {
        // A headline that says only "Announces August 2026 Distributions" tells a holder nothing they
        // can act on: the notice carries the amount, when it goes ex and is paid, and the one it
        // replaces, from the issuer's own declared record, and it opens the release itself.
        let (_g, app, c) = setup();
        set_settings(&c, &serde_json::from_value(json!({"releasesAll": true})).unwrap()).unwrap();
        bagholder_store::market::upsert_distributions(
            &c,
            "RDDY",
            &[bagholder_store::market::DistributionRecord { ex_date: "2026-08-31".into(), pay_date: "2026-09-04".into(), amount: Some(0.15), currency: "CAD".into() },
              bagholder_store::market::DistributionRecord { ex_date: "2026-07-31".into(), pay_date: "2026-08-06".into(), amount: Some(0.20), currency: "CAD".into() }],
            "test",
        )
        .unwrap();
        bagholder_store::market::upsert_quote(&c, "RDDY", &bagholder_store::market::QuoteRecord {
            price: Some(4.87),
            dividend_amount: Some(0.15),
            dividend_frequency: "Monthly".into(),
            ex_dividend_date: "2026-08-31".into(),
            ..Default::default()
        }, "tmx", "2026-09-15T14:00:00Z").unwrap();
        let first = bagholder_store::feeds::NewsItem {
            id: "tmx:7".into(), headline: "Harvest High Income Shares ETFs Announces August 2026 Distributions".into(),
            source: "Business Wire".into(), url: "https://money.tmx.com/en/quote/RDDY/news/7".into(),
            published_at: "2026-09-15T13:00:00Z".into(), summary: String::new(), kind: bagholder_store::feeds::NewsKind::Release, via: bagholder_store::feeds::Feed::Tmx,
        };
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &[first.clone()], &["tmx:7".to_string()]);   // the first read is history
        let second = bagholder_store::feeds::NewsItem {
            id: "tmx:8".into(), headline: "Harvest ETFs Announces September 2026 Distributions".into(),
            source: "Business Wire".into(), url: "https://money.tmx.com/en/quote/RDDY/news/7".into(),
            published_at: "2026-09-15T14:00:00Z".into(), summary: String::new(), kind: bagholder_store::feeds::NewsKind::Release, via: bagholder_store::feeds::Feed::Tmx,
        };
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &[first, second], &["tmx:8".to_string()]);
        let rows = posted(&app);
        assert_eq!(rows.len(), 1);
        let body = f(&rows[0], "body");
        let lines: Vec<&str> = body.split('\n').collect();
        assert_eq!(f(&rows[0], "title"), "Press release · RDDY");
        assert_eq!(lines[0], "Harvest ETFs Announces September 2026 Distributions");
        assert_eq!(lines[1], "$0.15 a share, monthly · ex Aug 31, paid Sep 4 · was $0.20");
        assert_eq!(f(&rows[0]["extra"], "url"), "https://money.tmx.com/en/quote/RDDY/news/7");
        assert_eq!(f(&rows[0]["extra"], "symbol"), "RDDY");
        assert!(app.feeds.record_reads.load(std::sync::atomic::Ordering::SeqCst) > 0,
                "the record is read again so the notice is not a day behind the release");
    }

    #[test]
    fn test_a_release_that_announces_nothing_of_the_kind_carries_the_headline_alone() {
        let (_g, app, c) = setup();
        bagholder_store::market::upsert_distributions(&c, "QNC", &[bagholder_store::market::DistributionRecord { ex_date: "2026-08-31".into(), pay_date: "2026-09-04".into(), amount: Some(0.15), currency: "CAD".into() }], "test").unwrap();
        assert_eq!(
            crate::feeds::release_notice(&app, "QNC", &[news_item("tmx:1", "Quantum eMotion Wins Certification", "", "", "2026-09-15T13:00:00Z")]),
            ("Press release · QNC".to_string(), "Quantum eMotion Wins Certification".to_string())
        );
        assert_eq!(
            crate::feeds::release_notice(&app, "NOSUCH", &[news_item("tmx:2", "Announces Monthly Distribution", "", "", "2026-09-15T13:00:00Z")]).1,
            "Announces Monthly Distribution",
            "no record for the listing: the headline stands alone"
        );
    }

    #[test]
    fn test_a_release_with_no_figures_carries_what_the_source_said() {
        let (_g, app, _c) = setup();
        let notice = crate::feeds::release_notice(&app, "QNC", &[news_item("tmx:1", "Quantum eMotion Wins Certification",
            "The certification covers its entropy module, which NIST listed this week.", "", "2026-09-15T13:00:00Z")]);
        assert_eq!(notice.1, "Quantum eMotion Wins Certification\nThe certification covers its entropy module, which NIST listed this week.");
    }

    #[test]
    fn test_a_disclosure_notice_carries_the_sentence_the_document_yielded() {
        let (_g, app, _c) = setup();
        let rows = vec![filing_full("sedar:1", bagholder_store::feeds::Regulator::Sedar, "2026-09-08T16:22", "Other Correspondence",
                               "GAB0590 Avis Acceptation WKSI",
                               "The company announces the acceptance of its prospectus by the Autorité des marchés financiers.", "")];
        let (title, body) = crate::feeds::filings_notice(&app, "QNC", &rows);
        assert_eq!(title, "New disclosure · QNC");
        assert_eq!(
            body.split('\n').collect::<Vec<_>>(),
            vec!["GAB0590 Avis Acceptation WKSI · SEDAR+",
                 "The company announces the acceptance of its prospectus by the Autorité des marchés financiers."]
        );
        let same = vec![filing_full("sedar:1", bagholder_store::feeds::Regulator::Sedar, "2026-09-08T16:22", "Other Correspondence",
                               "GAB0590 Avis Acceptation WKSI", "GAB0590 Avis Acceptation WKSI", "")];
        assert_eq!(crate::feeds::filings_notice(&app, "QNC", &same).1, "GAB0590 Avis Acceptation WKSI · SEDAR+",
                   "a summary that only repeats the line above it is not a second line");
    }

    #[test]
    fn test_a_form_with_no_document_read_is_named_in_words() {
        // nothing could be read from it: the form stands in, in words where the app knows the form,
        // since "6-K" alone names the paperwork and not what happened
        let (_g, app, _c) = setup();
        assert_eq!(crate::feeds::filings_notice(&app, "NBIS", &[filing_row("", "6-K", "")]).1, "Foreign issuer report (6-K) · SEC EDGAR");
        assert_eq!(crate::feeds::filings_notice(&app, "NBIS", &[filing_row("", "144", "")]).1, "Notice of proposed sale (144) · SEC EDGAR");
        assert_eq!(crate::feeds::filings_notice(&app, "NBIS", &[filing_row("", "40-F", "")]).1, "40-F · SEC EDGAR",
                   "a form the app has no words for keeps its code");
    }

    #[test]
    fn test_a_notice_carries_when_the_thing_happened() {
        // A release found today can have been published weeks ago: the notice carries the item's own
        // moment, so the panel can say when it happened rather than when it was told.
        let (_g, _app, _c) = setup();
        assert_eq!(crate::feeds::notice_moment(&[news_item("tmx:1", "", "", "", "2026-08-24T11:00:00Z"),
                                          news_item("tmx:2", "", "", "", "2026-08-31T07:00:00Z")]),
                   json!({"at": "2026-08-31T07:00:00Z"}), "the newest of them");
        assert_eq!(crate::feeds::notice_moment(&[filing_full("sedar:1", bagholder_store::feeds::Regulator::Sedar, "2026-09-08T16:22", "", "", "", "")]), json!({"at": "2026-09-08T16:22"}));
        assert_eq!(crate::feeds::notice_moment(&[news_item("x", "", "", "", "")]), json!({"at": ""}));
    }

    #[test]
    fn test_a_disclosure_notice_opens_the_document_it_is_about() {
        let (_g, _app, _c) = setup();
        assert_eq!(crate::feeds::notice_link(&[filing_full("sedar:9", bagholder_store::feeds::Regulator::Sedar, "2026-09-15T09:00", "", "", "", "https://www.sedarplus.ca/x?drmKey=9")]),
                   json!({"url": "https://www.sedarplus.ca/x?drmKey=9", "doc": "sedar:9", "source": "SEDAR+"}));
        assert_eq!(crate::feeds::notice_link(&[filing_full("sec:4", bagholder_store::feeds::Regulator::Sec, "2026-09-15T09:00", "", "", "", "https://www.sec.gov/x/4.htm")]),
                   json!({"url": "https://www.sec.gov/x/4.htm", "doc": "sec:4", "source": "SEC"}), "the SEC serves its own documents");
        assert_eq!(crate::feeds::notice_link(&[news_item("tmx:1", "", "", "https://money.tmx.com/en/quote/QNC/news/1", "2026-09-15T13:00:00Z")]),
                   json!({"url": "https://money.tmx.com/en/quote/QNC/news/1"}));
        assert_eq!(crate::feeds::notice_link(&[news_item("x", "", "", "", "2026-09-15T13:00:00Z")]), json!({}), "nothing to open, nothing claimed");
    }

    #[test]
    fn test_a_document_a_regulator_refuses_is_a_page_not_a_json_error() {
        // The document route opens in a tab of its own: a refusal has to read as words.
        let (_g, app, _c) = setup();
        let page = crate::feeds::document_error_page(&app, "QNC", "sedar:drm:x", "could not open the profile's documents to download from");
        assert!(page.contains("<!doctype html>"));
        assert!(page.contains("would not serve this document just now"));
        assert!(page.contains("could not open the profile&#x27;s documents to download from"));
        assert!(page.contains("/api/filings/doc?symbol=QNC&amp;id=sedar%3Adrm%3Ax"), "the retry goes back through the app");
    }

    // ---------------------------------------------------------------------------
    // one event is one notification
    // ---------------------------------------------------------------------------


    #[test]
    fn test_one_event_is_one_notification_whatever_id_it_arrives_under() {
        // The same release reaches the app from several sources, each with its own id and its own
        // date. It is one event and is told once, and meeting it again -- a week later, under another
        // id, from a source whose results dropped it and brought it back -- tells nothing.
        let (_g, app, c) = setup();
        set_settings(&c, &serde_json::from_value(json!({"releasesAll": true})).unwrap()).unwrap();
        let head = "Harvest ETFs Announces September 2026 Distributions";
        let older = wire_release("tmx:0", "An older release", "u0", "2026-09-01T11:30:00Z");
        let first = wire_release("tmx:1", head, "u1", "2026-09-14T11:30:00Z");
        let note = |rows: &[bagholder_store::feeds::NewsItem], new: &[&str]| {
            crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", rows, &new.iter().map(|x| x.to_string()).collect::<Vec<_>>())
        };
        note(&[older.clone()], &["tmx:0"]);                       // the listing's first read: history
        note(&[older.clone(), first.clone()], &["tmx:1"]);
        assert_eq!(posted(&app).iter().map(|r| f(r, "title")).collect::<Vec<_>>(), vec!["Press release · RDDY"], "told once, when it appeared");
        // Google's copy of the same release: its own id, a week's difference in its date
        let g2 = wire_release("gnews:2", head, "u2", "2026-09-21T07:00:00Z");
        note(&[older.clone(), first.clone(), g2.clone()], &["gnews:2"]);
        // and the wire's own copy drops out of the results and comes back under a new id
        note(&[older.clone(), g2.clone()], &[]);
        note(&[older, g2, wire_release("tmx:9", head, "u1", "2026-09-14T11:30:00Z")], &["tmx:9"]);
        assert_eq!(posted(&app).len(), 1, "one event, one notification");
    }

    #[test]
    fn test_the_back_catalogue_a_first_read_brings_can_never_ring_later() {
        // A source read for the first time brings history. That history is recorded as met, so the
        // same releases returning under other ids on later passes are recognised rather than rung.
        let (_g, app, c) = setup();
        set_settings(&c, &serde_json::from_value(json!({"releasesAll": true})).unwrap()).unwrap();
        let old: Vec<bagholder_store::feeds::NewsItem> = (0..3).map(|i| wire_release(&format!("tmx:{}", i), &format!("Release number {}", i), "u", &format!("2026-08-{:02}T11:30:00Z", 10 + i))).collect();
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &old, &old.iter().map(|r| r.id.clone()).collect::<Vec<_>>());
        // every one of them comes back under another source's ids, dated later, as a search's results shift
        let again: Vec<bagholder_store::feeds::NewsItem> = (0..3).map(|i| wire_release(&format!("gnews:{}", i), &format!("Release number {}", i), "u", &format!("2026-09-{:02}T07:00:00Z", 10 + i))).collect();
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &again, &again.iter().map(|r| r.id.clone()).collect::<Vec<_>>());
        assert!(posted(&app).is_empty(), "history stays history, whatever id it returns under");
    }

    #[test]
    fn test_a_month_old_release_is_never_told_however_it_reaches_the_app() {
        // What happened in the person's own app: a listing's wire carried a release dated 24 August;
        // weeks later a second source returned its own copy, dated 31 August, which was newer than the
        // stream's mark and had an id the listing had never held -- so the bell rang for an August
        // event. The stream now records what it has met, so the second copy is recognised; and
        // something that just happened is still told, through the same mark.
        let (_g, app, c) = setup();
        set_settings(&c, &serde_json::from_value(json!({"releasesAll": true})).unwrap()).unwrap();
        let head = "Harvest ETFs Announces August 2026 Distributions";
        let a = wire_release("tmx:1", head, "u1", "2026-08-24T11:30:00Z");
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &[a.clone()], &["tmx:1".to_string()]);   // the first read: history
        let b = wire_release("gnews:2", head, "u2", "2026-08-31T07:00:00Z");
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &[a.clone(), b.clone()], &["gnews:2".to_string()]);
        assert!(posted(&app).is_empty(), "the same release under another id: history, not news");
        let fresh = wire_release("tmx:3", "Harvest ETFs Announces September 2026 Distributions", "u3", "2026-09-15T11:30:00Z");
        crate::feeds::note_wire_releases(&app, &c, "RDDY", "TSX", &[a, b, fresh], &["tmx:3".to_string()]);
        assert_eq!(posted(&app).iter().map(|r| f(r, "title")).collect::<Vec<_>>(), vec!["Press release · RDDY"]);
        assert_eq!(f(&posted(&app)[0]["extra"], "at"), "2026-09-15T11:30:00Z");
    }

    #[test]
    fn test_what_a_stream_has_met_is_kept_by_what_the_thing_is() {
        // `store.events_told` / `store.mark_told`: the stream's memory, keyed by the event.
        let (_g, _app, c) = setup();
        let events = vec!["a".to_string(), "b".to_string()];
        assert!(st::events_told(&c, "news:X@TSX", &events).unwrap().is_empty());
        assert_eq!(st::mark_told(&c, "news:X@TSX", &events, "2026-09-15T14:00:00Z").unwrap(), 2);
        assert_eq!(st::events_told(&c, "news:X@TSX", &events).unwrap().len(), 2);
        assert!(st::events_told(&c, "news:Y@TSX", &events).unwrap().is_empty(), "each stream keeps its own");
        assert!(st::events_told(&c, "news:X@TSX", &[String::new()]).unwrap().is_empty(), "nothing is not an event");
    }

}
