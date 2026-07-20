//! U1's second half — the open-discipline confined-open primitives
//! (`spec/filesystem-write-path.md` §1): pinned `O_PATH` root, per-component `mkdirat` +
//! `openat2` under the `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV`
//! mask, leaf creation `O_CREAT|O_EXCL`. Real syscalls — **not pure**.
//!
//! This is the **one** module in the crate that carries `unsafe` (crate root is
//! `deny(unsafe_code)`; RQ7 → Option A, design 2026-07-19: box-reachable code carries confined
//! in-house `unsafe`, never a third-party wrapper). Certified by a **live probe on the deploy
//! kernel** (Sprint 2's standing discipline), never by unit test alone — a symlink at any
//! component must be *observed* to deny at runtime, not assumed from the flag.
//!
//! The seam: [`crate::leaf::leaf_validate`] produces the `SafeSegmentStack` (confining what the
//! box can *say*); this module confines where those bytes can *land* (symlink-escape, mount
//! crossing) — a distinct floor. The adversary here is **environmental** (a symlink or file
//! planted in the quarantine tree), never a box-said traversal (unsayable upstream).
#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use proto::SafeSegmentStack;

/// The confinement mask applied to **every** `openat2` in this module
/// (`spec/filesystem-write-path.md` §1.1). Each flag is the kernel-enforced form of a stated
/// requirement: `BENEATH` (stay under the root, reject absolute), `NO_SYMLINKS` (deny a symlink
/// at *any* component — the closer for the `O_NOFOLLOW`-final-component-only gap),
/// `NO_MAGICLINKS` (belt; implied by `NO_SYMLINKS`), `NO_XDEV` (no mount crossing).
const RESOLVE_MASK: u64 = libc::RESOLVE_BENEATH
    | libc::RESOLVE_NO_SYMLINKS
    | libc::RESOLVE_NO_MAGICLINKS
    | libc::RESOLVE_NO_XDEV;

/// Why a confined open was denied.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenDenial {
    /// A path component or the leaf failed under the confinement mask — carries the errno of the
    /// failing step (`ELOOP` a symlink, `EXDEV` an escape, `ENOTDIR` a non-dir component,
    /// `EEXIST` a leaf collision). Every one is a legible, fail-closed deny.
    Denied { errno: i32 },
    /// An empty element array — defensive; the leaf-validator guarantees ≥1 element.
    Empty,
}

/// Pin the quarantine root once, fail-closed on failure (`spec/filesystem-write-path.md` §1.0
/// step 1). `O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`. The returned fd is a valid `dirfd` for the
/// `*at` calls; its intermediate-component symlink safety is operator-config trust (spec R-H —
/// `O_NOFOLLOW` covers the final component only; the box cannot influence the root path).
pub fn pin_root(path: &str) -> std::io::Result<OwnedFd> {
    let c = CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `c` is a valid NUL-terminated C string, alive across the call; the flags are
    // constants; `open` returns an fd or -1 (errno set), both handled below.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh, owned, valid descriptor returned by `open`; wrapping transfers
    // ownership so it is closed exactly once on drop.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// One `openat2` under [`RESOLVE_MASK`]. Returns the owned fd or the failing errno.
fn openat2(dirfd: i32, name: &CStr, flags: u64, mode: u64) -> Result<OwnedFd, i32> {
    // `libc::open_how` is `#[non_exhaustive]`, so it is built zeroed (a plain repr(C) struct of
    // integers — all-zero is a valid, well-defined value) and its fields set.
    // SAFETY: `open_how` is a C struct of integer fields with no invalid bit patterns; zeroing is
    // a valid initialization.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = flags;
    how.mode = mode;
    how.resolve = RESOLVE_MASK;
    // SAFETY: `name` is a valid NUL-terminated C string alive across the call; `&how` points at a
    // fully-initialized `open_how` of the passed size; `dirfd` is a caller-owned descriptor. The
    // syscall returns a new fd or -1 (errno set), both handled below.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            dirfd,
            name.as_ptr(),
            std::ptr::addr_of!(how),
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if ret < 0 {
        Err(std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(0))
    } else {
        // SAFETY: `ret >= 0` is a fresh owned fd from a successful `openat2`; ownership transfers.
        Ok(unsafe { OwnedFd::from_raw_fd(ret as i32) })
    }
}

/// Confined-create the leaf: descend each intermediate under the mask (creating it if absent,
/// `O_EXCL`-less mkdir tolerating `EEXIST`, then re-opening under the mask so a symlink or
/// non-dir there is caught), then create the leaf `O_WRONLY|O_CREAT|O_EXCL` at mode `0600`
/// (`spec/filesystem-write-path.md` §1.2). Returns the leaf's owned write fd, or a legible deny.
///
/// The mask is enforced *during* kernel resolution, so there is no validate-then-open TOCTOU
/// window (spec §3 row 5).
pub fn confined_create_leaf(
    root: BorrowedFd<'_>,
    stack: &SafeSegmentStack,
) -> Result<OwnedFd, OpenDenial> {
    let segs = stack.segments();
    let Some((leaf, intermediates)) = segs.split_last() else {
        return Err(OpenDenial::Empty);
    };

    // Descend intermediates, holding every opened dir fd alive until we return (depth is bounded
    // by the leaf-validator's MAX_ELEMENTS, so this Vec is bounded).
    let mut held: Vec<OwnedFd> = Vec::with_capacity(intermediates.len());
    let mut cur_fd = root.as_raw_fd();
    for seg in intermediates {
        // Safe-segment bytes contain no interior NUL (alphabet is `[A-Za-z0-9._-]`), so this
        // cannot fail; map defensively rather than unwrap.
        let name = CString::new(seg.as_bytes()).map_err(|_| OpenDenial::Denied { errno: libc::EINVAL })?;
        // mkdirat, tolerate an already-present directory; any other errno denies.
        // SAFETY: `name` is a valid C string alive across the call; `cur_fd` is owned (root or a
        // held fd). Return handled below.
        let r = unsafe { libc::mkdirat(cur_fd, name.as_ptr(), 0o700) };
        if r < 0 {
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if e != libc::EEXIST {
                return Err(OpenDenial::Denied { errno: e });
            }
        }
        // Re-open under the mask: a symlink here → ELOOP, a non-dir → ENOTDIR, an escape → EXDEV.
        let opened = openat2(
            cur_fd,
            &name,
            (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
            0,
        )
        .map_err(|errno| OpenDenial::Denied { errno })?;
        cur_fd = opened.as_raw_fd();
        held.push(opened);
    }

    let name = CString::new(leaf.as_bytes()).map_err(|_| OpenDenial::Denied { errno: libc::EINVAL })?;
    openat2(
        cur_fd,
        &name,
        (libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC) as u64,
        0o600,
    )
    .map_err(|errno| OpenDenial::Denied { errno })
}
