//! Just enough of the legacy Excel container to read one published table.
//!
//! CIRO posts its short position report as a .xls of the 1997 kind: an OLE
//! compound file holding a stream of BIFF records. Only what such a report
//! uses is implemented -- text cells, numbers, and the packed number runs --
//! and every other record is skipped, so this reads a table someone published
//! and is in no sense a spreadsheet engine.

use serde_json::{json, Value};
use std::collections::HashMap;

const LABEL: u16 = 0x0204;
const NUMBER: u16 = 0x0203;
const RK: u16 = 0x027E;
const MULRK: u16 = 0x00BD;
const SST: u16 = 0x00FC;
const CONTINUE: u16 = 0x003C;
const LABELSST: u16 = 0x00FD;

const MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

fn u16_at(b: &[u8], off: usize) -> u16 {
    if off + 2 > b.len() { 0 } else { u16::from_le_bytes([b[off], b[off + 1]]) }
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    if off + 4 > b.len() { -1 } else { i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]) }
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    if off + 4 > b.len() { 0 } else { u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]) }
}

fn f64_at(b: &[u8], off: usize) -> f64 {
    if off + 8 > b.len() {
        return 0.0;
    }
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    f64::from_le_bytes(a)
}

/// `xls.streams`: the named streams of an OLE compound file.
pub fn streams(raw: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    if raw.len() < 512 || raw[..8] != MAGIC {
        return Err("not an OLE compound file".into());
    }
    let ssz = 1usize << u16_at(raw, 30);
    let msz = 1usize << u16_at(raw, 32);
    let nfat = i32_at(raw, 44);
    let dirstart = i32_at(raw, 48);
    let minicut = u32_at(raw, 56);
    let ministart = i32_at(raw, 60);
    let difstart = i32_at(raw, 68);
    let ndif = i32_at(raw, 72);
    if ssz == 0 || ssz > raw.len() {
        return Err("sector size out of range".into());
    }

    // A sector past the end of the file reads as the bytes that are there,
    // as a slice does in Python; only a table that has to be unpacked from a
    // sector cut short is refused, which is where Python's `unpack_from` raises.
    let sector = |i: i32| -> Result<&[u8], String> {
        let from = (512 + (i.max(0) as usize) * ssz).min(raw.len());
        let from = if i < 0 { raw.len() } else { from };
        Ok(&raw[from..(from + ssz).min(raw.len())])
    };
    let whole = |blk: &[u8]| -> Result<(), String> {
        if blk.len() < ssz { Err("truncated compound file".into()) } else { Ok(()) }
    };

    // the sectors holding the allocation table, listed in the header and then
    // chained
    let mut difat: Vec<i32> = (0..109).map(|k| i32_at(raw, 76 + k * 4)).collect();
    let mut node = difstart;
    let mut left = ndif;
    let mut walked = 0usize;
    while left > 0 && node >= 0 && walked < 1 << 20 {
        let blk = sector(node)?;
        whole(blk)?;
        for k in 0..(ssz / 4 - 1) {
            difat.push(i32_at(blk, k * 4));
        }
        node = i32_at(blk, ssz - 4);
        left -= 1;
        walked += 1;
    }
    let mut fat: Vec<i32> = Vec::new();
    for f in difat.iter().take(nfat.max(0) as usize) {
        if *f >= 0 {
            let blk = sector(*f)?;
            whole(blk)?;
            for k in 0..(ssz / 4) {
                fat.push(i32_at(blk, k * 4));
            }
        }
    }

    let chain = |start: i32| -> Vec<i32> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut i = start;
        while i >= 0 && (i as usize) < fat.len() && seen.insert(i) {
            out.push(i);
            i = fat[i as usize];
        }
        out
    };

    let cat = |ids: Vec<i32>| -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        for i in ids {
            buf.extend_from_slice(sector(i)?);
        }
        Ok(buf)
    };

    let dirdata = cat(chain(dirstart))?;
    let mut entries: Vec<(String, u8, i32, u32)> = Vec::new();
    let mut off = 0usize;
    while off + 128 <= dirdata.len() {
        let e = &dirdata[off..off + 128];
        let nlen = u16_at(e, 64) as usize;
        let take = nlen.saturating_sub(2).min(64);
        let units: Vec<u16> = (0..take / 2).map(|k| u16_at(e, k * 2)).collect();
        let name = String::from_utf16_lossy(&units);
        entries.push((name, e[66], i32_at(e, 116), u32_at(e, 120)));
        off += 128;
    }

    // a short stream lives inside the root entry's own stream, cut into
    // smaller pieces
    let root = entries.iter().find(|e| e.1 == 5).cloned();
    let mini = match &root {
        Some(r) if r.2 >= 0 => cat(chain(r.2))?,
        _ => vec![],
    };
    let mut minifat: Vec<i32> = Vec::new();
    if ministart >= 0 {
        let blk = cat(chain(ministart))?;
        for k in 0..(blk.len() / 4) {
            minifat.push(i32_at(&blk, k * 4));
        }
    }

    let mut out = HashMap::new();
    for (name, kind, start, size) in entries {
        if kind != 2 {
            continue;
        }
        let mut buf: Vec<u8>;
        if size < minicut && !minifat.is_empty() {
            buf = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut i = start;
            while i >= 0 && (i as usize) < minifat.len() && seen.insert(i) {
                let from = (i as usize) * msz;
                let to = (from + msz).min(mini.len());
                if from < mini.len() {
                    buf.extend_from_slice(&mini[from..to]);
                }
                i = minifat[i as usize];
            }
        } else {
            buf = cat(chain(start))?;
        }
        buf.truncate(size as usize);
        out.insert(name, buf);
    }
    Ok(out)
}

/// `xls.records`: the stream's records, as (id, body).
pub fn records(buf: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= buf.len() {
        let rid = u16_at(buf, i);
        let ln = u16_at(buf, i + 2) as usize;
        let to = (i + 4 + ln).min(buf.len());
        out.push((rid, &buf[i + 4..to]));
        i += 4 + ln;
    }
    out
}

/// `xls._number`: a packed number -- the low two bits are flags, the rest is
/// either a 30-bit signed integer or the top half of a double, and bit 0 means
/// the value is in hundredths.
fn number(v: u32) -> f64 {
    let x = if v & 2 != 0 {
        let n = v >> 2;
        if n & (1 << 29) != 0 { (n as i64 - (1i64 << 30)) as f64 } else { n as f64 }
    } else {
        f64::from_bits(((v & 0xFFFF_FFFC) as u64) << 32)
    };
    if v & 1 != 0 { x / 100.0 } else { x }
}

/// A record read that needs bytes the record does not have: where Python's
/// `unpack_from` raises, a cut-short workbook is refused rather than read as
/// zeros.
fn need(data: &[u8], upto: usize) -> Result<(), String> {
    if upto > data.len() { Err("truncated record".into()) } else { Ok(()) }
}

/// `xls._text`: a length, a flag saying how wide its characters are, then the
/// characters.
fn text(data: &[u8], off: usize) -> Result<String, String> {
    need(data, off + 3)?;
    let n = u16_at(data, off) as usize;
    let wide = data[off + 2] & 1 != 0;
    let from = off + 3;
    let to = (from + n * if wide { 2 } else { 1 }).min(data.len());
    Ok(decode(&data[from..to], wide))
}

/// Python's `bytes.decode(..., "replace")` for the two widths the format uses.
fn decode(body: &[u8], wide: bool) -> String {
    if wide {
        let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let mut s = String::from_utf16_lossy(&units);
        // an odd trailing byte is a truncated unit, which Python replaces
        if body.len() % 2 == 1 {
            s.push('\u{fffd}');
        }
        s
    } else {
        // latin-1: every byte is its own code point
        body.iter().map(|b| *b as char).collect()
    }
}

/// `xls._shared`: the workbook's shared strings, which cells then refer to by
/// number.
fn shared(chunks: &[&[u8]]) -> Result<Vec<String>, String> {
    let mut buf: Vec<u8> = Vec::new();
    for c in chunks {
        buf.extend_from_slice(c);
    }
    need(&buf, 8)?;
    let count = i32_at(&buf, 4);
    let mut pos: i64 = 8;
    let mut out = Vec::new();
    for _ in 0..count.max(0) {
        if pos < 0 || pos as usize + 3 > buf.len() {
            break;
        }
        let mut p = pos as usize;
        let n = u16_at(&buf, p) as usize;
        let flags = buf[p + 2];
        p += 3;
        let rich = if flags & 8 != 0 { need(&buf, p + 2)?; u16_at(&buf, p) as i64 } else { 0 };
        p += if flags & 8 != 0 { 2 } else { 0 };
        let far = if flags & 4 != 0 { need(&buf, p + 4)?; i32_at(&buf, p) as i64 } else { 0 };
        p += if flags & 4 != 0 { 4 } else { 0 };
        let wide = flags & 1 != 0;
        let width = n * if wide { 2 } else { 1 };
        let from = p.min(buf.len());
        let to = (p + width).min(buf.len());
        out.push(decode(&buf[from..to], wide));
        pos = p as i64 + width as i64 + rich * 4 + far;
    }
    Ok(out)
}

/// `xls.cells`: {(row, column): value} for the records this reads, ignoring
/// the rest.
pub fn cells(buf: &[u8]) -> Result<HashMap<(u16, u16), Value>, String> {
    let mut out: HashMap<(u16, u16), Value> = HashMap::new();
    let mut strings: Vec<String> = Vec::new();
    let mut collecting: Option<Vec<&[u8]>> = None;
    for (rid, data) in records(buf) {
        if rid == SST {
            collecting = Some(vec![data]);
            continue;
        }
        if rid == CONTINUE {
            if let Some(c) = collecting.as_mut() {
                c.push(data);
                continue;
            }
        }
        if let Some(c) = collecting.take() {
            strings = shared(&c)?;
        }
        match rid {
            LABEL => {
                need(data, 4)?;
                out.insert((u16_at(data, 0), u16_at(data, 2)), json!(text(data, 6)?));
            }
            LABELSST => {
                need(data, 10)?;
                let idx = i32_at(data, 6);
                let v = if idx >= 0 && (idx as usize) < strings.len() { strings[idx as usize].clone() } else { String::new() };
                out.insert((u16_at(data, 0), u16_at(data, 2)), json!(v));
            }
            NUMBER => {
                need(data, 14)?;
                out.insert((u16_at(data, 0), u16_at(data, 2)), json!(f64_at(data, 6)));
            }
            RK => {
                need(data, 10)?;
                out.insert((u16_at(data, 0), u16_at(data, 2)), json!(number(u32_at(data, 6))));
            }
            MULRK => {
                need(data, 4)?;
                let r = u16_at(data, 0);
                let first = u16_at(data, 2);
                if data.len() >= 6 {
                    for k in 0..((data.len() - 6) / 6) {
                        out.insert((r, first.wrapping_add(k as u16)), json!(number(u32_at(data, 6 + k * 6))));
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(c) = collecting {
        shared(&c)?;
    }
    Ok(out)
}

/// `xls.table`: the workbook's first stream as rows of cells, blanks filled
/// in.
pub fn table(raw: &[u8]) -> Result<Vec<Vec<Value>>, String> {
    let found = streams(raw)?;
    let buf = found
        .get("Workbook")
        .or_else(|| found.get("Book"))
        .ok_or_else(|| "no workbook stream".to_string())?;
    let grid = cells(buf)?;
    if grid.is_empty() {
        return Ok(vec![]);
    }
    let rows = grid.keys().map(|(r, _)| *r).max().unwrap_or(0);
    let cols = grid.keys().map(|(_, c)| *c).max().unwrap_or(0);
    Ok((0..=rows)
        .map(|r| (0..=cols).map(|c| grid.get(&(r, c)).cloned().unwrap_or_else(|| json!(""))).collect())
        .collect())
}
