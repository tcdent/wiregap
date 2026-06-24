//! G4 NEGATIVE (capability absence). The safe-to-log capability *is* the audited boundary: an
//! `UnsafeSegment` has no `as_str`. Reaching for the loggable view on a real name must **fail to
//! compile** — there is no path to it short of re-running the safe scan.

use proto::scan_unsafe;

fn main() {
    let real_name = scan_unsafe(b"secret file.txt").unwrap();
    let _loggable = real_name.segments()[0].as_str();
}
