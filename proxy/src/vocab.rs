//! Shared vocabulary (Sprint 5, Slice 2) — the `Address` / `Route` / `ToolIdentity` types that
//! U2 (address recognizer), U3 (route matcher), and U5 (register) all agree on, pinned in **one**
//! place before U2 and U5 fork so neither redefines them divergently.
//!
//! The address vocabulary **is** `proto::NetAddress`'s — re-exported here, agreed-with, never
//! redefined (the Slice-2 constraint). The canonical assembly format ([`assemble`]) is pinned
//! here too — the one shared producer — so U2's runtime assembly of an incoming address and U5's
//! compiled-in routes are byte-identical *by construction*, not by two implementations happening
//! to match.

use alloc::string::String;

/// The address vocabulary is proto's — re-exported so every consumer imports it from one place
/// (guaranteeing agreement). `Address` is the box's decoded `net-address`
/// (`proto::NetAddress` — `{ scheme, address: safe segment stack }`, `net.md` §1).
pub use proto::{NetAddress as Address, ReservedScheme, SafeSegment, SafeSegmentStack, Scheme};

/// The canonical assembly format (`spec/trusted-address.md` §3), pinned **once** so U2's runtime
/// assembly and U5's compiled-in routes produce byte-identical canonical forms:
///
/// > `https` `://` `<domain>` ( `/` `<path-segment>` )*
///
/// The proxy supplies `://` and `/`; the safe alphabet contains neither, so assembly is
/// **injective** — distinct addresses never collide in the match or the log
/// (`spec/trusted-address.md` §3). The first segment is the domain, the rest the path
/// (`net.md` §1).
///
/// Returns `None` for a non-https (reserved) scheme: the canonical form is defined only for the
/// admittable scheme. The scheme *policy* (deny reserved, deny-whole-request) is **U2's gate**,
/// not this format producer — `assemble` only produces the canonical form where one exists.
/// Total and panic-free: an empty stack (unreachable for a scanned address) yields `None`.
#[must_use]
pub fn assemble(addr: &Address) -> Option<String> {
    match addr.scheme {
        Scheme::Https => {}
        Scheme::Other(_) => return None,
    }
    // `split_first` is `None` only for an empty stack — unreachable for a `scan_safe` output
    // (≥1 segment), but handling it keeps `assemble` total.
    let (domain, path) = addr.address.segments().split_first()?;
    let mut out = String::from("https://");
    out.push_str(domain.as_str());
    for seg in path {
        out.push('/');
        out.push_str(seg.as_str());
    }
    Some(out)
}

/// A canonical route the register (U5) holds and the matcher (U3) byte-exact compares an
/// assembled [`Address`] against. Its interior is the canonical [`assemble`] form.
///
/// The interior is **private**; the **only** constructor is [`Route::new`], which
/// compile-time-validates its input (constraint C1). There is no unchecked path — no public
/// field, no `Default`, no `From`, no `Deserialize` — so "valid by type" is sound for U3 to rely
/// on without a runtime re-check (the U5 bypass audit, `005` Method part 4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Route(&'static str);

impl Route {
    /// Construct a canonical route, **validated at compile time** when called in a `const`
    /// context (as U5's register is): a malformed route makes the `assert!` fail const-evaluation,
    /// which is a **compile error** (constraint C1). Validity = *in the image of [`assemble`]*
    /// (`https://` + a safe domain + `/`-joined safe path segments) — so a dead register key that
    /// could never match any box request (`spec/trusted-address.md` JT-3) cannot be written.
    ///
    /// # Panics
    /// Panics (→ compile error in a `const`) if `canonical` is not a well-formed canonical route.
    #[must_use]
    pub const fn new(canonical: &'static str) -> Self {
        assert!(
            is_canonical_route(canonical),
            "malformed route: not in the image of assemble (https:// + safe domain + safe path)",
        );
        Route(canonical)
    }

    /// The canonical bytes U3 byte-exact matches an assembled address against. Returns the
    /// underlying `&'static str` — the route lives for the program, decoupled from `&self`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.0
    }

    /// Split the canonical form into `(host, path)` — **the one decomposition of a `Route` in the
    /// crate**, deliberately living on the type rather than being re-derived per consumer.
    ///
    /// Two places need it and must not disagree: U4's envelope check (which proves the matched route IS
    /// the vetted destination) and the upstream dial (which goes there). Separate implementations that
    /// happen to agree would be exactly the gap Sprint 19 removed one level down — the register and the
    /// dial agreeing by authorship rather than by construction — so there is one implementation and both
    /// read it.
    ///
    /// A canonical `Route` is `https://` + host + optional `/`-joined path (guaranteed by
    /// [`Route::new`]'s compile-time validation), so this is total: a bare domain yields `"/"`.
    #[must_use]
    pub fn split(&self) -> (&'static str, &'static str) {
        let rest = self.0.strip_prefix("https://").unwrap_or(self.0);
        match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]), // the path keeps its leading '/'
            None => (rest, "/"),                 // bare domain → root path
        }
    }
}

/// A compiled-in **filesystem root** path — the quarantine root (write target) or the readable root
/// (read scope), `architecture.md` §8's second configured thing (Sprint 16).
///
/// **Absolute-only, enforced at compile time.** A relative path would resolve against the process's
/// current working directory — an implicit, mutable, ambient input, unlike every other authority in this
/// design, which is explicit. Rejecting relative paths in the `const` removes the question entirely
/// rather than documenting a resolution base nobody would remember to check.
///
/// The interior is **private**; the only constructor is [`RootPath::new`]. Structural validity is
/// compile-checked; whether the path *exists*, is a directory, and carries the required mount
/// protections is a **runtime** authority check (the startup ladder's `FilesystemRoots` and
/// `KernelProtections` rungs) — a `const` cannot stat the filesystem, and pretending otherwise would be
/// the compile-time gate claiming more than it can know.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RootPath(&'static str);

impl RootPath {
    /// Construct a filesystem-root path, **structure-validated at compile time** in a `const` context:
    /// empty, relative, or NUL-bearing is a compile error.
    ///
    /// # Panics
    /// Panics (→ compile error in a `const`) if `path` is not a non-empty, absolute, NUL-free path.
    #[must_use]
    pub const fn new(path: &'static str) -> Self {
        assert!(
            is_absolute_root_path(path),
            "malformed filesystem root: expected a non-empty ABSOLUTE path (leading '/') with no NUL",
        );
        RootPath(path)
    }

    /// The path's bytes, for the runtime resolve/pin at the authority rung.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// **Where the upstream credential comes from** — never the credential itself (Sprint 16).
///
/// `architecture.md` §8 makes the credential's **custody** the configured thing ("proxy-attached vs
/// on-box is explicitly an operational decision"), not its value. So this descriptor names an
/// **environment variable**, compiled in and structurally validated like every other register entry;
/// the secret itself is read from that variable at startup and **never appears in source** (Sprint 11's
/// standing rule: a live secret is not committed, independent of anything proxy-specific).
///
/// What compiling the *descriptor* buys — stated plainly, because it is real but modest: it fixes **the
/// one place the proxy will ever look**, so no runtime input can redirect the credential's source, and
/// the register's C1/C3 properties hold (the box cannot author it; a malformed name will not compile).
/// It is deliberately **not** the same security weight as [`Route`], which fixes an effect boundary —
/// this fixes a lookup location.
///
/// The interior is **private** and holds only a variable *name*; the **only** constructor is
/// [`CredentialSource::env`]. There is no constructor, field, or variant that can carry a secret
/// **value**, so "a literal secret compiled into the register" is not merely discouraged — it is
/// *inexpressible* (bypass audit, `tests/ui/credential_source_no_literal_bypass.rs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CredentialSource(&'static str);

impl CredentialSource {
    /// Name the environment variable the upstream credential is read from, **structure-validated at
    /// compile time** in a `const` context: an empty or malformed variable name is a compile error.
    /// Validity is the POSIX environment-variable-name shape, `[A-Za-z_][A-Za-z0-9_]*` — structural
    /// only; *which* variable (and what it holds) stays the operator's call.
    ///
    /// # Panics
    /// Panics (→ compile error in a `const`) if `var_name` is not a well-formed env-var name.
    #[must_use]
    pub const fn env(var_name: &'static str) -> Self {
        assert!(
            is_env_var_name(var_name),
            "malformed credential source: expected a non-empty [A-Za-z_][A-Za-z0-9_]* env var name",
        );
        CredentialSource(var_name)
    }

    /// The environment variable's name — the only thing this descriptor carries. Safe to log: it is a
    /// variable *name*, never the secret it holds.
    #[must_use]
    pub const fn var_name(&self) -> &'static str {
        self.0
    }
}

/// A vetted client-side tool identity (JA-1) as a typed newtype, so a raw `&str` **never** reaches
/// U4's recognizer — the recognizer consumes `ToolIdentity`, not an unchecked string.
///
/// The interior is **private**; the **only** constructor is [`ToolIdentity::new`], which
/// compile-time-validates *structure* (a non-empty tool-type token). Which specific identities the
/// operator trusts (JA-1) stays the operator's separate call — U5 makes the set typed and
/// well-formed, it never decides trust. No unchecked construction path exists (bypass audit).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ToolIdentity(&'static str);

impl ToolIdentity {
    /// Construct a vetted tool identity, **structure-validated at compile time** in a `const`
    /// context: a malformed token (empty, or a byte outside the tool-type charset `[A-Za-z0-9_]`)
    /// is a compile error. This is *structural* well-formedness only — trust (which identities) is
    /// the operator's, expressed by *which* ones appear in the register.
    ///
    /// # Panics
    /// Panics (→ compile error in a `const`) if `identity` is not a well-formed tool-type token.
    #[must_use]
    pub const fn new(identity: &'static str) -> Self {
        assert!(
            is_tool_identity(identity),
            "malformed tool identity: expected a non-empty [A-Za-z0-9_] token",
        );
        ToolIdentity(identity)
    }

    /// The identity's bytes, for U4's exact-string match against the request's `tools[]` entries.
    /// Returns the underlying `&'static str`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// One byte of the **safe alphabet** `[A-Za-z0-9._-]` (the const-time twin of `proto::scan_safe`'s
/// alphabet). `is_alnum` is reported separately for the ≥1-alphanumeric rule. This is deliberately
/// the same alphabet as the wire scan; the cross-check test (`register.rs` suite) sweeps all 256
/// bytes to prove this const twin and `proto::scan_safe` never diverge.
const fn classify_safe_byte(c: u8) -> (bool, bool) {
    let is_alnum = c.is_ascii_alphanumeric();
    let in_alphabet = is_alnum || c == b'.' || c == b'_' || c == b'-';
    (in_alphabet, is_alnum)
}

/// A canonical-route validator: `https://` + a safe domain segment + `/`-joined safe path
/// segments, each segment non-empty, in `[A-Za-z0-9._-]`, with ≥1 alphanumeric, and never `.` or
/// `..`. Const, flat byte-walk (no subslicing) — boring by construction (constraint C5).
const fn is_canonical_route(s: &str) -> bool {
    let b = s.as_bytes();
    let n = b.len();
    // Prefix "https://" (8 bytes).
    let pfx = b"https://";
    if n < 8 {
        return false;
    }
    let mut i = 0;
    while i < 8 {
        if b[i] != pfx[i] {
            return false;
        }
        i += 1;
    }
    // Walk `/`-separated segments from index 8; end-of-string terminates the final segment.
    let mut seg_len: usize = 0;
    let mut seg_has_alnum = false;
    let mut seg_all_dots = true;
    i = 8;
    while i <= n {
        let at_end = i == n;
        let c = if at_end { b'/' } else { b[i] };
        if c == b'/' {
            // Segment boundary — validate the just-finished segment.
            if seg_len == 0 {
                return false; // empty segment (`//`, trailing `/`, or nothing after `https://`)
            }
            if seg_all_dots && (seg_len == 1 || seg_len == 2) {
                return false; // "." or ".."
            }
            if !seg_has_alnum {
                return false; // ≥1 alphanumeric
            }
            seg_len = 0;
            seg_has_alnum = false;
            seg_all_dots = true;
            if at_end {
                break;
            }
        } else {
            let (in_alphabet, is_alnum) = classify_safe_byte(c);
            if !in_alphabet {
                return false; // space, `:`, `%`, control, etc.
            }
            if is_alnum {
                seg_has_alnum = true;
            }
            if c != b'.' {
                seg_all_dots = false;
            }
            seg_len += 1;
        }
        i += 1;
    }
    true
}

/// A filesystem-root-path validator: non-empty, **absolute** (a leading `/`), and free of interior NUL
/// (which no C path can carry — `pin_root`'s `CString::new` would reject it at runtime, so catching it
/// here turns a startup failure into a build failure). Const, flat byte-walk, total on every `&str`.
/// Structural only: existence, directory-ness, and mount protections are the runtime authority rungs'.
const fn is_absolute_root_path(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b[0] != b'/' {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0 {
            return false;
        }
        i += 1;
    }
    true
}

/// An environment-variable-name validator: the POSIX shape `[A-Za-z_][A-Za-z0-9_]*` — non-empty, no
/// leading digit, no `=`, no whitespace, no punctuation. Const, flat byte-walk, total on every `&str`.
/// Structural only; *which* variable is the operator's call. A name outside this shape could not name
/// a real variable, so rejecting it at compile time turns a silently-never-resolving credential source
/// into a build failure.
const fn is_env_var_name(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    // First byte: a letter or underscore — never a digit (a leading digit is not a valid name).
    if !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        let c = b[i];
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return false;
        }
        i += 1;
    }
    true
}

/// A tool-identity validator: a non-empty token over `[A-Za-z0-9_]` (the tool-type charset —
/// `text_editor_20250728`, `bash_20250124`). Const, flat byte-walk. Structural only; trust is the
/// operator's (which identities appear in the register).
const fn is_tool_identity(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod validator_proof {
    //! U5 part 3 — the validating constructor's own total-function proof (`005` Method part 3): a
    //! compile-time gate is only as sound as the check it runs. Plus the **alphabet cross-check**
    //! that closes the one real risk of a const twin: `classify_safe_byte` must agree with
    //! `proto::scan_safe`'s alphabet over all 256 bytes, or a route could be compile-valid yet
    //! never match a runtime address (or vice-versa). Oracle = `proto::scan_safe` (independent).
    use super::{classify_safe_byte, is_absolute_root_path, is_canonical_route, is_env_var_name, is_tool_identity};

    const US: u8 = 0x1F;

    #[test]
    fn canonical_route_validator_boundaries() {
        // Accepts: the canonical forms in the image of `assemble`.
        assert!(is_canonical_route("https://api.anthropic.com/v1/messages"));
        assert!(is_canonical_route("https://example.com")); // bare domain, no trailing slash
        assert!(is_canonical_route("https://h/a/b/c"));
        assert!(is_canonical_route("https://a.b-c_d/x.y")); // full safe alphabet in segments
        // Rejects — each a distinct failure arm:
        assert!(!is_canonical_route("http://example.com")); // wrong scheme prefix
        assert!(!is_canonical_route("https://")); // no host (empty segment)
        assert!(!is_canonical_route("https://a//b")); // empty middle segment
        assert!(!is_canonical_route("https://a/")); // trailing slash → empty final segment
        assert!(!is_canonical_route("https://a b")); // space (out of alphabet)
        assert!(!is_canonical_route("https://a:8443")); // ':' (port smuggling) out of alphabet
        assert!(!is_canonical_route("https://a/../b")); // ".." traversal segment
        assert!(!is_canonical_route("https://a/./b")); // "." segment
        assert!(!is_canonical_route("https://...")); // all-dots, no alnum (and len-3 dots)
        assert!(!is_canonical_route("https://__/x")); // segment with no alphanumeric
        assert!(!is_canonical_route("")); // empty
        assert!(!is_canonical_route("https:/x")); // truncated prefix
    }

    #[test]
    fn tool_identity_validator_boundaries() {
        assert!(is_tool_identity("text_editor_20250728"));
        assert!(is_tool_identity("bash_20250124"));
        assert!(is_tool_identity("A1_b2"));
        assert!(!is_tool_identity("")); // empty
        assert!(!is_tool_identity("has space"));
        assert!(!is_tool_identity("web_fetch-20250910")); // '-' not in the tool-type charset
        assert!(!is_tool_identity("a/b")); // '/'
        assert!(!is_tool_identity("café")); // non-ASCII
    }

    /// The filesystem-root validator's own boundaries (Sprint 16). Both arms proven.
    #[test]
    fn absolute_root_path_validator_boundaries() {
        assert!(is_absolute_root_path("/run/zerotrust-proxy/quarantine")); // the registered quarantine
        assert!(is_absolute_root_path("/srv/zerotrust-proxy/readable")); // the registered readable root
        assert!(is_absolute_root_path("/")); // the root directory is absolute and well-formed
        assert!(!is_absolute_root_path("")); // empty
        assert!(!is_absolute_root_path("run/quarantine")); // RELATIVE — resolves against an ambient cwd
        assert!(!is_absolute_root_path("./quarantine")); // relative, explicitly
        assert!(!is_absolute_root_path("../quarantine")); // relative, and escaping
        assert!(!is_absolute_root_path("/a\0b")); // interior NUL — no C path can carry one
    }

    /// The env-var-name validator's own boundaries (Sprint 16) — the compile-time gate on the credential
    /// SOURCE is only as sound as this check. Both arms proven: well-formed names accept (so the gate is
    /// not vacuously-closed), malformed ones reject.
    #[test]
    fn env_var_name_validator_boundaries() {
        assert!(is_env_var_name("ANTHROPIC_API_KEY")); // the registered one
        assert!(is_env_var_name("_")); // bare underscore is a legal name
        assert!(is_env_var_name("_x9")); // leading underscore, digits after the first byte
        assert!(is_env_var_name("a")); // single letter
        assert!(!is_env_var_name("")); // empty — the case the const gate must reject
        assert!(!is_env_var_name("1KEY")); // leading digit
        assert!(!is_env_var_name("ANTHROPIC-API-KEY")); // '-' — a plausible typo, cannot name a var
        assert!(!is_env_var_name("A B")); // space
        assert!(!is_env_var_name("A=B")); // '=' — the env-string separator
        assert!(!is_env_var_name("KEY$")); // punctuation
        assert!(!is_env_var_name("clé")); // non-ASCII
        // A pasted SECRET does not look like a variable name — the gate catches the paste-in-the-wrong-
        // -place mistake structurally, on top of the value being inexpressible at all (bypass audit).
        assert!(!is_env_var_name("sk-ant-api03-XXXX"));
    }

    /// STACKS-LEVEL agreement (reviewer's Slice-3b finding, closed here): `is_canonical_route`
    /// twins `proto::scan_safe`'s **structure** (empty-segment, traversal, ≥1-alnum) as well as its
    /// alphabet — a second twin surface. This proves the *whole* validator never diverges from the
    /// wire scan: for each curated segment list, the route form (`https://` + join `/`) and the wire
    /// form (join `0x1F`) must agree on validity. Oracle = `proto::scan_safe` (independent). A drift
    /// on either surface — alphabet or structure — fails here. (Separators `/` and `0x1F` are never
    /// segment *content*, so they are excluded from the generated segments by construction.)
    #[test]
    fn const_validator_agrees_with_scan_safe_at_the_stacks_level() {
        // A curated segment palette spanning every structural/alphabet arm.
        let palette = [
            "api", "v1", "a.b-c_d", "docs", // valid
            "",     // empty segment
            ".", "..", // traversal
            "...", "___", "-_-", // no alphanumeric
            "a b", "a:b", "a@b", "a%b", "a#b", "a?b", // out-of-alphabet bytes
        ];
        // Exhaustive over lengths 1–3 of the palette (a deterministic combinatorial sweep — no RNG).
        for &s0 in &palette {
            check_stack(&[s0]);
            for &s1 in &palette {
                check_stack(&[s0, s1]);
                for &s2 in &palette {
                    check_stack(&[s0, s1, s2]);
                }
            }
        }
        // And the empty stack (zero segments).
        check_stack(&[]);
    }

    fn check_stack(segments: &[&str]) {
        use std::string::String;
        let route = {
            let mut r = String::from("https://");
            r.push_str(&segments.join("/"));
            r
        };
        let wire = segments.join("\u{1f}");
        let route_ok = is_canonical_route(&route);
        let wire_ok = proto::scan_safe(wire.as_bytes()).is_ok();
        assert_eq!(
            route_ok, wire_ok,
            "stack {segments:?}: route form {route:?} (ok={route_ok}) must agree with wire scan (ok={wire_ok})",
        );
    }

    #[test]
    fn const_alphabet_agrees_with_proto_scan_safe_over_all_256() {
        // The one real risk of a const twin: divergence from the wire scan. Sweep every byte and
        // assert `classify_safe_byte`'s `in_alphabet` matches whether `proto::scan_safe` accepts
        // that byte inside a segment — using proto as the INDEPENDENT oracle (its own alphabet),
        // exactly the `segment_alphabet.rs` model. A one-byte drift fails here.
        for b in 0u8..=255 {
            if b == US {
                continue; // the separator — never a content byte (proto's own convention)
            }
            let (in_alphabet, is_alnum) = classify_safe_byte(b);
            // proto oracle: `[b'a', b]` — leading 'a' satisfies ≥1-alnum, so acceptance turns only
            // on whether byte `b` is in the safe alphabet.
            let proto_accepts = proto::scan_safe(&[b'a', b]).is_ok();
            assert_eq!(
                in_alphabet, proto_accepts,
                "byte {b:#04x}: const twin (in_alphabet={in_alphabet}) must match proto::scan_safe (accepts={proto_accepts})",
            );
            // And the alnum sub-classification matches the stdlib (the ≥1-alnum rule's basis).
            assert_eq!(is_alnum, b.is_ascii_alphanumeric(), "byte {b:#04x}: alnum classification");
        }
    }
}
