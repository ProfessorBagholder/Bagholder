//! The recorded replies (`tests/replies/<source>/`): real answers captured from
//! each source, and beside them copies edited by hand, named `wrong-shape-…` and
//! `wrong-meaning-…` for what is wrong. A reply captured with a status other than
//! 200 is named `…-status-<code>…`.

#![allow(dead_code)]

use std::path::PathBuf;

use bagholder_core::json::Value;
use bagholder_sources::{ask, reply};

pub fn dir(source: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies").join(source)
}

pub fn read(source: &str, name: &str) -> String {
    std::fs::read_to_string(dir(source).join(name)).unwrap_or_else(|e| panic!("{source}/{name}: {e}"))
}

pub fn json(source: &str, name: &str) -> Value {
    reply::parse(&read(source, name)).unwrap()
}

/// The real answers of a source whose name starts with `prefix`.
pub fn answers(source: &str, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir(source))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".json") && !n.starts_with("wrong-") && !n.contains("-status-"))
        .collect();
    out.sort();
    out
}

/// The shape committed in `shapes/<file>` is the union of the answers' shapes
/// (`BAGHOLDER_BLESS=1` writes it).
pub fn shape_is_the_answers_union(file: &str, source: &str, prefix: &str, keyed: &[reply::Keyed]) {
    let names = answers(source, prefix);
    assert!(!names.is_empty(), "no recorded answers for {source}/{prefix}");
    let union = reply::union(names.iter().map(|n| reply::shape_keyed(&json(source, n), keyed)));
    let text = ask::shape_text(&union);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shapes").join(file);
    if std::env::var("BAGHOLDER_BLESS").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{} is not the union of the recorded answers", path.display());
}

/// A network that answers each URL it is given with a recorded reply, and
/// fails the test on any other request.
pub struct Recorded {
    pub answers: Vec<(String, u16, Vec<u8>)>,
    pub asked: std::sync::Mutex<Vec<String>>,
}

impl Recorded {
    pub fn new() -> Recorded {
        Recorded { answers: Vec::new(), asked: std::sync::Mutex::new(Vec::new()) }
    }

    /// Answer `url` with the recorded reply `source/name`, captured with `status`.
    pub fn with(mut self, url: &str, status: u16, source: &str, name: &str) -> Recorded {
        self.answers.push((url.to_string(), status, read(source, name).into_bytes()));
        self
    }
}

/// The recorded replies, shared between the network and the test that reads
/// what was asked.
pub struct Shared(pub std::sync::Arc<Recorded>);

impl bagholder_net::Transport for Shared {
    fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
        self.0.asked.lock().unwrap().push(ask.url.to_string());
        match self.0.answers.iter().find(|(u, _, _)| u == ask.url) {
            Some((_, status, body)) => Ok((*status, ask.url.to_string(), vec![], body.clone())),
            None => panic!("a request no recorded reply answers: {}", ask.url),
        }
    }
}

/// A network answered by `recorded`, on a clock set to `at`.
pub fn net(recorded: &std::sync::Arc<Recorded>, at: &str) -> bagholder_net::Net {
    let clock = std::sync::Arc::new(bagholder_net::ManualClock::at(at.parse().unwrap()));
    bagholder_net::Net::answered_by(clock, std::sync::Arc::new(bagholder_net::Limiter::new()), Box::new(Shared(recorded.clone())))
}
