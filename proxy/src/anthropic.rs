//! U4 — the Anthropic body-shape recognizer (`spec/anthropic-recognizer.md`): the whole trusted
//! surface for the worst-case egress destination, with **zero rewrite behind it**. Admit only the
//! proven-inert shape, deny everything else; never rewrite, never strip-and-proceed. Pure — no I/O.
//!
//! The crux (`spec` §2) is a **bounded, total structural JSON reader** — *not* a general parsing
//! library (which would import the unbounded-grammar bet the spine forbids, resolve
//! ambiguity by its own rules, and hide uncounted bulk). It is depth-bounded, deny-default at every
//! branch, and **ambiguity-rejecting** (`spec` §5 / SA8): duplicate keys, escaped keys, trailing
//! data, and non-canonical numbers all deny, so every admitted body has exactly one interpretation
//! — what the recognizer inspected is what Anthropic executes. The size bound is `proto`'s
//! (`Reassembler::with_capacity_bound`, the 32 MiB cap): the body is already reassembly-bounded, so
//! this reader never faces an unbounded input.
//!
//! **Build-time obligation discharged here** (`spec` header): every SA3/SA8 closure row is a corpus
//! case against this real recognizer, independently re-derived from the spec (`tests/anthropic_*`).

use crate::vocab::ToolIdentity;

/// Maximum structural nesting depth. **Reviewed and confirmed at 32 (2026-07-25).** JA-3 settled the
/// *shape* (bounded, fail-closed); this reviews the number. It is conservative in **both** directions,
/// which is why it needs no tuning: too low would only over-deny (fail-safe, and 32 is far deeper than
/// the whitelisted Messages shape reaches), while too high would risk the recursion's own stack depth.
/// 32 sits clear of both. As shallow as the whitelisted Messages shape needs; deeper
/// denies (`TooDeep`). This is what makes "recurse to full depth" finite and the recursion
/// terminate.
pub const MAX_DEPTH: usize = 32;

/// The minimal known-inert top-level fields (JA-2, design-resolved). Any other key denies the whole
/// request — which is what excludes `mcp_servers` and `container` *by construction* (not on the
/// list), not by a denylist that must stay complete.
const TOPLEVEL_WHITELIST: &[&str] = &[
    "model",
    "messages",
    "system",
    "max_tokens",
    "temperature",
    "top_p",
    "top_k",
    "stop_sequences",
    "thinking",
    "tools",
    "tool_choice",
    "metadata",
    "stream",
];

/// A bare custom tool's admitted keys (`spec` §3). A no-`type` tool entry admits iff its keys are a
/// subset of these; the `input_schema` interior is client-side-inert (the box runs it) so it is
/// depth/size-bounded but not capability-policed.
const CUSTOM_TOOL_KEYS: &[&str] = &["name", "description", "input_schema", "strict", "allowed_callers"];

/// A box-authored `POST /v1/messages` request, decomposed for recognition.
pub struct Request<'a> {
    pub verb: &'a str,
    pub domain: &'a str,
    pub path: &'a str,
    /// `anthropic-beta` header values (empty admit-set — any beta denies, JA-4).
    pub betas: &'a [&'a str],
    /// The reassembled JSON body (already `proto::Reassembler`-capped; `spec` §2.1).
    pub body: &'a [u8],
}

/// Why a request was denied. Every arm is a legible, fail-closed deny; there is no rewrite.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Denial {
    /// Stage 1 — verb/domain/path not an exact `(POST, api.anthropic.com, /v1/messages)`.
    BadEnvelope,
    /// Stage 4 — a capability-gating beta header (empty admit-set).
    BetaNotInert,
    /// Stage 0 — nesting past `MAX_DEPTH`.
    TooDeep,
    /// Stage 0 — a construct outside the accepted JSON profile (bad structure, comment, trailing
    /// comma, exotic whitespace, control byte, unterminated).
    BadJson,
    /// Stage 0 — a non-canonical number (leading `+`, hex, `NaN`, leading zero, …).
    BadNumber,
    /// Stage 0 / SA8 — a malformed or key-smuggling escape (bad `\u` hex, lone surrogate, or *any*
    /// escape in a key/`type` value — those are plain-ASCII identifiers).
    BadEscape,
    /// Stage 0 / SA8 — a duplicate key within one object.
    DuplicateKey,
    /// Stage 0 / SA8 — bytes after the top-level value.
    TrailingData,
    /// Stage 2 — a top-level key not in the whitelist (excludes `mcp_servers`, `container`, unknowns).
    UnknownField,
    /// Stage 3 — a `type`-tagged tool whose type is not an exact vetted client-side identity.
    UnvettedToolType,
    /// Stage 3 — a bare (no-`type`) tool with a key outside the custom-tool set.
    BadCustomTool,
    /// Stage 3 — `tools` is not an array, or an entry is not an object.
    BadToolsShape,
}

/// Recognize a request: envelope exact-match (Stage 1), empty beta admit-set (Stage 4), then the
/// bounded structural parse + field/tool recognition (Stages 0/2/3). `Ok(())` means **admit and
/// forward the body verbatim** (never rewritten — the `cch` body hash stays valid).
pub fn recognize(req: &Request, vetted: &[ToolIdentity]) -> Result<(), Denial> {
    // Stage 1 — envelope, exact-match (the U2/U3 exact-match discipline).
    //
    // **DEFENSE IN DEPTH — DO NOT DERIVE THIS FROM THE MATCHED ROUTE** (Sprint 19, Final State #6).
    // This hardcoded pair looks like duplication of `register::net::ROUTES`, and the obvious tidy-up is
    // "the matched Route already carries the domain and path, so read them from it instead". That change
    // would remove a real safety layer while reading as a cleanup in review.
    //
    // What it currently does: the envelope handed here is derived FROM the matched route, so comparing it
    // to this fixed pair proves the matched route IS `api.anthropic.com//v1/messages`. Every request that
    // gets admitted has therefore had its destination independently confirmed here — which is what makes
    // a misdial unreachable, and what makes per-route credential scoping unnecessary today (there is no
    // admissible second destination for the Anthropic credential to follow). Derive this from the route
    // and both of those stop being true in the same commit.
    // A change here is a design fork for the proxy team, never a unilateral simplification.
    if req.verb != "POST" || req.domain != "api.anthropic.com" || req.path != "/v1/messages" {
        return Err(Denial::BadEnvelope);
    }
    // Stage 4 — betas, empty admit-set: any header denies (placed early, a cheap total reject).
    if !req.betas.is_empty() {
        return Err(Denial::BetaNotInert);
    }
    // Stage 0 + 2 + 3 — the bounded structural reader over the body.
    let mut r = Reader { b: req.body, pos: 0 };
    r.recognize_body(vetted)
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Skip *only* the four canonical JSON whitespace bytes. Exotic whitespace is left in place and
    /// becomes a structural deny at the next expectation (SA8 whitespace row).
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), Denial> {
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            Err(Denial::BadJson)
        }
    }

    /// The top-level: an object, its keys whitelisted, `tools` recognized per-entry, and **no
    /// trailing data** after it.
    fn recognize_body(&mut self, vetted: &[ToolIdentity]) -> Result<(), Denial> {
        self.skip_ws();
        self.recognize_toplevel_object(vetted, 1)?;
        self.skip_ws();
        if self.pos != self.b.len() {
            return Err(Denial::TrailingData); // SA8
        }
        Ok(())
    }

    fn recognize_toplevel_object(&mut self, vetted: &[ToolIdentity], depth: usize) -> Result<(), Denial> {
        self.expect(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(());
        }
        let mut seen: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
        loop {
            self.skip_ws();
            let key = self.parse_identifier_string()?; // key: plain ASCII, no escapes
            if seen.iter().any(|k| k == &key) {
                return Err(Denial::DuplicateKey); // SA8
            }
            if !TOPLEVEL_WHITELIST.contains(&key.as_str()) {
                return Err(Denial::UnknownField); // Stage 2 — excludes mcp_servers/container/unknown
            }
            seen.push(key.clone());
            self.skip_ws();
            self.expect(b':')?;
            if key == "tools" {
                self.recognize_tools_array(vetted, depth + 1)?; // Stage 3
            } else {
                self.skip_value(depth + 1)?; // inert — structurally validated, not policed
            }
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(()),
                _ => return Err(Denial::BadJson),
            }
        }
    }

    fn recognize_tools_array(&mut self, vetted: &[ToolIdentity], depth: usize) -> Result<(), Denial> {
        if depth > MAX_DEPTH {
            return Err(Denial::TooDeep);
        }
        self.skip_ws();
        self.expect(b'[').map_err(|_| Denial::BadToolsShape)?;
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(());
        }
        loop {
            self.recognize_tool_entry(vetted, depth + 1)?;
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(()),
                _ => return Err(Denial::BadJson),
            }
        }
    }

    /// One tool entry: a `type`-tagged entry admits only on an exact vetted identity; a bare entry
    /// admits iff its keys ⊆ [`CUSTOM_TOOL_KEYS`]. A `type`-less entry is client-executed by
    /// construction, so its *name* carries no server effect (`spec` §6 row 13).
    fn recognize_tool_entry(&mut self, vetted: &[ToolIdentity], depth: usize) -> Result<(), Denial> {
        if depth > MAX_DEPTH {
            return Err(Denial::TooDeep);
        }
        self.skip_ws();
        self.expect(b'{').map_err(|_| Denial::BadToolsShape)?;
        self.skip_ws();
        let mut keys: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
        let mut type_value: Option<alloc::string::String> = None;
        if self.peek() == Some(b'}') {
            self.bump();
            // An empty tool object has no `type` → treated as a (degenerate) bare custom tool with
            // no keys → admitted (its key set is trivially a subset). Harmless: client-side, inert.
            return Ok(());
        }
        loop {
            self.skip_ws();
            let key = self.parse_identifier_string()?;
            if keys.iter().any(|k| k == &key) {
                return Err(Denial::DuplicateKey);
            }
            self.skip_ws();
            self.expect(b':')?;
            if key == "type" {
                self.skip_ws();
                type_value = Some(self.parse_identifier_string()?); // type value: plain ASCII id
            } else {
                self.skip_value(depth + 1)?; // other keys' values: structurally bounded, not policed
            }
            keys.push(key);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => break,
                _ => return Err(Denial::BadJson),
            }
        }
        match type_value {
            Some(t) => {
                // Type-tagged: admit only on an exact vetted client-side identity.
                if vetted.iter().any(|v| v.as_str() == t) {
                    Ok(())
                } else {
                    Err(Denial::UnvettedToolType) // server-side / unknown / new-version stamp
                }
            }
            None => {
                // Bare custom: every key must be in the custom-tool set.
                for k in &keys {
                    if !CUSTOM_TOOL_KEYS.contains(&k.as_str()) {
                        return Err(Denial::BadCustomTool);
                    }
                }
                Ok(())
            }
        }
    }

    /// Structurally validate any JSON value (deny-default, depth-bounded), without recognizing it —
    /// for inert values (message content, `input_schema` interiors).
    fn skip_value(&mut self, depth: usize) -> Result<(), Denial> {
        if depth > MAX_DEPTH {
            return Err(Denial::TooDeep);
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.skip_object(depth),
            Some(b'[') => self.skip_array(depth),
            Some(b'"') => self.skip_string(),
            Some(b't') => self.expect_lit(b"true"),
            Some(b'f') => self.expect_lit(b"false"),
            Some(b'n') => self.expect_lit(b"null"),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.skip_number(),
            _ => Err(Denial::BadJson), // deny-default: comment, trailing comma, exotic ws, EOF, …
        }
    }

    fn skip_object(&mut self, depth: usize) -> Result<(), Denial> {
        if depth > MAX_DEPTH {
            return Err(Denial::TooDeep);
        }
        self.expect(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(());
        }
        let mut seen: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
        loop {
            self.skip_ws();
            let key = self.parse_identifier_string()?;
            if seen.iter().any(|k| k == &key) {
                return Err(Denial::DuplicateKey); // SA8, at every object depth
            }
            seen.push(key);
            self.skip_ws();
            self.expect(b':')?;
            self.skip_value(depth + 1)?;
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(()),
                _ => return Err(Denial::BadJson),
            }
        }
    }

    fn skip_array(&mut self, depth: usize) -> Result<(), Denial> {
        if depth > MAX_DEPTH {
            return Err(Denial::TooDeep);
        }
        self.expect(b'[')?;
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(());
        }
        loop {
            self.skip_value(depth + 1)?;
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(()),
                _ => return Err(Denial::BadJson),
            }
        }
    }

    fn expect_lit(&mut self, lit: &[u8]) -> Result<(), Denial> {
        for &want in lit {
            if self.bump() != Some(want) {
                return Err(Denial::BadJson);
            }
        }
        Ok(())
    }

    /// A **plain-ASCII identifier string** (a key or a tool `type` value): `"..."` with no escapes
    /// and no bytes ≥ `0x80`. Any escape or non-ASCII byte denies `BadEscape` — key/type smuggling
    /// (SA8 row 10) is impossible because a legitimate identifier is never escaped, and a false
    /// deny here is fail-safe (block), never a false admit.
    fn parse_identifier_string(&mut self) -> Result<alloc::string::String, Denial> {
        self.expect(b'"')?;
        let mut out = alloc::string::String::new();
        loop {
            match self.bump() {
                None => return Err(Denial::BadJson), // unterminated
                Some(b'"') => return Ok(out),
                Some(b'\\') => return Err(Denial::BadEscape), // no escapes in identifiers
                Some(c) if c < 0x20 => return Err(Denial::BadJson), // raw control
                Some(c) if c >= 0x80 => return Err(Denial::BadEscape), // non-ASCII identifier
                Some(c) => out.push(c as char),
            }
        }
    }

    /// Skip a content string, validating escape well-formedness (SA8 `BadEscape`) — bad `\u` hex and
    /// lone surrogates deny; valid escapes and surrogate pairs pass. Content need not be decoded (it
    /// is inert), only scanned to a well-formed close.
    fn skip_string(&mut self) -> Result<(), Denial> {
        self.expect(b'"')?;
        loop {
            match self.bump() {
                None => return Err(Denial::BadJson),
                Some(b'"') => return Ok(()),
                Some(b'\\') => match self.bump() {
                    Some(b'"') | Some(b'\\') | Some(b'/') | Some(b'b') | Some(b'f') | Some(b'n')
                    | Some(b'r') | Some(b't') => {}
                    Some(b'u') => {
                        let cp = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            // high surrogate — must be followed by a low-surrogate `\uXXXX`.
                            if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                                return Err(Denial::BadEscape);
                            }
                            let lo = self.parse_hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&lo) {
                                return Err(Denial::BadEscape);
                            }
                        } else if (0xDC00..=0xDFFF).contains(&cp) {
                            return Err(Denial::BadEscape); // lone low surrogate
                        }
                    }
                    _ => return Err(Denial::BadEscape),
                },
                Some(c) if c < 0x20 => return Err(Denial::BadJson), // raw control byte
                Some(_) => {} // any other byte (incl. valid UTF-8 continuation) is inert content
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, Denial> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let c = self.bump().ok_or(Denial::BadEscape)?;
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => return Err(Denial::BadEscape),
            };
            v = v * 16 + d as u32;
        }
        Ok(v)
    }

    /// A canonical JSON number: optional `-`, an int (`0` or `[1-9][0-9]*`), optional `.` frac,
    /// optional `[eE][+-]?` exp. Rejects leading `+`, leading zero, hex, `NaN`, `Infinity` (SA8).
    fn skip_number(&mut self) -> Result<(), Denial> {
        if self.peek() == Some(b'-') {
            self.bump();
        }
        match self.bump() {
            Some(b'0') => {} // a leading 0 must not be followed by another digit (checked below)
            Some(c) if (b'1'..=b'9').contains(&c) => {
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    self.bump();
                }
            }
            _ => return Err(Denial::BadNumber),
        }
        // No digit may immediately follow a leading `0`.
        if matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
            return Err(Denial::BadNumber);
        }
        // Optional fraction.
        if self.peek() == Some(b'.') {
            self.bump();
            if !matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                return Err(Denial::BadNumber);
            }
            while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                self.bump();
            }
        }
        // Optional exponent.
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.bump();
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.bump();
            }
            if !matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                return Err(Denial::BadNumber);
            }
            while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                self.bump();
            }
        }
        Ok(())
    }
}
