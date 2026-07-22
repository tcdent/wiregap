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
    /// The wrapped sink — bench-only, for the harness to read the underlying `TxPath` drop counter.
    #[cfg(feature = "bench")]
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

#[cfg(test)]
mod integration_tests {
//! Host tests for the Ethernet encap/decap adapters — the wiring the live link forces. Proven with fake
//! seams (no hardware): encap prepends the header + L1-length field; decap strips the header AND the
//! min-frame padding via that length; a runt yields an empty L1 frame (no panic — the reactor's
//! has_frame→next_frame contract forbids a None here).

use super::{ETH_HDR, EthSink, EthSource};
use reactor::{RxSource, TransmitSink};

const DST: [u8; 6] = [0x90, 0xe2, 0xba, 0xd0, 0x68, 0xac];
const SRC: [u8; 6] = [0x90, 0xe2, 0xba, 0xc7, 0x69, 0xa0];

#[derive(Default)]
struct CaptureSink {
    emitted: Vec<Vec<u8>>,
}
impl TransmitSink for CaptureSink {
    fn emit(&mut self, frame: &[u8]) {
        self.emitted.push(frame.to_vec());
    }
}

/// A stub RxSource yielding pre-built (already-Ethernet) frames.
struct StubSource {
    frames: std::collections::VecDeque<Vec<u8>>,
    front: Vec<u8>,
}
impl StubSource {
    fn new(frames: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self { frames: frames.into_iter().collect(), front: Vec::new() }
    }
}
impl RxSource for StubSource {
    fn has_frame(&self) -> bool {
        !self.frames.is_empty()
    }
    fn next_frame(&mut self) -> Option<&[u8]> {
        self.front = self.frames.pop_front()?;
        Some(&self.front)
    }
}

#[test]
fn encap_prepends_header_and_l1_length() {
    let mut sink = EthSink::new(CaptureSink::default(), DST, SRC);
    let l1 = [0u8, 0, 0, 0, 0, 0, 0, 5, 0xAB, 0xCD]; // [session 5][payload AB CD], 10 bytes
    sink.emit(&l1);
    let out = &sink.inner().emitted[0];
    assert_eq!(&out[0..6], &DST, "dst MAC");
    assert_eq!(&out[6..12], &SRC, "src MAC");
    assert_eq!(&out[12..14], &10u16.to_be_bytes(), "802.3 length field = L1 length (10)");
    assert_eq!(&out[14..], &l1, "L1 frame follows the header");
}

#[test]
fn decap_strips_header_and_padding() {
    // An Ethernet frame carrying an 8-byte L1 frame (a bare ACK for session 5), padded to 60 bytes.
    let mut eth = Vec::new();
    eth.extend_from_slice(&DST);
    eth.extend_from_slice(&SRC);
    eth.extend_from_slice(&8u16.to_be_bytes()); // L1 length = 8
    eth.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 5]); // the L1 bare ACK
    eth.resize(60, 0x00); // min-frame padding
    let mut src = EthSource::new(StubSource::new([eth]));
    assert!(src.has_frame());
    let l1 = src.next_frame().unwrap();
    assert_eq!(l1, &[0, 0, 0, 0, 0, 0, 0, 5], "header + PADDING stripped → exactly the 8-byte L1 ACK");
}

#[test]
fn encap_then_decap_roundtrips_the_l1_frame() {
    // Encap an L1 frame, pad it as the wire would, then decap → the original L1 frame back.
    let mut sink = EthSink::new(CaptureSink::default(), DST, SRC);
    let l1 = [0u8, 0, 0, 0, 0, 0, 0, 9, 1, 2, 3, 4]; // [session 9][payload 1 2 3 4]
    sink.emit(&l1);
    let mut wire = sink.inner().emitted[0].clone();
    wire.resize(60, 0x00); // the device pads to 60 on the wire
    let mut src = EthSource::new(StubSource::new([wire]));
    assert_eq!(src.next_frame().unwrap(), &l1, "decap recovers the exact L1 frame, pad dropped");
}

#[test]
fn runt_shorter_than_the_header_yields_empty_not_none() {
    let mut src = EthSource::new(StubSource::new([vec![0u8; ETH_HDR - 1]]));
    assert!(src.has_frame());
    assert_eq!(src.next_frame().unwrap().len(), 0, "runt → empty L1 (transport drops it), never None");
}

#[test]
fn adversarial_over_length_is_clamped_no_panic() {
    // The 802.3 length field is adversary-influenced (device-written, from box-controlled wire bytes). A
    // length claiming MORE than the frame carries must NOT drive an out-of-bounds read — the returned L1
    // is clamped to the arrived bytes (the same bound-the-hostile-length discipline the driver RX guard
    // applies to a lying descriptor length). No panic.
    let mut eth = Vec::new();
    eth.extend_from_slice(&DST);
    eth.extend_from_slice(&SRC);
    eth.extend_from_slice(&0xFFFFu16.to_be_bytes()); // claims 65535 payload bytes...
    eth.extend_from_slice(&[0xAB; 20]); // ...but only 20 arrived (34-byte frame)
    let mut src = EthSource::new(StubSource::new([eth]));
    let l1 = src.next_frame().unwrap();
    assert_eq!(l1.len(), 20, "clamped to the arrived bytes (34 - 14 header), no OOB read");
    assert!(l1.iter().all(|&b| b == 0xAB), "the clamped slice is the real arrived payload");
}
}
