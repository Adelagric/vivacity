//! Linked together with `tests/foreign_pcre2.c` (see `build.rs`): this
//! binary holds an unprefixed `pcre2_compile_8` / `_pcre2_check_escape_8` /
//! `pcre2_match_8` next to vivacity's PCRE2. The test only links when the
//! latter's symbols are prefixed, and only passes when the match runs on
//! the real engine rather than the dummies.

#[test]
fn vivacity_pcre2_links_next_to_a_foreign_pcre2() {
    let re = pcre2::bytes::RegexBuilder::new()
        .build(r"(?<![\$:>])class\s++(?P<name>[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*+)")
        .expect("compile on the prefixed engine");
    let caps = re
        .captures(b"final class Foo\\Bar {}")
        .expect("match")
        .expect("a match");
    assert_eq!(&caps["name"], b"Foo");
    assert!(re.is_match(b"class X").unwrap());
    assert!(!re.is_match(b"$class X").unwrap());
}
