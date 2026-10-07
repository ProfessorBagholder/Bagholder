//! Where the app's lines go. A line the app says while it runs (a read, a failure
//! a person sees on the page anyway, a step of a pull) belongs in its log, not on
//! the terminal it was started from: the running server sets the sink to its log
//! file (`server::logfile`). With no sink set, as in a command-line tool or a test,
//! a line goes to stderr.

use std::sync::OnceLock;

static SINK: OnceLock<fn(&str)> = OnceLock::new();

/// Where every line goes from now on. Set once, when the server starts; a second
/// setting is refused, and the first stands.
pub fn set_sink(sink: fn(&str)) -> bool {
    SINK.set(sink).is_ok()
}

/// One line said by the app: to the sink, else to stderr.
pub fn line(line: &str) {
    match SINK.get() {
        Some(sink) => sink(line),
        None => eprintln!("{line}"),
    }
}
