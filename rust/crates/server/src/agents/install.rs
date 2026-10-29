//! A way's program installed by Bagholder on the computer it runs on. The page
//! may be open on another device, whose download would land there instead, so
//! Bagholder fetches the maker's own build for this system and processor, from the
//! maker's own host, into its folder for that way: no administrator, nothing of the
//! computer's touched, and Bagholder runs it from there. ngrok and Cloudflare's
//! connector are each one program file; Tailscale is not one of these, as it runs
//! as the system's service.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::tunnel::{run, Provider};

/// How a maker ships its program: the file itself, or an archive holding it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packed {
    Bare,
    Tgz,
    Zip,
}

/// A build: where it is, how it is packed, and the hosts it may come from on
/// every hop of a redirect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub packed: Packed,
    pub hosts: &'static [&'static str],
}

/// The largest build taken: ngrok's and cloudflared's are 10 to 40 MB.
const CEILING: u64 = 200 * 1024 * 1024;
/// How long a download or an unpacking may take.
const WAIT: Duration = Duration::from_secs(300);
const NGROK: &[&str] = &["bin.equinox.io"];
/// A GitHub release asset: github.com, then the download host it redirects to.
const GITHUB: &[&str] = &["github.com", "githubusercontent.com"];

fn maker(provider: Provider) -> &'static str {
    match provider {
        Provider::Ngrok => "ngrok",
        Provider::Cloudflare => "Cloudflare",
        _ => "Its maker",
    }
}

/// The maker's build of `provider`'s program for `os` and `arch` (as
/// `std::env::consts` names them), or why there is none.
pub fn source(provider: Provider, os: &str, arch: &str) -> Result<Source, String> {
    let none = || format!("{} has no build for this computer ({os}, {arch}).", maker(provider));
    let cpu = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return Err(none()),
    };
    let ngrok = |name: &str, packed| Source { url: format!("https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-{name}-{cpu}.{}", if packed == Packed::Tgz { "tgz" } else { "zip" }), packed, hosts: NGROK };
    let cloudflared = |file: String, packed| Source { url: format!("https://github.com/cloudflare/cloudflared/releases/latest/download/{file}"), packed, hosts: GITHUB };
    match (provider, os) {
        (Provider::Ngrok, "linux") => Ok(ngrok("linux", Packed::Tgz)),
        (Provider::Ngrok, "macos") => Ok(ngrok("darwin", Packed::Zip)),
        (Provider::Ngrok, "windows") => Ok(ngrok("windows", Packed::Zip)),
        (Provider::Cloudflare, "linux") => Ok(cloudflared(format!("cloudflared-linux-{cpu}"), Packed::Bare)),
        (Provider::Cloudflare, "macos") => Ok(cloudflared(format!("cloudflared-darwin-{cpu}.tgz"), Packed::Tgz)),
        // Cloudflare builds no connector for Windows on ARM
        (Provider::Cloudflare, "windows") if cpu == "amd64" => Ok(cloudflared(format!("cloudflared-windows-{cpu}.exe"), Packed::Bare)),
        _ => Err(none()),
    }
}

/// Whether `host` is one a build of `source` may come from.
pub fn allowed(source: &Source, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    source.hosts.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// The program's file name on this system.
fn exe(provider: Provider) -> Option<String> {
    let name = match provider {
        Provider::Ngrok => "ngrok",
        Provider::Cloudflare => "cloudflared",
        _ => return None,
    };
    Some(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
}

/// Where Bagholder keeps its own copy of `provider`'s program: beside that way's
/// other files.
pub fn path(home: &Path, provider: Provider) -> Option<PathBuf> {
    let dir = match provider {
        Provider::Ngrok => home.join("agents-ngrok"),
        Provider::Cloudflare => home.join("agents-cloudflare"),
        _ => return None,
    };
    exe(provider).map(|e| dir.join(e))
}

/// A build downloaded into `file`, from the source's hosts alone on every hop.
pub type Download<'a> = &'a dyn Fn(&Source, &mut File) -> Result<(), String>;

/// The download as Bagholder makes it: streamed to disk, no larger than `CEILING`.
pub fn download(source: &Source, file: &mut File) -> Result<(), String> {
    let ua = format!("Bagholder/{}", crate::app::APP_VERSION);
    bagholder_net::client::download(&source.url, &[("User-Agent", &ua)], WAIT, file, CEILING, &|host| allowed(source, host), &mut |_| true).map(|_| ()).map_err(|e| e.to_string())
}

/// `provider`'s program installed in Bagholder's folder from `source`: downloaded
/// beside it, taken from its archive when packed, made runnable, then put in place
/// whole. Nothing is left behind when it fails.
pub fn install(home: &Path, provider: Provider, source: &Source, download: Download) -> Result<PathBuf, String> {
    let target = path(home, provider).ok_or("Bagholder installs no program for this way.")?;
    let dir = target.parent().ok_or("no folder")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} could not be made: {e}", dir.display()))?;
    let part = dir.join(".download");
    let unpacked = dir.join(".unpacked");
    let result = (|| {
        let mut f = File::create(&part).map_err(|e| format!("{} could not be written: {e}", part.display()))?;
        download(source, &mut f)?;
        drop(f);
        let program = match source.packed {
            Packed::Bare => part.clone(),
            Packed::Tgz | Packed::Zip => unpack(&part, source.packed, &unpacked, provider)?,
        };
        runnable(&program)?;
        std::fs::rename(&program, &target).map_err(|e| format!("{} could not be put in place: {e}", target.display()))?;
        Ok(target.clone())
    })();
    // what was downloaded or unpacked goes, whatever came of the install
    #[expect(clippy::let_underscore_must_use, reason = "a part not there is gone already; one left is replaced by the next install, never read")]
    let _ = std::fs::remove_file(&part);
    #[expect(clippy::let_underscore_must_use, reason = "as the part: what is unpacked is made again for each install")]
    let _ = std::fs::remove_dir_all(&unpacked);
    result
}

/// The program from `archive`, which holds it and nothing else: an archive with
/// any other entry is refused before anything is taken from it.
fn unpack(archive: &Path, packed: Packed, into: &Path, provider: Provider) -> Result<PathBuf, String> {
    let want = exe(provider).ok_or("no program")?;
    let (list, take) = if packed == Packed::Tgz { ("-tzf", "-xzf") } else { ("-tf", "-xf") };
    let archive_s = archive.display().to_string();
    let listed = run::output("tar", &[list.to_string(), archive_s.clone()], &[], WAIT)?;
    let entries: Vec<&str> = listed.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if entries != [want.as_str()] {
        return Err(format!("{}'s download held something other than {want}: nothing was installed.", maker(provider)));
    }
    #[expect(clippy::let_underscore_must_use, reason = "an earlier unpacking left behind, or none: the folder is made new just below either way")]
    let _ = std::fs::remove_dir_all(into);
    std::fs::create_dir_all(into).map_err(|e| format!("{} could not be made: {e}", into.display()))?;
    run::output("tar", &[take.to_string(), archive_s, "-C".to_string(), into.display().to_string()], &[], WAIT)?;
    let program = into.join(&want);
    match std::fs::symlink_metadata(&program) {
        Ok(m) if m.is_file() => Ok(program),
        _ => Err(format!("{}'s download held no {want}: nothing was installed.", maker(provider))),
    }
}

fn runnable(program: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("{} could not be made runnable: {e}", program.display()))?;
    }
    #[cfg(not(unix))]
    let _ = program;
    Ok(())
}
