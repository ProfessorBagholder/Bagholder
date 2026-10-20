//! Every Wealthsimple address the app asks, in one place. `BAGHOLDER_WS_BASE`
//! points them all at a stand-in (the browser tests' signed-in server, a client
//! test): then nothing the app sends Wealthsimple leaves for any other host. The
//! older `bagholder_ws` client reads the same variable (`ws/src/session.rs`).

/// The variable that points every Wealthsimple address at a stand-in.
pub const BASE_VAR: &str = "BAGHOLDER_WS_BASE";

// Each real address is named only inside the one function that honours the
// variable, so no other code can reach the real host past a stand-in (brief 39, 1).

fn base() -> Option<String> {
    std::env::var(BASE_VAR).ok().map(|b| b.trim_end_matches('/').to_string()).filter(|b| !b.is_empty())
}

/// Wealthsimple's web app, whose origin its API calls carry.
pub fn web() -> String {
    const REAL: &str = "https://my.wealthsimple.com";
    base().unwrap_or_else(|| REAL.to_string())
}

/// The host of an address (`https://user@host:443/path` → `host`), empty for
/// one with none.
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    host.split(':').next().unwrap_or("")
}

/// The domain Wealthsimple's sign-in cookies are set for, beside `web()`: its
/// cookies are this domain's or a subdomain's. A stand-in's are its own host's.
pub fn cookie_domain() -> String {
    const REAL: &str = "wealthsimple.com";
    match base() {
        Some(b) => host_of(&b).to_ascii_lowercase(),
        None => REAL.to_string(),
    }
}

/// Whether a cookie's domain, or a page's host, is Wealthsimple's (or the
/// stand-in's): `cookie_domain()` itself or a subdomain of it, a leading dot
/// ignored.
pub fn is_wealthsimples(domain: &str) -> bool {
    let d = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    let own = cookie_domain();
    !own.is_empty() && (d == own || d.ends_with(&format!(".{own}")))
}

/// The GraphQL endpoint every read and order goes to.
pub fn graphql_url() -> String {
    format!("{}/graphql", web())
}

/// The token endpoint a saved sign-in is refreshed at.
pub fn token_url() -> String {
    const REAL: &str = "https://api.production.wealthsimple.com/v1/oauth/v2/token";
    match base() {
        Some(b) => format!("{b}/oauth/v2/token"),
        None => REAL.to_string(),
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
