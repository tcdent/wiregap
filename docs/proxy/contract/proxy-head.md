# Proxy ↔ Head — Boundary Contract

### The trusted→trusted relay: what the proxy decodes, what it relays opaquely

> **Contract, not spec** — states the proxy's side of the proxy↔head interface. Sprint
> `docs/teams/proxy/003-*.md`, Slice 2; status: **awaiting GB1–GB6 gate.** Clause tagging
> per `contract/box-proxy.md`: `CITE(owner)` / `DEFINE`; owners: **proto**, **head**
> (`zero.head-design`), **proxy**. Verb/display/input ids cited below are proposed,
> **unratified** (`model.md` §11 Open) — carried open throughout (GB3). Source currency
> pinned in §7.

---

## 0. Trust-face (GB5) — `CITE(proxy: terminal.md §2–§3, architecture.md §3–§4; head: head/architecture.md)`

**Trusted→trusted, and there is no gate — by design, not omission.** Both endpoints are
trusted; what transits is box-authored *content* (display cells, unsafe names, error
detail), made harmless by the two tier-1 design properties — the wire cannot represent
control, and the head renders without interpreting — never by a proxy check. The proxy's
role is a **memory-safe relay**; installing any content check here would make the proxy
an interpreter of the display service, the exact thing the relay posture refuses (the
JC-1 scoping in `contract/box-proxy.md` §2, accepted at its gate).

Network posture, settled: **the proxy is the head's only network peer** and it
**terminates the adversarial link and re-originates** toward the head — the box has no
route to the head, and the head is never network-exposed to box-authored
stack/scan/service attacks; the box's sole influence on the head is relayed payload
content within the bounded protocol (`CITE(head: head/architecture.md`, threat-layer
table`)`). Here **"terminates" is connection topology, not TLS termination**: the proxy provides
an interface to the adversarial side over the project's own wire protocol, never as a TLS server
(`approach.md` §1).

**Non-box network-exposure (added by the operator's flexible-transport direction, §3):** when the
transport spans an untrusted network (head remote, not local), the proxy↔head link is exposed to
*non-box* network actors — a surface the box-exposure property above does not cover. It does **not**
breach the spine: an injected frame reaches only the head's memory-safe parser and no-interpreter
renderer (worst case a misdraw, R-1), and effect-containment holds regardless of who drives the
link. The residue is operational: injected *input* would command the operator's agent, and the
link is eavesdroppable — authority and confidentiality, neither a defended spine invariant
(`principles.md` §6–§7). So **the proxy relay carries no access control and no peer-identity gate of
any kind** (operator direction, 2026-07-19): peer authentication, like transport security (TLS),
lives **outside the library** — external infrastructure the operator provisions on the
untrusted-network hop, the same off-surface posture policy ruled for TLS (`architecture.md` §11
L10). Both are the head/operator's to provide; neither is a proxy gate nor a spine requirement, and
the relay keeps no identity logic in its trusted surface.

## 1. The decode/relay pin (the load-bearing clause) — `CITE(proto: library.md §7)` + `DEFINE`

On this boundary the proxy decodes **L1/session routing and the binding verb only. It
deep-parses no terminal-service L2 in either direction.** (`library.md` §7: the proxy
relays "display, input, between box and head — routing only", deep-parsing only what it
acts on, which is net + file addressing on the *box* boundary, not here.)

- **Relayed opaquely, box → head:** `display` (0x11), `pane-state` (0x18), `list-entry`
  (0x25, the box answering a head-initiated `list`), `file-error` (0x27, box-emitted).
  The routing set is the `DEFINE` already accepted in `contract/box-proxy.md` §3.2 —
  cross-cited, not redefined.
- **Relayed opaquely, head → box:** `open-session` (0x00), `create-pty` (0x10),
  `key`/`resize`/`viewport`/`mouse`/`paste`/`focus` (0x12–0x17), `list` (0x24,
  head→box), `request-file` (0x26). The safe direction — well-formed, unsanitized, onto
  untrusted ground; input structure is **not a trusted surface** (`CITE(proto: model.md`
  §8 Input events; `proxy: terminal.md` §4`)`). No reflection: the display and input
  channels are split and one-directional; no query frame exists.
- **Proxy-authored toward the head: none — and this is now a proven disjointness, not an
  omission.** `library.md` §7's consumer table (cited **with its own status: illustrative,
  "for the team to refine against the actual binaries"**) reads as though the proxy authors
  `file-data`/`file-error`/`list-entry` *toward the head*; the built system refines that
  against the actual binaries, as the table invites. Proxy-authored `file-data`/
  `list-entry`/`file-error` are answers to **box**-initiated operations — a `file-read` or
  `list` of the *trusted host's* readable root, served by the proxy's own confined reader —
  and they flow **to the box**, never to the head. That direction is a **security floor**,
  not a routing detail: a proxy-authored host listing reaching the head would expose the
  trusted-host filesystem the head must never see. Sprint 15 built the head-browse channel
  on exactly this split — origination-keyed routing, with the head-facing relay
  capability-narrowed so it cannot reach the confined reader at all — and proved both
  directions disjoint with distinguishing canaries. What the head *does* receive under
  these verbs is **box-authored and relayed opaquely** (the box→head bullet above).

## 2. Head-side policies — cited as the head's, never owned here

- **Reserved display ops are ignored at the head** — `display.md` §2: ops `0x09`–`0xFF`
  reserved → *ignored*, the documented forward-compatibility path; reserved cell/
  attribute bits likewise the head's decoder's business. Consistent with the relay
  split: the proxy passes reserved content unpoliced; the receiving endpoint's consumer
  policy is its own. `CITE(proto: display.md §1–§2; head)`.
- **Misdirected/unsupported verbs at the head** (a box-emitted input verb riding a
  relayed session): the head's unsupported-verb handling — support is the boundary at
  every receiver (`CITE(proto: model.md §7)`).
- **The head's frame parser** consumes box bytes on the operator's machine and meets the
  tier-2 memory-safety bar; its renderer is tier-3 feature logic (`CITE(proxy:
  architecture.md §3)`). Nothing in this contract weakens or restates those placements.

## 3. Open-upstream markers (GB3) — none asserted as settled

- **Display attribute/cursor code sets** — proposed, unratified (`model.md` §11 Open;
  `display.md` §2 reserved ranges). Owner: **proto**.
- **Input-event verb ids and layouts** — locked with `zero.head-design`
  (`model.md` §11), ids unratified. Owner: **proto** + **head**.
- **Per-verb flag bits** — layout fixed (verb upper bytes), none defined. Owner: **proto**.
- **KeySym value agreements** — a head↔box standard adoption, not a protocol artifact;
  off this contract's surface entirely (sprint contract, Slice 2). Owner: **head** + box
  adapter.
- **`library.md` §7's consumer table** — self-labeled illustrative; cited at that
  strength only. Owner: **proto**.
- **The concrete proxy↔head transport framing** — the settled facts remain the *properties*
  (only-peer, terminate-and-re-originate, bounded protocol). The concrete wire is now
  **intentionally left flexible by operator direction (2026-07-19): the head connects over any
  protocol that works both locally and over the network.** This is a positive choice, not an
  unfilled gap — the *transport* (the envelope) is deliberately unpinned; the *protocol* it
  carries (the bounded L1/L2 display/input frames) is unchanged. Owner: **head** + proxy design.
  The one surface this flexibility adds — non-box network-exposure when the head is remote — is
  named in §0.

## 4. Judgment calls (GB6) — none new

This boundary introduces **no new judgment call**: the one `DEFINE` it rests on (the
box→head routing set) was made and accepted in `contract/box-proxy.md` (§3.2, JC-2), and
is cross-cited, not re-decided. The §3 transport-framing gap is an open *dependency*
routed to its owner, not a fork this contract recommends on.

## 5. Residuals — named and bounded

- **R-1 — Relayed content is unpoliced by the proxy** (reserved ops/bits included). By
  design: the guarantee is the wire's inexpressiveness + the head's no-interpreter
  invariant (both tier-1); the head's decoder documents ignore-as-forward-compat. Bound:
  the worst relayed content can do is misdraw a screen — "a misread, openly accepted as
  legibility, not security" (`CITE(proxy: terminal.md §4)`).
- **R-2 — Unratified ids under the contract.** Same bound as `box-proxy.md` R-1: the
  proxy's supported/routing sets are closed; ratification changes land by deliberate
  revision.
- **R-3 — The illustrative consumer table.** The `file-data`-to-head leg is cited at
  illustrative strength; if the team's refinement against actual binaries changes it,
  this contract's §1 third bullet tracks the source, not the other way around.
- **R-4 — Transport framing intentionally flexible** (§3; operator direction 2026-07-19). The
  settled properties (only-peer, re-originate, bounded protocol) hold regardless of which concrete
  framing lands, and no clause depends on the choice — so "any protocol that works local +
  network" is a safe resolution. The one surface it adds (non-box network-exposure when remote) is
  named in §0 and is operational, not a spine invariant.

## 6. Currency pins (2026-07-18)

**Re-pinned 2026-07-25** (Sprint 20 audit — currency-pin sweep). Verified each artifact
against the repository rather than carried forward:

| artifact | status | note |
|---|---|---|
| `docs/proxy/terminal.md` @ `da6c63c` | **current** | unchanged since the original pin |
| `display.md` @ `3f060c7`, sha-256 `4fd403788b7b4533` | **current** | commit and hash both still match |
| `head/architecture.md` @ `3f060c7`, sha-256 `1f0f8d275b63ede8` | **current** | commit and hash both still match |
| `proto/model.md` sha-256 `4a468afae792e6e3` | **committed** @ `c6c2b5e` (2026-07-22) | was pinned as working-tree; **same bytes**, now has a commit identity |
| `proto/library.md` sha-256 `c8cb29b87665e60e` | **committed** @ `c6c2b5e` (2026-07-22) | same, byte-identical to the pin |
| `contract/box-proxy.md` | **committed** @ `ae094cd` (2026-07-22) | was "gate-accepted this sprint (working tree)"; also working-tree-modified again by this sprint's pin correction |
| `docs/proxy/architecture.md` | **RE-PINNED** `dec2453` → `e22f7f1` (2026-07-24) | see the flag below |

**Flagged, not resolved here:** `architecture.md` advanced through four commits since the
original pin (Sprints 4, 5, 6, and 16's compiled-in config DSL). The identity above is
re-pinned mechanically; whether anything this contract *relies on* in `architecture.md`
drifted across those four commits is a **content** question, not a pin question, and is
not answered by this sweep. Raised as a candidate for the audit's cold-docs work rather
than resolved inside a re-pin.

Re-pin at the gate.
