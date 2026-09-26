//! The News card (`SPEC.md` §4 Markets, News; `docs/plans/stage-5-interface-and-running.md`,
//! A2): the headlines read for the market and for each listing held or watched,
//! one story per row, each tagged with the listings it was read for, filtered,
//! sorted and sent as far as the card has scrolled.
//!
//! The rows the sources wrote, and the issuers' filed releases, are asked
//! through [`super::markets::Tables`]; nothing here reads a store.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use bagholder_sources::venue;

/// The feed whose items belong to the market rather than to a listing.
const MARKET_FEED: (&str, &str) = ("*", "MARKET");

/// A headline as a source's reader stored it, for one listing (or the market).
#[derive(Clone, Debug, PartialEq)]
pub struct NewsRow {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    pub headline: String,
    /// The wire that carried it: the source the card names.
    pub wire: String,
    pub url: String,
    pub published_at: String,
    /// `story`, or `release` for a company's own; empty is a story.
    pub kind: String,
}

/// A news release an issuer filed with its regulator, as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct FiledRelease {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    /// Its title, once read; empty until then.
    pub subject: String,
    /// The document's kind (`News release`).
    pub form: String,
    /// The regulator it was filed with (`SEDAR+`, `SEC`).
    pub source: String,
    pub url: String,
    /// The day it was filed.
    pub date: String,
    /// Its title is still being read.
    pub pending: bool,
}

// --------------------------------------------------------------------------
// the wire
// --------------------------------------------------------------------------

/// A listing a headline was read for.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = symbol)]
#[serde(rename_all = "camelCase")]
pub struct NewsTag {
    /// Its bare ticker.
    pub symbol: String,
    pub exchange: String,
    pub held: bool,
    pub watched: bool,
    /// The listing's day change, as a fraction: the holding's where the book
    /// holds it, the watched listing's otherwise.
    pub percent_change: Option<f64>,
    /// The holding it is, where the book holds it.
    pub position_id: Option<String>,
}

/// What a filed release is, for opening it.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Filed {
    pub id: String,
    pub symbol: String,
    pub source: String,
    pub url: String,
    /// Its title is still being read.
    pub pending: bool,
}

/// One story.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
#[serde(rename_all = "camelCase")]
pub struct Headline {
    pub id: String,
    pub headline: String,
    /// The wire or publisher, or the regulator for a filed release.
    pub source: String,
    pub url: String,
    pub published_at: String,
    /// From the market's own feed rather than a listing's.
    pub market: bool,
    /// `story`, or `release` for a company's own.
    pub kind: String,
    pub tags: Vec<NewsTag>,
    /// A release as the issuer filed it, beside the wires'.
    pub filed: Option<Filed>,
}

/// How many of a listing's items each tab holds: a chip takes the tab that has
/// its items.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
pub struct ChipKinds {
    pub stories: usize,
    pub releases: usize,
}

/// The News card's list: as far as it has scrolled, with how many there are.
#[derive(Clone, Debug, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct HeadlinesDoc {
    pub items: Vec<Headline>,
    pub total: usize,
    /// Under a chip: how many of its items each tab holds.
    pub chip: Option<ChipKinds>,
    /// Why the issuers' filed releases are not on the Releases tab, when they
    /// could not be read.
    pub filed_failed: Option<String>,
}

/// What the card shows: its scope, its tab, its chip, what is typed in its box.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct Shown {
    /// `all`, `holdings` or `watchlist`.
    pub scope: String,
    /// `stories` or `releases`.
    pub kind: String,
    /// The chip's listing: its bare ticker and venue.
    pub symbol: Option<String>,
    pub exchange: Option<String>,
    pub query: String,
}

// --------------------------------------------------------------------------
// one story per row
// --------------------------------------------------------------------------

/// A headline as one story: letters and digits only, one case, one space
/// between words.
pub fn text_key(headline: &str) -> String {
    headline.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

const FRENCH_WORDS: [&str; 21] = [
    "annonce", "annoncent", "ses", "du", "des", "une", "pour", "avec", "sur", "résultats", "clôture",
    "croissance", "les", "et", "au", "aux", "dans", "son", "sa", "le", "la",
];
const FRENCH_LETTERS: [char; 15] = ['à', 'â', 'ç', 'é', 'è', 'ê', 'ë', 'î', 'ï', 'ô', 'û', 'ù', 'ü', 'ÿ', 'œ'];

/// Accented letters or French function words, two or more.
pub fn looks_french(headline: &str) -> bool {
    let t = headline.to_lowercase();
    if t.chars().filter(|c| FRENCH_LETTERS.contains(c)).count() >= 2 {
        return true;
    }
    t.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| FRENCH_WORDS.contains(w)).count() >= 2
}

/// Minutes since the epoch of an instant, for the three hours a translation
/// follows its original within; one stated without its offset is UTC's.
fn minutes(iso: &str) -> Option<i64> {
    let t = iso.trim();
    t.parse::<bagholder_core::jiff::Timestamp>().or_else(|_| format!("{t}Z").parse::<bagholder_core::jiff::Timestamp>()).ok().map(|t| t.as_second() / 60)
}

/// A listing as one listing whether the book names it `QNC.TO` or the watchlist
/// `QNC`: its bare ticker and its venue.
pub fn listing_key(symbol: &str, exchange: &str) -> (String, String) {
    (venue::root(symbol), exchange.trim().to_uppercase())
}

/// One story, and the listings it was read for.
#[derive(Clone, Debug, PartialEq)]
pub struct Story {
    pub id: String,
    pub headline: String,
    pub source: String,
    pub url: String,
    pub published_at: String,
    pub market: bool,
    pub kind: String,
    /// Each listing's key and its venue as the source wrote it.
    pub tags: Vec<((String, String), String)>,
    /// Every wire that carried it, its own first.
    pub wires: Vec<String>,
}

/// Every item kept, newest first, one story per row: the same wire id, or the
/// same headline under another id (a release carried by several wires, a story
/// republished per symbol, an update), its listings merged; a release posted in
/// French beside its English original (the same wire, a listing in common,
/// within three hours) is one story, the English row kept.
pub fn stories(rows: &[NewsRow]) -> Vec<Story> {
    let mut items: Vec<&NewsRow> = rows.iter().collect();
    items.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    let mut out: Vec<Story> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut by_text: HashMap<String, usize> = HashMap::new();
    for n in items {
        let is_market = n.symbol == MARKET_FEED.0 && n.exchange.eq_ignore_ascii_case(MARKET_FEED.1);
        let key = listing_key(&n.symbol, &n.exchange);
        let text = text_key(&n.headline);
        let known = by_id.get(&n.id).copied().or_else(|| if text.is_empty() { None } else { by_text.get(&text).copied() });
        if let Some(i) = known {
            if is_market {
                out[i].market = true;
            } else if !out[i].tags.iter().any(|(k, _)| *k == key) {
                out[i].tags.push((key, n.exchange.clone()));
            }
            // the same text on a wire and in a publisher's column is the release
            if n.kind == "release" {
                out[i].kind = "release".into();
            }
            if !out[i].wires.contains(&n.wire) {
                out[i].wires.push(n.wire.clone());
            }
            by_id.entry(n.id.clone()).or_insert(i);
            continue;
        }
        out.push(Story {
            id: n.id.clone(),
            headline: n.headline.clone(),
            source: n.wire.clone(),
            url: n.url.clone(),
            published_at: n.published_at.clone(),
            market: is_market,
            kind: if n.kind.is_empty() { "story".into() } else { n.kind.clone() },
            tags: if is_market { vec![] } else { vec![(key, n.exchange.clone())] },
            wires: vec![n.wire.clone()],
        });
        let i = out.len() - 1;
        by_id.insert(n.id.clone(), i);
        if !text.is_empty() {
            by_text.entry(text).or_insert(i);
        }
    }
    out.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    // the French twin of an English release
    let french: Vec<bool> = out.iter().map(|r| looks_french(&r.headline)).collect();
    let when: Vec<Option<i64>> = out.iter().map(|r| minutes(&r.published_at)).collect();
    let twin = |i: usize| {
        out.iter().enumerate().any(|(j, other)| {
            j != i && !french[j] && out[i].wires.iter().any(|w| other.wires.contains(w)) && other.tags.iter().any(|(k, _)| out[i].tags.iter().any(|(x, _)| x == k)) && matches!((when[i], when[j]), (Some(a), Some(b)) if (b - a).abs() <= 180)
        })
    };
    let drop: Vec<bool> = (0..out.len()).map(|i| french[i] && twin(i)).collect();
    out.into_iter().zip(drop).filter(|(_, d)| !d).map(|(s, _)| s).collect()
}

// --------------------------------------------------------------------------
// the card's list
// --------------------------------------------------------------------------

/// What the card knows of a listing: whether it is held (and as which holding)
/// or watched, and its day change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known {
    pub held: Option<(String, Option<f64>)>,
    pub watched: Option<Option<f64>>,
    /// Its venue as the book or the watchlist names it.
    pub exchange: String,
}

fn tag(key: &(String, String), exchange: &str, known: &HashMap<(String, String), Known>) -> NewsTag {
    let k = known.get(key);
    let held = k.and_then(|k| k.held.clone());
    let watched = k.and_then(|k| k.watched);
    NewsTag {
        symbol: key.0.clone(),
        exchange: exchange.to_string(),
        held: held.is_some(),
        watched: watched.is_some(),
        percent_change: match (&held, watched) {
            (Some((_, c)), _) => *c,
            (None, Some(c)) => c,
            (None, None) => None,
        },
        position_id: held.map(|(id, _)| id),
    }
}

/// How the card sorts, as its header names the column.
#[derive(Clone, Debug, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct NewsSort {
    pub key: String,
    pub dir: crate::views::Dir,
}

fn cmp_opt<T: PartialOrd>(a: &Option<T>, b: &Option<T>, desc: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a, b) {
        (None, None) => Equal,
        // nothing sinks, whichever the direction
        (None, _) => Greater,
        (_, None) => Less,
        (Some(x), Some(y)) => {
            let c = x.partial_cmp(y).unwrap_or(Equal);
            if desc { c.reverse() } else { c }
        }
    }
}

/// The card's list for what it shows, in its order: every item, not yet cut to
/// the window.
pub fn headlines(stories: &[Story], filed: &[FiledRelease], known: &HashMap<(String, String), Known>, shown: &Shown, sort: &NewsSort) -> (Vec<Headline>, Option<ChipKinds>) {
    let releases = shown.kind == "releases";
    let chip = shown.symbol.as_ref().map(|s| listing_key(s, shown.exchange.as_deref().unwrap_or_default()));
    let mine = |s: &Story| chip.as_ref().is_some_and(|c| s.tags.iter().any(|(k, _)| k == c));
    let chip_kinds = chip.as_ref().map(|_| {
        let (mut stories_n, mut releases_n) = (0, 0);
        for s in stories.iter().filter(|s| mine(s)) {
            if s.kind == "release" { releases_n += 1 } else { stories_n += 1 }
        }
        ChipKinds { stories: stories_n, releases: releases_n }
    });
    let held_or_watched = |s: &Story, held: bool| s.tags.iter().any(|(k, _)| known.get(k).is_some_and(|x| if held { x.held.is_some() } else { x.watched.is_some() }));
    let mut rows: Vec<Headline> = stories
        .iter()
        .filter(|s| (s.kind == "release") == releases)
        .filter(|s| match (&chip, shown.scope.as_str()) {
            (Some(_), _) => mine(s),
            (None, "holdings") => held_or_watched(s, true),
            (None, "watchlist") => held_or_watched(s, false),
            // `All`: the market's feed; its releases, every listing's
            (None, _) => if releases { !s.tags.is_empty() } else { s.market },
        })
        .map(|s| Headline {
            id: s.id.clone(),
            headline: s.headline.clone(),
            source: s.source.clone(),
            url: s.url.clone(),
            published_at: s.published_at.clone(),
            market: s.market,
            kind: s.kind.clone(),
            tags: s.tags.iter().map(|(k, ex)| tag(k, ex, known)).collect(),
            filed: None,
        })
        .collect();
    if releases {
        // the issuer's own, where no wire's copy is on the list
        let said: BTreeSet<String> = rows.iter().map(|r| text_key(&r.headline)).collect();
        for f in filed {
            if said.contains(&text_key(&f.subject)) && !f.subject.is_empty() {
                continue;
            }
            let key = listing_key(&f.symbol, &f.exchange);
            let exchange = if f.exchange.is_empty() { known.get(&key).map(|k| k.exchange.clone()).unwrap_or_default() } else { f.exchange.clone() };
            rows.push(Headline {
                id: format!("filed:{}", f.id),
                headline: if !f.subject.is_empty() { f.subject.clone() } else if !f.form.is_empty() { f.form.clone() } else { "News release".into() },
                source: f.source.clone(),
                url: f.url.clone(),
                published_at: f.date.clone(),
                market: false,
                kind: "release".into(),
                tags: vec![tag(&key, &exchange, known)],
                filed: Some(Filed { id: f.id.clone(), symbol: key.0.clone(), source: f.source.clone(), url: f.url.clone(), pending: f.pending }),
            });
        }
    }
    // the words typed: a short ticker is a listing's items, else every word in
    // the headline or its source
    let q = shown.query.trim().to_uppercase();
    if !q.is_empty() {
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let by_symbol: Vec<Headline> = if tokens.len() == 1 && q.chars().count() <= 5 { rows.iter().filter(|r| r.tags.iter().any(|t| t.symbol.starts_with(&q))).cloned().collect() } else { vec![] };
        rows = if !by_symbol.is_empty() {
            by_symbol
        } else {
            rows.into_iter().filter(|r| {
                let h = format!("{} {}", r.headline, r.source).to_uppercase();
                tokens.iter().all(|t| h.contains(t))
            }).collect()
        };
    }
    // `All` stories name no listing: only the time and the headline sort them
    let bare = shown.scope == "all" && chip.is_none() && !releases;
    let key = if bare && sort.key != "when" && sort.key != "news" { "when" } else { sort.key.as_str() };
    let desc = if bare && key != sort.key.as_str() { true } else { sort.dir == crate::views::Dir::Desc };
    let first = |r: &Headline| r.tags.first().cloned();
    rows.sort_by(|a, b| match key {
        "news" => cmp_opt(&Some(a.headline.to_lowercase()), &Some(b.headline.to_lowercase()), desc),
        "symbol" => cmp_opt(&first(a).map(|t| t.symbol.to_lowercase()), &first(b).map(|t| t.symbol.to_lowercase()), desc),
        "change" => cmp_opt(&first(a).and_then(|t| t.percent_change), &first(b).and_then(|t| t.percent_change), desc),
        _ => cmp_opt(&Some(a.published_at.clone()), &Some(b.published_at.clone()), desc),
    });
    (rows, chip_kinds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, symbol: &str, exchange: &str, headline: &str, wire: &str, at: &str, kind: &str) -> NewsRow {
        NewsRow { id: id.into(), symbol: symbol.into(), exchange: exchange.into(), headline: headline.into(), wire: wire.into(), url: String::new(), published_at: at.into(), kind: kind.into() }
    }

    #[test]
    fn one_story_is_one_row_its_listings_merged_and_a_french_twin_dropped() {
        let rows = vec![
            row("a", "QNC", "TSX-V", "Quantum eMotion closes financing", "Newsfile", "2026-09-20T12:00:00Z", "release"),
            // the same wire id read for another listing
            row("a", "QNC.TO", "TSX", "Quantum eMotion closes financing", "Newsfile", "2026-09-20T12:00:00Z", "release"),
            // the same headline under another id
            row("b", "PNG", "TSX-V", "Quantum eMotion Closes Financing!", "Globe", "2026-09-20T12:05:00Z", ""),
            // its French twin, an hour later on the same wire
            row("c", "QNC", "TSX-V", "Quantum eMotion annonce la clôture du financement", "Newsfile", "2026-09-20T13:00:00Z", "release"),
            row("m", "*", "MARKET", "Stocks rise", "Nasdaq", "2026-09-20T14:00:00Z", ""),
        ];
        let s = stories(&rows);
        assert_eq!(s.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["m", "b"]);
        let merged = &s[1];
        assert_eq!(merged.kind, "release", "the same text on a wire is the release");
        assert_eq!(merged.tags.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(), vec![("PNG".into(), "TSX-V".into()), ("QNC".into(), "TSX-V".into()), ("QNC".into(), "TSX".into())]);
        assert!(s[0].market && s[0].tags.is_empty());
    }

    #[test]
    fn the_card_filters_by_its_scope_tab_chip_and_words_and_sorts_by_its_column() {
        let rows = vec![
            row("1", "AAA", "TSX", "Alpha wins contract", "Globe", "2026-09-20T10:00:00Z", ""),
            row("2", "BBB", "NASDAQ", "Beta files results", "PR Newswire", "2026-09-20T11:00:00Z", "release"),
            row("3", "*", "MARKET", "Markets open higher", "Nasdaq", "2026-09-20T12:00:00Z", ""),
            row("4", "BBB", "NASDAQ", "Beta upgraded", "Zacks", "2026-09-20T13:00:00Z", ""),
        ];
        let s = stories(&rows);
        let mut known = HashMap::new();
        known.insert(("AAA".to_string(), "TSX".to_string()), Known { held: Some(("p1".into(), Some(0.02))), watched: None, exchange: "TSX".into() });
        known.insert(("BBB".to_string(), "NASDAQ".to_string()), Known { held: None, watched: Some(Some(-0.01)), exchange: "NASDAQ".into() });
        let when = NewsSort { key: "when".into(), dir: crate::views::Dir::Desc };
        let ids = |shown: Shown, sort: &NewsSort| headlines(&s, &[], &known, &shown, sort).0.into_iter().map(|h| h.id).collect::<Vec<_>>();
        assert_eq!(ids(Shown { scope: "all".into(), kind: "stories".into(), ..Shown::default() }, &when), vec!["3"]);
        assert_eq!(ids(Shown { scope: "holdings".into(), kind: "stories".into(), ..Shown::default() }, &when), vec!["1"]);
        assert_eq!(ids(Shown { scope: "watchlist".into(), kind: "stories".into(), ..Shown::default() }, &when), vec!["4"]);
        assert_eq!(ids(Shown { scope: "all".into(), kind: "releases".into(), ..Shown::default() }, &when), vec!["2"]);
        assert_eq!(ids(Shown { scope: "all".into(), kind: "stories".into(), symbol: Some("BBB".into()), exchange: Some("NASDAQ".into()), ..Shown::default() }, &when), vec!["4"]);
        assert_eq!(ids(Shown { scope: "watchlist".into(), kind: "stories".into(), query: "upgraded".into(), ..Shown::default() }, &when), vec!["4"]);
        let (_, chip) = headlines(&s, &[], &known, &Shown { scope: "all".into(), kind: "stories".into(), symbol: Some("BBB".into()), exchange: Some("NASDAQ".into()), ..Shown::default() }, &when);
        assert_eq!(chip, Some(ChipKinds { stories: 1, releases: 1 }));
        // a tag carries the holding's change, else the watched listing's
        let h = headlines(&s, &[], &known, &Shown { scope: "holdings".into(), kind: "stories".into(), ..Shown::default() }, &when).0;
        assert_eq!((h[0].tags[0].percent_change, h[0].tags[0].position_id.as_deref()), (Some(0.02), Some("p1")));
        // `All` stories sort by time or headline only
        let by_change = NewsSort { key: "change".into(), dir: crate::views::Dir::Asc };
        assert_eq!(ids(Shown { scope: "all".into(), kind: "stories".into(), ..Shown::default() }, &by_change), vec!["3"]);
    }

    #[test]
    fn a_filed_release_stands_beside_the_wires_unless_a_wire_carried_it() {
        let rows = vec![row("2", "BBB", "NASDAQ", "Beta files results", "PR Newswire", "2026-09-20T11:00:00Z", "release")];
        let s = stories(&rows);
        let filed = vec![
            FiledRelease { id: "f1".into(), symbol: "BBB".into(), exchange: "NASDAQ".into(), subject: "Beta Files Results".into(), form: "News release".into(), source: "SEC".into(), url: String::new(), date: "2026-09-20".into(), pending: false },
            FiledRelease { id: "f2".into(), symbol: "BBB".into(), exchange: "NASDAQ".into(), subject: String::new(), form: "News release".into(), source: "SEC".into(), url: String::new(), date: "2026-09-21".into(), pending: true },
        ];
        let sort = NewsSort { key: "when".into(), dir: crate::views::Dir::Desc };
        let (h, _) = headlines(&s, &filed, &HashMap::new(), &Shown { scope: "all".into(), kind: "releases".into(), ..Shown::default() }, &sort);
        assert_eq!(h.iter().map(|x| (x.id.as_str(), x.headline.as_str())).collect::<Vec<_>>(), vec![("filed:f2", "News release"), ("2", "Beta files results")]);
        assert!(h[0].filed.as_ref().unwrap().pending);
    }
}
