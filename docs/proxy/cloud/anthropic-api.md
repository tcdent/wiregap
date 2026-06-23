# The Anthropic API — The Core Cloud Endpoint and Its Capability Surface

### Exploratory research grounding the proxy's hardest egress destination

> An exploration document, companion to *Containment by Construction*, *The Console,
> Protocol & Multiplexer*, and the foundational *principles*. It records research into a
> single upstream destination — the Anthropic Messages API — that the proxy fronts on the
> box's behalf. It is the **worst-case instance** of the general posture formalized in
> [`approach.md`](./approach.md); read that for the cloud-wide principles this destination
> stresses hardest. It is research and theory used to inform the design; it is not the
> proxy's formal specification. Where it states a design direction, that direction is grounded in
> the spine (principally assertions 6, 7, 14, 17) and noted as such.
>
> The capability facts below were grounded against the live Anthropic API documentation on
> **2026-06-20**. The API surface drifts — tool type strings are date-versioned and the set
> grows — so the *enumerations* here are a snapshot, while the *structural conclusions* are
> meant to outlive any particular version string. That drift is itself a load-bearing fact;
> see [§6](#6-the-design-implication--an-intermediary-processor-emitting-one-inert-shape).

---

## 0. Why this endpoint, and why first

The box is a zero-trust execution environment that runs language-model agents. To function
at all, it must reach a model. Inference is the one capability the box cannot supply
itself, so the Anthropic Messages API is not an optional integration — it is **core to the
system's functionality**. The box's egress allowlist exists, in its first and most
essential entry, to permit exactly this call.

And it is the **worst-case** entry. Every other destination a human might curate onto the
egress allowlist (a package index, a documentation site, a Git remote) is reached the
ordinary way: the box addresses a host and a path, the proxy checks that `(host, path,
verb)` against the operator's standing authorizations, and either originates the call or
refuses it. The danger there is bounded by the destination the human reviewed.

The Anthropic API breaks that comfortable model, because **capability is selected in the
request body, not by the URL.** A request to `api.anthropic.com/v1/messages` can be a plain
text completion, or it can instruct Anthropic's own servers to fetch an arbitrary URL, run
code, or connect to a third-party endpoint — all to the *same host and path*, distinguished
only by JSON fields inside the (TLS-encrypted) body. A `(host, path, verb)` allowlist cannot
see the difference. The destination the human reviewed and the destination that punches
through containment are the same address.

> *"An exploratory endpoint that we consider core to our functionality and somewhat of a
> minefield in terms of its capabilities. It is really the worst case example and that's a
> good thing for us to have early in the design process."*

Holding the worst case early is deliberate. If the proxy's design survives this destination
— the one where capability hides in the body and the upstream is willing to act on the
box's behalf — every simpler allowlist entry is trivial by comparison. This document maps
the minefield so the design can be laid out against it from the start, rather than
discovering a buried capability after the architecture has hardened.

---

## 1. The shape of the request, and where the danger lives

The box originates a standard Messages API call: `POST /v1/messages` carrying a JSON body
with `model`, `messages`, `max_tokens`, sampling/thinking/effort parameters, an optional
`system` prompt, and — the load-bearing part — an optional `tools` array and a handful of
sibling fields (`mcp_servers`, `container`) plus request `anthropic-beta` headers.

Two facts about that body define the entire problem:

- **The box authors all of it.** Every byte of the request originates on untrusted ground.
  There is no field the box does not control.
- **Some of those fields are not data for the model — they are instructions to
  Anthropic's infrastructure.** A `tools` entry of the right shape, or an `mcp_servers`
  array, tells Anthropic's servers to *do something* and return the result.

The consequence is that any "safety" property the API documents about these fields is, for
us, void. The web fetch tool documents that it "can only fetch URLs that have previously
appeared in the conversation context" — but the box *writes* the conversation context, so
it seeds whatever URL it wants into a message and the restriction evaporates. Documented
restrictions constrain *Claude*; they do not constrain the *author of the body*, and under
our threat model the box is the author.

---

## 2. The first line: server-executed vs client-executed

Every capability the Messages API exposes sorts on one question:

> **Does Anthropic's infrastructure execute the tool, or does the client execute it?**

- **Client-executed tools are inert** from the API's perspective. The API only emits a
  `tool_use` block *describing* what it wants done; the client (here, the box) performs the
  action and hands back a `tool_result`. The API itself reaches nothing. The box running
  something is, by the spine, a contained non-event — its internal freedom is total
  (principles §19) and nothing it does inside reaches outside (§6). **These are out of
  scope.**

  > *"Anything that runs on the client is outside of our scope. We do not need to worry
  > about it."*

- **Server-executed tools run on Anthropic's infrastructure.** Anthropic's servers perform
  the action — fetch a URL, run code, connect to an endpoint — and return the result to the
  box. These are the only Messages API features that can matter to containment, because
  they are the only ones where something *other than the box* acts at the box's direction.

This line cleanly removes the large client-side category from consideration: bash, the text
editor, the memory tool, self-hosted computer use, and every custom `{name, description,
input_schema}` tool are the box's own business. What remains to examine is the server-side
set.

---

## 3. The second line: effect, not information

The server-side set is *not* uniformly dangerous, and conflating its members is a mistake
the design must not bake in. The spine does not defend information; it defends effect.

> *"We are not trying to firewall the box from access to information. Because the box is
> zero-trust, the box can acquire any information it wants. The box is not to execute code
> outside of its own bounds however."*

Confidentiality is explicitly discarded (principles §7): the box may egress whatever it
wants, and a leaked credential is revocable, not a defended harm. What is defended is
*reachable execution* — externally-effective action beyond the box's boundary (§6). So the
right question for each server-side tool is not "does information move?" but **"does the box
cause an effect beyond its bounds?"**

Applying that test splits the server-side tools in two:

- **Effect-bearing** — the box conscripts Anthropic's servers to *act* at a box-named
  destination, beyond the reach the operator authorized. These genuinely breach
  containment.
- **Information-or-inert** — the box gets Anthropic to *read* something, or to compute over
  the request's own contents. No external effect. Under the strict invariant, these do
  **not** breach containment.

The canonical illustration is the difference between web fetch and web search, examined in
[§5](#5-web-fetch-the-canonical-worst-case).

---

## 4. The capability surface (grounded 2026-06-20)

### Server-executed — Anthropic's infrastructure acts

| Capability | How it is selected in the request | Class | What Anthropic's infra does |
|---|---|---|---|
| **Web fetch** | `tools: [{ "type": "web_fetch_20250910" \| "_20260209" \| "_20260309" \| "_20260318", "name": "web_fetch" }]` (GA, no beta header) | **Effect** | Fetches a **box-named URL**. A GET can be a state-changing action (webhook, `?action=…`, an internal endpoint), executed from Anthropic's network position. |
| **MCP connector** | top-level `mcp_servers: [{ "type": "url", "url", "name", "authorization_token" }]` **plus** `tools: [{ "type": "mcp_toolset", "mcp_server_name" }]`; beta header `mcp-client-2025-11-20` | **Effect** | Connects outbound to a **box-named HTTPS MCP server** and runs its tools — including *write* tools (`create_*`, `delete_*`). Direct effect at an arbitrary endpoint. |
| **Code execution** | `tools: [{ "type": "code_execution_20250825" \| "_20260120" \| "_20260521", "name": "code_execution" }]`; `container` request param; `container.skills` for Agent Skills; beta `code-execution-2025-08-25` on some paths | **Effect (unverifiable)** | Runs box-authored Python/bash in Anthropic's sandbox. Documented as network-isolated — but isolation is unverifiable from our side, and the sandbox demonstrably reaches the web when it backs web fetch/search dynamic filtering. |
| **Advisor** | `tools: [{ "type": "advisor_20260301", "model": … }]`; beta `advisor-tool-2026-03-01` | Inert (compute) | A server-side secondary model call. No external reach, but a server-executed shape. |
| **Tool search** | `tools: [{ "type": "tool_search_tool_regex_20251119" }]` or `"_bm25_20251119"`, with other tools marked `defer_loading: true` | Inert (compute) | Regex/BM25 match **over the request's own tool list**. Reaches nothing external. |
| **Web search** | `tools: [{ "type": "web_search_20250305" \| "_20260209" \| "_20260318", "name": "web_search" }]` (GA, no beta header) | **Information** | Runs a **box-supplied query** against Anthropic's own search index and returns results. The box names a *query*, never a *destination*. |

### Client-executed — the box (or its harness) acts; out of scope

| Capability | How it is selected | Who executes |
|---|---|---|
| **Custom tool** | `{ "name", "description", "input_schema", "strict"?, "allowed_callers"? }` — no server `type` | The box. API only emits a `tool_use` block. **This is the inert shape.** |
| Bash | `{ "type": "bash_20250124", "name": "bash" }` | The box |
| Text editor | `{ "type": "text_editor_20250728", "name": "str_replace_based_edit_tool" }` | The box |
| Memory | `{ "type": "memory_20250818", "name": "memory" }` | The box |
| Computer use (self-hosted) | computer-use tool type | The box |

A subtlety worth recording: a server `type` tag does **not** by itself mean server-executed.
Bash, text editor, and memory are Anthropic-*defined* (they carry built-in `type` strings)
yet client-*executed*. The dividing line is *who runs it*, not whether the entry has a
`type`. This matters for [§6](#6-the-mechanism--verify-and-deny-never-modify-the-payload):
the recognizer admits only the bare custom shape, so it cannot lean on "tools with a
known-good type" — only `tools` entries with no `type` at all are recognized.

---

## 5. Web fetch: the canonical worst case

Web fetch is the sharpest single example, and the cleanest teaching case for why the body
must be processed rather than waved through.

A fetch is an HTTP GET, and most GETs are reads. But the box names the **exact URL**, and a
box-named GET is not necessarily a read: it can be a webhook trigger, a state-changing
`?action=delete` endpoint, or a request to an internal service. That is external **effect**,
not information. And it executes from **Anthropic's network vantage**, which can reach
destinations the box's own egress allowlist would never authorize — so the box gains effect
at endpoints the operator never reviewed, defeating the standing-authorization model
(principles §14) by routing around it.

Contrast web search. There the box supplies a **query string**, not a URL. Anthropic reads
its own search index and returns results. The box cannot name an endpoint to act upon — only
a phrase for Anthropic to read. The query leaving is information egress, which we do not
defend; the result returning is information acquisition, which the box is entitled to. Under
the strict effect-only invariant, **web search does not breach containment.**

That query-vs-URL asymmetry is exactly why Anthropic's own documentation carries a
data-exfiltration *Warning* on web fetch and none on web search. It is the boundary between
"the box read something" and "the box made Anthropic act."

The lesson the worst case teaches: **the harm is effect, never disclosure.** Any framing of
the proxy's body-processing as "stopping data from leaving" is wrong and contradicts §7. The
correct framing is "preventing the box from conscripting Anthropic's servers to act beyond
the box's authorized reach."

---

## 6. The mechanism — verify and deny, never modify the payload

Because capability rides in the body, the proxy's role on this hop is more than a pure
destination-matcher. It is an **intermediary processor** of the Anthropic API request — but
it processes by *recognizing*, not by *rewriting*.

> *"What we are adopting is perhaps an intermediary processor on the Anthropic API
> requests."*
>
> *"Authorship, in the sense that it reads, verifies, and encapsulates in SSL. But not
> authorship in the sense that it is expected to modify the payload."*

The proxy **reads** the box's request, **verifies** it against the **route's rule** — for the
Anthropic route, *inert client-side shapes only* (§2–§3) — and **encapsulates** a conforming
request, verbatim, inside its own authenticated TLS to Anthropic. It does **not** modify the
payload. This is assertion 17 in force — *the proxy enforces that emitted requests adhere to
the rules set for the route* — with the inert-shape requirement living as a **route rule**,
not as spine text. The box owns the body; the proxy owns the transport, the credential, and
the forward-or-deny decision. It is a gate, not an editor.

### Why not rewrite: block in the open, don't launder intent

Enforcement is **verify-and-deny, never rewrite.** The proxy does not edit a non-conforming
request into an acceptable one. Silently rewriting a request would **launder the box's
adversarial intent** — taking a request that reaches for forbidden capability and quietly
massaging it into one the upstream accepts. That is a dangerous precedent: the boundary would
be *manufacturing* conformance instead of *enforcing* it, and an injected agent's attempt
would vanish into a clean request rather than surface as a refusal. The proxy blocks in the
open; it never turns the box's request into something acceptable on the box's behalf.

### Why not rewrite: the body is signed (the reinforcing, practical reason)

Claude Code embeds a per-request **body-integrity hash** in the request — the `cch` value, an
`xxHash64` over the compact-serialized body, carried in a billing pseudo-header inside the
`system` array. It covers the whole body (messages, tools, metadata, model, thinking config);
editing, adding, or removing *any* of it flips the request to an upstream `400`. So any
modification of the body — including stripping a capability-bearing tool — invalidates the
signature, forcing the proxy to re-derive it from a **reverse-engineered, per-release-drifting
scheme**. That is real trusted-surface growth and a fragile maintenance coupling to an
undocumented mechanism.

Not modifying the payload dissolves the problem: the box signs its own body, the bytes ride
through unchanged, the signature stays valid, and the proxy carries **zero signing code** — it
is signature-agnostic. (A box using the raw API with no such hash is handled identically; the
proxy never touches the body either way. Whether the box authenticates as Claude Code or via
the raw API is a deployment question for the operator, not a containment one.)

### How verification stays robust: an exact-match whitelist of client-side tools

The check is a **whitelist by exact string**, never a denylist of known-bad types — the
server-side type strings are date-versioned and **growing** (`web_fetch_20250910 → _20260209
→ _20260309 → _20260318` in under a year), so a list of what to *block* is a bet that must
stay complete against an adversary who needs one gap (the unsound move principles §12 forbids).
A whitelist of what to *admit* carries no such bet: anything not on it is denied. What the
recognizer admits, exactly:

- **Custom tools** — `{name, description, input_schema}` with no `type`. **Not enforced** —
  they pass unchecked, because a client-side tool cannot affect the outcome (the API only
  emits a `tool_use` block for the box to run, and the box's own execution is contained).
- **`type`-tagged tools** — admitted only when the `type` is an **exact string match** to a
  known, vetted, client-side identity. Exact: `text_editor_20250728` is one string;
  `text_editor_20260101` is a *different* string and does not match. No prefix rule, no "a
  number may follow," no version fuzzing.

Everything else **fails closed**: a server-side `type` (web fetch, code execution, web/tool
search), an `mcp_servers` array, a `container` field, a capability-gating beta header, an
unknown `type`, or a new *version* of a previously-vetted tool. The verb (`POST`), domain
(`api.anthropic.com`), and path (`/v1/messages`) are likewise exact matches, denied otherwise.

Failing closed is the design working, not a regression. When Claude Code bumps a tool's date
stamp the new identity isn't on the whitelist, so egress to the route stops *legibly* — the
failure names the unvetted tool. The operator reviews what it is and what it can do, and
extends the whitelist only if it is inert/client-side (the effect-not-information line) — the
promotion pattern (principles §14) at the route-rule level. A capability Anthropic ships next
quarter is, by construction, not on the list → denied, with no proxy change and no chance of
silently widening. This is the spine's **native idiom** — the byte-map and leaf-validator
admit the known and deny the rest; they do not rewrite — and it is exactly why leaking the rule
to the box costs nothing: an exact-match whitelist has no gap to find.

### The recognizer is the entire guarantee

Because the bytes ride through unchanged, **there is no emit-side backstop.** Under a design
that rebuilt the request, a missed capability would simply not be re-emitted; here, a
recognizer that is too permissive *forwards* it. So the recognizer **is** the containment, and
it lives in the **security-invariant core** — total and reviewed, held to the reviewability
budget (principles §15) — not in feature-logic. Only deny-*unless*-proven-wholly-inert is
sound; a partial "catch-known-bad" check leaks, because nothing rewrites behind it.

### Non-conformance is denied, not silently fixed

Not modifying the payload **forecloses silent sanitization by construction**: the only
responses are forward-the-whole-request or deny it — the proxy never strips-and-proceeds. The
denial is surfaced **legibly**: a box reaching for a forbidden capability is visible to the
operator (the golden-period legibility of *Containment by Construction* §0), and the contract
("send an inert request or be refused") is a total function, specifiable and reviewable.

The harmless server-side tools (web search, tool search) are denied along with the rest, and
that is correct:

1. **They offer the box nothing.** The box is a full client. It can search the web through
   its own egress (a search endpoint on the allowlist) and filter its own tool list in its own
   harness. Web search and tool search are conveniences the box already owns; the only thing
   the server-side variants add is *doing it on Anthropic's infrastructure instead of the
   box's own* — which is precisely what we refuse.
2. **The recognizer cannot reliably tell them apart from the dangerous ones.** Web fetch
   *masquerades as a read* (it is a GET) but carries effect. Carving out "the harmless
   server-side tools" reopens per-tool danger reasoning over an adversary-controlled, drifting
   shape — back to the denylist trap.
3. **Each admitted shape grows the recognized surface for no containment gain** (principles
   §15, §17). The discipline is to make no exceptions, however harmless an individual feature
   looks.

The unifying observation behind all of it: **every server-side tool is "Anthropic runs a
client capability for you."** Code execution is Anthropic-as-your-sandbox, web fetch is
Anthropic-as-your-HTTP-client, the MCP connector is Anthropic-as-your-MCP-client, web/tool
search is Anthropic-as-your-retrieval-layer. The system's whole premise is that **the box
is the client/runtime** — untrusted, contained, fully capable inside its boundary. So
outsourcing any client capability to Anthropic is simultaneously *unnecessary* (the box has
it) and, for the effect-bearing ones, *forbidden* (it relocates the box's reach onto
infrastructure the operator's authorizations cannot see). The proxy's job on this hop is to
hold that line structurally: keep the box doing its own client work, and never let it
conscript Anthropic's servers as a second runtime.

---

## 7. What this deliberately does not admit

The whitelist admits custom tools (unenforced) and exact-matched vetted client-side
identities. Everything else is denied — none of it a real capability loss:

- **Every server-side capability vector** — web fetch, web search, code execution, the MCP
  connector (`mcp_servers`), the code-execution `container`, capability-gating beta headers.
  Never on the whitelist, by class. (Web search and tool search are server-side but inert in
  *effect* per §3; the whitelist carves no exception — they are server-side identities, so they
  are simply not on it.)
- **Any unvetted or newly-versioned `type`-tagged tool**, including a new date stamp on a
  previously-vetted tool. It fails closed until the operator reviews the new version and
  re-admits it if it is still inert — the boundary refusing to widen itself.

What is *not* excluded: **Anthropic's branded client-side built-ins** (`text_editor_*`,
`bash_*`, `memory_*`) are admissible by their exact vetted identity, so Claude Code's real
tools work — they simply re-vet on each version bump. And **custom client-side tools** pass
freely, since nothing the box runs itself can affect the outcome.

---

## 8. Open questions for the design

This document settles the *what* (the capability line, the effect-vs-information
refinement, the verify-and-deny mechanism). It leaves open design questions that belong to
the proxy's own specification work:

- **Placement of the recognizer.** Whether the Anthropic-request recognizer runs as a module
  bound to the proxy's transport layer, or as a separate isolated service the proxy delegates
  to, is undetermined — left open out of caution and correctness, not laziness. The recognizer
  reads attacker-authored JSON, so where it sits relative to the rest of the trusted core is a
  real boundary decision.
- **The exact recognized shape.** Precisely which fields and tool shapes the recognizer
  accepts (everything else denied) is an implementation detail for the builders, to be fixed
  against the then-current API and frozen as part of the trusted surface.
- **The recognizer is the security-invariant core, and must be reviewable as such.** It parses
  box-originated input with **no rewrite behind it**, so its totality *is* the guarantee:
  deny unless the request is proven wholly inert. It must be small and reviewable in a sitting
  (principles §15), recurse to full depth, and default-deny on anything unrecognized. This is
  the specific, named expansion §17 sanctions, and it should be reviewed as a security
  invariant — not feature-logic — when the spec hardens.
- **Authentication path is the operator's.** Whether the box presents as Claude Code (carrying
  the `cch` body hash) or uses the raw API (no integrity hash) is a deployment/technical
  question for the operator; either way the proxy does not touch the body, so its mechanism is
  unaffected.

The takeaway for the design that builds on this research: the hardest egress destination in
the system is the one we depend on most, its danger is structural (capability in the body,
not the address), and the sound answer is structural too — recognize the request completely
and admit only the inert shape, denying the rest, so the minefield becomes a surface with
exactly one safe path across it.
