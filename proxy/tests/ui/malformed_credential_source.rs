// A malformed env-var name as a const must fail to compile — the C1 compile-time gate applied to the
// credential SOURCE. (Dashes are a plausible typo: `ANTHROPIC-API-KEY` cannot name a real variable, so
// it would silently never resolve at runtime; the const gate turns that into a build failure.)
fn main() {
    const _BAD: proxy::vocab::CredentialSource =
        proxy::vocab::CredentialSource::env("ANTHROPIC-API-KEY");
    let _ = _BAD;
}
