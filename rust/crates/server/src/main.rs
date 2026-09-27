//! The Bagholder server: the page and its assets, the model behind them, the
//! Wealthsimple session and sync, orders, and the market-data loops.
//!
//! This file is the process: where its data and its page are, which port it
//! takes, what runs in the background, and how it stops. The HTTP server is
//! `http`.

mod app;
mod carry;
mod compare;
mod demo_facts;
mod docs;
mod broker_reads;
mod due;
mod engine_inputs;
mod clear;
mod csv_import;
mod entries;
mod figures;
mod following;
mod pull_broker;
mod read_sources;
mod events;
mod feeds;
mod http;
mod legacy_import;
mod legacy_orders;
mod market_context;
mod login;
mod notify;
mod orders;
mod session;
mod status;
mod update;
mod versions;
mod views;
mod wire;

use std::path::{Path, PathBuf};

use app::log;

const PORTS: [u16; 3] = [8765, 8766, 8767];
fn home_dir() -> Result<PathBuf, String> {
    let env = app::env_text("BAGHOLDER_HOME")?.unwrap_or_default();
    if !env.trim().is_empty() {
        return Ok(PathBuf::from(env.trim()));
    }
    let base = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    Ok(Path::new(&base).join(".bagholder-rust"))
}

/// Where this copy is: the checkout the executable was built in (the folder
/// holding `rust/Cargo.toml`), else the executable's own folder (a release or the
/// container, with the page beside it or in the binary), else the working folder.
fn root_dir() -> Result<PathBuf, String> {
    let env = app::env_text("BAGHOLDER_ROOT")?.unwrap_or_default();
    if !env.trim().is_empty() {
        return Ok(PathBuf::from(env.trim()));
    }
    let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    Ok(match exe.as_deref().and_then(Path::parent) {
        Some(dir) => checkout_of(dir).unwrap_or_else(|| dir.to_path_buf()),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    })
}

/// The checkout an executable in `dir` was built in: `rust/target/<profile>/`
/// (or `rust/target/<target>/<profile>/`) under the folder holding `rust/Cargo.toml`.
fn checkout_of(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().take(5).find(|d| d.join("rust/Cargo.toml").is_file()).map(Path::to_path_buf)
}

// --------------------------------------------------------------------------
// start
// --------------------------------------------------------------------------

fn port_choices() -> Result<Vec<u16>, String> {
    let env = app::env_text("BAGHOLDER_PORT")?.unwrap_or_default();
    Ok(match env.trim().parse::<u16>() {
        Ok(p) if p >= 1024 && env.trim().bytes().all(|c| c.is_ascii_digit()) => vec![p],
        _ => PORTS.to_vec(),
    })
}

fn open_browser(url: &str) {
    let cmd: (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    // the address is printed above whatever happens here: a browser that will not open is said beside it
    if let Err(e) = std::process::Command::new(cmd.0).args(&cmd.1).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        eprintln!("The browser could not be opened ({e}): open {url} yourself.");
    }
}

fn serve() -> i32 {
    // what the environment says, and the data folder made the app's alone: each
    // refused stops the server here, saying why
    let started = (|| -> Result<_, String> {
        let home = home_dir()?;
        std::fs::create_dir_all(&home).map_err(|e| format!("the data folder {} could not be made: {e}", home.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("the data folder {} could not be kept private: {e}", home.display()))?;
        }
        let b = app::env_text("BAGHOLDER_BIND")?.unwrap_or_default().trim().to_string();
        let bind_host = if b.is_empty() { "127.0.0.1".to_string() } else { b };
        Ok((home, root_dir()?, bind_host, port_choices()?))
    })();
    let (home, root, bind_host, ports) = match started {
        Ok(s) => s,
        Err(e) => {
            log(&format!("bagholder: {e}"));
            return 1;
        }
    };
    // a first start with nothing of its own takes the Python app's database, which the start then imports
    if let Some(python) = legacy_import::python_home(|k| std::env::var_os(k)) {
        match legacy_import::adopt_python_database(&home, &python) {
            Ok(Some(from)) => log(&format!("bagholder: a copy of the Python app's database {} is this app's first", from.display())),
            Ok(None) => {}
            Err(e) => {
                log(&format!("bagholder: {e}"));
                return 1;
            }
        }
    }
    let a = app::App::new(home, root, bind_host.clone());
    // an update the supervisor rolled back is said in the header by the server it started
    update::recall_failure(&a);
    // the local model keeps its file in this app's data folder, and is off until told so
    if let Err(e) = bagholder_market::localmodel::serve_from(&a.home) {
        log(&format!("bagholder: the local model is off: {}", e));
    }
    // the figure path: a book this build cannot open stops the server, saying why
    match figures::Figures::open(&a.home, bagholder_core::jiff::Timestamp::now()) {
        Ok(f) => {
            // the earlier store, carried once into the book and the market cache;
            // nothing opens it after this
            if let Err(e) = carry::from_old_store(&a, &f, bagholder_core::jiff::Timestamp::now()) {
                log(&format!("bagholder: {e}"));
                return 1;
            }
            a.set_figures(f);
            // the default tile row where none was chosen
            following::open(&a);
        }
        Err(e) => {
            log(&format!("bagholder: {e}"));
            return 1;
        }
    }
    // the market cache's first borrow brings it to this build's schema, so one that
    // cannot be opened stops the server here, saying why
    if let Err(e) = a.cache() {
        log(&format!("bagholder: the market cache could not be opened: {}", e));
        return 1;
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
    for &port in &ports {
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
            let ports: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
            eprintln!("Could not bind {}:{} ({})", bind_host, ports.join("-"), last_err);
            return 1;
        }
    };
    *a.port.lock().unwrap() = port;

    // from here on a change reaches an open page because it happened: every commit
    // on any connection (wired to the bus when the app was built), every write to
    // the app's state; the day turning in the person's zone is the scheduler's (`due.rs`)
    let events_for_localmodel = a.events.clone();
    bagholder_market::localmodel::on_change(move || events_for_localmodel.signal());
    // the figure path's reads, each when it is due
    a.spawn_with("bagholder-figures", due::run);
    // Wealthsimple: the pull and the balances, each when it is due
    a.spawn_with("bagholder-broker", broker_reads::run);
    a.spawn_with("bagholder-token", session::token_loop);
    a.spawn_with("bagholder-update-check", |app| {
        update::check_for_update(&app);
    });
    a.spawn_with("bagholder-orders-loop", |app| orders::orders_loop(&app));
    a.spawn_with("bagholder-bracket-loop", |app| orders::bracket_loop(&app));
    a.spawn_with("bagholder-exposure-loop", feeds::exposure_loop);
    a.spawn_with("bagholder-news-loop", feeds::news_loop);
    a.spawn_with("bagholder-market-loop", feeds::market_loop);
    a.spawn_with("bagholder-archive", feeds::archive_loop);
    a.spawn_with("bagholder-watch", feeds::watch_loop);
    a.spawn_with("bagholder-filings-sweep", feeds::filings_sweep_loop);
    a.spawn_with("bagholder-disclosure-reader", feeds::disclosure_read_loop);
    a.spawn_with("bagholder-shorts-sweep", feeds::shorts_sweep_loop);

    let url = format!("http://127.0.0.1:{}", port);
    println!("Bagholder  {}", url);
    // a second instance run for verification must not open anyone's browser
    if !app::env_on("BAGHOLDER_NO_BROWSER") {
        open_browser(&url);
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
    // the made-up book's facts and prices, for the browser tests and screenshots
    if args.first().map(String::as_str) == Some("demo-facts") {
        std::process::exit(demo_facts::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("source-health") {
        std::process::exit(read_sources::cli_health(&args[1..]));
    }
    let child = std::env::var("BAGHOLDER_CHILD").map(|v| v == "1").unwrap_or(false);
    if child || update::updates_off() {
        // the supervisor exists to restart an updated server; a copy that never updates runs plain
        std::process::exit(serve());
    }
    match home_dir() {
        Ok(home) => std::process::exit(update::supervise(&home, update::UPDATE_HEALTHY_SEC)),
        Err(e) => {
            log(&format!("bagholder: {e}"));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    //! The data folder, the root, the protocol, the image and the launcher.
    use std::path::{Path, PathBuf};

    /// The repository root, with this workspace in rust/.
    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// Where the Rust build keeps its data: ~/.bagholder-rust, or wherever BAGHOLDER_HOME says.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_the_default_folder_is_dot_bagholder_rust() {
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("BAGHOLDER_HOME").ok();
        std::env::remove_var("BAGHOLDER_HOME");
        let home = crate::home_dir().unwrap();
        if let Some(v) = previous {
            std::env::set_var("BAGHOLDER_HOME", v);
        }
        assert_eq!(home.file_name().unwrap(), ".bagholder-rust");
    }

    #[cfg(unix)]
    #[test]
    fn test_a_bagholder_home_that_is_not_text_is_refused_not_read_as_unset() {
        use std::os::unix::ffi::OsStrExt;
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("BAGHOLDER_HOME");
        std::env::set_var("BAGHOLDER_HOME", std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        let home = crate::home_dir();
        match previous {
            Some(v) => std::env::set_var("BAGHOLDER_HOME", v),
            None => std::env::remove_var("BAGHOLDER_HOME"),
        }
        assert!(home.unwrap_err().contains("BAGHOLDER_HOME"));
    }

    #[test]
    fn test_bagholder_home_decides_where_the_folder_is() {
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("BAGHOLDER_HOME").ok();
        let d = std::env::temp_dir().join(format!("bh-home-{}-elsewhere", std::process::id()));
        std::env::set_var("BAGHOLDER_HOME", &d);
        let home = crate::home_dir().unwrap();
        match previous {
            Some(v) => std::env::set_var("BAGHOLDER_HOME", v),
            None => std::env::remove_var("BAGHOLDER_HOME"),
        }
        assert_eq!(home, d);
    }

    /// A server built in a checkout finds the checkout by its workspace; anywhere
    /// else (a release, the container) there is none, and its own folder is the root.
    #[test]
    fn test_the_root_is_the_checkout_the_server_was_built_in() {
        let root = root().canonicalize().unwrap();
        for built in ["rust/target/release", "rust/target/debug", "rust/target/aarch64-apple-darwin/release"] {
            assert_eq!(crate::checkout_of(&root.join(built)), Some(root.clone()), "{built}");
        }
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(crate::checkout_of(elsewhere.path()), None);
    }

    #[test]
    fn test_protocol_matches_page() {
        // status::payload()["protocol"] is app::PROTOCOL by construction
        let ts = std::fs::read_to_string(root().join("web/src/lib/protocol.ts")).unwrap();
        let m = regex::Regex::new(r"export const PROTOCOL = '([^']+)'").unwrap().captures(&ts).expect("the page names its protocol");
        assert_eq!(&m[1], crate::app::PROTOCOL);
    }

    #[test]
    fn test_the_image_builds_this_workspace_and_the_page_from_the_repository_root() {
        let docker = std::fs::read_to_string(root().join("rust/Dockerfile")).unwrap();
        for line in [
            "COPY rust/rust-toolchain.toml rust/Cargo.toml rust/Cargo.lock ./",
            "COPY rust/crates crates",
            "COPY rust/docker-entrypoint.sh /usr/local/bin/bagholder-entrypoint",
            "COPY web/package.json web/package-lock.json ./",
            "COPY --from=web /web/dist ./web/dist",
        ] {
            assert!(docker.lines().any(|l| l.trim() == line), "rust/Dockerfile: {}", line);
        }
    }

    #[test]
    fn test_nothing_the_image_needs_is_kept_out_of_it() {
        let ignored: Vec<String> = std::fs::read_to_string(root().join(".dockerignore")).unwrap().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        for needed in ["rust", "rust/crates", "rust/Cargo.toml", "rust/Cargo.lock", "rust/rust-toolchain.toml", "rust/docker-entrypoint.sh", "web", "web/src", "web/public", "web/package.json", "web/package-lock.json"] {
            assert!(!ignored.iter().any(|i| i == needed), "{} kept out of the image", needed);
        }
    }

    // --- the launcher: `python3 bagholder.py` -------------------------------------------

    #[test]
    fn test_the_launcher_is_one_file_in_both_places() {
        // the owner starts `bagholder.py` at the root; a Python copy's supervisor restarts `python/bagholder.py`
        assert_eq!(std::fs::read(root().join("bagholder.py")).unwrap(), std::fs::read(root().join("python/bagholder.py")).unwrap());
    }

    #[cfg(unix)]
    mod launcher {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        fn python3() -> PathBuf {
            let path = std::env::var_os("PATH").unwrap_or_default();
            std::env::split_paths(&path).map(|d| d.join("python3")).find(|p| p.is_file()).expect("python3 on the PATH: the launcher is a Python file")
        }

        /// A made-up checkout: the workspace's manifest and the launcher in both places.
        fn checkout() -> tempfile::TempDir {
            let t = tempfile::tempdir().unwrap();
            let p = t.path();
            std::fs::create_dir_all(p.join("rust")).unwrap();
            std::fs::write(p.join("rust/Cargo.toml"), "").unwrap();
            std::fs::create_dir_all(p.join("python")).unwrap();
            std::fs::create_dir_all(p.join("bin")).unwrap();
            std::fs::copy(root().join("bagholder.py"), p.join("bagholder.py")).unwrap();
            std::fs::copy(root().join("python/bagholder.py"), p.join("python/bagholder.py")).unwrap();
            t
        }

        /// A program that writes its parent's id, its folder, one variable of its
        /// environment and its arguments, a line each, to `out` (appending).
        fn stand_in(at: &Path, out: &Path) {
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, format!("#!/bin/sh\n{{ echo \"$PPID\"; pwd -P; echo \"$BAGHOLDER_PROBE\"; for a in \"$@\"; do echo \"$a\"; done; echo ==; }} >> '{}'\n", out.display())).unwrap();
            std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn launch(dir: &Path, file: &str) -> std::process::Output {
            Command::new(python3()).arg(dir.join(file)).arg("--version").current_dir(std::env::temp_dir()).env("PATH", dir.join("bin")).env("BAGHOLDER_PROBE", "passed through").output().unwrap()
        }

        fn calls(out: &Path) -> Vec<Vec<String>> {
            let text = std::fs::read_to_string(out).unwrap_or_default();
            text.split("==\n").filter(|c| !c.is_empty()).map(|c| c.lines().map(String::from).collect()).collect()
        }

        fn real(p: &Path) -> String {
            p.canonicalize().unwrap().to_string_lossy().into_owned()
        }

        #[test]
        fn test_with_no_cargo_the_built_server_takes_the_launchers_place() {
            for file in ["bagholder.py", "python/bagholder.py"] {
                let t = checkout();
                let out = t.path().join("calls");
                stand_in(&t.path().join("rust/target/release/bagholder"), &out);
                let r = launch(t.path(), file);
                assert!(r.status.success(), "{file}: {}", String::from_utf8_lossy(&r.stderr));
                // the server's parent is whatever started the launcher: the launcher's process became the server
                assert_eq!(calls(&out), vec![vec![std::process::id().to_string(), real(t.path()), "passed through".into(), "--version".into()]], "{file}");
            }
        }

        #[test]
        fn test_with_cargo_the_checkout_is_built_and_run_in_its_workspace() {
            let t = checkout();
            let out = t.path().join("calls");
            stand_in(&t.path().join("bin/cargo"), &out);
            // a server built earlier is not what runs: the checkout's sources may be newer
            stand_in(&t.path().join("rust/target/release/bagholder"), &t.path().join("stale"));
            std::fs::create_dir_all(t.path().join("web/dist")).unwrap();
            std::fs::write(t.path().join("web/dist/index.html"), "").unwrap();
            let r = launch(t.path(), "python/bagholder.py");
            assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
            let cwd = real(&t.path().join("rust"));
            assert_eq!(calls(&out), vec![vec![std::process::id().to_string(), cwd, "passed through".into(), "run".into(), "--release".into(), "--bin".into(), "bagholder".into(), "--".into(), "--version".into()]]);
            assert!(!t.path().join("stale").exists());
        }

        #[test]
        fn test_a_page_not_yet_built_is_built_before_the_server() {
            let t = checkout();
            let npm_out = t.path().join("npm-calls");
            let out = t.path().join("calls");
            stand_in(&t.path().join("bin/npm"), &npm_out);
            stand_in(&t.path().join("bin/cargo"), &out);
            std::fs::create_dir_all(t.path().join("web")).unwrap();
            let r = launch(t.path(), "bagholder.py");
            assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
            let web = real(&t.path().join("web"));
            let npm: Vec<Vec<String>> = calls(&npm_out).into_iter().map(|c| c[1..].to_vec()).collect();
            assert_eq!(npm, vec![vec![web.clone(), "passed through".into(), "ci".into()], vec![web, "passed through".into(), "run".into(), "build".into()]]);
            assert_eq!(calls(&out).len(), 1, "then the server");
        }

        /// A built page older than any of the page's sources is built again; one
        /// newer than all of them is not.
        #[test]
        fn test_a_page_older_than_its_sources_is_built_again() {
            let at = |p: &Path, secs: u64| std::fs::File::options().write(true).open(p).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
            for source in ["src/lib/deep/x.ts", "public/favicon.png", "index.html", "package.json", "package-lock.json", "vite.config.ts"] {
                let t = checkout();
                let web = t.path().join("web");
                let npm_out = t.path().join("npm-calls");
                stand_in(&t.path().join("bin/npm"), &npm_out);
                stand_in(&t.path().join("bin/cargo"), &t.path().join("calls"));
                std::fs::create_dir_all(web.join("dist")).unwrap();
                std::fs::write(web.join("dist/index.html"), "").unwrap();
                std::fs::create_dir_all(web.join(source).parent().unwrap()).unwrap();
                std::fs::write(web.join(source), "").unwrap();
                // the build newer than the source: nothing to build
                at(&web.join(source), 1_000);
                at(&web.join("dist/index.html"), 2_000);
                assert!(launch(t.path(), "bagholder.py").status.success());
                assert!(calls(&npm_out).is_empty(), "{source}: current");
                // the source changed after the build
                at(&web.join(source), 3_000);
                assert!(launch(t.path(), "bagholder.py").status.success());
                assert_eq!(calls(&npm_out).len(), 2, "{source}: npm ci and npm run build");
            }
        }

        #[test]
        fn test_with_neither_it_says_what_to_run_and_fails() {
            let t = checkout();
            let r = launch(t.path(), "python/bagholder.py");
            assert_eq!(r.status.code(), Some(1));
            let said = String::from_utf8_lossy(&r.stderr);
            assert!(said.contains("cargo run --release --bin bagholder") && said.contains("npm ci && npm run build"), "{said}");
        }
    }
}

#[cfg(test)]
mod tests_common;

#[cfg(test)]
mod tests_misc;


#[cfg(test)]
mod tests_execution;
#[cfg(test)]
mod tests_orders;

#[cfg(test)]
#[cfg(test)]
mod tests_orders_wire_golden;
#[cfg(test)]
mod tests_types;
#[cfg(test)]
mod tests_docs_golden;
#[cfg(test)]
mod tests_routes_golden;
#[cfg(test)]
mod tests_boundary;
#[cfg(test)]
mod tests_lints;
#[cfg(test)]
mod tests_universes;
#[cfg(test)]
mod tests_failures;
#[cfg(test)]
mod tests_failures_lower;
