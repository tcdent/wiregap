//! Sprint 6 S4 — terminal relay certification (MB1-MB6). MB2 relay-fidelity (the frame emerges at
//! the RIGHT endpoint, byte-identical, opaque), MB3 zero-gating (routing by verb only — no content,
//! no peer identity, no TLS; the operator's two directions), MB5 robustness (malformed frames from
//! BOTH directions — head→box treated as potentially network-injected per proxy-head §0 — never
//! panic). MB1 cargo test; MB6 pure safe Rust.
//!
//! Frames are built as raw [verb u32 LE][opaque body] — the relay reads ONLY the verb and forwards
//! the body opaquely, so the tests never construct (or depend on) the terminal L2 body types. That
//! IS the point: the proxy runs no display/input decoder.

use proxy::terminal_relay::{Direction, Endpoint, direction_of, relay_frame};

fn frame(verb: u32, body: &[u8]) -> Vec<u8> {
    let mut f = verb.to_le_bytes().to_vec();
    f.extend_from_slice(body);
    f
}

/// A capturing endpoint — records what was emitted (a fake wire seam, transport-agnostic).
#[derive(Default)]
struct Capture { frames: Vec<Vec<u8>> }
impl Endpoint for Capture {
    fn emit(&mut self, f: &[u8]) { self.frames.push(f.to_vec()); }
}

/// MB2: a display frame (0x11, box→head) emerges at the HEAD endpoint, byte-identical; nothing leaks
/// to the box direction. Opaque — the exact input bytes (verb + whatever body) are forwarded.
#[test]
fn display_relays_box_to_head_byte_identical() {
    let f = frame(0x11, b"\x41\x00\x00\x00 arbitrary opaque cell bytes \xff");
    let mut head = Capture::default();
    let mut boxx = Capture::default();
    assert_eq!(relay_frame(&f, &mut head, &mut boxx), Some(Direction::BoxToHead));
    assert_eq!(head.frames, vec![f.clone()], "the frame reaches the head, verbatim");
    assert!(boxx.frames.is_empty(), "nothing leaks to the box direction");
}

/// MB2: an input frame (0x12 key, head→box) emerges at the BOX endpoint, byte-identical.
#[test]
fn key_relays_head_to_box_byte_identical() {
    let f = frame(0x12, b"opaque key event bytes");
    let mut head = Capture::default();
    let mut boxx = Capture::default();
    assert_eq!(relay_frame(&f, &mut head, &mut boxx), Some(Direction::HeadToBox));
    assert_eq!(boxx.frames, vec![f.clone()], "the input frame reaches the box, verbatim");
    assert!(head.frames.is_empty());
}

/// MB3: routing is by VERB direction only — no content, no peer identity, no source check. Two
/// display frames with DIFFERENT bodies route identically (by verb, not content).
#[test]
fn direction_is_by_verb_not_content() {
    assert_eq!(direction_of(0x11), Some(Direction::BoxToHead)); // display
    assert_eq!(direction_of(0x18), Some(Direction::BoxToHead)); // pane-state
    assert_eq!(direction_of(0x12), Some(Direction::HeadToBox)); // key
    assert_eq!(direction_of(0x15), Some(Direction::HeadToBox)); // mouse
    assert_eq!(direction_of(0x10), Some(Direction::HeadToBox)); // create-pty
    assert_eq!(direction_of(0x30), None); // net-address — not terminal
    assert_eq!(direction_of(0x00), None); // open-session (control) — not terminal

    let mut h1 = Capture::default(); let mut b1 = Capture::default();
    let mut h2 = Capture::default(); let mut b2 = Capture::default();
    assert_eq!(relay_frame(&frame(0x11, b""), &mut h1, &mut b1), Some(Direction::BoxToHead));
    assert_eq!(relay_frame(&frame(0x11, b"a full cell stream, different content"), &mut h2, &mut b2), Some(Direction::BoxToHead));
    // Same routing, different content — content never influences the decision.
}

/// MB5: malformed frames from BOTH directions never panic — head→box frames are treated as
/// potentially network-injected (proxy-head §0 residual), same robustness bar as the box side.
#[test]
fn fuzz_malformed_frames_both_directions_never_panic() {
    struct Rng(u64);
    impl Rng { fn next(&mut self) -> u64 { let mut x=self.0; x^=x<<13; x^=x>>7; x^=x<<17; self.0=x; x } }
    let mut rng = Rng(0x7E20_1234_ABCD_5678);
    let mut head = Capture::default();
    let mut boxx = Capture::default();
    for _ in 0..30_000 {
        let len = (rng.next() % 96) as usize;
        let f: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            relay_frame(&f, &mut head, &mut boxx)
        }));
        assert!(out.is_ok(), "relay panicked on frame {f:?}");
    }
}

/// A non-terminal / too-short frame relays nothing (relay_frame stays total).
#[test]
fn non_terminal_or_short_frame_is_a_noop() {
    let mut head = Capture::default();
    let mut boxx = Capture::default();
    assert_eq!(relay_frame(&[], &mut head, &mut boxx), None); // too short
    assert_eq!(relay_frame(&0x30u32.to_le_bytes(), &mut head, &mut boxx), None); // net verb
    assert!(head.frames.is_empty() && boxx.frames.is_empty());
}
