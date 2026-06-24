//! G4 NEGATIVE (type boundary). A consumer's logger accepts only the safe-to-log capability — a
//! `&SafeSegmentStack`. Handing it an `UnsafeSegmentStack` (a real name) must **fail to compile**.
//! This is the exact bound a real logger uses, not a strawman.

use proto::{SafeSegmentStack, scan_unsafe};

fn audit_log(address: &SafeSegmentStack) {
    for seg in address.segments() {
        let _ = seg.as_str();
    }
}

fn main() {
    let real_name = scan_unsafe(b"secret file.txt").unwrap();
    audit_log(&real_name);
}
