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

use proto::{ErrorBody, ErrorCode, FileType, ListEntry, UnsafeSegmentStack, scan_unsafe};

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

/// A streaming reader over one box-named **directory** (Sprint 10 Slice 2, `file.md` §3) — the read-path
/// analog of [`FileReader`] for `list`. Yields one [`ListEntry`] at a time from a **lazy**
/// `std::fs::read_dir` iterator (never slurps a huge directory), the entry name typed as an
/// `UnsafeSegment` (display-only, reused `scan_unsafe`, never re-derived).
///
/// **Fail-safe, not a gate** (`file.md` §3, exactly like [`FileReader`]): listing is harmless (the box
/// may enumerate any readable directory — `principles.md` §7), so a bug **fails safe**. The confinement
/// is the [`resolve`] **unsayable-traversal** join (readable root + `scan_unsafe` segments — no `/`,
/// `0x1F`, NUL, `.`, `..` sayable), **identical** to `FileReader`'s. This uses path-based `read_dir`
/// (safe `std`), so a followed final-component symlink lists a **different** directory's names — the
/// **accepted-harmless** fail-safe case (`fs_read`'s own posture: reading/listing is free, and
/// over-applying the write floors to a fail-safe channel is refused, MB3), **not** a hole. Adds **no**
/// `unsafe` (MB6): `read_dir` is safe `std`.
pub struct DirReader {
    entries: std::fs::ReadDir,
}

/// One step of the directory stream: the next entry, end-of-directory, or a fail-safe error.
#[derive(Debug)]
pub enum DirStep {
    /// The next directory entry — the caller frames this as one `list-entry` record.
    Entry(ListEntry),
    /// The directory is fully listed — the stream is done.
    Eof,
    /// A fail-safe error (non-directory / missing / i-o). Carries the proxy's own category, **no** box
    /// name (`file.md` §0).
    Error(ErrorBody),
}

impl DirReader {
    /// Open a box-named directory under the readable root, **fail-safe**. `read_dir` opens *and* confirms
    /// a directory: a regular file, missing path, or non-dir fails safe here (`NotADirectory`/`NotFound`),
    /// before any entry is pulled. Its iterator is **lazy** — entries are streamed one at a time by
    /// [`next_entry`](Self::next_entry), never slurped. Returns the reader or a fail-safe [`ErrorBody`]
    /// (no box name in it).
    pub fn open(readable_root: &Path, source: &UnsafeSegmentStack) -> Result<Self, ErrorBody> {
        let path = resolve(readable_root, source);
        match std::fs::read_dir(&path) {
            Ok(entries) => Ok(DirReader { entries }),
            Err(e) => Err(error_for(&e)),
        }
    }

    /// The next directory entry as a typed [`ListEntry`], [`DirStep::Eof`] at end, or a fail-safe error.
    /// **Bounded to one entry per call** (F2's lazy entry-source — never slurps). An entry whose name is
    /// not representable as an `UnsafeSegment` (a forbidden byte) is **skipped fail-safe** (display-only;
    /// an un-nameable entry is simply not listed, never logged). A symlink entry lists **as** a symlink
    /// (`metadata` does not follow it). Never panics.
    pub fn next_entry(&mut self) -> DirStep {
        loop {
            let entry = match self.entries.next() {
                None => return DirStep::Eof,
                Some(Ok(e)) => e,
                Some(Err(e)) => return DirStep::Error(error_for(&e)),
            };
            // The name, typed as UnsafeSegment (reused scan_unsafe, single-source alphabet; G4
            // display-only). A name with a forbidden byte (`/`/`0x1F`/NUL/`.`/`..`) is skipped fail-safe
            // — never rejected loudly, never logged (`.`/`..` are not yielded by read_dir anyway).
            use std::os::unix::ffi::OsStrExt;
            let file_name = entry.file_name();
            let name = match scan_unsafe(file_name.as_bytes()) {
                Ok(stack) => match stack.segments() {
                    [seg] => seg.clone(),
                    _ => continue, // a single name cannot be multi-segment — skip (unreachable in practice)
                },
                Err(_) => continue, // forbidden byte in the name — skip fail-safe
            };
            // Metadata WITHOUT following a symlink entry (so a symlink lists AS a symlink, not its
            // target). A per-entry metadata error stops the stream fail-safe.
            let md = match entry.metadata() {
                Ok(m) => m,
                Err(e) => return DirStep::Error(error_for(&e)),
            };
            let ft = md.file_type();
            let file_type = if ft.is_dir() {
                FileType::Dir
            } else if ft.is_symlink() {
                FileType::Symlink
            } else {
                // Regular file and any other object (fifo/socket/device) → File: a benign, display-only
                // default (FileType::Other is proto-private; the type is informational, not a gate).
                FileType::File
            };
            let (mtime_secs, mtime_nanos) = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos()));
            return DirStep::Entry(ListEntry {
                size: md.len(),
                mtime_secs,
                mtime_nanos,
                file_type,
                name,
            });
        }
    }
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
