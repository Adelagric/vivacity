# vivacity-pcre2

[pcre2](https://github.com/BurntSushi/rust-pcre2) 0.2.11, **unchanged**,
depending on `vivacity-pcre2-sys` instead of `pcre2-sys`: the same
high-level API (`pcre2::bytes::Regex`, …) on a PCRE2 built from source
with every symbol prefixed `vivacity_`, so that a program embedding
[vivacity](https://github.com/Adelagric/vivacity) can also link a PCRE2
of its own (PHP's `libphp.a` bundles one). See `vivacity-pcre2-sys` for
the why and the diff.

The only edit is `Cargo.toml` (`pcre2-sys = { package = "vivacity-pcre2-sys" }`,
`[lib] name = "pcre2"` so the sources and doctests compile as they are).

Andrew Gallant's, `Unlicense OR MIT` (`COPYING`, `LICENSE-MIT`, `UNLICENSE`).
