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
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use crate::app::{app, log, now_iso};

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

fn url() -> String {
    format!("http://127.0.0.1:{}/", *app().port.lock().unwrap())
}

fn icon() -> PathBuf {
    app().root.join("favicon.png")
}

/// `notify.settings`: every kind off until it is turned on from the menu.
pub fn settings(conn: &Connection) -> Result<Map<String, Value>> {
    let raw = bagholder_store::tables::get_meta(conn, SETTINGS_KEY, "")?;
    let parsed: Value = serde_json::from_str(if raw.is_empty() { "{}" } else { &raw }).unwrap_or_else(|_| json!({}));
    let m = parsed.as_object().cloned().unwrap_or_default();
    Ok(setting_keys().into_iter().map(|k| (k.to_string(), json!(crate::app::truthy(m.get(k))))).collect())
}

fn scopes(conn: &Connection, keys: &[&str], prefix: &str) -> Vec<String> {
    let on = settings(conn).unwrap_or_default();
    keys.iter().filter(|k| on.get(**k).and_then(|v| v.as_bool()).unwrap_or(false)).map(|k| k[prefix.len()..].to_lowercase()).collect()
}

/// `notify.disclosure_scopes`.
pub fn disclosure_scopes(conn: &Connection) -> Vec<String> {
    scopes(conn, &DISCLOSURE_SCOPES, "disclosures")
}

/// `notify.release_scopes`.
pub fn release_scopes(conn: &Connection) -> Vec<String> {
    scopes(conn, &RELEASE_SCOPES, "releases")
}

/// `notify.kind_on`.
pub fn kind_on(conn: &Connection, kind: &str) -> bool {
    match kind {
        "disclosures" => !disclosure_scopes(conn).is_empty(),
        "releases" => !release_scopes(conn).is_empty(),
        k => settings(conn).ok().and_then(|m| m.get(k).and_then(|v| v.as_bool())).unwrap_or(false),
    }
}

/// `notify.set_settings`: unknown keys and non-booleans are ignored.
pub fn set_settings(conn: &Connection, patch: &Value) -> Result<Map<String, Value>> {
    let mut cur = settings(conn)?;
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            if setting_keys().contains(&k.as_str()) {
                if let Value::Bool(b) = v {
                    cur.insert(k.clone(), json!(b));
                }
            }
        }
    }
    bagholder_store::tables::set_meta(conn, SETTINGS_KEY, &bagholder_store::tables::py_json(&Value::Object(cur.clone())))?;
    Ok(cur)
}

/// `notify.status`: the kinds, the native channel, and the unread count.
pub fn status(conn: &Connection) -> Result<Value> {
    let mut out = settings(conn)?;
    out.insert("native".into(), json!(native_channel()));
    out.insert("unread".into(), json!(bagholder_store::feeds::unread_notifications(conn)?));
    Ok(Value::Object(out))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// `notify.native_channel`.
pub fn native_channel() -> String {
    let mode = std::env::var(MODE_ENV).unwrap_or_default().trim().to_lowercase();
    if ["browser", "off", "0", "none"].contains(&mode.as_str()) {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        return if which("osascript").is_some() { "mac".into() } else { String::new() };
    }
    if cfg!(target_os = "windows") {
        return if which("powershell.exe").is_some() || which("pwsh.exe").is_some() || which("powershell").is_some() { "windows".into() } else { String::new() };
    }
    let display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false) || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
    if which("notify-send").is_some() && display {
        return "linux".into();
    }
    String::new()
}

/// `notify.fresh_since`: what a stream has that it has not shown before, and
/// nothing it held when it was first met.
///
/// Each stream carries a mark: the newest moment it has shown and the items it
/// showed at that moment. A stream met for the first time shows nothing and
/// its mark is set from it; after that it shows what is newer than the mark,
/// and what shares the mark's moment without having been shown.
pub fn fresh_since<A, I, S>(conn: &Connection, stream: &str, items: &[Value], at: A, ident: I, seen: S) -> Vec<Value>
where
    A: Fn(&Value) -> String,
    I: Fn(&Value) -> String,
    S: Fn(&Value) -> bool,
{
    let key = format!("{}{}", WATERMARK, stream);
    let raw = bagholder_store::tables::get_meta(conn, &key, "").unwrap_or_default();
    let (mark, shown_raw) = match raw.split_once('|') { Some((a, b)) => (a.to_string(), b.to_string()), None => (raw.clone(), String::new()) };
    let shown: Vec<String> = shown_raw.split(',').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect();
    let stamped: Vec<(String, String, &Value)> = items.iter().map(|i| (at(i), ident(i), i)).collect();
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
    let out: Vec<Value> = stamped
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
pub fn default_ident(i: &Value) -> String {
    match i {
        Value::Object(_) => crate::app::f(i, "id"),
        other => crate::app::s(Some(other)),
    }
}

fn wake() -> &'static (Mutex<u64>, Condvar) {
    static W: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
    W.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// `notify.emit`: one notification, if its kind is on and this key has not
/// been told before.
pub fn emit(conn: &Connection, kind: &str, key: &str, title: &str, body: &str, extra: Option<Value>) -> Option<Value> {
    if !KINDS.contains(&kind) || !kind_on(conn, kind) {
        return None;
    }
    post(conn, kind, key, title, body, extra)
}

/// `notify.test_notification`.
pub fn test_notification(conn: &Connection) -> Option<Value> {
    let stamp = {
        let now = crate::app::now_unix();
        let micros = ((now.fract()) * 1_000_000.0) as i64;
        format!("{}{:06}", now_iso().replace(['-', ':', 'T', 'Z'], ""), micros)
    };
    post(conn, "test", &format!("test:{}", stamp), APP_NAME, "Notifications reach you here.", None)
}

fn post(conn: &Connection, kind: &str, key: &str, title: &str, body: &str, extra: Option<Value>) -> Option<Value> {
    let channel = native_channel();
    // posted from here, the row is the server's own to show: seen from the start
    let row = bagholder_store::feeds::add_notification(conn, kind, key, title, body, extra.as_ref(), !channel.is_empty(), &now_iso()).ok()??;
    if !channel.is_empty() {
        enqueue(crate::app::f(&row, "title"), crate::app::f(&row, "body"), channel);
    }
    let (m, c) = wake();
    *m.lock().unwrap() += 1;
    c.notify_all();
    Some(row)
}

fn enqueue(title: String, body: String, chan: String) {
    static WORKER: OnceLock<Mutex<Sender<(String, String, String)>>> = OnceLock::new();
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<(String, String, String)>();
        crate::app::spawn("bagholder-notify", move || {
            for (title, body, ch) in rx {
                if !deliver(&ch, &title, &body) {
                    log(&format!("bagholder notify: {} not shown ({})", title, ch));
                }
            }
        });
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap().send((title, body, chan));
}

/// `notify.deliver`: post one notification through the system.
pub fn deliver(channel: &str, title: &str, body: &str) -> bool {
    match channel {
        "mac" => mac_deliver(title, body),
        "windows" => windows_deliver(title, body),
        "linux" => linux_deliver(title, body),
        _ => false,
    }
}

// --- macOS: an applet of Bagholder's own ---------------------------------------

fn mac_script() -> String {
    format!(
        "on run\n\tset t to system attribute \"BAGHOLDER_TITLE\"\n\tif t is \"\" then\n\t\topen location \"{}\"\n\telse\n\t\tdisplay notification (system attribute \"BAGHOLDER_BODY\") with title t\n\tend if\nend run\n",
        url()
    )
}

pub fn mac_app_path() -> PathBuf {
    app().home.join(format!("{}.app", APP_NAME))
}

fn mac_stamp() -> String {
    let mut h = openssl::sha::Sha1::new();
    h.update(mac_script().as_bytes());
    if let Ok(b) = std::fs::read(icon()) {
        h.update(&b);
    }
    h.finish().iter().map(|b| format!("{:02x}", b)).collect()
}

fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// `notify.mac_app`: the applet, built once and again whenever its script, the
/// app's address or the icon changes.
pub fn mac_app() -> Option<PathBuf> {
    let appdir = mac_app_path();
    let stamp_file = appdir.join("Contents/Resources/bagholder.stamp");
    let want = mac_stamp();
    if appdir.join("Contents/MacOS/applet").exists() && std::fs::read_to_string(&stamp_file).ok().as_deref() == Some(want.as_str()) {
        return Some(appdir);
    }
    match mac_build(&appdir, &want) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("bagholder notify: the notifier app could not be built: {}", e));
            None
        }
    }
}

fn mac_build(appdir: &Path, stamp: &str) -> std::io::Result<Option<PathBuf>> {
    if which("osacompile").is_none() {
        return Ok(None);
    }
    let work = std::env::temp_dir().join(format!("bagholder-notifier-{}", crate::app::uuid4()));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> std::io::Result<Option<PathBuf>> {
        let script = work.join("notifier.applescript");
        std::fs::write(&script, mac_script())?;
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
        if let Some(icns) = mac_icon(&work) {
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

fn mac_icon(work: &Path) -> Option<PathBuf> {
    let src = icon();
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

fn mac_deliver(title: &str, body: &str) -> bool {
    if let Some(appdir) = mac_app() {
        let out = Command::new("open")
            .args(["-n", "-W", "--env", &format!("BAGHOLDER_TITLE={}", title), "--env", &format!("BAGHOLDER_BODY={}", body)])
            .arg(&appdir)
            .output();
        match out {
            Ok(o) if o.status.success() => return true,
            Ok(o) => log(&format!("bagholder notify: the notifier app refused: {}", String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => log(&format!("bagholder notify: the notifier app refused: {}", e)),
        }
    }
    // without the app: the system's plain notification, under Script Editor's name
    Command::new("osascript")
        .args(["-e", "display notification (system attribute \"BAGHOLDER_BODY\") with title (system attribute \"BAGHOLDER_TITLE\")"])
        .env("BAGHOLDER_TITLE", title)
        .env("BAGHOLDER_BODY", body)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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

pub fn windows_script() -> String {
    WINDOWS_SCRIPT.replace("__URL__", &url()).replace("__APP__", APP_NAME)
}

fn windows_deliver(title: &str, body: &str) -> bool {
    // the app id a toast is shown under, with Bagholder's name and icon, in the
    // person's own registry hive
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let key = format!("HKCU\\Software\\Classes\\AppUserModelId\\{}", APP_NAME);
        run("reg", &["add", &key, "/v", "DisplayName", "/t", "REG_SZ", "/d", APP_NAME, "/f"]);
        if icon().exists() {
            run("reg", &["add", &key, "/v", "IconUri", "/t", "REG_SZ", "/d", &icon().to_string_lossy(), "/f"]);
        }
    });
    let shell = if which("powershell.exe").is_some() || which("powershell").is_some() { "powershell" } else { "pwsh" };
    Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", &windows_script()])
        .env("BAGHOLDER_TITLE", title)
        .env("BAGHOLDER_BODY", body)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// --- Linux ------------------------------------------------------------------------

fn linux_deliver(title: &str, body: &str) -> bool {
    let mut cmd = Command::new("notify-send");
    cmd.arg(format!("--app-name={}", APP_NAME));
    if icon().exists() {
        cmd.arg(format!("--icon={}", icon().to_string_lossy()));
    }
    cmd.args([title, body]).output().map(|o| o.status.success()).unwrap_or(false)
}

// --- the page's channel -------------------------------------------------------------

/// `notify.stream`: every row made after `after` (or after the stream opens),
/// each once, with a comment between them every heartbeat. `write` answers
/// false when the reader has gone.
pub fn stream<W: FnMut(&str) -> bool>(after: Option<i64>, mut write: W) {
    let mut last = match after {
        Some(a) => a,
        None => app().open().ok().and_then(|c| bagholder_store::feeds::latest_notification_id(&c).ok()).unwrap_or(0),
    };
    if !write(": bagholder\n\n") {
        return;
    }
    while !app().stopping() {
        let rows = app().open().ok().and_then(|c| bagholder_store::feeds::list_notifications(&c, last, "", false, 50, false).ok()).unwrap_or_default();
        if rows.is_empty() {
            let (m, c) = wake();
            let g = m.lock().unwrap();
            let _ = c.wait_timeout(g, HEARTBEAT);
            let rows = app().open().ok().and_then(|c| bagholder_store::feeds::list_notifications(&c, last, "", false, 50, false).ok()).unwrap_or_default();
            if rows.is_empty() {
                if !write(": ping\n\n") {
                    return;
                }
                continue;
            }
            for r in rows {
                let id = r["id"].as_i64().unwrap_or(0);
                last = last.max(id);
                if !write(&format!("id: {}\ndata: {}\n\n", id, bagholder_store::tables::py_json(&r))) {
                    return;
                }
            }
            continue;
        }
        for r in rows {
            let id = r["id"].as_i64().unwrap_or(0);
            last = last.max(id);
            if !write(&format!("id: {}\ndata: {}\n\n", id, bagholder_store::tables::py_json(&r))) {
                return;
            }
        }
    }
}
