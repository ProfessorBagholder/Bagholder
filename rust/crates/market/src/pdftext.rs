//! PDF text for filing summaries.
//!
//! Issuer PDFs store their text as subsetted-font glyph codes a naive reader
//! cannot turn back into words, so a real engine is needed. A system
//! `pdftotext` (poppler) is used when present, otherwise the
//! `pdf-extract` crate reads it. The two engines lay text out differently, so the words a
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

/// An engine can read a PDF now. The built-in one always
/// can.
pub fn available() -> bool {
    !disabled()
}

pub fn status() -> &'static str {
    if available() { "ready" } else { "off" }
}

/// Nothing is ever still being provisioned here.
pub fn pending() -> bool {
    false
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// Readable text from a PDF's bytes, or "" when the bytes are
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
    // text
    let owned = data.to_vec();
    match std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&owned)) {
        Ok(Ok(t)) => t.trim().to_string(),
        _ => String::new(),
    }
}

/// `pdftotext`'s text, or None where it could not give all of it (the PDF not
/// written to it whole, its output not read whole, thirty seconds passed), and
/// the built-in engine reads the PDF instead.
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
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let started = Instant::now();
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut s = stdout;
        std::io::Read::read_to_end(&mut s, &mut buf).map(|_| buf)
    });
    // thirty seconds at most
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                #[expect(clippy::let_underscore_must_use, reason = "it may have exited since it was last asked; either way it is given up on")]
                let _ = child.kill();
                #[expect(clippy::let_underscore_must_use, reason = "reaping a process given up on; its status answers nothing")]
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    // text from a PDF it was not given whole, or not read whole, is not its text
    writer.join().ok()?.ok()?;
    let out = reader.join().ok()?.ok()?;
    Some(String::from_utf8_lossy(&out).into_owned())
}
