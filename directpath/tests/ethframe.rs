//! Host tests for the Ethernet encap/decap adapters — the wiring the live link forces. Proven with fake
//! seams (no hardware): encap prepends the header + L1-length field; decap strips the header AND the
//! min-frame padding via that length; a runt yields an empty L1 frame (no panic — the reactor's
//! has_frame→next_frame contract forbids a None here).

use directpath::ethframe::{ETH_HDR, EthSink, EthSource};
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
