#![allow(unsafe_code)]

//! The **one** module that touches `unsafe` — thin, **sound** safe wrappers over the syscalls the
//! reactor needs, plus RAII fd types. `libc` supplies the declarations; every `unsafe` block has a
//! SAFETY comment discharging the syscall's preconditions (valid fd, correctly-sized buffer). Nothing
//! above this module is `unsafe` (the crate root is `#![deny(unsafe_code)]`).
//!
//! Syscalls wrapped: `epoll_create1` / `epoll_ctl` / `epoll_wait`, `eventfd`, `read`, `write`,
//! `clock_gettime(CLOCK_MONOTONIC)`. (`write` is the symmetric primitive used by [`Eventfd::notify`] —
//! the **E2-simulation / test** hook; the reactor's production loop only *reads* the eventfd. Flagged to
//! the lead as one beyond the contract's named list, required by the wake-on-write test.)

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use libc::{c_int, c_uint, c_void};

// ============================================================================
// Raw syscall wrappers — the only `unsafe` in the crate. Each returns a checked io::Result.
// ============================================================================

/// `epoll_create1(flags)` → an owned epoll fd.
fn epoll_create1(flags: c_int) -> io::Result<OwnedFd> {
    // SAFETY: `epoll_create1` takes only a flags int (no pointers, no preconditions beyond a valid flag
    // set — we pass EPOLL_CLOEXEC) and returns a fresh fd or -1.
    let fd = unsafe { libc::epoll_create1(flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd >= 0` is a fresh, exclusively-owned epoll fd from a successful `epoll_create1`; taking
    // ownership here is correct (no other owner exists).
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `eventfd(initval, flags)` → an owned eventfd.
fn eventfd(initval: c_uint, flags: c_int) -> io::Result<OwnedFd> {
    // SAFETY: `eventfd` takes two ints (no pointers) and returns a fresh fd or -1.
    let fd = unsafe { libc::eventfd(initval, flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful `eventfd` returns a fresh, exclusively-owned fd.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `epoll_ctl(epfd, op, fd, event)`. `event` is `Some` for ADD/MOD, `None` (NULL) for DEL.
fn epoll_ctl(
    epfd: BorrowedFd<'_>,
    op: c_int,
    fd: BorrowedFd<'_>,
    event: Option<&mut libc::epoll_event>,
) -> io::Result<()> {
    let ev_ptr = match event {
        Some(e) => e as *mut libc::epoll_event,
        None => core::ptr::null_mut(),
    };
    // SAFETY: `epfd` and `fd` are valid for the call (BorrowedFd guarantees the underlying fds stay open
    // for the borrow). `ev_ptr` is either a valid, correctly-laid-out `&mut epoll_event` (from libc) or
    // NULL — and NULL is permitted by the kernel for EPOLL_CTL_DEL. No other preconditions.
    let r = unsafe { libc::epoll_ctl(epfd.as_raw_fd(), op, fd.as_raw_fd(), ev_ptr) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `epoll_wait(epfd, events, maxevents=events.len(), timeout_ms)` → number of ready events.
fn epoll_wait(
    epfd: BorrowedFd<'_>,
    events: &mut [libc::epoll_event],
    timeout_ms: c_int,
) -> io::Result<usize> {
    // Clamp maxevents to the buffer length (and to c_int range) so the kernel cannot write past `events`.
    let maxevents = events.len().min(c_int::MAX as usize) as c_int;
    // SAFETY: `epfd` is a valid borrowed fd. `events.as_mut_ptr()` addresses `events.len()`
    // correctly-sized `epoll_event` slots; we pass `maxevents <= events.len()`, so the kernel writes at
    // most `maxevents` entries — never past the buffer. Returns the count or -1.
    let n = unsafe { libc::epoll_wait(epfd.as_raw_fd(), events.as_mut_ptr(), maxevents, timeout_ms) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// Read the opaque `u64` token (the `data.u64`) from a kernel-filled `epoll_event`. `epoll_event` is
/// `#[repr(packed)]` on some arches, so we read the field unaligned (no reference to a packed field) —
/// the only correct way, confined here. Lets the deny-`unsafe` loop dispatch readiness safely.
pub(crate) fn event_token(ev: &libc::epoll_event) -> u64 {
    // SAFETY: `addr_of!` forms a raw pointer without creating a (possibly-misaligned) reference to the
    // packed field; `read_unaligned` is valid for any alignment. `ev` is a live, kernel-initialized
    // `epoll_event`, so its `u64` field is initialized.
    unsafe { core::ptr::addr_of!(ev.u64).read_unaligned() }
}

/// `read(fd, buf, buf.len())` → bytes read.
pub(crate) fn read(fd: BorrowedFd<'_>, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: `fd` is a valid borrowed fd. `buf.as_mut_ptr()` / `buf.len()` describe a writable region of
    // exactly `buf.len()` bytes; `read` writes at most `buf.len()` bytes there. Returns the count or -1.
    let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut c_void, buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// `write(fd, buf, buf.len())` → bytes written.
fn write(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: `fd` is a valid borrowed fd. `buf.as_ptr()` / `buf.len()` describe a readable region of
    // exactly `buf.len()` bytes; `write` reads at most `buf.len()` bytes. Returns the count or -1.
    let n = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr() as *const c_void, buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// `clock_gettime(CLOCK_MONOTONIC)` → monotonic nanoseconds (matching `transport::Instant`'s ns ticks).
pub(crate) fn clock_gettime_monotonic() -> io::Result<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `&mut ts` is a valid, correctly-sized, writable `timespec` the kernel fills;
    // `CLOCK_MONOTONIC` is a valid clock id. Returns 0 or -1.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    // CLOCK_MONOTONIC yields non-negative components; combine to ns (saturating — unreachable in the
    // ~584-year u64 ns range, kept total rather than panicking).
    Ok((ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as u64))
}

// ============================================================================
// RAII fd types — safe surface over the wrappers above.
// ============================================================================

/// An owned `epoll` instance (closed on drop via `OwnedFd`).
#[derive(Debug)]
pub struct Epoll {
    fd: OwnedFd,
}

impl Epoll {
    /// Create an `epoll` instance with `EPOLL_CLOEXEC`.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            fd: epoll_create1(libc::EPOLL_CLOEXEC)?,
        })
    }

    /// Register `fd` with interest `events` (e.g. `EPOLLIN`) and an opaque `token` (the `u64` data).
    pub fn add(&self, fd: BorrowedFd<'_>, events: u32, token: u64) -> io::Result<()> {
        let mut ev = libc::epoll_event {
            events,
            u64: token,
        };
        epoll_ctl(self.fd.as_fd(), libc::EPOLL_CTL_ADD, fd, Some(&mut ev))
    }

    /// Deregister `fd`.
    pub fn delete(&self, fd: BorrowedFd<'_>) -> io::Result<()> {
        epoll_ctl(self.fd.as_fd(), libc::EPOLL_CTL_DEL, fd, None)
    }

    /// Modify `fd`'s interest/token — used to **re-arm** an `EPOLLONESHOT` app fd after back-pressure.
    pub fn modify(&self, fd: BorrowedFd<'_>, events: u32, token: u64) -> io::Result<()> {
        let mut ev = libc::epoll_event {
            events,
            u64: token,
        };
        epoll_ctl(self.fd.as_fd(), libc::EPOLL_CTL_MOD, fd, Some(&mut ev))
    }

    /// Wait up to `timeout_ms` (`-1` = block indefinitely) for readiness; fills `events`, returns count.
    pub fn wait(&self, events: &mut [libc::epoll_event], timeout_ms: i32) -> io::Result<usize> {
        epoll_wait(self.fd.as_fd(), events, timeout_ms)
    }
}

/// An owned `eventfd` — the reactor's idle-wakeup latch (E2 arms the real one via MSI-X→eventfd; here a
/// real eventfd stands in). Non-blocking + CLOEXEC.
#[derive(Debug)]
pub struct Eventfd {
    fd: OwnedFd,
}

impl Eventfd {
    /// Create a non-blocking, CLOEXEC eventfd initialized to 0.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            fd: eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC)?,
        })
    }

    /// Borrow the underlying fd (to register on an [`Epoll`]).
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Duplicate this eventfd handle (a second fd to the *same* eventfd object) via the safe std
    /// `OwnedFd::try_clone` — no `unsafe`. Used to hand a **notifier** to the E2-side / a test thread
    /// while the reactor owns the read side; eventfd writes are atomic.
    pub fn try_clone(&self) -> io::Result<Eventfd> {
        Ok(Eventfd {
            fd: self.fd.try_clone()?,
        })
    }

    /// Read-and-clear the latch. Returns the accumulated count (`>= 1` if it was signaled), or `0` if it
    /// was not signaled (`EAGAIN` on the non-blocking fd) — so the caller can re-check "truly cleared".
    pub fn read_clear(&self) -> io::Result<u64> {
        let mut buf = [0u8; 8];
        match read(self.fd.as_fd(), &mut buf) {
            Ok(8) => Ok(u64::from_ne_bytes(buf)),
            Ok(_) => Ok(0),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Signal the eventfd by `count`. **In production E2 / the kernel does this** (MSI-X→eventfd); this
    /// method is the **E2-simulation / test** hook. The reactor loop never writes the eventfd.
    pub fn notify(&self, count: u64) -> io::Result<()> {
        let buf = count.to_ne_bytes();
        match write(self.fd.as_fd(), &buf)? {
            8 => Ok(()),
            _ => Err(io::Error::other("short eventfd write")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slice-1 method: a real eventfd in a real epoll — wake-on-write and latch-clear-on-read, with the
    /// latch positively proven cleared (a subsequent wait times out, no busy-spin on a stale latch).
    #[test]
    fn eventfd_wakes_epoll_and_latch_clears_on_read() {
        let ep = Epoll::new().unwrap();
        let efd = Eventfd::new().unwrap();
        ep.add(efd.as_fd(), libc::EPOLLIN as u32, 1).unwrap();

        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 4];

        // Not signaled: a 0-ms wait returns empty.
        assert_eq!(ep.wait(&mut events, 0).unwrap(), 0, "no signal → times out empty");

        // E2-sim: signal it; epoll reports it readable.
        efd.notify(1).unwrap();
        assert_eq!(ep.wait(&mut events, 0).unwrap(), 1, "eventfd write wakes epoll");

        // Latch clear: read drains the count; a subsequent wait times out (latch truly cleared).
        assert_eq!(efd.read_clear().unwrap(), 1, "read returns the accumulated count");
        assert_eq!(ep.wait(&mut events, 0).unwrap(), 0, "latch cleared → no stale readiness");
        // And re-reading the cleared (non-blocking) eventfd reports no signal.
        assert_eq!(efd.read_clear().unwrap(), 0, "cleared latch reads as 0 (EAGAIN)");
    }

    #[test]
    fn monotonic_clock_is_nondecreasing() {
        let a = clock_gettime_monotonic().unwrap();
        let b = clock_gettime_monotonic().unwrap();
        assert!(b >= a, "CLOCK_MONOTONIC never goes backwards");
    }
}
