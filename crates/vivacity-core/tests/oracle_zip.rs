//! `extract_zip` against the oracle: the same archive extracted by
//! `unzip -qq <file> -d <dir>`, which is the command
//! `ZipDownloader::extractWithSystemUnzip` builds (phar 2.10.3, line 51 —
//! no `-o`), then the two trees compared: paths, kinds, contents, and on
//! unix the modes.
//!
//! Scope: archives `unzip` extracts with exit code 0. An archive that makes
//! it exit non-zero sends Composer through a second extraction with
//! `ZipArchive` on top of the partial tree (lines 146-179), which is a
//! two-stage behaviour settled by measurement in
//! docs/plans/v0.20-hostile-archives.md, not by this oracle.
//!
//! Prerequisites: `unzip` on the PATH (dev/CI). Missing unzip FAILS the
//! test — never a silent skip.
use std::path::{Path, PathBuf};
use std::process::Command;

enum Kind<'a> {
    File(&'a [u8]),
    Dir,
    Symlink(&'a str),
}

struct Spec<'a> {
    path: &'a str,
    kind: Kind<'a>,
    /// Permission bits stored in the entry (the file-type bits are added
    /// here), or `None` to leave `external_attributes` at zero.
    mode: Option<u32>,
}

fn file<'a>(path: &'a str, data: &'a [u8], mode: u32) -> Spec<'a> {
    Spec {
        path,
        kind: Kind::File(data),
        mode: Some(mode),
    }
}

fn dir<'a>(path: &'a str, mode: u32) -> Spec<'a> {
    Spec {
        path,
        kind: Kind::Dir,
        mode: Some(mode),
    }
}

fn link<'a>(path: &'a str, target: &'a str) -> Spec<'a> {
    Spec {
        path,
        kind: Kind::Symlink(target),
        mode: Some(0o777),
    }
}

fn build_zip(entries: &[Spec<'_>]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for e in entries {
        let opts = SimpleFileOptions::default();
        match &e.kind {
            Kind::File(data) => {
                let opts = match e.mode {
                    Some(m) => opts.unix_permissions(m),
                    None => opts,
                };
                w.start_file(e.path, opts).expect("start_file");
                std::io::Write::write_all(&mut w, data).expect("write");
            }
            Kind::Dir => {
                let opts = match e.mode {
                    Some(m) => opts.unix_permissions(m),
                    None => opts,
                };
                w.add_directory(e.path, opts).expect("add_directory");
            }
            Kind::Symlink(target) => {
                w.add_symlink(e.path, *target, opts.unix_permissions(0o777))
                    .expect("add_symlink");
            }
        }
    }
    w.finish().expect("finish").into_inner()
}

/// Rewrites the `external attributes` of one central-directory record,
/// found by its name: the only way to build an entry whose host is DOS, or
/// whose attributes are zero, since the writer always stores a unix mode.
/// Record layout (APPNOTE 4.3.12): signature, then the name length at +28,
/// the extra length at +30, the comment length at +32, the external
/// attributes at +38 and the name at +46.
fn set_central_attrs(zip: &mut [u8], name: &str, attrs: u32) {
    let mut i = 0;
    let mut patched = 0;
    while let Some(pos) = zip[i..]
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .map(|p| i + p)
    {
        let name_len = u16::from_le_bytes([zip[pos + 28], zip[pos + 29]]) as usize;
        let entry_name = &zip[pos + 46..pos + 46 + name_len];
        if entry_name == name.as_bytes() {
            zip[pos + 38..pos + 42].copy_from_slice(&attrs.to_le_bytes());
            patched += 1;
        }
        i = pos + 4;
    }
    assert_eq!(patched, 1, "one record named {name} expected");
}

/// Same, for the host byte of `version made by` (+4, high byte): 3 is unix,
/// 0 is DOS/FAT. The writer always says unix.
fn set_central_host(zip: &mut [u8], name: &str, host: u8) {
    let mut i = 0;
    let mut patched = 0;
    while let Some(pos) = zip[i..]
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .map(|p| i + p)
    {
        let name_len = u16::from_le_bytes([zip[pos + 28], zip[pos + 29]]) as usize;
        if &zip[pos + 46..pos + 46 + name_len] == name.as_bytes() {
            zip[pos + 5] = host;
            patched += 1;
        }
        i = pos + 4;
    }
    assert_eq!(patched, 1, "one record named {name} expected");
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
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&p).expect("readlink");
                out.push((
                    rel,
                    "symlink".to_owned(),
                    mode,
                    target.to_string_lossy().into_owned().into_bytes(),
                ));
            } else if meta.is_dir() {
                out.push((rel, "dir".to_owned(), mode, Vec::new()));
                walk(root, &p, out);
            } else {
                // A mode the archive chose can leave the file unreadable
                // (`unzip` obeys it); the mode is what is compared, so the
                // content is read after a chmod when it has to be.
                let data = match std::fs::read(&p) {
                    Ok(d) => d,
                    Err(_) => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt as _;
                            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))
                                .expect("chmod to read");
                        }
                        std::fs::read(&p).expect("read after chmod")
                    }
                };
                out.push((rel, "file".to_owned(), mode, data));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn oracle_extract(zip: &[u8], work: &Path) -> PathBuf {
    let file = work.join("dist.zip");
    std::fs::write(&file, zip).expect("write zip");
    let out = work.join("oracle");
    std::fs::create_dir_all(&out).expect("mkdir");
    let status = Command::new("unzip")
        .arg("-qq")
        .arg(&file)
        .arg("-d")
        .arg(&out)
        .status()
        .expect("unzip runs (install it: it is what Composer calls)");
    assert!(status.success(), "unzip failed: {status:?}");
    out
}

/// `ArchiveDownloader::install` after the extraction: the content of the
/// single top-level directory moved up. `is_dir()` follows links, as PHP's
/// does. Applied to the oracle tree so both sides compare after the same
/// rule (vivacity applies it inside `extract_zip`).
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

fn compare(zip: &[u8]) {
    let work = tempfile::tempdir().expect("tmp");
    let oracle = oracle_extract(zip, work.path());
    move_single_root_up(&oracle);
    let ours = work.path().join("ours");
    vivacity_core::extract::extract_zip(zip, &ours).expect("extract_zip");
    let (a, b) = (inventory(&oracle), inventory(&ours));
    assert_eq!(a, b, "tree differs from unzip's");
}

#[test]
fn every_file_mode_comes_from_the_archive() {
    compare(&build_zip(&[
        file("pkg/p600.txt", b"a", 0o600),
        file("pkg/p640.txt", b"b", 0o640),
        file("pkg/p644.txt", b"c", 0o644),
        file("pkg/p664.txt", b"d", 0o664),
        file("pkg/p666.txt", b"e", 0o666),
        file("pkg/p444.txt", b"f", 0o444),
        file("pkg/p700.sh", b"#!", 0o700),
        file("pkg/p755.sh", b"#!", 0o755),
        file("pkg/p775.sh", b"#!", 0o775),
    ]));
}

#[test]
fn the_special_bits_are_dropped_as_unzip_drops_them() {
    compare(&build_zip(&[
        file("pkg/setuid.sh", b"#!", 0o4755),
        file("pkg/setgid.sh", b"#!", 0o2755),
        file("pkg/sticky.sh", b"#!", 0o1755),
        file("pkg/plain.txt", b"x", 0o644),
    ]));
}

#[test]
fn directory_entries_carry_their_own_mode() {
    compare(&build_zip(&[
        dir("pkg/", 0o755),
        dir("pkg/d700/", 0o700),
        dir("pkg/d777/", 0o777),
        file("pkg/d700/a.txt", b"a", 0o644),
        file("pkg/d777/b.txt", b"b", 0o600),
    ]));
}

#[test]
fn a_symlink_keeps_its_target_and_its_neighbour_its_mode() {
    compare(&build_zip(&[
        file("pkg/real.txt", b"real", 0o600),
        link("pkg/alias.txt", "real.txt"),
        file("pkg/sub/deep.txt", b"deep", 0o640),
        link("pkg/sub/up.txt", "../real.txt"),
    ]));
}

/// Discovery-and-assertion: an entry with no unix mode at all (external
/// attributes zero, as a DOS host leaves them) and an entry with the DOS
/// read-only bit. `unzip` has a rule of its own there, and it is not the
/// one the `zip` crate synthesises.
#[test]
fn an_entry_without_a_unix_mode_follows_unzips_own_rule() {
    let mut zip = build_zip(&[
        file("pkg/plain.txt", b"a", 0o644),
        file("pkg/noattr.txt", b"b", 0o644),
        file("pkg/readonly.txt", b"c", 0o644),
    ]);
    set_central_attrs(&mut zip, "pkg/noattr.txt", 0);
    set_central_attrs(&mut zip, "pkg/readonly.txt", 0x01);
    compare(&zip);
}

/// A DOS/FAT host says nothing about unix permissions, and `unzip` has its
/// own answer there — not the one the `zip` crate synthesises (0664 / 0775).
#[test]
fn a_dos_host_entry_follows_unzips_own_rule() {
    let mut zip = build_zip(&[
        file("pkg/unix.txt", b"a", 0o644),
        file("pkg/dos.txt", b"b", 0o644),
        file("pkg/dosro.txt", b"c", 0o644),
        dir("pkg/dosdir/", 0o755),
    ]);
    for (name, attrs) in [
        ("pkg/dos.txt", 0u32),
        ("pkg/dosro.txt", 0x01),
        ("pkg/dosdir/", 0x10),
    ] {
        set_central_host(&mut zip, name, 0);
        set_central_attrs(&mut zip, name, attrs);
    }
    compare(&zip);
}
