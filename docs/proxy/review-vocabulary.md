# Review vocabulary — the gate bars, defined once

The proxy team's sprint documents cite gate bars by short name: `MB4`, `CB2`, `IB1`, `U3`,
`JC-1`, `L9`, `FS#5`, `R-3`. Before this document each family was **restated** in whichever
sprint doc needed it — the actual bar existed only as repetition across ten documents, which
can drift between restatements, and did (see *Corrections* at the end).

This is the single definition. **Sprint documents cite it; they do not restate it.** A sprint
that needs a bar narrowed or widened for its own scope says so explicitly *relative to this
document* ("MB6, widened this sprint to include …") rather than silently writing a different
version.

## Two families are defined elsewhere — cite those, not this document

Recording where, because re-defining them here would create exactly the duplication this
document exists to remove:

| family | canonical home | note |
|---|---|---|
| `JC-1` … `JC-4` | `contract/box-proxy.md` §"Joint concerns" (lines 57–80) | **Four**, not three — `JC-4` (log discipline) is easy to miss because most sprints only needed 1–3 |
| `L1` … `L11`+ | `architecture.md` §11, the open-questions ledger table | Each row carries its own status; the ledger is the live record, not a snapshot |

## `CB1`–`CB6` — the adversarial-closure bar (origin: Sprint 5)

For **tier-1 security-invariant code**, where the question is "does this correctly deny
everything it must deny".

- **CB1 — Coherence.** Prove the test run is the *current* binary's, not a stale one.
- **CB2 — The spec is the oracle.** Every accepted-spec closure row becomes a corpus case.
- **CB3 — Enumeration coverage, not test count.** The proof is that every enumerated arm is
  exercised — a count of tests proves nothing about coverage of the enumeration.
- **CB4 — Bind the whole chain.** No mock stands in for the real primitive.
- **CB5 — Property/fuzz testing for the recursive recognizer (U4).** Beyond the enumerated
  corpus: fuzz for the deny-default complement.
- **CB6 — Rust safety.** No panic on any corpus/fuzz input; termination is proven.

## `MB1`–`MB6` — the relay/plumbing bar (origin: Sprint 6)

For **tier-2 box-reachable supporting code**, where the question is inverted: "does this
correctly *not* gate, while staying live and safe". Sprint 6 set these as the standing bar and
every sprint since has applied them per slice.

- **MB1 — Coherence** (carries over from `CB1` unchanged): the test run is the current
  binary's, proven by a build-signature marker, never a stale-binary green.
  *REJECT on a stale-binary run.*
- **MB2 — Correctness / relay-fidelity.** The relay moves the *right* bytes to the *right*
  place, unmodified. The oracle is byte-identity (output == input) plus correct routing — not
  an admit/deny oracle.
  *REJECT if any relay's output diverges from its input, or a frame routes to the wrong
  session/service.*
- **MB3 — Tier-fidelity: it does not gate.** The relay path contains no content-based
  decision — no filter, no neutralization, no deny-on-content. The unifying tell: a relay
  routes by *envelope* (session id, verb); the moment it branches on what the *content is*,
  that is tier-1 logic that crept in un-reviewed.
  *REJECT + relocate on any content-based conditional, transform of relayed bytes (even a
  "harmless" normalization), or a failure mode drifting toward "denied on content" rather than
  "feature failed."*
- **MB4 — Liveness.** No deadlock, livelock, leak, starvation, or unbounded growth. Bounded
  resources; RAII/no-fd-leak; backpressure that blocks or drops cleanly, never hangs or grows
  unbounded.
  *REJECT on any deadlock, unbounded resource growth, fd/session leak, or evidence one channel
  can starve another, under load/churn/backpressure testing.*
  **Note (Sprint 6's own correction):** for *stateful* tier-2 code, MB4 is not merely
  deadlock-freedom — an unbounded binding map passes a deadlock check and still fails MB4.
- **MB5 — Memory safety under malformed input.** The no-panic half of `CB5`'s fuzz survives;
  its deny-default half does not apply (there is nothing to deny-by-default over). Fuzz
  malformed, truncated, oversized, boundary input.
  *REJECT on any panic, UB, or hang. The fuzz is required — repurposed for robustness, not
  dropped because the sprint is not adversarial.*
- **MB6 — Rust safety.** Pure safe Rust for all relay logic; `deny(unsafe_code)` at the crate
  root holds; `unsafe` is confined to the audited allow-list module set.
  *REJECT on any new `unsafe` outside that set, or a broken `deny(unsafe_code)` crate-root
  posture.*

  **MB6's allow-list is versioned, and this is its current value:**
  **`{open_discipline, privilege}`** — verified against source: `src/lib.rs:1`
  (`#![deny(unsafe_code)]`), `src/open_discipline.rs:16` and `src/privilege.rs:16` (each
  `#![allow(unsafe_code)]`). No other module re-allows it.

## `IB1`–`IB3` — the integration bar (origin: Sprint 8)

For **lifecycle/real-hardware integration**, where the question is "does the sequencing hold
against a real system rather than a fake".

- **IB1 — Lifecycle fail-closed.** Each rung either succeeds and advances, or fails and stops.
  *REJECT on any rung that degrades, logs-and-skips, or continues past a failure.*
- **IB2 — Privilege-drop observed, not asserted** (the sharpest of the three). The proof is an
  observation of post-drop credential state, not a claim that the drop was called.
  *REJECT unless the post-drop credential state is positively observed.*
- **IB3 — Real-API conformance, re-derived at the actual cited lines, split by layer.**
  *REJECT on any fake-wire assumption not re-verified against the real API.*

## `U1`–`U5` — the security-invariant core's five units (origin: Sprint 5)

Not a gate bar — the **naming scheme for the tier-1 units**, used throughout the code's own
module headers. Recorded here because the sprint docs and the source both cite these names
constantly and neither defines them in one place.

| unit | what it is | source |
|---|---|---|
| `U1` | filesystem leaf-validator + open-discipline primitives | `src/leaf.rs`, `src/open_discipline.rs` |
| `U2` | address representation consumption + address recognizer | `src/address.rs` |
| `U3` | route matcher / register enforcement (exact-match, deny-by-default, never widened at runtime) | `src/route.rs` |
| `U4` | the Anthropic request recognizer (`spec/anthropic-recognizer.md`) | `src/anthropic.rs` |
| `U5` | the compiled-in Rust register (`cloud/approach.md` §6) | `src/register.rs` |

`U2`, `U3` and `U5` share one vocabulary of validated types, pinned once in `src/vocab.rs`
specifically so they cannot diverge.

`U5`'s own guarantees are lettered `C1`–`C5` in `src/register.rs`'s module header: `C1` invalid
is a compile error; `C2` per-service; `C3` the box cannot author it; `C4` a change is
edit + recompile + re-review; `C5` boring, not clever.

## Sprint-document conventions

- **`FS#N`** — "Final State #N", an item in the sprint contract's *Final State* list. Scoped to
  the sprint that defines it: `FS#5` means something different in Sprint 18 than in Sprint 20.
  Always cite as "Sprint NNN's `FS#N`" when referring across sprints.
- **`R-N` / `RQ-N`** — a *Review Question* in the sprint contract, numbered within that sprint.
  Same cross-sprint ambiguity, same rule.
- **Slice N** — an ordered unit of work inside one sprint, gated at its close.

## Corrections this document makes to the prior restatements

Recorded rather than silently resolved, per the audit's own discipline.

1. **`MB6`'s allow-list drifted from its origin and was never corrected there.** Sprint 6
   defines it as `open_discipline` alone. Sprint 8 added `privilege` (the privilege-drop work).
   From Sprint 9 onward every restatement reads `{open_discipline, privilege}` — several
   describing it as "holds unchanged", which is true relative to Sprint 9 but not relative to
   the Sprint 6 origin they inherit from. The current value above is verified against source,
   not against any restatement.
2. **`JC` has four members; most sprints cite three.** `JC-4` (log discipline) is defined in
   `contract/box-proxy.md` alongside 1–3. A sprint citing "`JC-1`/`JC-2`/`JC-3`" is naming the
   three it needed, not the whole family.
3. **`MB2` and `MB3`'s names vary between restatements** — "Correctness / relay-fidelity" vs
   "Correctness/relay-fidelity"; "Tier-fidelity: it does not gate" vs "Tier-fidelity (does not
   gate)". Cosmetic, no semantic difference found; the spellings above are Sprint 6's.
4. **`MB1`–`MB6` were stated twice within Sprint 6 itself** — once as property definitions
   (Method) and once as REJECT clauses (Acceptance). Both are preserved above, merged per
   bar, because they carry different information: what the property *is*, and what *fails* it.
