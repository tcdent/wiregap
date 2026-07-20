// A malformed route (wrong scheme) as a const must fail to compile — the C1 compile-time gate.
fn main() {
    const _BAD: proxy::vocab::Route = proxy::vocab::Route::new("http://evil.example/x");
    let _ = _BAD;
}
