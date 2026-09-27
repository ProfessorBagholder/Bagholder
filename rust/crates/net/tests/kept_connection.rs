//! Kept connections: the transport keeps a connection per host. Served from a
//! loopback listener, never the network.

use bagholder_net::client;
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

/// A server that reads each request and then drops the connection without an
/// answer: the request reached it, the caller never hears so.
fn swallow() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let opens = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (o, q) = (opens.clone(), requests.clone());
    std::thread::spawn(move || {
        for s in l.incoming() {
            let s = match s { Ok(s) => s, Err(_) => return };
            o.fetch_add(1, Ordering::SeqCst);
            let mut r = BufReader::new(s);
            if read_request(&mut r) {
                q.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    (format!("http://127.0.0.1:{}/x", port), opens, requests)
}

#[test]
fn test_a_request_that_must_reach_its_host_at_most_once_is_never_sent_twice() {
    let (url, opens, requests) = swallow();
    let e = client::request_once("POST", &url, &[], Some(b"{}"), Duration::from_secs(5));
    assert!(e.is_err(), "no answer is an error for the caller to settle");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!((opens.load(Ordering::SeqCst), requests.load(Ordering::SeqCst)), (1, 1), "sent once, on one connection");
}

#[test]
fn test_a_request_that_must_reach_its_host_at_most_once_never_takes_a_kept_connection() {
    let (url, opens, requests) = serve(200, 1);
    // a kept connection the peer then closes
    assert_eq!(client::request("GET", &url, &[], None, Duration::from_secs(5)).unwrap().body, b"ok");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(client::request_once("POST", &url, &[], Some(b"{}"), Duration::from_secs(5)).unwrap().body, b"ok");
    assert_eq!((opens.load(Ordering::SeqCst), requests.load(Ordering::SeqCst)), (2, 2), "a fresh connection, one request, no retry on the stale one");
}

/// A server answering every request on a connection with `reply`, as many as come.
fn serve_raw(reply: &'static str) -> (String, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let opens = Arc::new(AtomicUsize::new(0));
    let o = opens.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let s = match s { Ok(s) => s, Err(_) => return };
            o.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut w = s.try_clone().unwrap();
                let mut r = BufReader::new(s);
                while read_request(&mut r) {
                    if w.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (format!("http://127.0.0.1:{}/x", port), opens)
}

#[test]
fn test_a_chunked_reply_is_read_to_its_last_line_and_its_connection_reads_the_next_one() {
    // two chunks, the last-chunk, a trailer field and the empty line that ends the body
    let (url, opens) = serve_raw("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n3;x=y\r\n!!!\r\n0\r\nX-Trailer: t\r\n\r\n");
    for _ in 0..3 {
        let r = client::request("GET", &url, &[], None, Duration::from_secs(5)).unwrap();
        assert_eq!(r.body, b"ok!!!");
    }
    assert_eq!(opens.load(Ordering::SeqCst), 1, "the kept connection holds nothing of the last reply: the next one reads there");
}

#[test]
fn test_a_reply_cut_off_before_its_end_is_an_error_not_a_shorter_body() {
    // the body announced as 10 bytes, 2 sent, then the connection closed
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming().take(2) {
            let s = s.unwrap();
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            if read_request(&mut r) {
                let _ = w.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nok");
            }
        }
    });
    let e = client::request("GET", &format!("http://127.0.0.1:{}/x", port), &[], None, Duration::from_secs(5)).err().expect("a cut-off reply is an error");
    assert!(e.to_string().contains("before the end of the body"), "{e}");
    // a chunked body cut off the same way
    let (url, _) = serve_raw("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nok");
    assert!(client::request("GET", &url, &[], None, Duration::from_secs(5)).is_err());
}
