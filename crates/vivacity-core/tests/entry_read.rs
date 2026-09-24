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

/// Every file of a tree, as `/`-separated relative paths, symlinks excluded
/// (the reader deliberately answers `None` for them, as does a PHP `require`
/// of a path a symlink would have pointed outside the package).
fn files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let p = entry.expect("entry").path();
            let meta = std::fs::symlink_metadata(&p).expect("meta");
            if meta.is_dir() {
                walk(root, &p, out);
            } else if !meta.file_type().is_symlink() {
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

fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for (path, data) in entries {
        if path.ends_with('/') {
            w.add_directory(path.trim_end_matches('/'), opts)
                .expect("dir");
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
