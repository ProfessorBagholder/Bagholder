//! Every Wealthsimple address the app asks, in one place. `BAGHOLDER_WS_BASE`
//! points them all at a stand-in (the browser tests' signed-in server, a client
//! test): then nothing the app sends Wealthsimple leaves for any other host. The
//! older `bagholder_ws` client reads the same variable (`ws/src/session.rs`).

/// The variable that points every Wealthsimple address at a stand-in.
pub const BASE_VAR: &str = "BAGHOLDER_WS_BASE";

const WEB: &str = "https://my.wealthsimple.com";
const TOKEN: &str = "https://api.production.wealthsimple.com/v1/oauth/v2/token";

fn base() -> Option<String> {
    std::env::var(BASE_VAR).ok().map(|b| b.trim_end_matches('/').to_string()).filter(|b| !b.is_empty())
}

/// Wealthsimple's web app, whose origin its API calls carry.
pub fn web() -> String {
    base().unwrap_or_else(|| WEB.to_string())
}

/// The GraphQL endpoint every read and order goes to.
pub fn graphql_url() -> String {
    format!("{}/graphql", web())
}

/// The token endpoint a saved sign-in is refreshed at.
pub fn token_url() -> String {
    match base() {
        Some(b) => format!("{b}/oauth/v2/token"),
        None => TOKEN.to_string(),
    }
}

/// The sign-in page, which the sign-in browser opens and the client id is read from.
pub fn login_url() -> String {
    format!("{}/app/login", web())
}

#[cfg(test)]
mod tests {
    /// With the variable set, every address is the stand-in's.
    #[test]
    fn every_address_is_the_stand_ins_when_one_is_named() {
        std::env::set_var(super::BASE_VAR, "http://127.0.0.1:9/");
        for url in [super::web(), super::graphql_url(), super::token_url(), super::login_url()] {
            assert!(url.starts_with("http://127.0.0.1:9"), "{url}");
        }
        std::env::remove_var(super::BASE_VAR);
        assert_eq!(super::graphql_url(), "https://my.wealthsimple.com/graphql");
    }
}
