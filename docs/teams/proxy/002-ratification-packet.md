# Sprint 002 — Ratification Packet

> Slice 3 deliverable: the consolidated judgment-call list from both Sprint-2 specs, each
> with its recommended default and owner, plus the two doc-wording-fix routings — one
> complete handoff to the PM for onward routing to design/policy. **Neither spec is
> frozen until these are ruled** (contract Final State 5; the Sprint-1 L10 pattern —
> gate-ACCEPT precedes ratification). Assembled by zero.proxy-dev, 2026-07-18; both specs
> gate-accepted (Slice 1: SA1–SA7; Slice 2: SA1–SA7).

Naming note: Slice-1 calls carry their spec's `J*` names; Slice-2 calls are `JT-*` —
no name is used by both specs (the `J4` collision flagged at planning was avoided by the
`JT-*` scheme).

## 1. Judgment calls — `docs/proxy/spec/filesystem-write-path.md` §5

| Call | Question | Recommended default | Owner |
|---|---|---|---|
| J1 | `openat2`-only, no fallback walk for pre-5.6 kernels | Yes — one reviewed path (`principles.md` §15); a failing platform pin is a platform-support decision, not a reason to widen the audited surface | design |
| J2 | Include `RESOLVE_NO_XDEV` (write path) and `nosymfollow` (mount) beyond the sketch | Yes — cheap, strictly narrowing; `nosymfollow` adds reader-side cover the write-path mask cannot reach | design |
| J3 | The startup-assertion convention itself (proxy verifies its own kernel-enforced floors at boot, fail-closed) | Yes — ratify as a standing convention; new, present in no source | design |
| J3b | `nosymfollow` verification method (not `statvfs`-visible) | Single-line `/proc/self/mountinfo` check | design |
| J-coll | Collision policy: `O_CREAT\|O_EXCL` always fails a name collision | Yes — no-overwrite is the requirement verbatim; hold-fd-per-transfer forecloses the only legitimate same-name rewrite mechanically | design |
| J4 | Root-path hardening (`openat2`+`RESOLVE_NO_SYMLINKS` from a pinned ancestor) vs name-as-operator-trust (residual R-H) | Don't harden — constrains legitimate operator symlinked root paths for zero box-surface gain; one-line change if ruled otherwise | design |
| J-kernel | May tier-1 floors rest on kernel enforcement at all (`RESOLVE_*`, mount flags) — the ledger-L11 class | Acceptable — the posture the kernel already carries; explicitly policy's call, routed with L11 | design + policy |

Provisional scalars (marked in-spec, non-blocking): `umask 077`; directories `0700`;
files `0600`.

## 2. Judgment calls — `docs/proxy/spec/trusted-address.md` §9

| Call | Question | Recommended default | Owner |
|---|---|---|---|
| JT-1 | Domain-segment validation beyond alphabet + byte-exact match (label rules, lengths, `..`-inside-domain) | No — exact match against the operator's canonical key is the whole guarantee; added validation is surface without containment gain | design |
| JT-2 | Scheme policy: admit `0`/https only; every other u32 value (incl. all reserved) denies | Yes — https-only; any extension is a deliberate, legible spec revision (promotion pattern), never default-open | design |
| JT-3 | Register load-time well-formedness check (dead-key lint: reject keys outside `assemble`'s image at config load) | Yes — new convention; operator-authored input (adversary class: none), fail-closed at load, kills the silent dead-entry footgun (RT-2) | design |

Provisional scalars (marked in-spec, non-blocking): assembled-length cap `1024`;
segment-count cap `64`.

## 3. Doc-wording-fix routings (stated in the specs; routed to owners via the PM)

| # | Source text | Divergence | Stated at | Owner |
|---|---|---|---|---|
| W1 | `filesystem.md` §1 — "refuse to follow a symlink at any component (`O_NOFOLLOW`)" | Overcredits the primitive: per `open(2)`, `O_NOFOLLOW` is final-component-only; `RESOLVE_NO_SYMLINKS` is what meets the any-component requirement (contract RQ2) | `spec/filesystem-write-path.md` §0 | `filesystem.md` owner |
| W2 | `approach.md` §7 — "the proxy supplies the structure — `://`, `.`, `/`" and "a *distinct* system" | Committed wire (`net.md` §1, `segment.rs`): the box supplies the domain's dots inside segment 0, and the address reuses the file-leaf stack/alphabet, distinct only by scheme prefix + position convention (contract RQ3) | `spec/trusted-address.md` §10 | `approach.md` owner |

## 4. Related standing items — informational, tracked elsewhere, not new routing

- Commit dependencies: `terminal.md` §5 per-vector fix (Sprint 1, still working-tree);
  `net.md`/`model.md`/`file.md` working-tree changes (pinned in
  `spec/trusted-address.md` §8); both specs' banners carry their dependencies.
- Deploy-host re-verification of the bench pins (spec §7 appendix, residual R-E) when a
  proxy deployment host is designated.
- L10 re-confirmation note (policy memo's since-corrected IOMMU premise) rides with L11 —
  `architecture.md` §11 addendum.
