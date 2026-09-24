//! The Bagholder server: the page and its assets, the model behind them, the
//! Wealthsimple session and sync, orders, and the market-data loops.
//!
//! This file is the process: where its data and its page are, which port it
//! takes, what runs in the background, and how it stops. The HTTP server is
//! `http`.

mod app;
mod compare;
mod docs;
mod engine_inputs;
mod pull_broker;
mod read_sources;
mod events;
mod feeds;
mod http;
mod legacy_import;
mod login;
mod model_cache;
mod notify;
mod orders;
mod session;
mod status;
mod update;
mod versions;

use std::path::{Path, PathBuf};

use app::{log, spawn};

const PORTS: [u16; 3] = [8765, 8766, 8767];
const ACTIVITY_PULL_SEC: i64 = 24 * 60 * 60;

fn home_dir() -> PathBuf {
    let env = std::env::var("BAGHOLDER_HOME").unwrap_or_default();
    if !env.trim().is_empty() {
        return PathBuf::from(env.trim());
    }
    let base = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    Path::new(&base).join(".bagholder-rust")
}

/// Where the page and its assets are: beside the executable, or in a checkout
/// the executable was built in, or the working folder.
fn root_dir() -> PathBuf {
    let env = std::env::var("BAGHOLDER_ROOT").unwrap_or_default();
    if !env.trim().is_empty() {
        return PathBuf::from(env.trim());
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        for d in dir.ancestors().take(4) {
            if d.join("ledger.html").is_file() {
                return d.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

// --------------------------------------------------------------------------
// start
// --------------------------------------------------------------------------

fn port_choices() -> Vec<u16> {
    let env = std::env::var("BAGHOLDER_PORT").unwrap_or_default();
    match env.trim().parse::<u16>() {
        Ok(p) if p >= 1024 && env.trim().bytes().all(|c| c.is_ascii_digit()) => vec![p],
        _ => PORTS.to_vec(),
    }
}

fn open_browser(url: &str) {
    let cmd: (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = std::process::Command::new(cmd.0).args(&cmd.1).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

fn serve() -> i32 {
    let home = home_dir();
    let _ = std::fs::create_dir_all(&home);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700));
    }
    let bind_host = { let b = std::env::var("BAGHOLDER_BIND").unwrap_or_default().trim().to_string(); if b.is_empty() { "127.0.0.1".to_string() } else { b } };
    let a = app::App::new(home, root_dir(), bind_host.clone());
    match a.open() {
        Ok(conn) => {
            if let Err(e) = bagholder_store::relabel::ensure(&conn) {
                log(&format!("bagholder: the store could not be prepared: {}", e));
                return 1;
            }
        }
        Err(e) => {
            log(&format!("bagholder: the store could not be opened: {}", e));
            return 1;
        }
    }
    session::boot_session(&a);

    // The runtime the HTTP server and the page streams run on. Everything else --
    // SQLite, the market and Wealthsimple clients, the background loops -- is
    // synchronous and runs on threads of its own or on the runtime's blocking pool.
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("bagholder-http").build() {
        Ok(rt) => rt,
        Err(e) => {
            log(&format!("bagholder: the runtime could not be started: {}", e));
            return 1;
        }
    };
    let mut bound = None;
    let mut last_err = String::new();
    for port in port_choices() {
        match runtime.block_on(tokio::net::TcpListener::bind((bind_host.as_str(), port))) {
            Ok(listener) => {
                bound = Some((listener, port));
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let (listener, port) = match bound {
        Some(b) => b,
        None => {
            let ports: Vec<String> = port_choices().iter().map(|p| p.to_string()).collect();
            eprintln!("Could not bind {}:{} ({})", bind_host, ports.join("-"), last_err);
            return 1;
        }
    };
    *a.port.lock().unwrap() = port;

    // from here on a change reaches an open page because it happened: every commit
    // on any connection (wired to the bus when the app was built), every write to
    // the app's state, the day turning
    events::signal_at_each_midnight(a.clone());
    let events_for_localmodel = a.events.clone();
    bagholder_market::localmodel::on_change(move || events_for_localmodel.signal());
    a.spawn_with("bagholder-auto-sync", session::auto_sync_loop);
    a.spawn_with("bagholder-market", |app| {
        feeds::refresh_market_data(&app);
    });
    a.spawn_with("bagholder-update-check", |app| {
        update::check_for_update(&app);
    });
    a.spawn_with("bagholder-quote-loop", feeds::quote_loop);
    a.spawn_with("bagholder-portfolio-loop", session::portfolio_loop);
    a.spawn_with("bagholder-orders-loop", |app| orders::orders_loop(&app));
    a.spawn_with("bagholder-bracket-loop", |app| orders::bracket_loop(&app));
    a.spawn_with("bagholder-exposure-loop", feeds::exposure_loop);
    // rows added before the bare-ticker convention (Wealthsimple's `.TO` on a dual listing) take it now
    if let Ok(conn) = a.open() {
        for w in bagholder_store::feeds::list_watchlist(&conn).unwrap_or_default() {
            let sym = w.symbol.clone();
            let bare = bagholder_model::venues::tmx_symbol(&sym);
            if !bare.is_empty() && bare != sym {
                let _ = bagholder_store::feeds::remove_watch(&conn, &sym, &w.exchange);
                let _ = bagholder_store::feeds::add_watch(&conn, &bare, &w.exchange, &w.name, &w.currency, &w.security_id, &w.added_at);
            }
        }
    }
    a.spawn_with("bagholder-news-loop", feeds::news_loop);
    a.spawn_with("bagholder-universe-loop", feeds::universe_loop);
    a.spawn_with("bagholder-market-loop", feeds::market_loop);
    a.spawn_with("bagholder-archive", feeds::archive_loop);
    a.spawn_with("bagholder-watch", feeds::watch_loop);
    a.spawn_with("bagholder-filings-sweep", feeds::filings_sweep_loop);
    a.spawn_with("bagholder-disclosure-reader", feeds::disclosure_read_loop);
    a.spawn_with("bagholder-shorts-sweep", feeds::shorts_sweep_loop);

    let url = format!("http://127.0.0.1:{}", port);
    println!("Bagholder  {}", url);
    // a second instance run for verification must not open anyone's browser
    if std::env::var("BAGHOLDER_NO_BROWSER").unwrap_or_default().trim().is_empty() {
        open_browser(&url);
    }
    if a.state.lock().unwrap().connected {
        let due = a.open().ok().and_then(|c| bagholder_store::admin::activity_pull_due(&c, app::now_unix() as i64 - 0).ok()).unwrap_or(false);
        let _ = ACTIVITY_PULL_SEC;
        if due {
            let b = a.clone();
            spawn("bagholder-boot-sync", move || {
                session::run_sync(&b, true, true);
            });
        } else {
            let b = a.clone();
            spawn("bagholder-listings", move || {
                if let Some(sess) = session::load_session(&b) {
                    let problems = session::fill_listings(&b, &sess, false);
                    if !problems.is_empty() {
                        b.state.lock().unwrap().error = problems.join("; ");
                    }
                }
            });
        }
    }

    // Ctrl-C and a service manager's TERM stop the app the way its own updater
    // does: the requests in hand finish, the streams end, the loops wake and leave.
    let stop_app = a.clone();
    runtime.spawn(async move {
        let term = async {
            #[cfg(unix)]
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut t) => { t.recv().await; }
                Err(_) => std::future::pending::<()>().await,
            }
            #[cfg(not(unix))]
            std::future::pending::<()>().await;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term => {}
        }
        stop_app.request_stop();
    });
    if let Err(e) = runtime.block_on(http::serve(listener, http::AppState { app: a.clone() })) {
        log(&format!("bagholder: the server stopped: {}", e));
    }
    // a producer thread still writing to a stream that has gone is not waited for
    runtime.shutdown_background();
    a.exit_code.load(std::sync::atomic::Ordering::SeqCst)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("bagholder {}", app::APP_VERSION);
        return;
    }
    // the book this build will move onto (docs/plans/stage-1-foundation.md), made
    // from a database kept today; nothing the app runs calls it yet
    if args.first().map(String::as_str) == Some("import-book") {
        std::process::exit(legacy_import::cli(&args[1..]));
    }
    // the new engine beside the old model on the same data (docs/plans/stage-2-engine.md)
    if args.first().map(String::as_str) == Some("compare-figures") {
        std::process::exit(compare::cli(&args[1..]));
    }
    // the readers of market data and facts, run once (docs/plans/stage-3a-sources.md)
    if args.first().map(String::as_str) == Some("read-sources") {
        std::process::exit(read_sources::cli_read(&args[1..]));
    }
    // Wealthsimple pulled into the book (docs/plans/stage-3b-wealthsimple.md)
    if args.first().map(String::as_str) == Some("pull-broker") {
        std::process::exit(pull_broker::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("source-health") {
        std::process::exit(read_sources::cli_health(&args[1..]));
    }
    let child = std::env::var("BAGHOLDER_CHILD").map(|v| v == "1").unwrap_or(false);
    if child || update::updates_off() {
        // the supervisor exists to restart an updated server; a copy that never updates runs plain
        std::process::exit(serve());
    }
    std::process::exit(update::supervise(&home_dir(), update::UPDATE_HEALTHY_SEC));
}

#[cfg(test)]
mod tests {
    //! The page, the package contents and the protocol check.
    use std::path::PathBuf;

    /// The repository root: the shared page and data, with this workspace in rust/.
    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn page() -> String {
        std::fs::read_to_string(root().join("ledger.html")).unwrap()
    }

    const SHIPPED: [&str; 3] = ["ledger.html", "lightweight-charts.js", "favicon.png"];

    /// Where the Rust build keeps its data: ~/.bagholder-rust, or wherever BAGHOLDER_HOME says.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_the_default_folder_is_dot_bagholder_rust() {
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("BAGHOLDER_HOME").ok();
        std::env::remove_var("BAGHOLDER_HOME");
        let home = crate::home_dir();
        if let Some(v) = previous {
            std::env::set_var("BAGHOLDER_HOME", v);
        }
        assert_eq!(home.file_name().unwrap(), ".bagholder-rust");
    }

    #[test]
    fn test_bagholder_home_decides_where_the_folder_is() {
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("BAGHOLDER_HOME").ok();
        let d = std::env::temp_dir().join(format!("bh-home-{}-elsewhere", std::process::id()));
        std::env::set_var("BAGHOLDER_HOME", &d);
        let home = crate::home_dir();
        match previous {
            Some(v) => std::env::set_var("BAGHOLDER_HOME", v),
            None => std::env::remove_var("BAGHOLDER_HOME"),
        }
        assert_eq!(home, d);
    }

    #[test]
    fn test_every_script_on_the_page_parses() {
        let node = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("node")).find(|p| p.is_file()));
        let Some(node) = node else {
            eprintln!("skipped: node is needed to parse the page's script");
            return;
        };
        let html = page();
        let scripts: Vec<&str> = regex::Regex::new(r"(?s)<script>(.*?)</script>").unwrap().captures_iter(&html).map(|c| c.get(1).unwrap().as_str()).collect();
        assert!(!scripts.is_empty(), "the page carries its script inline");
        for (i, js) in scripts.iter().enumerate() {
            let path = std::env::temp_dir().join(format!("bagholder-page-{}-{}.js", std::process::id(), i));
            std::fs::write(&path, js).unwrap();
            let r = std::process::Command::new(&node).arg("--check").arg(&path).output().unwrap();
            let _ = std::fs::remove_file(&path);
            assert!(r.status.success(), "script {} does not parse:\n{}", i, String::from_utf8_lossy(&r.stderr).chars().take(2000).collect::<String>());
        }
    }

    #[test]
    fn test_no_title_attribute_anywhere_on_the_page() {
        let found: Vec<String> = regex::Regex::new(r#" title=\\?["']"#).unwrap().find_iter(&page()).map(|m| m.as_str().to_string()).collect();
        assert_eq!(found, Vec::<String>::new(), "nothing on the page gets a browser tooltip");
    }

    #[test]
    fn test_protocol_matches_page() {
        let html = page();
        let m = regex::Regex::new(r#"const PROTOCOL = "([^"]+)""#).unwrap().captures(&html).expect("the page names its protocol");
        assert_eq!(&m[1], crate::app::PROTOCOL);
        // status::payload()["protocol"] is app::PROTOCOL by construction
        // and the Svelte page's
        let ts = std::fs::read_to_string(root().join("web/src/lib/protocol.ts")).unwrap();
        let m = regex::Regex::new(r"export const PROTOCOL = '([^']+)'").unwrap().captures(&ts).expect("the page names its protocol");
        assert_eq!(&m[1], crate::app::PROTOCOL);
    }

    #[test]
    fn test_the_image_carries_the_page_and_its_chart_library() {
        let docker = std::fs::read_to_string(root().join("rust/Dockerfile")).unwrap();
        let copy = regex::Regex::new(r"^COPY\s+(.*?)\s+\./\s*$").unwrap();
        let copied: Vec<String> = docker.lines().filter_map(|l| copy.captures(l.trim()).map(|c| c[1].to_string())).flat_map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        assert!(!copied.is_empty(), "the Dockerfile copies the app in");
        for needed in SHIPPED {
            assert!(copied.iter().any(|c| c == needed), "{} is served by the app", needed);
        }
    }

    #[test]
    fn test_nothing_the_image_needs_is_kept_out_of_it() {
        let ignored: Vec<String> = std::fs::read_to_string(root().join(".dockerignore")).unwrap().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        for needed in SHIPPED.iter().copied().chain(["rust", "rust/crates", "rust/Cargo.toml", "rust/Cargo.lock", "rust/rust-toolchain.toml", "rust/docker-entrypoint.sh"]) {
            assert!(!ignored.iter().any(|i| i == needed), "{} kept out of the image", needed);
        }
    }

    #[test]
    fn test_the_release_archive_carries_the_page_and_its_assets() {
        let out = match std::process::Command::new("git").arg("-C").arg(root()).arg("ls-files").output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            _ => {
                eprintln!("skipped: not a git checkout");
                return;
            }
        };
        let tracked: Vec<&str> = out.lines().collect();
        for needed in SHIPPED {
            assert!(tracked.contains(&needed), "{} untracked: the release archive would not carry it", needed);
        }
    }

    #[test]
    fn test_the_image_builds_this_workspace_from_the_repository_root() {
        let docker = std::fs::read_to_string(root().join("rust/Dockerfile")).unwrap();
        for line in ["COPY rust/rust-toolchain.toml rust/Cargo.toml rust/Cargo.lock ./", "COPY rust/crates crates", "COPY rust/docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint"] {
            assert!(docker.lines().any(|l| l.trim() == line), "rust/Dockerfile: {}", line);
        }
    }

    #[test]
    fn test_the_python_app_carries_the_same_version_and_protocol() {
        // both desktop apps are one product version and speak one protocol with the page
        let Ok(py) = std::fs::read_to_string(root().join("python/bagholder.py")) else {
            eprintln!("skipped: no python/ beside rust/");
            return;
        };
        let version = regex::Regex::new(r#"(?m)^APP_VERSION = "([^"]+)""#).unwrap().captures(&py).expect("APP_VERSION in python/bagholder.py");
        let protocol = regex::Regex::new(r#"(?m)^PROTOCOL = "([^"]+)""#).unwrap().captures(&py).expect("PROTOCOL in python/bagholder.py");
        assert_eq!(&version[1], crate::app::APP_VERSION);
        assert_eq!(&protocol[1], crate::app::PROTOCOL);
    }

    #[test]
    fn test_the_server_serves_what_ships_from_its_root() {
        // the legacy page and its files are read from the root at request time
        for needed in SHIPPED {
            assert!(root().join(needed).is_file(), "{} beside the workspace", needed);
        }
        let src = include_str!("http/assets.rs");
        assert!(src.contains("\"lightweight-charts.js\"") && src.contains("\"favicon.png\"") && src.contains("feeds::ledger_path("));
    }
}

#[cfg(test)]
mod tests_common;

#[cfg(test)]
mod tests_misc;


#[cfg(test)]
mod tests_brackets;

#[cfg(test)]
mod tests_orders;
#[cfg(test)]
mod tests_types;
#[cfg(test)]
mod tests_docs_golden;
#[cfg(test)]
mod tests_routes_golden;
