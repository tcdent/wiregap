//! **Ethernet encap/decap** between the reactor's frame seams and the live 82599 — the wiring the live
//! link forces (Sprint 7 Slice 1). The transport speaks L1 frames `[session id (8B BE)][payload]` with an
//! **implicit length** (the frame length IS the L1 length); the 82599 speaks **Ethernet frames**
//! `[dst MAC][src MAC][len/ethertype][payload]` and **pads** anything under 60 bytes to the minimum. Two
//! problems the single-frame Sprint-6 PoC never hit: (1) an L1 frame with no MAC header is rejected by the
//! peer's L2 filter; (2) min-frame padding makes the received length exceed the L1 length, so a bare ACK
//! (8B) padded to 60B would be mis-read as a 52-byte-payload data frame.
//!
//! Fix, WITHOUT touching the frozen transport/reactor: carry the L1 length in the **802.3 length field**
//! (bytes 12–13, the `< 0x0600` length interpretation), so the receiver strips both the 14-byte header
//! **and** the trailing pad and hands the transport exactly its L1 frame.
//! - [`EthSink`] wraps a `TransmitSink`: prepend `[dst][src][L1 len BE]`, emit through the driver TX.
//! - [`EthSource`] wraps a `RxSource`: read the length field, return `[14 .. 14+len]` (header + pad gone).
//!
//! Both are thin adapters BELOW the reactor's seams; the reactor still sees a pure `TransmitSink`/
//! `RxSource` of L1 frames. The transport and the reactor algorithm stay byte-frozen.

extern crate alloc;
use alloc::vec::Vec;
use reactor::{RxSource, TransmitSink};

/// Ethernet header length (dst 6 + src 6 + len/ethertype 2).
pub const ETH_HDR: usize = 14;

/// Strip the 14-byte Ethernet header and the min-frame padding from a received frame, returning the L1
/// frame `[session id][payload]` the transport expects. The 802.3 length field (bytes 12-13) is
/// adversary-influenced, so the returned slice is CLAMPED to the arrived bytes (no OOB). A runt shorter
/// than the header yields an empty slice (never a panic). Used by [`EthSource`] and by any harness that
/// drives the transport directly over the live rings.
pub fn decap(frame: &[u8]) -> &[u8] {
    if frame.len() < ETH_HDR {
        return &frame[frame.len()..];
    }
    let l1_len = u16::from_be_bytes([frame[12], frame[13]]) as usize;
    let end = (ETH_HDR + l1_len).min(frame.len());
    &frame[ETH_HDR..end]
}

/// Prepends a fixed Ethernet header (`[dst][src][L1 length in the 802.3 length field]`) to each L1 frame.
pub struct EthSink<S: TransmitSink> {
    inner: S,
    dst: [u8; 6],
    src: [u8; 6],
    scratch: Vec<u8>,
}

impl<S: TransmitSink> EthSink<S> {
    pub fn new(inner: S, dst: [u8; 6], src: [u8; 6]) -> Self {
        Self { inner, dst, src, scratch: Vec::new() }
    }
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: TransmitSink> TransmitSink for EthSink<S> {
    fn emit(&mut self, frame: &[u8]) {
        let len = frame.len().min(u16::MAX as usize) as u16;
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.dst);
        self.scratch.extend_from_slice(&self.src);
        self.scratch.extend_from_slice(&len.to_be_bytes()); // 802.3 length field = the L1 length
        self.scratch.extend_from_slice(frame);
        self.inner.emit(&self.scratch);
    }
}

/// Strips the 14-byte Ethernet header and uses the carried L1 length to drop min-frame padding, so the
/// reactor's transport sees exactly the L1 frame `[session id][payload]`.
pub struct EthSource<R: RxSource> {
    inner: R,
}

impl<R: RxSource> EthSource<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }
}

impl<R: RxSource> RxSource for EthSource<R> {
    fn has_frame(&self) -> bool {
        self.inner.has_frame()
    }

    fn next_frame(&mut self) -> Option<&[u8]> {
        // [`decap`] strips the header + pad and clamps to the arrived bytes. Never `None` when the inner
        // returned `Some` — the reactor's `has_frame`-then-`next_frame` contract forbids it (a runt yields
        // an empty L1 the transport's guard drops).
        Some(decap(self.inner.next_frame()?))
    }
}
