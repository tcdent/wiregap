# Sprint 009 — Reserved-Verb Deny and the Quarantine-Write Handler (Phase 4, receive-side)

> Proxy team, sprint 9, the first sprint of Phase 4. While Task 15 (the NIC team's
> originator-partition, which unblocks the box↔proxy reply path) sits at their commit gate,
> the team closes two receive-side, reply-path-independent gaps: Task 13/JC-1's
> reserved-verb-byte deny, and the Sprint-6 `file-chunk` stub's quarantine-write handler.
> Drafted by zero.proxy-lead from planning input by zero.proxy-dev and zero.proxy-review
> (2026-07-22).

---

## Scope Boundary

This sprint owns exactly two receive-side gaps, both independent of the proxy→box reply
path (R-c) and its Task-15 dependency:

1. **Task 13 / JC-1** — the reserved-verb-byte deny (`box-proxy.md` §2) is currently
   unenforced for the four verbs the proxy itself decodes now: `net-address`,
   `file-write`, `file-chunk`, `net-payload`.
2. **The quarantine-write handler** — `file-chunk` currently routes to
   `Routed::ToServiceHandler(Service::File)` with no handler behind it; nothing opens the
   confined quarantine fd or appends bytes.

It explicitly does **not** own:

- **Anything that requires the proxy to originate a session or send to the box.** Task 15
  (the NIC team's originator-partition) is at their commit gate but not yet landed. Building
  ahead of it risks guessing at a partition scheme the NIC team is still committing —
  confirmed team-wide during planning. The moment Task 15 lands, a properly-sized Phase-4
  reply-path integration sprint follows immediately (sizing `fs_read`/`cloud_relay`/
  `terminal_relay` box-send, `file-read`/`list` acting, and the JC-3 deny-notifications, all
  against the real originator-partition — not guessed at piecemeal here).
- **`file-read`/`list`'s reserved-byte check.** Both are in JC-1's acted-on set per
  `box-proxy.md` §3.1, but their decode-and-confine wiring is itself unbuilt today (verified:
  no call site anywhere calls `FileRead::decode`/`List::decode`/`FileReader::open` outside
  tests) and building it is origination-gated (streaming `file-data`/`list-entry` back is a
  proxy→box send). As currently *relayed* (not yet acted on), JC-1 explicitly exempts them
  ("on messages the proxy relays, it polices nothing, reserved bits included") — so they are
  out of this sprint's scope by the contract's own terms, not by omission. Whichever future
  slice wires their decode inherits the reserved-check then.
- **The reserved-scheme surface** (`net-address`, `scheme != 0`). Already enforced
  (`vocab::assemble` denies `Scheme::Other`) — this sprint confirms it stays true with a
  regression test, not a re-implementation.
- **The rung-3 config-gated authority checks** (FS roots, quarantine mount flags, `openat2`
  availability) named as placeholders in Sprint 8. Each needs the not-yet-built config DSL;
  building them now would be inventing that DSL as a side effect, which the PM has
  explicitly ruled against.
- **Any disk-quota/write-cap mechanism beyond what's named as a residual** (see Acceptance) —
  a per-file/per-session write cap is flagged, not necessarily built, pending the team's
  Slice-2 gate finding on whether it's needed now or a named, bounded residual.
- **The `SessionTable::new()` signature change** Task 15 introduces (a side parameter). This
  is the NIC team's breaking change; the lead is coordinating landing timing directly with
  `zero.nic-pm` (PM-sanctioned direct contact for this one item) so the one call site
  (`proxy/tests/dispatch.rs:331`) updates the moment their commit lands — tracked, not part
  of either slice below.

## Problem

Two receive-side gaps sat behind the Phase-3 sprint's narrower scope:

**JC-1/Task 13.** `proto::decode_message` masks `verb & 0xFF` by design (`message.rs`,
Decided Fork #8(ii)) — the reserved upper three bytes of the verb `u32` never reach the
decoded `Message` at all; proto polices no reserved value, by the committed convention that
reserved-*value* policy belongs to the consumer (`box-proxy.md` §1.2, §2). So the check has
to read the **raw** `peek_verb()` u32, before or alongside routing — and today, nothing
does: `dispatch.rs`'s `service_of_verb` computes `(verb & 0xff) >> 4` and `dispatch()`
matches `verb & 0xff` — the upper three bytes are ignored everywhere. Reviewer flagged this
at Sprint 6 as benign (masked routing meant reserved bits couldn't mis-route) but
unenforced; this sprint enforces the disposition JC-1 actually specifies: any nonzero
reserved surface on a message the proxy *acts on* denies the whole message.

**The quarantine-write handler.** Sprint 6 built the file service's read side
(`fs_read::FileReader`) and Sprint 5 built the write-side confinement primitives
(`open_discipline`'s confined `openat2`, `leaf_validate_prescanned`), but nothing connects
`file-chunk` to them: `dispatch.rs` routes `file-chunk` to `Routed::ToServiceHandler(
Service::File)` and stops there. The box can be admitted onto a quarantine destination
(`FileWriteAdmit`) but no bytes ever land — the write path is confined at the gate and
unbuilt behind it.

## Goal

Close both gaps as receive-only, reply-path-independent work, so the box's
`file-write`→`file-chunk` upload path actually deposits bytes on quarantine ground under the
full write-path confinement floors, and so every message the proxy acts on now honors JC-1's
uniform reserved-disposition — while keeping the sprint small enough to pivot immediately
into the reply-path integration sprint the moment Task 15 lands.

## Final State

1. A new `Routed::ReservedDeny { verb: u32 }` variant fires, deny-without-binding, for any
   of the four verbs `dispatch.rs` itself decodes (`net-address`, `file-write`,
   `file-chunk`, `net-payload`) whenever the raw `peek_verb()` u32 carries a nonzero
   reserved surface (upper three bytes nonzero). The check runs on the **raw** u32, never
   the masked low byte — a check on the masked value is a no-op that would pass vacuously,
   and the gate proves the check is discriminating, not just present.
2. **Precedence is pinned**: `service_of_verb` (on the masked low byte) is checked first. An
   unsupported low byte is `Unsupported`/JC-2 regardless of reserved bits — JC-1 applies only
   to messages the proxy *acts on*, and an unsupported verb is never acted on. Only a
   recognized, acted-on verb then gets the raw-reserved-bits check. This resolves the
   otherwise-ambiguous "unknown verb + reserved bits" case by construction, without inventing
   a third disposition.
3. **Deny-without-binding**: a `ReservedDeny` on a session's first frame leaves the session
   completely unbound — no service binding, no stored `Route`, no reassembler entry — proven
   positively (state asserted unchanged post-deny), mirroring the already-accepted JC-2
   unknown-verb-never-binds shape. A reserved-verb frame must never leave a session
   half-bound for a later clean frame to inherit.
4. The reserved-scheme surface (`net-address`, `scheme != 0` → `Scheme::Other` →
   `vocab::assemble` returns `None` → `NetDeny`) is confirmed unchanged by a regression test,
   not re-implemented or re-gated.
5. `file-chunk` is wired to a real quarantine-write handler: `open_discipline` opens the
   confined destination fd (the **full** write-path floor mask —
   `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV`, `O_CREAT|
   O_EXCL`, reusing the certified Sprint-5 primitive, never a weaker or re-implemented open)
   under a `quarantine_root` parameter (a config hook, the same shape as `fs_read`'s
   `readable_root: &Path`); the destination is confined by the existing
   `leaf_validate_prescanned` (reused, not re-validated) before the open. `file-chunk`
   bytes append verbatim at a proxy-tracked, per-session, monotonic cursor — the box
   supplies no offset and cannot seek or overwrite (`file.md` §1).
6. **Completion is the transport session ending** (`file.md` §1, cited not invented): one
   quarantine file per session; the held fd closes on the same `close_session` eviction
   hook that already closes the binding/reassembler-map residuals. No wire-level completion
   flag is added — the wire already settles this.
7. The held-fd map inherits the same bounded-per-epoch shape as the binding/reassembler
   maps (MB4), with the fd explicitly proven closed on session-end — an fd leak is a more
   acute resource bug than the existing map residuals (fds are scarcer than memory).
8. The unbounded-write/disk-fill question is named and resolved one way or the other at the
   Slice-2 gate (see Acceptance) — not shipped silently either way.
9. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — Task 13 / JC-1: the `ReservedDeny` path.**
`peek_verb`'s raw u32 is checked for a nonzero reserved surface on the four acted-on verbs,
after the `service_of_verb`-first precedence (Final State #2). New `Routed::ReservedDeny {
verb }`, deny-without-binding. Pure dispatch logic; hardware-free, fresh-re-run by the
reviewer (no capture model this sprint — both slices are hardware-free). Gate:

- The un-fakeable tell: the reserved test reads the **full** u32 (e.g. `verb & 0xFFFFFF00
  != 0`), never the masked low byte — a test that would fail if the check ran on the masked
  value, not merely a test that passes.
- Discriminating coverage: enumerate acted-on verb × {clean, reserved-in-verb}, covering
  more than one nonzero reserved pattern (a low reserved byte, a high reserved byte, a full
  `0xFFFFFF00`) — the reserved *space*, not one magic value. The same verb with zero upper
  bytes must still route/admit/deny by its real logic, unchanged.
- Deny-without-binding proven positively: after a first-frame `ReservedDeny`, assert no
  service binding, no stored `Route`, no reassembler entry exists for that session — not
  merely that `ReservedDeny` was the return value.
- The reserved-scheme regression: `Scheme::Other` still denies via `vocab::assemble`,
  unchanged and not double-gated.
- No origination: a `ReservedDeny` is silent (no error-verb notification to the box), same
  as every existing deny path (JC-3's notification stays R-c-deferred).

**Slice 2 — the quarantine-write handler.**
Wires `file-chunk` to a real confined write. Gate, split by the reviewer's named tier split:

- *Tier-1-adjacent confinement (reuse, adversarially closed)*: the fd opens through
  `open_discipline`'s existing certified `openat2` with the full write-floor mask (never a
  weaker read-path-style open) and `O_CREAT|O_EXCL`; the destination is confined by the
  existing `leaf_validate_prescanned`, reused not re-implemented. An adversarial destination
  corpus (`..` traversal, a symlink component, an absolute-path attempt, an escape past the
  root) must all confine/deny through the reused primitives — a re-implemented validator or
  a dropped `RESOLVE_*` flag is a REJECT (silent symlink-escape hole), regardless of whether
  the happy path is green.
- *Tier-2/3 append (MB bar)*: the cursor is proxy-owned, per-session, monotonic — no
  box-supplied offset, confirmed. The held-fd map inherits MB4's bounded-per-epoch shape
  plus fd-closed-on-session-end, proven (not asserted). The disk-fill/unbounded-write
  question is explicitly resolved at this gate: either a per-file/per-session write cap
  ships, or the reliance on a filesystem quota is named as a bounded, documented residual —
  not shipped silently either way, mirroring how Sprint 6 caught the unbounded read slurp.
  MB6 holds unchanged (`{open_discipline, privilege}` — no new `unsafe`; the append writes
  verbatim bytes through safe `std`).
- Completion/fd-close model: cited from `file.md` §1 (session-end = completion, one file
  per session), not invented — the gate confirms the citation, doesn't relitigate it.

## Non-Goals

- Do not build any reply-path box-send (`fs_read`/`cloud_relay`/`terminal_relay`'s
  box-facing halves, `file-read`/`list` acting, JC-3's error-verb notifications) ahead of
  Task 15 — confirmed team-wide that this risks guessing at a partition scheme the NIC team
  is still committing, and the encoders/relay-logic that would be "scaffolded" are already
  built and fake-testable, so there is little dead time this would fill.
- Do not invent a configuration system as a side effect of the `quarantine_root` parameter
  — it is a config *hook* (a plain parameter, mirroring `fs_read`'s `readable_root`), not a
  DSL; the actual config surface remains Sprint 8's named, routed residual (L12).
- Do not weaken the write-path floors to match the read-path's `O_NOFOLLOW`-final-only
  discipline — that discipline is `file-read`'s own (its failure mode is safe; the write
  path's is not), and importing it here would open a symlink-escape hole.
- Do not add a wire-level file-boundary/completion flag — `file.md` §1 already settles
  completion as the transport session ending; inventing a flag here would be an unneeded
  wire-contract change the spec doesn't call for.
- Do not re-implement the scheme-reserved check or the leaf-validator — both slices reuse
  certified Sprint-5 primitives; a re-implementation would be a tier-1 change requiring
  fresh adversarial certification for zero gain.

## Starting Surface

Audit first, in this order:

1. `docs/proxy/contract/box-proxy.md` §2 (JC-1's exact text) and §3.1 (the acted-on verb
   catalog, including `file-read`/`list`'s presence and why they're out of scope here).
2. `proto/src/message.rs` (`decode_message`'s `& 0xFF` mask, Decided Fork #8(ii)) and
   `proxy/src/dispatch.rs` (`peek_verb`, `service_of_verb`, `dispatch()`'s `verb & 0xff`
   match — where the raw u32 is already in scope today).
3. `proxy/src/vocab.rs`/`address.rs` (the existing `Scheme::Other` reserved-scheme deny, to
   confirm rather than reinvent).
4. `proxy/src/open_discipline.rs` and `proxy/src/leaf.rs` (the certified Sprint-5 write-floor
   primitives Slice 2 reuses) and `proxy/src/fs_read.rs` (the read-side sibling, for the
   `readable_root`-parameter shape Slice 2 mirrors as `quarantine_root`).
5. `docs/proto/file.md` §1 (the write-path spec: destination, cursor, completion-as-session-
   end — cited directly for Slice 2's fd-close model).
6. `proxy/src/dispatch.rs`'s existing `close_session`/MB4 residual pattern (the binding and
   reassembler maps) — the held-fd map mirrors this shape.

## Method

Both slices are hardware-free and fresh-re-runnable by the reviewer — no capture-model, no
bin-sha split this sprint (Slice 2 uses a temp quarantine root, the same pattern as
`fs_read`'s existing test scratch dir; no NIC involvement in either slice).

- **MB1–MB6** (Sprint 6's bar) apply to every new line in both slices. MB6 stays `{
  open_discipline, privilege }` — Slice 2 adds no new `unsafe`.
- **Slice 1's adversarial-closure bar**: the un-fakeable tell is that the reserved check
  reads the raw, unmasked verb — any test/implementation that would pass with the check on
  the masked low byte is not a real gate.
- **Slice 2's tier split, gated distinctly**: the destination-confinement half is
  tier-1-adjacent (a symlink/traversal escape from quarantine is a containment breach, the
  inverse of `fs_read`'s fail-safe read path) and gated on primitive *reuse*, adversarially
  closed with a destination corpus; the byte-append half is tier-2/3 plumbing gated at the
  MB bar (resource bounds, no new unsafe, cursor ownership).
- **Two forks were resolved during planning, not left open**: (1) the unknown-low-byte +
  reserved-bits precedence — settled by checking `service_of_verb` first (Final State #2);
  (2) the file-write completion/fd-close model — settled by direct citation of `file.md`
  §1's existing text (Final State #6), not a new design decision. Neither reached the PM;
  both were resolvable from already-committed documentation.

## Acceptance

- **Slice 1.** REJECT if the reserved check runs on the masked verb (vacuous pass) rather
  than the raw u32; REJECT if a `ReservedDeny` leaves any session-state trace (a binding, a
  stored `Route`, a reassembler entry); REJECT if the reserved-scheme regression is missing
  or the check is re-implemented rather than confirmed; REJECT if `ReservedDeny` triggers
  any proxy→box send.
- **Slice 2.** REJECT if the destination open is not `open_discipline`'s full write-floor
  mask (any dropped `RESOLVE_*` flag), or if the destination validator is re-implemented
  rather than reusing `leaf_validate_prescanned`; REJECT if the adversarial destination
  corpus (traversal, symlink, absolute-path, root-escape) is not proven confined/denied;
  REJECT if the held fd is not proven closed on session-end (a leak, not merely "should
  close"); REJECT if the disk-fill/unbounded-write question is left unaddressed (neither a
  cap nor a named, bounded residual) — silent shipping either way is not acceptable.
- **Sprint-level.** REJECT if either slice touches a proxy→box send path, or if the
  `quarantine_root` parameter grows into an ad hoc config mechanism.

## Review Questions

1. **The disk-fill/write-cap decision** (Slice 2, resolved at that gate, not pre-decided
   here): whether a per-file/per-session write cap ships this sprint or is named as a
   bounded residual pending a documented filesystem-quota reliance — the reviewer's own
   call at the gate, per Acceptance.
2. **`SessionTable::new()`'s Task-15 signature change** (tracked, not this sprint's slice
   work): timing coordinated directly with `zero.nic-pm`; the one call site
   (`proxy/tests/dispatch.rs:331`) updates the moment their commit lands.
3. **`file-read`/`list`'s eventual acting-wiring** (named, routed to the future reply-path
   sprint): when that wiring lands post-Task-15, it inherits JC-1's reserved-byte check the
   same way this sprint adds it for the other four verbs — flagged so it isn't forgotten
   once relay becomes acting.

## Retrospectives

### zero.proxy-dev

**What I built.** Two slices. Slice 1 (JC-1 ReservedDeny): on the four verbs the proxy *acts
on* (`net-address`, `file-write`, `file-chunk`, `net-payload`), any nonzero reserved surface —
the verb's upper three bytes, a reserved enum scheme, a reserved flag bit — denies the whole
message *without binding the session*. The check reads the **raw** `peek_verb` `u32`, not the
`decode_message`-masked low byte (Decided Fork #8(ii) drops the reserved bytes before they reach
`Message`), so it had to live in `route()` before dispatch, ordered after JC-2 (unsupported /
service-mismatch precede it). Slice 2 (quarantine-write handler): `QuarantineWriter`, one per
session — `file-write` declares the destination, `file-chunk` frames append verbatim at a
monotonic cursor, confinement **reusing** the certified Sprint-5 `confined_create_leaf` (full
write-floor mask, `O_CREAT|O_EXCL`, enforced during kernel resolution). Lazy open on first chunk,
per-file reject-before-grow cap, session-end fd eviction. No new `unsafe` (MB6 held at
`{open_discipline, privilege}`).

**What evidence changed the plan.**
- *The `../root-escape` vector is structurally unrepresentable — a layering finding, surfaced not
  worked around (Slice 2).* Building the adversarial destination corpus, I wrote a `..`-traversal
  case expecting the confined open's `RESOLVE_BENEATH` to catch it — and the test panicked at frame
  construction instead: `scan_safe` rejects a `..` (or `.`) segment as `TraversalSegment` at
  **decode**, so a `SafeSegmentStack` structurally cannot carry one and it never reaches a
  `QuarantineWriter`. The traversal vector is defended one layer up, at the proto codec;
  `RESOLVE_BENEATH`/`NO_XDEV` in the reused mask are belt-and-suspenders below that, proven in
  `open_discipline`'s own suite. I kept the finding *in* the corpus as an explicit
  `traversal_destinations_are_unrepresentable_rejected_at_the_proto_codec` assertion rather than
  silently deleting the case, and re-aimed the reuse test at the vectors that ARE representable
  (a clean-named **symlink component** → `RESOLVE_NO_SYMLINKS` deny; **already-exists** → `O_EXCL`
  deny, existing bytes untouched) — those genuinely exercise the primitive on the live path.

**What needed correcting.**
- *The relayed-exemption half of JC-1 shipped `checked=0` (Slice 1 hold).* My first Slice-1
  submission proved the *acting* half of JC-1 exhaustively (four verbs × three reserved patterns →
  `ReservedDeny`, deny-without-binding, plus the raw-vs-masked discriminator) but only *asserted* the
  relayed half — that display/file-read verbs with reserved bits still **relay** untouched, because
  the proxy polices nothing on relayed messages — without a test that actually drove a
  reserved-bit-set relayed verb through `route()`. The reviewer flagged it: a claimed behavior with
  no test exercising it is `checked=0`, which proves nothing. I added
  `jc1_exempts_relayed_verbs_reserved_bits_still_relay` (display `0x11`, file-read `0x22`, reserved
  bits set → `Relay(..)`, not `ReservedDeny`), closing the hold. The durable lesson: an *exemption*
  is a behavior too — "we deliberately don't check X here" needs a positive test that a would-be-X
  input flows through, exactly as much as the deny path does.

**What went cleanly.** The confined-open reuse was a genuine empty diff — `confined_create_leaf`
was not touched, so the write floor is the *same* certified primitive as the read path, and the
reviewer re-derived that (empty diff, full mask present, corpus exercised on the live Sprint-5
`open_discipline_probe` path). The disk-fill cap resolved at the gate as the sprint's Acceptance
intended — a provisional 32 MiB per-file reject-before-grow scalar marked for review, with the real
per-mount quota named as the config layer's job (Sprint-8 L12 residual), not smuggled into this
handler.

### zero.proxy-review

**`checked=0` proves nothing — now a named, recurring pattern, and it caught the *scoping* half.**
This is the third sprint the same discipline has held (Sprint 5 rejected U4 for a missing empirical
fuzz; Sprint 7 held Slice 1 for the unexercised collision-freedom guard; here Slice 1 held for the
unexercised relayed-exemption). The shape is always the same: a rule with two arms, one arm
thoroughly tested and the other `checked=0`, where the untested arm is the one the rule's *scope*
rests on. Slice 1 proved "an acted-on verb with reserved bits denies" across the reserved space, but
had no test that "a *relayed* verb with reserved bits is *forwarded*, not denied" — the exemption
half of JC-1. The code was correct (`acts_on` excludes relayed verbs), so this was never a code
defect; the point is that a load-bearing scoping property with no test is not *proven*, and a future
`acts_on` drift wrongly denying relayed frames would break forward-compat uncaught. The recurring
lesson worth naming: **when a rule has an "applies-to X, exempts Y" shape, testing the applies-to
side is half a gate — the exempt side needs its own discriminating test (a Y that is *not* caught),
or the scope is unguarded.** The added test denies its own tautology: display/file-read with reserved
bits must *relay*, which fails if `acts_on` ever grows to include them.

**Reuse verification is a diff against HEAD, not a re-read.** Slice 2's whole security argument was
"the confinement reuses the certified Sprint-5 `confined_create_leaf`, unchanged." Reading the
function and seeing the full RESOLVE mask is necessary but *not sufficient* — a reused tier-1
primitive is only certified-by-reuse if it is **byte-identical to the certified version**. So the
load-bearing check was `git diff HEAD -- open_discipline.rs` (empty = unchanged since the Sprint-8
commit that carries the Sprint-5 cert), *then* confirming the Sprint-5 probe adversarially exercises
*that* function (symlink-intermediate → ELOOP, existing-leaf → EEXIST, on the real kernel with
observed errnos), *then* confirming this sprint's code actually calls it (the only open path, no
re-implementation). The durable method: **to accept "reuses the certified primitive," diff the
primitive against the commit that certified it, verify the original cert exercised the exact behavior
now relied on, and confirm the new caller is on the live path — three checks, not one read.** A
re-read alone can't tell a reused primitive from a subtly-edited one.

**A second structural lever showed up worth carrying: confinement above the kernel, by
construction.** The quarantine-write's strongest destination guard isn't the kernel `RESOLVE_BENEATH`
belt — it's that the destination is a `SafeSegmentStack`, and `scan_safe` rejects `.`/`..`/`/` at
decode, so a traversal or absolute path is *structurally unrepresentable*, not merely caught. The
kernel floor is the belt-and-suspenders below an unsayable-by-construction property — the same
"traversal unsayable, not caught" principle the whole project leans on. When gating a confinement,
check for the *unrepresentable* layer first; it's stronger than any runtime check because there's
nothing to bypass.

**What the next sprint inherits.**
- *Two receive-side gaps closed:* JC-1 enforced and both-sides-proven (acted-on denies, relayed
  exempt), and the `file-chunk` stub is now a real confined quarantine writer under the full
  write-floor mask, MB-safe append, bounded and fd-clean.
- *One named residual carried:* the per-mount disk quota (Sprint-8 L12, the config layer) — this
  sprint built the per-*file* 32 MiB cap and named the per-*mount* total as L12's, since the handler
  structurally can't enforce a mount-wide quota.
- *The pending cross-team touch:* Task-15's `SessionTable::new()` signature change lands in
  `dispatch.rs` whenever the nic commit does (coordinated with `zero.nic-pm`), and — the bigger one —
  the moment Task-15's originator-partition lands, the reply-path integration sprint follows: the
  box-send halves of `fs_read`/`cloud_relay`/`terminal_relay`, `file-read`/`list` *acting* (which
  inherits JC-1's reserved check the same way the four acted-on verbs got it here), and JC-3's
  deny-notifications — none of it built ahead of the real partition semantics.
- *A self-correction worth remembering:* I over-flagged the file-write completion model as an open
  fork; `file.md` §1 already settled it ("completion is the transport session ending... there is no
  protocol close signal"). Not-inventing-a-fork-where-the-spec-is-settled is the same discipline as
  getting-small-where-the-future-begins — check the spec before routing a fork.

### zero.proxy-lead

**Both planning-stage forks resolved from documentation already on hand, and both stayed
resolved.** Fork A (unknown-verb + reserved-bits precedence) and Fork B (the file-write
completion model) both looked, on first read, like genuine PM-level questions — the kind
this team has correctly routed upward in prior sprints (Sprint 7's disambiguation fork,
Sprint 8's config forks). Neither was: Fork A resolved by construction once JC-1's own scope
("applies to acted-on messages") was applied literally, and Fork B was answered verbatim by
`file.md` §1, which the reviewer had cited but not yet re-read at the moment they flagged it
open. Checking the documentation before escalating — not after drafting an escalation — is
what kept this sprint at two slices instead of three messages to the PM waiting on answers
already sitting in the repo.

**A cross-team exception to the PM-routing discipline, used narrowly and closed
cleanly.** The PM explicitly invited direct contact with `zero.nic-pm` for exactly one
coordination item (the `SessionTable::new()` signature timing) — a scoped, single-purpose
exception to the standing rule (`[[feedback-lead-routes-through-pm-not-designer]]`), not a
reopening of it. The NIC PM's own reply ("route further coordination through
zero.proxy-pm") confirmed the boundary was understood on both sides and closed the channel
back to normal the moment its one purpose was served. Worth naming because the distinction
— PM-sanctioned exception vs. self-initiated direct contact — is easy to blur in the
moment; it held here because the invitation and its scope were both explicit.

**Team-formed planning produced a tighter sprint than either brief or lead-drafted scoping
would have.** The PM's brief named Task 13 and left "other purely-proxy backlog" and
"scaffolding ahead of Task 15" open-ended. Dev's grounding (the fs_read/cloud_relay/
terminal_relay origination-dependency finding) and the Task-15-timing update arriving
mid-formation together collapsed what could have been a three-slice sprint with speculative
reply-path scaffolding into two tight, independent slices — with the team itself naming why
the scaffolding wasn't worth building (the encoders are already built and fake-testable;
speculative origination code risks guessing at a partition scheme still being committed).
Forming the plan as a team surfaced that judgment before any contract line was written,
not after a slice was half-built.

**What the next sprint inherits.** The reply-path integration sprint (fs_read/cloud_relay/
terminal_relay box-send, `file-read`/`list` acting and their JC-1 reserved-check, the JC-3
deny-notifications) is the natural next Phase-4 sprint, gated only on Task 15's landing —
size it against the real originator-partition once committed, not against a guess. The
`SessionTable::new()` call-site update is tracked and small, coordinated for same-window
landing. Both retrospectives' methodological additions — checked=0 as a named recurring
gate discipline, the reuse-is-a-diff-not-a-re-read verification method — are durable and
apply beyond this sprint.
