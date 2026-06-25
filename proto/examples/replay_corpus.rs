//! Coverage harness (not part of the library — a std example binary). Replays a committed fuzz corpus
//! through the codec exactly as the matching fuzz target does, so a `-Cinstrument-coverage` build yields
//! the branch/region coverage the fuzz exploration achieves on the `proto` library.
//!
//! Two modes, by the corpus directory name:
//! - `fuzz/corpus/decode` (default) — `decode_message` (→ on Ok, encode + re-decode), the L2 path.
//! - `fuzz/corpus/reassemble` — `Reassembler::push` over `(chunk, continuation)` sequences, the
//!   reassembly path (mirrors `fuzz_targets/reassemble.rs`).

use std::fs;

use proto::{Reassembler, ReassemblyState, decode_message, encode_message};

const MAX_FRAME: usize = 4096;
const MAX_INPUT: usize = 1 << 16;
const SMALL_CAP: usize = 4096;
const MAX_CHUNKS: u32 = 4096;

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

/// Mirror `fuzz_targets/reassemble.rs`: parse `[u8 cont][u16 len][len bytes]…` and drive `push` direct.
fn replay_reassemble(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let mut r = Reassembler::new(SMALL_CAP);
    let mut rest = data;
    let mut chunks = 0u32;
    while let Some((cont_byte, after)) = rest.split_first() {
        let Some((lenb, after)) = after.split_first_chunk::<2>() else {
            break;
        };
        let len = u16::from_le_bytes(*lenb) as usize;
        let Some((chunk, tail)) = after.split_at_checked(len) else {
            break;
        };
        rest = tail;
        chunks += 1;
        if chunks > MAX_CHUNKS {
            break;
        }
        let continuation = cont_byte & 1 != 0;
        match r.push(chunk, continuation) {
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
        "reassembler.push (chunk, continuation)"
    } else {
        "decode_message"
    };
    eprintln!("replayed {replayed} corpus inputs through {which}");
}
