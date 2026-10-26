//! `bagholder pull-broker <book folder> [--replay <capture>] [--now <instant>] [--statements-only]`
//! (`docs/plans/stage-3b-wealthsimple.md`, "The pull"): Wealthsimple pulled into
//! the book, over the network with the folder's saved sign-in, or from a capture
//! of its replies. The pull and the records are the same either way.
//! `--statements-only`: the pull's statements walk alone, on the statements the
//! book keeps and the cash last stated, asking nobody: what a release's walk does
//! to a copy of a book (the release gate).
//! A developer's command: built only with the `pull-broker` feature, never in a release.

use std::path::PathBuf;

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_broker::BrokerAdapter;
use bagholder_core::jiff::Timestamp;
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::{Source, Wealthsimple};
use bagholder_wealthsimple::replay::Replay;

pub fn cli(args: &[String]) -> i32 {
    let usage = "usage: bagholder pull-broker <book folder> [--replay <capture folder>] [--now <instant>] [--statements-only]";
    let mut positional = Vec::new();
    let mut replay = None;
    let mut now = None;
    let mut statements_only = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--replay" => replay = it.next().cloned(),
            "--now" => now = it.next().and_then(|t| t.parse::<Timestamp>().ok()),
            "--statements-only" => statements_only = true,
            _ => positional.push(a.clone()),
        }
    }
    let Some(home) = positional.first() else {
        eprintln!("{usage}");
        return 2;
    };
    let now = now.unwrap_or_else(Timestamp::now);
    let done = if statements_only { walk(&PathBuf::from(home), now) } else { run(&PathBuf::from(home), replay.map(PathBuf::from).as_deref(), now) };
    match done {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("the pull failed: {e}");
            1
        }
    }
}

fn run(home: &std::path::Path, capture: Option<&std::path::Path>, now: Timestamp) -> Result<String, String> {
    match capture {
        Some(c) => {
            let mut adapter = Wealthsimple::new(Replay::read(c).map_err(|e| format!("{}: {e}", c.display()))?);
            pull_with(home, &mut adapter, now)
        }
        None => {
            // the folder's own: a second process refreshing its sign-in beside the app
            // could post a refresh token the app just spent, and lose the sign-in
            crate::app::hold_home(home)?;
            let net = bagholder_net::Net::new(std::sync::Arc::new(bagholder_net::SystemClock), std::sync::Arc::new(bagholder_net::Limiter::new()));
            let file = bagholder_wealthsimple::session::SessionFile { path: home.join("session.json") };
            let mut adapter = Wealthsimple::new(bagholder_wealthsimple::client::Client::new(&net, file));
            pull_with(home, &mut adapter, now)
        }
    }
}

/// The statements walk of a pull, alone, on the book's kept statements: a capture
/// with no reply in it answers every read, so a month the book keeps none of is a
/// failure, never a read. The broker's accounts behind each of the book's are its
/// own references to them, each in the one currency the broker last stated the
/// account's cash in (an account in several states each list's own). It counts
/// what it did, and each failure by its words with the accounts' ids left out.
fn walk(home: &std::path::Path, now: Timestamp) -> Result<String, String> {
    crate::app::hold_home(home)?;
    let empty = std::env::temp_dir().join(format!("bagholder-no-replies-{}", std::process::id()));
    std::fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
    let replay = Replay::read(&empty).map_err(|e| e.to_string());
    std::fs::remove_dir(&empty).map_err(|e| format!("{}: {e}", empty.display()))?;
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, now).map_err(|e| e.to_string())?;
    let Some(crate::figures::BrokerKeys { connection, keys_of, closed, currencies }) = crate::figures::wealthsimple_keys(&book).map_err(|e| e.to_string())? else {
        return Err("the book has no Wealthsimple connection".into());
    };
    let mut adapter = Wealthsimple::new(replay?).with_currencies(currencies);
    let mut failures = Vec::new();
    let today = adapter.day(now);
    let d = bagholder_broker::statements::run(&book, &mut adapter, connection, &keys_of, &closed, today, now, &mut |_| {}, &mut failures).map_err(|e| e.to_string())?;
    let mut out = format!("statements walked: withdrawn {}, booked {}, joined {}, unreconciled {}, feed only {}, failures {}\n", d.withdrawn, d.booked, d.joined, d.unreconciled.len(), d.feed_only.len(), failures.len());
    let mut said: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (_, f) in &failures {
        let words = keys_of.values().flatten().fold(f.to_string(), |w, k| w.replace(k.as_str(), "<account>"));
        *said.entry(words).or_default() += 1;
    }
    for (words, n) in said {
        out.push_str(&format!("  {n} × {words}\n"));
    }
    Ok(out)
}

fn pull_with<S: Source>(home: &std::path::Path, adapter: &mut Wealthsimple<S>, now: Timestamp) -> Result<String, String> {
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, now).map_err(|e| e.to_string())?;
    let wealthsimple = Broker::named("wealthsimple");
    let connection = match book.connections().map_err(|e| e.to_string())?.into_iter().find(|c| c.broker == wealthsimple) {
        Some(c) => c.id,
        None => book.add_connection(&wealthsimple, "Wealthsimple", now).map_err(|e| e.to_string())?,
    };
    let today = adapter.day(now);
    let r = pull(&book, adapter, connection, today, now, &mut |s| eprintln!("{s:?}")).map_err(|e| e.to_string())?;
    let mut out = String::new();
    use std::fmt::Write as _;
    writeln!(out, "accounts added {}, linked {}", r.accounts_added, r.accounts_linked).expect("writing to a String cannot fail");
    writeln!(out, "rows read {}: records new {}, revised {}, unchanged {}; no longer listed, removed {}", r.rows_read, r.records_new, r.records_revised, r.records_unchanged, r.removed.len()).expect("writing to a String cannot fail");
    for (record, key) in &r.removed {
        writeln!(out, "  removed {key} (record {record})").expect("writing to a String cannot fail");
    }
    for (account, why) in &r.suspect {
        writeln!(out, "suspect read of {account}, nothing removed: {why}").expect("writing to a String cannot fail");
    }
    writeln!(out, "moves of holdings linked {}", r.transfers_linked).expect("writing to a String cannot fail");
    writeln!(out, "account days stored {}, restated {}", r.days_stored, r.days_restated).expect("writing to a String cannot fail");
    writeln!(out, "requests {}", adapter.source.requests()).expect("writing to a String cannot fail");
    for (part, f) in &r.failures {
        writeln!(out, "failed: {part}: {f}").expect("writing to a String cannot fail");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pull over the network takes the data folder as the app does: beside an
    /// app using the folder it is refused, before any sign-in is read or
    /// refreshed (brief 71, C13).
    #[test]
    fn a_pull_beside_the_app_on_its_folder_is_refused() {
        let home = tempfile::tempdir().unwrap();
        // the app's hold on the folder: a lock of its own on the folder's lock file
        let app = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(home.path().join(crate::app::HOME_LOCK_FILE)).unwrap();
        app.try_lock().unwrap();
        let e = run(home.path(), None, "2026-10-20T12:00:00Z".parse().unwrap()).unwrap_err();
        assert!(e.contains("another Bagholder is using the data folder"), "{e}");
    }

    /// The statements walk alone asks nobody and names nothing: on a book whose
    /// account agrees with what was last stated, it counts nothing done.
    #[test]
    fn the_statements_walk_alone_asks_nobody_and_names_nothing() {
        use bagholder_core::account::{AccountRef, AccountStatus};
        let home = tempfile::tempdir().unwrap();
        let now: Timestamp = "2026-10-20T12:00:00Z".parse().unwrap();
        {
            let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, now).unwrap();
            let ws = Broker::named("wealthsimple");
            let conn = book.add_connection(&ws, "Wealthsimple", now).unwrap();
            book.add_account(conn, &[AccountRef::new(ws, "acct-1")], &bagholder_wealthsimple::account_types::account_type("CASH"), AccountStatus::Open, None, now).unwrap();
        }
        let out = walk(home.path(), now).unwrap();
        assert_eq!(out, "statements walked: withdrawn 0, booked 0, joined 0, unreconciled 0, feed only 0, failures 0\n");
    }
}
