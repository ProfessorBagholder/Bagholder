//! Asking through [`Net`]: every host through the one limiter, on the clock
//! handed in, a refusal resting the host for as long as it says. Served from a
//! loopback listener, never the network.

use bagholder_net::{host_of, retry_after, Ask, Limiter, ManualClock, Net, NetError, Pace};
use jiff::Timestamp;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A server answering every request with `head` (status line and headers) and
/// an empty body, counting what it is asked.
fn serve(head: &'static str) -> (String, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let asked = Arc::new(AtomicUsize::new(0));
    let a = asked.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(s) = s else { return };
            let a = a.clone();
            std::thread::spawn(move || {
                let mut w = s.try_clone().unwrap();
                let mut r = BufReader::new(s);
                loop {
                    let mut any = false;
                    loop {
                        let mut line = String::new();
                        match r.read_line(&mut line) {
                            Ok(0) | Err(_) => return,
                            Ok(_) => {}
                        }
                        if line == "\r\n" {
                            break;
                        }
                        any = true;
                    }
                    if !any {
                        return;
                    }
                    a.fetch_add(1, Ordering::SeqCst);
                    let _ = w.write_all(format!("{head}\r\nContent-Length: 0\r\n\r\n").as_bytes());
                }
            });
        }
    });
    (format!("http://127.0.0.1:{port}/x"), asked)
}

fn t0() -> Timestamp {
    "2026-09-24T14:00:00Z".parse().unwrap()
}

#[test]
fn a_reply_carries_its_status_and_the_time_it_arrived_on_the_clock_handed_in() {
    let (url, _) = serve("HTTP/1.1 404 Not Found");
    let clock = Arc::new(ManualClock::at(t0()));
    let net = Net::new(clock, Arc::new(Limiter::new()));
    let r = net.send(&Ask::get(&url, &[])).unwrap();
    assert_eq!(r.status, 404, "a status is an answer, not a transport failure");
    assert_eq!(r.received_at, t0());
}

#[test]
fn a_refusal_rests_the_host_and_nothing_is_sent_until_its_retry_after_passes() {
    let (url, asked) = serve("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120");
    let clock = Arc::new(ManualClock::at(t0()));
    let net = Net::new(clock.clone(), Arc::new(Limiter::new()));
    assert_eq!(net.send(&Ask::get(&url, &[])).unwrap().status, 429);
    clock.advance(Duration::from_secs(119));
    let e = net.send(&Ask::get(&url, &[])).unwrap_err();
    assert_eq!(e, NetError::Resting { host: "127.0.0.1".into(), until: "2026-09-24T14:02:00Z".parse().unwrap() });
    assert_eq!(asked.load(Ordering::SeqCst), 1, "a resting host is not asked");
    clock.advance(Duration::from_secs(1));
    assert_eq!(net.send(&Ask::get(&url, &[])).unwrap().status, 429);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[test]
fn a_refusal_without_retry_after_rests_the_host_for_its_pace() {
    let (url, _) = serve("HTTP/1.1 429 Too Many Requests");
    let clock = Arc::new(ManualClock::at(t0()));
    let limiter = Arc::new(Limiter::new());
    limiter.configure("127.0.0.1", Pace { gap: Duration::ZERO, rest: Duration::from_secs(600) });
    let net = Net::new(clock.clone(), limiter);
    net.send(&Ask::get(&url, &[])).unwrap();
    clock.advance(Duration::from_secs(599));
    assert!(matches!(net.send(&Ask::get(&url, &[])), Err(NetError::Resting { .. })));
}

#[test]
fn a_hosts_gap_is_waited_on_the_clock_handed_in() {
    let (url, _) = serve("HTTP/1.1 200 OK");
    let clock = Arc::new(ManualClock::at(t0()));
    let limiter = Arc::new(Limiter::new());
    limiter.configure("127.0.0.1", Pace { gap: Duration::from_secs(30), rest: Duration::from_secs(60) });
    let net = Net::new(clock.clone(), limiter);
    net.send(&Ask::get(&url, &[])).unwrap();
    let second = net.send(&Ask::get(&url, &[])).unwrap();
    assert_eq!(second.received_at, "2026-09-24T14:00:30Z".parse().unwrap(), "the second ask waited the host's gap");
}

#[test]
fn a_503_rests_the_host_only_when_it_says_for_how_long() {
    let (url, asked) = serve("HTTP/1.1 503 Service Unavailable");
    let clock = Arc::new(ManualClock::at(t0()));
    let net = Net::new(clock, Arc::new(Limiter::new()));
    net.send(&Ask::get(&url, &[])).unwrap();
    net.send(&Ask::get(&url, &[])).unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 2, "a bare 503 is a failure, not a request to rest");
}

#[test]
fn retry_after_reads_seconds_and_http_dates() {
    let now = t0();
    assert_eq!(retry_after("120", now), Some(Duration::from_secs(120)));
    assert_eq!(retry_after("Thu, 24 Sep 2026 14:05:00 GMT", now), Some(Duration::from_secs(300)));
    assert_eq!(retry_after("Thu, 24 Sep 2026 13:00:00 GMT", now), Some(Duration::ZERO), "a date passed means now");
    assert_eq!(retry_after("soon", now), None);
    assert_eq!(retry_after("", now), None);
}

#[test]
fn a_host_is_named_without_its_scheme_port_user_or_path() {
    assert_eq!(host_of("https://Query1.Finance.Yahoo.com/v8/finance/chart/AAPL?x=1"), "query1.finance.yahoo.com");
    assert_eq!(host_of("http://127.0.0.1:8799/x"), "127.0.0.1");
    assert_eq!(host_of("https://user@www.bankofcanada.ca"), "www.bankofcanada.ca");
}
