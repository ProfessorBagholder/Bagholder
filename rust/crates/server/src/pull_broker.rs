//! `bagholder pull-broker <book folder> --replay <capture> [--now <instant>]`
//! (`docs/plans/stage-3b-wealthsimple.md`, "The pull"): Wealthsimple pulled into
//! the book, from a capture of its replies. The network's client answers the
//! same interface; the pull and the records are the same either way.

use std::path::PathBuf;

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_broker::BrokerAdapter;
use bagholder_core::jiff::Timestamp;
use bagholder_core::Broker;
use bagholder_wealthsimple::replay::Replay;

pub fn cli(args: &[String]) -> i32 {
    let usage = "usage: bagholder pull-broker <book folder> --replay <capture folder> [--now <instant>]";
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
    let (Some(home), Some(capture)) = (positional.first(), replay) else {
        eprintln!("{usage}");
        return 2;
    };
    let now = now.unwrap_or_else(Timestamp::now);
    match run(&PathBuf::from(home), &PathBuf::from(capture), now) {
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

fn run(home: &std::path::Path, capture: &std::path::Path, now: Timestamp) -> Result<String, String> {
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, now).map_err(|e| e.to_string())?;
    let mut adapter = Replay::read(capture).map_err(|e| format!("{}: {e}", capture.display()))?;
    let wealthsimple = Broker::named("wealthsimple");
    let connection = match book.connections().map_err(|e| e.to_string())?.into_iter().find(|c| c.broker == wealthsimple) {
        Some(c) => c.id,
        None => book.add_connection(&wealthsimple, "Wealthsimple", now).map_err(|e| e.to_string())?,
    };
    let today = adapter.day(now);
    let r = pull(&book, &mut adapter, connection, today, now).map_err(|e| e.to_string())?;
    let mut out = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(out, "accounts added {}, linked {}", r.accounts_added, r.accounts_linked);
    let _ = writeln!(out, "rows read {}: records new {}, revised {}, unchanged {}; imported records replaced {}", r.rows_read, r.records_new, r.records_revised, r.records_unchanged, r.superseded);
    let _ = writeln!(out, "moves of holdings linked {}", r.transfers_linked);
    let _ = writeln!(out, "account days stored {}, restated {}", r.days_stored, r.days_restated);
    let _ = writeln!(out, "requests {}", adapter.asked.len());
    for (part, f) in &r.failures {
        let _ = writeln!(out, "failed: {part}: {f}");
    }
    Ok(out)
}
