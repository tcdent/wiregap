//! U1 — the filesystem write-path **leaf-validator** (`spec/leaf-validator.md`).
//!
//! A total function `&[u8] -> Result<SafeSegmentStack, LeafDenial>`: the box's destination is a
//! `0x1F`-delimited safe segment stack (`proto::scan_safe` — the per-leaf/structural floor,
//! *inherited*, not rebuilt) plus **the one genuinely-new arm**: the array-level element-count
//! upper bound (`spec/leaf-validator.md` §3 — the ≥1-element lower bound falls out of
//! `scan_safe`'s `EmptyStack` reject, so it is *not* a second check).
//!
//! Pure — no I/O. Where the bytes *land* (symlink-escape, the filesystem floor) is
//! [`crate::open_discipline`]'s job, a distinct total function; this module confines only what
//! the box can *say*.

use proto::{Reject, SafeSegmentStack, scan_safe};

/// The array-level element-count bound.
///
/// **Provisional** (`spec/leaf-validator.md` R-1): the *shape* (a bounded upper count,
/// fail-closed on exceed) is settled; the *number* is marked for review. Any value ≥ the
/// deepest legitimate quarantine path is safe — too low only *over-denies* (fails safe),
/// never over-admits.
pub const MAX_ELEMENTS: usize = 64;

/// Why a destination was denied. Exactly the two arms of the total function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeafDenial {
    /// The per-leaf / structural scan rejected (`proto::scan_safe`) — the *inherited* floor:
    /// out-of-alphabet byte, `.`/`..` traversal, empty segment, or an empty stack (the
    /// ≥1-element lower bound).
    Scan(Reject),
    /// The array-level count bound — the one arm this module adds over the inherited scan.
    TooManyElements { count: usize, max: usize },
}

/// The leaf-validator total function (`spec/leaf-validator.md` §1): `scan_safe` ∘ count-bound.
///
/// Total over every `&[u8]`: exactly one `Ok(stack)` or `Err(denial)`, no panic, terminates.
/// `dest` carries only box-supplied leaf-name bytes — the proxy owns the separators and the
/// quarantine root, so `dest` is a segment stack and nothing else (`/` is not representable in
/// the safe alphabet).
pub fn leaf_validate(dest: &[u8]) -> Result<SafeSegmentStack, LeafDenial> {
    // Inherited floor: per-leaf alphabet, ≥1-alnum, traversal-unsayable, ≥1-element — all
    // decided by proto::scan_safe (proven total in spec/scan-safe.md; per-byte domain swept in
    // proto's own segment_alphabet.rs, cited not re-swept here).
    let stack = scan_safe(dest).map_err(LeafDenial::Scan)?;

    // The array-level upper bound is the one new arm — shared with the prescanned entry so both
    // call-sites run one count-bound implementation (Sprint 6 S1: single-scan-site architecture).
    leaf_validate_prescanned(&stack)?;
    Ok(stack)
}

/// The **count-bound-only** leaf check, over an **already-scanned** [`SafeSegmentStack`]. The entry
/// the dispatch layer uses after `proto::FileWrite`'s decode-time `scan_safe`, avoiding a second
/// scan (Sprint 6 S1; Sprint 5's forward-looking double-scan note).
///
/// **Soundness of skipping the scan here (the type-construction audit):** a [`SafeSegmentStack`] can
/// be obtained *only* through `proto::scan_safe` — its interior `Vec` is a private field with no
/// other constructor (no `pub` field, `new`, `From`, `Default`, or `Deserialize`; verified in
/// `proto/src/segment.rs`, and re-asserted by the trybuild bypass case). So a value of this type is
/// **provably already floor-validated** (alphabet, ≥1-alnum, traversal-unsayable, ≥1-element), and
/// this entry drops **no** floor — it adds only the array-level upper bound the scan does not carry.
/// The ≥1-element lower bound is *unrepresentable* here (an empty stack cannot be constructed), so
/// it is inherited from the type, not re-checked.
pub fn leaf_validate_prescanned(stack: &SafeSegmentStack) -> Result<(), LeafDenial> {
    let count = stack.len();
    if count > MAX_ELEMENTS {
        return Err(LeafDenial::TooManyElements {
            count,
            max: MAX_ELEMENTS,
        });
    }
    Ok(())
}
