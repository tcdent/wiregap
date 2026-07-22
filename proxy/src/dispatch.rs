//! Sprint 6 S1 — the codec + dispatch layer (tier 2, `architecture.md` "wire-protocol parser"):
//! the demux that sits between the reactor's frame delivery and the services. It **decodes** proto's
//! L2 message, **binds** each session to a service by its first frame's verb, **routes** every
//! subsequent frame of that session to the bound service, and at the **tier-1 seam** hands gated
//! channels into the certified security core — never re-implementing or duplicating what the core
//! certifies.
//!
//! This module is a **sibling to the tier-1 core, never inside it** (`proxy::{leaf, address, route,
//! anthropic, register, vocab, open_discipline}` are the certified core; this consumes them). Its
//! own bar is memory-safety + correctness + liveness, **not** security logic — routing is by
//! *envelope* (session id, verb), **never** by content (`contract/box-proxy.md` §1.3, §1.4; the one
//! security-relevant wiring invariant, MB3).


use proto::{FileWrite, NetAddress, NetPayload, Reassembler, ReassemblyState, Reject, Wire, verb};
use transport::SessionId;

use crate::anthropic::{Denial, Request, recognize};
use crate::vocab::{Route, ToolIdentity};

/// Peek the L2 verb — the first `u32` (LE) of the payload — **without decoding the body**. This is
/// the whole of the deep-parsing the proxy does for a *relay* channel (`proxy-head.md` §1,
/// `library.md` §7: "decodes L1/session routing and the binding verb only; deep-parses no
/// terminal-service L2 in either direction"). `None` if the payload is shorter than the verb.
#[must_use]
pub fn peek_verb(payload: &[u8]) -> Option<u32> {
    payload
        .split_first_chunk::<4>()
        .map(|(v, _)| u32::from_le_bytes(*v))
}

/// The four services a session can bind to — one per verb block (`model.md` §8 verb-number blocks:
/// `0x0_` control, `0x1_` terminal, `0x2_` file, `0x3_` net).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    Control,
    Terminal,
    File,
    Net,
}

/// The service a verb belongs to — its block (the high nibble of the low byte). A verb outside the
/// four defined blocks has no service and is unsupported. **Pure routing by verb; no content.**
#[must_use]
pub fn service_of_verb(verb: u32) -> Option<Service> {
    match (verb & 0xff) >> 4 {
        0x0 => Some(Service::Control),
        0x1 => Some(Service::Terminal),
        0x2 => Some(Service::File),
        0x3 => Some(Service::Net),
        _ => None,
    }
}

/// The disposition of one delivered frame after decode + binding + routing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Routed {
    /// A single-frame **gated** decision made at the tier-1 seam: the net route matched (admit),
    /// carrying the matched [`Route`].
    NetRouteAdmit(Route),
    /// A single-frame gated decision: the net address was denied by the tier-1 core (reserved
    /// scheme or no matching route).
    NetDeny,
    /// A single-frame gated decision: the file-write destination passed the tier-1 leaf check
    /// (via `leaf_validate_prescanned` — the single-scan-site path, RQ2).
    FileWriteAdmit,
    /// A single-frame gated decision: the file-write destination was denied by the tier-1 core.
    FileWriteDeny,
    /// A `net-payload` chunk accumulated into the session's [`Reassembler`]; its header flag was
    /// `more`, so more chunks are expected — no decision yet (the body is incomplete).
    NetPayloadAccumulating,
    /// The final `net-payload` chunk completed the request body, which the tier-1 Anthropic
    /// recognizer ([`recognize`]) **ADMITTED**. Carries the reassembled, verbatim body — the exact
    /// bytes the recognizer vetted and the exact bytes to forward upstream (CB4-across-the-seam:
    /// recognized == reassembled == forwarded, no split, no rewrite).
    NetPayloadAdmit(alloc::vec::Vec<u8>),
    /// The completed request body was **DENIED** by the tier-1 recognizer — carries the legible
    /// [`Denial`]. Fail-closed; the body is not forwarded.
    NetPayloadDeny(Denial),
    /// A `net-payload` completed on a session with **no admitted `Route`** (no prior `net-address`,
    /// or one that was denied) — so no recognizer envelope can be sourced. Fail-closed: the body is
    /// **not** recognized and **not** forwarded (Sprint 8's no-prior-Route-denies rung). This is
    /// distinct from a recognizer [`Denial`]: recognition never runs, because there is nothing to
    /// vet the request *as*.
    NetPayloadNoRoute,
    /// A follow-on frame routed to its service's stateful handler (file-chunk writing;
    /// open-session control). The full lifecycle is the service's own. Carries the bound service.
    ToServiceHandler(Service),
    /// A relay channel (file-read, net-response, display/input) routed to its relay (S2–S4).
    Relay(RelayKind),
    /// A verb no proxy service supports, or a frame whose verb's service does not match the
    /// session's binding (`box-proxy.md` JC-2) — dropped, logged.
    Unsupported { verb: u32 },
    /// The L2 payload failed to decode (a structural `proto::Reject`) — dropped, logged.
    Decode(Reject),
}

/// Which relay a routed frame goes to (the actual relay logic is Slices 2–4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelayKind {
    /// `file-read` → the filesystem read service (S2).
    FileRead,
    /// `net-response`/`net-error` → the cloud response relay (S3).
    CloudResponse,
    /// `display`/input → the terminal relay (S4).
    Terminal,
}

/// The dispatch state: the per-session service binding (`box-proxy.md` §1.3). A session is bound
/// once (its first frame's verb) and never re-bound.
///
/// **Resource residual (MB4), named and hook-ready:** the binding map has one entry per session and
/// grows **O(sessions-per-epoch)** — a session is inserted but only removed via [`close_session`],
/// which awaits a transport session-close/give-up signal that **does not exist in the committed
/// transport yet** (a *separate* cross-crate gap from net-payload's reassembly, which needs a
/// proto-side length/end-marker instead — different owners, different fixes). So the map is bounded
/// *per epoch* (the box's session-id space resets on reboot — `nic/architecture.md` §8's
/// no-cross-epoch-state) but not reclaimed within one, exactly mirroring the transport's own
/// un-reclaimed receiver sessions (`transport` table R5). This is a *named, tested, hook-ready*
/// residual — not a correctness bug (SessionId is monotonic-unique within an epoch, so no stale
/// binding can mismatch) — resolved fully once the transport surfaces a session-close signal, at
/// which point [`close_session`] is wired to it.
///
/// The per-session `reassemblers` map (Sprint 7) inherits the **same** bounded-per-epoch shape and
/// the **same** [`close_session`] eviction hook. It is in fact a *tighter* residual: an entry exists
/// only while a session's net-payload body is mid-reassembly — a completed body or a cap-overflow
/// removes it immediately (see [`Dispatcher::route`]), so only in-flight sessions hold one. Each
/// buffer is itself capped by `REASSEMBLY_CAP` (reject-before-grow, `proto::Reassembler`), so the
/// per-entry size is bounded too, not just the entry count.
///
/// The per-session `routes` map (Sprint 8, Task 14) holds the matched [`Route`] from a session's
/// `net-address` admission, so a later `net-payload` on that session sources its recognizer envelope
/// (domain/path) from the **real** route rather than a hardcoded stub. It shares the binding map's
/// bounded-per-epoch shape and [`close_session`] eviction. A `net-payload` that completes with **no**
/// entry here (no prior admitted `net-address`) fails closed ([`Routed::NetPayloadNoRoute`]).
pub struct Dispatcher {
    bindings: alloc::collections::BTreeMap<SessionId, Service>,
    /// One in-flight net-payload reassembly buffer per session, created on the session's first
    /// `net-payload` chunk and removed the instant the body completes or overflows the cap. Shares
    /// the binding map's bounded-per-epoch shape and [`close_session`] eviction (see the type doc).
    reassemblers: alloc::collections::BTreeMap<SessionId, Reassembler>,
    /// The matched [`Route`] per session, stored on `net-address` admission — the real source of the
    /// `net-payload` recognizer envelope (Task 14). Shares the binding map's eviction shape.
    routes: alloc::collections::BTreeMap<SessionId, Route>,
    /// The per-session reassembly capacity bound. In production this is always
    /// `proto::verb::REASSEMBLY_CAP` (set by [`Dispatcher::new`]); a `#[cfg(test)]` constructor may
    /// inject a small cap to exercise the evict-on-overflow path cheaply — the field is otherwise
    /// invisible to callers.
    reassembly_cap: usize,
    /// Count of frames dropped as unsupported/undecodable — a liveness/observability counter, not a
    /// gate. (No box content ever enters it — a verb id and a `Reject` class are safe by
    /// construction.)
    dropped: u64,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    #[must_use]
    pub fn new() -> Self {
        Dispatcher {
            bindings: alloc::collections::BTreeMap::new(),
            reassemblers: alloc::collections::BTreeMap::new(),
            routes: alloc::collections::BTreeMap::new(),
            reassembly_cap: verb::REASSEMBLY_CAP,
            dropped: 0,
        }
    }

    /// **Test-only** ([`cfg(test)`]): a dispatcher whose net-payload reassembly is bounded by an
    /// explicit small `cap`, so the dispatch-level evict-on-overflow behavior is reachable cheaply
    /// (the production cap is 32 MiB). This does **not** exist in the production build — the
    /// `#[cfg(test)]` gate keeps it out of the public API entirely. The proto-level reject-before-grow
    /// mechanism itself is cited, not re-proven here (it is exhaustively certified in `proto/tests`
    /// with a small cap); this only confirms dispatch *propagates* the reject and *evicts* the entry.
    #[cfg(test)]
    fn with_reassembly_cap_for_test(cap: usize) -> Self {
        Dispatcher {
            reassembly_cap: cap,
            ..Dispatcher::new()
        }
    }

    /// Route one reactor-delivered frame: decode its L2 payload, bind or check the session, and
    /// dispatch by verb. `register` is the tier-1 route register (U5's compiled-in
    /// `register::net::ROUTES`) and `vetted` the tier-1 vetted-tool register
    /// (`register::anthropic::VETTED_TOOLS`) — the two seams that reach the certified core. **Total**:
    /// every input maps to exactly one [`Routed`], no panic.
    pub fn route(
        &mut self,
        session: SessionId,
        payload: &[u8],
        register: &[Route],
        vetted: &[ToolIdentity],
    ) -> Routed {
        // Peek the verb ONLY — no body decode (proxy-head.md §1: routing + binding verb only).
        let Some(verb) = peek_verb(payload) else {
            self.dropped += 1;
            return Routed::Decode(Reject::ShortBody {
                needed: 4,
                got: payload.len(),
            });
        };
        let Some(service) = service_of_verb(verb) else {
            self.dropped += 1;
            return Routed::Unsupported { verb };
        };

        // Session→service binding: bind on the first frame; thereafter the verb's service must match.
        match self.bindings.get(&session) {
            None => {
                self.bindings.insert(session, service);
            }
            Some(&bound) if bound == service => {}
            Some(_) => {
                self.dropped += 1;
                return Routed::Unsupported { verb };
            }
        }

        self.dispatch(session, verb, payload, register, vetted)
    }

    /// Route a bound frame by verb. The **gated** channels (net-address, file-write) — and *only*
    /// those — decode their specific body and invoke the certified tier-1 core (`library.md` §7's
    /// "deep-parse only what it acts on"). Every **relay** channel returns a [`RelayKind`] with the
    /// body **un-decoded** — the caller forwards the opaque payload; the receiver's own parser
    /// handles it. So the proxy runs *no* terminal/display/input decoder on box bytes.
    fn dispatch(
        &mut self,
        session: SessionId,
        verb: u32,
        payload: &[u8],
        register: &[Route],
        vetted: &[ToolIdentity],
    ) -> Routed {
        let body = &payload[4..]; // safe: peek_verb confirmed payload.len() >= 4
        match verb & 0xff {
            // ── Gated: deep-parse the body, invoke the tier-1 core (the only decodes on box bytes) ──
            verb::NET_ADDRESS => match NetAddress::decode(body) {
                Ok(addr) => match crate::route::match_route(&addr, register) {
                    Ok(route) => {
                        // Store the matched Route so this session's later net-payload sources its
                        // recognizer envelope from the real route (Task 14), not a hardcoded stub.
                        self.routes.insert(session, route);
                        Routed::NetRouteAdmit(route)
                    }
                    Err(_) => Routed::NetDeny,
                },
                Err(reject) => Routed::Decode(reject),
            },
            verb::FILE_WRITE => match FileWrite::decode(body) {
                // The single-scan-site path (RQ2): FileWrite::decode already scan_safe-validated the
                // stack, so the core does count-only via leaf_validate_prescanned.
                Ok(fw) => match crate::leaf::leaf_validate_prescanned(&fw.0) {
                    Ok(()) => Routed::FileWriteAdmit,
                    Err(_) => Routed::FileWriteDeny,
                },
                Err(reject) => Routed::Decode(reject),
            },

            // ── Gated stateful channel: reassemble the body, then invoke the tier-1 recognizer ─────
            verb::NET_PAYLOAD => self.route_net_payload(session, body, vetted),

            // ── Stateful follow-on channels: routed to their service handler, body un-decoded ──────
            verb::FILE_CHUNK => Routed::ToServiceHandler(Service::File), // append at the cursor
            verb::OPEN_SESSION => Routed::ToServiceHandler(Service::Control),

            // ── Relay channels: forward the OPAQUE payload; NO body decode (proxy-head.md §1) ──────
            verb::NET_RESPONSE | verb::NET_ERROR => Routed::Relay(RelayKind::CloudResponse),
            verb::FILE_READ | verb::FILE_DATA | verb::FILE_ERROR | verb::LIST | verb::LIST_ENTRY
            | verb::REQUEST_FILE => Routed::Relay(RelayKind::FileRead),
            verb::DISPLAY | verb::PANE_STATE | verb::KEY | verb::MOUSE | verb::PASTE | verb::FOCUS
            | verb::CREATE_PTY | verb::RESIZE | verb::VIEWPORT => Routed::Relay(RelayKind::Terminal),

            // An unknown id within a known block — no service supports it.
            _ => {
                self.dropped += 1;
                Routed::Unsupported { verb }
            }
        }
    }

    /// The `net-payload` handler (Sprint 7): decode this chunk's `[flag][content]`, push its
    /// **content** into the session's [`Reassembler`] with the header's `more` flag, and — only when
    /// the body is complete — hand the whole reassembled buffer to the tier-1 Anthropic recognizer.
    ///
    /// The reassembler concatenates *content* (the 1-byte flag is stripped at decode and never enters
    /// the buffer — the collision-freedom property, `proto::net` / net.md §2), so the completed body
    /// is byte-identical to the box's request body. On `Complete`, the same buffer is what
    /// [`recognize`] vets and what [`Routed::NetPayloadAdmit`] carries forward to be forwarded — one
    /// buffer, no split (CB4-across-the-seam). The recognizer envelope's domain/path are sourced from
    /// the session's admitted [`Route`] (Task 14); a completion with no stored Route fails closed
    /// ([`Routed::NetPayloadNoRoute`]). Fail-closed on cap-overflow (the poisoned reassembler is
    /// evicted) and on a malformed chunk header (`ShortBody`).
    fn route_net_payload(&mut self, session: SessionId, body: &[u8], vetted: &[ToolIdentity]) -> Routed {
        // Decode this chunk: strip the fixed 1-byte continuation-flag header, keep the content tail.
        let chunk = match NetPayload::decode(body) {
            Ok(chunk) => chunk,
            Err(reject) => {
                self.dropped += 1;
                return Routed::Decode(reject);
            }
        };
        let cap = self.reassembly_cap; // read before the mutable entry borrow (no self-alias)
        let reassembler = self
            .reassemblers
            .entry(session)
            .or_insert_with(|| Reassembler::new(cap));
        match reassembler.push(&chunk.content, chunk.more) {
            Ok(ReassemblyState::Accumulating) => Routed::NetPayloadAccumulating,
            Ok(ReassemblyState::Complete(request_body)) => {
                // The body is whole; the reassembler already reset, but drop the map entry so the
                // next request on this session starts a fresh reassembly (and the residual shrinks).
                self.reassemblers.remove(&session);
                // Source the recognizer envelope from the session's admitted Route (Task 14). No
                // stored Route (no prior admitted net-address) → fail closed: nothing to vet the
                // request *as*, so it is neither recognized nor forwarded.
                let Some(route) = self.routes.get(&session).copied() else {
                    return Routed::NetPayloadNoRoute;
                };
                let (domain, path) = split_route(route);
                // verb/betas are justified constants for the net service: the wire (`NetAddress` +
                // `net-payload`) conveys neither an HTTP method nor beta headers, and the recognizer
                // reads betas only from this envelope field, never from the body — so `betas: &[]` is
                // the literal truth of what the box can express, not a dropped field.
                let request = Request {
                    verb: "POST",
                    domain,
                    path,
                    betas: &[],
                    body: &request_body,
                };
                match recognize(&request, vetted) {
                    Ok(()) => Routed::NetPayloadAdmit(request_body),
                    Err(denial) => Routed::NetPayloadDeny(denial),
                }
            }
            Err(reject) => {
                // Cap-exceeded (or an absurd-length overflow): fail closed and evict the poisoned
                // reassembler so a subsequent chunk cannot resume a rejected, over-cap accumulation.
                self.reassemblers.remove(&session);
                self.dropped += 1;
                Routed::Decode(reject)
            }
        }
    }

    /// Evict a session's binding **and any in-flight reassembly buffer** — the **eviction hook** for
    /// the MB4 bounded-resource residual. It is structurally ready to wire to a transport
    /// session-close/give-up signal; that signal is not in the committed transport yet (the same
    /// cross-crate gap the residual names), so the hook is present-and-tested but not yet *driven*.
    /// Returns the evicted binding (the reassembler and stored route, if any, are dropped silently —
    /// they hold only in-flight box content / a matched route, nothing observable).
    pub fn close_session(&mut self, session: SessionId) -> Option<Service> {
        self.reassemblers.remove(&session);
        self.routes.remove(&session);
        self.bindings.remove(&session)
    }

    /// The number of live session bindings — the O(sessions-per-epoch) resource the residual names.
    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Frames dropped as unsupported/undecodable (liveness/observability, not a gate).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The service a session is bound to, if any (test/observability).
    #[must_use]
    pub fn binding(&self, session: SessionId) -> Option<Service> {
        self.bindings.get(&session).copied()
    }

    /// The number of sessions with an in-flight net-payload reassembly — the tighter MB4 residual
    /// (an entry lives only between a session's first chunk and its completing/overflowing chunk).
    #[must_use]
    pub fn reassembling_count(&self) -> usize {
        self.reassemblers.len()
    }
}

/// Split a canonical [`Route`] (`https://<domain>[/<path…>]`) into `(domain, path)` for the
/// recognizer envelope (Task 14). The `Route` is validated-canonical by construction
/// ([`crate::vocab::Route::new`]), so the `https://` prefix is guaranteed and the split is total:
/// the first `/` after the scheme separates the domain from the path (which keeps its leading `/`);
/// a path-less route (a bare domain) yields the root path `/`. The returned slices are `&'static`
/// (the `Route` owns a `&'static str`), so they outlive the `&self` borrow and can back a `Request`.
fn split_route(route: Route) -> (&'static str, &'static str) {
    let s = route.as_str();
    let rest = s.strip_prefix("https://").unwrap_or(s);
    match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]), // path keeps its leading '/'
        None => (rest, "/"),                 // bare domain → root path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A raw `net-payload` L2 frame: `[verb][1-byte flag][content]` (flag `0x00` = more, nonzero =
    /// final) — the bytes `route` receives. Built by hand to keep this unit test dependency-light.
    fn net_payload_frame(content: &[u8], more: bool) -> Vec<u8> {
        let mut v = verb::NET_PAYLOAD.to_le_bytes().to_vec();
        v.push(if more { 0 } else { 1 });
        v.extend_from_slice(content);
        v
    }

    /// Dispatch-level **evict-on-overflow** (the one genuinely new bit of logic at this layer): a
    /// per-session reassembly that exceeds the cap fails closed with `Decode(ReassemblyOverflow)`
    /// **and** evicts the poisoned buffer, so a later chunk cannot resume an over-cap accumulation.
    /// Uses the `#[cfg(test)]` small-cap injector so the overflow is reachable cheaply (production cap
    /// is 32 MiB). The reject-before-grow mechanism itself is cited, not re-proven — it is exhaustively
    /// certified in `proto/tests` (`reassembler.rs`, small-cap boundary); this confirms only that
    /// dispatch *propagates* the reject and *evicts* the entry. `register`/`vetted` are unused on the
    /// overflow path (the recognizer is never reached), so empty slices suffice.
    #[test]
    fn net_payload_overflow_fails_closed_and_evicts() {
        let mut d = Dispatcher::with_reassembly_cap_for_test(8);
        let s = SessionId(1);
        // First chunk (6 bytes, more): 6 ≤ 8 → accumulates, one in-flight buffer.
        assert_eq!(
            d.route(s, &net_payload_frame(&[0u8; 6], true), &[], &[]),
            Routed::NetPayloadAccumulating,
        );
        assert_eq!(d.reassembling_count(), 1);
        let dropped_before = d.dropped();
        // Second chunk (6 bytes): 6 + 6 = 12 > 8 → overflow, fail closed, evict the buffer.
        assert_eq!(
            d.route(s, &net_payload_frame(&[0u8; 6], true), &[], &[]),
            Routed::Decode(Reject::ReassemblyOverflow { cap: 8 }),
        );
        assert_eq!(d.reassembling_count(), 0, "the poisoned reassembler is evicted on overflow");
        assert_eq!(d.dropped(), dropped_before + 1, "the overflow frame is counted as dropped");

        // A fresh chunk on the same session starts a NEW reassembly (the evicted buffer did not
        // silently resume the over-cap accumulation).
        assert_eq!(
            d.route(s, &net_payload_frame(&[0u8; 4], true), &[], &[]),
            Routed::NetPayloadAccumulating,
        );
        assert_eq!(d.reassembling_count(), 1, "a post-overflow chunk begins a fresh, empty buffer");
    }
}
