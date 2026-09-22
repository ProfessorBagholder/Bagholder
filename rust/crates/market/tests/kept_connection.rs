//! Kept connections and the venue-keyed quote refresh:
//! the transport keeps a connection per host. Served from a loopback listener,
//! never the network.

use bagholder_market::client;
use bagholder_model::input::Listing;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Reads one request head; false when the peer is gone.
fn read_request(r: &mut BufReader<std::net::TcpStream>) -> bool {
    let mut any = false;
    loop {
        let mut line = String::new();
        match r.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        if line == "\r\n" {
            return any;
        }
        any = true;
    }
}

/// A server answering each request with `status`; `per_conn` requests are
/// served on a connection before it is dropped without saying so.
fn serve(status: u16, per_conn: usize) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let opens = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (o, q) = (opens.clone(), requests.clone());
    std::thread::spawn(move || {
        for s in l.incoming() {
            let s = match s { Ok(s) => s, Err(_) => return };
            o.fetch_add(1, Ordering::SeqCst);
            let q = q.clone();
            std::thread::spawn(move || {
                let mut w = s.try_clone().unwrap();
                let mut r = BufReader::new(s);
                for _ in 0..per_conn {
                    if !read_request(&mut r) {
                        return;
                    }
                    q.fetch_add(1, Ordering::SeqCst);
                    let _ = w.write_all(format!("HTTP/1.1 {} X\r\nContent-Length: 2\r\n\r\nok", status).as_bytes());
                }
            });
        }
    });
    (format!("http://127.0.0.1:{}/x", port), opens, requests)
}

#[test]
fn test_a_read_gives_its_connection_back_and_the_next_one_takes_it() {
    let (url, opens, requests) = serve(200, 100);
    for _ in 0..3 {
        let r = client::request("GET", &url, &[("User-Agent", "t")], None, Duration::from_secs(5)).unwrap();
        assert_eq!(r.body, b"ok");
    }
    assert_eq!(opens.load(Ordering::SeqCst), 1, "one connection for three reads");
    assert_eq!(requests.load(Ordering::SeqCst), 3);
}

#[test]
fn test_a_connection_the_server_closed_is_retried_once() {
    let (url, opens, _) = serve(200, 1);
    assert_eq!(client::request("GET", &url, &[], None, Duration::from_secs(5)).unwrap().body, b"ok");
    std::thread::sleep(Duration::from_millis(50));
    // the kept connection is closed by the peer; the read is retried on a new one
    assert_eq!(client::request("GET", &url, &[], None, Duration::from_secs(5)).unwrap().body, b"ok");
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

#[test]
fn test_a_refusal_still_reads_as_one() {
    let (url, _, _) = serve(404, 100);
    let e = client::request("GET", &url, &[], None, Duration::from_secs(5)).err().expect("an error");
    assert_eq!(e.code(), Some(404), "a symbol a source does not carry is still a 404");
}

#[test]
fn test_quote_refresh_keys_a_watched_listing_by_venue() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    let needing = bagholder_market::quotes::quote_symbols_needing_refresh(
        &conn,
        &[
            Listing::new("AAPL", "NEO", "CAD", "Shares"),
            Listing { quote_key: Some("AAPL@NASDAQ".into()), ..Listing::new("AAPL", "NASDAQ", "USD", "Shares") },
        ],
        1_800_000_000.0,
        bagholder_market::quotes::QUOTE_REFRESH_MINUTES,
    )
    .unwrap();
    let got: Vec<(String, String)> = needing.into_iter().map(|(k, s, _)| (k, s)).collect();
    assert_eq!(
        got,
        vec![("AAPL".to_string(), "cboe_ca".to_string()), ("AAPL@NASDAQ".to_string(), "yahoo_quote".to_string())],
        "the held CDR and the watched US listing keep separate quotes, each from a feed live for its market"
    );
}
