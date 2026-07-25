//! U5 — the compiled-in Rust register (operator direction, `approach.md` §6: "The register is
//! Rust, not a config file"). A **per-service** set of typed, compile-time-validated routes and
//! tool identities — plain declarative `const` arrays a reviewer reads top-to-bottom (C5: boring,
//! not clever; no proc-macro, no builder). Its guarantees:
//!
//! - **C1 — invalid is a compile error.** Each entry is built by [`crate::vocab::Route::new`] /
//!   [`crate::vocab::ToolIdentity::new`], whose `assert!` fails const-evaluation (a compile error)
//!   on a malformed entry. There is no unchecked constructor left to bypass it (the Slice-2
//!   placeholders were *replaced*, not shadowed).
//! - **C2 — per-service.** Each service ([`net`], [`anthropic`]) authors its own register.
//! - **C3 — the box cannot author it.** This is TCB source compiled in, never runtime input; the
//!   box sends bytes, and bytes cannot add a `const`.
//! - **C4 — a change is edit + recompile + re-review** (the promotion pattern at the source level;
//!   this is also what re-triggers the `principles.md` §15 reviewability sitting — ledger L3).
//!
//! This is a **sample reviewed register**; the operator edits these lists and recompiles.

/// The **net/address service** register (`spec/address-recognizer.md`): the destinations an
/// emitted `net-address` may match. U3 byte-exact matches an assembled address against [`ROUTES`].
pub mod net {
    use crate::vocab::Route;

    /// The operator's curated route destinations (a sample set). Each is a canonical `assemble`
    /// form; a malformed one would not compile.
    pub const ROUTES: &[Route] = &[
        // The core, essential entry — the Anthropic Messages API.
        Route::new("https://api.anthropic.com/v1/messages"),
        // Sample inert GET destinations (a package index, a docs site).
        Route::new("https://pypi.org/simple"),
        Route::new("https://docs.rs"),
    ];
}

/// The **filesystem service** register (`architecture.md` §8, `filesystem.md` §4–§5): the two roots that
/// are "explicitly config and explicitly the whole of the filesystem subsystem's state".
///
/// Both are compiled-in and const-validated for *shape* (non-empty, absolute). Whether they exist, are
/// directories, and — for the quarantine — sit on a mount carrying the required protections is checked
/// at **startup**, by the `FilesystemRoots` and `KernelProtections` authority rungs, which fail the whole
/// proxy closed rather than serving from an unverified root.
pub mod filesystem {
    use crate::vocab::RootPath;

    /// The **quarantine root** — the box's write target, the one place box-authored bytes land on
    /// trusted ground. Defaults under `/run`, which on a stock systemd host is a `tmpfs` already mounted
    /// `noexec,nosuid,nodev` (the protections `KernelProtections` requires — verified on the bench host,
    /// where `/tmp` and `$HOME` carry none of the three), and where this binary already keeps its head
    /// socket, so the runtime directory is the proxy's natural home.
    ///
    /// **Operator residuals, deliberately not decided by the team.** Two, both operational:
    /// 1. `/run` is `tmpfs`, so quarantine contents do not survive a reboot. Whether that volatility is
    ///    the desired custody model — versus a dedicated persistent mount carrying the same three flags
    ///    — is an operational call.
    /// 2. **Provisioning must make this directory WRITABLE BY THE DROP-TARGET UID**, not merely make it
    ///    exist. No startup rung can check this (the pin happens as root, and root bypasses the
    ///    permission check), so a root-owned `0755` directory validates every authority and then fails
    ///    at the first `file-chunk`. `systemd`'s `RuntimeDirectory=` alone is insufficient — it takes
    ///    ownership from `User=`, which must be `root` for the privileged NIC bring-up — so pair it with
    ///    an explicit `chown` to the drop-target uid.
    pub const QUARANTINE_ROOT: RootPath = RootPath::new("/run/zerotrust-proxy/quarantine");

    /// The **readable root** — the scope the box may read *from* the trusted host. Its breadth is
    /// "sanity, not safety" (`filesystem.md` §4–§5): reading is harmless, the validator confines either
    /// way. **A sample value, not a provisional one** (reviewed 2026-07-25). This module is a
    /// *sample reviewed register* (see the header) — the operator edits these entries and recompiles,
    /// so this path is an example of the shape, not a number awaiting a decision. Unlike the quarantine
    /// root it carries no mount-flag requirement: the operator sets it to whatever they intend to
    /// expose, and the validator confines either way. Nothing here is pending review.
    pub const READABLE_ROOT: RootPath = RootPath::new("/srv/zerotrust-proxy/readable");
}

/// The **Anthropic service** register: the vetted client-side tool identities U4 admits by exact
/// string (JA-1, pinned to the 2026-06-20 snapshot; fail-closed on drift — a new date stamp is a
/// different string and is not on this list until the operator adds it and recompiles).
pub mod anthropic {
    use crate::credential::CredentialSource;
    use crate::vocab::ToolIdentity;

    /// The operator's vetted client-side tool identities (a sample set). Structural validity is
    /// compile-checked; *which* identities are trusted is expressed by this list (JA-1).
    pub const VETTED_TOOLS: &[ToolIdentity] = &[
        ToolIdentity::new("text_editor_20250728"),
        ToolIdentity::new("bash_20250124"),
        ToolIdentity::new("memory_20250818"),
    ];

    /// **Where** the upstream credential is read from — the environment variable's *name*, never the
    /// secret (`architecture.md` §8: custody is the configured thing, not the value). Compiled in and
    /// const-validated like every other entry (C1: a malformed name is a compile error; C3: the box
    /// cannot author it), and structurally incapable of carrying a value — so the operator changes
    /// *where* the secret comes from by editing this line and recompiling (C4), while the secret itself
    /// never enters source control.
    pub const CREDENTIAL_SOURCE: CredentialSource = CredentialSource::env("ANTHROPIC_API_KEY");
}
