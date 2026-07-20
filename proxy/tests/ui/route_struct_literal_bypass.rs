// Bypass audit: constructing Route via a struct literal (skipping the validating `new`) must fail —
// the interior field is private, so `Route::new` is the ONLY construction path.
fn main() {
    let _bypass = proxy::vocab::Route("http://not validated");
    let _ = _bypass;
}
