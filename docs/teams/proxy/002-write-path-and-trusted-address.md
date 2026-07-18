# Sprint 002 — Filesystem Write-Path Primitives and the Trusted-Address Format

> Proxy team, sprint 2. The team settles two tier-1 security mechanisms the exploration
> docs left as sketches, turning them into specs. Drafted by zero.proxy-lead from planning
> input by zero.proxy-dev and zero.proxy-review (2026-07-18); green-lit by zero.proxy-pm.

---

## Scope Boundary

This sprint owns **two deliverables**, both new spec files (the source docs self-describe
as research-not-specification; a spec supersedes, it does not edit the source in place):

- `docs/proxy/spec/filesystem-write-path.md` — the open-discipline symlink floor
  (`filesystem.md` §1) and the quarantine inertness mechanism (`filesystem.md` §2).
  Architecture ledger items **L6, L7**.
- `docs/proxy/spec/trusted-address.md` — the proxy-side half of the trusted-address
  representation (`approach.md` §7). Not a ledger item by number, but the gap `approach.md`
  §7 names.

It explicitly does **not** own:

- **Fixing source-doc wording.** Two genuine doc/artifact divergences surfaced in planning
  (below) route to their owners via the PM; this sprint states them with citations and
  does not edit `filesystem.md` or `approach.md` in place.
- **Reopening Sprint 1's accepted placements.** The leaf-validator alphabet, every tier
  placement, and the ledger's existing rows are gate-accepted ground. L4/L5 (recognizer
  placement/shape) stay the cloud effort's; L12 (config mechanics) stays open — this
  sprint only *states* its interactions with L12 (root-fd re-pin, register spelling), it
  does not settle L12.
- **Ratifying the judgment calls itself.** Every point where two competent engineers could
  choose differently and the source doesn't adjudicate is routed to design/policy with a
  recommended default (§ Acceptance, SA7) — not settled unilaterally in the spec.
- **The wire-format redesign.** The box→proxy `net-address` wire representation
  (`docs/proto/net.md` §1, `proto/src/segment.rs`) is already committed; this sprint
  specifies only the proxy-side consumption of it (type, assembly, match, policy).

## Problem

Two tier-1 mechanisms are stated as requirements but not settled as specs:

**1. Filesystem write-path primitives.** `filesystem.md` §1 requires the open discipline to
keep the write "structurally rooted in the real quarantine subtree no matter what the path
contains" and states the primitives are "not yet design-reviewed... the intended direction,
not a settled spec." §2 requires the quarantine be inert ("nothing on the trusted side
executes in — or auto-processes" it) and states the enforcement mechanism is likewise
pending. Planning surfaced a **real defect in the sketch itself**, independently found by
both dev and reviewer: `filesystem.md` §1's literal text — "refuse to follow a symlink at
any component (`O_NOFOLLOW`)" — overcredits the primitive. Per `open(2)`, `O_NOFOLLOW`
refuses a symlink at the **final path component only**; an intermediate-component symlink
is still followed. As literally written, the sketch does not meet its own
any-component requirement. This sprint's spec must close the gap the sketch missed, not
merely formalize the sketch as written.

**2. Trusted-address format.** `approach.md` §7 states the requirement (a safe,
purpose-built representation for scheme/domain/path, incapable of expressing a dangerous
byte, "inspired by the file-leaf pattern but a distinct system") without specifying the
format. Planning found this premise **partially stale**: the box→proxy wire half already
exists as committed, normative spec and code — `docs/proto/net.md` §1 (a fixed `scheme`
u32 enum, `0`=https, reserved otherwise; then a safe segment stack, first segment the
domain with dots within it, rest the path) over the committed `proto/src/segment.rs` scan
(`[A-Za-z0-9._-]`, ≥1 alphanumeric, `0x1F`-delimited, verify-never-transform). Port,
userinfo, fragment, percent-encoding, and `..`-as-segment are already unrepresentable by
construction. **What is missing is the proxy-side half**: the `Address` type, assembly and
byte-exact match semantics, the scheme/reserved-value policy (the proto library validates
structure only; reserved-*value* policy is explicitly the consumer's — i.e., this sprint's),
caps, and the route register's canonical-spelling rule.

Planning also surfaced a **doc/artifact seam to flag, not fix**: `approach.md` §7 states
the proxy supplies the structural dots ("the proxy supplies the structure — `://`, `.`,
`/`") and calls the address representation "a distinct system" from the file-leaf pattern.
The committed `net.md` §1 + `segment.rs` show the **box** supplies the dots (they are
inside the domain's safe alphabet, within the first segment) and the address reuses the
**same stack and alphabet** as the file leaf, distinct only by a scheme prefix and a
position convention (first segment = domain). `approach.md` self-describes as
research-not-specification; `net.md`/`model.md` are the normative spec for the wire, so the
committed form likely governs. This sprint's spec cites both and routes the `approach.md`
wording fix to its owner via the PM — it does not resolve the divergence itself.

## Goal

Produce two settled specs for tier-1 mechanisms currently only sketched, each closing the
adversarial surface the requirement demands (not merely restating the sketch), with every
genuine judgment call named, given a recommended default, and routed to design/policy for
ratification rather than settled unilaterally.

## Final State

1. `docs/proxy/spec/filesystem-write-path.md` exists: the pinned-root, per-component
   resolve, and leaf-creation primitives (exact syscalls and flags) for the open-discipline
   floor; the mount flags and no-auto-processor discipline for quarantine inertness. Every
   claim about a primitive's semantics is grounded in the real syscall contract, not the
   source sketch.
2. `docs/proxy/spec/trusted-address.md` exists: the gap stated against the committed wire
   spec; the `Address` type, assembly rule, byte-exact match semantics, scheme/reserved
   policy, caps, and register-spelling rule for the proxy-side consumption.
3. Both specs name every residual and show it bounded (SA4) — no unstated gap.
4. Both specs are constructive/total, never parse-then-validate against an unbounded
   grammar (SA2, SA6) — this applies with special force to the address, which must be
   built from safe components, never parsed as a URL string.
5. A consolidated **ratification packet** (Slice 3) lists every judgment call from both
   specs with a recommended default and an owner, routed via the PM to design/policy —
   before either spec is treated as **frozen**. A spec may gate-ACCEPT with judgment calls
   flagged-and-routed-but-not-yet-ratified (the Sprint-1 L10 pattern): acceptance requires
   the calls be *correctly identified and routed*, not that the ruling already exist.
6. The two doc/artifact seams (the `O_NOFOLLOW` sketch overcredit; the `approach.md` §7
   dots/distinct-system divergence from `net.md`) are stated with citations in the relevant
   spec and routed to their doc owners via the PM — neither source doc is edited by this
   sprint.
7. The sprint document carries role retrospectives; the sprint is committed with only its
   intentional files.

## In Scope — ordered slices

**Slice 1 — Filesystem write-path primitives spec (L6 + L7 together — one subsystem, two
interlocking floors).**
Deliverable: `docs/proxy/spec/filesystem-write-path.md`, covering:
- *Open-discipline floor*: pinned root descriptor (`O_PATH|O_DIRECTORY|O_NOFOLLOW`,
  resolved once at startup, fail-closed on failure); per-component descent via `openat2`
  with `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV`; leaf
  creation via `O_CREAT|O_EXCL`; one fd held for a transfer's lifetime (never reopened
  mid-transfer, reconciling `O_EXCL` with `filesystem.md` §4's write-at-offset
  reassembly); a single reviewed path with no fallback walk (platform pin: Linux ≥5.6,
  `openat2`; probed at startup, `ENOSYS` fails closed).
- *Quarantine inertness*: dedicated mount, `noexec,nosuid,nodev,nosymfollow`; a startup
  assertion (`statvfs(2)` on the mount flags, fail-closed if absent) as a new, ratified
  convention; the no-auto-processor discipline stated as an enumerable host checklist
  (kernel-enforced floor vs. operational invariant, honestly distinguished — `noexec`
  blocks direct execution but not an interpreter invocation, which the checklist, not the
  mount, must close).
Gate: full independent acceptance (SA1–SA7, below).

**Slice 2 — Trusted-address spec, committed-seam-first.**
Deliverable: `docs/proxy/spec/trusted-address.md`, covering:
- *(a)* Gap analysis against the committed `net.md`/`segment.rs`, with working-tree
  currency pinned (both files are modified, uncommitted, as of sprint start).
- *(b)* The proxy-side spec: `Address` type over the committed segment stack;
  deterministic, injective assembly for match and log; byte-exact match against the route
  register (no case-folding, no normalization — verify-and-deny, never rewrite, per
  `approach.md` §2–3); the scheme/reserved-value policy (accept `0`/https only; every other
  value, including every reserved one, denied); caps beyond what the proto scan already
  bounds; the register's canonical-spelling storage rule.
- *(c)* The `approach.md` §7 seam (dots/distinct-system divergence from the committed wire)
  stated with both citations and routed, not resolved.
Gate: full independent acceptance (SA1–SA7, below).

**Slice 3 — Ratification packet (thin, closeout).**
The merged judgment-call list from Slices 1–2, each with a recommended default and a named
owner (design, policy, or both), handed to the PM for onward routing before either spec is
marked frozen. Includes the two doc-wording-fix routings (filesystem.md §1, approach.md
§7). Gate: the packet accurately and completely reflects every judgment call surfaced in
Slices 1–2 — a lighter check than SA1–SA7, since nothing new is authored here.

## Non-Goals

- Do not edit `filesystem.md` or `approach.md` — cite and route their wording gaps.
- Do not reopen any Sprint-1-accepted placement, tier, or ledger row (L4/L5/L12 stay as-is;
  this sprint only states its *interactions* with L12, never settles it).
- Do not add label-level validation beyond alphabet + exact match to the domain segment
  "for extra safety" — exact match against the operator's canonical register entry is the
  whole guarantee; added validation is surface without containment gain (a judgment call,
  routed with a recommended-against default, not decided here by default-safety instinct).
- Do not add a fallback resolution path alongside `openat2` "for older kernels" — one
  reviewed path (principles §15); a platform pin that fails is a platform-support decision
  for design, not a reason to widen the audited surface.
- Do not normalize a non-canonical address before matching — a non-canonical spelling
  simply matches nothing and is denied, legibly (verify-and-deny, never rewrite).
- Do not treat a provisional scalar (element counts, length bounds, mode bits) as a
  blocking judgment call — assign, mark for review, proceed.

## Starting Surface

Audit first, in this order:

1. The two requirement sources: `docs/proxy/filesystem.md` §1–2 (and §4–5 for the
   chunked-transfer interaction), `docs/proxy/cloud/approach.md` §7.
2. The address's actual behavior, anchored on the **committed code**:
   `proto/src/segment.rs` (`scan_safe`, committed and clean) is the hardest anchor for the
   Slice-2 gap analysis. `docs/proto/net.md` §1, `docs/proto/model.md` §6, and
   `docs/proto/file.md` §0 describe the same behavior in prose but are
   **working-tree-modified, uncommitted**, as of sprint start — validate against the
   committed seam, not the doc; re-pin doc currency at every phase boundary regardless.
3. `docs/proxy/architecture.md` ledger items L6, L7, L11 (kernel standing — both floors in
   this sprint rest on kernel enforcement, intensifying L11's weight without opening a new
   ledger row), L12 (config mechanics — state interactions, do not settle).
4. Primitive semantics from established POSIX/Linux documentation (`open(2)`, `openat2(2)`,
   `mount(8)`, kernel version gates) — cited word-for-word, not sketched (SA5).
5. Sprint 1's standing rule: a spec supersedes a stale or under-specified sketch; it does
   not edit the source doc in place. Wording fixes route to the doc's owner via the PM.

## Method

Sprint 1's method (source docs as anchor, re-derivation replaces the run) does not
transfer whole: **this sprint authors new mechanism, so there is no prior artifact to
diff against.** The anchor becomes three things, and the reviewer's independent adversarial
re-derivation replaces the diff:

- **The stated requirement**, verbatim, is the fixed point each spec must meet — not a
  weaker cousin of it (SA1).
- **The spine's method-mandate** — a total function over a finite, enumerable domain,
  reviewable by enumeration (principles §15–16); "make it impossible, do not make it
  caught" (`containment-by-construction.md` §1) — governs both specs' shape (SA2, SA6).
- **Adversarial closure**: the reviewer independently constructs the adversary's attempts
  (not the spec's own "threats considered" list) and confirms each is denied by the
  mechanism, and the spec *proves* closure rather than asserting reasonableness (SA3). This
  is Sprint 1's "re-derive, don't trust the report" applied to the attack surface.

Additional disciplines specific to authoring new security mechanism:

- **Primitive-semantics grounding.** Every claim about what a syscall or mount flag does is
  verified against its real contract (man page / kernel documentation / kernel version),
  not the source sketch's description of it — the `O_NOFOLLOW` finding is the standing
  example of why (SA5).
- **Carry, don't rediscover.** Prefer an atomic operation that carries a safety check into
  the action itself (`openat2`'s resolve flags) over a check-then-act split that opens a
  TOCTOU window (a symlink swapped in between validate and open).
- **Constructive, not validating**, for the address in particular: build the representation
  from safe components the proxy structures; never parse an arbitrary string against a
  grammar and then validate it (the unbounded-grammar bet the spine forbids).
- **Platform pins are bench pins.** A kernel-version requirement (`openat2` ≥5.6,
  `nosymfollow` ≥5.10) is stated and pinned in-spec the way the nic team pins a bench fact
  — verified against the actual deployment kernel during the sprint, not assumed.

## Acceptance

Seven gates apply to **each** spec at its slice close (SA1–SA6 verbatim from planning;
SA7 added — the judgment-call analog of Sprint 1's G4, since this sprint authors forks
Sprint 1 never had to):

- **SA1 — Requirement traceability.** The spec states the exact invariant of its source
  requirement — not a weaker one. REJECT if the spec enforces "confine most paths" for
  "escape unreachable," or "validate the URL" for "malicious byte unrepresentable."
  **Anchor-selection rule for the address (currency-of-source applied to this gate):**
  where the requirement source (`approach.md` §7) and the committed artifact
  (`net.md`/`segment.rs`) diverge, SA1's anchor is the **committed artifact** — the
  divergent source prose is stale-pending-correction (RQ3), not the standard the spec is
  held to. Do not flag the committed form as "not meeting the requirement" by mechanically
  anchoring on the stale prose.
- **SA2 — Totality/enumerability.** A total function over a finite, enumerable domain
  (principles §15–16): every input maps to admit-or-deny, no unhandled case, no parse over
  an unbounded grammar. REJECT on any unhandled input or a grammar-parsing bet.
- **SA3 — Adversarial closure (the load-bearing gate).** The reviewer independently
  constructs the adversary's attempts and confirms each is denied; the spec must *prove*
  closure, not assert reasonableness. The appendix checklists from planning (leaf,
  open-discipline, inertness, address) are the starting surface, re-pinned fresh at review
  time — not treated as exhaustive or final.
- **SA4 — Residual named and bounded.** Every mechanism's residual is named and shown
  bounded. REJECT on an unstated residual.
- **SA5 — Primitive-semantics grounding.** Every primitive claim is verified against the
  real syscall/mount contract, not the sketch. The `O_NOFOLLOW` final-component-only fact
  is the standing example this gate exists to catch.
- **SA6 — Constructive, not validating.** Especially for the address: built from safe
  components, never parsed-then-validated against an unbounded grammar. REJECT a
  parse-then-validate design.
- **SA7 — Judgment calls surfaced and routed, not silently settled.** Every point where the
  requirement leaves a genuine choice (the discriminator: would two competent engineers
  reasonably choose differently, with the source silent?) is named, given a recommended
  default, and routed to design/policy — never settled unilaterally in the spec. REJECT +
  route if a judgment call is presented as a settled conclusion.

Slice 3's gate is lighter: the ratification packet completely and accurately reflects
every judgment call raised in Slices 1–2, with no new authoring.

## Review Questions

1. **Spec file location (ruled by lead, mechanical):** two new files,
   `docs/proxy/spec/filesystem-write-path.md` and `docs/proxy/spec/trusted-address.md`.
   `filesystem.md`/`approach.md` remain research documents; their owners update the sketch
   text to point at the spec post-ratification, on their own schedule.
2. **Filesystem doc wording fix (routed via PM to the `filesystem.md` owner):**
   §1's "refuse to follow a symlink at any component (`O_NOFOLLOW`)" overcredits the
   primitive (final-component-only per `open(2)`); the spec states the correction and
   supersedes it, but the sketch text itself needs its own fix.
3. **Cloud doc wording fix (routed via PM to the `approach.md` owner):** §7's
   "the proxy supplies the structure... `.`" and "a distinct system" framing diverges from
   the committed `net.md`/`segment.rs` (the box supplies the domain's dots within the
   segment; the address reuses the file-leaf stack and alphabet). `net.md`/`model.md` are
   the normative spec; `approach.md` self-describes as research — the committed form likely
   governs, but this is the doc owner's fix to make, not this sprint's.
4. **The consolidated judgment-call list (for design/policy via the PM, non-blocking on
   gate acceptance per SA7):**
   - `openat2`-only with no fallback walk (recommended: yes — one reviewed path, §15).
   - `RESOLVE_NO_XDEV` + `nosymfollow` inclusion (recommended: yes — cheap, strictly
     narrowing; `nosymfollow`'s own verification method, since it isn't `statvfs`-visible,
     is a further sub-fork).
   - The startup-assertion convention itself — the proxy checking its own kernel-enforced
     floors at boot is new, present in no source (recommended: yes, ratify as a standing
     convention).
   - Domain-segment validation beyond alphabet + exact match (recommended: no — added
     validation is surface without containment gain).
   - Collision policy: `O_CREAT|O_EXCL` always fails a name collision and surfaces it
     (recommended: yes, fail-closed/no-overwrite is correct; confirm no legitimate
     re-write-same-transfer case is being foreclosed).
   - Whether tier-1 inertness may rest on kernel-enforced mount flags at all — the same
     class of question as L11 (kernel excluded from the reviewable core, yet the
     enforcement substrate here). Tie explicitly to L11 when routing; recommended default:
     acceptable, same posture the kernel already carries — but policy's call, not the
     spec's.
   - Reserved-scheme-value policy: accept only `0`/https now, or reserve room to extend
     (recommended: https-only for now; extending any reserved value later is a deliberate,
     legible spec revision — the promotion pattern, not a default-open policy).
5. **Currency dependencies (tracked, not blocking):** `docs/proto/net.md` and
   `docs/proto/file.md` are working-tree-modified as of sprint start; Sprint 1's
   `terminal.md` §5 commit dependency is still outstanding. Track both as commit
   dependencies in the new specs' banners.

## Retrospectives

### zero.proxy-review

**Evidence that changed the gating approach.** Three moments, each from deriving the surface
from the mechanism rather than reading the spec's own account:

- *SA7 is the new-mechanism analog of Sprint-1's G4, and it earned its keep.* Sprint 1 (pure
  composition) only had to check that source-opens were *carried*; Sprint 2 *authors* genuine
  forks, so the gate needs a routing discipline — every point where two competent engineers
  could choose differently, with the source silent, must be named-with-a-default-and-routed,
  never settled in the artifact. Every judgment call (J1–J4, J-coll, J-kernel, JT-1..3) flowed
  cleanly to the ratification packet because SA7 forced them out at each spec's close.
- *The root-pin residual (Slice 1) came from re-deriving the mechanism, not the table.* The spec
  made the `O_NOFOLLOW`-final-only limitation its headline correction — then pinned the
  quarantine root with `open(…O_NOFOLLOW)`, the very primitive it corrected, leaving the
  intermediate-symlink safety of the operator-configured root path unstated. Reading "what does
  the root pin actually resolve" (not "does the table look complete") surfaced it. SA4 forced it
  into the open as R-H; it spawned J4 (harden-vs-name, routed). This is the "unstated assumption
  hides as obviously fine" case the residual gate exists for.
- *Independent probe reproduction beat capture-acceptance.* Being on the bench kernel
  (`6.12.74`), I compiled and re-ran the `openat2` probe myself — 10/10, mask `0xf` = the exact
  §1.1 mask — after confirming its sha bound to the spec's claim and that it drove the spec's
  actual mechanism. Reproduction upgraded SA3/SA5 from capture-verified to
  independently-reproduced. (Contrast the nic silicon crossing, where re-running unconfined DMA
  was unsafe and capture-fallback was correct — the discipline is *reproduce when safe*, not
  *always accept the capture*.)

**What gating a NEW mechanism (vs Sprint-1 composition) taught the method.** With no prior
artifact to diff, the anchor is three-legged and the reviewer's own adversarial re-derivation
replaces the diff: the **verbatim requirement** (SA1), the **spine's total-function bar**
(SA2/SA6), and **adversarial closure the reviewer constructs** (SA3) — never the spec's own
threat list. Two sub-disciplines carried the weight:

1. *Two anchors, verified directly.* The committed **code** (`segment.rs`) for the parts already
   built — re-pin the cited line numbers yourself against the committed file, never a remembered
   one — and the real **syscall/man-page contract** for the primitives (verify, don't trust the
   sketch; the `O_NOFOLLOW`-final-only finding is the standing example).
2. *Constructive-not-validating is the shape that makes closure provable.* Both mechanisms build
   from safe components (a validated element array; scanned segments the proxy structures) and
   never parse a grammar. The address's injectivity proof rests entirely on the alphabet
   excluding the structural bytes (`:` `/`) — a property checkable by reading the accept-arms,
   not a parser to trust.

**What the next proxy sprint inherits.**

- *The ratification packet is the live register of routed calls* — J-kernel (→ policy, L11-tied),
  JT-1..3, and the W1/W2 doc-wording fixes are pending upstream. Carry them; do not re-derive.
- *Commit dependencies are real and must close before freeze:* `terminal.md` §5 (Sprint 1), and
  `net.md`/`model.md`/`file.md` (Sprint 2). A spec that cites working-tree source is soft until
  its source commits — anchor on the committed code where one exists (Slice 2 leaned on the
  committed `segment.rs`, not the working-tree `net.md`).
- *The gate discipline transfers by shape:* a NEW mechanism →
  requirement + spine + adversarial-closure + primitive-grounding + probe-reproduction; a
  COMPOSITION → re-derivation diff; a doc built on other docs → currency-of-source check. The
  next tier-1 mechanism — the cloud request recognizer (ledger L4/L5), a recursive recognizer
  over attacker-authored JSON — will be the sharpest new-mechanism gate yet, and it inherits this
  whole discipline: SA3 adversarial closure and SA6 constructive/total most of all, with the
  recognizer-placement fork (L4) routed, not settled.

### zero.proxy-dev (author)

**What worked, and why it should be kept.** Committed-seam-first paid for itself before a
line was authored: reading `proto/src/segment.rs` and `net.md` §1 during *planning* turned
"design a URL format" into "spec the consumer of a committed wire format" — smaller,
correct, and collision-free with another team's ratified ground. The bench-probe
discipline carried from the nic team's culture translated cleanly to spec work: a 10-case
`openat2` probe on the real kernel made SA3 rows *evidence* instead of man-page citation,
and the reviewer independently recompiling and re-running it (10/10, same errnos) is what
an evidence gate should look like. Keep both as the default for any future
mechanism-settling sprint.

**What evidence changed the plan.** Two findings, both from reads the plan almost skipped:
(1) the headline — `approach.md` §7's "format does not exist yet" premise was partially
stale against the committed proto reality, which reshaped Slice 2 from invention to gap
analysis; (2) the `O_NOFOLLOW` correction — the host man-page's own sentence ("symbolic
links in earlier components ... will still be followed") showed the source sketch did not
meet its own any-component requirement. Durable rule from both: **the sketch is not the
requirement — re-derive the requirement from the primitive's real contract and the
committed artifact before formalizing anything.**

**What failed and needed correction (two, each with the rule it taught).**
1. *The spec repeated the class of bug it was correcting.* §1.0's root pin used
   `O_NOFOLLOW` — final-component-only, the exact limitation §0 corrects — with the
   intermediate-component trust unstated. Harmless in substance (the root path is
   operator config, adversary class none) but an unstated residual on the very primitive
   under correction. Rule: **after writing a correction, grep your own spec for every
   other use of the corrected primitive and state each one's standing explicitly** — the
   corrected mistake reappears most easily in the corrector's own text.
2. *A dangling cross-reference invented a judgment call.* R-H cited "J4's reasoning" when
   no J4 existed in the spec's own list — the name lived in the contract's consolidated
   list for a different call. The reviewer's fix was right: the harden-vs-name choice IS
   a judgment call, so it became J4 properly. Rules: **a rationale reference must resolve
   within the artifact it appears in**, and **namespaced call IDs per artifact (the
   `JT-*` scheme) prevent cross-list collisions** — adopted mid-sprint, keep it.

**What the next sprint inherits.** (a) The ratification packet
(`002-ratification-packet.md`): ten routed calls + two wording routings; **neither spec
is frozen until ruled** — the next sprint building on either spec checks the rulings
first. (b) Standing commit dependencies: `terminal.md` §5 (Sprint 1), the
`net.md`/`model.md`/`file.md` working-tree changes pinned in `trusted-address.md` §8.
(c) The bench pin is a *bench* pin: every §7-appendix claim re-verifies on the designated
deploy host when one exists (residual R-E). (d) Two conventions that earned their keep
and should be reused: probe-as-sprint-record (source + sha in `docs/teams/`, spec points
at it), and requirement-verbatim §0 anchors that make SA1 checkable by quotation.

### zero.proxy-lead

**Why gate cadence changed from Sprint 1.** Sprint 1 was one composition doc with one
close-boundary gate; this sprint authored two independent, self-contained specs, so I
routed each to its own full SA1–SA7 gate at its own close rather than batching into one
end-of-sprint review. The right call, confirmed in hindsight: Slice 1's REJECT (the
root-pin residual) had nothing to do with Slice 2, and gating them separately meant the
Slice-2 author and reviewer weren't idling on an unrelated fix. **Rule: gate per
independent artifact, not per sprint, when a sprint bundles genuinely separable
deliverables.**

**The planning round earned its cost again, sharper this time.** Both dev and reviewer
independently surfaced the same real defect (`O_NOFOLLOW`'s final-component-only scope)
before either wrote a line of spec — convergent discovery from two different angles is a
strong signal a finding is real, not an artifact of one person's framing. Dev's
headline finding (the wire format already committed) reshaped the sprint's real scope for
mechanism 2 before Slice 2 started; I verified it on disk myself before letting it change
the contract's Problem section, the same discipline as Sprint 1's `terminal.md` check.
**Rule: a teammate's "this premise is stale" claim gets verified against the file before
it's allowed to reshape scope — regardless of how good the sourcing sounds in the
report.**

**A design-fork discipline that held under real pressure.** Unlike Sprint 1 (pure
composition, one late-arriving fork), this sprint authored *many* genuine judgment calls
by construction — new mechanism has no source to defer to. SA7 (added at contract time as
this sprint's G4-analog) held the line at every gate: every "how would you build this"
choice got a recommended default and a routed owner, never a silent settlement. The one
near-miss (R-H's dangling reference to a "J4" that didn't yet exist) was itself evidence
the discipline was working — the reviewer caught that a *stated rationale* was quietly
standing in for an *unrouted judgment call*, and the fix promoted it to a real, routed
item rather than papering over the gap.

**What I ruled without escalating, and why each was safe to.** (1) Spec file location
(new files under `docs/proxy/spec/`) — organizational, no design content. (2) The
deployment-host bench-pin question — no host is named anywhere in the docs, so this was
genuinely open, but the fix (use the current bench, label it bench-not-deploy, re-verify
later) is reversible and mirrors an existing team convention (the nic team's bench pins) —
not a security posture decision. Neither needed the PM; both were mechanical/process calls
inside my own authority to frame a sprint, not judgment calls about what the system should
guarantee.

**What the next sprint inherits.** (a) Per-artifact gating is the right cadence whenever a
sprint bundles independent deliverables — don't force them through one shared gate. (b)
Convergent findings from dev and reviewer, arrived at independently, are the strongest
signal in this loop that something is real — when both flag the same thing unprompted,
verify it fast and let it reshape the plan. (c) SA7's discipline (judgment calls routed,
never silently settled) is worth carrying forward as a standing gate whenever a sprint
authors genuinely new mechanism, not just this one. (d) The ratification packet
(`002-ratification-packet.md`) is now the team's live register of everything pending
policy/design — the next sprint checks it before assuming either spec is final.
