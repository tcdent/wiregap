//! The shared error body ([`model.md`] §8) — one grammar used by `file-error` and `net-error`.
//! Body shape `[code u32][detail tail]`: a category code, then optional rest-of-frame UTF-8 context.
//!
//! [`model.md`]: ../../docs/proto/model.md

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::verb;

/// A reserved error code — a value outside the defined `0..=6` range, carried verbatim.
///
/// Its inner value is **private** and is constructed only by the in-crate decoder ([`ErrorCode::from_u32`]),
/// which routes every known code to a named variant. So an [`ErrorCode::Unknown`] can never hold a
/// *known* code: `decode(encode(m)) == m` holds for **every representable** message, not merely
/// decode-produced ones (gate G1; Decided Fork #8 — enforce-by-construction, not documented invariant).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReservedCode(u32);

impl ReservedCode {
    /// The raw reserved code value (always `> 6`). For the consumer to apply its own policy.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// The error category. Known codes `0..=6` are named; an unrecognized code is **carried raw**
/// ([`ErrorCode::Unknown`]) rather than rejected — Decided Fork #8(i). Decode never transmutes; a
/// reserved value is preserved and re-encoded identically, leaving the deny decision to the consumer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorCode {
    Unspecified,
    NotFound,
    Denied,
    Unavailable,
    Timeout,
    Unreachable,
    Malformed,
    /// A code outside the defined `0..=6` range — see [`ReservedCode`].
    Unknown(ReservedCode),
}

impl ErrorCode {
    fn from_u32(value: u32) -> Self {
        // Codes are referenced from the single wire-numbers surface ([`crate::verb`]).
        match value {
            verb::ERR_UNSPECIFIED => ErrorCode::Unspecified,
            verb::ERR_NOT_FOUND => ErrorCode::NotFound,
            verb::ERR_DENIED => ErrorCode::Denied,
            verb::ERR_UNAVAILABLE => ErrorCode::Unavailable,
            verb::ERR_TIMEOUT => ErrorCode::Timeout,
            verb::ERR_UNREACHABLE => ErrorCode::Unreachable,
            verb::ERR_MALFORMED => ErrorCode::Malformed,
            // `other > MAX_ERROR_CODE` is guaranteed by the arms above — the only construction site
            // for `ReservedCode`, so the `> 6` invariant holds by construction.
            other => ErrorCode::Unknown(ReservedCode(other)),
        }
    }

    fn to_u32(self) -> u32 {
        match self {
            ErrorCode::Unspecified => verb::ERR_UNSPECIFIED,
            ErrorCode::NotFound => verb::ERR_NOT_FOUND,
            ErrorCode::Denied => verb::ERR_DENIED,
            ErrorCode::Unavailable => verb::ERR_UNAVAILABLE,
            ErrorCode::Timeout => verb::ERR_TIMEOUT,
            ErrorCode::Unreachable => verb::ERR_UNREACHABLE,
            ErrorCode::Malformed => verb::ERR_MALFORMED,
            ErrorCode::Unknown(reserved) => reserved.get(),
        }
    }
}

/// The shared error body: a machine-readable [`ErrorCode`] plus optional `detail`.
///
/// `detail` is a **verbatim** rest-of-frame blob (`Vec<u8>`), *not* a validated `String`: it is
/// rendered inertly by the receiver, so validating UTF-8 would impose a wire obligation and make the
/// body non-total — a malformed detail must still decode and relay (law 11, Decided Fork #7a). It may
/// hold non-UTF-8 bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub detail: Vec<u8>,
}

impl ErrorBody {
    /// Decode `[code u32][detail tail]`. The detail is the verbatim remainder (possibly empty).
    pub fn decode_body(body: &[u8]) -> Result<Self, Reject> {
        let (code, detail) = body
            .split_first_chunk::<4>()
            .ok_or(Reject::ShortBody {
                needed: 4,
                got: body.len(),
            })?;
        Ok(ErrorBody {
            code: ErrorCode::from_u32(u32::from_le_bytes(*code)),
            detail: detail.to_vec(),
        })
    }

    /// Append `[code u32][detail tail]` to `out`.
    pub fn encode_body(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.code.to_u32().to_le_bytes());
        out.extend_from_slice(&self.detail);
    }
}
