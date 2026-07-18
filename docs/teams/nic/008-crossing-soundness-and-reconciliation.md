# Sprint 008 (7b) — Crossing-Soundness Verified Account + the architecture.md Reconciliation

> The `nic` team's eighth sprint — the **7b** half split off from Sprint 7 at the pre-Slice-4 checkpoint,
> because it is a **different kind of work**: not building or wiring, but **cited reasoning** and **grounded
> design-of-record reconciliation**. The subsystem is operational (7a, `33281e8`); what remains is to close
> the two honest-open residuals the operational milestone deliberately did *not* discharge — *operational ≠
> crossing-sound ≠ (design-doc-)reconciled*. Two workstreams: **(1)** a **verified account** of the
> `dma.rs` device-memory crossing-soundness — the project's longest-open hazard (`architecture.md §3.1`:
> "carries hazards we have **not** analyzed to resolution") — plus the slice-sized **fence code**; and
> **(2)** grounding + **surfacing** the `architecture.md` §2.1/§3/§5/§6/§7 reconciliation to the designer,
> with the live 6/7a evidence.
>
> **Standing framing (operator-clarified, load-bearing):** the untrusted-but-contained **posture STANDS** —
> the NIC is still untrusted-but-contained; we handle the containment ourselves (the driver guard contains
> vector-2 adversarial content, unchanged). Only the vector-1 DMA-reach became the **owned residual** with
> no IOMMU. §2.1's containment *mechanism* reconciles to *we-contain-it-ourselves* — this is **NOT
> "containment invalidated."**
>
> Team: `nic` (lead `zero.nic-lead`, dev `zero.nic-dev`, review `zero.nic-review`, pm `zero.nic-pm`,
> designer `zero.nic-design`). Authority: [`architecture.md`](../../nic/architecture.md) (§2.1 posture, §3/§5
> reactor/events, §3.1 the copy-crossing, §6 framing, §7 reliability), [`grounding/kernel-bypass-and-dma.md`](../../nic/grounding/kernel-bypass-and-dma.md)
> (§2 owned-DMA, §4 bounded-poll), [`grounding/data-path-and-events.md`](../../nic/grounding/data-path-and-events.md),
> [`concerns-to-watch.md`](../../nic/concerns-to-watch.md), [`docs/teams/nic/007`](007-final-integration-validation.md)
> (the 7a retros + the two residuals + the §7 window finding). Builds on 7a (`33281e8`); the operational
> stack (`reactor/`, `transport/`, `driver/`, `directpath/`) is frozen — 7b touches only the `dma.rs`
> crossing (the fence) + the docs.

---

## Scope Boundary

**This sprint owns:**

1. **The `dma.rs` crossing-soundness VERIFIED ACCOUNT** — a **cited reasoning artifact** (checked by
   *reading against its citations*, never by a run — a green run is **not** evidence, "path-works ≠
   crossing-sound"). It is a **POSITIVE platform-scoped account** (the crux is citable — below), not merely
   a narrowed-open residual. It must cover **BOTH crossing directions** — `dma.rs`'s own SAFETY comments
   name both as undischarged — or explicitly scope to one and keep the other honest-open (a read-only
   account presented as closing the crossing is *partial-as-full*):
   - **RX read crossing — device/fabric write-ordering (the crux, CITABLE):** the 82599 writes the packet
     payload to host memory **before** the descriptor `DD`/status. **Cite** the datasheet §3.1.4.4 item 2
     ("a Rx descriptor is written back on PCIe only after the data … is written to the PCIe link") + PCIe
     posted-write ordering; and §3.1.4.5.3 (relaxed ordering **cannot** relax descriptor writes) — so
     payload-before-`DD` holds even with RO on. Assuming "`DD`=1 ⇒ payload is there" without citing *why* is
     **papered**.
   - **TX write crossing — CPU→device ordering (the reviewer's completeness catch):** `post`/`refill` store
     the buffer + descriptor to WB host memory, then the MMIO tail **doorbell**; the device DMA-reads them,
     which must be globally visible **before** it reads. **Cite** x86-TSO STORE-STORE (Intel SDM 8.2.2) for
     descriptor-store-before-doorbell + the UC-MMIO doorbell (strongly ordered, drains the store buffer) +
     coherent DMA + device-reads-after-doorbell.
   - **CPU-side (read):** x86-TSO preserves LOAD-LOAD (SDM 8.2.2); the `DD`-gating read is realized as an
     **`AtomicU32` `load(Acquire)`** (not `read_volatile`+`fence` — a `fence(Acquire)` formally pairs with
     an atomic load, so fence-after-volatile papers the exact formal pairing), which orders every
     subsequent load in the single-threaded reactor, cross-arch. `read_volatile` gives no-elide/single-width
     — **NOT** atomicity/ordering.
   - **No-tearing, addressed separately:** aligned single-word (`u32` `DD`/status) accesses are atomic on
     x86; the multi-byte **payload** is safe because it is read only **after** `DD`=1.
   - **The stated residual (two boundaries, not the ordering):** the account is sound *on x86 + PCIe +
     coherent DMA + this device/fabric* (the deployment), and the **Rust abstract machine does not model
     device-DMA memory** (a hardware argument, not a language proof). Both **STATED**, honest; claiming
     general/unconditional soundness, or hiding the platform-dependence, is not.
2. **The ordering CODE** — realize the read-crossing ordering as an **`AtomicU32` `load(Acquire)`** for the
   `DD`-gating read in `descriptor_done` (formally correct cross-arch; supersedes the fence-after-volatile
   which pairs an acquire with a non-atomic load — a paper); and, if the TX write crossing is in scope, a
   **release** before the tail doorbell (redundant on x86 via the UC-MMIO ordering, but portable). Slice-
   sized; below the frozen seams (the guard/copy-boundary + reactor algorithm stay byte-frozen); tested
   only for control-flow (poll-`DD`-then-read intact — the ordering *effect* is a reasoning artifact, a
   no-op on x86, certified by reading placement + intent, not by a testable output).
3. **The `architecture.md` reconciliation SURFACE (§2.1/§3/§5/§6/§7)** — the nic team **grounds** (what
   6/7a exercised) and **surfaces a proposed DESCRIPTIVE diff** for the **designer** to ratify/rewrite;
   the nic team does **not** self-apply the design authority. The items:
   - **§2.1** — the containment *mechanism* reconciles: *"structural hardware: the IOMMU"* + the *(B)
     IOMMU-airtight* requirement → *we-contain-it-ourselves* (driver guard, vector-2), with the vector-1
     DMA-reach the owned residual. **Posture stands; NOT invalidated.**
   - **§3/§5** — the `epoll`/`eventfd` idle-wakeup → **bounded-poll** (no interrupt on the pure-direct path).
   - **§6** — the framing length source: the live 82599's sub-60B padding makes the descriptor length
     unreliable, so the true L1 length rides in the 802.3 length field (below the transport frame, which
     stays frozen; §6's no-transport-length-field decision + adversary-length bound intact).
   - **§7** — the retransmit-window-vs-bounded-poll-interval margin: the K=32 drain latency (~11.5 ms)
     reaches the 10 ms window (masked to 0-spurious by drain-before-deadline, but the margin is violated) —
     surfaced with the 7a numbers; §7's own remedy is dial-up-the-window.
4. **Update `concerns-to-watch.md`** — the crossing-soundness residual **grounded** (if the account
   discharges it) or **narrowed** to its irreducible platform-dependence and kept honest-open; the
   owned-DMA operational grounding (what sustained operation exercised).

**This sprint does NOT own** (named, with where it belongs):

- **The designer's REWRITE of `architecture.md`** — the nic team surfaces a *proposed descriptive diff*;
  `zero.nic-design` ratifies/rewrites the design authority (`[[normative-spec-is-the-authority]]`). The
  proposed diff is **descriptive, not prescriptive** of the new text — a starting point, not a decision.
- **The policy-layer audit verdict** of the owned-DMA posture — a cross-team activity; the nic team grounds
  what operation exercised, not the audit.
- **Any operational-stack change** — `reactor/`/`transport/`/`driver` guard+copy-boundary/`directpath`
  stay frozen; 7b touches only the `dma.rs` crossing (the fence) + the docs. No new features.
- **The §7 remedy decision** — dial-up-the-window vs characterize-the-poll-interval is the designer's §7
  call; the nic team surfaces the finding + the 7a numbers.

## Problem

7a left two residuals honest-open and now-exercised. **(1)** The `dma.rs` crossing (reading device-mutated
memory: the volatile access, the `DD`→payload visibility/ordering, no-tearing) is **undischarged** — the
operational runs leaned on **implicit x86-TSO + coherent DMA without explicit fences** = *path-works, not a
soundness proof* (`concerns-to-watch.md`; `architecture.md §3.1`). **(2)** `architecture.md` still describes
the **pre-pivot** model — §2.1's IOMMU-as-the-containment-mechanism, §3/§5's epoll reactor, §6's
descriptor-length, §7's window-untested-under-load — and the 6/7a operation exercised the reality that
diverges from each, ungrounded and unsurfaced to the designer.

## Goal

The crossing-soundness is either a **verified cited account** (sound on the stated platform, with the fence
making the CPU-side explicit) or a **narrowed honest-open residual** (the irreducible platform-dependence,
never papered); and the `architecture.md` §2.1/§3/§5/§6/§7 reconciliation is **grounded + surfaced** to the
designer as a proposed descriptive diff, with the live evidence. Nothing dressed as resolved that isn't.

> "This section commits the copy *constraint*, not a claim that its unsafe implementation is understood."
> — `architecture.md §3.1` (the residual 7b closes or honestly narrows).

## In Scope — the ordered slices

**Slice 1 — The crossing-soundness verified account + the fence code.** The dev authors the cited account
(grounded in the real `dma.rs` access + the 82599 datasheet writeback semantics + PCIe posted-write
ordering + the Intel SDM x86-TSO), naming/citing each guarantee and the composition, stating the platform
residual; and adds the **acquire fence** between the `DD` load and the payload load. *Method: the account is
a reasoning artifact certified by READING against its citations (not a run); the fence is code below the
frozen seams (guard/reactor empty-diff), tested; if the rigorous account is larger than this slice, the
irreducible part is kept honestly open, not papered.*

**Slice 2 — The architecture.md reconciliation surface + concerns-to-watch update.** Ground what 6/7a
exercised and draft a **proposed descriptive diff** for §2.1/§3/§5/§6/§7 (posture-stands framing for §2.1);
surface it to `zero.nic-design` via the PM. Update `concerns-to-watch.md`: the crossing-soundness
grounded-or-narrowed; the owned-DMA operational grounding. *Method: descriptive-not-prescriptive; the nic
team grounds/proposes, the designer rewrites; the §2.1 posture-stands framing held exactly.*

## Non-Goals

- **No self-applied `architecture.md` rewrite** — surface a proposed diff; the designer rewrites.
- **No "it ran" evidence** for the crossing-soundness — it is cited reasoning; a green run proves nothing
  about soundness.
- **No papered partial account** — an incomplete soundness argument is kept honest-open, not presented as
  full discharge.
- **No operational-stack change** — the frozen crates stay byte-frozen; only `dma.rs` (the fence) + docs.
- **No §7-remedy or policy-audit decision** — surface the finding/grounding; the designer/policy-layer decide.

## Method

- **Cited-reasoning-against-specs.** The account is checked by **reading** the ordering argument against its
  **cited primary** authorities (the 82599 datasheet, the PCIe spec, the Intel SDM) — matched to the real
  `dma.rs` access, gap-free across **both** directions, residual stated — zero hardware. **Reject (seven
  tells):** "it ran" / volatile conflated with atomic-or-fence / the device-PCIe write-ordering **assumed
  not cited** / no-tearing hand-waved / a **partial** account (e.g. only the read direction) presented as
  full discharge / **a reference implementation (ixy, Linux ixgbe) cited as the ordering AUTHORITY** in
  place of the primary spec ("everyone does it this way" is path-works, not the spec-guarantee) / **coherent
  DMA conflated with write-ORDERING** (coherence is a *visibility* property — the device's writes become
  visible; it does **not** order the *sequence*; payload-before-`DD` comes from PCIe posted-write ordering,
  not coherence).
- **The fence is CPU-side; the device/fabric ordering is a cited hardware property.** The acquire fence
  makes the CPU-side load-load explicit/portable; the payload-before-`DD` device write-ordering is a
  hardware property we **cite** (datasheet + PCIe), not one the fence controls — so the account is
  sound-on-the-stated-platform with an honest residual, not general soundness.
- **Design-of-record surfaced-not-self-applied** (`[[normative-spec-is-the-authority]]`). The nic team
  grounds (what operation exercised) + proposes a **descriptive** diff; the designer ratifies/rewrites. The
  §2.1 framing is **posture-stands, mechanism-reconciles** — reject any surface that says "invalidated."
- **Honest-open** (`concerns-to-watch.md` rule). Ground a residual only where actually shown; else narrow
  it to the irreducible core and keep it visibly open — a confident-sounding crossing-soundness "fix" is the
  exact failure the file exists to prevent.

## Acceptance — the gates

1. **The crossing-soundness account is CITED + falsifiable + platform-scoped, OR the residual is honestly
   narrowed.** Each guarantee named + cited: the payload-before-`DD` device/PCIe write-ordering **cited**
   (not assumed — the crux); x86-TSO LOAD-LOAD (SDM) cited; volatile-is-not-a-fence stated; the no-tearing
   split (aligned-`DD`-atomic + payload-read-after-`DD`); the platform residual stated. **Rejected:** "it
   ran," volatile-as-fence, assumed device/PCIe ordering, hand-waved tearing, a partial presented as full.
2. **The fence code lands + is correct** — an acquire between the `DD` load and the payload load in
   `dma.rs`; the guard/copy-boundary + reactor algorithm stay **byte-frozen** (empty-diff); tested; the
   fence placement (after the `DD` load, before the payload load) certified.
3. **The reconciliation is grounded + surfaced as a proposed DESCRIPTIVE diff** (§2.1 mechanism-reconciles-
   not-invalidated / posture-stands; §3/§5 epoll→bounded-poll; §6 the 802.3-length below the frozen frame;
   §7 the window finding + the 7a numbers) — **not self-applied** to `architecture.md`; routed to the
   designer.
4. **`concerns-to-watch.md` updated** — the crossing-soundness grounded-or-narrowed (the irreducible
   platform-dependence honest-open if not fully discharged); the owned-DMA operational grounding recorded.
5. Independent review **certifies the account against its cited specs** (not the dev's summary; reject
   papering per gate 1), the fence placement + the frozen-crates empty-diff, and that the reconciliation is
   **descriptive + surfaced-not-self-applied** with the §2.1 posture-stands framing.

## Review Questions

- **R1 (reviewer/PM — does the fence fully discharge, or a residual remains?).** *Lean: the acquire fence
  makes the CPU-side load-load explicit/portable, but the device/PCIe payload-before-`DD` write-ordering is
  a cited hardware property (not fence-controlled), so the account is **sound-on-the-stated-platform** with
  the platform-scoping as the honest residual — not general soundness.* Confirm the bar: a platform-scoped
  sound account (with the residual stated) is an ACCEPT; a claim of general/unconditional soundness is a
  REJECT.
- **R2 (PM — the reconciliation routing).** The nic team's proposed descriptive diff goes **to
  `zero.nic-design` via the PM** (as the Sprint-6 §2 grounding was proposed for ratification), not
  self-applied. Confirm the route.
- **R3 (PM/designer — the §7 remedy, surfaced not decided).** The window-margin finding: §7's own remedy is
  "dial the window up freely if a measurement surprises us." *Lean: surface the finding + the 7a numbers +
  the poll-interval/ring-depth interaction; the designer rules dial-up-the-window vs characterize-the-
  interaction vs both.* This is the designer's §7 call; the nic team surfaces, does not decide.

---

## Outcome & Retrospectives — 7b close (2026-07-18)

**Validated. The project's longest-open hazard is grounded.** Both 7b slices independently ACCEPTED by
`zero.nic-review` — certified as **reasoning + discipline artifacts checked by reading against primary
specs, zero hardware** (a run is not part of it). The `dma.rs` device-memory crossing (`architecture.md
§3.1`: "not analyzed to resolution") is now a **positive, platform-scoped, cited account** (both crossing
directions), the load-bearing device citations verified word-for-word against the local 82599 datasheet
(§3.1.4.4 item 2, §3.1.4.5.3), the CPU-side realized as an `AtomicU32::load(Acquire)` + a `fence(Release)`
before each doorbell (below the frozen seams). `concerns-to-watch.md` is updated per its own rule (the
crossing-soundness discharged into the grounding doc; only the narrowed platform + language-model residual
open). The `architecture.md` §2.1/§3/§5/§6/§7 reconciliation is a **proposed descriptive diff surfaced to
the designer** — architecture.md itself untouched.

**What remains owned/open (not discharged by 7b):** for the **policy layer** — the owned-DMA / containment
residual (no IOMMU ⇒ vector-1 DMA-reach owned, not contained; 7a exercised it at sustained scale, production
untouched — *operational path-works, not a containment proof*; the audit owns the verdict). For the
**designer** — the surfaced reconciliation to ratify/rewrite (§2.1 **posture-stands / mechanism-reconciles**,
not "invalidated"; the §7 window-remedy the designer's call). *Operational ≠ crossing-sound ≠ contained.*

### Lead retrospective — zero.nic-lead (Sprint 7b, 2026-07-18)

**A different kind of sprint, and the loop adapted without losing its spine.** 7b built almost nothing — it
was cited reasoning + grounded reconciliation. The evidence discipline held anyway: the "deliverable" is a
reasoning artifact certified by *reading it against its primary specs*, and a green run is explicitly *not*
evidence of soundness ("path-works ≠ crossing-sound"). The seam discipline still applied — the account and
the atomic-acquire code landed below the git-verified byte-identical reactor/transport/guard, the throughline
from the QEMU stack to a certified crossing account on live silicon.

**The escalation calibration I recorded last sprint held.** I flagged up front that 7b had *no operator
fork* — the account bar is the reviewer's to hold, the reconciliation and the §7 remedy surface to the
*designer* — and it stayed true: the operator ruled only the green-light and the commit word, nothing
manufactured. Adopting the team-converged answers and proceeding is the calibration that keeps the human's
attention for what is genuinely theirs (`[[dont-escalate-team-converged-decisions]]`).

**Design-of-record surfaced, not self-applied — and the operator's framing held exactly.** The reconciliation
is a *proposed descriptive diff* (architecture.md empty-diff); the nic team grounds and surfaces, the
designer rewrites the authority (`[[normative-spec-is-the-authority]]`). §2.1 held to the operator's
correction — the untrusted-but-contained **posture stands**, the *mechanism* reconciles (vector-2 contained
by the driver guard, only vector-1 DMA-reach owned) — **not** "invalidated," the over-broad framing I had
conceded. The reviewer even caught its own too-broad §2.1 phrasing earlier and updated its memory; the frame
was held on both sides.

**Honest-open, correctly sorted.** The crossing-soundness was *discharged* (moved to the grounding doc) and
the residual *narrowed* to the platform + language-model boundary; the owned-DMA/containment residual was
kept *open* with its operational grounding (path-works, not containment). Two residuals, cleanly separated,
neither papered — the `concerns-to-watch` rule applied to the letter. What's left is genuinely in others'
hands now: the designer (ratification) and the policy layer (the containment verdict).

### Developer retrospective — zero.nic-dev (Sprint 7b, 2026-07-18)

7b was a different kind of sprint — cited **reasoning** + grounded design-of-record reconciliation, not
building.

**Built / grounded:** the `dma.rs` crossing-soundness verified account (`grounding/crossing-soundness.md`)
— a cited reasoning artifact certified by reading against primary specs, both crossing directions (RX-read
DD→payload, the write stores→doorbell, the TX buffer-reuse control-flow leg); the ordering code below the
frozen seams (the DD read as `AtomicU32::load(Acquire)`, a `fence(Release)` before each doorbell); the
`architecture.md` reconciliation as a proposed descriptive diff (surfaced dev→lead→PM→design, not
self-applied); the `concerns-to-watch.md` discharge-per-rule.

**Evidence / findings (what turned the account positive):** the crux is **citable** — payload-before-`DD`
is verbatim in datasheet §3.1.4.4 item 2, and §3.1.4.5.3 makes it hold even with relaxed ordering on — so
Slice 1 became a *positive* platform-scoped account, not a narrowed-open residual. The
atomic-acquire-**on-the-load** catch (a `fence(Acquire)` pairs with an atomic load, not a `read_volatile`,
so fence-after-volatile would paper the exact pairing). Both-directions completeness + the TX buffer-reuse
control-flow leg (reclaim gated on `DD`=1, the reuse-store control-dependent + non-speculative → no acquire
needed). The two rejects held: coherence ≠ ordering; reference-impl ≠ authority.

**What remains owned/open:** the owned-DMA / containment residual (no IOMMU ⇒ vector-1 DMA-reach owned; 7a
exercised it at sustained scale, production untouched — path-works, not a containment proof; the policy-layer
audit owns the verdict); the narrowed crossing residual (platform-scope + the Rust abstract machine not
modelling device-DMA memory — a hardware-level cited argument, not a language proof); the reconciliation
surfaced for `zero.nic-design` to ratify + the §7 remedy the designer's call. Across 6 → 7a → 7b the seam
discipline was the throughline — the reactor + transport + guard/copy-boundary stayed git-verified
byte-identical from the QEMU stack to a certified crossing account on live silicon; every fix and this
account landed below them.

### Reviewer retrospective — zero.nic-review (Sprint 7b, 2026-07-18)

**Certified** (a reasoning + discipline certification — checked by reading against primary specs, zero
hardware): the project's longest-open hazard — the `dma.rs` device-memory crossing — is now a positive,
platform-scoped, cited account (`grounding/crossing-soundness.md`), both directions. The load-bearing device
citations I verified word-for-word against the local 82599 datasheet (§3.1.4.4 item 2; §3.1.4.5.3), the
standard PCIe/SDM specs against established knowledge (honest not-local provenance). The `architecture.md`
§2.1/§3/§5/§6/§7 reconciliation is a descriptive, surfaced-not-self-applied proposed diff — architecture.md
itself untouched.

**Findings that changed the plan:** the both-directions completeness gap (the account as first framed covered
only the read crossing; `dma.rs`'s own SAFETY comments named both — a read-only account presented as closing
the crossing is partial-as-full; the account landed both). Two rejects added to the bar (reference-impl ≠
authority; coherence ≠ ordering). The code realized as atomics, not fence-after-volatile (the acquire *on*
the load). The TX buffer-reuse control-flow leg (the reverse race closed by control flow — reclaim gated on
`DD`=1, the reuse-store control-dependent + non-speculative).

**What remains owned/open:** for the **policy layer** — the owned-DMA/containment residual (no IOMMU ⇒
vector-1 owned; 7a's sustained operation grounds what it *exercised*, not containment; the audit owns the
verdict — operational ≠ contained). For the **designer** — the surfaced reconciliation to ratify/rewrite
(§2.1 posture-stands/mechanism-reconciles, not "invalidated"; the §7 window remedy the designer's §7 call).
The crossing-soundness residual is now only the narrowed platform + language-model boundary — platform-scoped-
sound, never unconditional. One success proved the path; the soundness is the cited account, never a run.
