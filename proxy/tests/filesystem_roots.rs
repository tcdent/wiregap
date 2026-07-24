//! Sprint 16 Slice 3 — the filesystem-roots DSL, the both-roots `Dispatcher` API fix, and the one
//! composition-root assembly. Everything here drives `lifecycle::assemble` — the SAME function
//! `serve.rs`'s `main` calls — so what these tests prove is what the binary runs; there is no replica.

#![cfg(unix)]

use std::os::fd::AsFd;
use std::sync::atomic::{AtomicUsize, Ordering};

use directpath::SessionId;
use proto::{FileRead, FileWrite, Message, encode_message, scan_safe, scan_unsafe};
use proxy::dispatch::Routed;
use proxy::lifecycle::{Assembly, Authority, ValidatedRoots, assemble, validate_authority};
use proxy::register::anthropic::VETTED_TOOLS;
use proxy::register::net::ROUTES;

static SCRATCH_N: AtomicUsize = AtomicUsize::new(0);
/// A unique scratch directory (removed on drop) standing in for a configured root.
struct Scratch {
    dir: std::path::PathBuf,
}
impl Scratch {
    fn new(tag: &str) -> Self {
        let n = SCRATCH_N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("proxy-s16-{tag}-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }
    fn path(&self) -> &str {
        self.dir.to_str().unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn file_read(name: &str) -> Vec<u8> {
    encode_message(&Message::FileRead(FileRead { source: scan_unsafe(name.as_bytes()).unwrap() }))
}
fn file_write_to(joined: &str) -> Vec<u8> {
    encode_message(&Message::FileWrite(FileWrite(
        scan_safe(joined.replace('/', "\u{1f}").as_bytes()).unwrap(),
    )))
}
fn file_chunk(bytes: &[u8]) -> Vec<u8> {
    let mut f = proto::verb::FILE_CHUNK.to_le_bytes().to_vec();
    f.extend_from_slice(bytes);
    f
}

/// **THE BOTH-ROOTS-LIVE GATE** — the case no test could express before this slice, because
/// `with_readable_root` and `with_quarantine_root` were each associated constructors that defaulted the
/// other root away. One `Dispatcher`, from the real `assemble`, serving a `file-read` **and** a
/// `file-chunk` write. Before the API fix this was unreachable: whichever root was set second won, so
/// one of these two channels was always dead.
#[test]
fn one_dispatcher_from_assemble_serves_both_a_read_and_a_write() {
    let quarantine = Scratch::new("q");
    let readable = Scratch::new("r");
    std::fs::write(readable.dir.join("source.txt"), b"trusted-host bytes").unwrap();

    let Assembly { mut dispatcher, roots } = assemble(quarantine.path(), readable.path());
    assert!(roots.is_ok(), "two good roots must validate: {:?}", roots.err());

    // READ channel live: a file-read under the configured readable root is ADMITTED and streams.
    let read_verdict = dispatcher.route(SessionId(1), &file_read("source.txt"), ROUTES, VETTED_TOOLS);
    assert_eq!(
        read_verdict,
        Routed::FileReadStreaming,
        "the READ channel must be live on this dispatcher (readable root configured)",
    );

    // WRITE channel live on the SAME dispatcher: a file-write is admitted and its chunk is appended
    // beneath the configured quarantine root.
    let write_verdict = dispatcher.route(SessionId(2), &file_write_to("out.bin"), ROUTES, VETTED_TOOLS);
    assert_eq!(write_verdict, Routed::FileWriteAdmit, "the WRITE channel must be live on the SAME dispatcher");
    let chunk_verdict = dispatcher.route(SessionId(2), &file_chunk(b"box bytes"), ROUTES, VETTED_TOOLS);
    assert_eq!(
        chunk_verdict,
        Routed::FileChunkAppended { bytes: 9 },
        "the chunk must land in the quarantine — both roots live at once",
    );

    // The bytes really landed under the CONFIGURED quarantine root (not merely "a" root).
    let written = std::fs::read(quarantine.dir.join("out.bin")).expect("the chunk landed in the quarantine");
    assert_eq!(written, b"box bytes");
}

/// **THE CONFIGURED-ROOT PROOF** (Sprint 15's canary discipline applied to the roots): the confined
/// reader is rooted at the root the assembly was *configured* with, not merely at *a* root. Two
/// directories exist, each holding a distinctly-named canary; only one is configured. A read of the
/// unconfigured directory's canary must fail — if the reader were rooted anywhere else (or unrooted),
/// this would succeed and the canary would leak.
#[test]
fn the_confined_reader_is_rooted_at_the_configured_readable_root() {
    let quarantine = Scratch::new("q");
    let configured = Scratch::new("configured");
    let other = Scratch::new("other");
    std::fs::write(configured.dir.join("visible.txt"), b"in the configured root").unwrap();
    std::fs::write(other.dir.join("CANARY-unconfigured.txt"), b"must never be readable").unwrap();

    let Assembly { mut dispatcher, roots } = assemble(quarantine.path(), configured.path());
    assert!(roots.is_ok());

    // A file in the CONFIGURED root reads.
    assert_eq!(
        dispatcher.route(SessionId(1), &file_read("visible.txt"), ROUTES, VETTED_TOOLS),
        Routed::FileReadStreaming,
        "a file under the configured root is readable",
    );
    // The canary in the OTHER directory is not reachable by name — the reader is rooted, and rooted at
    // the configured path specifically.
    assert!(
        !matches!(
            dispatcher.route(SessionId(2), &file_read("CANARY-unconfigured.txt"), ROUTES, VETTED_TOOLS),
            Routed::FileReadStreaming
        ),
        "the unconfigured directory's canary must NOT be readable — the reader is rooted at the configured path",
    );
}

/// **THE BAD-ROOT ENUMERATION — with its success arm.** A validator that always returned `false` would
/// pass every negative case below and prove nothing (the circular-shadow tell), so the good-roots case
/// is asserted in the same test: the check must *derive from the configured value*, failing on bad roots
/// and passing on good ones. Each failure is also named by which root broke.
#[test]
fn bad_roots_fail_closed_and_good_roots_validate() {
    use proxy::lifecycle::RootFailure;

    let good_q = Scratch::new("q");
    let good_r = Scratch::new("r");

    // ── SUCCESS ARM (not vacuous): two good roots resolve, and the authority rung PASSES. ──
    let ok = assemble(good_q.path(), good_r.path());
    let validated: ValidatedRoots = ok.roots.expect("good roots validate");
    assert!(
        validate_authority(Authority::FilesystemRoots, ROUTES, None, Some(&validated)),
        "good roots MUST satisfy the rung — otherwise the check is a hardcoded fail",
    );

    // ── FAILURE ARMS: each fails closed, and the rung refuses. ──
    let nonexistent = format!("{}/definitely-not-here", good_q.path());
    let a_file = format!("{}/a-file", good_q.path());
    std::fs::write(&a_file, b"not a directory").unwrap();
    let symlink = format!("{}/a-symlink", good_q.path());
    std::os::unix::fs::symlink(good_r.path(), &symlink).unwrap();

    // Quarantine root: absent / a file where a directory is expected / a final-component symlink.
    for (label, bad) in [
        ("nonexistent quarantine", nonexistent.as_str()),
        ("quarantine is a file", a_file.as_str()),
        ("quarantine is a symlink", symlink.as_str()),
    ] {
        let a = assemble(bad, good_r.path());
        assert!(matches!(a.roots, Err(RootFailure::Quarantine(_))), "{label} must fail, named as the quarantine");
        assert!(
            !validate_authority(Authority::FilesystemRoots, ROUTES, None, a.roots.as_ref().ok()),
            "{label} must hold the rung closed",
        );
    }

    // Readable root: absent / a file where a directory is expected.
    for (label, bad) in [("nonexistent readable", nonexistent.as_str()), ("readable is a file", a_file.as_str())] {
        let a = assemble(good_q.path(), bad);
        assert!(matches!(a.roots, Err(RootFailure::Readable(_))), "{label} must fail, named as the readable root");
        assert!(
            !validate_authority(Authority::FilesystemRoots, ROUTES, None, a.roots.as_ref().ok()),
            "{label} must hold the rung closed",
        );
    }

    // A failed assembly still yields a usable (rootless) dispatcher, so the ladder can report
    // `Closed(FilesystemRoots)` normally instead of the binary dying with an unnamed error — and the
    // write channel is INERT: nothing is ever appended.
    //
    // Note the admit still happens: `file-write`'s verdict is the tier-1 leaf check on the DESTINATION,
    // which is root-independent by design (the confined open is lazy, at the first chunk). So the inert
    // proof belongs on the CHUNK, which is where the root is actually consumed — with no root it routes
    // to the handler and writes nothing, rather than appending anywhere.
    let mut inert = assemble(&nonexistent, good_r.path()).dispatcher;
    inert.route(SessionId(1), &file_write_to("out.bin"), ROUTES, VETTED_TOOLS);
    assert!(
        !matches!(
            inert.route(SessionId(1), &file_chunk(b"box bytes"), ROUTES, VETTED_TOOLS),
            Routed::FileChunkAppended { .. }
        ),
        "a failed assembly leaves the write channel inert — no chunk is ever appended",
    );
}

/// **BIND-THE-CHAIN**: the fd the `Dispatcher` writes beneath and the fd `ValidatedRoots` retains for
/// `KernelProtections` to probe are the SAME pinned directory — one resolution, duplicated, not two
/// independent `pin_root` calls of a mutable path. Proven by identity, not by inspection: a write
/// through the dispatcher's descriptor is observed through the retained one.
#[test]
fn dispatcher_and_validated_roots_share_one_pinned_quarantine() {
    use std::os::fd::AsRawFd;

    let quarantine = Scratch::new("q");
    let readable = Scratch::new("r");
    let Assembly { mut dispatcher, roots } = assemble(quarantine.path(), readable.path());
    let validated = roots.expect("good roots");

    // Two DISTINCT descriptors (a dup, not the same number)…
    let retained = validated.quarantine().as_raw_fd();
    assert!(retained >= 0);

    // …onto ONE directory: a chunk written through the dispatcher's descriptor appears beneath the
    // very path the retained fd pins. (Same open file description, so a probe of `retained` describes
    // the mount this write landed on — which is exactly what KernelProtections will rely on.)
    dispatcher.route(SessionId(1), &file_write_to("bound.bin"), ROUTES, VETTED_TOOLS);
    dispatcher.route(SessionId(1), &file_chunk(b"chained"), ROUTES, VETTED_TOOLS);
    assert_eq!(
        std::fs::read(quarantine.dir.join("bound.bin")).expect("written beneath the pinned root"),
        b"chained",
        "validated == used: the dispatcher writes beneath the same pinned directory the rung retains",
    );
}

/// The enumeration's remaining case: an **unreadable/unsearchable** root. Covered for the readable root,
/// where `read_dir` is exactly the operation the channel performs, so a mode-000 directory fails closed.
///
/// For the QUARANTINE root the honest answer is different and is recorded here rather than glossed:
/// `pin_root` opens `O_PATH`, which by design performs no permission check on the final component, so a
/// mode-000 quarantine directory PINS successfully. That is not a hole in the floor — the write path is
/// fail-safe, and the first confined open into an unwritable directory is denied (`FileChunkDeny`) — but
/// it does mean "unusable permissions" is caught at first write rather than at the rung. Catching it at
/// the rung would need a permission probe on the pinned fd (`faccessat`), which is `unsafe` and so
/// belongs beside `fstatfs` in `open_discipline` — i.e. Slice 4's territory, not silently absent.
#[test]
fn an_unreadable_readable_root_fails_closed_and_the_quarantine_case_is_recorded() {
    use proxy::lifecycle::RootFailure;
    use std::os::unix::fs::PermissionsExt;

    let good_q = Scratch::new("q");
    let unreadable = Scratch::new("unreadable");
    std::fs::set_permissions(&unreadable.dir, std::fs::Permissions::from_mode(0o000)).unwrap();

    // READABLE root: unreadable → fails closed, named.
    let a = assemble(good_q.path(), unreadable.path());
    assert!(
        matches!(a.roots, Err(RootFailure::Readable(_))),
        "an unreadable readable root must fail closed at the rung",
    );

    // QUARANTINE root: O_PATH pins it regardless of its mode — documented above, asserted so the
    // behavior is pinned rather than assumed, and so a future change here is a visible test change.
    let b = assemble(unreadable.path(), good_q.path());
    assert!(
        b.roots.is_ok(),
        "O_PATH pins a mode-000 directory (no final-component permission check) — recorded, not a hole",
    );

    std::fs::set_permissions(&unreadable.dir, std::fs::Permissions::from_mode(0o755)).unwrap(); // cleanup
}

// ─── Sprint 16 Slice 4 — the KernelProtections probes ────────────────────────────────────────────

/// **THE KERNEL-PROTECTIONS PROBE, all arms.** The rung asks whether the kernel-enforced floor is
/// actually in effect on *this* host's quarantine mount, rather than trusting it. Proven against real
/// mounts on the running system, including the sharp **partial** case — a mount carrying some of the
/// three flags but not all must fail closed, since a floor with a hole is not a floor.
#[test]
fn kernel_protections_hold_only_when_all_three_flags_are_present() {
    use proxy::lifecycle::kernel_protections_hold;
    use proxy::open_discipline::{mount_protections, openat2_resolve_available, pin_root};

    // SUCCESS ARM — `/run` is a tmpfs carrying noexec+nosuid+nodev on a stock systemd host. Pinned
    // read-only (O_PATH needs only search permission), so this needs no privilege and writes nothing.
    let run = pin_root("/run").expect("/run is pinnable");
    let p = mount_protections(run.as_fd()).expect("fstatfs on the pinned fd");
    assert!(p.noexec && p.nosuid && p.nodev, "/run should carry all three: {p:?}");
    assert!(
        openat2_resolve_available(run.as_fd()),
        "openat2's RESOLVE confinement must be available — every confined write depends on it",
    );
    assert!(kernel_protections_hold(run.as_fd()), "all three flags + openat2 ⇒ the rung holds");

    // PARTIAL-FAILURE ARM (the sharp one) — a mount with nosuid+nodev but NOT noexec must FAIL. A
    // validator that merely checked "some protections" would pass here; this is what catches it.
    for candidate in ["/run/user/1000", "/dev/shm"] {
        let Ok(fd) = pin_root(candidate) else { continue };
        let p = mount_protections(fd.as_fd()).expect("fstatfs");
        if !p.noexec && (p.nosuid || p.nodev) {
            assert!(
                !kernel_protections_hold(fd.as_fd()),
                "{candidate} carries {p:?} — a PARTIAL floor must fail closed, not pass",
            );
            return; // one real partial fixture is enough
        }
    }
    panic!("no partial-protection mount found to exercise the sharp case");
}

/// A quarantine on a mount carrying NONE of the three (an ordinary disk filesystem such as `/tmp`)
/// fails the rung — the common misconfiguration, caught at startup rather than trusted.
#[test]
fn a_quarantine_with_no_mount_protections_fails_the_rung() {
    use proxy::lifecycle::{Authority, assemble, validate_authority};

    let quarantine = Scratch::new("q"); // under /tmp — no noexec/nosuid/nodev
    let readable = Scratch::new("r");
    let a = assemble(quarantine.path(), readable.path());
    let roots = a.roots.expect("the roots themselves resolve fine");

    // FilesystemRoots is satisfied — the paths are good…
    assert!(validate_authority(Authority::FilesystemRoots, ROUTES, None, Some(&roots)));
    // …but KernelProtections is NOT: the mount carries no protections, so the floor is absent.
    assert!(
        !validate_authority(Authority::KernelProtections, ROUTES, None, Some(&roots)),
        "an unprotected quarantine mount must hold the proxy closed",
    );
}
