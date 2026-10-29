//! ngrok set up by Bagholder (brief 81 §3): the person pastes their authtoken;
//! Bagholder keeps it in a config of its own and runs ngrok with it, on the dev
//! domain every ngrok account is given ("When you start an endpoint without a URL
//! … ngrok puts it on your dev domain automatically", ngrok's docs), read from
//! what ngrok says as it starts. ngrok's own config elsewhere is never read.

use std::path::{Path, PathBuf};

use super::tunnel::run;

pub struct Files {
    dir: PathBuf,
}

impl Files {
    pub fn new(home: &Path) -> Files {
        Files { dir: home.join("agents-ngrok") }
    }

    fn config(&self) -> PathBuf {
        self.dir.join("ngrok.yml")
    }

    /// Whether the person's authtoken is kept.
    pub fn has_token(&self) -> bool {
        self.config().is_file()
    }

    /// The authtoken kept, by ngrok itself, in Bagholder's config.
    pub fn save_token(&self, program: &str, token: &str) -> Result<(), String> {
        let token = token.trim();
        if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("That isn't an ngrok authtoken: copy it from ngrok’s page again.".into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let args = vec!["config".into(), "add-authtoken".into(), token.into(), "--config".into(), self.config().display().to_string()];
        run::output(program, &args, &[], run::WAIT).map(|_| ()).map_err(|e| format!("ngrok said: {}", e.lines().rev().find(|x| !x.trim().is_empty()).unwrap_or(&e).trim()))
    }

    /// The command that runs ngrok to `target`, on the account's dev domain, saying
    /// its address in a line Bagholder reads.
    pub fn run(&self, program: &str, target: &str) -> Option<Vec<String>> {
        if !self.has_token() {
            return None;
        }
        Some([program, "http", "--config", &self.config().display().to_string(), "--log", "stdout", "--log-format", "logfmt", &format!("http://{target}")].iter().map(|s| s.to_string()).collect())
    }
}

/// The host a line of ngrok's log says it serves (`url=https://{host}`).
pub fn address(line: &str) -> Option<String> {
    let rest = line.split("url=https://").nth(1)?;
    let host: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
    (host.contains('.')).then(|| host.to_ascii_lowercase())
}
