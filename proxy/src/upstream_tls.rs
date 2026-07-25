//! Sprint 18 Slice 1 — the verified TLS transport for the one upstream call site.
//!
//! **ONE TLS ROLE: client, outbound, to the upstream. Never a server, never inbound**
//! (`cloud/approach.md`: "the proxy is a TLS **client** to the public upstream, and it is **not** a TLS
//! server. It does not terminate TLS from the box"). The box terminates its own TLS and hands the proxy
//! **plaintext** over the project wire (`net-address` + `net-payload`), which carries no TLS at all — so
//! the proxy performs exactly one outbound handshake per call and **no inbound handshake ever**. That is
//! why the only configuration in this module is a `ClientConfig`, why the only entry point is
//! `TlsPipe::connect`, and why nothing here accepts a connection. `architecture.md` §11 L10 depends on
//! it: box bytes reach the TLS stack only as plaintext to encrypt, and the stack's parser only ever sees
//! the *cloud's* records — a claim that would not survive this module also being a server.
//!
//! (`rustls`'s SERVER API does appear below, in the `#[cfg(test)]` `testserver` module only, where it
//! stands in for the upstream so the refusals can be proven offline. It is never linked into the shipped
//! binary — a grep for `ServerConfig` in this crate should find that module and nothing else.)
//!
//! A **dumb, fixed pipe**: connect to a named host over TLS with the certificate genuinely verified,
//! write bytes, read bytes until the peer closes. It is not an HTTP client and deliberately contains no
//! HTTP knowledge — no redirect following, no proxy resolution, no content decoding, no response parsing.
//! Those behaviours are absent **by construction**, not disabled by configuration: the code that would
//! perform them does not exist here, so there is no default to inherit and no flag to forget.
//!
//! **Why a pipe rather than a client.** `proto/net.md` §3 requires the upstream response to reach the box
//! **verbatim** — status line, headers, body, exactly as received. Anything that parses the response to
//! separate those parts has already inspected and re-shaped what it relays, which `cloud/approach.md` §5
//! forbids ("never inspects, neutralizes, modifies, or sanitizes it"). The only way to relay bytes
//! untouched is never to interpret them, so this module reads and forwards opaque bytes and leaves every
//! interpretation to the box.
//!
//! **Framing: we frame what we EMIT, we never parse what we RECEIVE.** The request we send is ours to
//! construct, so it carries an ordinary `Content-Length` — that is us describing our own bytes. The
//! response is delimited by the **connection close**, not by any header we read: `Connection: close` on
//! the request means the peer closes when the response is complete, so EOF ends the stream and no
//! `Content-Length` or chunked-framing logic is needed on the way back. This is `net.md` §3's own model
//! made literal — "the stream ends when the proxy stops emitting (a transport event, not a protocol
//! close)". One call per connection, which fits the existing serial-bound-of-1 upstream shape.
//!
//! ## The bypass audit (`architecture.md` §11 L10 places the TLS stack off the box-byte axis; what must
//! be shown here is that this configuration does not DEFEAT the guarantee `rustls` owns)
//!
//! Every route by which certificate verification could be circumvented, and why each is absent:
//!
//! | Bypass route | Status here |
//! |---|---|
//! | A custom `ServerCertVerifier` replacing the default trust path | **Not used.** [`client_config`] calls only `with_root_certificates`, which installs `rustls`'s own verifier. |
//! | `dangerous_configuration` — the explicitly-named escape hatch for disabling verification | **Not used, and not reachable**: the API lives behind `ClientConfig::dangerous()`, and this crate never names it. |
//! | Hostname verification skipped | **Active.** The server name is passed to [`ClientConnection::new`] and `rustls` matches it against the certificate; there is no code path here that constructs a connection without one. |
//! | An empty or attacker-influenced root store | **Compiled-in and non-empty**: `webpki-roots::TLS_SERVER_ROOTS`, pinned at build time. Never the platform store, which would be ambient authority — an administrator-added CA would silently widen who may impersonate the upstream. |
//! | An unnamed crypto provider resolved from whatever is linked | **Explicit.** [`client_config`] selects `ring` by name via `builder_with_provider`. `rustls` 0.23's plain `builder()` resolves a *process-default* provider and panics if it cannot — a security-relevant choice left to link order. |
//! | TLS 1.2 downgrade | **Unavailable.** `tls12` is not enabled in `Cargo.toml`, so this client speaks TLS 1.3 only. |
//!
//! **The audit is necessary and NOT sufficient.** A disabled verifier connects perfectly, streams a
//! response, and satisfies every functional test — a successful round trip is *zero* evidence that
//! verification is on. What closes it is the negative-control run in the `negative_control` module below:
//! an
//! untrusted certificate, a certificate for the wrong hostname, and an expired certificate must each be
//! **refused**. The table above says the bypasses are absent; only the refusals show the verifier is
//! actually running.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

/// How long to wait for the TCP connection to the upstream host.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The build the verified client configuration is made from — **the audited configuration**, in one
/// place so the audit above has a single subject rather than a scattering of call sites.
///
/// Every security-relevant choice is explicit here: the crypto provider (`ring`, by name), the trust
/// anchors (compiled-in `webpki-roots`), and the default certificate verifier (installed by
/// `with_root_certificates`, never replaced). Nothing in this function reaches for
/// `ClientConfig::dangerous()`.
fn client_config() -> Arc<ClientConfig> {
    // The trust anchors: compiled in, pinned at build time, non-empty by construction.
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    client_config_with_roots(roots)
}

/// The audited build, with the trust anchors as the ONE parameter.
///
/// Production supplies the compiled-in `webpki-roots`; the negative-control tests supply a locally
/// generated CA. That single difference is deliberate and is what makes a local test meaningful: every
/// other security-relevant element — the named crypto provider, `rustls`'s own certificate verifier,
/// hostname matching, the TLS-1.3-only version set — is *this* function's, so the tests exercise the
/// real verification path rather than a parallel one built for testing. A test that had to construct its
/// own config would prove nothing about the config that ships.
fn client_config_with_roots(roots: RootCertStore) -> Arc<ClientConfig> {
    // The crypto provider is named, not resolved. `ClientConfig::builder()` would take a *process-default*
    // provider — a security property decided by what happens to be linked, and a panic if nothing is.
    let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the compiled-in protocol versions (TLS 1.3 only)")
        // Installs rustls's OWN verifier over these roots. Not a custom `ServerCertVerifier`.
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

/// A connected, verified TLS stream to one upstream host — the pipe itself.
///
/// Implements [`Read`] and [`Write`] over opaque bytes and offers nothing else: there is no method here
/// that interprets what flows through, because interpreting it is precisely what must not happen.
pub struct TlsPipe {
    stream: StreamOwned<ClientConnection, TcpStream>,
}

impl TlsPipe {
    /// Connect to `host:port` and complete a TLS handshake **with the certificate verified against the
    /// compiled-in roots and matched to `host`**.
    ///
    /// `host` is the name verification is performed against, so it must come from the operator's vetted
    /// destination — never from anything a response could influence. (Binding it to the vetted route is
    /// the *next* sprint's subject; this one supplies the mechanism that makes such a binding meaningful.)
    ///
    /// A failed handshake — untrusted issuer, wrong hostname, expired certificate — surfaces as an `Err`
    /// here and no bytes are ever exchanged.
    pub fn connect(host: &str, port: u16, io_timeout: Duration) -> io::Result<Self> {
        Self::connect_with_config(host, port, io_timeout, client_config())
    }

    /// [`TlsPipe::connect`]'s body, with the verified configuration injected. Private: the production
    /// entry point above is the only way in from outside this module, so no caller can supply a
    /// weakened configuration. The negative-control tests use it to trust a local CA — varying the trust
    /// anchors only, never the verifier.
    pub(crate) fn connect_with_config(
        host: &str,
        port: u16,
        io_timeout: Duration,
        config: Arc<ClientConfig>,
    ) -> io::Result<Self> {
        // Resolve and connect with a bounded wait: an unreachable or black-holed host must fail, never
        // hang a worker thread forever.
        //
        // EVERY resolved address is tried, not just the first. A host publishing both A and AAAA records
        // on a machine that can route only one family resolves fine and then refuses on the wrong one —
        // taking only `.next()` would turn a reachable upstream into a hard failure depending on
        // resolution order. (Found by this module's own success-arm test, which connected to ::1 while
        // the listener held 127.0.0.1.)
        let addrs: Vec<_> = (host, port).to_socket_addrs()?.collect();
        if addrs.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no address for upstream host"));
        }
        let mut last_err = None;
        let mut connected = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(sock) => {
                    connected = Some(sock);
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        let mut tcp = connected.ok_or_else(|| {
            last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address for upstream host"))
        })?;
        // Bounded reads AND writes: preserves the property Sprint 11 built with ureq's global timeout —
        // a hung upstream fails the call instead of leaking the worker thread that owns it.
        tcp.set_read_timeout(Some(io_timeout))?;
        tcp.set_write_timeout(Some(io_timeout))?;
        tcp.set_nodelay(true)?;

        // The name verification is performed against. Owned, so the connection does not borrow `host`.
        let server_name = ServerName::try_from(host)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid upstream host name"))?
            .to_owned();
        let mut conn = ClientConnection::new(config, server_name)
            .map_err(|e| io::Error::other(format!("tls setup failed: {e}")))?;

        // **DRIVE THE HANDSHAKE HERE — do not let it happen lazily.**
        //
        // `rustls` performs the handshake, and therefore CERTIFICATE VERIFICATION, on the first I/O, not
        // at construction. Returning the wrapper without this loop returns a pipe that has verified
        // NOTHING: `connect` reports success, the caller believes it holds a verified connection, and an
        // untrusted, expired, or wrong-host certificate is only rejected later — at the first read or
        // write, somewhere else, as a different kind of error.
        //
        // This is not hypothetical. The first version of this function did exactly that, and the module's
        // static audit was entirely accurate about it the whole time: the verifier WAS configured, the
        // roots WERE right, no bypass API was used — and all three refusal arms still failed, because
        // verification had not run yet when the value was handed back. That is precisely why the audit
        // alone cannot close this bar and the negative-control run is required.
        while conn.is_handshaking() {
            conn.complete_io(&mut tcp)
                .map_err(|e| io::Error::other(format!("tls handshake failed: {e}")))?;
        }

        Ok(TlsPipe { stream: StreamOwned::new(conn, tcp) })
    }

    /// Read the whole response into `sink` until the peer closes the connection — **close-delimited**,
    /// so nothing here inspects the bytes to discover where they end.
    ///
    /// Returns the number of bytes relayed. A clean TLS close and a plain TCP EOF are both "the response
    /// ended"; either is `Ok`. This is the read half of the pipe, kept here so Slice 2's relay is a
    /// straight forward of what this yields.
    pub fn read_to_end_into(&mut self, sink: &mut impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<usize> {
        let mut buf = [0u8; 16 * 1024];
        let mut total = 0usize;
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Ok(total), // EOF: the peer closed — the response is complete.
                Ok(n) => {
                    sink(&buf[..n])?;
                    total += n;
                }
                // A peer that closes without a TLS close_notify still ended the response; treat the
                // truncation as the end rather than losing bytes already relayed.
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(total),
                Err(e) => return Err(e),
            }
        }
    }
}

impl Read for TlsPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Write for TlsPipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}


#[cfg(test)]
pub(crate) mod testserver {
    //! The local TLS server the whole sprint's offline evidence runs against — shared, because two
    //! modules need the same one and a second copy would be a second thing to keep true.
    //!
    //! [`negative_control`](super::negative_control) uses it to present certificates that must be
    //! REFUSED; Slice 2's upstream tests use it to capture the exact bytes the proxy puts on the wire and
    //! to reply with scripted responses (a 302, a gzip-labelled body) that must be relayed untouched. No
    //! external service, no network, no bench hardware.
    //!
    //! Only the TRUST ANCHORS differ from production (a local CA instead of `webpki-roots`); the
    //! verifier, the named crypto provider, hostname matching, and the TLS-1.3-only version set are the
    //! shipped ones, via the shared [`client_config_with_roots`](super::client_config_with_roots).
    //!
    //! **The `rustls` SERVER API used here is this test double, not the proxy.** It impersonates the
    //! *upstream* — the peer the proxy dials — so that certificate refusals can be proven without a
    //! network. The proxy itself is a TLS client only and never terminates TLS from the box
    //! (`cloud/approach.md`, "no inbound TLS handshake ever"); this module is `#[cfg(test)]` and no part
    //! of it reaches the shipped binary.

    use super::*;
    use rcgen::{CertificateParams, DnType, IsCa, KeyPair, SanType};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::ServerConfig;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime};

    pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(5);

    /// A locally generated CA plus a leaf issued by it, with the leaf's SAN and validity window under
    /// the test's control — the three knobs the negative arms need.
    pub(crate) struct Fixture {
        ca_der: CertificateDer<'static>,
        leaf_chain: Vec<CertificateDer<'static>>,
        leaf_key: PrivateKeyDer<'static>,
    }

    pub(crate) fn fixture(san: &str, not_before: SystemTime, not_after: SystemTime) -> Fixture {
        let ca_key = KeyPair::generate().expect("ca key");
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.distinguished_name.push(DnType::CommonName, "s18 test ca");
        let ca = ca_params.self_signed(&ca_key).expect("ca self-sign");

        let leaf_key = KeyPair::generate().expect("leaf key");
        let mut leaf_params = CertificateParams::default();
        leaf_params.subject_alt_names = vec![SanType::DnsName(san.try_into().expect("san"))];
        leaf_params.not_before = not_before.into();
        leaf_params.not_after = not_after.into();
        leaf_params.distinguished_name.push(DnType::CommonName, san);
        let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).expect("leaf sign");

        Fixture {
            ca_der: ca.der().clone(),
            leaf_chain: vec![leaf.der().clone()],
            leaf_key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        }
    }

    /// A one-shot TLS listener presenting `fx`'s leaf. Returns the port; the thread serves one
    /// connection and exits, so nothing leaks between tests.
    pub(crate) fn serve_once(fx: &Fixture) -> u16 {
        let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(fx.leaf_chain.clone(), fx.leaf_key.clone_key())
            .expect("server cert");
        // Bind via the SAME name the client resolves, so listener and client agree on address family.
        let listener = TcpListener::bind(("localhost", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let cfg = Arc::new(cfg);
        let (ready, wait) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = ready.send(());
            if let Ok((sock, _)) = listener.accept() {
                // Drive the handshake far enough for the client to see the certificate, then drop.
                if let Ok(conn) = rustls::ServerConnection::new(cfg) {
                    let mut tls = StreamOwned::new(conn, sock);
                    let mut buf = [0u8; 256];
                    let _ = tls.read(&mut buf);
                }
            }
        });
        let _ = wait.recv();
        port
    }

    pub(crate) fn trusting(fx: &Fixture) -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        roots.add(fx.ca_der.clone()).expect("add test ca");
        client_config_with_roots(roots)
    }

    pub(crate) fn now_offset(secs: i64) -> SystemTime {
        if secs >= 0 {
            SystemTime::now() + Duration::from_secs(secs as u64)
        } else {
            SystemTime::now() - Duration::from_secs((-secs) as u64)
        }
    }

    /// A one-shot TLS listener that **captures the exact request bytes** and replies with `response`
    /// verbatim, then closes. The capture is what makes the request-side proof possible: the assertion is
    /// against the literal octets that crossed the wire, not against the arguments the caller passed in.
    ///
    /// Returns `(port, request_bytes)`. The receiver yields once the connection has been served.
    ///
    /// Reads until the request is complete rather than for a fixed count: headers end at the blank line,
    /// and the body length is taken from the `content-length` the PROXY wrote. That is not the proxy
    /// parsing a response — this is the test server, standing in for the upstream, doing what any HTTP
    /// server does with a request. Nothing in `proxy` gains a parser from it.
    pub(crate) fn serve_scripted(fx: &Fixture, response: &'static [u8]) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(fx.leaf_chain.clone(), fx.leaf_key.clone_key())
            .expect("server cert");
        let listener = TcpListener::bind(("localhost", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let cfg = Arc::new(cfg);
        let (captured, requests) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((sock, _)) = listener.accept() else { return };
            let Ok(conn) = rustls::ServerConnection::new(cfg) else { return };
            let mut tls = StreamOwned::new(conn, sock);
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                match tls.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        req.extend_from_slice(&buf[..n]);
                        if let Some(h) = find_headers_end(&req)
                            && req.len() >= h + content_length(&req[..h])
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tls.write_all(response);
            let _ = tls.flush();
            tls.conn.send_close_notify();
            let _ = tls.flush();
            let _ = captured.send(req);
        });
        (port, requests)
    }

    /// Byte offset just past the request's blank line, or `None` if the headers have not all arrived.
    fn find_headers_end(req: &[u8]) -> Option<usize> {
        req.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
    }

    /// The `content-length` the request declared, or 0 — the test server's own framing, not the proxy's.
    fn content_length(headers: &[u8]) -> usize {
        String::from_utf8_lossy(headers)
            .lines()
            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string()))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

}

#[cfg(test)]
mod negative_control {
    //! **The half a static audit cannot supply.** The module-level bypass table shows every
    //! verification-defeating route is absent; that is necessary and not sufficient, because a disabled
    //! verifier connects perfectly and streams a response — a successful round trip is *zero* evidence
    //! that verification is on. These tests are the evidence: certificates that must be REFUSED, and one
    //! that must be ACCEPTED so the refusals cannot be passing vacuously.

    use super::testserver::{IO_TIMEOUT, fixture, now_offset, serve_once, trusting};
    use super::*;

    /// Assert a refusal arm failed **for the right reason**, not merely that it failed.
    ///
    /// `r.is_err()` alone is not a proof: a listener that was not ready, a port race, or a fixture that
    /// minted the wrong validity window would each produce `Err` from `connect` and satisfy it, having
    /// tested nothing (reviewer's finding — and the same vacuous-pass class this module's own success arm
    /// exists to rule out, which I had guarded against in one direction and not the other).
    ///
    /// Two checks, because they exclude different things:
    /// - the **handshake marker** proves the failure happened during TLS verification rather than while
    ///   connecting — an infrastructural failure never reaches the handshake and so never carries it;
    /// - the **arm-specific reason** proves *this* arm failed on *its own* property: the wrong-host arm on
    ///   the name and the expired arm on validity, not incidentally on each other or on trust.
    fn assert_refused(r: io::Result<TlsPipe>, expected_reason: &str, arm: &str) {
        let err = match r {
            Ok(_) => panic!("{arm}: the connection was ACCEPTED — verification did not refuse it"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("tls handshake failed:"),
            "{arm}: failed before the handshake, so verification was never exercised: {err}",
        );
        assert!(
            err.contains(expected_reason),
            "{arm}: refused, but not for its own reason (wanted {expected_reason:?}): {err}",
        );
    }

    /// **THE SUCCESS ARM.** A correctly-issued, in-date certificate for the host being connected to is
    /// ACCEPTED. Without this the refusals below would be worthless — a client that rejects everything
    /// passes every negative test while verifying nothing.
    #[test]
    fn a_valid_certificate_for_the_right_host_connects() {
        let fx = fixture("localhost", now_offset(-3600), now_offset(3600));
        let port = serve_once(&fx);
        let r = TlsPipe::connect_with_config("localhost", port, IO_TIMEOUT, trusting(&fx));
        assert!(r.is_ok(), "a valid, trusted, in-date certificate must be accepted: {:?}", r.err());
    }

    /// **REFUSAL ARM 1 — untrusted issuer.** The same certificate, but the client does NOT trust the CA
    /// that issued it (production's `webpki-roots` stands in for "some other trust anchor set"). This is
    /// the arm a disabled verifier fails most obviously.
    #[test]
    fn an_untrusted_issuer_is_refused() {
        let fx = fixture("localhost", now_offset(-3600), now_offset(3600));
        let port = serve_once(&fx);
        // Production's own trust anchors: the local CA is not among them.
        let r = TlsPipe::connect("localhost", port, IO_TIMEOUT);
        assert_refused(r, "UnknownIssuer", "untrusted issuer");
    }

    /// **REFUSAL ARM 2 — wrong hostname.** Trusted issuer, in date, but issued for a different name than
    /// the one connected to. A separate code path in `rustls` from trust-chain validation: a
    /// configuration can get chain verification right and name checking wrong.
    #[test]
    fn a_certificate_for_the_wrong_hostname_is_refused() {
        let fx = fixture("not-the-host.example", now_offset(-3600), now_offset(3600));
        let port = serve_once(&fx);
        let r = TlsPipe::connect_with_config("localhost", port, IO_TIMEOUT, trusting(&fx));
        assert_refused(r, "not valid for name", "wrong hostname");
    }

    /// **REFUSAL ARM 3 — expired.** Trusted issuer, right hostname, but outside its validity window.
    /// Time validity is a third distinct path from chain trust and name matching.
    #[test]
    fn an_expired_certificate_is_refused() {
        let fx = fixture("localhost", now_offset(-7200), now_offset(-3600)); // expired an hour ago
        let port = serve_once(&fx);
        let r = TlsPipe::connect_with_config("localhost", port, IO_TIMEOUT, trusting(&fx));
        assert_refused(r, "certificate expired", "expired certificate");
    }
}
