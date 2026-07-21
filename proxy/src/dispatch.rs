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


use proto::{FileWrite, NetAddress, Reject, Wire, verb};
use transport::SessionId;

use crate::vocab::Route;

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
    /// A follow-on frame routed to its service's stateful handler (net-payload reassembly + the
    /// Anthropic recognizer; file-chunk writing). The full lifecycle is the service's own — see the
    /// reassembly-continuation seam note in the module tests. Carries the bound service.
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
#[derive(Default)]
pub struct Dispatcher {
    bindings: alloc::collections::BTreeMap<SessionId, Service>,
    /// Count of frames dropped as unsupported/undecodable — a liveness/observability counter, not a
    /// gate. (No box content ever enters it — a verb id and a `Reject` class are safe by
    /// construction.)
    dropped: u64,
}

impl Dispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Route one reactor-delivered frame: decode its L2 payload, bind or check the session, and
    /// dispatch by verb. `register` is the tier-1 route register (U5's compiled-in
    /// `register::net::ROUTES`) — the seam that reaches the core. **Total**: every input maps to
    /// exactly one [`Routed`], no panic.
    pub fn route(&mut self, session: SessionId, payload: &[u8], register: &[Route]) -> Routed {
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

        self.dispatch(verb, payload, register)
    }

    /// Route a bound frame by verb. The **gated** channels (net-address, file-write) — and *only*
    /// those — decode their specific body and invoke the certified tier-1 core (`library.md` §7's
    /// "deep-parse only what it acts on"). Every **relay** channel returns a [`RelayKind`] with the
    /// body **un-decoded** — the caller forwards the opaque payload; the receiver's own parser
    /// handles it. So the proxy runs *no* terminal/display/input decoder on box bytes.
    fn dispatch(&mut self, verb: u32, payload: &[u8], register: &[Route]) -> Routed {
        let body = &payload[4..]; // safe: peek_verb confirmed payload.len() >= 4
        match verb & 0xff {
            // ── Gated: deep-parse the body, invoke the tier-1 core (the only decodes on box bytes) ──
            verb::NET_ADDRESS => match NetAddress::decode(body) {
                Ok(addr) => match crate::route::match_route(&addr, register) {
                    Ok(route) => Routed::NetRouteAdmit(route),
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

            // ── Stateful follow-on channels: routed to their service handler, body un-decoded ──────
            verb::NET_PAYLOAD => Routed::ToServiceHandler(Service::Net), // reassembly + recognizer
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

    /// Evict a session's binding — the **eviction hook** for the MB4 bounded-resource residual. It
    /// is structurally ready to wire to a transport session-close/give-up signal; that signal is not
    /// in the committed transport yet (the same cross-crate gap as the net-payload continuation
    /// bit), so the hook is present-and-tested but not yet *driven*. Returns the evicted binding.
    pub fn close_session(&mut self, session: SessionId) -> Option<Service> {
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
}
