//! Port of tests/test_localmodel.py: endpoint resolution, checksum
//! verification, chat reply parsing and graceful failure. Nothing is
//! downloaded, spawned or requested: `localmodel::hooks` stand in.

use bagholder_market::localmodel::{self, hooks};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;

/// The model's state is process-wide, and so are the env pins.
static LOCK: Mutex<()> = Mutex::new(());

fn guard() -> std::sync::MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    hooks::clear();
    localmodel::reset_state();
    // nothing in these tests may reach a real server
    hooks::DETECT.with(|h| *h.borrow_mut() = Some(Box::new(|| None)));
    hooks::ENSURE.with(|h| *h.borrow_mut() = Some(Box::new(|| {})));
    g
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bh-localmodel-test-{}-{}", std::process::id(), name));
    let _ = std::fs::create_dir_all(&d);
    d
}

// --- EndpointTest

#[test]
fn test_a_running_endpoint_is_used_and_nothing_is_provisioned() {
    let _g = guard();
    hooks::DETECT.with(|h| *h.borrow_mut() = Some(Box::new(|| Some(("http://127.0.0.1:11434".into(), "llama3.2".into())))));
    let kicked = Rc::new(Cell::new(0));
    let k = kicked.clone();
    hooks::ENSURE.with(|h| *h.borrow_mut() = Some(Box::new(move || k.set(k.get() + 1))));
    assert_eq!(localmodel::endpoint(), "http://127.0.0.1:11434");
    assert_eq!(localmodel::status(), "ready");
    assert_eq!(kicked.get(), 0, "a detected endpoint must not trigger a download");
    localmodel::reset_state();
}

#[test]
fn test_no_endpoint_kicks_provisioning_and_returns_empty() {
    let _g = guard();
    let kicked = Rc::new(Cell::new(0));
    let k = kicked.clone();
    hooks::ENSURE.with(|h| *h.borrow_mut() = Some(Box::new(move || k.set(k.get() + 1))));
    assert_eq!(localmodel::endpoint(), "");
    assert_eq!(kicked.get(), 1, "with nothing running, provisioning is kicked off");
}

#[test]
fn test_status_defaults_to_off() {
    let _g = guard();
    assert_eq!(localmodel::status(), "off");
}

// --- VerifyTest

#[test]
fn test_a_file_is_verified_against_the_pinned_sha256() {
    let _g = guard();
    let p = scratch("verify").join("m.llamafile");
    std::fs::write(&p, b"hello world").unwrap();
    let saved = std::env::var("BAGHOLDER_LLAMAFILE_SHA256").ok();
    // sha256("hello world")
    std::env::set_var("BAGHOLDER_LLAMAFILE_SHA256", "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    assert!(localmodel::verified(&p));
    std::env::set_var("BAGHOLDER_LLAMAFILE_SHA256", "0".repeat(64));
    assert!(!localmodel::verified(&p), "a wrong checksum is refused");
    match saved {
        Some(v) => std::env::set_var("BAGHOLDER_LLAMAFILE_SHA256", v),
        None => std::env::remove_var("BAGHOLDER_LLAMAFILE_SHA256"),
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn test_a_missing_file_is_not_verified() {
    let _g = guard();
    assert!(!localmodel::verified(&PathBuf::from("/no/such/file")));
}

#[test]
fn test_download_refuses_a_host_off_the_allowlist() {
    let _g = guard();
    let saved = std::env::var("BAGHOLDER_LLAMAFILE_URL").ok();
    std::env::set_var("BAGHOLDER_LLAMAFILE_URL", "https://evil.example.com/x.llamafile");
    let ok = localmodel::download(&scratch("download").join("m"));
    match saved {
        Some(v) => std::env::set_var("BAGHOLDER_LLAMAFILE_URL", v),
        None => std::env::remove_var("BAGHOLDER_LLAMAFILE_URL"),
    }
    assert!(!ok);
}

// --- ChatTest

#[test]
fn test_chat_is_empty_when_no_endpoint() {
    let _g = guard();
    hooks::ENDPOINT.with(|h| *h.borrow_mut() = Some(Box::new(String::new)));
    hooks::POST.with(|h| *h.borrow_mut() = Some(Box::new(|_, _| panic!("no request without an endpoint"))));
    assert_eq!(localmodel::chat("hi", 90), "");
}

#[test]
fn test_chat_parses_an_openai_shaped_reply() {
    let _g = guard();
    hooks::ENDPOINT.with(|h| *h.borrow_mut() = Some(Box::new(|| "http://127.0.0.1:8121".into())));
    let seen = Rc::new(RefCell::new(String::new()));
    let s = seen.clone();
    hooks::POST.with(|h| *h.borrow_mut() = Some(Box::new(move |url, _| {
        *s.borrow_mut() = url.to_string();
        Ok(r#"{"choices": [{"message": {"content": "A concise summary."}}]}"#.into())
    })));
    assert_eq!(localmodel::chat("summarize this", 90), "A concise summary.");
    assert_eq!(*seen.borrow(), "http://127.0.0.1:8121/v1/chat/completions");
}

#[test]
fn test_chat_swallows_a_backend_error() {
    let _g = guard();
    hooks::ENDPOINT.with(|h| *h.borrow_mut() = Some(Box::new(|| "http://127.0.0.1:8121".into())));
    hooks::POST.with(|h| *h.borrow_mut() = Some(Box::new(|_, _| Err("connection refused".into()))));
    assert_eq!(localmodel::chat("x", 90), "");
}

// --- WaitReadyTest

fn wait(seconds: f64, available: Vec<bool>, status: &'static str) -> bool {
    hooks::ENDPOINT.with(|h| *h.borrow_mut() = Some(Box::new(String::new)));
    hooks::NO_SLEEP.with(|h| *h.borrow_mut() = true);
    let tries = RefCell::new(available.into_iter());
    let last = Cell::new(false);
    hooks::AVAILABLE.with(|h| *h.borrow_mut() = Some(Box::new(move || {
        // a constant answer is given as one element, repeated
        if let Some(v) = tries.borrow_mut().next() { last.set(v); }
        last.get()
    })));
    hooks::STATUS.with(|h| *h.borrow_mut() = Some(Box::new(move || status)));
    localmodel::wait_ready(seconds)
}

#[test]
fn test_it_waits_for_a_model_that_is_starting() {
    let _g = guard();
    assert!(wait(30.0, vec![false, false, true, true], "starting"));
}

#[test]
fn test_it_waits_while_one_is_being_detected() {
    let _g = guard();
    assert!(wait(30.0, vec![false, true, true], "detecting"));
}

#[test]
fn test_it_does_not_wait_for_a_download() {
    let _g = guard();
    assert!(!wait(30.0, vec![false], "downloading"));
}

#[test]
fn test_it_does_not_wait_when_there_is_nothing_coming() {
    let _g = guard();
    for phase in ["off", "failed"] {
        assert!(!wait(30.0, vec![false], phase), "{}", phase);
    }
}

#[test]
fn test_a_model_already_up_is_not_waited_for_at_all() {
    let _g = guard();
    assert!(wait(30.0, vec![true], "ready"));
}

#[test]
fn test_no_time_to_wait_means_no_wait() {
    let _g = guard();
    assert!(!wait(0.0, vec![false], "starting"));
}
