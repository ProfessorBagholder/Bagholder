//! The news types: every `Feed` round-trips through its own name and through
//! an id's prefix, and a stored item reads back as the item its feed answered.

use bagholder_store::feeds::{self, Feed, NewsItem, NewsKind};
use bagholder_store::{open_db, relabel};

fn db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    (dir, conn)
}

const ALL: [Feed; 7] = [Feed::Tmx, Feed::TmxMedia, Feed::Nasdaq, Feed::NasdaqPress, Feed::Yahoo, Feed::Sa, Feed::Gnews];

#[test]
fn test_every_feed_parses_back_to_itself() {
    for f in ALL {
        assert_eq!(Feed::parse(f.as_str()), Some(f), "{}", f.as_str());
    }
    assert_eq!(Feed::parse("nonsense"), None);
}

#[test]
fn test_of_id_reads_the_feed_a_prefix_names() {
    assert_eq!(Feed::of_id("tmx:10"), Some(Feed::Tmx));
    assert_eq!(Feed::of_id("nasdaq:1"), Some(Feed::Nasdaq));
    assert_eq!(Feed::of_id("yahoo:a1"), Some(Feed::Yahoo));
    assert_eq!(Feed::of_id("sa:1"), Some(Feed::Sa));
    assert_eq!(Feed::of_id("gnews:1"), Some(Feed::Gnews));
    assert_eq!(Feed::of_id("unknown:1"), None);
    assert_eq!(Feed::of_id("nothing-here"), None);
}

#[test]
fn test_a_replaced_item_reads_back_the_same_through_its_feed() {
    let (_d, conn) = db();
    let item = NewsItem {
        id: "tmx:10".into(),
        headline: "Kraken Closes $40M Financing Round".into(),
        source: "GlobeNewswire".into(),
        url: "https://money.tmx.com/en/quote/PNG/news/10".into(),
        published_at: "2026-09-05T12:00:00Z".into(),
        summary: "Kraken Robotics closed its bought deal.".into(),
        kind: NewsKind::Release,
        via: Feed::Tmx,
    };
    feeds::replace_news(&conn, "PNG", "TSX-V", &[item.clone()], "2026-09-05T12:05:00Z").unwrap();
    let stored = feeds::news_for(&conn, "PNG", "TSX-V").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].feed, Some(Feed::Tmx));
    assert_eq!(stored[0].item(), Some(item));
}
