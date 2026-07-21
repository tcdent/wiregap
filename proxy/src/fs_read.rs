//! Sprint 6 S2 — the filesystem read service (tier 2, `filesystem.md` §3; `architecture.md` tier-2
//! "FS read service"). The box names a **source** — an [`UnsafeSegmentStack`] (real names, from
//! `proto::scan_unsafe`, which already forbids `/`, `0x1F`, NUL, `.`, `..`) — under the operator's
//! readable root; this service **opens one file and STREAMS its bytes** back as `file-data`
//! (`filesystem.md` §3's own language), in bounded [`READ_CHUNK`] pieces.
//!
//! **This is addressing, not a gate** (`filesystem.md` §3): reading is harmless (the box may acquire
//! any information — `principles.md` §7), so a bug **fails safe** — the box reads some other
//! readable file, or nothing. The open discipline is the *read path's* own: `O_NOFOLLOW`
//! **final-component-only** (`file.md` §2; `box-proxy.md`'s file-read row) — **correct** here
//! because a followed intermediate symlink only reads a different file. Over-applying the *write*
//! floors (`RESOLVE_NO_SYMLINKS`, the full mask) would be tier-1 security logic on a fail-safe
//! channel, growing the trusted surface for zero gain (MB3) — refused.
//!
//! **Bounded memory + no-block (MB4/MB5):** the reader streams `READ_CHUNK` bytes at a time, so a
//! large legitimate file never blows memory; and a **regular-file check** on the opened fd rejects a
//! device / FIFO / socket / directory (so `read` can never run forever on `/dev/zero`-class sources
//! reached via an intermediate symlink), with `O_NONBLOCK` at open so a FIFO can't block the open
//! itself. `O_NOFOLLOW` goes through **safe `std`** (`OpenOptionsExt::custom_flags`) — this service
//! adds **no** `unsafe` (MB6); the sole confined-unsafe module remains `open_discipline`.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use proto::{ErrorBody, ErrorCode, UnsafeSegmentStack};

/// The bounded read/stream chunk: at most this many bytes are held in memory per step, and each
/// becomes one `file-data` frame. Provisional (a buffering knob, not a security bound).
pub const READ_CHUNK: usize = 64 * 1024;

/// One step of the stream: up to [`READ_CHUNK`] bytes, end-of-file, or a fail-safe error.
#[derive(Debug)]
pub enum ReadStep {
    /// Up to `READ_CHUNK` bytes of file content — the caller frames this as one `file-data`.
    Data(Vec<u8>),
    /// The file is fully read — the stream is done.
    Eof,
    /// A fail-safe error mid-stream (i/o). Carries the proxy's own category, **no** box name.
    Error(ErrorBody),
}

/// A streaming reader over one box-named source. Holds the open fd and yields bounded chunks —
/// memory stays at `READ_CHUNK` regardless of file size.
#[derive(Debug)]
pub struct FileReader {
    file: File,
}

impl FileReader {
    /// Open a box-named source under the readable root, **fail-safe**. `O_NOFOLLOW` (final
    /// component) + `O_NONBLOCK` (so a FIFO can't block the open) through safe `std`, then a
    /// **regular-file check** on the opened fd (TOCTOU-free): a device / FIFO / socket / directory
    /// fails safe *here*, before any read, so streaming can never block or run forever on a
    /// non-regular source. Returns the reader, or a fail-safe [`ErrorBody`] (no box name in it).
    pub fn open(readable_root: &Path, source: &UnsafeSegmentStack) -> Result<Self, ErrorBody> {
        let path = resolve(readable_root, source);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|e| error_for(&e))?;

        // Regular-file check on the opened fd: reject device/FIFO/socket/dir (the never-EOF and
        // block classes). `is_file()` is true only for a regular file.
        match file.metadata() {
            Ok(m) if m.is_file() => {}
            Ok(_) => {
                return Err(ErrorBody {
                    code: ErrorCode::Unavailable,
                    detail: Vec::new(),
                });
            }
            Err(e) => return Err(error_for(&e)),
        }
        Ok(FileReader { file })
    }

    /// The next stream step: up to [`READ_CHUNK`] bytes, [`ReadStep::Eof`], or a fail-safe error.
    /// Memory is bounded to `READ_CHUNK` per call. Never panics.
    pub fn next_chunk(&mut self) -> ReadStep {
        let mut buf = alloc_chunk();
        match self.file.read(&mut buf) {
            Ok(0) => ReadStep::Eof,
            Ok(n) => {
                buf.truncate(n);
                ReadStep::Data(buf)
            }
            Err(e) => ReadStep::Error(error_for(&e)),
        }
    }
}

fn alloc_chunk() -> Vec<u8> {
    vec![0u8; READ_CHUNK]
}

/// Resolve a box-named source under the readable root: the proxy supplies the root and every
/// separator; the box supplies only leaf names (`scan_unsafe` guaranteed no `/`, `0x1F`, NUL, `.`,
/// `..`). So the join cannot traverse out by *saying* `..` — unsayable traversal, here for addressing.
fn resolve(readable_root: &Path, source: &UnsafeSegmentStack) -> PathBuf {
    let mut path = readable_root.to_path_buf();
    for seg in source.segments() {
        use std::os::unix::ffi::OsStrExt;
        path.push(std::ffi::OsStr::from_bytes(seg.as_bytes()));
    }
    path
}

/// Map an `io::Error` to a proxy-owned category. **No box-supplied path** enters the body — only a
/// category code and an empty detail (unsafe names are never logged, `file.md` §0).
fn error_for(e: &std::io::Error) -> ErrorBody {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::Denied,
        _ => ErrorCode::Unavailable,
    };
    ErrorBody {
        code,
        detail: Vec::new(),
    }
}
