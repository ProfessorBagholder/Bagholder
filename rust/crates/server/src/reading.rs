//! What the page says about reading Wealthsimple (brief 79, amended 2026-10-28;
//! `SPEC.md` pages-11, pages-13): one plain sentence, and only when Wealthsimple
//! as a whole cannot be read. A part of a read that fails (one account's history,
//! the holdings, buying power) is asked again at its own next turn and goes to the
//! log, once when it starts failing and once when it answers again: the page
//! names no step, no account id and no reason. The owner, 2026-10-28: "stop
//! fixing patches … you designed this wrong".

use std::collections::BTreeMap;

use bagholder_broker::Failure;
use bagholder_core::jiff::Timestamp;

use crate::app::log;

/// The page's sentence when Wealthsimple cannot be reached at all.
pub const UNREACHABLE: &str = "Wealthsimple couldn't be reached.";
/// The page's sentence when Wealthsimple answers and cannot be read: it refused,
/// or answered in a shape not read.
pub const UNREADABLE: &str = "Wealthsimple couldn't be read.";

/// The page's sentence when what was read could not be kept: the book on this
/// machine failed, not Wealthsimple.
pub const NOT_KEPT: &str = "What Wealthsimple answered could not be kept on this machine.";

/// The page's sentence for a pass's failures: one only where the accounts read,
/// which every pass makes first and every other read hangs on, failed; none where
/// a part alone did.
pub fn whole(failures: &[(String, Failure)]) -> Option<&'static str> {
    failures.iter().find(|(part, _)| part == "accounts").map(|(_, f)| match f {
        Failure::Unreachable(_) => UNREACHABLE,
        _ => UNREADABLE,
    })
}

/// The parts of one kind of pass failing now, by part, with why: each logged once
/// when it starts failing and once when a pass of that kind meets no failure of
/// it.
#[derive(Default)]
pub struct Failing(BTreeMap<String, String>);

impl Failing {
    /// A pass of `what` met `failures`: what started failing, and what answers
    /// again, logged.
    pub fn pass(&mut self, what: &str, failures: impl IntoIterator<Item = (String, String)>) {
        let now: BTreeMap<String, String> = failures.into_iter().collect();
        for (part, why) in &now {
            if !self.0.contains_key(part) {
                log(&if why.is_empty() { format!("bagholder: {what}: {part}") } else { format!("bagholder: {what}: {part} could not be read: {why}") });
            }
        }
        for part in self.0.keys().filter(|p| !now.contains_key(*p)) {
            log(&format!("bagholder: {what}: {part} answers again"));
        }
        self.0 = now;
    }

    /// Whether any part is failing now.
    pub fn any(&self) -> bool {
        !self.0.is_empty()
    }
}

/// What a pull's failures leave the page: Wealthsimple's one sentence where it
/// could not be read at all (said from the second pull in a row, `Confirmed`),
/// else nothing; "Synced" moved to `now` where it could be read; each part of a
/// day's pull failing logged as it starts and as it ends. The sentence said, if any.
pub fn after_pull(st: &mut crate::app::State, failures: &[(String, Failure)], daily: bool, now: Timestamp) -> Option<&'static str> {
    let whole = whole(failures);
    if whole.is_none() {
        st.last_sync = now.to_string();
    }
    if daily {
        st.pull_failing.pass("the day's pull", parts(failures));
        if failures.is_empty() {
            st.pull_answered = Some(now);
        }
    }
    st.sync_problem.check(whole.map(String::from));
    st.error = st.sync_problem.said().unwrap_or_default();
    whole
}

/// What a balances read's failures leave the page, as `after_pull`.
pub fn after_balances(st: &mut crate::app::State, failures: &[(String, Failure)], now: Timestamp) {
    st.balances_failing.pass("the balances", parts(failures));
    if failures.is_empty() {
        st.balances_answered = Some(now);
    }
    st.balances_problem.check(whole(failures).map(String::from));
    st.portfolio_error = st.balances_problem.said().unwrap_or_default();
}

/// A Wealthsimple pass's failures, as the log takes them.
pub fn parts(failures: &[(String, Failure)]) -> Vec<(String, String)> {
    failures.iter().map(|(p, f)| (p.clone(), f.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_accounts_read_failing_is_wealthsimple_as_a_whole() {
        let part = |p: &str, f: Failure| (p.to_string(), f);
        assert_eq!(whole(&[part("history:a-1", Failure::Refused("no".into())), part("holdings", Failure::Unreachable("down".into()))]), None, "parts failing: nothing for the page");
        assert_eq!(whole(&[part("accounts", Failure::Unreachable("down".into()))]), Some(UNREACHABLE));
        assert_eq!(whole(&[part("accounts", Failure::Refused("401".into()))]), Some(UNREADABLE));
        assert_eq!(whole(&[part("accounts", Failure::Mismatch("a field".into()))]), Some(UNREADABLE));
    }

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn header(st: &crate::app::State) -> String {
        crate::status::failures(st, &[])
    }

    /// Every part of a pull and of the balances failing, with the session holding,
    /// says nothing on the page, however often; the accounts read refused says the
    /// one sentence, from the second pass in a row, and nothing else.
    #[test]
    fn parts_failing_say_nothing_and_wealthsimple_unread_says_one_plain_sentence() {
        let mut st = crate::app::State::default();
        let every_part: Vec<(String, Failure)> = ["cash", "buying power", "holdings", "outside accounts", "history:anon-tfsa-1", "margin rate:anon-margin-1", "units:anon-tfsa-1", "the balance by day of a line of credit", "activity:anon-tfsa-1"]
            .into_iter()
            .map(|p| (p.to_string(), Failure::Refused("HTTP 500".into())))
            .collect();
        for n in 0..3 {
            let now = at(&format!("2026-10-28T1{n}:00:00Z"));
            after_pull(&mut st, &every_part, true, now);
            after_balances(&mut st, &every_part, now);
            assert_eq!(header(&st), "", "pass {n}: a part failing is no sentence of the page's");
        }
        let refused = vec![("accounts".to_string(), Failure::Refused("401".into()))];
        after_pull(&mut st, &refused, true, at("2026-10-28T14:00:00Z"));
        assert_eq!(header(&st), "", "one pass is not said");
        after_pull(&mut st, &refused, true, at("2026-10-28T15:00:00Z"));
        after_balances(&mut st, &refused, at("2026-10-28T15:00:00Z"));
        after_balances(&mut st, &refused, at("2026-10-28T15:05:00Z"));
        assert_eq!(header(&st), UNREADABLE, "the one sentence, said once whichever read met it");
        let header_now = header(&st);
        assert!(!header_now.contains("accounts") && !header_now.contains("401") && !header_now.contains("anon"), "no step, id or reason");
        after_pull(&mut st, &[], true, at("2026-10-28T16:00:00Z"));
        after_balances(&mut st, &[], at("2026-10-28T16:00:00Z"));
        assert_eq!(header(&st), "");
    }

    /// "Synced" is the last pull's time while every read answers at its turn,
    /// whatever reads are made only once a day; a read whose figures are shown
    /// failing at its turn since shows that read's last answer.
    #[test]
    fn synced_ages_only_where_a_shown_read_failed_at_its_turn() {
        let mut st = crate::app::State::default();
        after_pull(&mut st, &[], true, at("2026-10-28T08:00:00Z"));
        for m in [5, 10, 15] {
            after_balances(&mut st, &[], at(&format!("2026-10-28T08:{m:02}:00Z")));
        }
        assert_eq!(crate::status::synced(&st), "2026-10-28T08:00:00Z", "every read answering: the pull's time");
        let failed = vec![("holdings".to_string(), Failure::Unreachable("down".into()))];
        after_balances(&mut st, &failed, at("2026-10-28T08:20:00Z"));
        assert_eq!(crate::status::synced(&st), "2026-10-28T08:00:00Z", "the pull's is older than the balances' last answer");
        after_pull(&mut st, &[], true, at("2026-10-28T09:00:00Z"));
        after_balances(&mut st, &failed, at("2026-10-28T09:05:00Z"));
        assert_eq!(crate::status::synced(&st), "2026-10-28T08:15:00Z", "the balances' last answer");
        after_balances(&mut st, &[], at("2026-10-28T09:10:00Z"));
        assert_eq!(crate::status::synced(&st), "2026-10-28T09:00:00Z", "answering again: the pull's time");
    }

    /// Every sentence the page is given about Wealthsimple is one of the two plain
    /// ones: a scan of the server's code for every write to them names no step,
    /// id or reason (`format!` builds none of them).
    #[test]
    fn the_page_is_given_only_the_plain_sentences_about_wealthsimple() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut writes = 0;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().map(Result::unwrap) {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if !name.ends_with(".rs") || name.starts_with("tests_") || name == "reading.rs" {
                    continue;
                }
                for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                    let code = line.split("//").next().unwrap_or_default();
                    if code.contains("sync_problem.check(") || code.contains("balances_problem.check(") {
                        writes += 1;
                        assert!(!code.contains("format!") && (code.contains("reading::") || code.contains("whole")), "{name}:{} gives the page a sentence of its own: {}", n + 1, code.trim());
                    }
                }
            }
        }
        assert!(writes >= 1, "the scan reads the writes");
    }

    #[test]
    fn a_part_is_logged_as_it_starts_failing_and_as_it_answers_again() {
        let mut f = Failing::default();
        f.pass("the balances", vec![("holdings".into(), "refused".into())]);
        assert!(f.any());
        f.pass("the balances", vec![("holdings".into(), "refused".into())]);
        f.pass("the balances", vec![]);
        assert!(!f.any());
    }
}
