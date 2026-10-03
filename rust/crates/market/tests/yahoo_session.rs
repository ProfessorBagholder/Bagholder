//! Yahoo answers its crumb and its `quoteSummary` statistics only to a session that
//! opens with a browser's handshake (`shorts::yahoo_quote_summary`); a plain client
//! is turned away with a 429. Every reader of them goes through that one session:
//! no other file of the crate names those endpoints.

#[test]
fn every_yahoo_statistics_read_goes_through_the_browser_session() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut naming = Vec::new();
    for e in std::fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "rs") {
            let text = std::fs::read_to_string(&p).unwrap();
            if text.contains("finance/quoteSummary") || text.contains("v1/test/getcrumb") {
                naming.push(p.file_name().unwrap().to_string_lossy().to_string());
            }
        }
    }
    assert_eq!(naming, vec!["shorts.rs".to_string()], "only the session's own file asks Yahoo for these");
}
