//! `bagholder pull-broker <book folder> [--replay <capture>] [--now <instant>]`
//! (`docs/plans/stage-3b-wealthsimple.md`, "The pull"): Wealthsimple pulled into
//! the book, over the network with the folder's saved sign-in, or from a capture
//! of its replies. The pull and the records are the same either way.

use std::path::PathBuf;

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_broker::BrokerAdapter;
use bagholder_core::jiff::Timestamp;
use bagholder_core::Broker;
use bagholder_wealthsimple::adapter::{Source, Wealthsimple};
use bagholder_wealthsimple::replay::Replay;

pub fn cli(args: &[String]) -> i32 {
    let usage = "usage: bagholder pull-broker <book folder> [--replay <capture folder>] [--now <instant>]";
    let mut positional = Vec::new();
    let mut replay = None;
    let mut now = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--replay" => replay = it.next().cloned(),
            "--now" => now = it.next().and_then(|t| t.parse::<Timestamp>().ok()),
            _ => positional.push(a.clone()),
        }
    }
    let Some(home) = positional.first() else {
        eprintln!("{usage}");
        return 2;
    };
    let now = now.unwrap_or_else(Timestamp::now);
    match run(&PathBuf::from(home), replay.map(PathBuf::from).as_deref(), now) {
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
            let net = bagholder_net::Net::new(std::sync::Arc::new(bagholder_net::SystemClock), std::sync::Arc::new(bagholder_net::Limiter::new()));
            let file = bagholder_wealthsimple::session::SessionFile { path: home.join("session.json") };
            let mut adapter = Wealthsimple::new(bagholder_wealthsimple::client::Client::new(&net, file));
            pull_with(home, &mut adapter, now)
        }
    }
}

fn pull_with<S: Source>(home: &std::path::Path, adapter: &mut Wealthsimple<S>, now: Timestamp) -> Result<String, String> {
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, now).map_err(|e| e.to_string())?;
    let wealthsimple = Broker::named("wealthsimple");
    let connection = match book.connections().map_err(|e| e.to_string())?.into_iter().find(|c| c.broker == wealthsimple) {
        Some(c) => c.id,
        None => book.add_connection(&wealthsimple, "Wealthsimple", now).map_err(|e| e.to_string())?,
    };
    let today = adapter.day(now);
    let r = pull(&book, adapter, connection, today, now).map_err(|e| e.to_string())?;
    let mut out = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(out, "accounts added {}, linked {}", r.accounts_added, r.accounts_linked);
    let _ = writeln!(out, "rows read {}: records new {}, revised {}, unchanged {}; imported records replaced {}; no longer listed, removed {}", r.rows_read, r.records_new, r.records_revised, r.records_unchanged, r.superseded, r.removed.len());
    for (record, key) in &r.removed {
        let _ = writeln!(out, "  removed {key} (record {record})");
    }
    for (account, why) in &r.suspect {
        let _ = writeln!(out, "suspect read of {account}, nothing removed: {why}");
    }
    let _ = writeln!(out, "moves of holdings linked {}", r.transfers_linked);
    let _ = writeln!(out, "account days stored {}, restated {}", r.days_stored, r.days_restated);
    let _ = writeln!(out, "requests {}", adapter.source.requests());
    for (part, f) in &r.failures {
        let _ = writeln!(out, "failed: {part}: {f}");
    }
    Ok(out)
}
