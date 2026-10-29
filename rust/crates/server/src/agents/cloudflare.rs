//! Cloudflare Tunnel set up by Bagholder (brief 81 §3): the person signs in on
//! Cloudflare's page and names the address; Bagholder makes the tunnel and the
//! address's DNS record, and runs it. Every file is in Bagholder's own folder:
//! cloudflared is given that folder as its home and is named each file, so a
//! Cloudflare set up on this computer for anything else (its own `~/.cloudflared`,
//! `/etc/cloudflared`) is never read or changed.

use std::path::{Path, PathBuf};

use super::tunnel::run;

pub struct Files {
    dir: PathBuf,
}

impl Files {
    pub fn new(home: &Path) -> Files {
        Files::at(home.join("agents-cloudflare"))
    }

    /// The same files in another folder of Bagholder's: the page's own tunnel
    /// (`remote`) keeps its tunnel there, apart from the agents'.
    pub fn at(dir: PathBuf) -> Files {
        Files { dir }
    }

    /// The sign-in `tunnel login` writes: under the home it is given.
    pub fn cert(&self) -> PathBuf {
        self.dir.join(".cloudflared").join("cert.pem")
    }

    fn config(&self) -> PathBuf {
        self.dir.join("config.yml")
    }

    fn credentials(&self) -> PathBuf {
        self.dir.join("tunnel.json")
    }

    fn name_file(&self) -> PathBuf {
        self.dir.join("tunnel-name")
    }

    pub fn signed_in(&self) -> bool {
        self.cert().is_file()
    }

    /// The tunnel Bagholder made, once it made one.
    pub fn tunnel(&self) -> Option<String> {
        if !self.credentials().is_file() {
            return None;
        }
        std::fs::read_to_string(self.name_file()).ok().map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
    }

    /// The folder, and the config cloudflared reads instead of any other: empty.
    fn prepare(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.dir.join(".cloudflared")).map_err(|e| e.to_string())?;
        if !self.config().is_file() {
            std::fs::write(self.config(), "").map_err(|e| e.to_string())?;
        }
        self.secure()
    }

    /// The sign-in, the tunnel's credentials and their folders, readable by the
    /// person alone (`tunnel::private`): kept so after each is written.
    pub fn secure(&self) -> Result<(), String> {
        super::tunnel::private(&[self.dir.clone(), self.dir.join(".cloudflared")], &[self.cert(), self.credentials(), self.config(), self.name_file()])
    }

    /// cloudflared's arguments for `tunnel {rest}`, held to this folder.
    pub fn args(&self, rest: &[String]) -> Vec<String> {
        let mut a = vec!["--config".into(), self.config().display().to_string(), "tunnel".into(), "--origincert".into(), self.cert().display().to_string()];
        a.extend(rest.iter().cloned());
        a
    }

    /// Its home: this folder, on every system.
    pub fn env(&self) -> Vec<(String, String)> {
        let d = self.dir.display().to_string();
        vec![("HOME".into(), d.clone()), ("USERPROFILE".into(), d)]
    }

    /// The sign-in's command: `tunnel login`, which prints the page to open and
    /// writes the sign-in here once the person authorizes a domain.
    pub fn sign_in(&self, program: &str) -> Result<Vec<String>, String> {
        self.prepare()?;
        Ok(std::iter::once(program.to_string()).chain(self.args(&["login".into()])).collect())
    }

    /// The tunnel made, once, and the address's DNS record pointed at it: the
    /// address as Cloudflare made it. A name without the domain ("agents") is put
    /// on the domain the person authorized, so they never type the domain.
    pub fn set_up(&self, program: &str, address: &str) -> Result<String, String> {
        if !self.signed_in() {
            return Err("Sign in to Cloudflare first.".into());
        }
        self.prepare()?;
        let name = match self.tunnel() {
            Some(n) => n,
            None => {
                // a name of its own, so a tunnel of the same name in the account is never taken
                let n = super::secret::id("bagholder-").to_ascii_lowercase().replace('_', "-");
                let created = run::output(program, &self.args(&["create".into(), "--credentials-file".into(), self.credentials().display().to_string(), n.clone()]), &self.env(), run::WAIT);
                created.map_err(|e| plain(&e))?;
                std::fs::write(self.name_file(), &n).map_err(|e| e.to_string())?;
                n
            }
        };
        self.secure()?;
        let said = run::output(program, &self.args(&["route".into(), "dns".into(), name, address.into()]), &self.env(), run::WAIT).map_err(|e| plain(&e))?;
        made(&said).or_else(|| address.contains('.').then(|| address.to_ascii_lowercase())).ok_or_else(|| "Cloudflare didn’t say the address it made: type the whole address, such as agents.yourdomain.com.".into())
    }

    /// The sign-in `from` holds, copied here: one sign-in serves both tunnels, and
    /// each keeps its own copy so removing one never strands the other.
    pub fn take_sign_in(&self, from: &Files) -> Result<(), String> {
        if self.signed_in() {
            return Ok(());
        }
        if !from.signed_in() {
            return Err("Sign in to Cloudflare first.".into());
        }
        self.prepare()?;
        std::fs::copy(from.cert(), self.cert()).map_err(|e| format!("Cloudflare’s sign-in could not be copied: {e}"))?;
        self.secure()
    }

    /// What the sign-in lets Bagholder do at Cloudflare's API: read from the block
    /// cloudflared writes into it, and never kept anywhere else.
    pub fn access(&self) -> Option<Access> {
        access_of(&std::fs::read_to_string(self.cert()).ok()?)
    }

    /// The tunnel's id, as its credentials name it: what its address points at
    /// (`{id}.cfargotunnel.com`).
    pub fn tunnel_id(&self) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(self.credentials()).ok()?).ok()?;
        v.get("TunnelID").and_then(serde_json::Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
    }

    /// The tunnel Bagholder made, deleted at Cloudflare (its connections first:
    /// `-f`). Nothing is done where Bagholder made none.
    pub fn delete_tunnel(&self, program: &str) -> Result<(), String> {
        let Some(name) = self.tunnel() else { return Ok(()) };
        run::output(program, &self.args(&["delete".into(), "-f".into(), name]), &self.env(), run::WAIT).map(|_| ()).map_err(|e| plain(&e))
    }

    /// Everything of Bagholder's for Cloudflare on this computer gone: the sign-in,
    /// the tunnel's credentials, its name and its config. The connector Bagholder
    /// installed in the folder (`install::path`) stays: the page's own tunnel runs it too.
    pub fn forget(&self) -> Result<(), String> {
        let gone = |r: std::io::Result<()>, p: &Path| match r {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{} could not be removed: {e}", p.display())),
            _ => Ok(()),
        };
        let signed = self.dir.join(".cloudflared");
        gone(std::fs::remove_dir_all(&signed), &signed)?;
        for f in [self.credentials(), self.name_file(), self.config()] {
            gone(std::fs::remove_file(&f), &f)?;
        }
        // the folder itself, once nothing else is in it
        if std::fs::read_dir(&self.dir).is_ok_and(|mut d| d.next().is_none()) {
            return gone(std::fs::remove_dir(&self.dir), &self.dir);
        }
        Ok(())
    }

    /// The command that runs the tunnel Bagholder made, to `target`.
    pub fn run(&self, program: &str, target: &str) -> Option<Vec<String>> {
        let name = self.tunnel()?;
        let rest = vec!["run".into(), "--credentials-file".into(), self.credentials().display().to_string(), "--url".into(), format!("http://{target}"), name];
        Some(std::iter::once(program.to_string()).chain(self.args(&rest)).collect())
    }
}

/// The domain the sign-in was made for, its account, and the token cloudflared was
/// given for Cloudflare's API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    pub zone: String,
    pub token: String,
    /// The account the domain is in.
    pub account: String,
}

/// The access a sign-in holds: its `ARGO TUNNEL TOKEN` block, base64 of JSON
/// naming the zone and the API token.
pub fn access_of(pem: &str) -> Option<Access> {
    let body = pem.split("-----BEGIN ARGO TUNNEL TOKEN-----").nth(1)?.split("-----END ARGO TUNNEL TOKEN-----").next()?;
    let v: serde_json::Value = serde_json::from_slice(&base64(body)?).ok()?;
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
    Some(Access { zone: text("zoneID")?, token: text("apiToken")?, account: text("accountID")? })
}

/// Standard base64, whitespace between its lines skipped; none where it is not.
fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | u32::from(value(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The address `route dns` says it made, or found already made for this tunnel.
pub fn made(said: &str) -> Option<String> {
    let host = |w: &str| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-').to_ascii_lowercase();
    said.lines().find_map(|l| {
        if let Some(rest) = l.split("Added CNAME ").nth(1) {
            return rest.split_whitespace().next().map(host);
        }
        let i = l.find(" is already configured to route to your tunnel")?;
        l[..i].split_whitespace().last().map(host)
    })
    .filter(|h| h.contains('.'))
}

/// The page a line of `tunnel login` asks the person to open.
pub fn sign_in_url(line: &str) -> Option<String> {
    line.split_whitespace().find(|w| w.starts_with("https://dash.cloudflare.com/")).map(str::to_string)
}

/// What Cloudflare said, in plain words: the address taken, an address on a domain
/// that is not the one signed in with, else its own last line.
pub fn plain(said: &str) -> String {
    let l = said.to_ascii_lowercase();
    if l.contains("already exists") && l.contains("record") {
        "That name is already used on your domain: use a different name.".into()
    } else if l.contains("zone") && (l.contains("not found") || l.contains("could not find") || l.contains("does not belong")) {
        "That address isn’t on the domain you signed in with: use a name that ends in that domain.".into()
    } else {
        format!("Cloudflare said: {}", said.lines().rev().find(|x| !x.trim().is_empty()).unwrap_or(said).trim())
    }
}
