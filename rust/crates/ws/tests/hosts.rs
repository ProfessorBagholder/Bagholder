//! Every address the older client asks is the stand-in's when one is named
//! (`BAGHOLDER_WS_BASE`): its GraphQL, its token calls and the origin its calls
//! carry. Its hosts are private constants read only by these functions.

use bagholder_ws::session::{graphql_url, oauth_url, web_origin};

#[test]
fn every_address_is_the_stand_ins_when_one_is_named() {
    let fixture = bagholder_ws::standin::fixture(Box::new(|_| (200, vec![], b"{}".to_vec())));
    let base = std::env::var("BAGHOLDER_WS_BASE").expect("the fixture names its stand-in");
    for url in [graphql_url(), oauth_url(), web_origin()] {
        assert!(url.starts_with(&base), "{url} is not the stand-in's {base}");
    }
    drop(fixture);
}
