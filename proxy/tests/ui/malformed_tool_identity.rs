// A malformed tool identity (contains a space) as a const must fail to compile.
fn main() {
    const _BAD: proxy::vocab::ToolIdentity = proxy::vocab::ToolIdentity::new("has space");
    let _ = _BAD;
}
