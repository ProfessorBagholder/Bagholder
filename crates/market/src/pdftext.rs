//! PDF text for filing summaries.
//!
//! Issuer PDFs store their text as subsetted-font glyph codes a naive reader
//! cannot turn back into words, so a real engine is needed. A system
//! `pdftotext` (poppler) is used when present, exactly as the Python app does;
//! otherwise the `pdf-extract` crate reads it, where the Python app uses
//! pdfminer.six. The two engines lay text out differently, so the words a
//! summary is made from can differ between them; the subject, read from the
//! PDF's own metadata, does not.
//!
//! `BAGHOLDER_NO_PDF=1` disables extraction.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn disabled() -> bool {
    std::env::var("BAGHOLDER_NO_PDF").map(|v| !v.is_empty()).unwrap_or(false)
}

/// `pdftext.available`: an engine can read a PDF now. The built-in one always
/// can.
pub fn available() -> bool {
    !disabled()
}

/// `pdftext.status`.
pub fn status() -> &'static str {
    if available() { "ready" } else { "off" }
}

/// `pdftext.pending`: nothing is ever still being provisioned here.
pub fn pending() -> bool {
    false
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// `pdftext.text`: readable text from a PDF's bytes, or "" when the bytes are
/// not a PDF or nothing can be read from them. Never fails.
pub fn text(data: &[u8]) -> String {
    if disabled() || !data.starts_with(b"%PDF-") {
        return String::new();
    }
    if let Some(exe) = which("pdftotext") {
        if let Some(out) = run_pdftotext(&exe, data) {
            let got = out.trim().to_string();
            if !got.is_empty() {
                return got;
            }
        }
    }
    // the reader can panic on a malformed file; a filing it cannot read has no
    // text, as a pdfminer exception does in Python
    let owned = data.to_vec();
    match std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&owned)) {
        Ok(Ok(t)) => t.trim().to_string(),
        _ => String::new(),
    }
}

fn run_pdftotext(exe: &std::path::Path, data: &[u8]) -> Option<String> {
    let mut child = Command::new(exe)
        .args(["-q", "-nopgbrk", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let bytes = data.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let started = Instant::now();
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut s = stdout;
        let _ = std::io::Read::read_to_end(&mut s, &mut buf);
        buf
    });
    // `timeout=30`, as the Python call gives it
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    let _ = writer.join();
    let out = reader.join().ok()?;
    Some(String::from_utf8_lossy(&out).into_owned())
}
