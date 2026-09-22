//! `extract_tar` against the oracle: the same archive extracted by
//! `PharData::extractTo` (what `TarDownloader` calls), then the two trees
//! compared — paths, contents, and on unix the modes. The archives are
//! built here to hold what npm tarballs never have all at once: a directory
//! entry with an odd mode, files with 0666 / 0664 / 0755 / 0600, a symlink
//! and a hard link (PharData writes them as empty files), a pax long name,
//! and a root that is not `package/`.
//!
//! Prerequisites: `php` on the PATH (dev/CI). Missing php FAILS the test.
use std::path::{Path, PathBuf};
use std::process::Command;

fn php() -> PathBuf {
    PathBuf::from("php")
}

struct Spec<'a> {
    path: &'a str,
    kind: tar::EntryType,
    mode: u32,
    data: &'a [u8],
    link: &'a str,
}

fn spec<'a>(
    path: &'a str,
    kind: tar::EntryType,
    mode: u32,
    data: &'a [u8],
    link: &'a str,
) -> Spec<'a> {
    Spec {
        path,
        kind,
        mode,
        data,
        link,
    }
}

fn build_tgz(entries: &[Spec<'_>]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for e in entries {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(e.kind);
        h.set_mode(e.mode);
        h.set_mtime(0);
        h.set_size(e.data.len() as u64);
        match e.kind {
            tar::EntryType::Symlink | tar::EntryType::Link => {
                builder
                    .append_link(&mut h, e.path, e.link)
                    .expect("append link");
            }
            _ => {
                builder
                    .append_data(&mut h, e.path, e.data)
                    .expect("append data");
            }
        }
    }
    let tar_bytes = builder.into_inner().expect("tar");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut enc, &tar_bytes).expect("gzip");
    enc.finish().expect("gzip")
}

/// `(relative path, kind, mode, content)` of every entry under `root`,
/// sorted. Mode 0 on non-unix.
fn inventory(root: &Path) -> Vec<(String, String, u32, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String, u32, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let p = entry.path();
            let meta = std::fs::symlink_metadata(&p).expect("meta");
            let rel = p
                .strip_prefix(root)
                .expect("rel")
                .to_string_lossy()
                .replace('\\', "/");
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt as _;
                meta.permissions().mode() & 0o7777
            };
            #[cfg(not(unix))]
            let mode = 0;
            if meta.is_dir() {
                out.push((rel, "dir".to_owned(), mode, Vec::new()));
                walk(root, &p, out);
            } else if meta.file_type().is_symlink() {
                out.push((rel, "symlink".to_owned(), mode, Vec::new()));
            } else {
                out.push((
                    rel,
                    "file".to_owned(),
                    mode,
                    std::fs::read(&p).expect("read"),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn oracle_extract(tgz: &[u8], work: &Path) -> PathBuf {
    let file = work.join("dist.tgz");
    std::fs::write(&file, tgz).expect("write tgz");
    let out = work.join("oracle");
    let status = Command::new(php())
        .arg("-r")
        .arg("$a = new PharData($argv[1]); $a->extractTo($argv[2], null, true);")
        .arg(&file)
        .arg(&out)
        .status()
        .expect("php runs");
    assert!(status.success(), "PharData failed");
    out
}

/// `ArchiveDownloader::install` after the extraction: the single top-level
/// directory's content moved up. Applied to the oracle tree so both sides
/// compare after the same rule (vivacity applies it inside `extract_tar`).
fn move_single_root_up(out: &Path) {
    let mut top: Vec<PathBuf> = std::fs::read_dir(out)
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.file_name().is_some_and(|n| n != ".DS_Store"))
        .collect();
    if top.len() == 1 && top[0].is_dir() {
        let root = top.remove(0);
        for e in std::fs::read_dir(&root).expect("read_dir") {
            let e = e.expect("entry").path();
            std::fs::rename(&e, out.join(e.file_name().expect("name"))).expect("rename");
        }
        std::fs::remove_dir(&root).expect("rmdir");
    }
}

fn compare(tgz: &[u8]) {
    let work = tempfile::tempdir().expect("tmp");
    let oracle = oracle_extract(tgz, work.path());
    move_single_root_up(&oracle);
    let ours = work.path().join("ours");
    vivacity_core::extract::extract_tar(tgz, &ours).expect("extract_tar");
    let (a, b) = (inventory(&oracle), inventory(&ours));
    assert_eq!(a, b, "tree differs from PharData's");
}

#[test]
fn npm_shaped_archive_with_every_mode() {
    use tar::EntryType as T;
    compare(&build_tgz(&[
        spec("package/package.json", T::Regular, 0o644, b"{}", ""),
        spec("package/index.js", T::Regular, 0o664, b"x", ""),
        spec("package/LICENSE", T::Regular, 0o666, b"mit", ""),
        spec("package/bin/cli.js", T::Regular, 0o755, b"#!", ""),
        spec("package/secret", T::Regular, 0o600, b"s", ""),
        spec(
            "package/lib/deep/dir/file.txt",
            T::Regular,
            0o644,
            b"deep",
            "",
        ),
    ]));
}

#[test]
fn directory_entries_links_and_long_names() {
    use tar::EntryType as T;
    let long = format!("v1.1/{}/file.txt", "l".repeat(120));
    compare(&build_tgz(&[
        spec("v1.1/", T::Directory, 0o700, b"", ""),
        spec("v1.1/sub/", T::Directory, 0o777, b"", ""),
        spec("v1.1/sub/a.txt", T::Regular, 0o640, b"a", ""),
        spec("v1.1/link", T::Symlink, 0o777, b"", "sub/a.txt"),
        spec("v1.1/hard", T::Link, 0o644, b"", "v1.1/sub/a.txt"),
        spec(&long, T::Regular, 0o600, b"long", ""),
    ]));
}

#[test]
fn two_top_level_entries_are_kept_as_is() {
    use tar::EntryType as T;
    compare(&build_tgz(&[
        spec("a/x.txt", T::Regular, 0o644, b"x", ""),
        spec("b.txt", T::Regular, 0o644, b"y", ""),
    ]));
}
