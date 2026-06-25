//! The net service seed: `net-address` ([`net.md`] §1). Body shape `[scheme u32][safe segment
//! stack]` — a fixed scheme prefix, then the **safe** address stack (the proxy supplies `://` and
//! `/`, matches the assembled address against the route, and logs it).
//!
//! [`net.md`]: ../../docs/proto/net.md

use alloc::vec::Vec;

use crate::errors::ErrorBody;
use crate::reject::Reject;
use crate::segment::{SafeSegmentStack, scan_safe};
use crate::verb;
use crate::wire::{Wire, take_prefix, verbatim_verb};

/// A reserved scheme value (`≥ 1`), carried verbatim.
///
/// Its inner value is **private** and is constructed only by the in-crate decoder
/// ([`Scheme::from_u32`]), which routes `0` to [`Scheme::Https`]. So a [`Scheme::Other`] can never
/// hold the known `https` value: `decode(encode(m)) == m` holds for every representable message (G1;
/// Decided Fork #8 — enforce-by-construction).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReservedScheme(u32);

impl ReservedScheme {
    /// The raw reserved scheme value (always `≥ 1`).
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// The request scheme. `0` is `https`; any other value is **carried raw** ([`Scheme::Other`]) rather
/// than rejected — Decided Fork #8(i). Never transmuted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    Https,
    /// A scheme value outside the defined set — see [`ReservedScheme`].
    Other(ReservedScheme),
}

impl Scheme {
    fn from_u32(value: u32) -> Self {
        match value {
            0 => Scheme::Https,
            // `other >= 1` is guaranteed — the only construction site for `ReservedScheme`.
            other => Scheme::Other(ReservedScheme(other)),
        }
    }

    fn to_u32(self) -> u32 {
        match self {
            Scheme::Https => 0,
            Scheme::Other(reserved) => reserved.get(),
        }
    }
}

/// `net-address` (`box → proxy`): a scheme and a safe address stack.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetAddress {
    pub scheme: Scheme,
    pub address: SafeSegmentStack,
}

impl Wire for NetAddress {
    const VERB: u32 = verb::NET_ADDRESS;

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.scheme.to_u32().to_le_bytes());
        self.address.encode(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        let (scheme, rest) = take_prefix::<4>(body)?;
        // The safe scan consumes the entire remainder (the stack runs to the frame end), so there is
        // no trailing-bytes case here — the scan either accepts all of `rest` or rejects.
        let address = scan_safe(rest)?;
        Ok(NetAddress {
            scheme: Scheme::from_u32(u32::from_le_bytes(*scheme)),
            address,
        })
    }
}

// ---- the net verbs (005, seam 5B) --------------------------------------------------------------

verbatim_verb! {
    /// `net-response` (`proxy → box`): the upstream response, a **verbatim** tail. The box reassembles
    /// the response itself (the asymmetry: ingress lands on untrusted ground, relayed per-frame —
    /// approach.md §5); the proxy does **not** reassemble it.
    NetResponse, verb::NET_RESPONSE
}

/// `net-error` (`proxy → box`): an upstream/route error — the shared [`ErrorBody`] grammar (one body,
/// many verbs; reuse, not re-implementation).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetError(pub ErrorBody);

impl Wire for NetError {
    const VERB: u32 = verb::NET_ERROR;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode_body(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        ErrorBody::decode_body(body).map(NetError)
    }
}

verbatim_verb! {
    /// `net-payload` (`box → proxy`): the verbatim request body, **reassembled** across frames by the
    /// proxy ([`crate::Reassembler`]) before this decodes. Verb `0x33` rides in frame 1 only; continuation
    /// frames are raw payload concatenated opaquely, so the **complete** buffer (`[0x33][full payload]`)
    /// decodes through `decode_message` **once** here — a single verbatim tail over the full payload.
    NetPayload, verb::NET_PAYLOAD
}
