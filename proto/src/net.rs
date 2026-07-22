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

/// `net-payload` (`box → proxy`): one **chunk** of the request body — a 1-byte fixed-width
/// **continuation flag** header (`0x00` = more chunks follow, nonzero = final), then the verbatim
/// content tail. This is `model.md` §6's `[fixed-width prefix][one frame-bounded tail]` shape (the
/// same shape as [`NetAddress`]). The proxy's [`crate::Reassembler`] concatenates each chunk's
/// `content` until a chunk's flag is nonzero. The flag lives in the message **header**, never in the
/// accumulated content, so it can never collide with a box-authored content byte — the boundary is
/// fixed-width and structural (`model.md` §11), never a value searched for in content. Supersedes the
/// earlier "transport continuation signal" framing (no such transport field exists — confirmed at
/// the Sprint-6 reassembly finding).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetPayload {
    /// `true` = more chunks follow (`0x00` on the wire); `false` = this is the final chunk (nonzero).
    pub more: bool,
    /// This chunk's verbatim content (the flag prefix stripped) — concatenated by the reassembler.
    pub content: Vec<u8>,
}

impl Wire for NetPayload {
    const VERB: u32 = verb::NET_PAYLOAD;

    fn encode(&self, out: &mut Vec<u8>) {
        out.push(if self.more { 0 } else { 1 });
        out.extend_from_slice(&self.content);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        // The fixed-width 1-byte flag prefix, then the verbatim content tail; an empty body has no
        // flag byte → `Reject::ShortBody`. The boundary is fixed width, never a box-authored length.
        let (flag, content) = take_prefix::<1>(body)?;
        Ok(NetPayload {
            more: flag[0] == 0,
            content: content.to_vec(),
        })
    }
}
