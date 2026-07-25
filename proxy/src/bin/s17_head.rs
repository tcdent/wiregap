//! Sprint 17 Slice 2 — `s17_head`: the scripted HEAD client for the live end-to-end capture, bin-sha
//! self-declared.
//!
//! A real socket client, not a human at a terminal (Sprint 14's scripted-head precedent). What needs
//! proving is that the head-facing socket carries a real `display` frame whose marker traces back to the
//! box's own observation of the real upstream exchange; a person typing proves nothing further and makes
//! the capture unrepeatable.
//!
//! **MUST RUN AS ROOT**, and this is a property of the proxy's current shape rather than a choice here:
//! `serve` binds the rendezvous socket *before* its privilege drop, so under this host's `umask 022` the
//! socket is created root-owned `0755` and only root may `connect()`. The unprivileged-head-connect path
//! is an explicit non-goal of this sprint and remains an open residual on the transport shape
//! (`proxy-head.md` §3) — so this capture proves the channel works, NOT that an unprivileged operator
//! head can attach to it. A `PermissionDenied` here is that residual showing up, not a defect.
//!
//! **START ORDER IS LOAD-BEARING.** `serve` blocks on `accept()` *before* it constructs the NIC, so this
//! client must connect before the box is started, or the capture simply hangs:
//!   operator provisions → `serve` starts (blocks on accept) → **this client connects** → `serve`
//!   proceeds into `bring_up` → `s17_box` drives.
//! The short connect-retry below covers only the startup race between `serve` binding and this starting;
//! it does not reorder anything.
//!
//! SCOPE HONESTY: this is not a renderer. The proxy relays terminal frames opaquely (`proxy-head.md` §1)
//! and the marker is an opaque payload, so what is proven is that the CHANNEL delivered a box-authored
//! display frame carrying a value derived from the box's own observation of the real upstream exchange.
//!
//! **Which marker to expect (corrected for Sprint 18).** The box emits one of two, and they mean
//! different things:
//!   - `resp-fp:<fingerprint>/<len>` — **the expected path.** The upstream answered and its bytes were
//!     relayed verbatim, so the box fingerprinted the real response. Under the operator's no-billing
//!     ruling that response is a 401 from a deliberately-invalid credential — which is still a genuine
//!     upstream *response*, not an error the proxy invented.
//!   - `net-error:<code>:irt=<session>` — the transport-failure path: nothing came back at all.
//!
//! This doc previously said the marker *is* `net-error:...`, describing the superseded design in which a
//! 401 never reached the box and the proxy substituted its own error code. Sprint 18's verbatim pipe
//! removed that substitution.
//!
//! Build+run (bench only, as root): `cargo run --features bench-capture --bin s17_head`.

use std::io;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use proxy::head::decode_head_frame;

/// The rendezvous socket `serve` binds (its `HEAD_SOCKET_PATH`).
const HEAD_SOCKET_PATH: &str = "/run/zerotrust-proxy/head.sock";
/// How long to keep retrying the initial connect — covers only the startup race with `serve`'s bind.
const CONNECT_WINDOW: Duration = Duration::from_secs(30);
/// How long to wait for the display frame once connected: the box's round trip includes a real upstream
/// call, so this is generous by design.
const FRAME_WINDOW: Duration = Duration::from_secs(120);

fn main() -> io::Result<()> {
    println!("{}", benchtools::Provenance::capture().header("proxy_s17_head"));
    println!("head: connecting to {HEAD_SOCKET_PATH} (must be root — the socket is root-owned)\n");

    // Connect, retrying only for the startup race. A PermissionDenied is reported as the named residual
    // it is, not as an opaque failure.
    let deadline = Instant::now() + CONNECT_WINDOW;
    let mut stream = loop {
        match UnixStream::connect(HEAD_SOCKET_PATH) {
            Ok(s) => break s,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "head: PERMISSION DENIED on {HEAD_SOCKET_PATH} — the socket is bound root-owned \
                     before serve's privilege drop, so this client must run as root (sprint non-goal: \
                     the unprivileged-head-connect path, proxy-head.md §3)."
                );
                return Err(e);
            }
            Err(e) if Instant::now() < deadline => {
                // Not yet bound — serve has not reached its bind, or is between unlink and bind.
                std::thread::sleep(Duration::from_millis(100));
                let _ = e;
            }
            Err(e) => {
                eprintln!("head: could not connect within the startup window: {e}");
                return Err(e);
            }
        }
    };
    println!("head: CONNECTED — serve may now proceed into bring_up; start s17_box next.");

    // Read frames until the box→head display arrives. Every frame is recorded, so an unexpected one is
    // visible in the capture rather than silently skipped.
    stream.set_read_timeout(Some(FRAME_WINDOW))?;
    let mut seen = 0usize;
    let started = Instant::now();
    loop {
        let frame = match decode_head_frame(&mut stream) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("head: no display frame arrived ({e}) after {} frame(s)", seen);
                return Err(e);
            }
        };
        seen += 1;
        let verb = if frame.payload.len() >= 4 {
            u32::from_le_bytes(frame.payload[0..4].try_into().expect("len >= 4")) & 0xff
        } else {
            0
        };
        println!(
            "head: frame #{seen} session={:#x} verb={verb:#04x} len={}",
            frame.session.0,
            frame.payload.len()
        );

        if verb == proto::verb::DISPLAY {
            let marker = String::from_utf8_lossy(&frame.payload[4..]).into_owned();
            println!("\nhead: DISPLAY frame received after {:?}", started.elapsed());
            println!("head: marker = {marker}");
            println!("head:   <- must equal the marker s17_box recorded from its own observation.");
            println!("head:      resp-fp:<fingerprint>/<len>  = EXPECTED — the upstream answered and its");
            println!("head:                                     bytes were relayed verbatim.");
            println!("head:      net-error:<code>:irt=<session> = transport failure — nothing came back.");
            println!("\nCAPTURE (head leg) PASS — the real socket carried a box-authored display frame");
            println!("whose marker derives from the box's own observation of the real upstream exchange.");
            return Ok(());
        }
        println!("head: (not a display frame — continuing)");
    }
}
