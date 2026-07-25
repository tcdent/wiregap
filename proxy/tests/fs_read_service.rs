//! Sprint 6 S2 — filesystem read service certification (MB1-MB6, streaming). MB2 correctness (reads
//! the named file's bytes intact, streamed), MB3 fails-SAFE + no-write-floor-creep, MB4/MB5 bounded
//! memory + no-block (streams READ_CHUNK at a time; device/FIFO/dir fail safe), MB6 pure safe Rust.
#![cfg(target_os = "linux")]

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

use proto::{ErrorCode, scan_unsafe};
use proxy::fs_read::{FileReader, READ_CHUNK, ReadStep};

struct Scratch(PathBuf);
static COUNTER: AtomicU32 = AtomicU32::new(0);
impl Scratch {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("proxy-s2-{}-{}", process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn root(&self) -> &Path { &self.0 }
}
impl Drop for Scratch { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

fn src(segments: &[&str]) -> proto::UnsafeSegmentStack {
    scan_unsafe(segments.join("\u{1f}").as_bytes()).unwrap()
}

/// Drain a FileReader to a single Vec (for assertions), asserting each chunk is <= READ_CHUNK.
fn drain(mut r: FileReader) -> Result<Vec<u8>, ErrorCode> {
    let mut out = Vec::new();
    loop {
        match r.next_chunk() {
            ReadStep::Item(chunk) => {
                assert!(chunk.len() <= READ_CHUNK, "each streamed chunk is bounded by READ_CHUNK");
                out.extend_from_slice(&chunk);
            }
            ReadStep::Eof => return Ok(out),
            ReadStep::Error(e) => return Err(e.code),
        }
    }
}

/// MB2: a real file under the readable root streams back its exact bytes.
#[test]
fn reads_a_named_file_intact() {
    let sc = Scratch::new();
    std::fs::create_dir_all(sc.root().join("docs")).unwrap();
    std::fs::write(sc.root().join("docs/readme.txt"), b"hello \x00 world \xff").unwrap();
    let r = FileReader::open(sc.root(), &src(&["docs", "readme.txt"])).expect("opens");
    assert_eq!(drain(r).unwrap(), b"hello \x00 world \xff");
}

/// MB4/MB5: a large file (> READ_CHUNK) streams in BOUNDED chunks — total bytes correct, no single
/// chunk exceeds READ_CHUNK, so memory never blows up regardless of file size.
#[test]
fn large_file_streams_in_bounded_chunks() {
    let sc = Scratch::new();
    let big: Vec<u8> = (0..(READ_CHUNK * 3 + 123)).map(|i| (i & 0xff) as u8).collect();
    std::fs::write(sc.root().join("big.bin"), &big).unwrap();
    let mut r = FileReader::open(sc.root(), &src(&["big.bin"])).expect("opens");
    let mut total = 0usize;
    let mut chunks = 0;
    loop {
        match r.next_chunk() {
            ReadStep::Item(c) => { assert!(c.len() <= READ_CHUNK); total += c.len(); chunks += 1; }
            ReadStep::Eof => break,
            ReadStep::Error(e) => panic!("unexpected error {:?}", e.code),
        }
    }
    assert_eq!(total, big.len(), "streamed the whole file");
    assert!(chunks >= 4, "a 3x+ file must take multiple bounded chunks, got {chunks}");
}

/// MB3 fails-safe: a missing file yields a fail-safe NotFound — never a panic, never a breach.
#[test]
fn missing_file_fails_safe() {
    let sc = Scratch::new();
    assert_eq!(FileReader::open(sc.root(), &src(&["nope.txt"])).unwrap_err().code, ErrorCode::NotFound);
}

/// MB3 fails-safe: the error carries NO box-supplied path (file.md §0).
#[test]
fn error_carries_no_box_name() {
    let sc = Scratch::new();
    let e = FileReader::open(sc.root(), &src(&["secret_name_the_box_chose.txt"])).unwrap_err();
    assert!(e.detail.is_empty(), "no box name may enter the error body");
}

/// MB3 fail-safe on a FINAL-component symlink: O_NOFOLLOW refuses it (the read bar — NOT the write
/// mask). A fail-safe error, not a breach.
#[test]
fn final_component_symlink_fails_safe_via_o_nofollow() {
    let sc = Scratch::new();
    std::fs::write(sc.root().join("real.txt"), b"real").unwrap();
    std::os::unix::fs::symlink("real.txt", sc.root().join("link.txt")).unwrap();
    assert!(FileReader::open(sc.root(), &src(&["link.txt"])).is_err());
}

/// MB4/MB5: a DIRECTORY (non-regular) fails safe at open via the regular-file check — before any read.
#[test]
fn directory_source_fails_safe() {
    let sc = Scratch::new();
    std::fs::create_dir_all(sc.root().join("adir")).unwrap();
    assert!(FileReader::open(sc.root(), &src(&["adir"])).is_err(), "a directory is not a regular file");
}

/// MB4/MB5 (the never-block/never-EOF class): a FIFO fails safe — O_NONBLOCK prevents the open from
/// blocking, and the regular-file check rejects it before any read. (The reviewer's /dev/zero-class
/// concern, tested with a FIFO — the same is_file gate blocks a char device too.)
#[test]
fn fifo_source_fails_safe_without_blocking() {
    let sc = Scratch::new();
    let fifo = sc.root().join("pipe");
    let c = CString::new(fifo.to_str().unwrap()).unwrap();
    // mkfifo is the only libc call needed to build the fixture; the service under test uses safe std.
    let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
    assert_eq!(rc, 0, "mkfifo fixture");
    // Must return promptly (no block) with a fail-safe error, not hang or stream forever.
    assert!(FileReader::open(sc.root(), &src(&["pipe"])).is_err(), "a FIFO must fail safe, not block");
}

/// MB5: oversized / many-segment source stacks never panic — always data-or-error.
#[test]
fn oversized_and_odd_sources_never_panic() {
    let sc = Scratch::new();
    let long_name = "x".repeat(4096);
    let many: Vec<&str> = vec!["a"; 200];
    for segs in [vec![long_name.as_str()], many, vec!["name with spaces"], vec!["a.b.c"]] {
        let _ = FileReader::open(sc.root(), &src(&segs)); // must not panic; fails safe if invalid
    }
}
