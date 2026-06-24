# Sprint 005 — Net Service

> The fifth sprint of the `proto` crate: the box↔proxy **net** service — `net-response`, `net-error`,
> `net-payload` — and the two genuinely-new mechanisms the prior sprints queued: the **L1 frame envelope**
> (the continuation bit, deferred since 001 Fork #4) and the **bounded, fail-closed reassembler** (the
> protocol's *one* trusted-side reassembly). This is the first **trusted-side** decoder: the **proxy**
> holds the reassembly buffer, so totality here is **TCB defense** — a malformed or unbounded payload
> must not crash, exhaust, or hang the proxy. The request **recognizer is off-wire** (proxy/cloud's
> security-invariant core, not the crate); the crate reassembles the verbatim payload and hands it off.
>
> Team: `proto` (lead `zero.proto-lead`, dev `zero.proto-dev`, review `zero.proto-review`).
> Grounded in [`net.md`](../../proto/net.md), [`model.md`](../../proto/model.md) §1/§2/§3,
> [`library.md`](../../proto/library.md) §1/§5/§8, [`approach.md`](../../proxy/cloud/approach.md) §1–5,
> and the 001–004 contracts.

---

## Scope Boundary

**This sprint owns:**

1. **The L1 frame envelope** — `[session_id u32][flags u32: bit0 = continuation, bits 1–31 reserved →
   drop][L2 [verb][body]]` (8 bytes; L2 stays 4-byte aligned). The crate **parses** it
   (`decode_frame`/`encode_frame`); the consumer routes by `session_id` (the crate never acts on it).
   Resolves the §8 envelope-ownership decision (001 Fork #4 deferred it).
2. **The bounded reassembler** — accumulates the L2 payload while the continuation bit is set, finalizes
   on the clear bit, **bounded by the cap**, **fail-closed** on cap-exceed or never-ending. Single-session
   (the crate holds no map; the **proxy** keys `session_id` → `Reassembler` — no_std-friendly). Opaque:
   concatenates L2 bytes without parsing them; the boundary is the **frame + the continuation bit only**,
   never a length in content (law 11).
3. **`net-payload` (0x33)** — box→proxy, the verbatim request body reassembled by the proxy before the
   off-wire recognizer reads it.
4. **`net-response` (0x31)** — proxy→box, **verbatim, NOT reassembled** (the box reassembles; free relay).
5. **`net-error` (0x32)** — reuses `ErrorBody`.

`IMPLEMENTED` grows 17 → 20. `net-address` 0x30 is a 001 seed.

**This sprint does NOT own** (named, with its owner):

- **The request recognizer** — off-wire, in proxy/cloud's security-invariant core (approach.md §3/§8);
  the crate hands it the reassembled verbatim bytes, never recognizes.
- **`net-response` reassembly** — the proxy does **not** reassemble the response (the asymmetry: egress is
  hardened because the recognizer is total over a whole request; the response lands on the untrusted box
  and is relayed per-frame, the box reassembling — net.md §3 / approach.md §5). Do not leak the
  reassembler onto the response path.
- **`key` 0x12** (keysym table), **`open-session` 0x00**, **feature-gating** — later sprints.

## Problem

Everything to date is one-message-per-frame, stateless, single-slice, box-side. `net-payload` breaks all
four: a real request (a multi-kilobyte-to-multi-megabyte Anthropic POST, image-bearing) exceeds the wire
frame, so it spans frames via the continuation bit and the **proxy** reassembles the whole request before
the recognizer can rule on it (recognition is total over a complete request — net.md §2 / approach.md §2).
This is the first trusted-side buffer, the first stateful/multi-frame decoder, and it brings the L1
envelope (deferred since 001) into scope.

## Goal

The box encodes and the proxy decodes the net verbs through the one crate-owned grammar. The L1 envelope
decodes totally (continuation bit from a fixed position, never content). The reassembler accumulates a
multi-frame payload opaquely and **cannot be made to crash, over-read, unbounded-allocate, or hang by any
frame sequence the box authors** — a payload exceeding the cap or never ending **fails closed** (denied).
`net-response`/`net-error` are simple verbatim/ErrorBody messages. The prior protected surfaces stay
byte-identical.

## Final State

- `envelope.rs`: `Frame{session_id: u32, continuation: bool, l2: &[u8]/Vec<u8>}`; `decode_frame` (total;
  `< 8` → `ShortBody`; reserved flag bits dropped) / `encode_frame`. The crate parses; the consumer
  routes by `session_id`.
- `reassembler.rs`: `Reassembler{acc, cap}`; `push(chunk, continuation) → Accumulating | Complete(bytes)
  | Err(ReassemblyOverflow{cap})`. Single-session; the proxy holds the `session_id → Reassembler` map.
- `net.rs`: `NetResponse(Vec<u8>)` verbatim, `NetError(ErrorBody)`, `NetPayload` (the reassembled bytes).
- Dispatch wired for 0x31/0x32/0x33; `IMPLEMENTED` = 20; `REASSEMBLY_CAP` (provisional) in the
  wire-numbers surface. `Reject` gains `ReassemblyOverflow` (structural/resource — see Decided Forks).
- The protected anchors — segment/cell/display/dimensions + the new **errors.rs** (net-error reuses
  `ErrorBody`) — byte-identical.

## In Scope — ordered slices (review seams)

The developer produces the **5A pre-carve creation manifest** (the envelope + the reassembler + the 3
verbs) and **stops for review before any carve**.

- **5A — the L1 envelope + the bounded reassembler** (the headline TCB-defense mechanism, isolated first).
  `decode_frame`/`encode_frame`; `Reassembler` (accumulate, finalize, **fail-closed at the cap**,
  never-ending via the cap). *Gates:* the **TCB-defense fail-closed gate** (G2 over multi-frame
  sequences) + G2 source-discipline. Gated standalone before the verbs ride it.
- **5B — the 3 verbs + dispatch.** `NetPayload` using the reassembler; `NetResponse` (verbatim, **NOT
  reassembled**); `NetError(ErrorBody)` (reuse). Dispatch arms 0x31/0x32/0x33; `IMPLEMENTED` = 20. *Gate:*
  round-trip + dispatch + reuse-not-reimplement (net-error through `ErrorBody`); the asymmetry held.
- **5C — multi-frame vectors + fuzz + G0.** Hand-authored **multi-frame** golden vectors (a payload across
  N frames → joined bytes; a **cap-exceed reject** vector; a **never-ending reject** vector) + net-response/
  net-error vectors; a **second fuzz target** driving frame→envelope→reassembler over arbitrary frame
  sequences (the TCB-defense make-or-break); full G0 with the coverage anchor re-tightened to
  `covered == IMPLEMENTED(20)`. *Gates:* G2 (full, both fuzz targets), G3, G0, G6.d.

## Non-Goals

- The off-wire recognizer; `net-response` reassembly (the asymmetry); `key`/`open-session`/feature-gating.
- A box-authored length anywhere in the framing (law 11 — the boundary is the frame + the continuation bit).
- **Do not edit** segment/cell/display/dimensions/**errors.rs** (net-error reuses `ErrorBody` via its
  public API).
- The cap is **not** a blocker — a provisional const (see Escalations); the gate tests the *mechanism*.

## Starting Surface

005 is **additive**; the prior surfaces are untouched. New: `src/envelope.rs` (the L1 `Frame` +
decode/encode), `src/reassembler.rs` (the `Reassembler` state machine), `src/net.rs` (+3 verbs). Touches:
`reject.rs` + `ReassemblyOverflow`; `verb.rs` `IMPLEMENTED` 17→20 + the provisional envelope-layout consts
+ `REASSEMBLY_CAP`; `message.rs` +3 arms; `lib.rs` +exports. The 4 prior anchors + the new **errors.rs**
anchor (net-error reuses `ErrorBody`) stay byte-identical. The L1 envelope + reassembler are new layers
**above** `decode_message` (L2).

Inventory shape (as prior): `verb id | name | direction | body shape | decoder kind | type | module |
005 action`, plus the envelope layout + the reassembler state machine.

## Method

Inherits the established method (owned decode; bounded accessors; build coherence; non-circular
hand-authored vectors; the per-seam live-path cascade — here **proxy/trusted-side**; content-hash
behavior preservation; stop-for-review). The genuinely-new discipline:

- **TCB-defense totality (stricter than box-side robustness).** The proxy holds the buffer, so a
  malformed/unbounded payload DoS-ing the proxy is a real attack. The make-or-break: a payload that
  **exceeds the cap, or never ends** (continuation bit never clears) **fails closed** — a typed reject,
  **never panic, never over-read, never unbounded-alloc, never hang**. The cap does **double duty**: it
  bounds the buffer (alloc) **and** terminates a never-ending stream (the reassembler checks the cap each
  frame and denies once exceeded — so "never-ending" reduces to "cap-exceed", the same fail-closed path;
  it does **not** wait for a clear bit it may never get). *The cap-boundary is a binding off-by-one point*
  (`accumulated > cap` → reject; define `==` inclusive or exclusive precisely).
- **The new alloc-cap relationship.** Prior decoders were frame-bounded (`alloc ≤ frame_size·k`). The
  reassembler accumulates **across** frames, so its buffer is bounded by **the cap** (`> one frame`). The
  reassembly-buffer alloc gate is therefore: the buffer never exceeds the cap (fail-closed when it would),
  proven dynamically by the multi-frame fuzz over the never-ending/cap-exceed cases.
- **The L1 envelope — boundary from frame + bit, never content.** The continuation bit is read from a
  **fixed flag-byte position** (`flags` bit 0), never from content; accumulation is **opaque** (concatenate
  the L2 bytes without parsing them; the complete message decodes normally only *after* reassembly). No
  box-authored length anywhere — adversarial content cannot steer the reassembly (law 11, carry-not-
  rediscover applied to framing). `decode_frame` is total (`< 8` bytes → `ShortBody`; reserved flag bits
  1–31 dropped per the 002 reserved-bit-drop).
- **The asymmetry — `net-response` is NOT reassembled.** The reassembler must not leak onto the response
  path; egress is hardened (the recognizer is total over a whole request), ingress is a free per-frame
  relay (the box reassembles).
- **Single-session, crate holds no map.** `Reassembler` is one session's accumulation; the **proxy** keys
  `session_id → Reassembler` (the crate allocates no map — no_std-friendly).
- **A second fuzz target.** The existing target drives `decode_message` (L2, single-frame). A **new** target
  drives frame→envelope→reassembler over **arbitrary frame sequences** (the multi-frame TCB-defense path) —
  a single-frame fuzz is a subset-shadow on the reassembler.
- **Corpus hygiene (review's refinement).** Review's 5C fuzz reproduction runs **read-only** (`-runs=0` for
  the coverage figure) or against a **temp copy** (for the totality run), so it does **not** mutate the
  committed corpus; the dev's cmin stays last and on-disk == committed with no post-review re-minimize.

## Acceptance

All **001–004 gates G0–G6 carry forward**, green on the post-005 tree. New/refined obligations:

- **The TCB-defense fail-closed gate (the make-or-break).** Over **multi-frame** sequences: **cap-exceed**
  (accumulate past the cap), **never-ending** (continuation always set), **truncated session** (frames stop
  mid-reassembly, no clear bit), arbitrary/interleaved sequences, and a **malformed envelope** — ALL →
  `Ok(complete) | Err(denied)`, **never panic / over-read / unbounded-alloc / hang**. The cap-boundary as a
  typed reject (`ReassemblyOverflow{cap}`), the off-by-one precise. **The fail-closed must be EXERCISED at a
  reachable cap, not merely structurally present** — at 32 MiB neither a fast test nor the bounded-input
  fuzz can drive `acc` past the cap (8200+ frames per input), so cap-exceed would be `checked=0` (a
  subset-shadow on the make-or-break itself). `Reassembler{acc, cap}` takes the cap as a **field**, so the
  gate exercises it three ways: **(1) mechanism** — construct a `Reassembler` with a **small test-cap**;
  assert `cap-1` → Ok, `cap` → per the inclusive/exclusive rule, `cap+1` → `Err(ReassemblyOverflow{cap})`,
  the boundary **derived from the test-cap field** (never a hardcoded size — re-derives at any cap); **(2)
  never-ending** — a small test-cap, push continuation-always-set frames until `acc > cap` → `Err` (not
  accumulate-forever, not a hang); **(3) the multi-frame fuzz constructs the `Reassembler` with a small
  cap** so it genuinely reaches cap-exceed and proves fail-closed (no panic/over-read/unbounded-alloc/hang)
  over arbitrary sequences (its global alloc-cap set above the test-cap but small enough to catch an
  *unbounded* bug-alloc). **(4) Production wiring** — a separate deterministic test asserts the net-payload
  reassembler is constructed with `REASSEMBLY_CAP` (32 MiB) — production value confirmed wired (can't
  silently drift) **without** a 32-MiB alloc. The reassembly buffer never exceeds the cap (the cap *is* the
  alloc bound — distinct from `MAX_FRAME·k`). A 5A where cap-exceed/never-ending is only structurally
  present (the 32-MiB cap, never driven past it) = REJECT.
- **G1 — round-trip.** Over the 20 `IMPLEMENTED` verbs. The envelope round-trips (`decode_frame`/
  `encode_frame`); a multi-frame payload reassembles to the joined bytes and the complete message decodes;
  `net-response` verbatim; `net-error` through `ErrorBody`.
- **G3 — non-circular MULTI-FRAME vectors.** Hand-authored byte-by-byte (review re-derives a sample: the
  envelope layout + the continuation bit per frame + the opaque join): a payload across N frames → joined
  bytes; the **cap-exceed reject** + **never-ending reject** vectors (the permanent fail-closed guards). A
  new vector *shape* — a **frame sequence**, not a single slice; the harness feeds the reassembler a
  sequence.
- **G4 / G5** — unchanged (no new segment/logger surface; the payload is verbatim/opaque, never logged —
  net.md / approach.md §7: only the safe address is logged).
- **G6** — `#![no_std]`+`alloc`+`forbid(unsafe_code)` intact; the standing `x86_64-unknown-none` build.
- **`Reject`** gains `ReassemblyOverflow{cap}` — a **structural/resource** reject (content-agnostic: it
  rejects on the accumulated byte-count bound, not on any value), so the structural-only coherence tell
  holds (no value-policy variant).
- **Behavior preservation (content-hash).** segment/cell/display/dimensions byte-identical, **plus the new
  errors.rs anchor** (net-error reuses `ErrorBody`, must not edit it). Additive edits rehash; a changed
  protected-surface hash is a **REJECT**.

## Decided Forks (lead rulings — engineering, co-formulated; dev + review concur)

1. **Envelope (§8 resolved): crate owns the L1 layout + parses it.** `[session_id u32 LE][flags u32 LE:
   bit0 continuation, bits 1–31 reserved → drop][L2]` — 8 bytes, L2 4-byte aligned (the dev's alignment
   rationale over a 1-byte flag, consistent with model §5). `decode_frame → Frame{session_id, continuation,
   l2}`; `encode_frame` box-side (single-source, anti-drift). The crate parses; the **consumer routes by
   `session_id`** (the crate never acts on identity/multiplexing — that is the transport's/consumer's).
   `< 8` → `ShortBody`. The envelope **layout is provisional** in the wire-numbers surface (session-id
   width, the continuation-bit position) — see Escalations.
2. **Reassembler: crate ships it, single-session, fail-closed.** `Reassembler{acc, cap}`; `push(chunk,
   continuation)` → `Accumulating | Complete(bytes) | Err(ReassemblyOverflow{cap})`. The crate holds **no
   session map** (the proxy keys `session_id → Reassembler` — no_std-friendly). Never-ending handled by the
   **same cap** (check each frame, deny once exceeded — does not await a clear bit). Never a content length
   (law 11). `ReassemblyOverflow{cap}` is **structural/resource** (content-agnostic resource bound, not a
   value policy — the coherence tell holds).
3. **Cap = provisional 32 MiB.** Matches the Anthropic API's own ~32 MB request limit, so the proxy fails
   closed exactly where the upstream would and **admits legitimate image-bearing POSTs** (8 MiB would
   wrongly deny them — *conceded*). **Provisional** in the wire-numbers surface (operator reviews alongside
   the other numbers); **distinct from `MAX_FRAME·k`** (this is the total reassembled-request bound). The
   gate tests the fail-closed **mechanism** at this value, not the value itself (mechanism-over-value), with
   the **boundary test source-anchored to the const** (re-derives at any cap). In the wire-numbers surface
   it is labeled the **fail-closed REASSEMBLY CAPACITY BOUND** — a capacity/security tuning
   (memory-buffered vs legitimate-request-size), **distinct in *kind*** from the protocol numbers
   (verb-ids/op-set/error-codes/MouseKind/envelope-layout) — so the operator reviews it with the
   capacity/security lens.
4. **`net-response` 0x31 → `NetResponse(Vec<u8>)`** verbatim, **NOT reassembled** (the asymmetry). **`net-
   error` 0x32 → `NetError(ErrorBody)`** reusing `ErrorBody` (route through, no re-impl).
5. **Module split:** new `envelope.rs` (L1 `Frame`) + `reassembler.rs` (the state machine) + `net.rs` (the
   3 verbs) — the envelope + reassembler are new layers above `decode_message` (L2).
6. **A second fuzz target** drives frame→envelope→reassembler over arbitrary frame sequences (the
   multi-frame TCB-defense path), distinct from the L2 `decode_message` target.

## Escalations (operator-reserved — routed via the PM)

- **The reassembler cap value — PROVISIONAL, not a blocker.** Assigned 32 MiB (the Anthropic ~32 MB limit),
  marked provisional in the wire-numbers surface; the operator reviews it alongside the verb-ids / op-set /
  error codes / MouseKind / the envelope layout. The gate tests the fail-closed *mechanism*, so the
  provisional value does not weaken it. (Per the PM + the no-block-on-scalars steer.)
- **The L1 envelope layout** (session-id width, the flag-byte/continuation-bit position) + verb-ids — all
  **provisional** pending operator ratification via the wire-numbers surface; vectors re-blessed if a
  number moves.

## Review Questions

1. **Ratify the reassembler cap (32 MiB), the L1 envelope layout, and the verb-ids** in the wire-numbers
   surface. (Operator — review, not a gate.)
2. Confirm the 5A creation manifest + seam carve before any code. (Review, at the 5A pre-carve stop.) —
   **Confirmed; sprint accepted across 5A/5B/5C.**

---

## Postmortem — Sprint 005 retrospectives

*Sprint 005 closed 2026-06-24: all gates accepted by independent review across 5A/5B/5C. The net service is
complete (net-response/net-error/net-payload + net-address from 001), and the two genuinely-new mechanisms
are built, gated, and proven — the L1 frame envelope (resolving the envelope decision deferred since 001
Fork #4) and the bounded, fail-closed reassembler (the protocol's one trusted-side reassembly). The
TCB-defense make-or-break — a malformed/unbounded payload cannot crash, over-read, unbounded-allocate, or
hang the proxy — is genuinely exercised at a reachable cap and reproduced (1M-exec fuzz against a temp
copy, the capping allocator never firing). 20 of 22 verbs implemented; only `key` 0x12 (keysym table) and
`open-session` 0x00 remain. Builds on the 001–004 postmortems; durable lessons, not status.*

### Developer (`zero.proto-dev`)

**What moved the plan.** (1) *A fail-closed resource bound must be EXERCISED at a reachable cap; the
value-INDEPENDENT off-by-one lets a small cap validate the production one* — three pieces, no 32-MiB alloc:
a mechanism/boundary test at a small reachable cap (`cap-1`/`cap`/`cap+1` from the cap field), a
production-wiring test (`cap() == REASSEMBLY_CAP`, no alloc), and a fuzz at a small cap so cap-exceed is
genuinely hit with a capping allocator above it that never fired (reject-before-grow *is* the alloc bound).
Scales to any cap. (Chosen over a literal 32-MiB feed — surfaced as a fork, ruled.) (2) *A cross-cutting
layering decision resolves cleanly only when its consumer arrives* — the L1 envelope held open from 001
landed in 005 because the reassembler, which reads the continuation bit, forced a concrete layout. (3)
*Opaque-concat + decode-ONCE: a multi-frame reassembler must be content-blind* — concatenate raw L2
opaquely (verb in frame 1 only, continuations raw), decode once on the whole; per-frame verb-decode would
mis-read raw payload as a verb and leak content-awareness into framing (law-11). (4) *A stateful/multi-frame
mechanism needs its OWN adversarial fuzz target* — the single-frame decode target never enters the
reassembler (a subset-shadow).

**What failed / surprised.** The shared corpus is a *two-party* hygiene problem — dev-cmin-last is necessary
but not sufficient (002/003 needed a reconcile because the reviewer's fuzz run appended after it); 005 was
clean because both held (dev cmin-last on both corpora + reviewer reproduces read-only/temp-copy).
`ReassemblyOverflow` is the first non-parse-geometry reject but still structural/resource
(content-agnostic byte-count), so the no-value-policy coherence tell holds.

**What's next inherits.** Net service complete; L2 codec 20/22. Only `key` 0x12 (blocked on the keysym
table) + `open-session` 0x00 (control, trivial empty body — the smallest verb left) remain; then 22/22.
Beyond L2: feature-gating, the control sprint (the open-session seed), further L1 transport
(multiplexing/transport-stack) on the envelope+reassembler foundation. Carries: the L1 layer; the
structural/resource reject; the small-cap-mechanism + production-wiring pattern; per-mechanism-shape fuzz;
both-corpora/two-party hygiene. Anchors now 6 (errors.rs added). All ids/op-set/codes + the L1 envelope
layout + REASSEMBLY_CAP provisional.

### Reviewer (`zero.proto-review`)

**What 005 added to the method.** (1) *The make-or-break can itself be a subset-shadow — a fail-closed bound
is only proven if the test DRIVES PAST it.* A cap no test or fuzz reaches is structurally-present but
never-exercised (cap-exceed at `checked=0`). Resolution: parameterize the bound (cap as a field), exercise
the mechanism at a small reachable cap, run the fuzz at a small cap so cap-exceed is genuinely reached, and
confirm the production value wired separately — the `≤cap`/`>cap` off-by-one is value-independent, so the
small-cap test validates the production cap too. (2) *Reject-before-grow IS the alloc bound* — the check is
before the append (the buffer never allocates past the cap), `checked_add` so overflow also fails closed,
proven by a capping allocator set just above the small fuzz cap that never aborts over 1M execs. (3)
*Opaque, content-blind reassembly (law-11 framing)* — the boundary is the frame + a fixed-position
continuation bit, never a content length; verify the continuation frames are NOT parsed. (4) *A
stateful/multi-frame mechanism needs its own fuzz target* driving its real input shape (a frame sequence),
judged by whether it reaches that mechanism, misses explained by source. (5) *Two-party corpus hygiene* —
dev cmin-last + reviewer reproduces without mutating the committed corpus (`-runs=0` / temp copy).

**TCB-defense vs box-side robustness.** 001–004 decoded box-side (totality is robustness). 005 is the first
trusted-side decoder: the proxy holds the buffer, so a malformed/unbounded payload DoS-ing the proxy is a
real attack — the fail-closed is TCB *defense*, a stricter gate. The recognizer stays off-wire (the crate
reassembles + bounds + hands off, never recognizes). The crate bounds *one* session; the multi-session
count bound is the proxy's (named, out of scope). **Lesson:** identify which side decodes; a trusted-side
decoder's totality is fail-closed-under-adversarial-input, exercised — not "it doesn't crash."

**What's next inherits.** The fail-closed-EXERCISED discipline; reject-before-grow + checked_add +
capping-allocator-above-the-small-cap; opaque content-blind reassembly; own-fuzz-target per new mechanism;
two-party corpus hygiene. The protected-anchor set is now six (recompute all). `key` 0x12 is the one
operator-blocking item (the unpinned `>0x10FFFF` named-key table); `open-session` 0x00 is the
session-allocation seed (pairs with the envelope `session_id` this sprint pinned). Neither is a new hard
mechanism — the crate is near complete.

### Lead (`zero.proto-lead`)

**Process lessons.** (1) *A gate-design flaw is itself a fork — surface and rule it BEFORE the carve.* The
reviewer caught pre-carve that the fail-closed was unreachable at 32 MiB (the make-or-break would be a
subset-shadow); I ruled the small-cap design over the dev's literal-32-MiB feed and folded it into the
contract *before* any code — not discovered in the evidence. The make-or-break of a sprint deserves its
gate-design settled at the manifest, not the seam. (2) *A scalar is provisional + visible, not a blocker;
concede to the better-grounded value.* I conceded my 8-MiB cap to the dev's 32 MiB (grounded in the
Anthropic request limit — fails closed where the upstream does, admits image POSTs); the value is
provisional and mechanism-gated, never an escalation. (3) *A recurring two-party operational problem needs
both parties' discipline, baked into the close.* After the 002/003/004 corpus reconciles, the reviewer's
read-only/temp-copy reproduction (the cleaner fix) + the dev's cmin-last made 005 clean first-time on both
corpora. (4) *Hold a cross-cutting layering decision until its consumer forces a concrete shape* — the L1
envelope resolved cleanly in 005 (its consumer, the reassembler, arrived) where pre-deciding it in 001
would have guessed.

**005 inherits (process).** The contract is the source of truth; the standing gates + the
exercise-the-fail-closed discipline + the two-party corpus hygiene + the six anchors carry forward; numbers
stay provisional. The wire is near complete (20/22). The one operator-blocking item is the **keysym table**
(it gates `key` 0x12, the last verb) — surfaced to the operator via the PM. `open-session` 0x00 (trivial)
and feature-gating (the library §8 packaging decision) are unblocked; the PM sequences 006.
