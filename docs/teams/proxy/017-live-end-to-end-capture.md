# Sprint 017 — The Live End-to-End Capture (Sprint 16's Claim (ii))

> Proxy team, sprint 17. Discharges the one item Sprint 16 deliberately left
> optional and held: a genuine live round trip through the real, unmodified
> `serve` binary — real 82599 hardware, a real credential, a real upstream
> call to production Anthropic, a connected head, and a provisioned,
> writable quarantine — under the full Sprint-8/14 capture-model
> discipline. Drafted by zero.proxy-lead from planning input by
> zero.proxy-dev, following the PM's direction (2026-07-24).

---

## Scope Boundary

This sprint owns exactly one live capture and the two defects planning
found block it from ever running as a genuine two-process capture:

1. **Two real defects in `serve.rs`**, found while planning the capture,
   each a hard blocker rather than a design nicety:
   - **The peer box MAC is zeroed** (`PEER_BOX_MAC = [0u8; 6]`), a
     deliberate fail-loud placeholder. A real two-process capture needs the
     proxy's bench card addressed to the box's real bench card.
   - **The head socket has no unlink-before-bind.** `UnixListener::bind`
     with a stale socket file left by a prior run fails `EADDRINUSE` on
     every re-run — a capture sprint re-runs by nature (a failed attempt,
     a tweak, a reviewer request), so the first hiccup wedges the bench.
2. **A new bench harness, `s17_box`**, driving `Endpoint::Box` directly on
   the second bench card (`0000:63:00.0`) — a real, separate process, not a
   `two_facade` in-process role, and not a replica of any part of the
   composition root.
3. **The capture itself**: the real `serve` binary, unmodified beyond
   item 1's two fixes and two further diagnostic additions (below), run
   against a deliberately invalid credential (the operator's no-billing
   ruling, Final State #5), a real upstream connection, a scripted head
   client, and an operator-provisioned quarantine.
4. **Two additional `serve`/`upstream.rs` diagnostics, found at their
   respective design/implementation gates**: (a) `on_verdict`'s
   `Routed::NetPayloadAdmit(body)` arm prints `lifecycle::body_fingerprint`
   of the admitted body; `s17_box` prints the identical function's output
   for the body it sent — matching fingerprints, observed live, close the
   request-side tell. (b) `upstream_call`'s error site prints the real
   `ureq::Error`'s `Display` (e.g. `"http status: 401"`) before it is
   mapped away to the box-facing `ErrorCode` — found implementing the
   operator's no-billing ruling, since the box-facing code alone collapses
   every HTTP-status failure into the same generic value. Both are
   content-free with respect to anything secret, additive, and change no
   behavior or authority logic.

It explicitly does **not** own:

- **Any change to `bring_up`, `assemble`, or the authority ladder.** Those
  are Sprint 16's proven, accepted design; this sprint drives them, it does
  not touch them.
- **A `two_facade`-based harness.** Verified during planning:
  `bring_up`/`lifecycle::bring_up` calls `Subsystem::construct` *inside its
  own body* (`lifecycle.rs:374`), while `two_facade` hands a role closure
  an *already-constructed* `Subsystem`. Carrying the real composition path
  inside `two_facade` would require hand-rolling `bring_up`'s body outside
  it — a replica of the composition root, which is exactly what Sprint 16
  Slice 3 worked to eliminate. `two_facade` is also a single process (the
  privilege drop is process-wide, dropping the box thread too — a fiction
  production does not have), inverting the box/proxy trust separation this
  capture is supposed to exercise.
- **A compiled DSL entry for the peer MAC.** Surfaced during planning as a
  design question (below, resolved directly): the peer MAC is an
  addressing/wiring parameter like `PROXY_BDF` and `HEAD_SOCKET_PATH`
  (already bare `const`s in `serve.rs`, not part of the Sprint-16 authority
  register) — it gates nothing at the ladder and its correctness affects
  whether frames are received at all, not whether an untrusted input is
  admitted. It follows the existing bare-`const` precedent, not the
  register pattern.
- **Testing the Anthropic integration itself.** Sprint 11 closed that
  ground; this sprint's upstream call is a vehicle to prove the round
  trip, not a re-test of the client's correctness. No content assertions,
  no retries, no streaming-shape checks on the response.
- **A negative control or a re-litigation of Sprint 16's authority design.**
  This is a positive liveness/round-trip demonstration on top of already-
  accepted, already-gated logic.
- **Building or automating `/run/zerotrust-proxy`'s provisioning into the
  harness.** Creating and `chown`-ing it needs root and is a **deployment
  act**, done by the operator outside the harness — a harness that
  provisions its own quarantine could never catch a mis-provisioned one,
  masking the exact gap Sprint 16 documented and named as a residual.

## Problem

Sprint 16 closed with all four authority rungs real and `Readiness::Served`
reachable, hardware-free — a genuine, load-bearing claim, but deliberately
**not** a proof that the real binary serves a real round trip. Three things
stood between `Served` and a working proxy, all named at Sprint 16's close:
quarantine writability by the drop-target uid, the live hardware round
trip, and reload-without-restart (unrelated, out of scope here). The PM has
now directed the second one: run it.

Planning this capture (rather than assuming `serve.rs` was ready to drive
as-is) found it is **not**, for two structural reasons, both verified
directly against source:

- `serve.rs`'s `PEER_BOX_MAC` is a deliberate zeroed placeholder
  (`serve.rs:80`) — correct for a solo, receive-nothing-real run, but a
  real box on a real second card needs its real MAC configured, or the
  proxy's NIC will never recognize its frames.
- `serve.rs` binds its head-rendezvous socket
  (`UnixListener::bind(HEAD_SOCKET_PATH)`, `serve.rs:121`) with no
  unlink-before-bind — a residual named at Sprint 13's review and never
  closed. A capture sprint's normal operating mode (attempt, fail, adjust,
  re-attempt) will hit a stale socket file on the very first re-run.

Planning also found a load-bearing **ordering constraint**: `serve.rs`
blocks on `listener.accept()` **before** calling `bring_up` (i.e. before
the NIC is even constructed) — so the head must connect first, and a
harness or operator sequence that starts the box before the head connects
will simply hang.

## Goal

Prove, on real bench hardware, that the unmodified (beyond the two
blocking fixes) `serve` binary — driven by a real, separate box process on
the second bench card — completes one genuine round trip: a box-originated
request reaches the proxy, is forwarded verbatim to production Anthropic
under a real credential, the real response returns to the box, and the
box's resulting display frame reaches a connected head over the real
socket — with the quarantine write path also proven live, against a
provisioned-but-not-proxy-managed directory, closing the one authority-
adjacent gap Sprint 16 could not close by construction.

## Final State

1. **`PEER_BOX_MAC` is configured, not zeroed**, for this capture — a
   compiled constant (matching `PROXY_BDF`'s existing bare-`const`
   pattern, not the Sprint-16 authority register; see Scope Boundary). The
   real bench MACs (read from sysfs while ixgbe-bound): `41:00.0` (proxy) =
   `90:e2:ba:c7:69:a0`; `63:00.0` (box) = `90:e2:ba:d0:68:ac`. Each side's
   binary is configured with the *other's* MAC as its peer.
2. **`serve.rs` unlinks a stale socket before binding**: a best-effort
   `std::fs::remove_file(HEAD_SOCKET_PATH)` (ignoring `NotFound`)
   immediately before `UnixListener::bind`, so a repeated capture attempt
   never wedges on `EADDRINUSE`. Behavior-preserving for a clean first run;
   fixes exactly the stale-socket case.
3. **`serve.rs`'s `on_verdict` prints `lifecycle::body_fingerprint` of the
   admitted body on `Routed::NetPayloadAdmit`** — found during Slice 2's
   design review (reviewer's finding): without this, the request-side
   bind-the-chain tell has no observer at all (nothing else in scope
   records the body the proxy actually forwards), so a printed nonce
   alone was never checkable. A content-free diagnostic print in an
   already-present no-op closure — no behavior or authority-logic change.
   `s17_box` prints the identical function's output for the body it sent;
   the capture compares the two live.
   **Hash the raw JSON body on both sides, never the framed L2
   payload** — verified against source (a named pitfall, not a style
   note): `Routed::NetPayloadAdmit(request_body)` carries the same
   `request_body` handed to `recognize` and then `send`-`POST`-ed
   byte-verbatim (`dispatch.rs:742,745`; `upstream.rs:358`), so the
   proxy's fingerprint is over the raw JSON — but `s17_box` builds two
   distinct values, `anthropic_body(&nonce)` (the raw JSON) and
   `net_payload_l2(&anthropic_body(...))` (the L2-framed wire payload);
   fingerprinting the latter (the variable in hand at the send call) can
   never match and would fail the capture at the bench for a pure framing
   reason that reads like a bind-the-chain violation. `s17_box`
   fingerprints `anthropic_body`'s return value at the point it is
   constructed, not `payload` at the send site.
   **Not a cryptographic hash — a deliberate substitution, ratified by
   both dev and reviewer, not the `sha2` originally specified.** Found
   during implementation: `serve` is the *deployed production binary*, so
   gating a crypto crate behind either `serve` or `bench-capture` still
   lands it in the artifact the operator actually runs — neither of the
   two options this contract originally offered achieves the TCB-surface
   goal they were meant for. `lifecycle::body_fingerprint` is instead an
   8-line, dependency-free 64-bit FNV-1a plus the exact length (`<hash>/
   <len>`), living once in `lifecycle.rs` and called identically by both
   binaries (no separate implementations that could drift), unit-tested
   for determinism and sensitivity to truncation, reordering, and
   appended bytes. **Why this is sufficient, not a shortcut**: the value
   is printed for a reviewer to observe, never branched on (`on_verdict`
   returns `Continue` unconditionally regardless of the match) — a
   collision cannot be *exploited*, only mislead an observer, and
   misleading the observer would require already controlling the
   forwarding path under observation, at which point a matching value
   would simply be printed instead. What must be ruled out is *accidental*
   divergence between two cooperating in-house processes, not adversarial
   collision-crafting; FNV-1a's XOR/odd-prime-multiply structure makes
   any single-byte substitution change the output deterministically, and
   the exact length catches truncation/append outright. The deeper reason
   it suffices: the underlying guarantee (`recognized == reassembled ==
   forwarded`) is *structural* — Sprint 11 proved the same `Vec` moves
   from admit to POST with no re-derivation — so the fingerprint confirms
   that structural property held on a live run; it does not substitute
   for it, and no hash strength would rescue the claim if the structure
   were absent. **Named boundary, so this tool is not reused where it
   would be the wrong one**: if bind-the-chain ever became an *enforced*
   control (code refusing to forward on a mismatch), or if the compared
   values ever crossed a boundary where a party could craft a collision,
   a non-cryptographic fingerprint would need revisiting — this
   fingerprint is scoped to an observational, non-adversarial, in-house
   comparison, never cited as adversarial evidence.
4. **`s17_box`**, a new bench binary on `0000:63:00.0`: constructs
   `Endpoint::Box` directly (never `two_facade`), drives one net-request
   through the real reactor/transport (`open(net_address) -> sid`,
   `send(sid, net_payload)`, the ruled two-call one-session sequence) with
   a fresh nonce embedded in the body, and receives the correlated
   `net-error` — **the designed, expected outcome under the operator's
   no-billing ruling (Final State #5), not a failure mode** — recording
   its decoded code **legibly** in its own log, so the outcome is checkable
   rather than looking indistinguishable from the harness being stuck.
   On receiving it, `s17_box` emits one display frame toward the head
   carrying a marker derived from that observation (the box→head chain Q3
   established: the head only ever observes display frames, which are the
   box's to emit). **Also drives one `file-write` + `file-chunk`, carrying
   a second, distinct nonce, into the provisioned quarantine** (found
   during planning: Final State #9's quarantine-write-as-dropped-uid tell
   has no producer without this — only the box can emit a `file-chunk`,
   and nothing else in this sprint's scope does).
5. **No billed call — the operator's ruling (2026-07-24), superseding
   this item's earlier "minimal and cheap" framing**: an invalid, clearly
   bogus credential is used deliberately, chosen only to reliably
   authenticate-reject rather than to resemble a real key. The upstream
   call is a genuine, real TLS connection and HTTP POST to the real,
   hardcoded Anthropic endpoint — the request body is still the tier-1
   -admissible minimal shape (model `claude-haiku-4-5-20251001`,
   `max_tokens: 1`, one message), since that shape still has to clear the
   route/recognizer gate to be forwarded at all — but the tell this
   sprint needs is now **a real `401`, not a successful response**, and
   no metered usage is billed.
   **The exact credential value is pinned, not left to the bench**
   (reviewer's finding — three of the plausible readings sink the capture
   in ways that look like different, unrelated failures): it must be (a)
   **non-empty** — `Credential::from_env_value`/`validate_authority`
   treat an empty value as absent, so an empty credential fails the
   *authority rung* and `serve` exits before any call is ever made, which
   would read as a Sprint-16 regression, not a credential mistake; (b) **a
   valid HTTP header value** — visible ASCII only, no whitespace or
   control characters, since the value goes straight into
   `.header("x-api-key", key)` and anything the `http` crate rejects
   produces `ureq::Error::Http` (the request never leaves the machine),
   making the reachability tell vacuous while the log still looks like a
   network failure to anyone not reading closely; (c) **well-formed but
   wrong**, which is what actually produces the real `401`. Concretely:
   a non-empty, visible-ASCII, control-character-free, self-describing
   string that does not resemble a real key —
   `deliberately-invalid-capture-credential`, or equivalent. **This
   capture is only possible because of a Sprint-16 decision, which this
   is the first live exercise of** (reviewer's note): Sprint 16
   deliberately declined to validate the credential's *shape* at the
   ladder (its own Non-Goal states "the upstream call is the real proof
   of a good credential"), so an obviously-bogus value passes the
   authority rung precisely because of that choice. Worth recording both
   ways: a decision that paid off here, and a standing warning — if the
   ladder ever gains credential-shape validation, it would break this
   capture's method and should be weighed against that cost.
   **Open finding, routed to dev, not yet resolved**: the `401`-takes-the
   -`Err`-path mechanism currently depends on `ureq`'s *default*
   `http_status_as_error` setting — `upstream_call`'s agent builder never
   sets it explicitly, so the capture's whole tell structure implicitly
   rests on an unpinned third-party default that a future dependency
   bump could silently flip, with no signal. This is inconsistent with
   this sprint's own standing discipline of pinning things explicitly
   (exact BDFs, exact feature sets, the remap recipe). Dev to explicitly
   configure the expected behavior in the agent builder (or otherwise pin
   it) rather than leaving it implicit, before this item is considered
   closed.
6. **A real diagnostic gap, found implementing the ruling, closed by one
   small addition — not by trusting the box's error code alone**: the
   box only ever receives `NetError(Unavailable)`, and *every* HTTP-status
   failure collapses into that same generic code
   (`upstream.rs::map_ureq_error`'s catch-all — verified against source:
   `ureq::Error::StatusCode(u16)` carries the real status but has no
   explicit match arm, so it falls into `_ => ErrorCode::Unavailable`
   alongside TLS failures, unexpected i/o errors, and anything else not
   already named). So the box alone cannot distinguish "reached the real
   host, got a `401`" from a purely local failure — the operator's actual
   goal (proving the real path was reached) would be unproven by the
   box's observation alone. **Fix**: `upstream_call`'s error site prints
   the real `ureq::Error`'s `Display` (for `StatusCode`, exactly `"http
   status: 401"`, verified to carry nothing sensitive — no headers, no
   body, no credential) as a diagnostic, content-free with respect to
   anything secret, before it is mapped away to the box-facing
   `ErrorCode`. This is the same shape as Final State #3's fingerprint
   addition — additive, observational, no change to what the box
   receives or to `MB3`'s opaque-relay behavior — extending the "one
   further diagnostic addition" this sprint's Scope Boundary already
   permits to include this second one, found at the same gate for the
   same reason.
7. **The round trip is bound end to end, honestly narrowed for the
   no-billing plan**: request-side, **`body_fingerprint(admitted) ==
   body_fingerprint(sent)`, observed live from both ends** — sharpened
   during Slice 2's design review (reviewer's finding): the box's printed
   nonce alone has no observer (nothing in this sprint's scope records
   the body the proxy actually forwards), so a substring match was never
   actually checkable. The fingerprint (Final State #3's diagnostic;
   ratified as dependency-free rather than cryptographic, see there for
   the full reasoning) is stronger than a substring match anyway, since it
   covers the *whole* body rather than one substring, and it logs no body
   content at all. This is the standing `recognized == reassembled ==
   forwarded` tell made observable live — an **accidental-divergence
   check between two cooperating in-house processes, not a cryptographic
   binding** — not merely a restated code-level property, and it is
   **unaffected by the credential ruling**: the request body is admitted
   and forwarded exactly the same way whether the credential is valid or
   not, since authentication happens upstream, after forwarding.
   Response-side, **the diagnostic `401` (Final State #6), correlated via
   `in_reply_to` and the serial-bound-of-1 exclusivity, is what this
   sprint's response-side tell now is** — stated with the honest,
   narrower scope this actually has: a fixed `401` rejection is **not**
   per-call-unique the way a real success response's `id` would have
   been, so this proves *reachability and correct real-path behavior*
   (a genuine TLS connection and HTTP round trip to the real, hardcoded
   endpoint, correctly translated into a `NetError` delivered on the
   box's own correlated session) — **not** a content-integrity or
   anti-fabrication proof of a successful call. The self-declared bin-sha
   still binds these observations to the reviewed bytes; nothing here
   claims more than what is actually established.
   **This tell lives in two separate logs, not one** (reviewer's
   precision point): the box only ever sees the generic `NetError
   (Unavailable)` in `box.log`; the `"http status: 401"` line is on
   `serve`'s own stderr. Establishing the tell means correlating *both*
   logs (serve's status-and-fingerprint lines against the box's
   net-error and `in_reply_to`) — the capture procedure must capture and
   correlate both, not treat `box.log` alone as the record the way the
   quarantine tell already can. **A `401` does not by itself prove the
   credential was attached to the request** (reviewer's precision
   point): Anthropic returns the same `401` for a wrong key and an
   absent one alike, so header-attachment stays a *code* property (the
   `.header("x-api-key", key)` call is unconditional) — not something
   this capture directly observes. "Correct real-path behavior" should
   not be read as proving that leg specifically.
8. **A scripted head client observes the real display frame**, carrying a
   marker now derived from **the box's own observation of the net-error**
   (not a response `id`, which no longer exists under this plan) — reusing
   Sprint 14's nonce'd-injection precedent (a real socket client, not a
   human terminal) — proving the socket carries a frame whose marker
   traces to the box's real net-error observation, not a human-verified
   but non-repeatable proof.
9. **Both filesystem roots are operator-provisioned, not just the
   quarantine** (found independently during planning by both dev and
   reviewer: `assemble` validates *both* roots, and the compiled default
   readable root, `/srv/zerotrust-proxy/readable`, does not exist on the
   bench host — `/srv` itself is root-owned — so the ladder would close at
   `Closed(FilesystemRoots)` before the credential rung is ever reached,
   after the entire bench sequence up to that point has already run).
   `/srv/zerotrust-proxy/readable` needs only to **exist**, nothing more —
   `KernelProtections` probes *only* the quarantine fd
   (`lifecycle.rs:185`, `r.quarantine()`); the readable root's validation
   is deliberately the asymmetric resolve-plus-directory-check (Sprint 16),
   so it does **not** need protected mount flags, and `/srv`'s ordinary
   `ext4 rw,relatime` (no `noexec`/`nosuid`/`nodev`) is correct as-is — do
   not "fix" this by relocating the readable root onto a protected mount.
   **The quarantine's chown target is the subdirectory, never its parent
   — stated explicitly because the naive reading is the dangerous one**:
   `HEAD_SOCKET_PATH` (`/run/zerotrust-proxy/head.sock`) lives in the
   *parent* directory, while `QUARANTINE_ROOT`
   (`/run/zerotrust-proxy/quarantine`) is a *subdirectory* of it. If the
   parent itself were chowned to the drop-target uid, that uid could then
   write into the same directory where `serve` (as root, pre-drop) does
   Slice 1's new unlink-then-bind of the head socket — an unlink/bind race
   introduced by this sprint's own fix. The provisioning step is
   therefore: **parent (`/run/zerotrust-proxy`) stays root-owned `0755`,
   traversable; only the `quarantine` subdirectory is `chown`-ed** to the
   drop-target uid/gid. `/srv/zerotrust-proxy/readable` exists and is
   likewise made traversable/readable by the drop-target uid — not
   required by this capture's tells (no `file-read` is in scope), but
   provisioned correctly anyway, since planting the identical
   validated-as-root/read-as-nobody trap on the read side while already
   fixing it on the write side would be a needless residual. Both roots
   provisioned by the operator, **outside** the harness, per Non-Goals.
   The capture records the **observed pre-run state** of both (owner,
   mode; the quarantine mount's `noexec,nosuid,nodev`) in its header, then
   proves a real file-chunk write lands in the quarantine, written as the
   dropped uid — the one proof `Served` structurally cannot provide
   (Sprint 16's named residual), closed here by evidence rather than by a
   rung.
10. **The exact start order is followed and enforced/documented**:
   operator provisions both roots → `serve` starts and blocks on `accept`
   → the scripted head connects → `serve` proceeds into `bring_up` →
   `s17_box` starts driving. Written down explicitly, since it is not
   obvious and reversing box/head order hangs the capture.
   **The scripted head client runs as root** (reviewer's finding,
   verified empirically on this host: the socket is bound *before* the
   privilege drop, so under the standard `umask 022` it is created
   root-owned mode `0755` — only the owner has write, and Unix-socket
   `connect()` requires write permission, so **only root can connect
   today**). Stated here in the same explicit register as the box/head
   ordering, because the failure mode is not an error but a **silent
   hang** at exactly the step everything else already waits on.
   **Named, not silently accepted**: if the real deployment's head is an
   ordinary, unprivileged operator terminal, it could not connect to this
   socket either — a capture whose scripted head happens to run as root
   would prove a round trip the real deployment shape cannot reproduce.
   Ruled directly, without escalating (the proxy↔head transport's exact
   shape is already an explicitly open, unpinned residual —
   `docs/proxy/contract/proxy-head.md` §3, "the transport is deliberately
   unpinned"): this sprint runs the scripted head as **root**, and
   Non-Goals names the unprivileged-head-connect path as an explicit,
   out-of-scope residual this capture does **not** prove. Giving the
   socket a real ownership story (a `chmod`/`chown` after bind, or binding
   after the drop) is new production capability resolving that open
   transport question — not this sprint's trade to make as a side effect
   of a capture.
11. **The credential reaches `serve` without ever touching the process
   table or shell history — the same hygiene discipline applies even
   though this value is now a deliberately invalid one, not a live
   secret** (found during planning: this host's `sudo` has `env_reset`
   set, so a plain `sudo ./serve` would NOT pass `ANTHROPIC_API_KEY`
   through — closing at `Closed(Credential)` with everything else
   correctly provisioned). The operator invocation is `sudo -E ./serve`,
   with the credential exported in the **operator's own shell
   environment** beforehand — never `sudo ANTHROPIC_API_KEY=... ./serve`,
   which would place the value in the process table and shell history.
   `Credential`'s `is_some_and(|c| !c.is_empty())` rung requirement
   (Sprint 16) is satisfied by any non-empty value, valid or not — the
   ladder has no way to distinguish an invalid credential from a valid
   one at the authority rung (by design: validating the secret's *shape*
   was already rejected as a Non-Goal at Sprint 16), so `Served` is
   reached exactly as before, and authentication is checked only by the
   real upstream call.
12. **The bench is left as found**: both cards are `ixgbe`-bound before
    this capture; each binary's `construct` unbinds its own for the
    run. The operator steps include rebinding both cards to `ixgbe`
    afterward, so a later sprint's sysfs MAC read (Final State #1) and any
    other bench user find the cards in their standing state.
13. **Two artifact-hygiene gates, since this run makes a real external
    call to a third party — the credential half is now lower-stakes
    (an invalid, deliberately bogus value) but still checked, since a
    stray real key could land in the same keyfile mechanism by
    operator error**: (a) the request body sent to Anthropic is
    **synthetic by construction** — the nonce plus a fixed, minimal
    filler message, never repo content, host identifiers, or path
    information; nothing about this bench or codebase is published to a
    third party. (b) the capture's own log — not just `serve.rs`'s
    existing redacted startup line — is **reviewed for credential
    material before it is committed or shared**; `serve.rs` itself only
    ever prints the variable's name and presence (Sprint 16), but the new
    harness output and any error paths have not been through that same
    check and must be before the capture artifact is attached to this
    sprint document.
14. **Full capture-model discipline, with `serve`'s coherence established
    externally rather than self-declared** — found at the Slice 3 gate
    (dev's finding, ratified): `s17_box` and `s17_head` self-declare their
    full `Provenance` header (`sha256`, `rustc`, profile, `rustflags`,
    `git`, host, unix-ts) exactly as before, but `serve` cannot — adding
    `benchtools` (which shells out to `sha256sum`/`hostname` to capture
    it) to the `serve` feature would put two subprocess-spawns into the
    *deployed production binary* for a diagnostic, the same class of TCB
    cost this sprint already rejected for the fingerprint. **`serve`'s
    coherence is instead established externally**: hash
    `/proc/<pid>/exe` of the *live, running* `serve` process — the
    kernel's own handle on the executing image, obtained with zero
    changes to `serve.rs` — and re-hash the on-disk artifact after the
    run to prove it did not change underneath. This gives the identical
    property Provenance's self-declare exists for (binding the number to
    the bytes that actually ran, not a potentially-stale build) via an
    external observation rather than an internal one. `serve`'s
    build-time bin-sha (computed the same way `Provenance` would, from
    the built artifact before the run) plus its exact `cargo` invocation
    are still recorded and reviewed exactly as any other binary's would
    be — only the *live-coherence* step differs in mechanism, not in
    what it proves.
    **Why this is not a weaker binding, verified against source**
    (reviewer): `Provenance::capture()` hashes `std::env::current_exe()`,
    which on Linux resolves through `/proc/self/exe` — so the self-declare
    and this external read hash the *same kernel-maintained link to the
    executing inode*, just observed from outside the process instead of
    inside; the property (binding the number to the bytes that ran) is
    identical, only who attests differs. **Read during the `accept`
    window, as root, by explicit pid — three load-bearing specifics**:
    (a) read it while `serve` is blocked on `listener.accept()` (before
    `bring_up`) — a deterministic, stably-alive window with no race
    against the process exiting; (b) the reader must be **root** —
    reading another process's `/proc/<pid>/exe` requires
    `ptrace_may_access`, which root always satisfies regardless of
    `serve`'s own privilege state at the time; (c) **use the explicit pid
    form, `/proc/<pid>/exe`, never `/proc/self/exe`** — a genuine trap:
    a child `sha256sum` given the literal string `/proc/self/exe`
    resolves `self` to *its own* process and silently hashes the wrong
    binary, producing a plausible-looking hash that would read as a
    coherence failure while actually being a procedure bug. **Record the
    pid and `/proc/<pid>/cmdline` alongside the hash** — if `serve` is
    launched under `sudo` or any wrapper, the pid in hand may belong to
    the wrapper rather than `serve` itself, and `cmdline` is what makes
    the identification checkable rather than assumed.
    **The full acceptance chain is four values, all recorded so a
    mismatch is diagnosable rather than a hunt**: the reviewer's own
    fresh remap-recipe rebuild, the build-time bin-sha, the `/proc`
    -during-run hash, and the on-disk-after-run hash. Each disagreement
    means something specific: on-disk-after differing from `/proc`-during
    means the binary was replaced mid-run; the reviewer's rebuild
    differing from the build-time hash means recipe drift — most likely
    the feature set (the exact concern this Final State item's earlier
    half already names).
    **If any source changes between capture attempts, the WHOLE
    four-value chain is re-recorded for every affected binary, not just
    the one line that looks stale** (reviewer's finding, preventive): a
    capture sprint re-runs by nature, and the natural failure mode is a
    quick fix to one binary followed by an immediate re-run, silently
    producing a capture whose provenance is current for one binary and
    stale for the others — which a reviewer reproducing from the
    recorded recipe would then see as a coherence failure that is
    actually a partial re-record. Any rebuild, however small, means a
    fresh build-time bin-sha, a fresh `/proc`-during read, a fresh
    on-disk-after hash, and the exact invocation re-recorded, for that
    binary. **"Affected" is wider than the file that motivated the
    change** (dev's sharpening): all three binaries link the same
    `proxy` library, so a change anywhere in `src/` (not just a
    `src/bin/*.rs`) changes all three bin-shas, not only the one whose
    behavior prompted the edit — the natural mistake is treating a
    `lifecycle.rs`/`dispatch.rs`/`head.rs` change as "the proxy side" and
    re-recording `serve` alone, leaving `s17_box`/`s17_head` stale for a
    reason that has nothing to do with them. Practical rule: a change
    confined to one `src/bin/<name>.rs` re-records that binary only; a
    change anywhere else in `src/` re-records all three. **Re-run the
    preconditions from scratch on any re-attempt, never assume the bench
    state persists**: a prior attempt may have left a card unbound, and
    `/run` is tmpfs — a reboot between attempts silently removes the
    quarantine, and the ladder would then close at `FilesystemRoots`
    looking like a regression rather than what it is. Exact-BDF
    allowlists (`41:00.0` for `serve`, `63:00.0`
    for `s17_box`), never device-id enumeration, the production NIC
    (`61:00.0`) never touched; operator paste-and-confirm before any
    hardware run; the reviewer's verdict splits binary-coherence (theirs,
    verified fresh via the remap-recipe) from run-outcome (the dev's
    self-pinned capture, flagged as such). **The exact `cargo` invocation
    for each of the three binaries is recorded alongside its header**,
    including its full feature list (`serve` for `serve`;
    `bench-capture` for `s17_box`/`s17_head`) — sharpened from Sprint
    14's experience (reviewer's finding): `Provenance`'s header does not
    capture the feature set, and this capture has two different feature
    gates across three binaries. Building all three in one invocation
    with the union of features (`--features serve,bench-capture`) can
    produce a `serve` binary that is *not* byte-identical to one built
    with `--features serve` alone — feature unification can change what
    the library compiles to — so reproducing with the wrong feature list
    would look like a coherence failure that is actually a wrong recipe.
    Recording the exact invocation per binary makes coherence a one-pass
    verification rather than a round trip back to the dev.
15. The sprint document carries role retrospectives; the sprint is
    committed with only its intentional files.

## In Scope — ordered slices

**Slice 1 — the two `serve.rs` blockers.**
`PEER_BOX_MAC` configured (Final State #1); unlink-before-bind on the head
socket (Final State #2). Small, isolated, behavior-preserving except for
the peer addressing this capture specifically needs. Gate: existing test
suite passes unchanged; a manual re-bind check (bind, drop, re-bind) proves
the stale-socket fix.

**Slice 2 — `s17_box` and `s17_head`, the bench harness.**
`s17_box` (Final State #4): real `Endpoint::Box` construction, one
nonce'd net-request, response receipt, the resulting display-frame emission
toward the head, **and one `file-write`/`file-chunk` pair carrying a second
nonce into the quarantine** (the only producer of the quarantine-write
tell; found during planning as finding A, folded into Final State #9).
Legible `net-error` recording if the upstream call fails — **now the
designed, expected outcome under the operator's no-billing ruling (Final
State #5), not a fallback**. **`serve.rs`'s `body_fingerprint` diagnostic
addition** (Final State #3, ratified non-cryptographic — dependency-free,
not `sha2`) and **the `401`-status diagnostic** (Final State #6, found at
this same gate), both required for their respective tells to have an
observer at all. **`s17_head`**, the scripted head client (Final State
#8): built in this slice rather than Slice 3, a scope extension beyond
the slice's original naming that review endorsed as a genuine risk
reduction (no hardware touched building it; both ends of the socket
reviewed together; no unreviewed component arrives at the bench during
Slice 3), recorded here rather than silently absorbed. Gate: compiles
under `--features bench-capture`; exact-BDF literal on `s17_box`;
reviewed against the box/head chain (Final State #8) and the file-write
tell (Final State #9) before any hardware run.

**Slice 3 — the live capture.**
Operator provisions **both** filesystem roots — quarantine and readable —
per Final State #9's correction (findings B/C: the compiled readable-root
default does not exist on the bench host, and `assemble` validates both),
with the quarantine's *subdirectory*, not its parent, `chown`-ed (the
unlink/bind-race sharpening); the operator invokes `serve` as
`sudo -E ./serve` with the deliberately invalid credential pre-exported
in their own shell (Final State #5/#11; finding B: plain `sudo` strips it
via `env_reset`), never as a `sudo VAR=...` command line. The two
binaries run in the enforced start order (Final State #10); the round
trip completes end to end (Final State #7, the fingerprint match and the
diagnostic `401` as the two tells) with the no-billing upstream call
(Final State #5/#6); the scripted head (run as root, per Final State
#10's residual) observes the real display frame (Final State #8); the
quarantine write proceeds against the provisioned directory with pre-run
state recorded (Final State #9); both bench cards are rebound to `ixgbe`
afterward (Final State #12); the capture log is scrubbed for credential
material before it is attached to this document (Final State #13). Gate:
full capture-model discipline (Final State #14); every tell present
(request-body-fingerprint-match, response-401-status-diagnostic,
head-frame-marker, quarantine-write-as-dropped-uid); operator
paste-and-confirm executed before hardware is touched.

## Non-Goals

- **Do not treat this capture as proving an unprivileged head can
  connect.** Found and verified during planning (reviewer): the head
  socket is bound *before* the privilege drop, so under the standard
  `umask 022` it is created root-owned mode `0755` — only root can
  `connect()` to it today. This sprint's scripted head client runs as
  root; the real deployment's head connection method (whether it is
  root-run, or the socket needs a real ownership story — a `chmod`/
  `chown` after bind, or binding after the drop) is the already-open
  proxy↔head transport-shape question (`docs/proxy/contract/proxy-head.md`
  §3, "deliberately unpinned"), and resolving it is explicitly **not**
  this sprint's trade — a capture that quietly ran root and called the
  round trip "proven" without naming this would over-claim.
- Do not build a `two_facade`-based harness for this capture — verified
  structurally incompatible with driving the real composition root
  (Scope Boundary).
- Do not add a compiled DSL/register entry for the peer MAC — a bare
  `const` matching the existing `PROXY_BDF`/`HEAD_SOCKET_PATH` pattern,
  not a security authority.
- Do not modify `bring_up`, `assemble`, or any Sprint-16 authority logic.
- Do not attempt a successful model call, ask the model to echo the
  nonce, or assert on response content, streaming shape, or retries — the
  operator's ruling makes a `401` the designed outcome, and the round-trip
  tells are structural (serial bound, `in_reply_to`, the diagnostic
  status code), not content-based; testing Anthropic's behavior is not
  this sprint's territory.
- Do not automate `/run/zerotrust-proxy`'s or `/srv/zerotrust-proxy`'s
  creation or ownership inside the harness — a deployment act, done by the
  operator, precisely so the capture can catch a mis-provisioned directory
  rather than paper over it.
- Do not pass the credential on a `sudo` command line
  (`sudo ANTHROPIC_API_KEY=... ./serve`) — the operator invocation is
  `sudo -E` with the variable pre-exported in their own shell, so the live
  secret never lands in the process table or shell history.
- Do not treat this capture as reload-without-restart's resolution — that
  residual (`architecture.md` §11, L12) stays open, unrelated to this
  sprint.
- Do not touch the production NIC (`0000:61:00.0`) under any
  circumstance — exact-BDF allowlists on both binaries, paste-and-confirm
  before every hardware run.

## Starting Surface

Audit first, in this order:

1. `proxy/src/bin/serve.rs` — `PEER_BOX_MAC`, the head-socket bind, and the
   accept-before-`bring_up` ordering this sprint's start sequence depends
   on.
2. `proxy/src/lifecycle.rs`'s `bring_up` — confirming `Subsystem::construct`
   is called *inside* it (the reason `two_facade` cannot host this
   capture).
3. `directpath/src/harness.rs`'s `two_facade` — read to confirm why it does
   *not* apply here, not to adapt it.
4. `proxy/src/bin/s5_capture.rs` and `s14_quiescent_wake.rs` — the
   bin-sha-self-declare, exact-BDF, and capture-model discipline this
   sprint's two binaries reuse verbatim.
5. `proxy/src/lifecycle.rs::assemble` and `proxy/src/register/filesystem`
   — the compiled quarantine **and readable** root paths this capture
   provisions against (Sprint 16); the readable-root default does not
   exist on the bench host and must be provisioned too (finding C).
6. This host's `sudoers` (`env_reset` is set) — confirms the operator
   invocation must be `sudo -E` with the credential pre-exported, not a
   plain `sudo` (finding B).
7. `[[silicon-evidence-anchor-degrades]]` — the standing evidence-model
   precedent for a reviewer who cannot freely re-run privileged hardware.

## Method

- **Capture-model, same discipline as every prior hardware sprint**: each
  binary self-declares its running bin-sha; the reviewer's verdict splits
  binary-coherence (theirs, verified fresh via the remap-recipe) from
  run-outcome (the dev's self-pinned capture, flagged as such).
- **Exact-BDF bench safety** (the standing hard constraint): `serve` gates
  on `0000:41:00.0`, `s17_box` on `0000:63:00.0`, both via `Bdf::parse`
  literals, never device-id enumeration; the production NIC must never be
  touched; operator paste-and-confirm before any hardware run.
- **Start order is load-bearing, not incidental**: provision → `serve`
  starts (blocks on `accept`) → head connects → `serve` proceeds to
  `bring_up` → `s17_box` starts. Reversing box-before-head hangs the
  capture; this is written down so it is not rediscovered at the bench.
- **`serve` is killed externally after the capture, never made to
  self-exit** — found at the Slice 3 gate: `serve` is the real production
  loop and correctly never terminates on its own (`on_verdict` always
  returns `Continue`); a production proxy that exits after one round trip
  would be a genuine regression, so the fix is an external `SIGTERM`
  (escalating to `SIGKILL` only if ignored), never a change to
  `serve.rs`'s behavior for capture convenience. **Sequenced strictly
  after the quarantine write is verified, never immediately on `s17_box`
  exiting** (reviewer's finding): the box's last phase completes on the
  transport ACK, which the reactor generates on frame receipt — *before*
  the proxy's app-level dispatch actually performs the confined open and
  write. A kill landing in that window would lose the write and fail
  tell #4 in a way that reads as a proxy defect, not a script race — the
  worst kind of false negative. The teardown step therefore verifies the
  quarantine file exists and contains nonce#2 *first*, then kills `serve`,
  then takes the on-disk-after hash.
- **The round-trip tells are structural, never content-based**: the
  request-side bind-the-chain tell — a live `body_fingerprint` match
  between the body `serve.rs`'s `on_verdict` admits and the body
  `s17_box` sent, not a printed nonce with no observer (found and closed
  during Slice 2's design review) — the response-side tell, under the
  operator's no-billing ruling, is the **diagnostic `401` status** printed
  from the real `ureq::Error`, correlated via `in_reply_to` and the
  serial-bound-of-1 exclusivity (honestly narrower than the originally
  planned unsynthesizable-`id` tell would have been — see Final State #7
  for the precise scope of what this does and does not prove), the
  head-side marker tell (a display frame traceable to the box's own
  net-error observation), and the quarantine-side write tell (a real file
  landing as the dropped uid). No tell depends on the model's
  instruction-following, and none requires a successful call. Stated
  honestly: this rules out a coincidental or purely-local failure; it is
  not an anti-fabrication proof, and the capture model does not require
  one.
- **The provisioned quarantine's chown target is the subdirectory, never
  its parent.** `HEAD_SOCKET_PATH` lives in the parent
  (`/run/zerotrust-proxy`); `QUARANTINE_ROOT` is a subdirectory of it. The
  parent stays root-owned and merely traversable; only the `quarantine`
  subdirectory is `chown`-ed to the drop-target uid. Chowning the parent
  would make it writable by an unprivileged uid while `serve` (as root,
  pre-drop) performs Slice 1's new unlink-then-bind of the head socket
  inside it — an unlink/bind race this sprint's own fix would otherwise
  introduce.
- **The upstream request body is synthetic by construction**: the nonce
  plus a fixed, minimal filler message only — never repo content, host
  identifiers, or path information. This is a genuine call to a third
  party; nothing about this bench or codebase is published in it.
- **The capture log is reviewed for credential material before it is
  attached to this document.** `serve.rs` itself only ever prints the
  credential's variable name and presence (Sprint 16), but the new
  harness's own output and its error paths have not been through that
  same check and must be scrubbed before sharing or committing.
- **Provisioning is recorded, not performed.** The capture's header names
  the observed pre-run state of the quarantine (owner, mode, mount flags)
  precisely because `Served` cannot prove writability — this is the one
  place evidence, not a rung, closes Sprint 16's named residual.

## Acceptance

- REJECT a harness built on `two_facade` for the real composition path —
  structurally a replica, not the shipped path.
- REJECT a peer-MAC fix implemented as a new compiled DSL/register
  entry — matches the existing bare-`const` addressing pattern, or send it
  back.
- REJECT a capture that asks the model to echo the nonce, or that asserts
  on response content/streaming shape — the tells are structural, not
  content-based.
- REJECT a capture that provisions or `chown`s either filesystem root
  itself inside the harness — must be an operator act, recorded not
  performed.
- REJECT a `s17_box` build that cannot produce the quarantine-write tell —
  the net-request/response/display-frame path alone cannot; the file-write
  pair is required in scope.
- REJECT an operator invocation that passes the credential on the `sudo`
  command line, or a plain `sudo` that would silently drop it via
  `env_reset` — `sudo -E` with a pre-exported variable only.
- REJECT proceeding to Slice 3 without both filesystem roots provisioned
  — the readable root's compiled default does not exist on the bench host
  by default and its absence closes the ladder before the credential rung.
- REJECT chowning `/run/zerotrust-proxy` itself (the parent) to the
  drop-target uid — only the `quarantine` subdirectory; chowning the
  parent introduces an unlink/bind race on the head socket.
- REJECT attempting a successful upstream call, or leaning on response
  content (an `id`, `usage`, or otherwise) as a tell — the operator's
  ruling makes a `401` the designed outcome; the diagnostic status code,
  correlated via `in_reply_to` and the serial-bound-of-1 exclusivity, is
  the response-side tell, honestly scoped as reachability-and-correct-
  behavior, not content-integrity.
- REJECT relying on the box's `NetError` code alone to identify a `401`
  specifically — `map_ureq_error`'s catch-all collapses every HTTP-status
  failure into the same generic code; the diagnostic print of the real
  `ureq::Error` (Final State #6) is what actually establishes it was a
  `401`, not a TLS failure or an unrelated local error.
- REJECT an upstream request body carrying anything beyond the nonce and a
  fixed minimal filler — no repo content, host identifiers, or path
  information published to the third party.
- REJECT attaching the capture log to this document before it has been
  reviewed for credential material in the harness's own output and error
  paths.
- REJECT a request-side tell resting on an unobserved value — the box
  printing a nonce that nothing on the proxy side ever records is not a
  tell; the `body_fingerprint(admitted) == body_fingerprint(sent)` match,
  checked live from both ends, is what actually closes it.
- REJECT `s17_box` fingerprinting the L2-framed payload instead of the raw
  JSON body — the two are different values by construction and can never
  match the proxy's fingerprint of the admitted body.
- REJECT an unconditional `sha2` (or equivalent cryptographic) dependency
  added to the `proxy` crate for this diagnostic — `serve` is the
  deployed production binary, so gating behind either feature still lands
  it in what the operator runs; `body_fingerprint`'s dependency-free
  design (ratified by both dev and reviewer) is the standing answer, not
  a placeholder pending a "real" hash.
- REJECT citing this fingerprint as adversarial or anti-fabrication
  evidence — it is an accidental-divergence check between two cooperating
  in-house processes, printed for observation and never branched on; if
  bind-the-chain here ever became an *enforced* control, or the compared
  values ever crossed a boundary where a party could craft a collision,
  this design would need revisiting before reuse.
- REJECT a capture missing any of the four tells
  (request-body-fingerprint-match, response-401-status-diagnostic,
  head-frame-marker, quarantine-write-as-dropped-uid).
- REJECT any device-id-based enumeration, a missed exact-BDF gate, or a
  missed operator paste-and-confirm step on either binary.
- REJECT claiming this capture resolves reload-without-restart or any
  other Sprint-16 residual beyond the live round trip and the quarantine-
  writability gap.
- REJECT any report or retrospective that presents this capture as proving
  an unprivileged head can connect — the scripted head runs as root, and
  the real deployment's head-connection method is a separate, still-open
  question this sprint does not resolve.

## Review Questions

None outstanding at contract time. The two `serve.rs` blockers (peer MAC,
stale socket) and the `two_facade`-incompatibility finding are verified
facts of the current source, not open design forks; the peer-MAC placement
(bare `const`, not a register entry) is resolved directly against the
existing `PROXY_BDF`/`HEAD_SOCKET_PATH` precedent in the same file, not
escalated. Route to review for confirmation before execution, given the
real-hardware and real-upstream-cost stakes.
