# Box ↔ Proxy — Boundary Contract

### The security boundary: per-verb consumer policy over the committed protocol

> **Contract, not spec** — states the proxy's side of the box↔proxy interface: what
> crosses, what the proxy decodes vs relays, and its disposition for every verb, reserved
> field, and failure. Sprint `docs/teams/proxy/003-*.md`, Slice 1; status: **awaiting
> GB1–GB6 gate.**
>
> **Clause tagging (GB1):** every clause is `CITE(owner)` — restating another owner's
> settled artifact — or `DEFINE` — the proxy's own side. Owners: **proto** (the protocol
> effort), **nic** (transport/substrate), **head** (`zero.head-design`), **cloud** (the
> recognizer effort, ledger L4/L5), **proxy** (us).
>
> **Couplings carried, not hidden:** verb ids and error codes are proposed, **unratified**
> (`model.md` §11 Open) — every id below is cited as proposed, and JC-1 (§2) is what makes
> building on them fail-closed meanwhile. The two Sprint-2 specs cited are accepted,
> **not frozen** (ratification packet pending); clauses citing them track any ruling that
> flips a default. Source currency pinned in §7.

---

## 0. Trust-face (GB5) — `CITE(proxy: architecture.md §1–§2; proto: model.md §8)`

This is the **gated security boundary**: every byte arriving here is box-authored,
untrusted, landing on trusted ground. The proxy performs exactly three operations on
box-originated bytes — **recognize** (the net request), **confine-and-write** (the file
destination; the wire carries no offset), and **relay** (everything else, display
included) — with **no neutralization step** anywhere (`model.md` §8 rationale; the
normative no-offset fact per `file.md` §1 and `model.md` §11 Decided).

*Routed finding (not fixed here):* `model.md` §8's rationale blockquote still reads
"confine-and-write (file destination **+ offset**)" — stale commentary against its own
§11 Decided list ("no offset — files are serial") and `file.md` §1. Non-normative per the
doc's reading convention; the fix is **proto**'s, routed via the designer. This contract
derives from the normative text only.

## 1. Layering and routing — `CITE(nic: architecture.md §3, §8; proto: library.md §7)`

1. The transport (L1) demuxes **sessions**: box is sole session-id allocator; a session
   opens by emission (first frame on a fresh id); no cross-epoch state; give-up is silent
   reclaim (`nic/architecture.md` §8).
2. The proto crate (L2) decodes **structure only** — verb, body shape, alphabet — and
   rejects on structural violation with a typed `Reject`. It polices no reserved value
   (the committed convention: reserved-*value* policy is the consumer's — i.e., §2).
3. The proxy **binds a session to a service on its first frame's verb** and thereafter
   routes that session's messages to the bound endpoint: display-service verbs → the
   head relay; file verbs → the file service; net verbs → the net service. The proxy
   **deep-parses only what it acts on** (net + file addressing) **and relays the rest
   inert** (`library.md` §7).
4. Supported-set enforcement, not origin enforcement: a verb "belonging" to another
   direction is just bytes; what protects the trusted side is that the proxy has **no
   code path** for it (`model.md` §7 — "origin is not enforcement; support is").

## 2. Uniform consumer policies — `DEFINE`, each routed (GB6, §5)

- **JC-1 — Reserved disposition, one rule.** On every message the proxy *acts on*
  (decodes): any nonzero reserved surface — the verb `u32`'s upper three bytes, a
  reserved enum value (scheme ≠ 0, per the accepted `trusted-address.md` §4 precedent),
  a reserved flag bit — **denies the whole message**. On messages the proxy *relays*, it
  polices nothing (reserved bits included): relay is inert by construction, and policing
  relayed content would make the proxy an interpreter of the display service — the
  receiving endpoint's consumer policy is its own (head). *This rule is also the
  forward-compatibility floor: whatever ids/flags ratify later, today's proxy denies
  them — fail-closed, no contract rewrite.*
- **JC-2 — Unknown/unsupported verb.** A first frame whose verb binds no proxy service:
  the session never binds; its frames drop; one log line (verb id, session id). A message
  within a bound session whose verb that service does not support: deny the message, log
  the verb id; the session stands (the transport reclaims dead sessions on its own).
- **JC-3 — Reject taxonomy.** (a) **Structural** (proto `Reject`, any session): drop the
  message, log the `Reject` class — no return signal (the protocol's grain is
  uni-directional; no new return verbs). (b) **Policy deny on an acted service**: deny
  plus the service's existing error verb — `file-error` / `net-error` with `code 2`
  (denied) or `6` (malformed) — and the log line. (c) **Relayed sessions**: no proxy
  policy exists to violate; only (a) applies. Nothing is ever rewritten, stripped, or
  fixed up — deny-whole is the only failure shape.
- **JC-4 — Log discipline (the enumeration approach.md §7 promises).** Reaches the log:
  safe segment stacks (file-write destination, the assembled net address), verb ids,
  `Reject` classes, error codes, session ids — all safe-by-construction or numeric.
  Never reaches the log: **unsafe** stacks (file-read sources, list paths — real names,
  display-only; the type split makes this compile-enforced: no `as_str` on
  `UnsafeSegment`), payload bytes, chunk bytes, input text. `CITE(proto: segment.rs,
  file.md §0)` for the type enforcement; the per-verb column in §3 applies it.
- **Completion/timeout pin (file-write).** Completion is **the transport session
  ending** — no close verb, no declared length (`file.md` §1). Abandonment: the
  transport's give-up reclaims the session silently (`nic/architecture.md` §8); the file
  service closes the held fd on its own idle timeout (scalar, provisional) and the
  partial deposit stays as inert bytes (spec residual R-F). No offset, no length, no
  close signal crosses the boundary.

## 3. The verb catalog — per-verb disposition

All ids are the **proposed, unratified** numbers (`model.md` §8 "Verb numbers", §11
Open); body shapes cite `model.md` §6's three decoders. Column "kind" tags the
*disposition* clause; body/structure facts are `CITE(proto)` throughout.

### 3.1 Verbs the proxy acts on (box → proxy) — the gated set

| Verb (id) | Body (decoder) | Proxy disposition | Policy & floor | Reject path | Log | Kind |
|---|---|---|---|---|---|---|
| `file-write` (0x20) | **safe** segment stack (scan) | Bind session to file service; destination → the write gate | Leaf-validator + open-discipline floors, `CITE(proxy: spec/filesystem-write-path.md §1)`; JC-1 on verb bytes | `file-error(2)` + log | destination stack (safe) + verdict | DEFINE |
| `file-chunk` (0x21) | verbatim blob | **First chunk lazily opens the confined fd** (the floor runs here); each chunk appends at the proxy-tracked cursor through the held fd — **no offset exists on the wire** | Sequential append, `CITE(proto: file.md §1)`; the open-discipline floor (`CITE(proxy: spec/filesystem-write-path.md §1)`) runs at the **first-chunk open** (lazy build) — a destination-policy deny surfaces here, not at `file-write` | **Splits JC-3:** confined-open-deny → **(b)** `file-error(2)`; cap-trip → **(b)** `file-error(6)`; I/O failure → **(b)** notify; no-admitted-`file-write` / malformed frame → **(a)** drop+log (silent) | byte count; verdict on a deny | DEFINE |
| `file-read` (0x22) | **unsafe** segment stack | Confine under the readable root; stream back as `file-data` | Read path fails **safe** (wrong file = harmless, `CITE(proxy: filesystem.md §3)`); `file.md` §2's `O_NOFOLLOW` read discipline is the *read* path's own (final-component; its bar is memory-safety + fail-safe, **not** the write floors — see §6 R-3) | `file-error(1/2)` + log (code only) | **source never logged** (unsafe) | DEFINE |
| `list` (0x24, box→proxy) | **unsafe** segment stack | Enumerate under the readable root via native `read_dir`, respond `list-entry` records | `CITE(proto: file.md §3)` — symlinks reported never followed; unsafe names display-only | `file-error(1/2)` + log (code only) | **path never logged** | DEFINE |
| `net-address` (0x30) | `u32` scheme + **safe** stack | Construct `Address`, match the register, hold for the payload | Scheme==0 only (JC-1/JT-2) + byte-exact match, `CITE(proxy: spec/trusted-address.md §2–§5)` | deny → `net-error(2)` + log | assembled address + verdict | DEFINE |
| `net-payload` (0x33) | verbatim blob, spans frames | Reassemble **opaquely** (bounded, reject-before-grow, `REASSEMBLY_CAP` provisional 32 MiB, `CITE(proto: library.md §8, net.md §2)`) → hand off-wire to the recognizer | Recognizer internals are **cloud**'s (ledger L4/L5) — a hole with an owner, not restated here | recognizer deny → `net-error(2)`; cap trip → `net-error(6)`; + log | never logged (payload) | DEFINE / hole: CITE(cloud) |

### 3.2 Verbs the proxy relays (box → head) — the inert set

| Verb (id) | Body | Proxy disposition | Kind |
|---|---|---|---|
| `display` (0x11) | `u32` cell stream | **Relay, opaque.** No decode beyond routing; inert by construction (`CITE(proto: display.md; proxy: terminal.md §2–3)`); reserved bits pass — the head's consumer policy is **head**'s | DEFINE (routing only) |
| `pane-state` (0x18) | 3×`u32` fixed | Relay, opaque — scrollbar extent + input interest; not a trusted surface (`CITE(proto: model.md §8 Input events)`) | DEFINE (routing only) |
| `list-entry` (0x25, box→head) | metadata + **unsafe** name records | Relay when the box answers a head-initiated `list` (head listing the box); unsafe names display-only at the head | DEFINE (routing only) |
| `file-error` (0x27, box-emitted) | shared error body | Relay on the session it answers | DEFINE (routing only) |

### 3.3 Misdirected verbs — head→box ids arriving from the box

`open-session` (0x00), `create-pty` (0x10), `key`/`resize`/`viewport`/`mouse`/`paste`/
`focus` (0x12–0x17), `request-file` (0x26), and any id in the reserved ranges: **no proxy
service supports them**, so JC-2 applies — an unbound first frame never binds; inside a
bound service session they are unsupported-verb denies. On a *relayed* session they pass
to the head, whose unsupported-verb handling is its own (`CITE(proto: model.md §7)` —
support is the boundary, at every receiver). Kind: DEFINE (the proxy's supported sets are
exactly §3.1 + the §3.2 routing set).

### 3.4 Proxy-authored verbs (proxy → box) — the free direction

`file-data` (0x23), `list-entry` (0x25, host listing), `net-response` (0x31), `net-error`
(0x32), `file-error` (0x27, proxy-emitted): proxy-authored or upstream-relayed bytes
landing on **untrusted ground** — the safe direction, relayed/streamed verbatim, no gate
(`CITE(proxy: cloud/approach.md §5; proto: net.md §3, file.md §2–3)`). `net-response` is
streamed frame-by-frame, never reassembled by the proxy (`CITE(proto: net.md §3)`).
Error `detail` text is rendered inert at its receiver (`CITE(proto: model.md §8 error
body)`). Kind: CITE.

## 4. What this contract does not own — holes with owners

- **The recognizer** (net-payload's consumer): placement and exact recognized shape —
  **cloud** effort, ledger L4/L5. The contract stops at "hand the reassembled bytes
  off-wire."
- **Verb-id / error-code / flag-bit ratification** — **proto** (`model.md` §11 Open).
  JC-1 is the standing mitigation.
- **Head-side consumer policy** for relayed and misdirected verbs — **head**.
- **`REASSEMBLY_CAP`'s value** — **proto** (provisional 32 MiB, flagged for a
  capacity/security review, `library.md` §8).

## 5. Judgment calls — routed, not settled (GB6)

| Call | Recommended default | Owner |
|---|---|---|
| JC-1 | Uniform reserved-deny on acted messages; relayed messages unpoliced | design + policy |
| JC-2 | Unknown verb: unbound session never binds (drop + one log line); in-session unsupported verb: deny message, session stands | design |
| JC-3 | Reject taxonomy as §2: structural = drop+log; policy = deny + existing error verb; no new return verbs | design |
| JC-4 | Log enumeration as §2/§3 columns | design |
| Scalars | file-service idle timeout; per-service log-rate bound | provisional, marked |

## 6. Residuals — named and bounded

- **R-1 — Unratified ids under the contract.** Every id cited is proposed. Bound: JC-1 —
  a later ratification that changes or adds ids cannot widen today's proxy, which denies
  what it does not support; the contract's table updates by deliberate revision.
- **R-2 — Pending-ratification coupling.** Sprint-2 spec citations (§3.1) track the
  ratification packet; a flipped default (e.g. S1-J4 hardening) changes the cited floor,
  not this contract's structure.
- **R-3 — The read path's thinner floor.** `file.md` §2 names `O_NOFOLLOW` for
  `file-read` — final-component-only. That is the read path's stated discipline, and its
  failure mode is *safe* (the box reads some other readable file; disclosure undefended)
  — the write-path floors deliberately do not apply. Stated so no reader credits the read
  path with the write path's guarantees. `CITE(proxy: filesystem.md §3; proto: file.md §2)`.
- **R-4 — Relay passes reserved bits.** By JC-1's split, box→head relayed content is
  unpoliced by the proxy; the head's parser meets the tier-2 bar and its consumer policy
  is head's own. The alternative (proxy policing display internals) would re-import
  interpretation into the relay — refused by design.
- **R-5 — `model.md` §8 commentary staleness** ("+ offset") — routed to proto via the
  designer; normative text is consistent; no contract clause derives from the stale
  phrase.

## 7. Currency pins (2026-07-18)

Committed: `proto/src/segment.rs` @ `a7ccd6c`; `docs/proxy/architecture.md`
**re-pinned 2026-07-25** `dec2453` → `e22f7f1` (L14/L15 resolved) — the identity moved
across four commits (Sprints 4, 5, 6, 16); the **L14 and L15 rows this contract cites are
byte-unchanged** across all four, verified row-by-row rather than assumed, so nothing
cited here drifted; `docs/proxy/filesystem.md` §4 no-offset fix @ `7f7de0b`;
`docs/proxy/terminal.md` §5 @ `da6c63c`; Sprint-2 specs @ `c4690bf` (+ the authorized
offset-wording amendment, working tree, `filesystem-write-path.md` sha-256
`940ee9de64bc35cf…`). Working-tree (uncommitted), sha-256 prefixes: `model.md`
`4a468afae792e6e3`, `file.md` `5461602d24cf8b4e`, `net.md` `02c8ca7a58202bda`,
`library.md` `c8cb29b87665e60e`. **`trusted-address.md` re-pinned 2026-07-25** to
`0a1d8f53e12e761f` (was `72478d48cb11a980`) — that spec was corrected during the Sprint 20
audit, which staled this pin by construction: correcting a document is exactly the act that
invalidates every currency pin to it. Re-pin at the gate.
