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

/// The **Anthropic service** register: the vetted client-side tool identities U4 admits by exact
/// string (JA-1, pinned to the 2026-06-20 snapshot; fail-closed on drift — a new date stamp is a
/// different string and is not on this list until the operator adds it and recompiles).
pub mod anthropic {
    use crate::vocab::ToolIdentity;

    /// The operator's vetted client-side tool identities (a sample set). Structural validity is
    /// compile-checked; *which* identities are trusted is expressed by this list (JA-1).
    pub const VETTED_TOOLS: &[ToolIdentity] = &[
        ToolIdentity::new("text_editor_20250728"),
        ToolIdentity::new("bash_20250124"),
        ToolIdentity::new("memory_20250818"),
    ];
}
