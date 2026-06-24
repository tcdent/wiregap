//! Coverage harness (not part of the library — a std example binary). Replays a committed fuzz corpus
//! through the codec exactly as the matching fuzz target does, so a `-Cinstrument-coverage` build yields
//! the branch/region coverage the fuzz exploration achieves on the `proto` library.
//!
//! Two modes, by the corpus directory name:
//! - `fuzz/corpus/decode` (default) — `decode_message` (→ on Ok, encode + re-decode), the L2 path.
//! - `fuzz/corpus/reassemble` — `frame → decode_frame → Reassembler::push`, the L1/reassembly path
//!   (mirrors `fuzz_targets/reassemble.rs`).

use std::fs;

use proto::{Reassembler, ReassemblyState, decode_frame, decode_message, encode_message};

const MAX_FRAME: usize = 4096;
const MAX_INPUT: usize = 1 << 16;
const SMALL_CAP: usize = 4096;
const MAX_FRAMES: u32 = 4096;

fn replay_decode(data: &[u8]) {
    if data.len() > MAX_FRAME {
        return;
    }
    if let Ok(msg) = decode_message(data) {
        let wire = encode_message(&msg);
        let redecoded = decode_message(&wire).expect("re-decode must succeed");
        assert_eq!(msg, redecoded);
        assert_eq!(encode_message(&redecoded), wire);
    }
}

/// Mirror `fuzz_targets/reassemble.rs`: parse `[u16 len][frame]…` and drive the live L1 path.
fn replay_reassemble(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let mut r = Reassembler::new(SMALL_CAP);
    let mut rest = data;
    let mut frames = 0u32;
    while let Some((lenb, after)) = rest.split_first_chunk::<2>() {
        let len = u16::from_le_bytes(*lenb) as usize;
        let Some((frame, tail)) = after.split_at_checked(len) else {
            break;
        };
        rest = tail;
        frames += 1;
        if frames > MAX_FRAMES {
            break;
        }
        if let Ok(f) = decode_frame(frame) {
            match r.push(f.l2, f.continuation) {
                Ok(ReassemblyState::Accumulating) => {}
                Ok(ReassemblyState::Complete(buf)) => {
                    let _ = decode_message(&buf);
                    r = Reassembler::new(SMALL_CAP);
                }
                Err(_) => {
                    r = Reassembler::new(SMALL_CAP);
                }
            }
        }
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fuzz/corpus/decode".to_string());
    let reassemble = dir.contains("reassemble");
    let mut replayed = 0usize;
    for entry in fs::read_dir(&dir).expect("corpus directory") {
        let path = entry.expect("dir entry").path();
        if !path.is_file() {
            continue;
        }
        let data = fs::read(&path).expect("read corpus seed");
        if reassemble {
            replay_reassemble(&data);
        } else {
            replay_decode(&data);
        }
        replayed += 1;
    }
    let which = if reassemble {
        "frame->decode_frame->reassembler.push"
    } else {
        "decode_message"
    };
    eprintln!("replayed {replayed} corpus inputs through {which}");
}
