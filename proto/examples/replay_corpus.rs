//! Coverage harness (not part of the library — a std example binary). Replays the committed fuzz
//! corpus through the codec exactly as the fuzz target does (decode → on Ok, encode + re-decode), so
//! a `-Cinstrument-coverage` build yields the branch/region coverage the fuzz exploration achieves on
//! the `proto` library. See the 1C evidence for the build + `llvm-cov` invocation.

use std::fs;

use proto::{decode_message, encode_message};

const MAX_FRAME: usize = 4096;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fuzz/corpus/decode".to_string());
    let mut replayed = 0usize;
    for entry in fs::read_dir(&dir).expect("corpus directory") {
        let path = entry.expect("dir entry").path();
        if !path.is_file() {
            continue;
        }
        let data = fs::read(&path).expect("read corpus seed");
        if data.len() > MAX_FRAME {
            continue;
        }
        if let Ok(msg) = decode_message(&data) {
            let wire = encode_message(&msg);
            let redecoded = decode_message(&wire).expect("re-decode must succeed");
            assert_eq!(msg, redecoded);
            assert_eq!(encode_message(&redecoded), wire);
        }
        replayed += 1;
    }
    eprintln!("replayed {replayed} corpus inputs through decode_message");
}
