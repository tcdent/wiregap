# Sprint 001 — Session Protocol (UART + ACK + ID, pure software)

> The `nic` team's first build sprint (epic **E5**): the session-management heart of the box↔proxy
> transport — the one protocol whose instances *are* the sessions. A UART-style byte pipe plus exactly
> two additions: a per-frame ACK and a session ID. Strict stop-and-wait, retransmit-on-silence with a
> bounded give-up, open-by-emission, no close signal, no cross-epoch state. Built as **pure software**
> against three seams (a transmit sink, a monotonic clock, an inbound-frame feed) that the data path
> (E3) and reactor (E4) implement later — so every invariant is provable with a virtual clock and a
> fake wire, no hardware. This document is the contract: the single source of truth for what this
> sprint owns, what it does not, and what evidence closes it.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`docs/nic/architecture.md`](../../nic/architecture.md) §4, §6, §7, §8 (canonical
> nomenclature §1.1); epic E5 in [`docs/nic/epics.md`](../../nic/epics.md).

---

## Status — green-lit as a split (post proto-010 reconciliation)

A cross-component conflict between this sprint's authority and the built `proto` crate was raised to the
operator and reconciled. **proto sprint 010** ("excise the L1 envelope — L1/L2 boundary correction",
`f9575ad`) removed proto's L1 envelope: proto is now L2 `[verb][body]` only, holds no `session_id`, and
the commit states **"the L1 frame is the transport's… This unblocks the transport team."** Verified by
two independent passes against committed HEAD + `model.md` (the stated authority). Resulting gate:

**Full GO (designer + PM, 2026-06-25), one narrow hold.** All of E5 proceeds; hold only the reclaim code
until the §8 amendment ratifies.

- **Layout cluster resolved in nic's favor:** nic owns the L1 frame, session id is **u64** (§1.1), **no
  flag byte** (the continuation bit is the later reassembly sprint's, out of E5).
- **Directionality (§4) RESOLVED — arch §6 unidirectional is the authority; the crate conforms.** A
  session's data is one-directional, so role/direction disambiguates data from ACK with no flag — the
  whole sprint builds on this. proto's `model.md:353` ("both directions on one session") is now **stale
  and is proto-team doc cleanup; it does not block nic** (arch §6 committed is the authority, not a
  memory or proto's stale line). Slices 3–4 (the role model) are **unblocked**. The reviewer's gate
  fixes [A]–[D] are folded into gates 4–5.
- **Reclaim (§5/R5) RESOLVED — timeout-only (operator, 2026-06-25).** Reclaim is **give-up timeout
  only**: a session reclaims solely when an outstanding unacked frame goes unacked past the give-up
  bound. **No consumer-driven clean-completion path.** A session with nothing in flight (dormant or
  cleanly completed) is **never reclaimed** by the transport — it lingers until epoch reset (the
  operator-accepted memory profile, bounded by per-epoch session count). The designer is updating arch
  §8 to match. The reclaim code is now **fully buildable and finalizable**; no holds remain on E5.

---

## Scope Boundary

**This sprint owns** the session protocol as logic — the subsystem's one-line scope, *manage sessions
within an epoch* (§1.1):

1. The **frame codec**: `[session ID][payload]` — a 64-bit session-ID prefix and an opaque payload
   tail, with **no software length field** (length is supplied by the seam, originating at the RX
   descriptor in E3) and **no flag byte** (§6).
2. The **session table** and **open-by-emission**: an unknown inbound session ID instantiates a session
   and delivers its payload upward; a known ID routes to its session. Zero protocol-type branching at
   dispatch (§4). **No cross-epoch state** — no memory of a retired ID (§8).
3. The **sender role** — strict **stop-and-wait** (at most one unacked frame in flight per session),
   buffer held until ACK, the **~10 ms retransmit** deadline, and the **~1–5 s give-up** → *silent
   internal reclaim* (free buffer, drop session, emit nothing), firing only on an outstanding unacked
   frame, never on idleness (§6, §7).
4. The **receiver role** — deliver the payload upward and emit a **bare ACK** (a return frame on the
   session ID, opposite direction); data-vs-ACK by **role/direction**, never a flag (§6).
5. **Session-ID allocation** (opener side): a 64-bit monotonic counter, unique within an epoch (§1.1).
6. The **interleaving surface** the reactor drives: a **ready-set** of sessions with a frame to send
   (drained trivially — no scheduler, no QoS, §4) and the **next retransmit deadline** to compose into
   `epoll_wait` (the composition itself is E4).
7. The **self-validation loop**: pure-software proof of every protocol invariant against a fake wire +
   virtual clock — round-trip, multi-session interleave, loss→retransmit, give-up→reclaim, dormant≠dead,
   epoch-reuse.

**This sprint does NOT own** (named, with where it belongs instead):

- **The data path / copy boundary / DMA rings (E3)** — descriptors, the RX-descriptor length read,
  copying frames out of DMA. Here a frame enters the protocol as already-owned bytes via the seam.
- **The event reactor (E4)** — epoll, the MSI-X→eventfd idle-wakeup, busy-poll-then-block, the real
  `epoll_wait` deadline composition. Here the clock and wakeup are seams; the protocol exposes the
  ready-set and next-deadline but does not own the loop.
- **Device bring-up / link (E1) and IOMMU/VFIO (E2)** — no device, no syscalls in this crate's logic.
- **The application layer** — PTY/file/request semantics, and **the binding** the first frame carries:
  the transport routes bytes and **does not read the binding** (§4, §8).
- **The L2 wire codec** — the verbs/messages that ride *inside* the payload are the **`proto` crate**.
  This transport carries the payload opaquely.
- **The continuation-bit reassembler** — spanning a logical message across frames is a later
  **reassembly sprint** (gated on the operator-reserved cap). See R4 on the §6 "no flag byte" / proto
  "L1 envelope" seam.
- **The alternating bit** — deliberately omitted (§6); this sprint implements without it and builds no
  dedup. **Honest framing:** there is **no flag byte and no live reserved bit** in the current frame —
  reinserting the alternating (or a continuation) bit later is a **frame-format change / re-scope**, not
  a spare bit already carried on the wire. The frame is exactly `[session ID][payload]`.

## Problem

There is no transport crate. The session protocol is fully specified across `architecture.md` §4/6/7/8,
but no Rust *manages sessions within an epoch*: nothing instantiates a session by emission, holds a
single frame stop-and-wait, retransmits on silence and reclaims a dead peer silently, or proves it does
so over hostile timing. The box and proxy cannot speak until this one protocol — *the same code on both
ends* — exists and is shown correct in pure software, before E3/E4 wrap it in DMA and an epoll loop.

## Goal

A standalone transport crate that **manages sessions within an epoch** and proves it as a closed loop:
a session opens by emission, carries exactly one frame in flight, is made reliable by a bare ACK,
retransmits on a fixed ~10 ms silence, reclaims itself silently after the ~1–5 s give-up, never reaps a
merely-dormant session, and holds no cross-epoch state — every one of these shown by test against a
virtual clock and a fake wire, with **zero hardware and zero syscalls in the crate's logic**. The heart
is built and proven so E3 (data path) and E4 (reactor) have a sound, audited protocol to wrap.

> "Multiplexing is instantiation of protocol and the driver layer is interleaving those multiplexed
> requests across its single wire single-threaded protocol." — the operator (`architecture.md` §4).

## Final State

- The crate compiles standalone (see R2 std/no_std), zero non-dev dependencies, and runs its entire
  self-validation in pure software (no device, no privileged calls).
- **Frame:** `[u64 session ID][payload]` encodes/decodes round-trip byte-identically; a bare ACK is the
  session ID with empty payload; data-vs-ACK is decided by the endpoint's **role** for that session.
- **Open-by-emission:** an unknown inbound ID instantiates a session and delivers its first payload up;
  no opened-event is emitted. A known ID routes with no protocol-type branch.
- **Stop-and-wait:** a sender holds ≤1 unacked frame; it does not advance until the ACK returns; the
  held buffer is released exactly on ACK.
- **Retransmit/give-up:** on no ACK by ~10 ms the frame re-emits; after the ~1–5 s give-up the sender
  frees its buffer, drops the session, and emits nothing — and this fires **only** with an outstanding
  unacked frame. A dormant session (nothing in flight) is never reaped.
- **No cross-epoch state:** the table holds only live sessions; a session ID reused in a new epoch is a
  new session by emission, with nothing to reconcile.
- **Allocation:** the opener hands out 64-bit monotonic IDs, unique within an epoch.
- **Interleaving surface:** the ready-set and next-deadline are exposed for the reactor; the protocol
  performs no scheduling/pacing/fairness itself.

## In Scope — the ordered slices

**Slice 1 — Crate skeleton + frame codec + the seams. [GREEN-LIT]** Greenfield crate (module layout by
concern). Define the seams as traits/params so the protocol runs with no syscalls: a **transmit sink**
("emit these bytes"), a **monotonic clock** ("now", advanceable in test), and an **inbound-frame entry**
("these owned bytes arrived"). **Pin the fourth boundary the dev flagged**: upward delivery of a received
payload (and the open-by-emission "delivers up") is either a **fourth seam** (an upward-delivery sink) or
the **return value** of the inbound-feed call — the dev pins which in the inventory, and it decides
whether the crate calls out or hands back. Encode/decode `[u64 session ID][payload]`, **big-endian**
session-ID prefix (R3, now nic-internal — proto holds no session id). Bare-ACK shape. **State**: the
ACK-bound retained buffer (held for retransmit) is **app-owned bytes, not a DMA buffer** — the DMA copy
lives behind the transmit sink in E3, so the sink does not leak E3 allocation into this crate. *Method:
round-trip + adversarial-bytes-never-panic self-tests; a fake wire records emitted frames, a virtual
clock starts the timing surface.*

**Slice 2 — Session table + open-by-emission + ID allocation. [GREEN-LIT]** ID→session map; unknown ID
instantiates + delivers up; known ID routes; no retired-ID memory (epoch-reuse = new session). The
opener's 64-bit monotonic allocator. *Method: tests for open-by-emission, route-to-existing, epoch-reuse;
**zero-branch dispatch is a structural review property (gate 6), not a runtime test** — label it so.*

**Carried external invariants** (this sprint cannot prove them — they live in E3/E4; track as
dependencies, do not assume silently): (i) **the reactor drains the RX ring before checking any
deadline** (E4 ordering) and (ii) **the link does not reorder** (E3/wire). §6's flag-free correctness
(no stale-ACK duplicate) rests on both; the Slice-4 harness *imposes* them, it does not prove them.

**Slice 3 — Sender role: stop-and-wait + retransmit + give-up. [GREEN-LIT].** Stop-and-wait, retransmit,
and the give-up → silent-reclaim path all build on the one-directional model. Reclaim is **timeout-only**
(R5): give-up reclaims a session only on an outstanding unacked frame past the bound; a session with
nothing in flight is never reclaimed. Per-session sender state: one frame in
flight, buffer held, ~10 ms deadline armed; ACK releases and advances; deadline re-emits; ~1–5 s give-up
silently reclaims (no event); fires only on an outstanding frame. Expose next-deadline. *Method: virtual
clock drives loss/retransmit, give-up→reclaim, and dormant-never-reaped; assert buffer lifetime and
"emits nothing on give-up".*

**Slice 4 — Receiver role + interleaving ready-set + full self-validation. [GREEN-LIT].** Receiver
delivers up and emits a bare ACK; the ready-set of send-pending sessions
drains trivially. Then the closed-loop proof: two endpoints (same code, opposite roles) over a fake wire
+ virtual clock — multi-session interleave, injected ACK loss, give-up reclaim, dormant≠dead, epoch
reset. The harness **processes inbound frames before evaluating retransmit deadlines** (replicating the
E4 RX-before-deadline ordering) and **imposes the no-reorder wire assumption** — both stated harness
properties, so the no-stale-ACK / no-spurious-retransmit case is tested under the production timing
model, not a different one. *Method: a deterministic in-memory harness wiring two protocol instances
through a controllable fake wire; every §6/§7/§8 invariant asserted.* (Reviewer [A]–[D] fold here on
unblock.)

## Non-Goals

- No device, no DMA, no epoll, no syscalls in the crate's logic — those are E1–E4 seams.
- No application-layer semantics; no reading of the binding; no L2 verb decode (that is `proto`).
- No reassembly / continuation handling; no alternating-bit dedup (reserved-bit slot documented only).
- No adaptive RTO, no RTT estimation, no backoff, no congestion/flow control, no QoS/pacing/fairness,
  no heartbeat/liveness probe — all explicitly shed (§6, §7).
- **Do not delete or stub a specified behavior because the test harness does not exercise it** — e.g.
  the give-up path must exist and be tested even though a real dead peer is rare; trace the spec and the
  self-validation before trimming.

## Starting Surface

Greenfield — audit first the **three seam definitions** and pin them before any session logic, because
they are the entire boundary between this sprint and E3/E4: what exactly the transmit sink, the clock,
and the inbound-frame entry promise, and that none of them leak a syscall into the crate's logic. Then
inventory the protocol surface the dev produces before editing:

```
behavior (§ref) -> seam(s) it touches (wire|clock|inbound|upward) -> role (sender|receiver|both) -> state it owns -> test that proves it
```

Cross-check the established sibling: `proto` (no_std + alloc, zero deps, module-by-service, self-validating
loop) is the convention pattern to match unless R2 says otherwise.

## Method

- **Build against the seams; prove in pure software.** A virtual clock (manually advanced) and a fake
  wire (records emitted frames, lets a test inject inbound frames / drop ACKs) drive every timing and
  reliability assertion — deterministic, no real time, no device.
- **Same code both ends.** The one protocol implementation plays sender or receiver per session by which
  end opened it; the closed-loop harness runs two instances of the *same* code against each other.
- **Invariant-level evidence, not "looks right":** each closed claim cites the §-reference it realizes,
  the test that exercises it, and the observed assertion (buffer freed exactly on ACK; nothing emitted on
  give-up; dormant session survives past the give-up window; reused ID is a fresh session).
- **Frame encoding is a stable contract** — once accepted, the byte layout of `[session ID][payload]`
  and the bare ACK are held fixed (E3/E4 and the peer depend on it); a change is a re-scope, not a tweak.
- **Totality over hostile input:** arbitrary inbound bytes yield a typed outcome or a clean drop, never a
  panic, never an over-read — the descriptor-supplied length is treated as adversary-influenced (bounded,
  never trusted), consistent with the driver-is-the-guard posture even though E3 owns the descriptor.

## Acceptance — the gates that close the sprint

Gates 1–3 close the green-lit **Slices 1–2**; gates 4–5 close the **held Slices 3–4** (released when §4
lands). Gate 6 (independent review) applies to whatever is being accepted at the boundary.

1. **[Slices 1–2]** The crate builds clean and the self-validation runs green in pure software (no
   device, no privileged calls). Evidence carries a **build-coherence marker** — the test-binary mtime
   post-dates the source, or a build signature — so green is provably from the *current* source, not a
   stale binary (the same discipline that pinned sprint 000's sha).
2. **[Slices 1–2] Frame codec:** the golden pins **exactly** the `[u64 session ID][payload]` layout —
   the 8-byte **big-endian** prefix bytes, **no length field, no flag byte**; a bare ACK is the session
   ID with empty payload = **exactly 8 bytes**, and an empty-payload data frame and a bare ACK are
   **byte-identical** (role/direction disambiguates, §6 — pin that equivalence). Arbitrary bytes never
   panic/over-read. **Do not** pin payload contents — that is proto's L2 contract, opaque here.
3. **[Slices 1–2] Session management:** open-by-emission instantiates + delivers up with no
   opened-event; known-ID routes with **zero protocol-type branch** (structural, re-derived by review,
   not a runtime test); epoch-reuse yields a new session; no retired-ID memory.
4. **[Slices 3–4, HELD] Stop-and-wait + reliability:** **≤1 unacked frame is structural** — a single
   in-flight slot (an `Option`, not a queue), re-derivable at the cited line; a second payload offered
   while one is in flight is **back-pressured** (no unbounded internal queue) and emits **no** second
   wire frame. Buffer lifetime is proven via an **observable** (Drop/refcount), with the negative cases:
   held across a retransmit (same buffer re-emitted, not freed+realloc), not freed on any
   non-ACK/non-give-up event, freed on ACK, freed on give-up. Retransmit fires **at** the configured
   interval and give-up **at** the configured bound (exact-value assertions on the virtual clock; the
   ~10 ms / ~1–5 s numbers are provisional, dialable per §7). Give-up performs **positive** reclaim —
   buffer **freed** *and* session **removed from the table** — *in addition to* wire-silence (silence
   alone is satisfied by an unimplemented give-up; it is necessary, not sufficient); fires only on an
   outstanding frame; a dormant session survives **past** the give-up bound (reclaim is timeout-only —
   R5 resolved; a session with nothing in flight is never reclaimed).
5. **[Slices 3–4, HELD] Interleaving:** the ready-set drains multiple sessions over the one wire with
   **no scheduler** (re-derived structurally — no priority/QoS/fairness at the dispatch site); the
   exposed next-deadline matches the earliest armed retransmit.
6. Independent review re-derives every gate from the current source at cited lines and a settled
   post-build run — not from the dev's summary — and confirms no seam leaks a syscall into the logic and
   no specified behavior (give-up, dormant survival) was trimmed because the harness made it look unused.

## Review Questions

- **R1 — directionality. [RESOLVED — arch §6 unidirectional, designer/PM 2026-06-25].** A session's
  data is **one-directional**; role/direction disambiguates data from ACK with no flag. The *who-opens*
  half: build **both role-halves**, role per-session by who opened, box-sole allocate per §1.1.
  proto's `model.md:353` "bidirectional" line is stale (proto-team cleanup), not the authority. Slices
  3–4 build on this.
- **R5 — reclaim semantics (§8). [RESOLVED — timeout-only, operator 2026-06-25].** Reclaim is **give-up
  timeout only**; there is **no consumer-driven clean-completion path**. A session reclaims solely on an
  outstanding unacked frame exceeding the give-up bound; one with nothing in flight is never reclaimed
  (lingers to epoch reset — accepted memory profile). The designer is updating arch §8 to match. The
  reclaim code is finalized to this ruling; no E5 holds remain.
- **R2 — crate identity & std posture. [RESOLVED — lead, matches proto].** Crate name `transport`
  (sibling to `proto`); `#![no_std]` + `alloc`, zero non-dev deps, `#![forbid(unsafe_code)]`, matching
  the proto convention — the syscall surface stays in the E3/E4 seams. Raise it if you want a different
  name/posture; otherwise this stands.
- **R3 — frame endianness. [RESOLVED — big-endian].** The 64-bit session-ID prefix is **big-endian**.
  Now purely nic-internal: proto excised `session_id` (sprint 010), so there is no cross-crate reader to
  reconcile; "same code both ends" keeps it self-consistent.
- **R4 — continuation flag / "no flag byte". [RESOLVED by proto-010, out of E5].** proto excised its L1
  envelope and `flags` field; the operator relocated the continuation indicator to **the transport's L1
  frame** (the reassembler now takes a `continuation` bool). The E5 frame stays `[u64 session ID]
  [payload]` with **no flag**; carrying a continuation indicator is a **frame-format change owned by the
  later reassembly sprint**, which is where arch §6's "no flag byte" gets reconciled. Out of E5 scope.

---

## Outcome & Postmortem

**Verdict: ACCEPTED, sprint closed.** Independently re-derived by `zero.nic-review` from the working-tree
source at cited lines + its own settled run across both binaries (not the dev's summary). All gates green.

**Delivered:** the `transport` crate (E5) — `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, zero
non-dev deps, edition 2024 — L1 frame codec + three seams; session table + open-by-emission + 64-bit
allocator; sender role (stop-and-wait, retransmit, timeout-only give-up); receiver role + ready-set;
two-instance closed-loop self-validation. **33 tests** (25 unit + 8 closed-loop), four slices.

**Gates:** (1) clean build + green pure-software, build-coherence pinned, no-syscall **compile-enforced**
(`x86_64-unknown-none`); (2) frame golden byte-for-byte (`[u64 BE id][payload]`, no length, no flag; bare
ACK ≡ empty-data at exactly 8 bytes); (3) open-by-emission with no opened-event, zero protocol-type
branch, no cross-epoch state; (4) structural ≤1, back-pressure, Rc/Weak buffer lifetime with negatives,
retransmit-at-interval, **timeout-only give-up** with positive reclaim (freed + removed + silent);
(5) ready-set drains, no scheduler, next-deadline = earliest armed; (6) anti-trim closed.

**Cross-component reconciliation (the defining event):** reading this sprint's authority against the
*built* `proto` crate (not its docs) surfaced five contradictions on the shared L1 frame and session
model. Escalated to the operator with two independent passes. Resolution: **proto sprint 010** excised
its L1 envelope (nic owns L1); **arch §6 unidirectional** governs (proto's stale `model.md:353` is
proto-team cleanup); **reclaim is timeout-only** (R5). The sprint ran as a split — Slices 1–2 green while
the directionality fork resolved, then Slices 3–4.

**E1–E4 inherit:** the three seams as the integration contract (TransmitSink, Clock, inbound-frame entry;
upward delivery is `on_inbound`'s return value, not a 4th seam); the **two carried external invariants**
the transport cannot prove — (i) the reactor drains RX before checking any deadline, (ii) the link does
not reorder — on which §6's flag-free no-stale-ACK correctness rests; the copy boundary (ACK-bound buffer
is app-owned; DMA copy lives behind the sink in E3); the **frozen wire contract** (`[u64 BE id][payload]`,
a change is a re-scope); timeout-only lifecycle with `next_deadline` exposed for E4's `epoll_wait`;
provisional 10 ms / 2 s timing, dialable and bench-gated (§7) — confirm under load on silicon.

### Retrospective — nic-lead (zero.nic-lead)
- **Approach:** Framed E5 as pure-software logic against three seams; formed the sprint as a team
  (dev + review critiqued the draft before promotion); routed one slice at a time; arbitrated every
  report against the gates; deferred the cross-component architecture conflict to the operator rather
  than ruling it myself.
- **Evidence that changed the plan:** the dev's read against proto-*as-built* turned my R4 (a tidy
  layering question) into a five-way foundational conflict needing an operator ruling. I held the sprint,
  hardened the premise with two independent passes before escalating, and the reconciliation let me run
  the sprint as a **split** — Slices 1–2 proceeding while the directionality fork resolved — so the team
  never idled on a blocked fork.
- **Held the line on:** not building the role model against an *unlanded* ruling (the reviewer's catch
  that "ratified unidirectional" lived in a memory, not committed authority — we waited for arch §6 to
  govern); getting the smallest independent grounding before escalating; and keeping the contract canon
  at the green-light boundary, not letting it drift into a devlog.
- **Next sprint (E1–E4) inherits (process):** validate against *built reality*, not just the spec —
  the contradiction was docs-vs-shipped-code, invisible until someone read the Rust; surface
  cross-component forks to the operator early with an independently-confirmed premise; and the
  formulate → team-review → green-light → one-owner-per-slice → independent-acceptance → retro → scoped
  commit rhythm held up under a mid-sprint architecture reconciliation.

### Retrospective — nic-dev (zero.nic-dev)
*"Validate against the BUILT crate, not just the spec — and hand the next layer the invariants you could
not prove."*
- **Built:** the transport crate (E5) across four slices — L1 codec + 3 seams; table + open-by-emission +
  64-bit allocator; sender (stop-and-wait, retransmit, timeout-only give-up); receiver + ready-set;
  two-instance closed-loop. no_std + alloc, forbid(unsafe), zero non-dev deps, 33 tests.
- **Evidence that shaped the plan — headline:** the L1/L2 boundary find. Reading this sprint's authority
  against proto **as built** (not its doc) surfaced five contradictions on the shared L1 frame
  (id width/flag/owner) + session model (uni vs bidirectional, close). proto had a resolved-but-
  spec-crossing L1 envelope, shipped 5 sprints, invisible in the docs — only the Rust showed it. Held the
  sprint → operator → proto-010 excised its envelope (nic owns L1); the sprint became a split. Also:
  compile-enforced no_std (`x86_64-unknown-none`) turned "no syscalls in crate logic" from an assertion
  into a lookup; Rc-refcount made buffer lifetime checkable; the borrow checker confirmed zero-copy.
- **Surprised:** the contradiction was docs-vs-**shipped-code**, not docs-vs-docs — team consensus inside
  proto had ratified an envelope its own spec forbade; reading the artifact caught it. And one stale line
  (`model.md` "both directions on one session") was load-bearing for the whole flag-free ACK mechanism.
- **E1–E4 inherit:** the three seams as the integration contract; the two carried external invariants
  (drain-RX-before-deadline E4; no-reorder E3) — highest-value carry-forward; the copy boundary (retained
  buffer is app-owned; DMA copy behind the TransmitSink in E3); the frozen wire contract
  (`[u64 BE id][payload]`, no length, no flag; bare ACK ≡ empty data); timeout-only lifecycle, no close
  signal, no cross-epoch state, `next_deadline` for E4's `epoll_wait`; provisional 10 ms / 2 s timing,
  dialable + bench-gated (§7) — confirm under load on silicon.

### Retrospective — nic-review (zero.nic-review)
- **Acceptance approach:** Re-derive every gate from the actual source at cited lines + my own settled
  post-build run — never the dev's summary. Build-coherence proven each time (test-binary mtime/sha256 >
  newest source; re-run the exact binary), and the no-syscall property taken as **compile-enforced**
  (`x86_64-unknown-none`, std cannot link) rather than asserted.
- **Evidence that changed a verdict (headlines):** (1) Sprint 0 — caught that the `get_regs` I audited
  was kernel 7.1.0 while the driver that *ran* was 6.12.74, downgrading a clean ACCEPT to
  ACCEPT-with-flagged-residual until the 6.12.74 source was version-pinned (`uname -v` == src-pkg) and
  re-derived. (2) E5 plan — caught the give-up gate was **circular**: "emits nothing" is satisfied by an
  unimplemented give-up, so it could not tell correct-reclaim from not-implemented; required positive
  reclaim observables (buffer freed + session removed), which the accepted code then asserted.
- **Held the line on:** #4 directionality stays OPEN on *committed* authority (`model.md:353`
  bidirectional) — a brief/memory does not override committed source; it cleared only when committed
  arch §6 was ruled to govern. And R5: the give-up timeout is the *only* reclaim — verified structurally
  (no consumer-driven close/complete/teardown in the public API), not by test alone.
- **Next sprint (E1–E4) inherits:** (i) any kernel/driver-version property must be re-derived against the
  driver that *actually runs*, version pinned — a reference tree is not the running binary; (ii) the
  E4 harness must **replicate** the production RX-before-deadline ordering (and no-reorder) — the
  alternating-bit omission is only honestly proven under it; (iii) carry build-coherence +
  compile-enforced no-syscall as standing acceptance gates; (iv) byte-for-byte goldens stay non-circular
  (hardcoded bytes, not value-derived).
