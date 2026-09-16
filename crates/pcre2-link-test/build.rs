// Compiles `tests/foreign_pcre2.c` — a stand-in for PHP's bundled PCRE2:
// the unprefixed `pcre2_compile_8`, `_pcre2_check_escape_8` and
// `pcre2_match_8` — and links the object flat into this crate's test
// binaries (`rustc-link-arg-tests`: a static archive member nothing
// references would never be pulled, and the test would prove nothing).
// With vivacity-pcre2-sys's prefix the link succeeds and the regex in
// `tests/link.rs` runs on the real engine; without it, GNU ld and lld
// report duplicate symbols (the failure ePHPm hit) and Apple's ld lets the
// dummies shadow the engine — which the test's match assertions catch.
// Verified both ways on 2026-09-17.
fn main() {
    println!("cargo:rerun-if-changed=tests/foreign_pcre2.c");
    let objects = cc::Build::new()
        .file("tests/foreign_pcre2.c")
        .compile_intermediates();
    for obj in objects {
        println!("cargo:rustc-link-arg-tests={}", obj.display());
    }
}
