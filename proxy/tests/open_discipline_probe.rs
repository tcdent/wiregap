//! U1 open-discipline **live probe** (Sprint 5, CB1 + the deploy-kernel probe discipline). The
//! Rust successor to Sprint 2's `openat2_probe.c` — it exercises the **real**
//! `open_discipline` module against environmental fixtures on the live bench kernel and asserts
//! each denial is *observed*, never assumed from the flag. The oracle is independent: each
//! fixture is planted to trigger a specific errno derived from the syscall contract, and the
//! assertion is against the module's real output.
//!
//! Bench: this runs on whatever kernel `cargo test` runs on; the deploy-host re-verification
//! duty (spec R-E) carries forward. Skipped cleanly on non-Linux (openat2 is Linux ≥5.6).
#![cfg(target_os = "linux")]

use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

use proto::scan_safe;
use proxy::open_discipline::{OpenDenial, confined_create_leaf, pin_root};

/// A hand-rolled unique scratch dir (no tempfile dependency — the security core stays lean).
/// Cleaned on drop.
struct Scratch(PathBuf);

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Scratch {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("proxy-u1-probe-{}-{}", process::id(), n));
        std::fs::create_dir_all(&dir).expect("create scratch root");
        Scratch(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn root_str(&self) -> String {
        self.0.to_str().expect("utf-8 scratch path").to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a `SafeSegmentStack` from `0x1F`-joined segments via the real proto scan.
fn stack(segments: &[&str]) -> proto::SafeSegmentStack {
    let joined = segments.join("\u{1f}");
    scan_safe(joined.as_bytes()).expect("test segments are valid safe segments")
}

/// P-FRESH: a fresh valid leaf under the pinned root is created, at mode 0600.
#[test]
fn fresh_leaf_creates_at_0600() {
    let sc = Scratch::new();
    let root = pin_root(&sc.root_str()).expect("pin root");
    let fd = confined_create_leaf(root.as_fd(), &stack(&["a", "b", "file.txt"]))
        .expect("fresh valid path admits");
    drop(fd);
    let meta = std::fs::metadata(sc.path().join("a/b/file.txt")).expect("leaf exists");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o600, "leaf mode must be 0600");
    }
}

/// P-SYMLINK-INTERMEDIATE (the load-bearing row — the O_NOFOLLOW-gap closer): a symlink planted
/// as an *intermediate* component denies with ELOOP, observed at runtime.
#[test]
fn symlink_intermediate_component_denies_eloop() {
    let sc = Scratch::new();
    // Plant: root/evil -> /etc (an absolute-target symlink as an intermediate dir).
    std::os::unix::fs::symlink("/etc", sc.path().join("evil")).expect("plant symlink");
    let root = pin_root(&sc.root_str()).expect("pin root");
    let got = confined_create_leaf(root.as_fd(), &stack(&["evil", "passwd_clone"]));
    assert_eq!(
        got.err(),
        Some(OpenDenial::Denied { errno: libc::ELOOP }),
        "an intermediate symlink must deny ELOOP (RESOLVE_NO_SYMLINKS) — the gap O_NOFOLLOW missed",
    );
}

/// P-SYMLINK-FINAL: a symlink at the leaf name denies (RESOLVE_NO_SYMLINKS at the final
/// component; O_EXCL would also refuse the existing name).
#[test]
fn symlink_at_leaf_denies() {
    let sc = Scratch::new();
    std::os::unix::fs::symlink("/etc/passwd", sc.path().join("link")).expect("plant leaf symlink");
    let root = pin_root(&sc.root_str()).expect("pin root");
    let got = confined_create_leaf(root.as_fd(), &stack(&["link"]));
    // ELOOP (symlink resolution refused) or EEXIST (name exists) — both are correct fail-closed
    // denials; assert it is one of them, not an admit.
    match got {
        Err(OpenDenial::Denied { errno }) => {
            assert!(
                errno == libc::ELOOP || errno == libc::EEXIST,
                "leaf symlink must deny ELOOP or EEXIST, got errno {errno}",
            );
        }
        other => panic!("leaf symlink must deny, got {other:?}"),
    }
}

/// P-COLLISION: an existing regular file at the leaf name denies EEXIST (O_CREAT|O_EXCL,
/// no-overwrite).
#[test]
fn existing_leaf_denies_eexist() {
    let sc = Scratch::new();
    std::fs::write(sc.path().join("taken"), b"prior").expect("plant existing file");
    let root = pin_root(&sc.root_str()).expect("pin root");
    let got = confined_create_leaf(root.as_fd(), &stack(&["taken"]));
    assert_eq!(
        got.err(),
        Some(OpenDenial::Denied { errno: libc::EEXIST }),
        "an existing leaf must deny EEXIST (O_EXCL no-overwrite)",
    );
}

/// P-FILE-AS-DIR: a regular file where an intermediate directory is expected denies ENOTDIR.
#[test]
fn file_as_intermediate_denies_enotdir() {
    let sc = Scratch::new();
    std::fs::write(sc.path().join("notadir"), b"x").expect("plant file");
    let root = pin_root(&sc.root_str()).expect("pin root");
    let got = confined_create_leaf(root.as_fd(), &stack(&["notadir", "leaf"]));
    assert_eq!(
        got.err(),
        Some(OpenDenial::Denied { errno: libc::ENOTDIR }),
        "a file as an intermediate component must deny ENOTDIR (O_DIRECTORY)",
    );
}

/// P-IDEMPOTENT-DIR: re-using an existing *intermediate directory* across two transfers is fine
/// (mkdirat tolerates EEXIST) — only the leaf is no-overwrite, not the path dirs.
#[test]
fn existing_intermediate_dir_is_reused() {
    let sc = Scratch::new();
    let root = pin_root(&sc.root_str()).expect("pin root");
    confined_create_leaf(root.as_fd(), &stack(&["shared", "first"])).expect("first leaf");
    // Same intermediate dir, different leaf — must succeed (dir reuse, not a collision).
    confined_create_leaf(root.as_fd(), &stack(&["shared", "second"])).expect("second leaf reuses dir");
    assert!(sc.path().join("shared/first").exists());
    assert!(sc.path().join("shared/second").exists());
}
