// Bypass audit — THE load-bearing negative proof for the credential descriptor: a literal secret must
// be INEXPRESSIBLE, not merely discouraged. `CredentialSource` carries a variable *name* in a private
// field and has no value-carrying constructor, so the only way to smuggle a secret in would be a struct
// literal — which does not compile. `CredentialSource::env` is the ONLY construction path, and it takes
// a name that is read from the environment at runtime, never a value compiled into source.
fn main() {
    let _bypass = proxy::vocab::CredentialSource("sk-ant-a-literal-secret-value");
    let _ = _bypass;
}
