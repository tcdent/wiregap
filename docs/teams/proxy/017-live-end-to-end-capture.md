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
   item 1's two fixes and one further diagnostic addition (below — a
   second was originally planned and is now superseded by Sprint 18's
   verbatim relay), run against a deliberately invalid credential (the
   operator's no-billing ruling, Final State #5), a real upstream
   connection, a scripted head client, and an operator-provisioned
   quarantine.
4. **One `serve`/`lifecycle.rs` diagnostic** (a second was originally
   planned here and is now superseded — see the note at Final State #6):
   `on_verdict`'s `Routed::NetPayloadAdmit(body)` arm prints
   `lifecycle::body_fingerprint` of the admitted body; `s17_box` prints the
   identical function's output for the body it sent — matching
   fingerprints, observed live, close the request-side tell. Content-free
   with respect to anything secret, additive, and changes no behavior or
   authority logic.

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
   **`net-response` carrying the real endpoint's own rejection bytes,
   verbatim** — **the designed, expected outcome under the operator's
   no-billing ruling (Final State #5) combined with Sprint 18's verbatim
   relay, not a failure mode** (corrected from this Final State's original
   text, which predates Sprint 18 and expected a `net-error`; a `net-error`
   here now means only a genuine transport failure, confirmed against
   `proxy/src/bin/s17_box.rs:299-324`) — recording the response
   **legibly** in its own log, so the outcome is checkable rather than
   looking indistinguishable from the harness being stuck. On receiving
   it, `s17_box` emits one display frame toward the head carrying a
   marker derived from `lifecycle::body_fingerprint` of the received
   response (the box→head chain Q3 established: the head only ever
   observes display frames, which are the box's to emit). **Also drives
   one `file-write` + `file-chunk`, carrying
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
   control characters. **Corrected against current source**: under the
   removed `ureq`/`http`-crate path, a malformed value was rejected
   locally (`ureq::Error::Http`, the request never leaving the machine).
   Sprint 18's hand-rolled `write_request` (`proxy/src/upstream.rs:468`)
   does no such validation — `key` is interpolated directly into the
   header line with no local safety net at all. **This is a gap in
   Sprint 18 FS#5's own proof, not only a robustness gap** (reviewer's
   sharpening): FS#5 proves the wire request is exactly the admitted body
   plus the credential header via full-buffer equality — proven only for
   a *well-formed* credential. A CRLF in the credential does not merely
   malform one header; it terminates the header block early, so
   `content-length` and the real body become part of whatever follows,
   and the body boundary moves. FS#5's proven property is therefore
   *false* for a malformed credential, and FS#5's own test cannot catch
   this because it uses a well-formed key — an unstated precondition on
   an already-accepted proof, not merely a missing validation.
   `write_request`'s own doc comment stops one step short of this: it
   argues the credential is operator-authored and never box-influenced
   ("no header line takes a box-supplied value"), which is a correct
   *containment* argument and entirely silent on *format-breakage* — it
   answers "can the box escape" and not "can this break byte-exactness".
   **A genuine residual, named here and routed to the PM as a finding,
   not fixed by this sprint**: Non-Goals bar changing `bring_up`/
   authority logic, and `write_request` is further still than that.
   **This sprint's mitigation, in scope as a gate item rather than a
   deferral** (reviewer's addition): the capture procedure asserts the
   credential value is well-formed — visible ASCII, no CR, no LF —
   *before* the run, converting "pinned by discipline" into "checked at
   the one place it matters for this run", with no Sprint-18 code
   touched. See Slice 3's gate and the Acceptance list; (c) **well-formed but
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
   **Formerly an open finding routed to dev — now resolved by
   elimination, not by pinning.** This item originally found that the
   `401`-takes-the-`Err`-path mechanism depended on `ureq`'s unpinned
   *default* `http_status_as_error` setting, and asked dev to pin it
   explicitly before this item closed. Sprint 18 removed the setting
   along with `ureq` itself: there is no HTTP-status-based branch left
   anywhere in the upstream call (`upstream_call`/`stream_response`
   relay any TLS-connected response verbatim regardless of status,
   confirmed at Final State #6/#4) — so there is no third-party default
   left to silently flip. The concern this item raised is fully closed,
   not merely addressed.
6. **SUPERSEDED by Sprint 18 — the diagnostic this item originally added no
   longer exists, and nothing needs to replace it.** This Final State
   originally found that the box only ever received `NetError(Unavailable)`
   (`upstream.rs::map_ureq_error`'s catch-all collapsed every HTTP-status
   failure into that one generic code) and closed the gap with a
   diagnostic print of `ureq::Error`'s `Display` on `serve`'s own stderr,
   since that was the only place the real status was ever visible.
   Sprint 18 removed `ureq` and `map_ureq_error` entirely and made the
   upstream relay verbatim in both directions (Sprint 18 FS#4): a `401`
   is not an error at all under the current design — any TLS-connected
   call streams its response, status line and headers included, to the
   box as an ordinary `net-response` (confirmed against source:
   `proxy/src/upstream.rs`'s `stream_response`/`enqueue_net_response`
   path; `net_error` is now reached only by a genuine transport failure —
   confirmed against `proxy/src/bin/s17_box.rs:299-324`, already updated
   to this reality). The box now observes the real `HTTP/1.1 401
   Unauthorized` status line directly, which is strictly more than the
   old diagnostic ever gave an observer outside `serve`'s own stderr — see
   Final State #7 for the response-side tell this enables.
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
   Response-side, **rebuilt entirely on Sprint 18's verbatim relay
   (Final State #6 is superseded) and now rests on two observations made
   directly at the box, correlated via `in_reply_to` and the
   serial-bound-of-1 exclusivity** (reviewer's correction, twice
   retracted and reissued during the relay's own design — recorded here
   so it is not re-derived): both live in `box.log`, in one place, not
   split across two logs as the superseded design required.
   1. **The status line, read directly, not inferred.** `s17_box`'s
      accumulated `response` bytes begin `HTTP/1.1 401 Unauthorized` —
      the real endpoint's own line, relayed verbatim, never a `serve`-side
      diagnostic standing in for it.
   2. **Anthropic's own `request-id` response header — genuinely
      per-call-unique**, correcting this Final State's earlier framing:
      under the removed diagnostic design a fixed `401` was *not*
      per-call-unique the way a real success response's `id` would have
      been; under verbatim relay the header route is open too (Sprint 18
      FS#4 relays the whole response, headers included), so `request-id`
      restores exactly the unsynthesizable, per-call evidence that a
      successful call's `id` would have given, without needing a
      successful call. **Verified against a live response (Slice 3's
      capture, 2026-07-25)**: `request-id: req_011CdP6wybnrmm3yDQ5pibFb`,
      present and well-formed, corroborated twice independently — the
      header and the JSON error body's own `request_id` field agree — so
      the full claim below fired and the narrowed one was not needed.
      **The narrowed claim, written in advance rather than left to be
      decided at report time** (reviewer's finding), stands for any
      future re-run where the header is absent: this Final State's claim
      is then *reachability and correct real-path behavior only* — the
      status line alone, with **no per-call uniqueness** — and the
      capture report must state it in exactly that narrower form, not
      merely note the header's absence as an aside.
   **A `401`'s wording does not itself prove the credential was
   attached** (reviewer's precision note on Slice 3's own evidence,
   applying regardless of outcome): "invalid x-api-key" does not
   distinguish a *wrong* key from an *absent* one. Credential attachment
   is proven by Sprint 18 FS#5's byte-exact wire check (the header is
   provably on the wire), not by the rejection message's content — the
   conclusion holds, but this is the reason that actually carries it.
   **The correlation tell is at its weakest when the box's first session
   is 0** (reviewer's note from Slice 3's run): `in_reply_to = Some(0)`
   against request session `0x0` is correct (the box is
   `Originator::Low`, so its first session genuinely is `0`) but
   indistinguishable at that value from an uninitialized default. Not a
   defect in Slice 3's capture — a free strengthening for any re-run:
   have `s17_box` open one throwaway session before the `net-address`
   session, landing the real correlation on a nonzero id.
   Together these prove *reachability, correct real-path behavior, and*
   (via `request-id`) *that this specific call, not a stale or fabricated
   observation, is what is on record* — still **not** a content-integrity
   proof of a successful call, since none was attempted.
   **A response arriving does not by itself prove the credential was
   attached to the request** (reviewer's precision point, still holds
   under the new design): Anthropic returns the same `401` for a wrong
   key and an absent one alike, so header-attachment stays a *code*
   property (`write_request`'s credential-header call is unconditional,
   Sprint 18 FS#5) — not something this capture directly observes.
   "Correct real-path behavior" should not be read as proving that leg
   specifically.
8. **A scripted head client observes the real display frame**, carrying a
   marker derived from **`body_fingerprint` of the box's received
   response** (corrected: earlier text derived this from a net-error
   observation, predating Sprint 18's relay; there is no net-error on the
   expected path any more — see Final State #4) — reusing Sprint 14's
   nonce'd-injection precedent (a real socket client, not a human
   terminal) — proving the socket carries a frame whose marker traces to
   the box's real, received response, not a human-verified but
   non-repeatable proof.
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
    **Slice 3's coherence rebuild proved this feature-unification risk
    from source, not merely by rebuild-fingerprint** (reviewer): a
    `cargo clean -p proxy` followed by separate rebuilds of all three
    exact invocations reproduced every recorded bin-sha byte-identically
    — a stronger proof than a no-recompile check, since a drifted source
    tree could not recompile to identical bytes from a cleaned target.
    **A second recipe dependency, found and corrected in the same
    pass — the benchtools-linked binaries are TARGET-DIRECTORY-dependent,
    `serve` is not** (reviewer's finding, the near-miss worth recording):
    a first rebuild attempt into a separate target directory (to force a
    from-source compile without touching the tree) reproduced `serve`
    exactly but produced *different* bytes for `s17_box`/`s17_head` — two
    of three not matching is the exact shape of a coherence failure, and
    it was the recipe, not the tree (`benchtools`' build-script output
    path is embedded; `serve`, which does not depend on it, is
    unaffected). Confirmed, not inferred: a from-source rebuild in the
    *default* target directory reproduced both exactly. **Reproduction
    therefore requires the same target directory, alongside the same
    absolute source path, rustc, profile, rustflags, and per-binary
    feature set** — a reviewer following the recipe as written before
    this correction would have reported a false mismatch.
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
nonce'd net-request, response receipt (a `net-response` carrying the real
endpoint's verbatim rejection, per Sprint 18 — a `net-error` now means
only a genuine transport failure), the resulting display-frame emission
toward the head, **and one `file-write`/`file-chunk` pair carrying a second
nonce into the quarantine** (the only producer of the quarantine-write
tell; found during planning as finding A, folded into Final State #9).
**`serve.rs`'s `body_fingerprint` diagnostic addition** (Final State #3,
ratified non-cryptographic — dependency-free, not `sha2`) is required for
the request-side tell to have an observer at all; the response-side tell
needs no equivalent addition, since Sprint 18's relay already delivers the
status line and headers the box reads directly (Final State #6,
superseded). **`s17_head`**, the scripted head client (Final State
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
trip completes end to end (Final State #7 — the request-side fingerprint
match, and the response-side status-line-plus-`request-id` pair, as the
two tells) with the no-billing upstream call (Final State #5); the
scripted head (run as root, per Final State
#10's residual) observes the real display frame (Final State #8); the
quarantine write proceeds against the provisioned directory with pre-run
state recorded (Final State #9); both bench cards are rebound to `ixgbe`
afterward (Final State #12); the capture log is scrubbed for credential
material before it is attached to this document (Final State #13). Gate:
full capture-model discipline (Final State #14); every tell present
(request-body-fingerprint-match, response-status-line-and-request-id,
head-frame-marker, quarantine-write-as-dropped-uid); **the credential
value is asserted well-formed — visible ASCII, no CR, no LF — before
anything is unbound** (reviewer's addition, folding Final State #5's
proof-precondition finding into a checked precondition rather than a
pinned-by-discipline assumption; no Sprint-18 code touched); operator
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
  during Slice 2's design review) — the response-side tell, under Sprint
  18's verbatim relay, is **the box's own directly-observed status line
  (`HTTP/1.1 401 Unauthorized`) plus Anthropic's `request-id` response
  header**, correlated via `in_reply_to` and the serial-bound-of-1
  exclusivity (Final State #7 — this supersedes the originally planned
  `serve`-stderr diagnostic, and the header restores the per-call-unique
  evidence a successful call's `id` would have given), the head-side
  marker tell (a display frame traceable to the box's own response
  fingerprint), and the quarantine-side write tell (a real file landing
  as the dropped uid). No tell depends on the model's
  instruction-following, and none requires a successful call. Stated
  honestly: this rules out a coincidental or purely-local failure; it is
  not an anti-fabrication proof of a *successful* call, and the capture
  model does not require one.
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
  *body* content (an `id`, `usage`, or otherwise) as a tell — the
  operator's ruling makes a `401` the designed outcome; the response-side
  tell is the status line and Anthropic's `request-id` header (Final
  State #7), correlated via `in_reply_to` and the serial-bound-of-1
  exclusivity, honestly scoped as reachability-and-correct-behavior plus
  per-call uniqueness, not body content-integrity.
- REJECT a response-side tell that only checks "a response arrived"
  without confirming the relayed bytes actually begin `HTTP/1.1 401` and
  carry Anthropic's `request-id` header — a response arriving for an
  unrelated reason (a stray local listener, a misrouted session) would
  falsely satisfy a presence-only check; the specific status line and
  header are what establish the real endpoint was reached and rejected
  this specific call, not merely that some bytes came back.
- REJECT a credential-echo check scoped to the response body alone —
  under verbatim relay the credential could in principle surface
  anywhere in the relayed bytes (status line, headers, or body); the
  check must scan the *whole* buffer the box received, never a chosen
  part of it (the same full-buffer-over-enumerated-parts discipline
  Sprint 18 FS#5 established on the request side; reviewer's finding).
- REJECT proceeding to Slice 3 without asserting the pinned credential
  value is well-formed (visible ASCII, no CR, no LF) first — Sprint 18
  FS#5's byte-exact-wire proof is proven only for a well-formed
  credential (a CRLF would move the body boundary, not merely malform a
  header), so this capture's own no-code-touched mitigation is a checked
  precondition, not an assumption (reviewer's finding).
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
  (request-body-fingerprint-match, response-status-line-and-request-id,
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

## Retrospectives

### zero.proxy-review

**What I verified.** Before hardware: the bench MACs read from sysfs rather than
trusted; `/run`'s writability established by an actually-refused `mkdir` rather
than by its mount flags; the recognizer's top-level whitelist confirmed to admit
the planned request body; and the readable root found missing and unprovisioned,
which would have failed the capture at its last step after the full bench
sequence. At the gate: binary coherence, which was my half of the split verdict.
Run-outcome was taken from the self-pinned capture and flagged as such
throughout, never as independently witnessed.

**The coherence methodology, which is the most transferable thing here.** A
rebuild that reports *zero recompilation* proves only that cargo's fingerprint —
mtime and size — is unchanged. It does **not** prove that the current source
*content* compiles to the recorded bytes. So the verification was `cargo clean -p
proxy` (12,762 files, 3.5 GiB removed) followed by each binary's exact recorded
invocation, run separately. All three reproduced byte-identically. That is a
content-to-binary proof, and it *subsumes* the mtime, cargo-fingerprint, and
byte-comparison arguments rather than inheriting them: a source tree that had
drifted cannot recompile to identical bytes from a cleaned target. For a capture
whose source was never committed, this is the only thing that closes the
dirty-tree question.

**The near-miss inside that methodology, which matters more than the result.**
My first attempt built into a *separate* target directory, to force a from-source
compile without perturbing the tree. `serve` matched exactly; `s17_box` and
`s17_head` did not. Two of three failing is precisely the shape of a coherence
failure — and it was **my recipe**, not the tree. The two that diverged are the
two that link `benchtools`, whose build script output is target-directory
dependent; `serve` does not link it and was unaffected. What prevented a false
report was not caution but *investigating the asymmetry* — the pattern of which
binaries failed was itself the evidence, because a genuine tree drift would not
have spared exactly the one binary with no build-script dependency. I had warned
this team, before this capture, that a wrong recipe manufactures a mismatch that
looks like a coherence failure. I then produced one. **The reproduction recipe
must therefore specify the target directory** alongside the absolute source path,
rustc, profile, rustflags, and per-binary feature set — and the invocations must
be run separately, since feature unification links a bench-enabled `directpath`
into the production binary and changes `serve`'s bytes.

**What changed my read.** This sprint inverted its own premise twice, and both
times I was on the wrong side first. I argued for *pinning* the behaviour that
converted an upstream `401` into a proxy-authored error — which was itself the
MB3 violation, forbidden in terms by `cloud/approach.md` §5, which had even
pre-refused the shape of my argument. Then the full-raw-relay ruling superseded my
own stated claim that the box could not see the status line. The pattern in both:
I reasoned from a plausible model of the system instead of reading the artifact,
and it happened while *extending* something I had got right rather than while
testing someone else's claim.

**Two corrections of myself worth writing down.** First, I specified that the
`/proc/PID/exe` read must be performed as root, because reading another process's
exe link requires `ptrace_may_access` — and then, in the same message,
recommended recording `/proc/PID/cmdline` without carrying that reasoning across.
`cmdline` happens to be world-readable, so the recommendation works on this host
*incidentally*; under `hidepid=` it would have silently produced nothing, which
is the exact failure shape it existed to prevent. Dev's `sudo`-ordering fix
repaired a mechanism whose specification I under-wrote. Second, when the evidence
artifact gained two corrections, I verified that both were *present* — true — and
then narrated *who wrote them*, which I had not checked and which was recorded in
the file. I verified the fact and inferred the story. Same shape as the reversals
above; harmless in outcome, which is exactly when it slips through.

**The best evidence in this capture was not designed.** The upstream chose
`Transfer-Encoding: chunked`, and the raw chunk framing survived into the relayed
bytes — the length line and the terminating `0\r\n\r\n`, confirmed by `od`. Any
general-purpose HTTP client de-chunks that; the proxy did not, because it has no
response parser at all. That demonstrates the dumb-verbatim-pipe property using an
encoding nobody anticipated, which is stronger than the synthetic arms precisely
because nobody chose it. A capture should leave room for the real system to
surprise it, and should record what it did *not* design for — that is often the
strongest thing in the artifact.

**What the next sprint should inherit.** *Commit the source before a capture
run.* This capture's dirty tree was already a known weakness; the cost
materialised when a single post-capture prose fix — one I ruled should be made —
permanently stranded `s17_head`'s captured binary, since its source was never
committed and was overwritten. Slice 3's coherence is consequently **not uniform**:
`serve` and `s17_box` remain re-derivable, `s17_head` is anchored on a preserved
artifact plus a gate-time verification. Also carried: the correlation tell is at
its weakest when the session id is `0`, where a correct correlation and an
uninitialised default are indistinguishable — one throwaway session before the
net-address session removes the ambiguity at zero cost; and credential
*attachment* is established by FS#5's byte-exact wire check, never by the `401`'s
wording, which does not distinguish a wrong key from an absent one.

### zero.proxy-dev

**This sprint was blocked by its own investigation, and that was the sprint working.** It opened as a
live-capture sprint and stopped when the operator caught `http_status_as_error(true)` — a flag I had
verified thoroughly and never questioned. That stop produced Sprint 18 (the verbatim pipe) and Sprint 19
(the register fixes the dial), and only then could this one run. The capture that eventually passed is
not the capture that was designed: it proves the *relay* path rather than the error path, because an
invalid credential now arrives as the upstream's own 401 instead of a proxy-invented `net-error`. **A
capture sprint that cannot be run yet is evidence about the system, not an obstacle to it.**

**What the live run gave that no test server could.** The single strongest piece of evidence in the whole
capture was not designed for: the real response came back `Transfer-Encoding: chunked`, and **the raw
chunk framing survived into the relayed bytes** — the `82` length line and the terminating `0\r\n\r\n`,
confirmed by `od`. Any general-purpose HTTP client would have de-chunked it. Sprint 18's synthetic gzip
arm proves the same property, and this is better evidence, for one reason: **nobody chose it.** A test
server can only produce what someone thought to script; the real upstream picked an encoding we never
considered and the pipe relayed it untouched anyway. Worth keeping as a general principle for what a live
capture is *for* — not to re-confirm what the unit tests already cover, but to be surprised.

The `request-id` header was likewise a gift of the new shape (`req_011CdP6wybnrmm3yDQ5pibFb`), and it
corroborates itself: the same id appears independently in the response header *and* in the JSON body's
`request_id` field. Because the FS#7 narrowing was written **before** the run, its absence would have
produced a stated, narrower claim rather than an argument about how much the capture proved. Pre-writing
both branches cost twenty minutes and removed a whole category of after-the-fact reasoning.

**What failed, and the pattern connecting most of it.** Five things went wrong that were mine:

- **The credential well-formedness check accepted the input it existed to reject.** I wrote the obvious
  `grep -qE '^[!-~]+$'`. `grep` is line-oriented, so a credential containing LF splits into two lines and
  the pattern matches the first — it passed an LF-bearing value, the one character that would terminate
  the header block early and move the body boundary. Rewritten as a byte-count comparison, which has no
  line semantics.
- **My first reap probe used `exec`**, which replaced the shell, so the wrapper had no child and the
  child-sweep branch never ran. It passed having tested nothing.
- **My first restore after the feature-unification experiment rebuilt only `serve`**, leaving `s17_box`
  and `s17_head` sitting at their both-features bytes. Caught by comparing all three against the
  preserved copies rather than assuming one command undid one command.
- **I sent a message referencing an attachment I had not written yet.** Trivially avoidable; write the
  artifact, then reference it.
- **I edited `s17_head.rs` in place when it had never been committed** — see below.

The first three are the same failure: *a check that ran, reported success, and verified nothing.* That is
the identical shape as Sprint 18's vacuous FS#10 arm and Slice 1's `is_err()`-only refusals. The
counter-discipline that actually worked, every time it was applied: **exercise a new check against the
inputs it must REJECT before trusting it.** All three were caught that way, by me, before they reached
anything that mattered — which is the improvement over earlier in this arc, where review caught them.

**The one that could not be fixed after the fact.** `s17_head.rs` was never committed. After the capture
was accepted, it needed a one-line-class prose fix, I edited it in place, and the source that produced
the capture's `cd7d327d…` binary **ceased to exist** — no git history, not in the index, no copy on disk.
The binary is preserved and review independently re-hashed it, so the evidence stands; but one of three
binaries permanently lost its source-to-binary link.

I chose not to reconstruct the file, and that call is worth recording because the temptation was real: the
pre-fix text was still in my working context and I could have produced a plausible file in a minute. **A
reconstructed source would look like evidence while being an assertion** — it might not even rebuild to
`cd7d327d…`, and no one downstream could tell by inspection. The gap is the state of the record, so the
gap is what the record says.

**COMMIT SOURCE BEFORE THE RUN.** That is the lesson, and it generalises further than the one file: it is
the same shape as the *bin-sha binds bytes, not git* limit flagged in the evidence chain, where the
binaries self-declared `git: 0ebfaa0` against a dirty tree that could never rebuild them. **Both say the
git record has to be made BEFORE the measurement, not after.** Any future capture should commit the
source first, so that (a) the provenance header's git rev means something, and (b) a post-capture fix
moves the tree forward instead of stranding the evidence behind an untracked file.

**A second artifact-integrity lesson, from the two defects the run found.** Both were the same class: an
artifact asserting something no longer true. `s17_head`'s output described the superseded `net-error`
design while printing a `resp-fp:` marker, and the teardown printed "check manually" on a perfectly clean
run because it branched on a bind write that returns `EBUSY` when the card is *already* bound. The second
matters more than it looks — **a teardown that cries wolf every run trains an operator to skim past the
one message that matters** when a card genuinely is left unbound. Same reasoning retired the script's
stale `DO NOT RUN — STALE` banner once the capture had passed: that banner is read immediately before
someone touches hardware, and a false one is worse than none.

**What the next sprint should inherit.**

- **Re-read the tells against what the pipe does NOW before any re-run.** This capture was blocked for an
  entire sprint because an upstream-client change silently invalidated response-side tells designed
  against the old one. That is a standing hazard, not a one-off: the tells couple to a component that
  other sprints own.
- **Build the three binaries with three SEPARATE `cargo` invocations.** Measured, not theoretical:
  combining them unifies features and produces a different `serve` (`3d0cbaab…` vs the capture's
  `38e14a57…`), because `bench-capture` enables `directpath/bench` and the unified build links the bench
  internals *into the production binary* — precisely what the feature gate exists to prevent. Recorded in
  `docs/proxy/tools/s17-capture.md`.
- **`request-id` is now a real per-call-unique tell** available to any future capture, and stronger than
  anything designed here. It did not exist as an option before Sprint 18 made headers reach the box.
- **The runner lives at `docs/proxy/tools/s17-capture.sh`** with companion notes, no longer in an
  ephemeral scratchpad. Artifacts land outside the repo by design.
- **The paste-and-confirm gate held, and holding it was right.** The production NIC at `0000:61:00.0`
  carries this host's default route and reports the *identical* `0x10fb` device id as both bench cards.
  Neither I nor the lead self-cleared it; it went to the operator. Keep that shape — the cost is one
  message and the failure mode is taking the host off the network, which also severs the path the failure
  would be reported on.

### zero.proxy-lead

**What this sprint actually was, versus what it looked like at hand-off.** The
PM's directive read as "run the capture and close it" — three sentences. What it
required first was reconciling a sprint contract drafted before Sprint 18 against
a codebase Sprint 18 had since rewritten underneath it: the response-side tell's
entire mechanism (a `ureq`-error diagnostic) no longer existed in source, and the
contract still described it as current. Resuming a held sprint is not a resume
button; it is a currency check on the contract itself before anything is routed,
and this one needed a real correction pass, not a footnote.

**Where I drew my own line, and where it moved.** I read source, ran builds and
tests, and edited the sprint contract directly throughout — verification and
process-of-record are mine to do. I did not edit `proxy/src/`, the capture
script, or the evidence artifact's substance; those routed to dev. The one
edit I made to an artifact outside the contract — the bracketed correction on
the archived evidence chain — I made because review named it explicitly as mine
(a record-correction, not a code fix), and I would not have made it unprompted.
Worth stating plainly: I nearly over-claimed credit for that same annotation
before checking whether dev had already added one independently. Catching that
before sending it mattered more than the annotation itself.

**The hardware gate was the cleanest moment in the sprint, and it worked because
nobody tried to be clever about it.** Dev held rather than self-cleared the
paste-and-confirm step; I held rather than clearing it myself, for the same
reason — an error there takes the host off its own network, which is exactly
the class of decision this team's discipline reserves for the actual operator.
Escalating a decision I could have technically approved was not caution for
its own sake: the whole point of routing it up was that neither of us carried
the standing to absorb that specific risk, and naming that plainly was cheaper
than any alternative.

**The s17_head bin-sha consequence is the sprint's sharpest lesson, and it is
about sequencing, not about any single mistake.** A ruling I made (fix the stale
prose, don't re-run hardware for a comment) had a cost neither dev nor I priced
in advance: the fix permanently orphaned one binary's source, because that file
had never been committed. Nobody did anything wrong in isolation — the
contract's own Method already named the dirty-tree risk as an open limit before
the run — but the sprint closed that risk's actual bill only after the fact,
and only because review pushed on exactly this point rather than accepting the
first "it's fine, preserved separately" answer. That is the shape to watch for
going forward: an accepted risk that is still abstract at gate time can become
concrete the moment someone makes an unrelated, individually-correct edit
afterward.

**On the team's own discipline holding under real stakes.** Every gate in this
sprint that mattered — the paste-and-confirm, the coherence-versus-verdict
question, the missing-source disclosure — was resolved by someone naming a
limit precisely rather than rounding it off to "close enough." Dev choosing not
to reconstruct a lost source file when reconstruction was trivially available,
review distinguishing a verified fact from an inferred story about that fact,
and both catching their own errors before I had to — none of that is
guaranteed by the process I run; it is the actual behavior of the team, and it
is worth recording as the reason this sprint's evidence is trustworthy rather
than merely green.

**What the next sprint should inherit, beyond what dev and review already
named.** Commit source before any hardware run, without exception — not a
style preference, a concrete lesson with a concrete cost attached. When a held
sprint resumes after a dependency sprint has landed, budget for a contract
reconciliation pass before routing any execution; assume the contract is stale
until checked, not until proven so. And hold the line that a slice-boundary
ACCEPT is a verdict about a state of the world at a time — a later, individually
correct fix does not retroactively unmake it, but it can still owe the record
an honest note about what changed underneath it.
