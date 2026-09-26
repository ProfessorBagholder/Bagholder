//! Embeds the built page (`web/dist`) in the binary, so a release is one file
//! that serves its own page. With no `web/dist` at build time (a checkout where
//! the page has not been built) nothing is embedded and the server reads the
//! folder from disk at run time instead.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let dist = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed={}", dist.join("index.html").display());
    let mut files = Vec::new();
    files_under(&dist, &mut files);
    files.sort();
    let mut src = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        let Ok(full) = f.canonicalize() else { continue };
        let name = f.strip_prefix(&dist).unwrap().to_string_lossy().replace('\\', "/");
        writeln!(src, "    ({:?}, include_bytes!({:?})),", name, full.display().to_string()).unwrap();
    }
    src.push_str("];\n");
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded_page.rs"), src).unwrap();
}
