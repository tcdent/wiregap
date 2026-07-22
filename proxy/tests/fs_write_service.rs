//! Sprint 9 Slice 2 — the quarantine-write handler, standalone gate: the confined-open REUSE (an
//! adversarial destination corpus all denied THROUGH the full write-floor mask, never a re-implemented
//! validator) and the monotonic cursor (bytes land verbatim, in order, no offset), on real disk under
//! a temp quarantine root (the same scratch-dir pattern as `fs_read`'s tests).

use std::os::fd::{AsFd, OwnedFd};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

use proto::{SafeSegmentStack, scan_safe};
use proxy::fs_write::{QuarantineWriter, WriteStep};
use proxy::open_discipline::pin_root;

static N: AtomicU32 = AtomicU32::new(0);

struct Scratch {
    dir: std::path::PathBuf,
}
impl Scratch {
    fn new() -> Self {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("proxy-s9w-{}-{}", process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }
    fn root_fd(&self) -> OwnedFd {
        pin_root(self.dir.to_str().unwrap()).unwrap()
    }
    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.dir.join(rel)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn dest(joined: &str) -> SafeSegmentStack {
    scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap()
}

/// Happy path: chunks append verbatim, in order, at the monotonic cursor (no offset, no seek) — the
/// bytes land at the confined destination on disk (intermediate dirs created), and `written()` is the
/// running byte count.
#[test]
fn appends_verbatim_at_the_monotonic_cursor() {
    let sc = Scratch::new();
    let root = sc.root_fd();
    let mut w = QuarantineWriter::declare(dest("sub/file.txt"));
    assert_eq!(w.write_chunk(root.as_fd(), b"hello "), WriteStep::Appended(6));
    assert_eq!(w.write_chunk(root.as_fd(), b"world"), WriteStep::Appended(5));
    assert_eq!(w.written(), 11, "the cursor is the monotonic running byte count");
    drop(w); // close the fd (flush + release)
    assert_eq!(std::fs::read(sc.path("sub/file.txt")).unwrap(), b"hello world");
}

/// The `..` traversal / root-escape vector is defended ONE LAYER UP, at the proto codec: `scan_safe`
/// rejects a `..` (or `.`) segment as `TraversalSegment`, so a decoded `SafeSegmentStack` structurally
/// cannot carry one — it never reaches a `QuarantineWriter`. (RESOLVE_BENEATH/NO_XDEV in the reused
/// mask are the belt-and-suspenders below that, proven in `open_discipline`'s own suite.)
#[test]
fn traversal_destinations_are_unrepresentable_rejected_at_the_proto_codec() {
    assert!(scan_safe(b"..").is_err(), "`..` is not a representable safe segment");
    assert!(scan_safe("..\u{1f}escape".as_bytes()).is_err(), "a `..` component is rejected at decode");
}

/// The adversarial destination corpus that IS representable (scan_safe-accepted names) — every one
/// denies THROUGH the reused confined-create primitive (the full write-floor mask + `O_CREAT|O_EXCL`),
/// never a re-implemented validator. A dropped `RESOLVE_NO_SYMLINKS`/`O_EXCL` would open these holes.
#[test]
fn adversarial_destinations_all_deny_through_the_reused_primitive() {
    let sc = Scratch::new();
    let root = sc.root_fd();

    // (1) a symlink component escaping the root → RESOLVE_NO_SYMLINKS deny; nothing follows the link.
    std::os::unix::fs::symlink("/etc", sc.path("link")).unwrap();
    let mut w = QuarantineWriter::declare(dest("link/passwd"));
    assert!(matches!(w.write_chunk(root.as_fd(), b"x"), WriteStep::Denied(_)), "a symlink component must deny");

    // (2) an already-existing file → O_EXCL deny (no overwrite); the existing bytes are untouched.
    std::fs::write(sc.path("exists"), b"old").unwrap();
    let mut w = QuarantineWriter::declare(dest("exists"));
    assert!(matches!(w.write_chunk(root.as_fd(), b"new"), WriteStep::Denied(_)), "an existing file (O_EXCL) must deny");
    assert_eq!(std::fs::read(sc.path("exists")).unwrap(), b"old", "the existing file is untouched (no overwrite)");
}
