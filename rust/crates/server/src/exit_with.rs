//! `BAGHOLDER_EXIT_WITH=<pid>`: a copy of the app started for some work (a
//! scratch server, a browser test's, a capacity probe) ends when the process
//! that started it ends, so no copy outlives the work (brief 64, 5). It is woken
//! by that process's exit itself, never by looking again on a clock: a pidfd on
//! Linux, a kqueue `NOTE_EXIT` on macOS, a process handle's wait on Windows.

/// The process to end with, as the environment names it: none when unset.
pub fn pid() -> Result<Option<u32>, String> {
    match crate::app::env_text(VAR)? {
        None => Ok(None),
        Some(v) if v.trim().is_empty() => Ok(None),
        Some(v) => v.trim().parse::<u32>().map(Some).map_err(|_| format!("{VAR}={v:?} is not a process id")),
    }
}

pub const VAR: &str = "BAGHOLDER_EXIT_WITH";

/// `then` run, once, on a thread of its own, when process `pid` exits. Refused
/// when that process is not running now: a copy is never started for work that
/// has already ended.
pub fn watch(pid: u32, then: impl FnOnce() + Send + 'static) -> Result<(), String> {
    let exited = wait_for(pid)?;
    std::thread::Builder::new()
        .name("exit-with".into())
        .spawn(move || {
            exited();
            then();
        })
        .map_err(|e| format!("{VAR}: the watch could not start: {e}"))?;
    Ok(())
}

fn gone(pid: u32) -> String {
    format!("{VAR}: process {pid} is not running, so the work this copy was started for has ended")
}

/// What blocks until `pid` exits, set up now so that a process already gone
/// refuses here.
#[cfg(target_os = "linux")]
fn wait_for(pid: u32) -> Result<Box<dyn FnOnce() + Send>, String> {
    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor or -1
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) } as libc::c_int;
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return Err(if e.raw_os_error() == Some(libc::ESRCH) { gone(pid) } else { format!("{VAR}: process {pid} could not be watched: {e}") });
    }
    Ok(Box::new(move || {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // readable once the process has exited; a signal that interrupts the wait waits again
        // SAFETY: one valid pollfd, no timeout
        while unsafe { libc::poll(&mut p, 1, -1) } < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {}
        // SAFETY: the descriptor pidfd_open gave, closed once
        unsafe { libc::close(fd) };
    }))
}

#[cfg(target_os = "macos")]
fn wait_for(pid: u32) -> Result<Box<dyn FnOnce() + Send>, String> {
    // SAFETY: kqueue takes nothing and returns a new descriptor or -1
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return Err(format!("{VAR}: process {pid} could not be watched: {}", std::io::Error::last_os_error()));
    }
    let ev = libc::kevent { ident: pid as libc::uintptr_t, filter: libc::EVFILT_PROC, flags: libc::EV_ADD | libc::EV_ONESHOT, fflags: libc::NOTE_EXIT, data: 0, udata: std::ptr::null_mut() };
    // SAFETY: one change, no events asked for, no timeout
    if unsafe { libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
        let e = std::io::Error::last_os_error();
        // SAFETY: the descriptor kqueue gave, closed once
        unsafe { libc::close(kq) };
        return Err(if e.raw_os_error() == Some(libc::ESRCH) { gone(pid) } else { format!("{VAR}: process {pid} could not be watched: {e}") });
    }
    Ok(Box::new(move || {
        // SAFETY: an all-zero kevent is a valid place for the kernel to write one event
        let mut out: libc::kevent = unsafe { std::mem::zeroed() };
        // SAFETY: no changes, one event asked for, no timeout
        while unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut out, 1, std::ptr::null()) } < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {}
        // SAFETY: the descriptor kqueue gave, closed once
        unsafe { libc::close(kq) };
    }))
}

#[cfg(windows)]
fn wait_for(pid: u32) -> Result<Box<dyn FnOnce() + Send>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_FAILED};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
    // SAFETY: a handle that may only be waited on, not inherited; null when the process is not there
    let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        return Err(gone(pid));
    }
    // a raw handle is not Send; the address is carried and is only used on the watch's thread
    let raw = h as usize;
    Ok(Box::new(move || {
        let h = raw as windows_sys::Win32::Foundation::HANDLE;
        // SAFETY: the handle OpenProcess gave, waited on and closed once
        unsafe {
            if WaitForSingleObject(h, INFINITE) == WAIT_FAILED {
                crate::app::log(&format!("{VAR}: the wait on process {pid} failed: {}", std::io::Error::last_os_error()));
            }
            CloseHandle(h);
        }
    }))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn wait_for(pid: u32) -> Result<Box<dyn FnOnce() + Send>, String> {
    Err(format!("{VAR}: watching process {pid} is not built for this system"))
}
