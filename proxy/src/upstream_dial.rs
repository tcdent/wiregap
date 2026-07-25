//! **The dial — the concrete upstream client that implements the seam.**
//!
//! Sprint 20 Slice 3 split this out of `upstream.rs`, which had grown to hold four unrelated concerns
//! under one "upstream" name. The three that remain after the split are a *layered* set with a real
//! dependency edge, not four peers:
//!
//! - [`crate::upstream`] — the seam types and the loop-side ingress adapter. Knows nothing about
//!   Anthropic, TLS, or credentials.
//! - [`crate::credential`] — the credential's descriptor, value, and resolution policy.
//! - **this module** — the dial that *implements* `upstream`'s [`UpstreamSpawn`](crate::upstream::UpstreamSpawn) seam using
//!   [`crate::upstream_tls`]'s verified pipe and a [`Credential`](crate::credential::Credential).
//!
//! **This module depends on `upstream`; `upstream` does not depend on it.** The channel vocabulary
//! (`UpstreamFrame`/`UpstreamHandle`/`UPSTREAM_CHANNEL_CAP`) is the seam *between* the two halves, which
//! is why the dial imports it rather than owning it — the adapter can be driven by a test double with no
//! network at all, and this module can be replaced without touching the loop.
//!
//! `rustls` is used ONLY below this seam, never in the box-reachable core (tier-3, off the L10 review
//! budget). No proxy-owned `unsafe` (MB6). The box's request never carries or influences the credential;
//! the credential never reaches a box-facing frame or a log.

use std::io::{self, Read, Write};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::Duration;

use proto::ErrorCode;
use transport::SessionId;

use crate::credential::Credential;
use crate::upstream::{UPSTREAM_CHANNEL_CAP, UpstreamFrame, UpstreamHandle, UpstreamSpawn};
use crate::upstream_tls::TlsPipe;
use crate::vocab::Route;

/// The `anthropic-version` header the proxy sends on the upstream call — a proxy-owned constant (the box
/// cannot influence it; the wire conveys no HTTP headers).
///
/// **Not a provisional scalar — reclassified 2026-07-25.** Marking this "provisional, marked for review"
/// was a category error: it is a **vendor API contract string**, not a tunable with a range. There is no
/// number to pick and no local reasoning that could improve it — the only correct value is whatever the
/// upstream currently accepts, and `2023-06-01` is that value.
///
/// **The real hazard is drift, and nothing here detects it.** If the upstream deprecates this version,
/// the failure arrives as an upstream *response* (which the proxy relays verbatim, correctly) and no
/// proxy-side signal distinguishes it from any other 4xx. That is a monitoring gap, not a wrong constant,
/// and it belongs to whoever owns upstream-drift watch — recorded here so the next reader does not mistake
/// a vendor string for a knob to turn.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The bounded global timeout on the upstream call — a hung/slow upstream never leaks a thread or a
/// channel forever (it fails `Timeout` → `net-error`, the worker exits, the channel closes).
///
/// **Reviewed and confirmed at 120s (2026-07-25)** — with one interaction flagged rather than silently
/// accepted. This is a **whole-call** bound, and the pipe is close-delimited: the proxy reads to EOF
/// without parsing, so a long generation occupies the call for its full duration. A response that takes
/// longer than 120s to complete therefore fails `Timeout` and reaches the box as a `net-error`, not as a
/// truncated response — fail-safe, but it *is* a product-visible ceiling on generation length rather than
/// purely a liveness guard. 120s is generous for the non-streaming Messages shape this proxy admits;
/// raising it is a product decision about acceptable latency, not a correctness fix, so it is left as-is
/// and the ceiling is stated.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(120);

/// The chunk size the response body is streamed in — bounded per-read memory (the outbound analog of
/// `fs_read::READ_CHUNK`). Each read becomes one verbatim `net-response` frame.
const RESPONSE_CHUNK: usize = 16 * 1024;

// ─── Sprint 11 Slice 2 — the real Anthropic upstream client (below the seam) ──────────────────────────
//
// `rustls` is used ONLY here (below the `NicRun`/reply seam), never in the box-reachable core
// (tier-3, off the L10 review budget). No proxy-owned `unsafe` (MB6). The box's request never carries or
// influences the credential; the credential never reaches a box-facing frame or a log.


/// The real upstream client (Sprint 11 Slice 2): POSTs the recognizer-admitted body **verbatim** to the
/// Anthropic Messages API over a direct `rustls` pipe — no HTTP client — and streams the response
/// **verbatim** back
/// through the [`UpstreamSpawn`] seam into the Sprint-10 push-queue. The destination is **not** a
/// parameter of this type: it arrives per-call as the matched `Route` (Sprint 19), so the register is the
/// only place a destination is authored.
/// **Holds no destination.** The endpoint it dials arrives per-call from the matched `Route`
/// (Sprint 19); there is deliberately no `endpoint` field, so there is nothing for a dial to diverge
/// from the register *to*.
pub struct AnthropicClient {
    credential: Credential,
}

impl AnthropicClient {
    #[must_use]
    /// Takes ONLY the credential. The endpoint is no longer an input here — it arrives per-call from the
    /// matched `Route` — so a caller cannot supply a destination that differs from the register's.
    pub fn new(credential: Credential) -> Self {
        AnthropicClient { credential }
    }
}

impl UpstreamSpawn for AnthropicClient {
    fn spawn(&self, _session: SessionId, destination: Route, body: Vec<u8>) -> UpstreamHandle {
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
        // ONE decomposition of the matched route, here, feeding peer + SNI + Host alike.
        let dest = Destination::of(destination);
        let key = self.credential.expose().to_string();
        let join = std::thread::spawn(move || {
            match upstream_call(dest, &key, &body) {
                Ok(reader) => stream_response(reader, &tx),
                Err(code) => {
                    let _ = tx.send(UpstreamFrame::Failed(code));
                }
            }
            // `tx` dropped on return → channel closes → the loop observes end-of-response.
        });
        UpstreamHandle { rx, join: Some(join) }
    }
}

/// The dial target: **one value, derived once**, that the TLS peer, the SNI, and the HTTP `Host` header
/// are all read out of.
///
/// This exists to make the peer-binding a single property rather than three that happen to agree.
/// [`Route::split`] is the crate's one decomposition and [`Destination::of`] is the only constructor
/// reached from production, so a matched route cannot end up connecting to one host, presenting a second
/// name for verification, and addressing a third in the request. Sprint 19 removed exactly this shape one
/// level down — a dial that agreed with the register by authorship rather than by construction.
#[derive(Clone, Copy)]
struct Destination {
    /// The TCP peer, the name certificate verification runs against, AND the `Host` header. One field.
    host: &'static str,
    /// The request-line target.
    path: &'static str,
}

impl Destination {
    /// Decompose the **matched** `Route` — the vetted destination the register authored and U3/U4 proved
    /// the box's address equals. Nothing else constructs a production `Destination`.
    fn of(route: Route) -> Self {
        let (host, path) = route.split();
        Destination { host, path }
    }
}

/// The port every upstream call dials. Fixed, not derived: `Route`'s canonical form is `https://` +
/// host + path and carries no port, so there is no box-influenced value here at all.
const HTTPS_PORT: u16 = 443;

/// The blocking upstream call — **the network leg**, now a direct TLS pipe with no HTTP client in it
/// (Sprint 18). Connects to `dest`, writes the request, and hands back the live pipe so the caller
/// streams the response **exactly as it arrives**.
///
/// What comes back is the WHOLE upstream response — status line, headers, body — unparsed (designer
/// ruling, 2026-07-24). This function frames what the proxy *emits* and reads nothing of what it
/// *receives*: there is no response parser here to be wrong, and therefore no status, no encoding, and no
/// redirect for the proxy to have an opinion about. `cloud/approach.md` §5's "never inspects, neutralizes,
/// modifies, or sanitizes" is satisfied structurally rather than by a configuration flag.
fn upstream_call(dest: Destination, key: &str, body: &[u8]) -> Result<TlsPipe, ErrorCode> {
    call_over(dest, key, body, || TlsPipe::connect(dest.host, HTTPS_PORT, UPSTREAM_TIMEOUT))
}

/// [`upstream_call`]'s body with the socket-opening step injected — **the only** thing that varies. The
/// tests supply an opener aimed at the local test server (a name production cannot reach, on a port
/// `Route` cannot express); everything from the request bytes onward is the shipped path, so what they
/// prove is what production does. Slice 1's `connect_with_config` precedent: move the test below the
/// seam rather than widen a production type to accommodate it.
fn call_over(
    dest: Destination,
    key: &str,
    body: &[u8],
    open: impl FnOnce() -> io::Result<TlsPipe>,
) -> Result<TlsPipe, ErrorCode> {
    let mut pipe = open().map_err(|e| map_transport_error(&e))?;
    write_request(&mut pipe, dest, key, body).map_err(|e| map_transport_error(&e))?;
    Ok(pipe)
}

/// Write the request: the **fixed** header set, then the recognizer-admitted body byte-verbatim.
///
/// **The complete list of what goes on the wire, and why each line has to be there** — this is the
/// request-side half of "a dumb, fixed, verbatim pipe", so the set is closed and auditable by reading it:
///
/// | line | why it cannot be dropped |
/// |---|---|
/// | `POST <path> HTTP/1.1` | the request line; `path` is the register's, via [`Destination`] |
/// | `host` | HTTP/1.1 requires it — and it is the SAME field as the TLS peer and SNI |
/// | `x-api-key` | the credential. The one thing the proxy adds that the box never sees |
/// | `anthropic-version` | the API requires it; a fixed compiled-in constant |
/// | `content-type` | the API requires it; fixed |
/// | `content-length` | frames the body so the upstream knows where it ends |
/// | `connection: close` | makes the response close-delimited, so the relay ends on a transport event |
///
/// Nothing else. No `user-agent`, no `accept`, no `accept-encoding`, no cookies, no proxy headers — an
/// HTTP client adds those on your behalf, which is precisely why there is no longer an HTTP client here.
/// The absence of `accept-encoding` is also why nothing arrives compressed to be tempted to decompress.
///
/// **The box contributes `body` and nothing else, and it cannot escape the body.** `content-length` is
/// computed here from the actual byte count and the body is written after the blank line, so bytes the
/// box authored are read by the upstream as body regardless of what they contain — a `\r\n` in the
/// payload is payload. `key`, `host`, and `path` are all compiled-in or operator-authored, never
/// box-influenced, so no header line takes a box-supplied value at all.
fn write_request(w: &mut impl Write, dest: Destination, key: &str, body: &[u8]) -> io::Result<()> {
    let mut req = Vec::with_capacity(HEADROOM + body.len());
    req.extend_from_slice(
        format!(
            "POST {} HTTP/1.1\r\n\
             host: {}\r\n\
             x-api-key: {}\r\n\
             anthropic-version: {}\r\n\
             content-type: application/json\r\n\
             content-length: {}\r\n\
             connection: close\r\n\
             \r\n",
            dest.path,
            dest.host,
            key,
            ANTHROPIC_VERSION,
            body.len(),
        )
        .as_bytes(),
    );
    req.extend_from_slice(body);
    // One write for the whole request: the credential never sits in a partially-flushed buffer across a
    // failure, and the upstream sees headers and body together.
    w.write_all(&req)?;
    w.flush()
}

/// Slack for the fixed header block so the request buffer is sized once. Not a limit on anything.
const HEADROOM: usize = 256;

/// Stream a response **verbatim** into the channel as [`UpstreamFrame::Chunk`]s (MB3 — never inspected,
/// neutralized, or transformed). Bounded per-read memory ([`RESPONSE_CHUNK`]). A read error mid-stream →
/// `Failed(Unavailable)`. Stops early if the loop hung up (the box session closed).
///
/// Every byte the upstream sent is relayed, starting at the status line: the box receives the response
/// the cloud actually gave, and decides for itself what a 401 or a 302 means. An HTTP status is a
/// RESPONSE, not an error (operator ruling, 2026-07-24) — and with no parser here, there is now no code
/// path that *could* turn one into a proxy-invented `net-error`.
fn stream_response(mut reader: impl Read, tx: &SyncSender<UpstreamFrame>) {
    let mut buf = [0u8; RESPONSE_CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return, // EOF — the caller drops `tx` (channel close = end-of-response)
            Ok(n) => {
                if tx.send(UpstreamFrame::Chunk(buf[..n].to_vec())).is_err() {
                    return; // the receiver (loop) is gone — stop
                }
            }
            Err(_) => {
                let _ = tx.send(UpstreamFrame::Failed(ErrorCode::Unavailable));
                return;
            }
        }
    }
}

/// Map a transport failure to the proxy's own [`ErrorCode`] category (no upstream detail, no credential —
/// the box learns only *that* it failed and roughly why). Timeout → `Timeout`; unresolvable or
/// unreachable peer → `Unreachable`; everything else, TLS verification failures included → `Unavailable`.
///
/// **Reached only when there is no response to relay.** A refused connection, a name that does not
/// resolve, a certificate that fails verification, a hung upstream: in each case nothing came back, so
/// `net-error` is the only thing the box can be told (`proto/net.md` §4 — net-error is for what is NOT an
/// upstream response). An HTTP status never arrives here; it is a response, and responses go down the
/// relay untouched.
///
/// A read timeout on a socket carrying `SO_RCVTIMEO` surfaces as `WouldBlock`, not `TimedOut`, so both
/// map to `Timeout` — the distinction is the platform's, not the proxy's.
fn map_transport_error(e: &io::Error) -> ErrorCode {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => ErrorCode::Timeout,
        ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::AddrNotAvailable
        | ErrorKind::NotFound => ErrorCode::Unreachable,
        _ => ErrorCode::Unavailable,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // The recognizer-admitted body: ONE definition, shared with `upstream`'s adapter tests rather
    // than duplicated here — the same discipline as `upstream_tls::testserver`.
    use crate::upstream::tests::ADMIT_BODY;
    use crate::register::net::ROUTES;

    /// MB3 (unit): `stream_response` relays the response body VERBATIM — the concatenated `Chunk`s equal
    /// the input bytes exactly, never inspected or transformed. Driven over an in-memory `Cursor` (no
    /// network).
    #[test]
    fn stream_response_relays_verbatim() {
        let input: Vec<u8> = (0..(RESPONSE_CHUNK * 2 + 500)).map(|i| (i % 251) as u8).collect();
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP * 4);
        stream_response(Cursor::new(input.clone()), &tx);
        drop(tx);
        let mut got = Vec::new();
        while let Ok(frame) = rx.recv() {
            match frame {
                UpstreamFrame::Chunk(c) => got.extend_from_slice(&c),
                UpstreamFrame::Failed(_) => panic!("no failure expected on a clean read"),
            }
        }
        assert_eq!(got, input, "the response is streamed byte-verbatim (MB3)");
    }

    // ---------------------------------------------------------------------------------------------
    // Sprint 18 Slice 2 — the pipe's own evidence.
    //
    // Every test below runs against the shared local TLS server (`upstream_tls::testserver`), through
    // `call_over` — the shipped `upstream_call` body with only the socket target swapped. What varies is
    // the thing that MUST vary for an offline test (a name production cannot reach, on a port `Route`
    // cannot express); the request bytes, the relay, and the error mapping are all the production path.
    // ---------------------------------------------------------------------------------------------

    use crate::upstream_tls::testserver::{IO_TIMEOUT, Fixture, fixture, now_offset, serve_scripted, trusting};

    /// A fixture whose leaf is valid, in date, and named `localhost` — the host every arm below dials.
    fn local_fixture() -> Fixture {
        fixture("localhost", now_offset(-3600), now_offset(3600))
    }

    /// Run the real call against a local server on `port`, dialling it as `host`.
    fn call_local(fx: &Fixture, host: &'static str, port: u16, key: &str) -> Result<TlsPipe, ErrorCode> {
        let dest = Destination { host, path: "/v1/messages" };
        call_over(dest, key, ADMIT_BODY, || {
            TlsPipe::connect_with_config(host, port, IO_TIMEOUT, trusting(fx))
        })
    }

    /// Drain a pipe through the SAME `stream_response` production uses, returning the relayed bytes.
    fn relayed(pipe: TlsPipe) -> Vec<u8> {
        let (tx, rx) = sync_channel(UPSTREAM_CHANNEL_CAP);
        let worker = std::thread::spawn(move || stream_response(pipe, &tx));
        let mut got = Vec::new();
        while let Ok(frame) = rx.recv_timeout(Duration::from_secs(10)) {
            match frame {
                UpstreamFrame::Chunk(c) => got.extend_from_slice(&c),
                UpstreamFrame::Failed(code) => panic!("unexpected transport failure: {code:?}"),
            }
        }
        worker.join().expect("relay worker");
        got
    }

    /// **FINAL STATE #5 — the request side, byte-exact.** The wire request is the admitted body plus the
    /// credential header and the fixed framing, and *nothing else*.
    ///
    /// Asserted as ONE full-buffer equality rather than a set of `contains` checks, because `contains`
    /// can only find what you thought to look for: it passes just as happily with a `user-agent`, an
    /// `accept-encoding`, or a cookie the HTTP client added on your behalf. Equality is the only form
    /// that proves the *absence* of everything unlisted — which is the actual claim being made about a
    /// dumb, fixed pipe.
    #[test]
    fn the_wire_request_is_exactly_the_admitted_body_plus_the_credential_header() {
        let fx = local_fixture();
        let (port, requests) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        let pipe = call_local(&fx, "localhost", port, "sk-secret-key").expect("the local call connects");
        let req = requests.recv_timeout(Duration::from_secs(10)).expect("the server captured a request");
        drop(pipe);

        let mut want: Vec<u8> = format!(
            "POST /v1/messages HTTP/1.1\r\nhost: localhost\r\nx-api-key: sk-secret-key\r\n\
             anthropic-version: 2023-06-01\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n",
            ADMIT_BODY.len()
        )
        .into_bytes();
        want.extend_from_slice(ADMIT_BODY);

        assert_eq!(
            String::from_utf8_lossy(&req),
            String::from_utf8_lossy(&want),
            "the wire request must be EXACTLY the fixed header set plus the admitted body — no extra \
             header, no re-encode",
        );
        // BIND-THE-CHAIN, stated separately so a future edit to `want` cannot quietly lose it: the body
        // on the wire is the admitted bytes themselves.
        assert!(req.ends_with(ADMIT_BODY), "the body is the admitted bytes, byte-identical");
    }

    /// **FINAL STATE #4 — the response side, verbatim, including the status line.** The box receives the
    /// upstream's own bytes from the first octet: a 401 is relayed as a 401, not replaced by a
    /// proxy-invented `net-error` (`cloud/approach.md` §5; operator ruling 2026-07-24). This is the case
    /// Sprint 17's capture will exercise live against a deliberately-invalid credential.
    #[test]
    fn the_whole_response_is_relayed_verbatim_status_line_and_headers_included() {
        const RESPONSE: &[u8] = b"HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\n\
                                  content-length: 62\r\n\r\n\
                                  {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}";
        let fx = local_fixture();
        let (port, _requests) = serve_scripted(&fx, RESPONSE);
        let pipe = call_local(&fx, "localhost", port, "sk-invalid").expect("the local call connects");
        assert_eq!(
            String::from_utf8_lossy(&relayed(pipe)),
            String::from_utf8_lossy(RESPONSE),
            "the ENTIRE upstream response — status line, headers, body — reaches the box unmodified",
        );
    }

    /// **FINAL STATE #8, defect 1 — a redirect is RELAYED, never followed.**
    ///
    /// The ureq defect had two halves and this proves both: the client followed 3xx automatically, and it
    /// did not strip `x-api-key` when doing so — so a redirect chosen by whatever answered the connection
    /// could walk the credential to a host the register never vetted.
    ///
    /// The `Location` here points at a SECOND live local server. The proof is not only that the 302 came
    /// back, but that the second server received *nothing at all*: no follow-up request, and therefore no
    /// credential delivered anywhere the route did not name.
    #[test]
    fn a_redirect_is_relayed_not_followed_and_the_credential_goes_nowhere_else() {
        let fx = local_fixture();
        let (elsewhere_port, elsewhere) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\n\r\nFOLLOWED");
        let location = format!("https://localhost:{elsewhere_port}/stolen");
        let redirect: &'static [u8] = Box::leak(
            format!("HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\n\r\n").into_bytes()
                .into_boxed_slice(),
        );
        let (port, _requests) = serve_scripted(&fx, redirect);

        let pipe = call_local(&fx, "localhost", port, "sk-secret-key").expect("the local call connects");
        let got = relayed(pipe);

        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(redirect),
            "the 302 is handed to the box verbatim — the proxy has no opinion about a status code",
        );
        assert!(
            elsewhere.recv_timeout(Duration::from_secs(2)).is_err(),
            "the redirect target was CONTACTED — the redirect was followed and the credential travelled \
             to a host the register never vetted",
        );
    }

    /// **FINAL STATE #8, defect 2 — an ambient proxy environment variable is ignored.**
    ///
    /// Proven in a CHILD PROCESS, because `std::env::set_var` is `unsafe` under edition 2024 and this
    /// crate is `deny(unsafe_code)` outside its two audited modules (MB6) — so the environment is set the
    /// only way a test here legitimately can: on a child. The child re-runs the ignored arm below, which
    /// performs a real round trip to a local server. If any part of the upstream path consulted
    /// `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY`, the connection would go to the black-holed address instead
    /// and the child would fail.
    ///
    /// The black hole is `127.0.0.1:1` — nothing listens, so a consulted proxy setting cannot accidentally
    /// succeed.
    #[test]
    fn an_ambient_proxy_environment_variable_is_ignored() {
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .args(["--exact", "--ignored", "--nocapture", "upstream_dial::tests::proxy_env_child_arm"])
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "socks5://127.0.0.1:1")
            .env("https_proxy", "http://127.0.0.1:1")
            .env("http_proxy", "http://127.0.0.1:1")
            .output()
            .expect("re-exec the test binary");
        assert!(
            out.status.success(),
            "with proxy variables set, the upstream call did not reach its destination — something in \
             the path consults the ambient environment:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("1 passed"),
            "the child ran no test — the arm name or the harness flags drifted, so this proved nothing:\n{}",
            String::from_utf8_lossy(&out.stdout),
        );
    }

    /// The child arm for the test above. `#[ignore]`d so it runs ONLY under the parent's re-exec, with the
    /// proxy variables set; on its own it would prove nothing (a plain run has no proxy in the
    /// environment, so it would pass vacuously).
    #[test]
    #[ignore = "driven by an_ambient_proxy_environment_variable_is_ignored, which sets the environment"]
    fn proxy_env_child_arm() {
        assert!(
            std::env::var("HTTPS_PROXY").is_ok(),
            "the child must inherit the proxy environment, or it tests nothing",
        );
        let fx = local_fixture();
        let (port, requests) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        let pipe = call_local(&fx, "localhost", port, "sk-secret-key")
            .expect("the call must reach the destination, not the proxy named in the environment");
        let req = requests.recv_timeout(Duration::from_secs(10)).expect("the DESTINATION got the request");
        assert!(req.starts_with(b"POST /v1/messages HTTP/1.1\r\n"), "an origin-form request line");
        assert!(
            !String::from_utf8_lossy(&req).contains("http://"),
            "an absolute-form request line means the request was addressed to a proxy",
        );
        assert_eq!(String::from_utf8_lossy(&relayed(pipe)).len(), 40);
    }

    /// **FINAL STATE #8, defect 3 — a compressed body is relayed COMPRESSED.**
    ///
    /// The response below is real gzip, labelled `content-encoding: gzip`. An HTTP client would hand the
    /// caller the decompressed plaintext and quietly drop the header, which is a modified response: the
    /// box would be told the upstream said something it did not literally say, and the bytes it received
    /// would no longer be the bytes on the wire. Both the compressed octets and the header must survive.
    #[test]
    fn a_gzip_labelled_body_is_relayed_compressed_with_its_header_intact() {
        /// Valid gzip of `{"id":"msg_gzip_probe"}`.
        const GZIP_BODY: [u8; 43] = [
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xab, 0x56, 0xca, 0x4c, 0x51,
            0xb2, 0x52, 0xca, 0x2d, 0x4e, 0x8f, 0x4f, 0xaf, 0xca, 0x2c, 0x88, 0x2f, 0x28, 0xca, 0x4f,
            0x4a, 0x55, 0xaa, 0x05, 0x00, 0xce, 0x47, 0x4c, 0xc3, 0x17, 0x00, 0x00, 0x00,
        ];
        let mut response =
            b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: 43\r\n\r\n".to_vec();
        response.extend_from_slice(&GZIP_BODY);
        let response: &'static [u8] = Box::leak(response.into_boxed_slice());

        let fx = local_fixture();
        let (port, _requests) = serve_scripted(&fx, response);
        let got = relayed(call_local(&fx, "localhost", port, "sk-secret-key").expect("connects"));

        assert_eq!(got, response, "the compressed body and its header are relayed untouched");
        assert!(
            !got.windows(b"msg_gzip_probe".len()).any(|w| w == b"msg_gzip_probe"),
            "the plaintext appeared — the body was DECOMPRESSED somewhere in the path",
        );
    }

    /// **FINAL STATE #10a — the destination IS the register's, for every route it holds.**
    #[test]
    fn a_destination_is_exactly_the_matched_routes_host_and_path() {
        for route in ROUTES {
            let dest = Destination::of(*route);
            let rest = route.as_str().strip_prefix("https://").expect("a canonical route is https://");
            let tail = rest.strip_prefix(dest.host).expect("the host is the route's leading segment");
            assert!(
                tail == dest.path || (tail.is_empty() && dest.path == "/"),
                "{route:?}: host + path must recompose the route (a bare domain gains the root path)",
            );
            assert!(!dest.host.contains('/'), "the host carries no path");
            assert!(dest.path.starts_with('/'), "the path is origin-form");
        }
    }

    /// **FINAL STATE #10b — peer, SNI, and `Host` cannot be steered apart.**
    ///
    /// Both arms dial the SAME listener, whose certificate names `localhost` and nothing else. Only the
    /// single [`Destination::host`] field differs:
    ///
    /// - `localhost` — the handshake succeeds (so the SNI and the verified peer name are `localhost`) AND
    ///   the captured request says `host: localhost`. All three agree.
    /// - `127.0.0.1` — reaches the *identical* listener, so nothing about reachability changed, and the
    ///   handshake is REFUSED because the name is not in the certificate.
    ///
    /// The second arm is the load-bearing one: it shows the `Host` header cannot be set to one name while
    /// the connection is verified against another, because there is only one field to set. Three separate
    /// derivations that happened to agree would let the arms diverge; one value cannot.
    #[test]
    fn peer_sni_and_host_header_cannot_be_steered_apart() {
        let fx = local_fixture();

        let (port, requests) = serve_scripted(&fx, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        let pipe = call_local(&fx, "localhost", port, "sk").expect("the SAN-matching name is accepted");
        let req = requests.recv_timeout(Duration::from_secs(10)).expect("captured");
        drop(pipe);
        assert!(
            String::from_utf8_lossy(&req).contains("\r\nhost: localhost\r\n"),
            "the Host header carries the same name the certificate was verified against",
        );

        // Arm 2. A certificate that is valid, in date, and issued by a CA the client trusts — but which
        // names a DIFFERENT host. It is served on a `localhost` listener, so the dialled name reaches the
        // peer exactly as in arm 1; the ONLY thing that changed is whether the certificate carries the
        // name being dialled.
        //
        // (This arm was first written dialling `127.0.0.1` at the arm-1 listener. It "passed" as a
        // failure and proved nothing: that listener binds via `localhost`, which resolves to `::1` here,
        // so the connection was REFUSED before any TLS work — the vacuous-pass shape Slice 1's review
        // caught in the refusal arms. The `Unreachable` code, not `Unavailable`, is what exposed it.)
        let elsewhere = fixture("not-the-destination.example", now_offset(-3600), now_offset(3600));
        let (port2, requests2) = serve_scripted(&elsewhere, b"HTTP/1.1 200 OK\r\n\r\n");

        let err = TlsPipe::connect_with_config("localhost", port2, IO_TIMEOUT, trusting(&elsewhere))
            .err()
            .expect("a certificate that does not name the dialled host must be refused");
        let err = err.to_string();
        assert!(
            err.contains("tls handshake failed:"),
            "the arm failed before the handshake, so it exercised nothing: {err}",
        );
        assert!(err.contains("not valid for name"), "refused, but not on the NAME: {err}");

        // The join, stated as the consequence that matters: no request was written at all, so there is no
        // way to put `host: localhost` on a wire whose peer was not verified as `localhost`. One field
        // drives the verified name and the header alike; separate derivations could have diverged here.
        let captured = requests2.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        assert!(
            captured.is_empty(),
            "request bytes reached a peer that failed verification: {:?}",
            String::from_utf8_lossy(&captured),
        );

        // And the box-facing code for that failure is `Unavailable`, not `Unreachable`: the peer was
        // reached, so this is not "could not get there" — it is "nothing trustworthy came back".
        let refused = call_local(&elsewhere, "localhost", serve_scripted(&elsewhere, b"x").0, "sk")
            .err()
            .expect("refused");
        assert_eq!(refused, ErrorCode::Unavailable, "a verification failure has no response to relay");
    }

    /// Transport-error mapping, on the real path: a refused connection becomes `net-error(Unreachable)`
    /// over the seam. Driven at `call_over` because a `Route` cannot name a local target (no `http://`,
    /// no port — the canonical form forbids both, deliberately).
    #[test]
    fn a_refused_connection_maps_to_net_error_unreachable() {
        let dest = Destination { host: "localhost", path: "/v1/messages" };
        // Port 1: nothing listens, so the connect is refused before any TLS work begins.
        let err = call_over(dest, "sk", ADMIT_BODY, || TlsPipe::connect(dest.host, 1, IO_TIMEOUT))
            .err()
            .expect("a refused connection must fail");
        assert_eq!(err, ErrorCode::Unreachable, "a refused connection → net-error(Unreachable)");
    }

}
