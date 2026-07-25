# Sprint 018 — The Upstream Client Becomes a Dumb, Fixed, Verbatim Pipe

> Proxy team, sprint 18. Closes three pre-existing defects in Sprint 11's
> upstream client — redirect-following (which forwards the operator's
> credential to an unreviewed destination), ambient environment-variable
> destination override, and silent response decompression — all found
> while investigating Sprint 17's live-capture wire-protocol requirement.
> The designer has ruled the approach: the upstream connection is a fixed,
> explicit, verbatim pipe to exactly one reviewed register destination;
> general-purpose HTTP client behavior beyond that is forbidden by
> construction, not by configuration. Drafted by zero.proxy-lead from
> planning input by zero.proxy-dev, following the PM/designer's ruling
> (2026-07-24).

---

## Scope Boundary

**"Hand-rolled" names only the trivial HTTP above TLS — never TLS
itself.** Stated precisely so the phrase cannot drift into the wrong
reading (reviewer's distinction, load-bearing for everything below):
hand-rolling the handshake, record layer, certificate parsing, or crypto
is a **reject on sight**, with no acceptance bar to weigh it against —
the existence of a bar would only invite the attempt. What this sprint
actually builds is: use **`rustls` directly** (a dependency already
present, reached via `ureq` today — no new supply chain, only a direct
dependency edge on a crate already in the binary) for the TLS connection
itself, and hand-roll only the request write and the raw, close-delimited
response read above it.

This sprint owns replacing `upstream.rs`'s `ureq`-based upstream call with
that pipe:

1. Sends the tier-1-admitted request bytes to the one hardcoded, register-
   approved endpoint over a genuine TLS connection (via `rustls` directly).
2. Reads the response **raw, to EOF, close-delimited** — no
   `Content-Length`/chunked-framing parsing, so there is nothing to
   accidentally isolate or transform.
3. Relays those raw bytes (status line, headers, body — the full response
   as the upstream sent it) to the box **verbatim** through the existing
   `net-response` channel, unmodified.
4. Structurally cannot follow a redirect, honor an ambient proxy
   environment variable, or auto-decompress — because none of that
   machinery exists in the replacement, not because it is configured off.

It explicitly does **not** own:

- **Sprint 17's live-capture redesign.** That sprint is **blocked** on
  this one, not merely paused: every response-side tell depends on what
  actually reaches the box, which changes once headers are relayed
  (Anthropic's own `request-id` header becomes reachable — a stronger,
  per-call-unique tell than anything designed so far). Do not re-run
  Sprint 17's capture against the old pipe while this sprint is open.
- **Any change to `bring_up`, `assemble`, the authority ladder, or any
  Sprint-16 DSL entry.** This sprint touches `upstream.rs` only.
- **A transport-layer tee of `ureq`'s internals.** Considered and
  rejected during planning: `ureq`'s `unversioned::transport` module is
  explicitly documented as not semver-stable (breaking changes land in
  minor bumps — a poor foundation for a tier-relevant relay), and a tee
  would only fix the verbatim-body question, leaving redirect-following
  and decompression fully intact underneath. "Off by construction" is
  the ruling; a tee gives "off by configuration" at best, and here it
  would not even give that for two of the three defects.
- **Reconstructing a status line or header block from parsed parts.** A
  named reject, not merely a style preference: rebuilding a status line
  and headers from `ureq`'s (or anything else's) already-parsed
  `Response` object and prepending it to the body is **re-serialization,
  not verbatim** — it silently normalizes header casing, ordering,
  duplicate headers, obs-folding, the reason phrase, and the HTTP
  version, and no test asserting "a status line was relayed" would catch
  it. The bytes must be genuinely raw, read directly off the TLS stream,
  never rebuilt from structured parts.
- **Re-certifying `rustls` itself.** `architecture.md` L10 (`zero.policy`,
  2026-07-18) already places the upstream TLS stack **outside** the
  box-reachable surface and outside the §15/§16 reviewability budget —
  its parser sees only the cloud's records, adversary class *upstream*.
  L10 covers the library's internals; it does not cover **our
  configuration of it**, which is the only thing that changes when `ureq`
  is replaced with `rustls` used directly. This sprint's TLS-adjacent bar
  targets exactly that configuration shift — never a re-audit of
  `rustls`'s own correctness, which would be scope creep in the wrong
  direction and duplicate L10's ruling.

## Problem

Investigating Sprint 17's raw-response-relay requirement surfaced three
independent, pre-existing defects in `upstream.rs` (Sprint 11, already
shipped and accepted), each an inherited default of the general-purpose
`ureq` HTTP client that nobody explicitly chose:

1. **Redirect-following, and the credential rides along.** `ureq`'s
   default is `max_redirects: 10`, never overridden. Traced what survives
   a redirect (`ureq-proto`'s redirect handling): with
   `RedirectAuthHeaders::Never`, `ureq` strips exactly `Authorization`,
   `Cookie`, and `Content-Length` — **not** custom headers. The proxy's
   credential rides `x-api-key`, a custom header, so it is **forwarded to
   the redirect target**. A 301/302 from the endpoint would send the
   box's admitted request *and* the operator's credential to a
   destination the register never approved — directly contradicting two
   documented tier-1 properties (`cloud/approach.md` §6, "no
   redirect-following"; `architecture.md`'s route-matcher row, "never
   widened at runtime"). Compounded by `https_only` also defaulting
   false, so a redirect to plain HTTP is not refused either. Anthropic
   does not redirect today — "the upstream currently doesn't do the
   thing" is not the property the design claims to hold.
2. **The destination is silently overridable by ambient environment
   variables.** `ureq`'s default (`Proxy::try_from_env()`) reads
   `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` and routes through whatever
   they name, carrying the credential. Not box-reachable, but an
   implicit, mutable, invisible-at-the-call-site input, exactly the
   class of thing Sprint 16 already rejected for filesystem-root paths
   (an ambient, mutable resolution base unlike every other explicit
   authority in this design).
3. **The response is already not verbatim.** `ureq`'s default features
   (`["rustls", "gzip"]`) mean it advertises `accept-encoding: gzip` and
   transparently decompresses the response before the proxy's code ever
   sees it — a real transformation of what the server sent, violating
   `cloud/approach.md` §5's "never modifies" independent of the
   status/header question Sprint 17 started with.

**Severity, per the designer's ruling**: defects 1 and 2 are
effect-invariant violations — the box (via a redirect) or the credential
(via either path) reaches or is exposed to an unreviewed destination.
Defect 3 is a posture/trusted-surface violation — the proxy grows surface
to defend a direction (ingress) that needs no defense.

**Root cause, per the designer**: the upstream connection must be a
*fixed, explicit, verbatim* pipe to exactly one reviewed register
destination. General-purpose HTTP client defaults that add behavior
beyond that are forbidden by the committed design, not merely
undesirable.

## Goal

Replace `upstream.rs`'s `ureq`-based call with a pipe built on `rustls`
used directly (never a hand-rolled TLS layer) that makes all three
defects structurally absent — no redirect logic exists to
follow, no environment-proxy logic exists to honor, no decompressor
exists to transform — and relays the real, raw upstream response
(status, headers, body) to the box exactly as the server sent it,
unblocking Sprint 17's live-capture redesign.

## Final State

1. **`upstream.rs`'s upstream call no longer uses `ureq`.** The 56 lines
   it replaces (`upstream_call` + `map_ureq_error`) are removed; the
   final line count of the replacement is recorded against that number
   as a real measurement, not an impression — the honest, open question
   from planning (a hand-rolled version's size relative to what it
   replaces was never assumed, only estimated as "likely smaller").
   **Checkpoint, not a foregone conclusion**: if the replacement needs
   substantially more trusted-side code than expected, that is itself a
   finding to surface at the Slice 2 gate, not to push past because the
   approach was already committed to.
2. **A genuine TLS connection, via `rustls` used directly** (already a
   linked dependency, promoted to a direct one — no new supply chain),
   replaces `ureq` for this one call site. **Two bars, not one, because
   a static audit alone cannot close this** (reviewer's finding, load-
   bearing): a static configuration audit is *necessary* — verified
   against the specific pinned `rustls` version's actual surface, not a
   remembered API name: verification left at the library's **default
   trust path** (no custom `ServerCertVerifier`); the library's
   explicitly-named dangerous accessor (whatever permits bypassing
   verification, e.g. a "dangerous configuration" API) is **not used**,
   and its gating feature is **not enabled**; hostname verification is
   **active**; the root store's provenance is named and non-empty. This
   is a **bypass audit**, the same shape as Sprint 5's DSL bypass audit
   (enumerate every route the guarantee could be circumvented through,
   show each absent or unreachable) recombined with Sprint 15's
   capability-narrowing (the dangerous path gated behind a named,
   feature-gated accessor is what makes this structural, not a read of
   current behavior) — **not** the crossing-soundness reasoning-artifact
   discipline used for the NIC DMA-ordering account, which was for
   certifying the team's *own* reasoning where no library owned the
   guarantee; here `rustls` owns the guarantee by L10's ruling, and what
   must be shown is that the configuration does not *defeat* it.
   **Sharpened again during planning: with the machinery absent rather
   than merely configured off, the audit collapses to the stronger,
   shorter claim "the machinery does not exist"** — the earlier
   "configuration cannot be defeated" framing was for machinery that is
   still present and disabled; here there is nothing to defeat.
   **Two items verified missing from the audit during planning, both
   folded in**: (a) **the crypto provider** — `rustls`'s
   `ClientConfig::builder()` resolves the *process-default*
   `CryptoProvider` and **panics** if one cannot be resolved
   (`client_conn.rs`, confirmed against the pinned 0.23.42), and that
   default exists today only because `ureq` arranges it; removing `ureq`
   removes the thing that currently makes resolution succeed. The
   provider — the code that actually performs the handshake and
   verification — must be **selected explicitly**
   (`builder_with_provider`, or exactly one provider feature enabled),
   never left to process-default resolution, which is the same
   inherited-default class as defects 1–3. (b) **the root store is a
   deliberate, named choice, not whichever one gets wired**: compiled-in
   roots (`webpki-roots`) are pinned at build time and reviewed like
   every other compiled-in authority in this design (the register, the
   filesystem roots, the credential descriptor); the platform trust
   store is **ambient authority** — an administrator-added CA silently
   widens who may impersonate the upstream, invisible at the call site.
   `webpki-roots` is the choice, stated with this reason, not left
   implicit.
   **The `Cargo.toml` spec itself must be written precisely, or "no new
   supply chain" becomes false** (dev's finding, verified): `rustls`
   0.23's `default` features include `aws_lc_rs` (pulling `aws-lc-rs`
   *and* `aws-lc-sys`, a C/NASM build dependency) and `tls12`. The
   crate's own naive form (`rustls = "0.23"`) would therefore add a
   *second* crypto stack rather than reuse the one already linked
   (confirmed: this tree's `Cargo.lock` carries `ring`, not `aws-lc`,
   because `ureq`'s own default enables `rustls` via its `_ring`
   feature). The correct spec is `default-features = false` with `ring`
   explicitly enabled — the same explicit-selection requirement above,
   arriving from the dependency declaration rather than the API call.
   **TLS 1.2 support is dropped, deliberately and verified safe**: not
   enabling the opt-in `tls12` feature yields a TLS-1.3-only client for
   free. Verified directly against the real endpoint before pinning this
   (not assumed): `api.anthropic.com:443` negotiates TLS 1.3 cleanly
   (`openssl s_client -tls1_3`, confirmed connection and cipher
   negotiation). But the static audit is **not sufficient alone**: a
   disabled or misconfigured verifier connects perfectly, streams a
   response, and passes every tell — a successful round trip is *zero*
   evidence verification is on, the sharpest instance of
   green-is-not-acceptance in this project. The audit is closed by the
   **negative control** (Final State #9), which a run — not a read — is
   the only thing that can supply.
3. **The request is sent, and the response read, close-delimited to
   EOF.** **Framing asymmetry, stated precisely so it cannot be
   misread** (dev's finding): this sprint frames what it *emits* (the
   proxy's own request necessarily carries a `Content-Length`, since the
   proxy is the author and knows the length) but never parses what it
   *receives* — `Connection: close`, read to EOF, no
   `Content-Length`/chunked-framing logic applied to the *response*, so
   there is nothing to parse there and therefore nothing to accidentally
   isolate or transform. A reader (or a reviewer at the gate) seeing a
   `Content-Length` anywhere in the diff must not mistake it for a
   contradiction — it belongs to the direction this sprint frames, not
   the direction it forbids parsing. This is `proto/net.md` §3's own
   framing made literal for the response: "the stream ends when the
   proxy stops emitting — a transport event, not a protocol close." Fits
   the existing serial-bound-of-1, one-call-per-connection shape rather
   than fighting it; connection reuse is no loss here.
   **The connect/read/write timeouts `ureq`'s `timeout_global`
   (`UPSTREAM_TIMEOUT`, 120s) provided must be explicitly preserved on
   the raw socket** (dev's finding): Sprint 11 chose that bound
   deliberately so a hung upstream cannot leak a worker thread forever;
   a hand-rolled socket has no timeout unless one is set explicitly, and
   "replace 56 lines" is exactly the shape of change that silently drops
   a behavior nobody re-tests. Named here as a must-preserve, not left
   to be remembered.
4. **The response relays to the box genuinely raw** — status line,
   headers, body, exactly as received, streamed through the existing
   `net-response` channel unmodified. Proven **byte-identical** against
   what a controlled local server actually sent (Final State #7's
   infrastructure), not merely "looks like a response was relayed."
   Nothing in the code path constructs, reconstructs, or re-serializes a
   status line or header block from parsed parts — there is no parsing
   step to have parsed them from.
5. **A panicked upstream worker is never presented to the box as a
   completed response** (reviewer's finding, folded in directly — small
   and contained to `upstream.rs`, not a scope question). Verified: on
   the worker channel disconnecting, `UpstreamIngress::drain` calls
   `dispatcher.close_reply(session)` — a clean end-of-stream telling the
   box the response finished normally — *before* checking whether the
   join actually succeeded, then discards the result (`let _ =
   join.join()`). A worker that panics drops its `Sender`, the channel
   disconnects the same way a normal completion would, and the panic
   payload is silently swallowed, never logged. **This sprint makes it
   matter, not just theoretical**: the explicit crypto-provider selection
   (Final State #2) removes one *known* panic site (`rustls` panicking
   on an unresolvable provider) but the swallow-all-panics behavior
   remains for any other panic in that worker — Sprint 18 knowingly adds
   a documented panic path into a spot that hides all of them. The
   consequence is also worse under the new pipe: a truncated response
   under the old body-only design was a truncated JSON payload; under
   the raw pipe it could be a truncated **status line or header block**,
   so the box would parse a partial HTTP response rather than a partial
   payload. **`proto/net.md` §4 already settles the correct behavior**:
   "any outcome that is not an upstream response... is one `net-error`"
   — a crashed worker producing a partial or empty stream is precisely
   *not* a response, so presenting it as a completed one is the same
   class of spec violation this sprint already corrects, running in the
   opposite direction (a real response wrongly delivered as an error,
   vs. a non-response wrongly delivered as a complete response — both
   break the §3/§4 split). Fix: check the join result instead of
   discarding it; on `Err`, log it and enqueue a `net-error` rather than
   a clean close, so the box learns the outcome was not a response.
   **Scoped narrowly, on purpose**: this item is the one known,
   Sprint-18-caused panic site's consequence; a general audit of what
   else in the worker could panic is a separate, later concern, not this
   sprint's to widen into.
6. **Verbatim proven in the REQUEST direction too, not just the
   response** — found independently by both dev and reviewer, converging
   on the same gap from different angles: `ureq` injects headers the
   proxy never authored (`user_agent`, `accept`, and `accept_encoding`
   all default to `AutoHeaderValue::Default`, confirmed in `config.rs`;
   `accept_encoding`'s default is already named as half of defect 3, but
   `user-agent`/`accept` are the same class and were unmentioned). This
   matters more than it looks: `cloud/approach.md` §5 makes **egress the
   sensitive surface**; U4's recognizer vets the request *including*
   headers (its `betas` field); and the credential is the **one**
   authorized post-vet addition (`architecture.md` §8) — any other
   injected header is unvetted content on the wire, bind-the-chain-
   shaped, not cosmetic. The replacement removes this for free (the
   proxy constructs the request itself), which is exactly why it is an
   **explicit acceptance property**, not an incidental consequence:
   Slice 1's local server captures the request it received and asserts
   the wire bytes are **precisely the admitted body plus the credential
   header, nothing else** — verbatim proven in both directions, response
   by raw read-to-EOF, request by capture-and-compare, no hardware and
   no network needed for either.
7. **A local TLS test server, with a certificate the test suite
   controls**, built once (Slice 1) and reused by Slice 2's behavioral
   proofs. Exercises the certificate-verification negative arms
   (Final State #9) and, on top, the three defects' behavioral absence,
   the request-side verbatim proof (Final State #6), and the Host-header
   binding (Final State #10) — cheap because the same infrastructure
   serves every slice's gate.
8. **All three defects proven absent, behaviorally, not just by
   construction** — the local server makes each one directly testable:
   serve a `302` and assert the box receives the `302` **verbatim**
   rather than the proxy chasing it; set `HTTP_PROXY` to a black hole and
   assert the connection still goes direct to the real target; serve a
   `gzip`-encoded body and assert the box receives the **compressed**
   bytes, not a decompressed transformation. Each is a positive
   demonstration that the removed machinery's absence is real, not an
   assumption resting on "we deleted the code that would have done it."
9. **Certificate verification's negative arms, each proven refused —
   the actual demonstration static analysis cannot provide**: an
   **untrusted/self-signed certificate**, a **valid certificate presented
   for the wrong hostname**, and an **expired certificate** must each
   cause the connection to be **refused**. Three arms, not two — added
   during planning: time-validity is a distinct path in `rustls` from
   trust-chain and hostname verification by the same argument that
   separated those two, and the local-server infrastructure already
   generates certificates, so the marginal cost of a third is near zero.
   A configuration can get any one of the three right while getting
   another wrong. Built against a **locally generated certificate and a
   local TLS listener** — no external bad-cert service, no network
   dependency, fully reproducible, no bench hardware needed. Symmetric
   with Sprint 16's bad-root discipline in the opposite orientation:
   there, a success arm was required alongside a failure enumeration;
   here, the real connection *is* the success arm, and these are the
   failure arms — all required for the same reason. A successful-
   connection test alone proves nothing about verification; only the
   refusal arms do.
10. **The vetted destination binds THREE things, not two — the TLS peer,
   the SNI, and the HTTP `Host` header** (reviewer's finding, sharpened
   during planning: a third leg was missing from the first draft, which
   bound only the TLS peer to the vetted destination). HTTP/1.1
   requires a `Host` header, and `Host` determines which virtual host
   the request actually *addresses* — so a correctly-verified TLS peer
   with a `Host` naming somewhere else is a request addressed off-
   register over a properly-verified connection. **Newly load-bearing
   precisely because of this sprint**: `ureq` set `Host` from the URL,
   correct by construction and never a choice; hand-rolling the request
   write makes `Host` something someone must set, and per `proto/net.md`
   §2 the box supplies "query, headers, body" — so `Host` is potentially
   **box-influenced**. The dangerous case is a box-supplied `Host`
   differing from the vetted destination and silently honored (a
   missing `Host` is comparatively safe — HTTP/1.1 makes it a protocol
   error, failing noisily). `Host` is therefore **governed by the
   register entry**, never by anything the box sent, and proven on the
   same local server as Final State #6: a request whose box-authored
   headers name a different host still reaches the wire with the vetted
   `Host`. The TLS peer, the SNI, and the `Host` header are checked as
   **one property** — the vetted destination — not three facts assumed
   to agree because two of them were separately proven.
11. **The credential-echo gate (Sprint 17's contract) is extended to
    cover the header block, not just the body** — with headers now
    relayed, header-echoing (by servers or intermediaries) is a more
    common risk class than body-echoing, so the residual Sprint 17
    named grows under this fix, not shrinks. Recorded here since it
    becomes checkable once this sprint's pipe exists, even though the
    actual capture (Sprint 17) is not this sprint's to run.
12. The sprint document carries role retrospectives; the sprint is
    committed with only its intentional files.

## In Scope — ordered slices

**Slice 1 — the TLS transport and certificate verification, alone.**
`rustls` used directly for the connection, with an explicitly-selected
crypto provider and `webpki-roots` as the named root store (Final State
#2); the close-delimited, read-to-EOF request/response mechanics and the
preserved connect/read/write timeouts (Final State #3); the local TLS
test server with a locally-generated certificate (Final State #7). Given
its own gate rather than reviewed as incidental scaffolding inside
"build the pipe," because this is where getting it wrong silently
weakens TLS. Gate, two bars: the static bypass audit (Final State #2 —
default trust path, no custom `ServerCertVerifier`, the dangerous
accessor unused and its feature disabled, hostname verification active,
root-store provenance named, crypto provider explicitly selected) **and**
the negative-control run (Final State #9 — untrusted-cert, wrong-
hostname, and expired-cert each refused, a valid cert proven to
connect) — the audit alone cannot close this, since a disabled verifier
passes every positive test.

**Slice 2 — the verbatim pipe on top (both directions), `ureq` removed, the panic fix, and the peer-binding join.**
The raw-byte relay through the existing `net-response` path (Final State
#4); the panicked-worker fix, so a crashed worker is never presented as a
completed response (Final State #5); the request-side verbatim proof —
the wire request is precisely the admitted body plus the credential
header, nothing else (Final State #6); `ureq` removed entirely from
`upstream.rs`, replaced by the Slice-1 transport, with the final line
count measured against the 56 lines removed (Final State #1); the three
defects proven behaviorally absent using Slice 1's local server (Final
State #8); the TLS peer, SNI, and HTTP `Host` header all proven bound to
the vetted destination as one property, not provable from Slice 1 and
the defect-absence proofs separately (Final State #10); the credential-
echo gate extended to headers (Final State #11). `ureq` removal is
folded into this slice rather than split out — it is mechanical once the
pipe works, and splitting it would leave a slice boundary where the
crate has two upstream paths open at once. Gate: verbatim relay proven
byte-identical in **both** directions; the "no reconstruction" reject
enforced structurally (nothing in the code path parses a status line or
header block at all); a panicked worker proven to surface as a
`net-error`, never a clean close; all three defects' absence
behaviorally demonstrated; the vetted-destination-is-the-TLS-peer-and-
SNI-and-`Host` join proven as one property; `ureq` fully removed from
the crate's dependency tree for this call site.

## Non-Goals

- Do not fix the three defects by re-configuring `ureq` (disabling
  redirects, disabling env-proxy resolution, disabling gzip) — the
  ruling requires the machinery **absent**, not disabled; "unrepresentable
  beats unused," the same discipline already applied at Sprint 15's
  confused-deputy floor and Sprint 16's self-taking builders.
- Do not use `ureq`'s `unversioned::transport` tee — explicitly
  documented as not semver-stable, and it would not fix defects 1 or 3
  regardless.
- Do not reconstruct a status line or header block from any parsed
  response representation, however faithful the reconstruction looks —
  genuinely raw bytes only.
- Do not extend this sprint's TLS-verification rigor to the
  Tier-1/Tier-2 adversarial-closure bar — the upstream leg is the
  **upstream** adversary class (`architecture.md` L10), off the
  box-byte axis and outside the reviewability budget; this sprint's bar
  is *demonstrable correctness*, via the negative-arm proofs, not a
  security-invariant-core certification.
- Do not re-run or redesign Sprint 17's live capture as part of this
  sprint — Sprint 17 is blocked on this sprint's output, not folded into
  it; its redesign (the response-side tell now resting on the real
  `request-id` header) is Sprint 17's to finish once this closes.
- Do not treat "connects successfully" as evidence of correct certificate
  verification — only the negative (refusal) arms establish that.
- **Do not attempt Final State #10's peer-binding join without its own
  prerequisite closing first.** Found while sizing Slice 1 (dev,
  verified): the destination the proxy actually dials
  (`AnthropicClient::endpoint`, set from a `serve.rs` constant) is
  today *independent* of the vetted `Route` the register admits against
  — `UpstreamSpawn::spawn`'s signature carries no destination parameter,
  and the per-session stored `Route` is discarded after the U4
  recognizer vet, never reaching the dial. The two strings match today
  only by coincidence of authorship, not by construction — with three
  registered routes, a request matching a *different* one would still
  dial Anthropic. This is a genuine fourth defect, the same class as
  the other three (a documented tier-1 property — "the register fixes
  the destination" — with nothing enforcing it), but its fix (threading
  the matched `Route`'s destination from `Dispatcher` through
  `UpstreamIngress::on_admit` into `UpstreamSpawn::spawn`, removing
  `AnthropicClient::endpoint`/`ANTHROPIC_ENDPOINT` as independent
  inputs) touches `dispatch.rs` and `serve.rs`, outside this sprint's
  own Scope Boundary. **Ruled by the PM (2026-07-24): split as its own
  prerequisite sprint, sequenced between Slice 1 and Slice 2.** Slice 1
  (TLS transport, certificate verification) does not depend on it and
  proceeds now; Slice 2's join proof (Final State #10) is gated on the
  prerequisite sprint closing first — building Slice 2 before it would
  produce a well-verified TLS connection to a destination chosen by an
  unrelated constant, not the property the join item claims to prove.

## Starting Surface

Audit first, in this order:

1. `proxy/src/upstream.rs`'s `upstream_call`/`map_ureq_error` — the 56
   lines this sprint replaces, and the exact behavior (redirect-following,
   env-proxy, gzip) each defect traces to.
2. `ureq`'s `Config::default()` (`config.rs`) and `run.rs`'s status-error
   gate — the exact defaults this sprint removes the *machinery* for,
   not merely configures off.
3. `ureq-proto`'s redirect handling (`redirect.rs`) — the exact header
   set `RedirectAuthHeaders::Never` strips (`Authorization`, `Cookie`,
   `Content-Length` — not custom headers), confirming why the credential
   rides along today.
4. `cloud/approach.md` §5 (verbatim/opaque relay) and §6 (no
   redirect-following, no runtime-widened destination); `proto/net.md`
   §3 (verbatim response, transport-event-delimited) and §4 (net-error's
   normative scope — "any outcome that is not an upstream response");
   `architecture.md`'s route-matcher tier-1 row and L10 (upstream
   adversary-class attribution).
5. Existing `Cargo.lock` entries for `rustls`, `webpki-roots`, `ring` —
   confirming these are already linked (via `ureq`) before this sprint
   promotes them to direct dependencies.

## Method

- **"Off" means absent, not disabled.** The recurring standard this
  sprint applies from two prior sprints: capability-narrowing over
  behavioral avoidance (Sprint 15's confused-deputy floor; Sprint 16's
  self-taking builders removing the mutual-exclusion hazard
  structurally). A configuration pin is exactly the shape of thing that
  already failed here — every one of the three defects is an inherited
  default nobody chose.
- **Success-arm/failure-arm discipline applies to certificate
  verification, in the opposite orientation from Sprint 16's bad-root
  enumeration.** There, a success arm was required alongside a failure
  enumeration; here, the real connection *is* the success arm, and the
  three negative arms (untrusted-cert, wrong-hostname, expired-cert, each
  refused) are what must be added — all four required for the same
  reason: an always-refuses (or always-accepts) verifier would pass an
  incomplete test trivially.
- **`ureq` was getting some things right too — the acceptance bar
  enumerates what must not be *lost*, not only what must be fixed**
  (reviewer's framing). This sprint moves several properties from
  "correct by `ureq`'s construction" to "correct by our construction":
  `Host` set from the URL (now Final State #10's job), request framing
  and timeouts (Final State #3), no unauthorized header additions beyond
  `ureq`'s own auto-headers (Final State #6). The three defects
  (Problem, above) are what prompted this sprint; the inherited
  correctness is what the replacement must demonstrably keep, named
  explicitly rather than
  assumed to survive a rewrite.
- **A static audit is necessary, never sufficient, for the TLS
  configuration shift.** The bypass-audit checklist (Final State #2) is
  required and structural, but a disabled or misconfigured verifier
  reads clean on the checklist *and* connects successfully — only the
  negative-control run (Final State #9) can show verification is
  actually enforced. Green (a working round trip) is not acceptance
  here in its sharpest form in this project: a successful call is zero
  evidence verification is on.
- **The raw-relay proof is byte-identical, not "looks relayed."**
  Compare against a controlled local server's actual sent bytes, not
  against an expectation of what a response "should" look like.
- **Reviewability budget, not containment, is this sprint's bar for the
  TLS leg — and it targets the configuration, not the library.** L10
  already places `rustls`'s own internals outside the box-reachable
  surface and the review budget; what this sprint's bar covers is
  strictly *our configuration* of it (the bypass audit), never a
  re-certification of `rustls` itself.
- **The vetted destination and the TLS peer are proven as one property,
  not two.** Certificate verification being correct and the redirect/
  env-proxy defects being structurally absent are each necessary but
  do not, added together, prove the join — that verification is checked
  against the *same host* U3 vetted, unchangeable by anything reached
  after vetting.
- **Measure, do not presume, the surface-size direction.** Planning's
  own instinct (a pipe built directly on `rustls` is likely *smaller*
  than a general-purpose client's redirect/proxy/cookie/decompression
  machinery) is a lean, not a given — Final State #1's line-count
  measurement is the actual answer, and a larger-than-expected result is
  a finding to report, not a reason to route around the gate.

## Acceptance

- REJECT any fix implemented as `ureq` configuration (disabled
  redirects/env-proxy/gzip) rather than the machinery's structural
  absence.
- REJECT a certificate-verification claim resting only on a successful
  connection, or only on the static bypass audit — both the audit
  (Final State #2) and the negative-control run (Final State #9,
  untrusted-cert, wrong-hostname, and expired-cert each refused) are
  required; neither alone closes it.
- REJECT process-default `CryptoProvider` resolution, or a root store
  left as "whichever one gets wired" rather than the named, deliberate
  `webpki-roots` choice (Final State #2).
- REJECT any reconstruction of a status line or header block from a
  parsed response representation, however faithful it looks.
- REJECT a worker panic (or any join failure) surfacing as a clean
  response completion rather than a `net-error` (Final State #5) — the
  box must never read a crashed call as a successful one.
- REJECT claiming verbatim relay proven in only one direction — both the
  response (Final State #4) and the request (Final State #6) need the
  byte-identical proof; the credential header is the *only* authorized
  addition to what the box admitted.
- REJECT a hand-rolled request/response path that drops `ureq`'s
  connect/read/write timeout behavior (Final State #3) — a hung upstream
  must not be able to leak a worker thread forever, same as before this
  sprint.
- REJECT claiming a defect's absence without the local-server behavioral
  proof (Final State #8) — construction alone is necessary, not
  sufficient, given this sprint's own root-cause finding that
  configuration-only fixes are exactly what failed before.
- REJECT extending this sprint's rigor to a re-certification of
  `rustls` itself — L10 already covers the library's internals; this
  sprint's bar is the configuration shift only.
- REJECT proving certificate verification and the redirect/env-proxy
  defects' absence as separate facts without also proving the join
  (Final State #10) — that the TLS peer, the SNI, *and* the HTTP `Host`
  header are all bound to the vetted destination specifically,
  unchangeable by anything reached after vetting or supplied by the box.
- REJECT treating Sprint 17's capture as re-runnable against the old
  pipe while this sprint is open, or as automatically resolved once this
  sprint closes — its tell redesign is separate, follow-on work.
- REJECT closing this sprint without the credential-echo gate extended
  to cover headers (Final State #11).

## Review Questions

None outstanding at contract time. The certificate-verification
acceptance bar was resolved during planning (reviewer): not the
crossing-soundness reasoning-artifact discipline (that precedent is for
certifying the team's own reasoning where no library owns the guarantee;
here `rustls` owns it by L10's ruling), but a bypass audit (Sprint 5's
DSL bypass-audit shape) combined with capability-narrowing (Sprint 15)
and the confined-unsafe pattern, closed by a negative-control run since
no static audit alone can demonstrate a verifier is actually enforcing.
The final line count vs. the 56 lines replaced (Final State #1) is open
by design, not a blocking question — the Slice 2 gate reports the actual
number, not an estimate.
