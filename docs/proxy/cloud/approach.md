# The Cloud Egress Approach

### How the proxy treats upstream destinations

> An exploration document formalizing the proxy's posture toward the **cloud** — any
> upstream destination the box reaches through the proxy. It generalizes the reasoning
> worked out against the hardest single destination, the Anthropic Messages API, which is
> documented as the worst-case instance in [`anthropic-api.md`](./anthropic-api.md). These
> principles handle that worst case and subsume the simpler destinations. Companion to
> *Containment by Construction* and the foundational *principles*; grounded in the spine
> (principally assertions 6, 7, 12, 14, 15, 17). It is research and theory that informs the
> proxy's formal specification; it is not that specification.

---

## 0. The cloud is one destination class, with a worst case

The box reaches nothing on its own; every outbound call passes through the proxy, which
holds the credential, the link to the upstream, and the operator's curated set of permitted
destinations. "The cloud" is that whole outbound surface — a package index, a documentation
site, a Git remote, the Anthropic API. The destinations differ only in **how much
capability they carry in-band**:

- Most expose capability only through the *address* — a host, a path, a verb. The danger is
  bounded by the destination the operator reviewed.
- A few carry capability in the *body* — the same address can mean "read text" or "run code
  / fetch an arbitrary URL / act on the box's behalf," distinguished only by fields inside
  the request. The Anthropic API is the sharpest such case.

The approach below is specified to survive the second kind. A destination that only exposes
host+path+verb is strictly easier and falls out as a special case of the same machinery.
Holding the worst case as the design target is deliberate: if the proxy is sound against a
destination where capability hides in the body and the upstream is willing to act for the
box, every simpler destination is trivial.

---

## 1. The proxy owns the upstream call, not the payload

The box's request is something the proxy **reads, verifies, and — if it conforms —
encapsulates verbatim** inside its own authenticated TLS to the upstream. The proxy owns the
transport, the credential, and the forward-or-deny decision; the box owns the payload,
including any client-side request signing the payload carries. **The proxy is a gate, not an
editor: it does not modify what the box sent.**

This generalizes *Containment by Construction* §3.2 — "the boundary device does not route
the box's packets; it originates its own fixed upstream call" — at the transport and
encapsulation level, without extending it to rewriting the content. The decisive property —
that nothing forbidden reaches the upstream — is achieved by §2's recognition, not by
composing a fresh request. Authorship here means *owning the call*, never *authoring the
payload*.

---

## 2. Admission is an exact-match whitelist; the unrecognized is made unreachable

A route's rule is a **whitelist by exact match** of the known-permitted request — never a
denylist of known-bad. The proxy admits a request only by recognizing it against that
whitelist and **forwards verbatim only what it fully recognizes**; anything outside the
whitelist — an unknown field, a capability-bearing shape, a changed identity — **denies the
whole request**, making the forbidden thing **unreachable** (principles §12's second
structural mode — the egress allowlist's own logic, extended from the *destination* to the
*request shape*). The match is **exact**: an exact verb, an exact domain, an exact path, an
exact identity — no prefix rule, no version fuzzing, no "a number may follow."

Two properties follow, and they hold for **every route** — this is the general cloud rule, not
a quirk of any one upstream:

- **It fails closed; the future cannot push anything through.** A capability or identity the
  upstream introduces later is, by construction, not on the whitelist → denied, with zero
  proxy change. The drift that defeats a denylist (date-versioned, growing capability strings)
  is a non-event for a whitelist: it simply isn't matched.
- **Only what can affect the outcome is enforced.** Inert content — anything the box's own
  side executes, or that cannot cause effect — is not policed; the whitelist gates exactly the
  elements that could become reachable effect and lets the rest pass. (On the Anthropic route:
  server-side tool identities are whitelisted-or-denied by exact string; client-side tools pass
  unenforced — see [`anthropic-api.md`](./anthropic-api.md) §6.)

This is the spine's **native idiom** — the byte-map and the leaf-validator admit the known and
deny the rest; they do not rewrite — and it is why leaking a route's rule to the box costs
nothing: an exact-match whitelist has no gap to find.

---

## 3. The recognizer is the entire guarantee, and the contract is deny-or-forward

Because the bytes ride through unchanged, **there is no emit-side backstop**: under a design
that rebuilt the request, a missed capability would simply not be re-emitted; here, a
recognizer that is too permissive *forwards* it. So the recognizer **is** the containment,
and it lives in the **security-invariant core** — total and reviewed, held to the
reviewability budget (principles §15) — not in feature-logic. It must deny *unless* the
request is proven wholly inert; a partial "catch-known-bad" check leaks, because nothing
rewrites behind it.

Enforcement is **verify-and-deny, never rewrite.** The proxy does not edit a non-conforming
request into an acceptable one — silently rewriting a request would **launder the box's
adversarial intent**, massaging a reach-for-capability into a request the upstream accepts.
That is a dangerous precedent: the boundary would be *manufacturing* conformance rather than
*enforcing* it, and an injected agent's attempt would vanish into a clean request instead of
surfacing as a refusal. The proxy **blocks in the open.**

Not modifying the payload also **forecloses silent sanitization by construction**: the only
responses are forward-the-whole-request or deny it — the proxy never strips-and-proceeds. The
denial is surfaced **legibly**, where rewriting-into-acceptability would erase the signal a box
reaching for forbidden capability gives the operator (the golden-period legibility of
*Containment by Construction* §0). The contract — "send an inert request or be refused" — is a
total function over the input, specifiable and reviewable by enumeration, which is what the
guarantee rests on (§15–16), not the accreted silent fixups the design refuses (§1, terraces).

---

## 4. The line is effect, not information

A destination, and a request shape, is permitted to the extent it lets the box **acquire
information** — which is free, because confidentiality is not defended (principles §7) — and
forbidden to the extent it lets the box **cause external execution or effect** beyond its
bounds (§6). Curation turns on "can this be made to escalate to effect," never on "does data
leave." Framing any part of the cloud approach as stopping exfiltration is wrong and
contradicts the spine; the proxy stops the box from making the upstream *act*, not from the
box *knowing*.

---

## 5. Egress is the sensitive surface; the response is a free relay

The outbound request is where the trusted surface and the legible contract live. The inbound
response is the opposite: it lands on **untrusted ground** — the box — where interpretation
is already assumed and harmless (console §1.1). So the proxy relays the response **opaquely**
and **never** inspects, neutralizes, modifies, or sanitizes it. This holds for every cloud
destination, because the response always flows to the box, never onto trusted ground.

This asymmetry is the payoff of the zero-trust posture: hardening goes on egress, where the
box could otherwise reach effect; ingress falls out for free. The temptation to "sanitize
what comes back from the cloud" is to be refused — it would grow the trusted surface to
defend a direction that needs no defense and would re-import the confidentiality worldview
the design discarded.

---

## 6. Routes are the operator's standing-authorization register

Each authorized destination is a **route**: a destination together with the **rules** an
emitted request to it must satisfy. The set of routes is the operator's curated register of
standing authorizations (principles §14), not a list of "safe" addresses. Assertion 17 is the
proxy's whole charge here — *it enforces that emitted requests adhere to the rules set for the
route.*

The rules are **route-specific, not spine text**. For an inert destination the rule may be
only "an allowlisted GET"; for the Anthropic route the rule is "inert client-side shapes only"
(§2–§3). This is why the spine assertion is mechanism-agnostic: it mandates *that* the proxy
enforces each route's rules, and leaves *what* those rules are to the route. The register
governs which routes exist; the route's rule governs what an emitted request may be; the proxy
**never widens either on its own** — no redirect-following (a redirect is the box reaching an
unreviewed destination behind an approved one), no destination the box can influence at
runtime, no rule the box can author.

When a route's legitimate traffic changes — an upstream increments a tool's version, adds a
path, rotates an identity — the proxy **fails closed** until the operator reviews the change
and extends the rule. That review *is* the operator widening the register, deliberately, after
seeing what the new thing does — the **promotion pattern** (§14) at the route-rule level. The
proxy never makes that judgment itself; it refuses, surfaces the refusal legibly, and waits.
This loop is the same for every route, capability-bearing or inert.

**The register is Rust, not a config file.** The register and each route's rule are expressed as a
**per-service Rust DSL** — compiled-in code the operator writes and reviews like any other part of
the TCB — not a serialized config format (no YAML/JSON/TOML file loaded at runtime). This is a
deliberate reviewability choice (operator direction, 2026-07-19): a serialized register would need
a parser on the trusted side and a runtime load/validate step whose defect would alter the tier-1
gate's input; expressing it as typed Rust removes that parser entirely, makes an invalid route or
tool-identity a **compile error rather than a runtime check** (§12's structural mode applied to
configuration), keeps the box structurally unable to author it (compiled in, never loaded from
anywhere box-reachable — §14), and makes the register a reviewed artifact the operator holds in a
sitting alongside the rest (§15). A register change is therefore an **edit, recompile, and
re-review** — the promotion pattern (§14) at the source level, and the answer to what re-triggers
the reviewability sitting for a register change.

---

## 7. A request is a trusted address and an untrusted payload

Every web request the box makes decomposes into two kinds of data, classified by **trust** —
and the classification governs what the proxy *represents and keeps*, never what it *allows*
(gating is orthogonal to it and unchanged by it):

- **Trusted — the address: scheme, domain, path.** Bounded and *very definable*. It reuses the
  file leaf's **safe segment stack** — the same scanned-segment machinery and restricted, loggable
  alphabet (`proto/net.md` §1; `proto/model.md` §6) — positioned for a URL rather than duplicated
  into a separate system: a fixed `scheme` prefix and a position convention make it a distinct
  *address*, while the encoding is shared. The box supplies the segments — the first is the domain,
  its dots carried *within* that segment's safe alphabet, the rest the path — and the proxy supplies
  the structure it may not author, `://` and `/`. A separator or control byte is unrepresentable, so
  the assembled address is safe by construction. Spec-complete for the address it must carry and
  incapable of expressing a dangerous byte, it is the only box-originated data the proxy puts into
  its own representation, matches against the route (§2, §6), and writes to a log.

- **Untrusted — the payload: query parameters, headers, body** (for GET and POST alike; a GET's
  query string is payload, not address). A separate field. It is **never** put through the safe
  representation, never logged, never rendered. The proxy reconstructs it into the actual HTTP
  request on the way out, and that is the only thing it is ever used for.

This is a **data classification, not a gating change.** It governs which bytes are clean enough
to *keep* — to represent, match, and log — and which are only ever passed through transiently.
The route's gating (the whitelist, the per-route rules of §2–§3) is unchanged and operates
exactly as before; this layer only governs representation and persistence. A route may still
inspect an untrusted payload in-memory to make its allow/deny call (the Anthropic route reads
the body for its tool whitelist) — inspected for the decision, yes; represented or persisted,
never.

The audit consequence falls out for free: **the only box-originated content that ever reaches a
log is the address, which is safe by construction.** The dangerous content — the payload — is
structurally barred from logs because it never enters the representation logs are drawn from. A
route's audit trail is *"request to [safe address] — allowed or denied, and why,"* with no
payload in it and no neutralization pass required. This is the structural answer to "logs
detonate when rendered in a terminal": there are no dangerous box bytes in the log to detonate.

---

## 8. The uniform shape, and where the surface lives

Every route is the same three-step shape:

> **match** the route against the register → **recognize** the request against that route's
> rule → **forward** it verbatim inside the proxy's TLS, or **deny** it.

What varies is only the recognizer's weight, because it tracks the route's rule. For an inert
route it is "is this an allowlisted GET?" — near nothing. For the Anthropic route it is a full
recursive recognizer over the request body. The proxy is, in the operator's framing, an **intermediary processor**
for cloud egress — but one that processes by *recognizing*, not *rewriting*.

This is an honest statement about the trusted surface. For capability-bearing destinations
the egress guard is a per-destination request **recognizer** that reads box-controlled input,
so it sits on the box-reachable surface — and, because nothing rewrites behind it, in the
**security-invariant core** (principles §15). The worst-case destination sets the high-water
mark for that surface; the simpler destinations add nothing to it, so keeping
capability-bearing destinations *few* is how the trusted surface stays small. And because the
proxy does **not** modify the payload, it carries no re-serialization and no upstream signing
code — any client-side integrity signature the box embeds (e.g. Claude Code's body hash)
rides through intact. The surface is **recognition only**.

---

*The cloud is reached only through calls the proxy reads, verifies, and forwards unchanged —
admitting only the inert request and refusing the rest, encapsulated in TLS the proxy owns,
to a register the operator owns and the proxy never widens. The box may learn anything out
there; it may make nothing out there act. Everything the upstream sends back lands on
untrusted ground and is relayed untouched. The worst-case destination — capability in the
body, an upstream willing to act — is the one the approach is built against, so every simpler
destination is already covered. Its grounding is [`anthropic-api.md`](./anthropic-api.md).*
