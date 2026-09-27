//! `read_zip_entry` / `read_tar_entry` against the extraction itself: every
//! archive here is extracted by `extract_zip` / `extract_tar` — already
//! compared to PHP's `ZipArchive` and `PharData` by the oracle tests — and
//! then every file of the resulting tree is read back through the entry
//! reader. The invariant is that the reader answers exactly what the
//! extracted tree holds, at the same addresses, so a bundle class can be
//! read out of a dist before it is laid out.
use std::path::{Path, PathBuf};
use vivacity_core::extract::{extract_tar, extract_zip, read_tar_entry, read_zip_entry};

fn work(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vivacity-entry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("work dir");
    d
}

/// Every readable path of a tree, as `/`-separated relative paths, symlinks
/// INCLUDED: a read of the laid-out file follows them, so they belong to the
/// invariant. Excluding them is what let the reader disagree with the
/// extraction on a symlinked class file.
fn files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let p = entry.expect("entry").path();
            let meta = std::fs::symlink_metadata(&p).expect("meta");
            if meta.is_dir() {
                walk(root, &p, out);
            } else {
                out.push(
                    p.strip_prefix(root)
                        .expect("rel")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// Is this filesystem case-insensitive, as macOS's and Windows' default ones
/// are? The reader's case fallback only has to agree with the extracted tree
/// where the tree itself folds case.
fn case_insensitive(dir: &Path) -> bool {
    let probe = dir.join("CaseProbe");
    std::fs::write(&probe, b"x").expect("probe");
    let folded = dir.join("caseprobe").exists();
    std::fs::remove_file(&probe).expect("clean probe");
    folded
}

fn build_tgz(entries: &[TarSpec<'_>]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, kind, data) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(*kind);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_size(data.len() as u64);
        if *kind == tar::EntryType::Symlink {
            builder.append_link(&mut h, path, *data).expect("link");
        } else {
            builder
                .append_data(&mut h, path, data.as_bytes())
                .expect("data");
        }
    }
    let tar_bytes = builder.into_inner().expect("tar");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut enc, &tar_bytes).expect("gzip");
    enc.finish().expect("gzip")
}

/// `path -> data`; a `path` ending in `/` is a directory entry, and a `data`
/// prefixed with `link:` makes a symlink to the rest.
fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for (path, data) in entries {
        if path.ends_with('/') {
            w.add_directory(path.trim_end_matches('/'), opts)
                .expect("dir");
        } else if let Some(target) = data.strip_prefix("link:") {
            w.add_symlink(*path, target, opts).expect("symlink");
        } else {
            w.start_file(*path, opts).expect("file");
            std::io::Write::write_all(&mut w, data.as_bytes()).expect("write");
        }
    }
    w.finish().expect("finish").into_inner()
}

/// `(path, kind, content or link target)` of one tar entry.
type TarSpec<'a> = (&'a str, tar::EntryType, &'a str);

const BUNDLE: &str = "<?php\nnamespace Acme;\nuse Symfony\\Component\\HttpKernel\\Bundle\\Bundle;\nclass AcmeBundle extends Bundle\n{\n}\n";

#[test]
fn zip_entry_reads_what_the_extraction_lays_out() {
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "single-root",
            vec![
                ("acme-bundle-9f8e7d/", ""),
                ("acme-bundle-9f8e7d/composer.json", "{\"name\":\"acme/b\"}"),
                ("acme-bundle-9f8e7d/src/", ""),
                ("acme-bundle-9f8e7d/src/AcmeBundle.php", BUNDLE),
            ],
        ),
        (
            "no-root",
            vec![
                ("composer.json", "{\"name\":\"acme/b\"}"),
                ("src/AcmeBundle.php", BUNDLE),
            ],
        ),
        (
            // Two top-level entries: nothing is stripped, so the addresses
            // keep their first component.
            "two-roots",
            vec![("a/composer.json", "{}"), ("b/src/AcmeBundle.php", BUNDLE)],
        ),
        (
            // `extract_zip` writes a real symlink (mode 0o120777), and a read
            // of it yields the target's bytes — so the reader must follow it.
            "symlink",
            vec![
                ("acme-9f8/", ""),
                ("acme-9f8/src/RealBundle.php", BUNDLE),
                ("acme-9f8/src/AcmeBundle.php", "link:RealBundle.php"),
                // Through a directory and back up with `..`.
                ("acme-9f8/lib/Deep.php", "link:../src/RealBundle.php"),
            ],
        ),
    ];
    for (tag, entries) in cases {
        let bytes = build_zip(&entries);
        let dest = work(&format!("zip-{tag}"));
        extract_zip(&bytes, &dest).expect("extract");
        let laid_out = files(&dest);
        assert!(!laid_out.is_empty(), "{tag}: nothing extracted");
        for rel in &laid_out {
            let expected = std::fs::read(dest.join(rel)).expect("read");
            assert_eq!(
                read_zip_entry(&bytes, rel).expect("read entry"),
                Some(expected),
                "{tag}: {rel} read back differently"
            );
        }
        assert_eq!(
            read_zip_entry(&bytes, "src/Absent.php").expect("absent"),
            None
        );
        // A directory of the extracted tree is not a file to read.
        assert_eq!(read_zip_entry(&bytes, "src").expect("dir"), None);
        std::fs::remove_dir_all(&dest).expect("clean");
    }
}

#[test]
fn tar_entry_reads_what_the_extraction_lays_out() {
    use tar::EntryType::{Directory, Regular, Symlink};
    let cases: Vec<(&str, Vec<TarSpec<'_>>)> = vec![
        (
            "single-root",
            vec![
                ("package/", Directory, ""),
                ("package/composer.json", Regular, "{\"name\":\"acme/b\"}"),
                ("package/src/AcmeBundle.php", Regular, BUNDLE),
            ],
        ),
        (
            "no-root",
            vec![
                ("composer.json", Regular, "{}"),
                ("src/AcmeBundle.php", Regular, BUNDLE),
            ],
        ),
        (
            // PharData writes a symlink as an empty file; the reader
            // answers None rather than that empty content, which is the
            // safe direction for "does this file declare a bundle".
            "symlink",
            vec![
                ("package/", Directory, ""),
                ("package/src/AcmeBundle.php", Regular, BUNDLE),
                ("package/src/Alias.php", Symlink, "AcmeBundle.php"),
            ],
        ),
    ];
    for (tag, entries) in cases {
        let bytes = build_tgz(&entries);
        let dest = work(&format!("tar-{tag}"));
        extract_tar(&bytes, &dest).expect("extract");
        for rel in files(&dest) {
            let expected = std::fs::read(dest.join(&rel)).expect("read");
            let got = read_tar_entry(&bytes, &rel).expect("read entry");
            if tag == "symlink" && rel == "src/Alias.php" {
                // The extraction writes the empty file PharData writes.
                assert!(expected.is_empty(), "{tag}: {rel} should be empty");
                assert_eq!(got, None, "{tag}: a symlink must not answer");
                continue;
            }
            assert_eq!(got, Some(expected), "{tag}: {rel} read back differently");
        }
        assert_eq!(
            read_tar_entry(&bytes, "src/Absent.php").expect("absent"),
            None
        );
        std::fs::remove_dir_all(&dest).expect("clean");
    }
}

#[test]
fn tar_entry_takes_the_last_of_a_repeated_name() {
    use tar::EntryType::{Directory, Regular};
    // A tar stream may carry the same name twice; `extractTo` writes both in
    // order, so the file left on disk holds the LAST one's bytes.
    let bytes = build_tgz(&[
        ("package/", Directory, ""),
        (
            "package/src/AcmeBundle.php",
            Regular,
            "<?php // nothing here",
        ),
        ("package/src/AcmeBundle.php", Regular, BUNDLE),
    ]);
    let dest = work("tar-dup");
    extract_tar(&bytes, &dest).expect("extract");
    let laid_out = std::fs::read(dest.join("src/AcmeBundle.php")).expect("read");
    assert_eq!(
        String::from_utf8_lossy(&laid_out),
        BUNDLE,
        "the extraction keeps the last entry"
    );
    assert_eq!(
        read_tar_entry(&bytes, "src/AcmeBundle.php").expect("read entry"),
        Some(laid_out),
        "the reader must keep the last one too"
    );
    std::fs::remove_dir_all(&dest).expect("clean");
}

#[test]
fn entries_fold_case_like_the_filesystem() {
    // An entry whose case does not match the class name is only readable as
    // the class expects it on a case-insensitive filesystem — macOS's and
    // Windows' default. The reader accepts a case-only difference after an
    // exact match fails: it then agrees with such a tree, and over-answers
    // on a case-sensitive one, which is the safe direction for the caller
    // (a needless hand-over, never a missed bundle).
    let zip_bytes = build_zip(&[("acme-9f8/", ""), ("acme-9f8/src/acmebundle.php", BUNDLE)]);
    let dest = work("zip-case");
    extract_zip(&zip_bytes, &dest).expect("extract");
    let asked = read_zip_entry(&zip_bytes, "src/AcmeBundle.php").expect("case");
    if case_insensitive(&dest) {
        assert_eq!(
            asked,
            Some(std::fs::read(dest.join("src/AcmeBundle.php")).expect("read")),
            "the tree folds case here, so the reader must too"
        );
    } else {
        assert!(asked.is_some(), "over-answering is the safe direction");
    }
    // An exact match still wins over one that only differs in case.
    let both = build_zip(&[
        ("acme-9f8/", ""),
        ("acme-9f8/src/acmebundle.php", "<?php // wrong case"),
        ("acme-9f8/src/AcmeBundle.php", BUNDLE),
    ]);
    assert_eq!(
        read_zip_entry(&both, "src/AcmeBundle.php")
            .expect("exact")
            .map(|b| String::from_utf8_lossy(&b).into_owned()),
        Some(BUNDLE.to_owned())
    );
    std::fs::remove_dir_all(&dest).expect("clean");

    let tgz = build_tgz(&[
        ("package/", tar::EntryType::Directory, ""),
        (
            "package/src/acmebundle.php",
            tar::EntryType::Regular,
            BUNDLE,
        ),
    ]);
    assert!(
        read_tar_entry(&tgz, "src/AcmeBundle.php")
            .expect("case")
            .is_some(),
        "the tar reader folds case the same way"
    );
}
