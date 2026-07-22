//! Sprint 9 Slice 2 — the quarantine-write handler. The box uploads a file to quarantine ground:
//! `file-write` declares the destination (a **safe** segment stack), `file-chunk` frames append its
//! bytes. **One file per session** (`file.md` §1); completion is the transport session ending — no
//! wire close signal, no offset (the box cannot seek or overwrite).
//!
//! **Confinement reuses the certified Sprint-5 primitive**
//! [`crate::open_discipline::confined_create_leaf`] — the FULL write-floor mask
//! (`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV`, `O_CREAT|O_EXCL`),
//! enforced *during* kernel resolution (no validate-then-open TOCTOU). Never a weaker or
//! re-implemented open. The destination is `leaf_validate_prescanned`-validated at `file-write` time
//! (in `dispatch`), reused not re-validated here.
//!
//! **No new `unsafe`** (MB6 stays `{open_discipline, privilege}`): the append writes verbatim bytes
//! through safe [`std::fs::File::write_all`]; the confined open's fd becomes an owned `File` that
//! closes on drop (session-end eviction).

use std::fs::File;
use std::io::Write;
use std::os::fd::{BorrowedFd, OwnedFd};

use proto::SafeSegmentStack;

use crate::open_discipline::{OpenDenial, confined_create_leaf};

/// The per-session/per-file quarantine write cap (bytes) — the disk-fill floor. **Reject-before-grow**:
/// a chunk that would push the file past the cap is denied BEFORE any open/write, so the file never
/// grows past it (the same shape as `proto`'s `REASSEMBLY_CAP` for reads). Provisional scalar (32 MiB),
/// marked for review — a cheap per-file DoS floor; a real per-mount disk quota is the config layer's
/// (Sprint-8 L12 residual), not this handler's.
pub const QUARANTINE_WRITE_CAP: u64 = 32 * 1024 * 1024;

/// The outcome of appending one `file-chunk`.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteStep {
    /// `n` bytes appended at the monotonic cursor.
    Appended(usize),
    /// The confined open denied the destination (traversal / symlink / already-exists / …): the box
    /// named a destination the write-floor mask refuses. Fail-closed; nothing written.
    Denied(OpenDenial),
    /// The chunk would push the file past [`QUARANTINE_WRITE_CAP`] — denied before any write.
    Capped,
    /// The `write` itself failed (errno) — e.g. the filesystem is full. Fail-closed.
    WriteFailed(i32),
}

/// One session's quarantine write: a validated destination, a **lazily**-opened write file (opened on
/// the first chunk, so a `file-write`-then-abandon leaves no empty file), and a monotonic cursor. The
/// held `File` closes on drop — the session-end eviction hook does exactly that.
pub struct QuarantineWriter {
    dest: SafeSegmentStack,
    file: Option<File>,
    written: u64,
    /// The per-file cap. Always [`QUARANTINE_WRITE_CAP`] in production (set by [`declare`]); a
    /// `#[cfg(test)]` constructor injects a small cap to exercise reject-before-grow cheaply.
    cap: u64,
}

impl QuarantineWriter {
    /// Declare a write for a `file-write`-validated destination. No fd is opened yet (lazy).
    #[must_use]
    pub fn declare(dest: SafeSegmentStack) -> Self {
        Self { dest, file: None, written: 0, cap: QUARANTINE_WRITE_CAP }
    }

    /// **Test-only**: a writer with an explicit small `cap`, so the reject-before-grow behavior is
    /// reachable without a 32 MiB write. Invisible to the production build.
    #[cfg(test)]
    fn declare_with_cap(dest: SafeSegmentStack, cap: u64) -> Self {
        Self { dest, file: None, written: 0, cap }
    }

    /// Append one `file-chunk` under `root` (the pinned quarantine root). Opens the confined
    /// destination on the **first** chunk via the reused [`confined_create_leaf`] (full mask,
    /// `O_CREAT|O_EXCL`), then appends verbatim at the monotonic cursor. Cap-checked reject-before-grow.
    pub fn write_chunk(&mut self, root: BorrowedFd<'_>, bytes: &[u8]) -> WriteStep {
        // Reject-before-grow: never let the file exceed the cap (checked before any open or write; a
        // usize→u64 overflow on an absurd length also fails closed via checked_add).
        match self.written.checked_add(bytes.len() as u64) {
            Some(total) if total <= self.cap => {}
            _ => return WriteStep::Capped,
        }
        // Lazy open on the first chunk — confined by the reused primitive (full write-floor mask).
        if self.file.is_none() {
            match confined_create_leaf(root, &self.dest) {
                Ok(fd) => self.file = Some(open_owned(fd)),
                Err(denial) => return WriteStep::Denied(denial),
            }
        }
        // Append verbatim (safe std; NO seek — the box supplies no offset, so the cursor is monotonic).
        let file = self.file.as_mut().expect("opened just above");
        match file.write_all(bytes) {
            Ok(()) => {
                self.written += bytes.len() as u64;
                WriteStep::Appended(bytes.len())
            }
            Err(e) => WriteStep::WriteFailed(e.raw_os_error().unwrap_or(-1)),
        }
    }

    /// Bytes written so far (the monotonic cursor) — observability / test.
    #[must_use]
    pub fn written(&self) -> u64 {
        self.written
    }
}

/// Wrap the confined-open's owned fd as a `File` (safe; `File: From<OwnedFd>`), so appends go through
/// safe `std` and the fd closes on drop.
fn open_owned(fd: OwnedFd) -> File {
    File::from(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::scan_safe;
    use std::os::fd::AsFd;
    use std::process;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    fn temp_root() -> (std::path::PathBuf, OwnedFd) {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("proxy-s9w-unit-{}-{}", process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let fd = crate::open_discipline::pin_root(dir.to_str().unwrap()).unwrap();
        (dir, fd)
    }

    /// The per-file cap: reject-before-grow at a small injected cap — a chunk that would push past the
    /// cap is `Capped` **before** any write, and the on-disk file never grows past the cap.
    #[test]
    fn cap_rejects_before_grow() {
        let (dir, root) = temp_root();
        let mut w = QuarantineWriter::declare_with_cap(scan_safe(b"f").unwrap(), 8);
        assert_eq!(w.write_chunk(root.as_fd(), &[0u8; 6]), WriteStep::Appended(6)); // 6 <= 8
        assert_eq!(w.write_chunk(root.as_fd(), &[0u8; 6]), WriteStep::Capped); // 6+6=12 > 8 → no write
        assert_eq!(w.written(), 6, "the capped chunk did not grow the file");
        drop(w);
        assert_eq!(std::fs::metadata(dir.join("f")).unwrap().len(), 6, "on-disk size never exceeded the cap");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
