//! **The upstream credential, end to end — the descriptor and the value in one place.**
//!
//! Sprint 20 Slice 3 co-located these. They were ~1400 lines apart: the compile-time descriptor
//! ([`CredentialSource`](crate::credential::CredentialSource)) sat in `vocab.rs` with the other validated newtypes, while the runtime value
//! ([`Credential`](crate::credential::Credential)) and its **entire resolution policy** sat inside `upstream.rs`, buried in the module
//! that dials Anthropic. Answering "where does the credential come from, and what happens when the
//! variable is empty" meant finding two places, and the second was not where anyone would look.
//!
//! This is the one secret in the system and `architecture.md` §8 names its *custody* as a configured
//! thing, so it should be readable in one sitting. Nothing here changed in this move except location:
//! same types, same fields, same policy, same privacy.
//!
//! **The two halves and why both are here:**
//! - [`CredentialSource`](crate::credential::CredentialSource) — *where to look*. A compiled-in, structure-validated environment variable
//!   **name**. It carries no value and has no value-carrying constructor, so a literal secret in the
//!   register is inexpressible rather than merely discouraged (bypass audit,
//!   `tests/ui/credential_source_no_literal_bypass.rs`).
//! - [`Credential`](crate::credential::Credential) — *what was found there*. The secret itself, read at startup, with a **redacted**
//!   `Debug` so it cannot reach a log, and a resolution policy that is total over every `env::var`
//!   outcome.
//!
//! The descriptor is compile-time and the value is runtime; keeping them adjacent is what makes the
//! seam between "the operator fixed the lookup location" and "the process read it" legible at a glance.

// ─── DO NOT RENAME `env`/`var_name` FOR CONSISTENCY WITH THE SIBLING NEWTYPES ────────────────────────
//
// `CredentialSource` shares a shape with `vocab.rs`'s `Route`/`RootPath`/`ToolIdentity`: a
// `&'static str` newtype, a `const fn` constructor whose `assert!` validates, a `const fn` accessor.
// Unifying all four behind a macro or generic was **considered and declined** in Sprint 20 Slice 7 —
// the reasoning is recorded in full at the `RULING` block in `vocab.rs`, above `Route`.
//
// **This type's divergent naming is load-bearing to that decision, not cosmetic.** The ruling turns on
// the U5 bypass audit staying verifiable by *reading*, and part of that argument is that `env` (not
// `new`) and `var_name` (not `as_str`) say what this type is: it names a **location**, never a value,
// and its accessor returns something **safe to log**. "There is no value-carrying constructor" reads
// materially weaker if the constructor is called `new`.
//
// So renaming these to match the siblings would not be a tidy-up — it would delete one of the reasons
// the four were left separate, silently, while making them *look* more unifiable than they are. If you
// are here to make the four consistent, read the `RULING` block first; it names what would legitimately
// reopen the decision, and "the names now match" is not it.

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
/// It is deliberately **not** the same security weight as [`Route`](crate::vocab::Route), which fixes an effect boundary —
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
    /// **The name `env` is load-bearing — do not rename it to `new` for consistency with
    /// `vocab.rs`'s sibling newtypes.** It does argumentative work in Sprint 20 Slice 7's `RULING`
    /// (see `vocab.rs`, above `Route`): "there is no value-carrying constructor" reads materially
    /// weaker if the constructor is called `new`. Renaming removes a premise that ruling depends on.
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
    ///
    /// **The name `var_name` is load-bearing — do not rename it to `as_str`** for consistency with
    /// `vocab.rs`'s siblings. It says the accessor returns a *name*, which is what makes "safe to log"
    /// legible at the call site; `as_str` says nothing. Same Sprint 20 Slice 7 `RULING` as `env`.
    #[must_use]
    pub const fn var_name(&self) -> &'static str {
        self.0
    }
}

/// The upstream credential (the Anthropic API key) — the concrete form of Sprint 8's
/// [`Authority::Credential`](crate::lifecycle::Authority::Credential). Trusted-side only. Its `Debug` is
/// **redacted** so it can never reach a log or diagnostic; nothing echoes it to the box.
pub struct Credential(String);

impl Credential {
    #[must_use]
    pub fn new(key: String) -> Self {
        Credential(key)
    }

    /// Resolve the credential from its compiled [`CredentialSource`] descriptor (Sprint 16) — read the
    /// named environment variable at startup. The descriptor fixes *where* to look; this reads it.
    /// **Empty is absent** (see [`Credential::from_env_value`]).
    #[must_use]
    pub fn from_source(source: &CredentialSource) -> Option<Credential> {
        Self::from_env_value(std::env::var(source.var_name()))
    }

    /// The resolution **policy**, pure and total over every `env::var` outcome, so it is directly
    /// testable without mutating the process environment (which under edition 2024 would require
    /// `unsafe` — forbidden by the crate's `deny(unsafe_code)`).
    ///
    /// **Set-but-empty is treated exactly as unset**: an empty string is not a credential, and letting
    /// one satisfy `Authority::Credential`'s presence check would be a silently-open authority rung —
    /// the opposite of the fail-closed guarantee the rung exists to make. Non-UTF-8 is likewise absent:
    /// a value the proxy cannot even read is not a credential it can present.
    fn from_env_value(value: Result<String, std::env::VarError>) -> Option<Credential> {
        match value {
            Ok(v) if !v.is_empty() => Some(Credential(v)),
            // Unset, set-but-empty, or non-UTF-8 → absent. Fail closed, identically in all three cases.
            _ => None,
        }
    }

    /// **The secret itself — the one accessor that exposes it, for the one caller that must.**
    ///
    /// Added by Sprint 20 Slice 3's split, and it is the ONLY thing that move could not carry over
    /// verbatim. While `Credential` lived inside `upstream.rs`, the dial read the private field
    /// directly (`self.credential.0`) — legal because they shared a module, and invisible for exactly
    /// the same reason. Co-locating the credential moved the dial out of that privilege, so the read
    /// had to become explicit.
    ///
    /// **This IS a widening, and the bound is what justifies it — not its absence.** Stated precisely
    /// because the imprecise version ("no widening occurred") would be false: the secret's reachable
    /// surface went from *module-private* to *crate-visible*. What makes that acceptable is that the
    /// widening is minimal (`pub(crate)`, never `pub`), forced (a private field cannot cross a module
    /// boundary, and the split had to happen), and **bounded** (one caller, named below).
    ///
    /// The field itself never widens, and `expose` is the single place in the crate where the secret
    /// leaves this type — so "who reads the credential" went from answerable-only-by-reading-`upstream.rs`
    /// to answerable by grepping one method name.
    ///
    /// **STANDING CHECK, not a one-time approval.** Its one legitimate caller writes the `x-api-key`
    /// header. Anything else calling this is a finding — the credential must never reach a box-facing
    /// frame, a log, or a diagnostic. Any slice touching this module or `upstream_dial` re-confirms
    /// **exactly one caller** before closing, the same way this project re-reviews a provisional scalar
    /// rather than trusting that it was settled once.
    #[must_use]
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// Whether this credential is the empty string — the one property of the secret that is safe to
    /// expose (it reveals no content). Lets the authority rung fail closed on an empty credential
    /// however it was constructed, not only on one resolved through [`Credential::from_env_value`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl core::fmt::Debug for Credential {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
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

#[cfg(test)]
mod tests {
    //! The credential's own tests, co-located with it (Sprint 20 Slice 3). Two of these previously
    //! lived in `upstream.rs` and one in `vocab.rs` — the same ~1400-line split the module fixes,
    //! reproduced in the test layout. `from_env_value` is private, so its test can only live here
    //! (a `tests/` integration test links the lib without `--test` and cannot see it).

    use super::*;
    use crate::register::net::ROUTES;

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

    /// **THE EMPTY-ENV-VAR GATE**: a set-but-empty variable resolves to *absent*, **identically** to
    /// unset — and the `Authority::Credential` rung then fails closed on it. Driven through the pure
    /// policy so it is deterministic and needs no process-environment mutation (which under edition 2024
    /// would require `unsafe`, forbidden by the crate's `deny(unsafe_code)`).
    #[test]
    fn set_but_empty_env_var_is_absent_exactly_like_unset() {
        use crate::lifecycle::{Authority, validate_authority};
        use std::env::VarError;

        // The three absent cases — set-but-empty, unset, and unreadable — are one outcome: None.
        let empty = Credential::from_env_value(Ok(String::new()));
        let unset = Credential::from_env_value(Err(VarError::NotPresent));
        let non_utf8 = Credential::from_env_value(Err(VarError::NotUnicode("\u{fffd}".into())));
        assert!(empty.is_none(), "a set-but-empty variable is NOT a credential");
        assert!(unset.is_none());
        assert!(non_utf8.is_none(), "a value the proxy cannot read is not a credential");

        // A real value resolves — so the gate is not vacuously-closed (the success arm).
        let present = Credential::from_env_value(Ok("sk-test-value".into()));
        assert!(present.is_some(), "a non-empty value DOES resolve");

        // Chained to the authority rung: empty fails closed EXACTLY as unset does, and only a real
        // credential validates. This is the property the rung exists to guarantee.
        assert!(
            !validate_authority(Authority::Credential, ROUTES, empty.as_ref(), None),
            "an empty credential must FAIL the authority rung, not satisfy it by mere presence",
        );
        assert!(!validate_authority(Authority::Credential, ROUTES, unset.as_ref(), None));
        assert!(validate_authority(Authority::Credential, ROUTES, present.as_ref(), None));

        // The rung's own second floor: an empty credential constructed DIRECTLY (not through the
        // resolution policy) also fails — the guarantee does not depend on how it was built.
        let hand_built_empty = Credential::new(String::new());
        assert!(
            !validate_authority(Authority::Credential, ROUTES, Some(&hand_built_empty), None),
            "empty fails closed however constructed",
        );
    }

    /// The credential's `Debug` is redacted — it can never reach a log or diagnostic.
    #[test]
    fn credential_debug_is_redacted() {
        let cred = Credential::new("sk-super-secret".into());
        assert_eq!(format!("{cred:?}"), "Credential(<redacted>)");
        assert!(!format!("{cred:?}").contains("secret"), "the key never appears in Debug output");
    }
}
