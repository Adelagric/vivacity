//! The Composer phar the PHP oracles run, shared by every `tests/oracle_*.rs`
//! through `#[path]` (they live in three crates).
//!
//! It is the `composer` found on the PATH, copied under a `.phar` name — the
//! `phar://` stream wants the extension. The copy used to be made ONCE, to a
//! fixed name in the temp directory, and reused forever: after an upgrade of
//! Composer every oracle went on testing the previous one, without a word. The
//! copy is now named after the source's identity (canonical path, size,
//! mtime), so a different Composer gets a different copy. Found on 2026-10-09
//! while running the suite against a 2.11-dev snapshot, which needed a
//! separate `TMPDIR` to be tested at all.

#[allow(dead_code)]
pub fn composer_phar() -> std::path::PathBuf {
    let out = std::process::Command::new("which")
        .arg("composer")
        .output()
        .expect("`which composer` runs");
    let found = String::from_utf8(out.stdout).expect("utf8");
    let found = found.trim();
    assert!(
        !found.is_empty(),
        "composer must be installed and on the PATH: the oracles run it"
    );
    // Through any symlink (Homebrew's `bin/composer` points into a versioned
    // keg): an upgrade moves the link, and the identity must follow it.
    let src = std::fs::canonicalize(found).expect("canonicalize composer");
    let meta = std::fs::metadata(&src).expect("stat composer");
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src.to_string_lossy().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let target = std::env::temp_dir().join(format!(
        "vivacity-oracle-composer-{h:016x}-{}-{mtime}.phar",
        meta.len()
    ));
    if !target.exists() {
        // Through a temporary name and a rename: test binaries run in
        // parallel and must never read a half-written phar.
        let tmp = target.with_extension(format!("phar.{}.tmp", std::process::id()));
        std::fs::copy(&src, &tmp).expect("copy the composer phar");
        if std::fs::rename(&tmp, &target).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    target
}
