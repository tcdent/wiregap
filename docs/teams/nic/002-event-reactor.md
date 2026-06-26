# Sprint 002 — The Event Reactor (E4)

> The `nic` team's second build sprint (epic **E4**): the single-threaded heartbeat that drives the
> transport. One `epoll` over the NIC idle-wakeup eventfd, the earliest retransmit deadline, and the
> application's source fds; **poll-then-block** (busy-poll the RX ring while frames are present, block
> on the eventfd when it is empty); the retransmit deadline composed into the same `epoll_wait`; single
> producer into the one TX ring, no locks. It wraps the **Sprint-1 `transport` crate** (its `next_deadline`
> and ready-set are the integration surface) and stands on **seams** to E2 (the MSI-X→eventfd wiring) and
> E3 (the RX-drain / TX-write / copy boundary) that arrive with the prototype cards — so the reactor's
> own logic is provable **on any Linux, no NIC**, with real `epoll`/`eventfd`, fake RX/TX, and real pipes
> as application fds. This document is the contract: the single source of truth for what this sprint owns,
> what it does not, and what evidence closes it.
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`).
> Authority: [`docs/nic/architecture.md`](../../nic/architecture.md) §3 (incl. §3.1 copy boundary), §5;
> [`docs/nic/grounding/data-path-and-events.md`](../../nic/grounding/data-path-and-events.md) §4; epic E4
> in [`docs/nic/epics.md`](../../nic/epics.md). Builds on Sprint 1 (`transport`, committed `53bc323`).

---

## Scope Boundary

**This sprint owns** the reactor — the flat, single-threaded event loop (§3):

1. The **`epoll` multiplexer**: one `epoll` instance watching the NIC idle-wakeup **eventfd**, the
   earliest **retransmit deadline** (composed as the `epoll_wait` timeout), and the **application source
   fds**. Every readiness is an event (§3).
2. **Poll-then-block** (the inversion of ixy, `data-path-and-events.md` §4): busy-poll the RX-drain seam
   while frames are present; block on `epoll_wait(eventfd, deadline)` only when the ring is empty. **No
   adaptive mode-switching**, no rate estimator.
3. **Deadline composition**: read `transport::next_deadline()` and compose it into the `epoll_wait`
   timeout — "block on the eventfd *or* the retransmit deadline, one loop, two exit reasons." On a
   deadline wake, drive the transport's retransmit/give-up `poll`.
4. **Driving the transport — the synchronous-sink model** (verified against the committed Sprint-1 API,
   `53bc323`). The transport **emits synchronously** through a `TransmitSink` passed *into* every call
   (`open`/`send`/`on_inbound`/`poll`, each `<S: TransmitSink>` taking `sink: &mut S`); `ready_set()` is
   a **read-only view** of in-flight sessions (`Iterator<Item = SessionId>`), **not** a queue to drain.
   So the reactor holds the **one** `TransmitSink` (the E3 TX-write) and threads it (`&mut`) through
   every transport call; emission happens *inside* those calls. **"Sole TX writer" = sole holder of that
   sink** (structural, compile-enforced — §D below), not a ready-set drain. `ready_set` is used for
   observability and the app-fd **back-pressure** decision (item 7). Each drained RX frame →
   `on_inbound(&frame, &mut sink)` → handle the returned `Delivery` upward (a bare ACK returns `None`).
   (Per the committed API, `on_inbound` takes no `now` — only `open`/`send`/`poll` do.)
5. On an eventfd wake, **`read()` the eventfd** to clear the latch (`data-path-and-events.md` §4).
6. The **thin syscall wrappers** the loop needs — `epoll_create1`/`epoll_ctl`/`epoll_wait`, `eventfd`,
   `read`, and **`clock_gettime(CLOCK_MONOTONIC)`** (the reactor produces `now: Instant` per iteration to
   pass into every transport call and to compute the deadline) — `unsafe` confined to that FFI module and
   nothing else (§E). The reactor implements its own injectable **clock seam** over `clock_gettime` so
   all timing logic is pure/deterministic in test (inject `now`).
7. **App-fd back-pressure (the §3 no-staging invariant at the E4↔E5 seam).** `send` returns `WouldBlock`
   when a session has a frame in flight. When an app fd mapped to such a session is readable, the reactor
   **pauses** that fd (epoll de-register / `EPOLLONESHOT`) — it does **not** keep reading and buffering
   (that is the intermediary staging buffer §3 rejects) — and **re-arms** it when the ACK clears the
   session.

**This sprint does NOT own** (named, with where it belongs):

- **The MSI-X → eventfd arming** (`VFIO_DEVICE_SET_IRQS`, `IVAR`/`GPIE`/`EIMS`) — that is **E2** (VFIO).
  Here the wakeup eventfd is a **seam**: the reactor owns the epoll-over-eventfd loop; in test a real
  `eventfd` is written to simulate "the ring went non-empty."
- **The RX/TX rings, descriptors, and the copy boundary** — that is **E3** (data path). The RX-drain
  seam yields **already-copied, app-owned** frames (E3 performs the §3.1 copy-out at the edge); the
  TX-write seam consumes app-owned bytes (E3 copies into the DMA buffer). The reactor never touches
  device memory. See R3.
- **Device bring-up / link (E1) and VFIO/IOMMU setup (E2)** — no device in this sprint.
- **The session protocol** — owned by `transport` (E5, Sprint 1). The reactor *drives* it through its
  public surface (`on_inbound`, `next_deadline`, ready-set, `poll`); it does not reimplement protocol logic.
- **The application layer** — what an app fd's bytes *mean* (PTY/file/request) and the binding live above
  the reactor; the reactor multiplexes fds and routes bytes, it does not interpret them.

## Problem

The `transport` crate proves the session protocol in pure logic against three seams (transmit sink,
clock, inbound-frame entry), but **nothing drives it in a real event loop**. There is no `epoll` heartbeat
turning "a frame arrived" or "a retransmit deadline passed" into calls on the transport, no composition of
its `next_deadline` into a real `epoll_wait`, nothing holding its `TransmitSink` and driving emission as
the single producer, and no multiplexing of the application's own fds onto the same loop. Until the reactor exists, the transport
is a library no runtime calls; E1–E3 (the device, DMA, rings) have nothing to wake.

## Goal

A single-threaded reactor that drives the `transport` crate from real readiness events — one `epoll`
over the NIC eventfd, the retransmit deadline, and the app fds; poll-then-block; sole TX writer, no locks
— proven to run the full heartbeat **on any Linux with no NIC**, using real `epoll`/`eventfd`, a fake
RX/TX standing in for E3, and real pipes as app fds. The loop is built and proven so that when E2/E3 land,
wiring the real MSI-X eventfd and the real rings into the seams lights up the path with no reactor change.

> "One thread owns the wire, both rings, all session state, and one `epoll`." — `architecture.md` §3.

## Final State

- A reactor that compiles (see R1/R2) and runs its self-validation on any Linux, no NIC, no privilege
  beyond `epoll`/`eventfd`/pipes.
- **Poll-then-block** proven: drains the RX seam fully while frames are present; blocks on `epoll_wait`
  only when empty; takes zero "interrupts" (eventfd reads) during a continuous frame stream.
- **Deadline composition** proven: the `epoll_wait` timeout equals the transport's earliest
  `next_deadline` (relative to now); a deadline wake drives the transport `poll` (retransmit/give-up).
- **Single producer**: the reactor is the only holder of the `TransmitSink` (compile-enforced via
  `&mut`/move); structurally lock-free (one thread owns rings and session state, no `spawn`/`Arc`/`Mutex`).
- **Multiplexing**: one `epoll` services the NIC eventfd, the deadline, and app fds together; app-fd
  readiness drives session sends.
- **Latch discipline**: every eventfd wake is followed by a `read()` that clears it; no busy-spin on a
  stale latch.
- `unsafe` confined to the syscall-FFI wrappers; the loop logic above them is safe.

## In Scope — the ordered slices

**Slice 1 — Crate skeleton + the syscall wrappers + the seam contracts.** The reactor crate (std;
`#![deny(unsafe_code)]` crate-root + a single `sys`/`ffi` module that locally `#[allow(unsafe_code)]`,
so confinement is **compile-enforced** — §E; `deny` not `forbid` — `forbid` is non-overridable (E0453)
and cannot host the one needed exception), depending on `transport`. Thin **sound** safe wrappers over
`epoll_create1`/`epoll_ctl`/`epoll_wait`, `eventfd`, `read`, `clock_gettime(CLOCK_MONOTONIC)` — each
upholds the syscall's preconditions (valid fd, correctly-sized events buffer) with a SAFETY comment;
lean on the `libc` crate for **declarations only** (vetted) rather than hand-rolling syscall numbers /
`epoll_event` layout. **Write the seam contracts** (the map before the carve): the **wakeup eventfd**
(E2) fires on the *ring-non-empty edge* (§5); the **RX-drain** (E3) yields **all currently-available
OWNED frames then empty**, return type **owned** (`Vec`/`Box`, never `&[u8]` into a ring — so
device-writable memory structurally cannot flow past the seam); the **TX-write** (E3) **is** the
`transport::TransmitSink` taking app-owned bytes; the **clock** is the reactor's injectable seam over
`clock_gettime`. *Method: a real `eventfd` in a real `epoll`; assert wake-on-write and latch-clear-on-read.*

**Slice 2 — The poll-then-block core (sink wired here).** Busy-poll the RX-drain while frames are present
→ `on_inbound(&frame, &mut sink)` (which **synchronously emits** any bare ACK through the sink — so the
TX sink is wired from this slice, not first in Slice 3) → deliver the `Delivery`; block on `epoll_wait`
only when the drain is empty. The ixy inversion (poll-first, block-when-empty), no adaptive switching.
*Method: a fake RX feeding N frames then empty; assert full drain before any block, a continuous stream
takes zero eventfd reads; a simulated eventfd write wakes a blocked loop; **and the lost-wakeup test** —
write the fake eventfd in the window *between* drain-empty and the block, assert `epoll_wait` returns
immediately and the loop re-drains, no frame missed.*

**Slice 3 — Deadline composition + single-producer drive.** Produce `now` via the clock seam; compute
the `epoll_wait` timeout from `transport::next_deadline()` and `now` via a **pure `compute_timeout`**:
`None` → block indefinitely (`-1`); deadline `<= now` → `0`; future → `ceil(ns→ms)` (round **up**, a
pinned rule). On a deadline wake, drive `transport::poll(now, &mut sink)` (retransmit/give-up — emission
is synchronous through the held sink). *Method: test `compute_timeout` deterministically across all three
edges (None / past / sub-ms); assert the retransmit fires through the deadline wake; assert the reactor is
the sole sink holder (§D).*

**Slice 4 — App-fd multiplexing + back-pressure + full integration.** Register app source fds (real
pipes) on the same `epoll`; a readable app fd drives `send(.., now, &mut sink)`. **Back-pressure policy:**
on `WouldBlock` (session in flight), **pause** that fd (de-register / `EPOLLONESHOT`) — do **not** read
past one frame or buffer — and re-arm it when the ACK clears the session. Then the closed heartbeat: real
`epoll`/`eventfd` + the real `transport` + fake RX/TX + real pipes — a frame bubbles to its session; a
readable app fd goes out as a session send; a deadline fires a retransmit; all interleaved on one
single-threaded loop, no locks. *Method: a deterministic harness driving readiness in a fixed order;
assert the §3 loop end-to-end, including app-fd-readable-while-in-flight does NOT read past one frame and
resumes on ACK.*

## Non-Goals

- No device, no VFIO, no MSI-X arming, no DMA rings, no copy-boundary implementation — all E2/E3 seams.
- No multi-threading, no thread pool, no work queue, no intermediary staging buffer (the §3-rejected
  "rings → staging → worker-pool → pacing" pattern), no locks.
- No adaptive interrupt/poll mode-switching, no rate estimator (ixy's `interrupts.c` is **not** ported).
- No async runtime / executor (`tokio`/`mio`/`async-std`) — the loop is one explicit `epoll`; bringing a
  framework would violate minimal-trusted-surface and reviewable-in-one-sitting.
- No protocol logic — that is `transport`; the reactor only calls its public surface.
- **Do not delete or stub a §3/§4 behavior because the no-NIC harness does not exercise it** — e.g. the
  eventfd latch-clear and the poll-then-block inversion must exist and be tested even though a fake RX
  makes "blocking" look optional; trace the authority before trimming.

## Starting Surface

Audit first the **seam boundary** and the **transport integration surface**, because they are the entire
edge between this sprint and E2/E3/E5:
- the E2 seam (wakeup eventfd), the E3 seams (RX-drain yielding app-owned frames; TX-write taking
  app-owned bytes) — and that none of them smuggle device/DMA ownership into the reactor;
- the `transport` public surface this loop drives: `on_inbound`, `next_deadline`, the ready-set, `poll`.

Inventory shape the dev produces before editing:

```
loop step (§ref) -> readiness source (NIC eventfd | deadline | app fd | RX-drain) -> transport call -> effect (deliver up | TX write | poll) -> test that proves it
```

Cross-check the sibling: `transport` (Sprint 1, no_std) — the reactor is its std driver; keep the
`unsafe`-confined / minimal-deps / self-validating-loop conventions.

## Method

- **Real primitives, no NIC.** Build on real `epoll`/`eventfd` and real pipes; fake only the E2/E3 seams
  (a writable eventfd for "ring non-empty"; an in-memory RX-drain and TX-capture). The reactor logic runs
  unchanged when the real seams land.
- **Confine the impure boundary, test the logic deterministically.** Separate the pure decisions
  (timeout-ms from `next_deadline`+now; poll-vs-block; which session a ready fd maps to) from the
  `epoll_wait`/`read` syscalls, so timing-sensitive behavior is asserted on the pure logic, not on
  wall-clock `epoll_wait` races. Syscall integration is exercised with real fds and short waits.
- **`unsafe` is the review target**: confined to the syscall-FFI wrappers, small and marked; the loop
  above is safe Rust.
- **Single-producer is structural**: one thread, one owner of rings/session state — re-derivable, not a
  lock that "happens not to contend."
- **Evidence is concrete**: cite the loop step, the readiness source, the transport call, and the observed
  effect (frame delivered to the right session; zero eventfd reads during a stream; timeout == earliest
  deadline; exactly one TX writer) — never "looks right."

## Acceptance — the gates that close the sprint

1. The reactor builds clean and its self-validation runs green on a plain Linux host (no NIC, no
   privilege beyond epoll/eventfd/pipes); evidence carries a build-coherence marker (test-binary
   post-dates source / a build signature), so green is from the current source.
2. **Poll-then-block (positive observables, both brackets):** the RX-drain is fully emptied while frames
   are present **before** any `epoll_wait` block; a **continuous stream incurs zero eventfd reads** (the
   loop never blocks); and the block path is positively proven — a write to the fake wakeup eventfd wakes
   a loop that **actually blocked** (made no progress until the write), after which the eventfd counter is
   **0** *and* a subsequent `epoll_wait` with no new write does **not** immediately return (latch truly
   cleared, no busy-spin on a stale latch). **Lost-wakeup:** an eventfd write *between* drain-empty and the
   block returns immediately and re-drains — no frame missed.
3. **Deadline composition (pure, all edges):** `compute_timeout(next_deadline, now)` is a pure function,
   tested deterministically: `None` → `-1` (block indefinitely — a mis-map to `0` busy-spins); `<= now` →
   `0`; future → `ceil(ns→ms)` round-up (pinned). A deadline wake drives `transport::poll(now, &mut sink)`
   and the retransmit emits synchronously through the held sink.
4. **Single producer (compile-enforced) / multiplexing / back-pressure:** the reactor holds the one
   `TransmitSink` by `&mut`/move so the type system excludes a second writer; **no `thread::spawn`, no
   `Arc`/`Mutex`/`RwLock`** over rings or session state (grep → zero). One `epoll` services the NIC
   eventfd + deadline + app fds together; app-fd readiness drives a session send; and **on `WouldBlock`
   the reactor pauses that fd and reads no further / buffers nothing, re-arming on ACK** (the §3
   no-staging invariant) — all shown in the closed heartbeat with the three readiness kinds interleaved.
5. **`unsafe` confinement (compile-enforced) + soundness:** `#![deny(unsafe_code)]` at the crate root
   with a single `sys`/`ffi` module locally allowing it (`deny`, not `forbid` — `forbid` is
   non-overridable, E0453, and cannot host the one needed exception; re-derived from the module attribute, not a
   grep); and each FFI wrapper's **safe API is sound** — upholds the syscall preconditions (valid fd,
   correctly-sized events buffer) with a SAFETY comment per `unsafe` block, so misuse from the safe loop
   cannot cause UB.
6. **Pure/impure split + no dead-path:** the timing/decision logic (`compute_timeout`, poll-vs-block,
   fd→session) is **pure functions** tested deterministically; the syscalls are a thin impure shell. The
   fake E2/E3 seams are **injected trait impls**, never `#[cfg(test)]` branches in the loop body — review
   re-derives that the audited loop is identical real-vs-fake (`cfg(test)` only in `mod tests`).
7. Independent review re-derives every gate from current source at cited lines and a settled post-build
   run — not from the dev's summary — and confirms no seam smuggles device/DMA ownership into the reactor
   (the RX-drain return type is owned), no §3/§4 behavior was trimmed because the no-NIC harness made it
   look unused, and no async framework or adaptive-mode policy crept in.

## Review Questions

- **R1 — crate vs module. [RESOLVED — separate `reactor` crate; dev + review agree].** A separate crate
  depending on `transport` keeps `transport`'s compile-enforced `no_std`/no-syscall property intact (a
  std-gated module would erode it). Raise it if you want otherwise.
- **R2 — syscall dependency & `unsafe` posture. [RESOLVED — lead, dev + review agree].** `#![deny(
  unsafe_code)]` crate-root + one `sys`/`ffi` module locally allowing it (`deny`, not `forbid` —
  compile-confirmed: `forbid` is non-overridable, E0453, so it cannot host the confined exception); **the
  `libc` crate for declarations only** (vetted) rather than hand-rolling syscall numbers / `epoll_event`
  layout (which is *more* unsafe surface for no gain); **no** async/event-loop crate. Confined `unsafe`
  is unavoidable in E4 (E5/`transport` could use `forbid` only because it has *zero* unsafe; the syscall
  FFI is E4's review target). Object if you want a different
  dependency posture.
- **R3 (PM) — the E3/E4 copy-boundary split.** §3.1 says "the reactor copies each frame out of the DMA
  region," but epic **E3** owns "the copy boundary." Lean/resolution: the **copy is E3's**, at the RX
  edge inside the RX-drain seam, which hands the reactor **already-copied, app-owned** frames; §3.1 names
  the *locus* (the reactor's RX edge), E3 is the *implementer*. The E4-checkable property is that the
  RX-drain return type is **owned** (`Vec`/`Box`), so device memory structurally cannot flow past the
  seam — E4 proves it *consumes* owned bytes, not that E3 copied correctly (containment is conditional on
  the seam contract). Confirm the split so E3 and E4 agree on who copies.
- **R4 (reviewer/PM) — no-NIC acceptance basis. [reviewer-affirmed sound, under two conditions now gated].**
  Sound *because the fake E2/E3 seams are trait impls at the same boundary the real ones implement* —
  provided (a) **no `cfg(test)` forks in the loop** (gate 6) and (b) the **lost-wakeup race is modeled**
  (gate 2). The real MSI-X arming (E2) and ring DMA (E3) stay seamed/bench-gated; "lights up with no
  reactor change" holds only because the **seam contracts** (Slice 1) give E2/E3 a spec to meet. Confirm
  this is the accepted E4 close basis, hardware wiring carried as a tracked E2/E3 dependency.
- **R5 (PM → E3) — the TX-side copy twin & mempool sizing.** Sprint 1 made the ACK-bound retained buffer
  **app-owned** (`Rc` in `transport`), with the DMA copy behind the sink **per emit** — so E3's DMA TX
  buffer is **TX-completion-bound** (a ring-depth pool), while ACK-hold sizing applies to the transport's
  app-buffer pool. But `data-path-and-events.md` §3 sizes the *mempool* by ACK-hold (it assumed the DMA
  buffer is ACK-bound). This is **not E4-blocking** (seamed), but it is a §3 doc reconciliation for **E3**.
  The alternative — holding the DMA buffer ACK-bound — would reopen Sprint 1; flagging, not recommending.
- **R6 (PM) — synchronous-sink adaptation (verified, FYI/confirm).** Verified against the committed
  Sprint-1 API: the transport emits **synchronously** via an injected `TransmitSink`; `ready_set()` is a
  read-only view. E4 therefore adapts to that model (the reactor is the sole sink holder) with **no
  transport change**. A deferred-drain model would reopen Sprint 1 — not recommended. Confirm we adapt
  rather than reopen.

---

## Outcome & Postmortem

**Verdict: ACCEPTED, sprint closed.** Independently re-derived by `zero.nic-review` from the working-tree
source at cited lines + its own settled run (debug *and* release where a `debug_assert` is load-bearing),
coherence-pinned each time. All gates green; the one hygiene finding (`block()` going production-dead) was
closed by construction before close.

**Delivered:** the `reactor` crate (E4) — std, `#![deny(unsafe_code)]` crate-root + one `sys` FFI module
(`#[allow]`), `libc` declarations only, depending on `transport`. One `epoll` over the wakeup eventfd +
retransmit deadline + app fds; **poll-then-block** (the ixy inversion); pure `compute_timeout` deadline
composition; single-producer drive (sole `TransmitSink` holder); app-fd mux with `EPOLLONESHOT`
back-pressure. **19 tests**, runs the full heartbeat on any Linux, no NIC.

**Gates:** (1) clean build + coherence marker; (2) poll-then-block — full-drain-before-block, zero
eventfd reads on a stream, positive block proof, true latch-clear, lost-wakeup race; (3) `compute_timeout`
pure across all edges (None→`-1`, past→`0`, future→ceil-up, `i32::MAX` clamp), deadline-wake drives
`poll`; (4) sole sink holder (compile-enforced), token dispatch, app-fd back-pressure buffering nothing
(unread frame waits in the kernel pipe — the §3 no-staging invariant), closed heartbeat; (5) `deny`+`allow`
confinement, compile-enforced *and* sound (`event_token`'s packed `read_unaligned`; `epoll_wait` clamps
`maxevents`); (6) no `cfg(test)` in the loop — the audited loop *is* the production loop; (7) owned RX-drain.

**Integration corrections (validate against the BUILT crate, not the doc):** the draft's "drain the
ready-set to TX" did not match the committed transport, which emits **synchronously** via an injected
`TransmitSink` (`ready_set` is a read-only view) — reframed to the sole-sink-holder model, no transport
change (R6). Also corrected mid-sprint: `forbid`→`deny` (a `forbid` root cannot host a module `allow` —
E0453); `on_inbound` takes no `now`; and `run_once` now delegates the eventfd-wake to `block()` so the
gate-2 tests cover the live path.

**Next sprints inherit (E1/E2/E3 implement the seams this reactor was built against):**
- **E2 (VFIO/IOMMU):** the wakeup-eventfd seam — arm MSI-X→eventfd (`VFIO_DEVICE_SET_IRQS`), ring-non-empty
  edge; the reactor epolls + read-clears it, real eventfd drops in with no reactor change. Re-verify the
  latch / lost-wakeup against the **real, kernel-written** VFIO eventfd (not the test writer).
- **E3 (DMA rings / copy boundary):** the RX-drain seam (yield all-available **owned** frames; copy-out at
  the RX edge §3.1 — device memory must not flow past the seam; E4's containment is *conditional* on that
  copy, and the §3.1 crossing soundness stays open in `concerns-to-watch.md`) and the TX-write seam (IS
  `transport::TransmitSink`, app-owned bytes, copies into DMA per emit). **R5:** E3's TX mempool is
  ring-depth (TX-completion-bound), not ACK-hold — reconcile `data-path-and-events.md` §3.
- **E1 (bring-up/link):** the silicon under E2/E3; static anchors grounded in Sprint 0.
- **Carried external invariants:** **drain-RX-before-deadline is now realized by the reactor** (`run_once`
  drains fully before it blocks/evaluates the deadline) — E4 provides it by construction; **no-reorder**
  stays E1/E3/wire to honor. Clock: production `MonotonicClock` over `clock_gettime`; provisional
  ~10 ms / ~1–5 s timing stays bench-gated (§7) on real silicon.

### Retrospective — nic-lead (zero.nic-lead)
- **Approach:** framed E4 as the reactor driving the Sprint-1 `transport` against E2/E3 seams (provable
  no-NIC); formed the sprint as a team (dev + review critiqued before promotion); routed one slice at a
  time; arbitrated each report against the gates; deferred the copy-boundary / TX-twin design forks to the
  PM/E3 rather than ruling them.
- **Evidence that changed the plan:** the dev's read against the *committed* transport (not its doc)
  caught that my "drain the ready-set" model was wrong — the recurring "validate against built reality"
  lesson, now twice over. Both teammates independently converged on the app-fd back-pressure gap (the §3
  no-staging invariant at the E4↔E5 seam), which became a full gate. I also corrected two errors in my own
  contract mid-sprint (`forbid`→`deny`, `on_inbound`'s `now`) once the dev compile-verified them.
- **Held the line on:** closing the `block()`-production-dead divergence **by construction** (have
  `run_once` call `block()`) rather than accepting "equivalent for now" — the make-it-impossible ethos
  applied to the proof itself; and not reopening Sprint 1 (adapt E4 to the built sink model).
- **Next sprint inherits (process):** the seam contracts a sprint pins become the spec the later sprints
  must meet — write them before the carve; and a test must run the *live* path, not a dead helper.

### Retrospective — nic-dev (zero.nic-dev)
*"Validate against the committed crate, not the doc — and make the test run the live path."*
- **Built:** the reactor crate (E4) — one epoll over the eventfd + retransmit deadline + app fds;
  poll-then-block (ixy inversion); pure `compute_timeout` deadline composition; single-producer drive
  (sole `TransmitSink` holder); app-fd mux with `EPOLLONESHOT` back-pressure. Against E2/E3 seams + an
  injectable clock; runs on any Linux, no NIC. std, `deny(unsafe)` root + one `sys` FFI module, `libc`
  declarations only, 19 tests.
- **Evidence — headline:** the synchronous-sink integration catch. The draft "drain the ready-set to the
  TX-write seam" did not match the committed transport (it emits synchronously via the `TransmitSink`
  threaded into its calls; `ready_set` is a read-only view). Reading the built crate reshaped the TX model:
  hold one sink, thread it everywhere; sole-writer = sole holder. (Same lesson as Sprint 1's L1/L2.) Also:
  `forbid`→`deny` (E0453, compile-confirmed); `on_inbound` takes no `now`; and the reviewer's `block()`
  production-dead catch → fixed by `run_once` delegating to `block()`.
- **Surprised:** the "validate against the BUILT artifact" lesson recurred verbatim — design docs lag code,
  the committed crate is the contract. And a packed `epoll_event` field needs an unaligned read, forcing
  token dispatch's one unsafe into the `sys` module — reinforcing the confinement boundary.
- **Next sprints inherit:** E2 the wakeup-eventfd seam (MSI-X edge); E3 the owned-frame RX-drain + the
  TX-write `TransmitSink` (+ R5 TX mempool = ring-depth, reconcile §3); drain-RX-before-deadline now
  realized by the reactor; no-reorder still E1/E3/wire; the clock seam over `clock_gettime`.

### Retrospective — nic-review (zero.nic-review)
- **Acceptance approach:** re-derive every gate from working-tree source + my own settled run (debug *and*
  release where a `debug_assert` is load-bearing), coherence-pinned each time; confinement taken as
  compile-enforced (the clean build under `deny` *is* the proof), never a grep.
- **Evidence that changed a verdict (headline):** the loop-subsumption catch — `run_once` replaced
  `poll_then_block`, leaving `block()` production-dead, so the load-bearing positive-block / latch-clear /
  lost-wakeup tests rode a helper the live loop no longer called. I verified the behavior transferred *and*
  flagged the divergence risk; the dev closed it by construction. At plan time I added the app-fd-vs-
  stop-and-wait back-pressure gap (no internal queue; the unread frame waits in the kernel pipe — the §3
  no-staging invariant), which became a full gate.
- **Held the line on:** target the *live* path, not a dead helper — a green on `block()` proved nothing
  about `run_once` until they were one code path; and the corrected `deny`-not-`forbid` confinement
  (`forbid`+`allow` = E0453, I compiled the probe).
- **Next sprints (E1/E2/E3) inherit:** the seam contracts this sprint pinned are now the spec E2/E3 must
  meet — re-verify the latch / lost-wakeup against the **real** VFIO eventfd (kernel-written, not
  test-written), and E3's RX-drain must hand owned copied-out frames (E4's containment is conditional on
  that copy; the §3.1 crossing soundness is still open in `concerns-to-watch.md`). Carry `deny`+`allow` +
  per-`unsafe` SAFETY-soundness as the unsafe surface grows (MMIO/DMA/VFIO), and build-coherence +
  no-dead-helper as standing gates.
