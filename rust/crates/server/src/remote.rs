//! The page from anywhere (`docs/plans/page-from-anywhere.md`): Bagholder's own
//! page through a Cloudflare tunnel Bagholder makes, at a name on the person's
//! domain, behind Cloudflare Access.
//!
//! Access asks for the sign-in (GitHub's, where the person's Zero Trust team has
//! it) before any request reaches this computer, and lets in the one email the
//! person named. Bagholder doesn't take Access's word through the tunnel alone: each
//! request must carry Access's signed token for this application and that email,
//! checked here against the team's own keys, so an Access rule deleted or loosened
//! lets no one in. Bagholder makes the Access application with a token the person
//! creates on Cloudflare's page (`TOKEN_PAGE`): the tunnel's sign-in manages tunnels
//! and DNS alone.
//!
//! The sign-in is GitHub's, through a login method of Bagholder's own: a private
//! GitHub app named for it, made from a manifest on GitHub's page with one press
//! (`manifest`), allowed to read the person's email and nothing else, and added to
//! Cloudflare as a login method the application alone uses. A login another of the
//! person's applications uses is never borrowed. It is made once for a domain and
//! found again by its name (`github_name`): GitHub won't make two apps of one name,
//! and won't let Bagholder delete one, so a Remove keeps it for the next set-up.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use bagholder_core::jiff::{SignedDuration, Timestamp};
use openssl::pkey::{PKey, Public};
use serde::{Deserialize, Serialize};

use crate::agents::{cloudflare, tunnel};

/// The header Access adds to each request it lets through: its signed token.
pub const JWT_HEADER: &str = "cf-access-jwt-assertion";
/// How far a token's times may be from this computer's clock.
pub const LEEWAY: SignedDuration = SignedDuration::from_secs(60);
/// How often the team's keys may be asked again for a key not seen yet.
pub const KEYS_AGAIN: SignedDuration = SignedDuration::from_mins(1);
/// How long a device stays signed in at Access before it asks again.
pub const SESSION: &str = "24h";

/// Cloudflare's page for a token Bagholder may make the Access application with:
/// Access's applications and policies, and its login methods, to edit (the team is
/// read with them). The person presses Create there and pastes the token into
/// Bagholder.
pub const TOKEN_PAGE: &str = "https://dash.cloudflare.com/profile/api-tokens?permissionGroupKeys=%5B%7B%22key%22%3A%22access%22%2C%22type%22%3A%22edit%22%7D%2C%7B%22key%22%3A%22access_acct%22%2C%22type%22%3A%22edit%22%7D%5D&accountId=*&zoneId=all&name=Bagholder";
/// Where GitHub sends the person once Bagholder's GitHub app is made, on the page
/// they made it from.
pub const MADE: &str = "/api/remote/github/made";
/// How long a manifest handed to the page waits for the person to press Create on
/// GitHub: GitHub's own bound for the whole flow.
pub const MANIFEST_LIFE: SignedDuration = SignedDuration::from_hours(1);
/// Manifests waiting at once: the next drops the oldest.
pub const MANIFESTS_KEPT: usize = 5;
/// Where a Cloudflare account gets its Zero Trust team, once.
pub const TEAM_PAGE: &str = "https://one.dash.cloudflare.com/";
/// The person's API tokens at Cloudflare: where the token made for Bagholder is
/// deleted, which only the person can do.
pub const TOKENS_PAGE: &str = "https://dash.cloudflare.com/profile/api-tokens";

/// What is kept between starts, in Bagholder's folder, readable by the person alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    /// The tunnel's public name, once Cloudflare made it.
    #[serde(default)]
    pub host: String,
    /// Whether the tunnel runs with Bagholder.
    #[serde(default)]
    pub on: bool,
    /// The one email Access lets in, and Bagholder checks.
    #[serde(default)]
    pub email: String,
    /// The Zero Trust team's own address: `{team}.cloudflareaccess.com`.
    #[serde(default)]
    pub team: String,
    /// The Access application Bagholder made, and its audience tag: what its
    /// tokens name.
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub aud: String,
    /// Bagholder's own GitHub sign-in, once made: its GitHub app and the login
    /// method Cloudflare holds it as (its secret is Cloudflare's alone).
    #[serde(default)]
    pub github: Option<GitHubLogin>,
}

/// Bagholder's GitHub app, and the login method it is at Cloudflare.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubLogin {
    /// Its name on GitHub, as GitHub's page says it.
    pub name: String,
    /// Its address on GitHub (`github.com/settings/apps/{slug}`).
    pub slug: String,
    pub client_id: String,
    /// Its secret, kept only until Cloudflare holds it: Cloudflare can refuse the
    /// login method (a token without the right), and it is added again with it.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// The login method's id at Cloudflare: empty until Cloudflare took it.
    #[serde(default)]
    pub login: String,
}

impl Kept {
    /// Whether the page is set up behind Access: a name, its application and the
    /// email it lets in.
    pub fn set_up(&self) -> bool {
        !self.host.is_empty() && !self.team.is_empty() && !self.aud.is_empty() && !self.email.is_empty()
    }
}

/// Why a request's token is refused, for the log; or the team's keys to fetch first.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Signed with a key not known yet: the team's keys are asked again.
    UnknownKey,
    /// Made by another team than the one kept: the team may have been renamed, so
    /// Cloudflare is asked for its address again.
    OtherTeam,
    Bad(String),
}

#[derive(Default)]
pub struct Remote {
    /// What is kept on disk, read on first use.
    kept: Mutex<Option<Kept>>,
    /// The team's signing keys by id, and when they were last asked.
    keys: Mutex<(BTreeMap<String, PKey<Public>>, Option<Timestamp>)>,
    /// The page's tunnel: running or not, and what it last said.
    pub tunnel: Mutex<tunnel::Tunnels>,
    /// What the last check through the address found: none until one ran since
    /// the tunnel started.
    ready: Mutex<Option<Result<(), String>>>,
    /// The Zero Trust team the person's token found, once asked: empty when the
    /// account has none yet.
    team: Mutex<Option<String>>,
    /// Which run of the check from outside is current (`http::remote`): a run
    /// ends as soon as another begins, or the tunnel stops.
    checks: std::sync::atomic::AtomicU64,
    /// What the last try of the check found, while it keeps trying.
    last_try: Mutex<String>,
    /// The manifests handed to the page, waiting for GitHub to send their code back:
    /// each state, and when it ends.
    manifests: Mutex<Vec<(String, Timestamp)>>,
    /// When the team's address was last asked of Cloudflare, on a pass from another.
    team_asked: Mutex<Option<Timestamp>>,
    /// Why Cloudflare didn't take Bagholder's GitHub app as a login method, until it does.
    github_said: Mutex<String>,
}

/// Bagholder's folder for the page's address.
pub fn dir(home: &Path) -> PathBuf {
    home.join("remote")
}

fn kept_file(home: &Path) -> PathBuf {
    dir(home).join("remote.json")
}

fn token_file(home: &Path) -> PathBuf {
    dir(home).join("access-token")
}

/// The page's own Cloudflare tunnel, apart from the agents'.
pub fn cloudflare(home: &Path) -> cloudflare::Files {
    cloudflare::Files::at(dir(home).join("cloudflare"))
}

/// What is kept on disk: nothing set up when there is none, or it can't be read
/// (said in the log), so no name is ever answered from a file half read.
fn read(home: &Path) -> Kept {
    let said = |e: &dyn std::fmt::Display| crate::app::log(&format!("bagholder: the page’s address could not be read, so it is off: {e}"));
    match std::fs::read_to_string(kept_file(home)) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            said(&e);
            Kept::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Kept::default(),
        Err(e) => {
            said(&e);
            Kept::default()
        }
    }
}

/// `text` written to `file` in Bagholder's folder for the page, whole or not at
/// all, readable by the person alone.
fn write_private(home: &Path, file: &Path, text: &str) -> Result<(), String> {
    let d = dir(home);
    std::fs::create_dir_all(&d).map_err(|e| format!("{} could not be made: {e}", d.display()))?;
    let part = file.with_extension("part");
    std::fs::write(&part, text).map_err(|e| format!("{} could not be written: {e}", part.display()))?;
    tunnel::private(&[d], &[part.clone()])?;
    std::fs::rename(&part, file).map_err(|e| format!("{} could not be written: {e}", file.display()))
}

/// Bytes in the URL-safe base64 alphabet, padded or not.
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    if !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'=') {
        return None;
    }
    let mut t = s.trim_end_matches('=').replace('-', "+").replace('_', "/");
    while t.len() % 4 != 0 {
        t.push('=');
    }
    openssl::base64::decode_block(&t).ok()
}

/// The team's keys, from its `certs` answer: each RSA key by its id.
pub fn keys_of(v: &serde_json::Value) -> BTreeMap<String, PKey<Public>> {
    let mut out = BTreeMap::new();
    for k in v.get("keys").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let text = |f: &str| k.get(f).and_then(serde_json::Value::as_str);
        if text("kty") != Some("RSA") {
            continue;
        }
        let key = (|| {
            let n = openssl::bn::BigNum::from_slice(&b64url_decode(text("n")?)?).ok()?;
            let e = openssl::bn::BigNum::from_slice(&b64url_decode(text("e")?)?).ok()?;
            PKey::from_rsa(openssl::rsa::Rsa::from_public_components(n, e).ok()?).ok()
        })();
        if let (Some(kid), Some(key)) = (text("kid"), key) {
            out.insert(kid.to_string(), key);
        }
    }
    out
}

/// A token Access signed for this application and the email the person named,
/// checked against the team's keys: RS256 alone, its audience, its issuer, its
/// times, and its email.
pub fn check(jwt: &str, kept: &Kept, keys: &BTreeMap<String, PKey<Public>>, now: Timestamp) -> Result<(), Refused> {
    let bad = |s: &str| Refused::Bad(s.to_string());
    if !kept.set_up() {
        return Err(bad("the address is not set up"));
    }
    let mut parts = jwt.split('.');
    let (Some(h), Some(p), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { return Err(bad("not a token")) };
    let head: serde_json::Value = b64url_decode(h).and_then(|b| serde_json::from_slice(&b).ok()).ok_or_else(|| bad("its header can't be read"))?;
    if head.get("alg").and_then(serde_json::Value::as_str) != Some("RS256") {
        return Err(bad("not signed with RS256"));
    }
    let kid = head.get("kid").and_then(serde_json::Value::as_str).ok_or_else(|| bad("no key id"))?;
    let key = keys.get(kid).ok_or(Refused::UnknownKey)?;
    let sig = b64url_decode(sig).ok_or_else(|| bad("its signature can't be read"))?;
    let signed = (|| {
        let mut v = openssl::sign::Verifier::new(openssl::hash::MessageDigest::sha256(), key).ok()?;
        v.update(format!("{h}.{p}").as_bytes()).ok()?;
        v.verify(&sig).ok()
    })();
    if signed != Some(true) {
        return Err(bad("its signature is wrong"));
    }
    let claims: serde_json::Value = b64url_decode(p).and_then(|b| serde_json::from_slice(&b).ok()).ok_or_else(|| bad("its claims can't be read"))?;
    let aud_ok = match claims.get("aud") {
        Some(serde_json::Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(kept.aud.as_str())),
        Some(serde_json::Value::String(s)) => *s == kept.aud,
        _ => false,
    };
    if !aud_ok {
        return Err(bad("made for another application"));
    }
    if claims.get("iss").and_then(serde_json::Value::as_str) != Some(format!("https://{}", kept.team).as_str()) {
        return Err(Refused::OtherTeam);
    }
    let at = |f: &str| claims.get(f).and_then(serde_json::Value::as_i64).and_then(|s| Timestamp::from_second(s).ok());
    match at("exp") {
        Some(exp) if exp + LEEWAY > now => {}
        _ => return Err(bad("expired")),
    }
    if at("nbf").is_some_and(|nbf| nbf - LEEWAY > now) {
        return Err(bad("not valid yet"));
    }
    let email = claims.get("email").and_then(serde_json::Value::as_str).unwrap_or("");
    if kept.email.is_empty() || !email.eq_ignore_ascii_case(&kept.email) {
        return Err(Refused::Bad(format!("made for {email:?}, not the email the person named")));
    }
    Ok(())
}

impl Remote {
    /// What is kept, as last written.
    pub fn kept(&self, home: &Path) -> Kept {
        let mut held = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        held.get_or_insert_with(|| read(home)).clone()
    }

    /// `kept` written, then used from here on.
    pub fn keep(&self, home: &Path, kept: Kept) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&kept).map_err(|e| e.to_string())?;
        write_private(home, &kept_file(home), &text)?;
        *self.kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(kept);
        self.keys.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
        Ok(())
    }

    /// A request's token checked against the keys held (`check`).
    pub fn check(&self, jwt: &str, kept: &Kept, now: Timestamp) -> Result<(), Refused> {
        check(jwt, kept, &self.keys.lock().unwrap_or_else(|e| e.into_inner()).0, now)
    }

    /// The team's keys asked again, unless they were asked within `KEYS_AGAIN`:
    /// a token signed with a key not seen yet is checked once they are in.
    pub fn fetch_keys(&self, team: &str, now: Timestamp) -> Result<(), String> {
        {
            let held = self.keys.lock().unwrap_or_else(|e| e.into_inner());
            if held.1.is_some_and(|t| t + KEYS_AGAIN > now) {
                return Err("the team’s keys were asked a moment ago".into());
            }
        }
        self.keys.lock().unwrap_or_else(|e| e.into_inner()).1 = Some(now);
        let v = get_json(&format!("https://{team}/cdn-cgi/access/certs"), &[])?;
        let keys = keys_of(&v);
        if keys.is_empty() {
            return Err("the team named no keys".into());
        }
        self.keys.lock().unwrap_or_else(|e| e.into_inner()).0 = keys;
        Ok(())
    }

    /// The keys a test signs with.
    #[cfg(test)]
    pub fn set_keys(&self, keys: BTreeMap<String, PKey<Public>>) {
        *self.keys.lock().unwrap_or_else(|e| e.into_inner()) = (keys, None);
    }

    pub fn ready(&self) -> Option<Result<(), String>> {
        self.ready.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_ready(&self, r: Option<Result<(), String>>) {
        *self.ready.lock().unwrap_or_else(|e| e.into_inner()) = r;
    }

    /// The team the token found, once asked: empty when there is none.
    pub fn team(&self) -> Option<String> {
        self.team.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_team(&self, team: Option<String>) {
        *self.team.lock().unwrap_or_else(|e| e.into_inner()) = team;
    }

    /// A new run of the check from outside: the one before ends at its next try.
    pub fn next_check(&self) -> u64 {
        self.set_last_try(String::new());
        self.checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// Whether `run` is still the check's current run.
    pub fn checking(&self, run: u64) -> bool {
        self.checks.load(std::sync::atomic::Ordering::SeqCst) == run
    }

    pub fn last_try(&self) -> String {
        self.last_try.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_last_try(&self, said: String) {
        *self.last_try.lock().unwrap_or_else(|e| e.into_inner()) = said;
    }

    pub fn github_said(&self) -> String {
        self.github_said.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_github_said(&self, said: String) {
        *self.github_said.lock().unwrap_or_else(|e| e.into_inner()) = said;
    }

    /// Whether the team's address may be asked of Cloudflare again: once a minute
    /// at most, however many passes name another team.
    pub fn may_ask_team(&self, now: Timestamp) -> bool {
        let mut held = self.team_asked.lock().unwrap_or_else(|e| e.into_inner());
        if held.is_some_and(|t| t + KEYS_AGAIN > now) {
            return false;
        }
        *held = Some(now);
        true
    }

    /// A manifest handed out: its state, which GitHub sends back with its code.
    pub fn manifest_out(&self, now: Timestamp) -> String {
        let state = crate::agents::secret::secret("");
        let mut held = self.manifests.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|(_, until)| *until > now);
        held.push((state.clone(), now + MANIFEST_LIFE));
        while held.len() > MANIFESTS_KEPT {
            held.remove(0);
        }
        state
    }

    /// Whether a manifest handed out is still waiting on GitHub.
    pub fn making_github(&self, now: Timestamp) -> bool {
        self.manifests.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(_, until)| *until > now)
    }

    /// The manifest `state` names taken back, once: false when this Bagholder never
    /// handed it out, or it waited too long.
    pub fn manifest_back(&self, state: &str, now: Timestamp) -> bool {
        let mut held = self.manifests.lock().unwrap_or_else(|e| e.into_inner());
        let found = !state.is_empty() && held.iter().any(|(s, until)| *until > now && crate::agents::secret::same(s.as_bytes(), state.as_bytes()));
        if found {
            held.clear();
        }
        found
    }
}

/// The token the person made for Access, when they gave one.
pub fn token(home: &Path) -> Option<String> {
    std::fs::read_to_string(token_file(home)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// The person's token kept, once Cloudflare said it works.
pub fn keep_token(home: &Path, token: &str) -> Result<(), String> {
    write_private(home, &token_file(home), token)
}

/// Bagholder's files for the page gone: what is kept, the token the person made
/// (Bagholder keeps no credential it no longer uses), and the tunnel's own.
pub fn forget(home: &Path) -> Result<(), String> {
    for f in [kept_file(home), token_file(home)] {
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{} could not be removed: {e}", f.display())),
            _ => {}
        }
    }
    cloudflare(home).forget()
}

// ---- Cloudflare's API, with the person's token ----

const WAIT: Duration = Duration::from_secs(30);
const API: &str = "https://api.cloudflare.com/client/v4";

fn user_agent() -> String {
    format!("Bagholder/{}", crate::app::APP_VERSION)
}

fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let ua = user_agent();
    let mut h = vec![("Accept", "application/json"), ("User-Agent", ua.as_str())];
    h.extend_from_slice(headers);
    let said = bagholder_net::client::request("GET", url, &h, None, WAIT).map_err(|e| e.to_string())?;
    serde_json::from_slice(&said.body).map_err(|e| format!("the answer could not be read: {e}"))
}

/// One call to Cloudflare's API: its `result`, or what Cloudflare said is wrong.
fn api(token: &str, method: &str, path: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value, String> {
    let (auth, ua) = (format!("Bearer {token}"), user_agent());
    let headers = [("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json"), ("User-Agent", ua.as_str())];
    let text = body.map(serde_json::Value::to_string);
    let said = bagholder_net::client::request_any(method, &format!("{API}{path}"), &headers, text.as_deref().map(str::as_bytes), WAIT).map_err(|e| format!("Cloudflare can’t be reached: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&said.body).map_err(|e| format!("Cloudflare’s answer could not be read: {e}"))?;
    if v.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null));
    }
    Err(errors_of(&v).unwrap_or_else(|| format!("Cloudflare refused it (HTTP {}).", said.status)))
}

/// What Cloudflare's answer says went wrong, in its words.
pub fn errors_of(v: &serde_json::Value) -> Option<String> {
    let said: Vec<String> = v.get("errors").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|e| e.get("message").and_then(serde_json::Value::as_str)).map(str::to_string).collect();
    (!said.is_empty()).then(|| format!("Cloudflare said: {}", said.join("; ")))
}

/// Whether Cloudflare takes the token at all.
pub fn token_works(token: &str) -> Result<(), String> {
    api(token, "GET", "/user/tokens/verify", None).map(|_| ())
}

/// The account's Zero Trust team, by its own address: none when the account has
/// no team yet, which only Cloudflare's dashboard makes (`TEAM_PAGE`).
pub fn team(token: &str, account: &str) -> Result<Option<String>, String> {
    match api(token, "GET", &format!("/accounts/{account}/access/organizations"), None) {
        Ok(v) => Ok(v.get("auth_domain").and_then(serde_json::Value::as_str).map(str::to_ascii_lowercase).filter(|d| !d.is_empty())),
        Err(e) if e.to_ascii_lowercase().contains("not found") || e.to_ascii_lowercase().contains("not enabled") => Ok(None),
        Err(e) => Err(e),
    }
}

/// The name of Bagholder's GitHub app for `domain`, as GitHub's page shows it: the
/// same for a domain every time, so a set-up finds the one made before; unique on
/// GitHub, and at most 34 characters long (a long domain cut to fit).
pub fn github_name(domain: &str) -> String {
    format!("Bagholder at {domain}").chars().take(34).collect()
}

/// The address GitHub gives an app of `name` (`github.com/settings/apps/{slug}`):
/// lower case, every run of other characters one hyphen.
pub fn slug_of(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Bagholder's GitHub login among the account's login methods, by its name: its id.
pub fn login_of(listed: &serde_json::Value, name: &str) -> Option<String> {
    listed.as_array()?.iter().find(|p| p.get("type").and_then(serde_json::Value::as_str) == Some("github") && p.get("name").and_then(serde_json::Value::as_str) == Some(name)).and_then(|p| p.get("id").and_then(serde_json::Value::as_str)).map(str::to_string)
}

/// Bagholder's GitHub login made by an earlier set-up for this domain, if Cloudflare
/// still has it.
pub fn find_login(token: &str, account: &str, name: &str) -> Result<Option<String>, String> {
    let listed = api(token, "GET", &format!("/accounts/{account}/access/identity_providers"), None)?;
    Ok(login_of(&listed, name))
}

/// The manifest of Bagholder's GitHub app: private to the person's account, allowed
/// to read their email (what Cloudflare's GitHub login asks of GitHub) and nothing
/// else, no webhook, signing in through Cloudflare's callback at `team`; GitHub
/// sends them back to `back` (the page they made it from) once it is made.
pub fn manifest(team: &str, domain: &str, back: &str) -> serde_json::Value {
    serde_json::json!({
        "name": github_name(domain),
        "url": format!("https://github.com/{}", crate::app::REPO),
        "description": format!("Signs you in to Bagholder at {domain}, through Cloudflare Access."),
        "hook_attributes": { "url": format!("https://{team}/"), "active": false },
        "redirect_url": format!("{back}{MADE}"),
        "callback_urls": [format!("https://{team}/cdn-cgi/access/callback")],
        "public": false,
        // the manifest's own name for the email permission: GitHub refuses the
        // `email_addresses` its REST pages use ("Default permission records resource
        // is not included in the list")
        "default_permissions": { "emails": "read" },
        "request_oauth_on_install": false,
    })
}

/// The app GitHub's answer for a manifest names: its name, its address, and its
/// client id and secret.
pub fn made_of(v: &serde_json::Value) -> Option<(String, String, String, String)> {
    let text = |p: &str| v.pointer(p).and_then(serde_json::Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
    Some((text("/name")?, text("/slug")?, text("/client_id")?, text("/client_secret")?))
}

/// The app GitHub made from Bagholder's manifest, from the code it sent back.
pub fn github_made(code: &str) -> Result<serde_json::Value, String> {
    // GitHub's code goes into the path: only a code's characters are taken
    if code.is_empty() || code.len() > 128 || !code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err("GitHub sent back no code for the app.".into());
    }
    let ua = user_agent();
    let headers = [("Accept", "application/vnd.github+json"), ("User-Agent", ua.as_str()), ("X-GitHub-Api-Version", "2022-11-28")];
    let said = bagholder_net::client::request("POST", &format!("https://api.github.com/app-manifests/{code}/conversions"), &headers, Some(b""), WAIT).map_err(|e| format!("GitHub didn’t answer for the app: {e}"))?;
    serde_json::from_slice(&said.body).map_err(|e| format!("GitHub’s answer for the app could not be read: {e}"))
}

/// Bagholder's GitHub app added to Cloudflare as a login method of its own: its id.
pub fn make_login(token: &str, account: &str, name: &str, client_id: &str, client_secret: &str) -> Result<String, String> {
    let body = serde_json::json!({ "name": name, "type": "github", "config": { "client_id": client_id, "client_secret": client_secret } });
    let made = api(token, "POST", &format!("/accounts/{account}/access/identity_providers"), Some(&body))?;
    made.get("id").and_then(serde_json::Value::as_str).map(str::to_string).filter(|s| !s.is_empty()).ok_or_else(|| "Cloudflare named no login method.".into())
}

/// The Access application for `host`, set as a person sets one up in Cloudflare's
/// dashboard, every other setting Cloudflare's own: Bagholder's own GitHub login
/// (`login`) alone, and one rule requiring it and `email`.
pub fn application(host: &str, email: &str, login: &str) -> serde_json::Value {
    serde_json::json!({
        "name": format!("Bagholder at {host}"),
        "domain": host,
        "type": "self_hosted",
        "session_duration": SESSION,
        "allowed_idps": [login],
        "policies": [{
            "name": format!("{email} alone"),
            "decision": "allow",
            "include": [{ "login_method": { "id": login } }],
            "require": [{ "email": { "email": email } }],
        }],
    })
}

/// The Access application made, its id and audience tag: one Bagholder made before
/// for `host` (by its name) is replaced; one the person made is left as it is.
pub fn make_application(token: &str, account: &str, host: &str, email: &str, login: &str) -> Result<(String, String), String> {
    let listed = api(token, "GET", &format!("/accounts/{account}/access/apps"), None)?;
    let ours = |a: &&serde_json::Value| a.get("domain").and_then(serde_json::Value::as_str).is_some_and(|d| d.eq_ignore_ascii_case(host)) && a.get("name").and_then(serde_json::Value::as_str) == Some(format!("Bagholder at {host}").as_str());
    for old in listed.as_array().into_iter().flatten().filter(ours) {
        if let Some(id) = old.get("id").and_then(serde_json::Value::as_str) {
            api(token, "DELETE", &format!("/accounts/{account}/access/apps/{id}"), None)?;
        }
    }
    let made = api(token, "POST", &format!("/accounts/{account}/access/apps"), Some(&application(host, email, login)))?;
    let text = |f: &str| made.get(f).and_then(serde_json::Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
    Ok((text("id").ok_or("Cloudflare named no application.")?, text("aud").ok_or("Cloudflare gave the application no audience tag.")?))
}

/// The Access application deleted; one already gone is fine.
pub fn delete_application(token: &str, account: &str, id: &str) -> Result<(), String> {
    match api(token, "DELETE", &format!("/accounts/{account}/access/apps/{id}"), None) {
        Err(e) if !e.to_ascii_lowercase().contains("not found") => Err(e),
        _ => Ok(()),
    }
}

/// Whether the address, asked from outside as a stranger asks it, sends to the
/// team's sign-in and shows nothing of the page.
pub fn asks_for_sign_in(status: u16, location: Option<&str>, team: &str) -> bool {
    (300..400).contains(&status) && location.is_some_and(|l| l.starts_with(&format!("https://{team}/")))
}

/// `https://{host}/` asked from this computer the way the internet asks it: its name
/// looked up in public DNS (never this computer's resolver, which may still hold an
/// answer from before the name was made), then that public address alone, through
/// Cloudflare and the tunnel.
pub fn check_through(host: &str, team: &str) -> Result<(), String> {
    let outside = crate::agents::fetch::Outside::network();
    let answers = outside.public_dns.resolve(host, 443).map_err(|e| format!("{host} can’t be looked up yet: {e}"))?;
    let ip = answers.first().copied().filter(|_| answers.iter().all(|a| crate::agents::fetch::public(*a))).ok_or_else(|| format!("{host} isn’t in public DNS yet."))?;
    let ua = user_agent();
    let said = outside.get.get(std::net::SocketAddr::new(ip, 443), &format!("https://{host}/"), &[("User-Agent", ua.as_str())], crate::agents::fetch::FETCH_LIMIT).map_err(|e| format!("{host} didn’t answer yet: {e}"))?;
    let location = said.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.as_str());
    if asks_for_sign_in(said.status, location, team) {
        Ok(())
    } else if let Some(to) = location.filter(|_| (300..400).contains(&said.status)) {
        let to = to.split('/').nth(2).unwrap_or(to);
        Err(format!("{host} sent the sign-in to {to}, not to your team’s address."))
    } else {
        Err(format!("{host} answered without asking for the sign-in (HTTP {}).", said.status))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use openssl::pkey::Private;

    pub const TEAM: &str = "team.cloudflareaccess.com";
    pub const AUD: &str = "aud-1";
    pub const EMAIL: &str = "me@example.com";

    pub fn kept() -> Kept {
        Kept { host: "app.example.com".into(), on: true, email: EMAIL.into(), team: TEAM.into(), app_id: "app-1".into(), aud: AUD.into(), github: None }
    }

    /// A key Access might sign with, and the team's keys holding its public half as `kid`.
    pub fn key(kid: &str) -> (PKey<Private>, BTreeMap<String, PKey<Public>>) {
        let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
        let private = PKey::from_rsa(rsa.clone()).unwrap();
        let public = PKey::public_key_from_pem(&private.public_key_to_pem().unwrap()).unwrap();
        (private, BTreeMap::from([(kid.to_string(), public)]))
    }

    fn b64(bytes: &[u8]) -> String {
        crate::agents::secret::b64url(bytes)
    }

    /// A token as Access makes it: `head` and `claims` signed with `key` by RS256.
    pub fn signed(key: &PKey<Private>, head: serde_json::Value, claims: serde_json::Value) -> String {
        let input = format!("{}.{}", b64(head.to_string().as_bytes()), b64(claims.to_string().as_bytes()));
        let mut s = openssl::sign::Signer::new(openssl::hash::MessageDigest::sha256(), key).unwrap();
        s.update(input.as_bytes()).unwrap();
        format!("{input}.{}", b64(&s.sign_to_vec().unwrap()))
    }

    pub fn claims(now: Timestamp) -> serde_json::Value {
        serde_json::json!({ "aud": [AUD], "email": EMAIL, "iss": format!("https://{TEAM}"), "exp": now.as_second() + 3600, "nbf": now.as_second() - 10, "iat": now.as_second() - 10, "type": "app" })
    }

    pub fn head(kid: &str) -> serde_json::Value {
        serde_json::json!({ "alg": "RS256", "kid": kid, "typ": "JWT" })
    }

    /// Access's token for this application and the email named passes, and each
    /// way a token can be wrong is refused on its own.
    #[test]
    fn a_token_is_taken_only_signed_by_the_team_for_this_application_and_email() {
        let now = Timestamp::now();
        let (k, keys) = key("k1");
        let ok = |t: &str| check(t, &kept(), &keys, now);
        assert_eq!(ok(&signed(&k, head("k1"), claims(now))), Ok(()));
        // the email is folded as Access folds it
        let mut upper = claims(now);
        upper["email"] = serde_json::json!("ME@example.com");
        assert_eq!(ok(&signed(&k, head("k1"), upper)), Ok(()));

        let with = |f: &str, v: serde_json::Value| {
            let mut c = claims(now);
            c[f] = v;
            signed(&k, head("k1"), c)
        };
        let refused = |t: String, what: &str| assert!(matches!(ok(&t), Err(Refused::Bad(_))), "{what}");
        refused(with("aud", serde_json::json!(["another"])), "another application's");
        assert_eq!(ok(&with("iss", serde_json::json!("https://other.cloudflareaccess.com"))), Err(Refused::OtherTeam), "another team's: its address asked again, never taken");
        refused(with("exp", serde_json::json!(now.as_second() - 120)), "expired");
        refused(with("nbf", serde_json::json!(now.as_second() + 600)), "not valid yet");
        refused(with("email", serde_json::json!("someone@example.com")), "another email");
        let mut none = claims(now);
        none.as_object_mut().unwrap().remove("email");
        refused(signed(&k, head("k1"), none), "no email");
        // signed with another key under the same id
        let (other, _) = key("k1");
        refused(signed(&other, head("k1"), claims(now)), "another key");
        // the claims changed after signing
        let good = signed(&k, head("k1"), claims(now));
        let mut parts: Vec<&str> = good.split('.').collect();
        let mut c = claims(now);
        c["email"] = serde_json::json!("someone@example.com");
        let forged = b64(c.to_string().as_bytes());
        parts[1] = &forged;
        refused(parts.join("."), "claims changed after signing");
        // no signature taken but RS256's: none, and HS256 keyed with anything
        let unsigned = format!("{}.{}.", b64(serde_json::json!({ "alg": "none", "kid": "k1" }).to_string().as_bytes()), b64(claims(now).to_string().as_bytes()));
        refused(unsigned, "alg none");
        let hs = signed(&k, serde_json::json!({ "alg": "HS256", "kid": "k1" }), claims(now));
        refused(hs, "HS256");
        for junk in ["", "a.b", "a.b.c.d", "not a token"] {
            refused(junk.to_string(), junk);
        }
        // a key the team has not been asked for yet: asked again, not refused outright
        assert_eq!(ok(&signed(&k, head("k2"), claims(now))), Err(Refused::UnknownKey));
        // and nothing is taken while no email or audience is set up
        let mut bare = kept();
        bare.email.clear();
        assert!(check(&signed(&k, head("k1"), claims(now)), &bare, &keys, now).is_err());
    }

    /// The team's keys as its `certs` answer gives them: each RSA key by its id,
    /// verifying what its private half signed.
    #[test]
    fn the_team_s_keys_are_read_from_its_answer() {
        let now = Timestamp::now();
        let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
        let answer = serde_json::json!({ "keys": [
            { "kid": "k9", "kty": "RSA", "alg": "RS256", "use": "sig", "n": b64(&rsa.n().to_vec()), "e": b64(&rsa.e().to_vec()) },
            { "kid": "ec", "kty": "EC", "crv": "P-256", "x": "AA", "y": "AA" },
        ] });
        let keys = keys_of(&answer);
        assert_eq!(keys.keys().collect::<Vec<_>>(), ["k9"]);
        let k = PKey::from_rsa(rsa).unwrap();
        assert_eq!(check(&signed(&k, head("k9"), claims(now)), &kept(), &keys, now), Ok(()));
    }

    /// The Access application lets in the email alone, through Bagholder's own
    /// GitHub login alone, as a person sets one up in the dashboard: nothing of
    /// Cloudflare's own defaults changed (instant authentication, the App Launcher).
    #[test]
    fn the_access_application_lets_in_the_email_alone_through_bagholder_s_own_login() {
        let app = application("app.example.com", EMAIL, "login-bh");
        assert_eq!(app["domain"], "app.example.com");
        assert_eq!(app["type"], "self_hosted");
        assert_eq!(app["allowed_idps"], serde_json::json!(["login-bh"]));
        assert_eq!(app["policies"], serde_json::json!([{ "name": "me@example.com alone", "decision": "allow", "include": [{ "login_method": { "id": "login-bh" } }], "require": [{ "email": { "email": EMAIL } }] }]));
        for default in ["auto_redirect_to_identity", "app_launcher_visible"] {
            assert!(app.get(default).is_none(), "{default} is Cloudflare's own");
        }
    }

    /// Bagholder's GitHub app: named for the domain where the name fits, private,
    /// reading the email alone, no webhook, signing in through the team's callback,
    /// and sending the person back to the page they made it from.
    #[test]
    fn bagholder_s_github_app_reads_the_email_alone_and_signs_in_through_the_team() {
        let m = manifest(TEAM, "example.com", "http://127.0.0.1:8765");
        assert_eq!(m["name"], "Bagholder at example.com");
        assert_eq!(m["public"], false);
        assert_eq!(m["default_permissions"], serde_json::json!({ "emails": "read" }));
        assert_eq!(m["hook_attributes"]["active"], false);
        assert!(m.get("default_events").is_none());
        assert_eq!(m["callback_urls"], serde_json::json!(["https://team.cloudflareaccess.com/cdn-cgi/access/callback"]));
        assert_eq!(m["redirect_url"], "http://127.0.0.1:8765/api/remote/github/made");
        let long = github_name("a-very-long-domain-name-of-someone.example.com");
        assert_eq!(long, "Bagholder at a-very-long-domain-na", "cut to fit, the same every time");
        assert_eq!(slug_of("Bagholder at example.com"), "bagholder-at-example-com");
        // found again by its name alone: never another GitHub login of the account's
        let listed = serde_json::json!([{ "id": "otp", "type": "onetimepin", "name": "" }, { "id": "wiki", "type": "github", "name": "GitHub" }, { "id": "ours", "type": "github", "name": "Bagholder at example.com" }]);
        assert_eq!(login_of(&listed, "Bagholder at example.com"), Some("ours".into()));
        assert_eq!(login_of(&listed, "Bagholder at other.com"), None);
        let answer = serde_json::json!({ "id": 1, "name": "Bagholder at example.com", "slug": "bagholder-at-example-com", "client_id": "Iv23x", "client_secret": "s3cret", "pem": "-----BEGIN", "owner": { "login": "someone", "id": 7 } });
        assert_eq!(made_of(&answer), Some(("Bagholder at example.com".into(), "bagholder-at-example-com".into(), "Iv23x".into(), "s3cret".into())));
        assert_eq!(made_of(&serde_json::json!({ "name": "x" })), None);
        // a manifest is taken back once, and only one this Bagholder handed out
        let r = Remote::default();
        let now = Timestamp::now();
        let state = r.manifest_out(now);
        assert!(r.making_github(now));
        assert!(!r.manifest_back("someone-else's", now));
        assert!(r.manifest_back(&state, now));
        assert!(!r.manifest_back(&state, now), "once");
        assert!(!r.making_github(now));
        let late = r.manifest_out(now);
        assert!(!r.manifest_back(&late, now + MANIFEST_LIFE + SignedDuration::from_secs(1)), "waited too long");
    }

    /// From outside, the address must send to the team's sign-in: anything else,
    /// the page itself above all, is not ready.
    #[test]
    fn the_address_is_ready_only_asking_for_the_team_s_sign_in() {
        assert!(asks_for_sign_in(302, Some("https://team.cloudflareaccess.com/cdn-cgi/access/login/app.example.com?kid=x"), TEAM));
        assert!(!asks_for_sign_in(200, None, TEAM), "the page answered");
        assert!(!asks_for_sign_in(302, Some("https://evil.example/cdn-cgi"), TEAM));
        assert!(!asks_for_sign_in(302, Some("https://team.cloudflareaccess.com.evil.example/"), TEAM));
        assert_eq!(errors_of(&serde_json::json!({ "success": false, "errors": [{ "code": 10000, "message": "Authentication error" }] })), Some("Cloudflare said: Authentication error".into()));
    }

    /// What is kept is written readable by the person alone, and a file that can't
    /// be read is nothing set up.
    #[test]
    fn what_is_kept_is_private_and_a_file_half_read_is_nothing() {
        let home = tempfile::tempdir().unwrap();
        let r = Remote::default();
        r.keep(home.path(), kept()).unwrap();
        assert_eq!(read(home.path()), kept());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(kept_file(home.path())).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(dir(home.path())).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::write(kept_file(home.path()), "{ not json").unwrap();
        assert_eq!(read(home.path()), Kept::default());
        assert!(!Kept::default().set_up());
    }
}
