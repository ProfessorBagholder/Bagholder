//! Port of tests/test_xls.py.
use bagholder_market::xls;
use serde_json::{json, Value};

fn record(rid: u16, body: &[u8]) -> Vec<u8> {
    let mut v = rid.to_le_bytes().to_vec();
    v.extend((body.len() as u16).to_le_bytes());
    v.extend(body);
    v
}
fn hdr(row: u16, col: u16) -> Vec<u8> { [row.to_le_bytes(), col.to_le_bytes(), 0u16.to_le_bytes()].concat() }
fn label(row: u16, col: u16, text: &str, wide: bool) -> Vec<u8> {
    let mut b = hdr(row, col);
    b.extend((text.chars().count() as u16).to_le_bytes());
    b.push(wide as u8);
    if wide { for u in text.encode_utf16() { b.extend(u.to_le_bytes()); } } else { b.extend(text.chars().map(|c| c as u8)); }
    record(xls::LABEL, &b)
}
fn number(row: u16, col: u16, v: f64) -> Vec<u8> { let mut b = hdr(row, col); b.extend(v.to_le_bytes()); record(xls::NUMBER, &b) }
fn packed(v: i64, cents: bool) -> u32 { (((v << 2) | 2 | cents as i64) & 0xFFFF_FFFF) as u32 }
fn rk(row: u16, col: u16, raw: u32) -> Vec<u8> { let mut b = hdr(row, col); b.extend(raw.to_le_bytes()); record(xls::RK, &b) }
fn mulrk(row: u16, first: u16, raws: &[u32]) -> Vec<u8> {
    let mut b = [row.to_le_bytes(), first.to_le_bytes()].concat();
    for r in raws { b.extend(0u16.to_le_bytes()); b.extend(r.to_le_bytes()); }
    b.extend((first + raws.len() as u16 - 1).to_le_bytes());
    record(xls::MULRK, &b)
}
fn sst(strings: &[&str]) -> Vec<u8> {
    let mut b = [(strings.len() as i32).to_le_bytes(), (strings.len() as i32).to_le_bytes()].concat();
    for s in strings { b.extend((s.len() as u16).to_le_bytes()); b.push(0); b.extend(s.as_bytes()); }
    record(xls::SST, &b)
}
fn labelsst(row: u16, col: u16, idx: i32) -> Vec<u8> { let mut b = hdr(row, col); b.extend(idx.to_le_bytes()); record(xls::LABELSST, &b) }

fn put_i32(buf: &mut [u8], at: usize, v: i32) { buf[at..at + 4].copy_from_slice(&v.to_le_bytes()); }
fn put_u16(buf: &mut [u8], at: usize, v: u16) { buf[at..at + 2].copy_from_slice(&v.to_le_bytes()); }

fn container(stream: &[u8], name: &str) -> Vec<u8> {
    let sectors = (stream.len() + 511) / 512;
    let mut data = stream.to_vec();
    data.resize(sectors * 512, 0);
    let mut header = vec![0u8; 512];
    header[0..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    put_u16(&mut header, 28, 0xFFFE);
    put_u16(&mut header, 30, 9);
    put_u16(&mut header, 32, 6);
    put_i32(&mut header, 44, 1);
    put_i32(&mut header, 48, 1);
    put_i32(&mut header, 56, 4096);
    put_i32(&mut header, 60, -2);
    put_i32(&mut header, 68, -2);
    put_i32(&mut header, 72, 0);
    for i in 0..109 { put_i32(&mut header, 76 + i * 4, if i == 0 { 0 } else { -1 }); }
    let mut table = vec![0xffu8; 512];
    put_i32(&mut table, 0, -3);
    put_i32(&mut table, 4, -2);
    for i in 0..sectors { put_i32(&mut table, 8 + i * 4, if i == sectors - 1 { -2 } else { 3 + i as i32 }); }
    let mut dir = vec![0u8; 512];
    let mut entry = |at: usize, n: &str, kind: u8, start: i32, size: u32| {
        let raw: Vec<u8> = n.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        dir[at..at + raw.len()].copy_from_slice(&raw);
        put_u16(&mut dir, at + 64, raw.len() as u16 + 2);
        dir[at + 66] = kind;
        put_i32(&mut dir, at + 116, start);
        dir[at + 120..at + 124].copy_from_slice(&size.to_le_bytes());
    };
    entry(0, "Root Entry", 5, -2, 0);
    entry(128, name, 2, 2, stream.len() as u32);
    [header, table, dir, data].concat()
}

#[test]
fn test_text_numbers_and_packed_runs_all_land_in_their_cells() {
    let half = u32::from_le_bytes(2.0f64.to_le_bytes()[4..8].try_into().unwrap()) & 0xFFFF_FFFC;
    let stream = [label(0, 0, "Security Symbol", false), label(0, 1, "Exchange Code", false), label(1, 0, "QNC", false), label(1, 1, "TSXV", false),
                  mulrk(1, 2, &[packed(2667164, false), packed(64077, false)]), number(2, 0, 1.5), rk(2, 1, packed(250, true)), rk(2, 2, half)].concat();
    let c = xls::cells(&stream).unwrap();
    assert_eq!(c[&(0, 0)], json!("Security Symbol"));
    assert_eq!(c[&(1, 0)], json!("QNC"));
    assert_eq!(c[&(1, 2)].as_f64(), Some(2667164.0));
    assert_eq!(c[&(1, 3)].as_f64(), Some(64077.0));
    assert_eq!(c[&(2, 0)].as_f64(), Some(1.5));
    assert_eq!(c[&(2, 1)].as_f64(), Some(2.5));
    assert_eq!(c[&(2, 2)].as_f64(), Some(2.0));
}

#[test]
fn test_a_negative_packed_number_keeps_its_sign() {
    assert_eq!(xls::cells(&rk(0, 0, packed(-110130, false))).unwrap()[&(0, 0)].as_f64(), Some(-110130.0));
}

#[test]
fn test_wide_text_reads_as_written() {
    assert_eq!(xls::cells(&label(0, 0, "1911 GOLD", true)).unwrap()[&(0, 0)], json!("1911 GOLD"));
}

#[test]
fn test_shared_strings_are_read_through_the_cells_that_point_at_them() {
    let stream = [sst(&["ZYUS LIFE SCIENCES", "ZYUS"]), labelsst(0, 0, 0), labelsst(0, 1, 1), labelsst(0, 2, 9)].concat();
    let c = xls::cells(&stream).unwrap();
    assert_eq!(c[&(0, 0)], json!("ZYUS LIFE SCIENCES"));
    assert_eq!(c[&(0, 1)], json!("ZYUS"));
    assert_eq!(c[&(0, 2)], json!(""));
}

#[test]
fn test_records_it_does_not_read_are_skipped_rather_than_breaking_the_row() {
    let stream = [record(0x0208, &[0u8; 16]), label(0, 0, "ONE", false), record(0x00E0, &[1u8; 20]), rk(0, 1, packed(395141, false))].concat();
    let c = xls::cells(&stream).unwrap();
    assert_eq!(c[&(0, 0)], json!("ONE"));
    assert_eq!(c[&(0, 1)].as_f64(), Some(395141.0));
}

#[test]
fn test_the_workbook_stream_is_found_and_read_as_a_table() {
    let mut stream = [label(0, 0, "Security Issue Name", false), label(0, 1, "Security Symbol", false), label(0, 2, "Exchange Code", false),
                      label(1, 0, "HIGH TIDE INC.", false), label(1, 1, "HITI", false), label(1, 2, "TSXV", false),
                      mulrk(1, 3, &[packed(124186, false), packed(-10870, false)])].concat();
    stream.extend(vec![0u8; 5000]);
    let rows = xls::table(&container(&stream, "Workbook")).unwrap();
    assert_eq!(rows[0], vec![json!("Security Issue Name"), json!("Security Symbol"), json!("Exchange Code"), json!(""), json!("")]);
    let r1: Vec<Value> = rows[1].iter().map(|v| v.as_f64().map(|f| json!(f)).unwrap_or(v.clone())).collect();
    assert_eq!(r1, vec![json!("HIGH TIDE INC."), json!("HITI"), json!("TSXV"), json!(124186.0), json!(-10870.0)]);
}

#[test]
fn test_something_that_is_not_a_container_says_so() {
    assert!(xls::table(b"Security,Symbol\nHITI,TSXV\n").is_err());
}

#[test]
fn test_a_container_without_a_workbook_says_so() {
    assert!(xls::table(&container(&[0u8; 5000], "Nothing")).is_err());
}
