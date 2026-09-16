// vivacity-pcre2-sys: pcre2-sys 0.2.10's build script with two changes.
//
// 1. PCRE2 is always built from the vendored source (`upstream/`), never
//    taken from the system: the point of this crate is a predictable
//    binary whatever the host has installed.
// 2. Every symbol of that build is prefixed `vivacity_` (PCRE2_SYMBOL_PREFIX,
//    honoured by the patched `upstream/include/pcre2.h` and
//    `upstream/src/pcre2_internal.h`), and the archive is named
//    `libvivacity_pcre2.a`. A program that embeds vivacity and links its
//    own PCRE2 — PHP's libphp.a bundles one, with the same unprefixed
//    `pcre2_*_8` / `_pcre2_*_8` names — no longer gets duplicate symbols.
//
// The rest follows pcre2/NON-AUTOTOOLS-BUILD like upstream: copy a few
// files, set a few defines, build.

use std::path::PathBuf;

fn main() {
    let target = std::env::var("TARGET").unwrap();
    let upstream = PathBuf::from("upstream");

    // Set some config options. We mostly just use the default values. We do
    // this in lieu of patching config.h since it's easier.
    let mut builder = cc::Build::new();
    builder
        .define("PCRE2_CODE_UNIT_WIDTH", "8")
        .define("PCRE2_SYMBOL_PREFIX", "vivacity_")
        .define("HAVE_STDLIB_H", "1")
        .define("HAVE_MEMMOVE", "1")
        .define("HAVE_CONFIG_H", "1")
        .define("PCRE2_STATIC", "1")
        .define("STDC_HEADERS", "1")
        .define("SUPPORT_PCRE2_8", "1")
        .define("SUPPORT_UNICODE", "1");
    if target.contains("windows") {
        builder.define("HAVE_WINDOWS_H", "1");
    }
    enable_jit(&target, &mut builder);

    builder
        .include(upstream.join("src"))
        .include(upstream.join("deps"))
        .include(upstream.join("include"));
    for result in std::fs::read_dir(upstream.join("src")).unwrap() {
        let dent = result.unwrap();
        let path = dent.path();
        if path.extension().map_or(true, |ext| ext != "c") {
            continue;
        }
        // Apparently PCRE2 doesn't want to compile these directly, but only as
        // included from pcre2_jit_compile.c.
        //
        // ... and also pcre2_ucptables.c, which is included by pcre2_tables.c.
        // This is despite NON-AUTOTOOLS-BUILD instructions saying that
        // pcre2_ucptables.c should be compiled directly.
        if path.ends_with("pcre2_jit_match.c")
            || path.ends_with("pcre2_jit_misc.c")
            || path.ends_with("pcre2_ucptables.c")
        {
            continue;
        }
        builder.file(path);
    }

    if std::env::var("PCRE2_SYS_DEBUG").unwrap_or(String::new()) == "1"
        || std::env::var("DEBUG").unwrap_or(String::new()) == "1"
    {
        builder.debug(true);
    }
    builder.compile("vivacity_pcre2");
}

// On `aarch64-apple-ios` clang fails with the following error.
//
//   Undefined symbols for architecture arm64:
//     "___clear_cache", referenced from:
//         _sljit_generate_code in libforeign.a(pcre2_jit_compile.o)
//   ld: symbol(s) not found for architecture arm64
//
// aarch64-apple-tvos         https://bugreports.qt.io/browse/QTBUG-62993?gerritReviewStatus=All
// x86_64-apple-ios           disabled for device–simulator consistency (not tested)
// x86_64-apple-tvos          disabled for device–simulator consistency (not tested)
// armv7-apple-ios            assumed equivalent to aarch64-apple-ios (not tested)
// armv7s-apple-ios           assumed equivalent to aarch64-apple-ios (not tested)
// i386-apple-ios             assumed equivalent to aarch64-apple-ios (not tested)
// x86_64-apple-ios-macabi    disabled out of caution (not tested) (needs attention)
// aarch64-linux-android      does not build
// armv7-linux-androideabi    does not build
// aarch64-unknown-linux-musl does not build
// *-*-*-musleabi*            does not build
//
// We may want to monitor developments on the `aarch64-apple-darwin` front as
// they may end up propagating to all `aarch64`-based targets and the `x86_64`
// equivalents.
fn enable_jit(target: &str, builder: &mut cc::Build) {
    if target == "aarch64-linux-android" {
        return;
    }
    if target == "armv7-linux-androideabi" {
        return;
    }
    if target == "aarch64-unknown-linux-musl" {
        return;
    }
    if target.contains("musleabi") {
        return;
    }
    if target.contains("apple-ios") || target.contains("apple-tvos") {
        return;
    }
    builder.define("SUPPORT_JIT", "1");
}
