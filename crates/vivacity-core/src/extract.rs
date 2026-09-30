//! Extraction of a zip dist into a directory, stripping the single root
//! directory of GitHub/Packagist zipballs (ArchiveDownloader's rule: strip iff
//! the archive has exactly one top-level entry and it is a directory;
//! otherwise everything is extracted as is), and a DISTRUSTFUL extraction:
//! - paths: `enclosed_name()` (rejects `..` and absolute paths);
//! - symlinks (unix mode S_IFLNK): relative target only, and the lexically
//!   resolved path must stay inside the package root;
//! - executable bits preserved (binaries depend on them);
//! - refusal of absurd decompressed sizes (crude zip bomb).

use crate::error::{Error, Result};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Maximum decompressed size of a dist (512 MB); beyond that, something is
/// wrong (the largest real packages weigh a few tens of MB).
const MAX_UNCOMPRESSED: u64 = 512 * 1024 * 1024;

const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;

/// Path of an entry, or an error if it is absolute or contains `..`:
/// enclosed_name accepts an inner `..` (`r/../x` stays inside the root) that
/// stripping the first component would turn into an escape, and no
/// legitimate dist contains one.
fn entry_path(entry: &zip::read::ZipFile<'_>, dest: &Path) -> Result<PathBuf> {
    entry
        .enclosed_name()
        .filter(|p| {
            p.components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        })
        .ok_or_else(|| Error::HostileArchive {
            dest: dest.to_path_buf(),
            reason: format!("invalid entry path: {:?}", entry.name()),
        })
}

/// Number of leading components to strip from each entry: 1 if the archive
/// has exactly one top-level entry and it is a directory
/// (`ArchiveDownloader::install`, `$singleDirAtTopLevel`; a top-level
/// `.DS_Store` is not counted, and then disappears along with the root
/// directory), 0 otherwise, in which case everything is moved, `.DS_Store`
/// included (`rename($temporaryDir, $path)` onto an empty target).
fn root_strip(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, dest: &Path) -> Result<usize> {
    let mut entries: Vec<(PathBuf, bool)> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(Error::zip(dest))?;
        entries.push((entry_path(&entry, dest)?, entry.is_dir()));
    }
    Ok(root_strip_of(
        entries.iter().map(|(p, d)| (p.as_path(), *d)),
    ))
}

/// The rule itself, over `(entry path, is a directory entry)`.
fn root_strip_of<'a>(entries: impl Iterator<Item = (&'a Path, bool)>) -> usize {
    let mut top: std::collections::BTreeMap<std::ffi::OsString, bool> =
        std::collections::BTreeMap::new();
    for (raw, dir_entry) in entries {
        let mut comps = raw
            .components()
            .filter(|c| matches!(c, Component::Normal(_)));
        let Some(Component::Normal(first)) = comps.next() else {
            continue;
        };
        let is_dir = dir_entry || comps.next().is_some();
        if !is_dir && first == ".DS_Store" {
            continue;
        }
        *top.entry(first.to_owned()).or_insert(false) |= is_dir;
    }
    usize::from(top.len() == 1 && top.values().all(|d| *d))
}

/// Memoized `create_dir_all`: avoids one `create_dir_all` (hence one stat +
/// N syscalls) per zip entry when hundreds of files share the same parent
/// directory. Inserts the path and its newly created ancestors.
fn ensure_dir(p: &Path, made: &mut std::collections::HashSet<PathBuf>) -> Result<()> {
    if made.contains(p) {
        return Ok(());
    }
    std::fs::create_dir_all(p).map_err(Error::io(p))?;
    let mut cur = Some(p);
    while let Some(c) = cur {
        if !made.insert(c.to_path_buf()) {
            break; // ancestor already known: so is the rest
        }
        cur = c.parent();
    }
    Ok(())
}

pub fn extract_zip(zip_bytes: &[u8], dest: &Path) -> Result<()> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(Error::zip(dest))?;
    let mut made: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    ensure_dir(dest, &mut made)?;
    let strip = root_strip(&mut archive, dest)?;
    // Symlinks this archive has created, by stripped relative path. Nothing may
    // be written THROUGH one: `check_symlink_target` resolves a target
    // lexically, which assumes every component of the path is a real
    // directory — and an archive can break that assumption with two entries.
    // `a -> .` passes (it stays inside), then `a/b -> ..` passes too (its
    // lexical parent is `a`, so `a/..` is the root), yet on disk `a` is the
    // root, so `a/b` points OUTSIDE and a file written under it escapes.
    // `unzip`, which is what `ZipDownloader::extract` runs, refuses exactly
    // this: "checkdir error: … exists but is not directory".
    let mut symlinked: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(Error::zip(dest))?;
        total = total.saturating_add(entry.size());
        if total > MAX_UNCOMPRESSED {
            return Err(Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: format!("uncompressed size > {MAX_UNCOMPRESSED} bytes"),
            });
        }
        let raw = entry_path(&entry, dest)?;
        let stripped: PathBuf = raw
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .skip(strip)
            .collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        if let Some(link) = symlinked
            .iter()
            .find(|s| stripped.starts_with(s) && stripped.as_path() != s.as_path())
        {
            return Err(Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: format!(
                    "entry {} would be written through the symlink {}",
                    stripped.display(),
                    link.display()
                ),
            });
        }
        let out = dest.join(&stripped);

        let mode = entry.unix_mode();
        let is_symlink = mode.is_some_and(|m| m & S_IFMT == S_IFLNK);

        if entry.is_dir() {
            ensure_dir(&out, &mut made)?;
        } else if is_symlink {
            let mut target = String::new();
            entry.read_to_string(&mut target).map_err(Error::io(&out))?;
            check_symlink_target(&stripped, &target, dest, &symlinked)?;
            if let Some(p) = out.parent() {
                ensure_dir(p, &mut made)?;
            }
            let _ = std::fs::remove_file(&out);
            symlinked.insert(stripped.clone());
            #[cfg(unix)]
            crate::clone::symlink_like_unzip(std::path::Path::new(&target), &out)?;
            // Windows: Composer extracts with `unzip` or `7z` when one is on
            // the PATH (`ZipDownloader::$unzipCommands`), which recreate the
            // link when the process may create links (the GitHub runner);
            // with PHP's ZipArchive the entry becomes an ordinary file whose
            // content is the target. Same rule here: a link when such a tool
            // is present and the link can be made, the file otherwise.
            #[cfg(windows)]
            {
                let tool_present = std::env::var_os("PATH").is_some_and(|path| {
                    std::env::split_paths(&path)
                        .any(|dir| dir.join("unzip.exe").exists() || dir.join("7z.exe").exists())
                });
                let linked =
                    tool_present && std::os::windows::fs::symlink_file(&target, &out).is_ok();
                if !linked {
                    std::fs::write(&out, target.as_bytes()).map_err(Error::io(&out))?;
                }
            }
            #[cfg(not(any(unix, windows)))]
            std::fs::write(&out, target.as_bytes()).map_err(Error::io(&out))?;
        } else {
            if let Some(p) = out.parent() {
                ensure_dir(p, &mut made)?;
            }
            let mut buf = Vec::with_capacity(entry.size().min(MAX_UNCOMPRESSED) as usize);
            entry.read_to_end(&mut buf).map_err(Error::io(&out))?;
            std::fs::write(&out, &buf).map_err(Error::io(&out))?;
            #[cfg(unix)]
            if let Some(m) = mode {
                if m & 0o111 != 0 {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))
                        .map_err(Error::io(&out))?;
                }
            }
        }
    }
    Ok(())
}

/// Extraction of a `tar` dist (asset-packagist: npm's `.tgz`), as
/// `TarDownloader` does it through `PharData::extractTo` — measured on
/// PHP 8.5 (docs/plans/v0.17-tar-dist.md):
/// - a file gets the exact mode of its header (no umask);
/// - a directory entry's mode is ignored (0777 & ~umask, like the
///   implicit parents — npm archives have no directory entries at all);
/// - a symlink or hard-link entry becomes an EMPTY regular file with the
///   header mode (PharData never creates links: nothing to check);
/// - any other entry type (device, fifo) is refused.
///
/// Then `ArchiveDownloader`'s single-root rule, shared with zip. Paths
/// with `..` or absolute are refused (PharData normalises them lexically;
/// no real archive has one).
pub fn extract_tar(tgz_bytes: &[u8], dest: &Path) -> Result<()> {
    use std::io::Read as _;
    let hostile = |reason: String| Error::HostileArchive {
        dest: dest.to_path_buf(),
        reason,
    };
    let path_of = |bytes: &[u8]| -> Result<PathBuf> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| hostile("entry path is not UTF-8".to_owned()))?;
        let p = Path::new(text);
        if p.is_absolute()
            || !p
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(hostile(format!("invalid entry path: {text:?}")));
        }
        Ok(p.to_path_buf())
    };
    // Whole archive read once: the single-root rule needs every path
    // before the first write, and a tar stream is not seekable.
    struct Entry {
        path: PathBuf,
        kind: tar::EntryType,
        #[cfg_attr(not(unix), allow(dead_code))]
        mode: u32,
        data: Vec<u8>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut total: u64 = 0;
    let decoder = flate2::read::GzDecoder::new(tgz_bytes);
    let mut archive = tar::Archive::new(decoder);
    let iter = archive.entries().map_err(Error::io(dest))?;
    for entry in iter {
        let mut entry = entry.map_err(Error::io(dest))?;
        let kind = entry.header().entry_type();
        let path = {
            let raw = entry.path_bytes();
            path_of(&raw)?
        };
        let mode = entry.header().mode().map_err(Error::io(dest))? & 0o7777;
        let size = entry.header().size().map_err(Error::io(dest))?;
        total = total.saturating_add(size);
        if total > MAX_UNCOMPRESSED {
            return Err(hostile(format!(
                "uncompressed size > {MAX_UNCOMPRESSED} bytes"
            )));
        }
        let data = match kind {
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                let mut buf = Vec::with_capacity(size.min(MAX_UNCOMPRESSED) as usize);
                entry.read_to_end(&mut buf).map_err(Error::io(dest))?;
                buf
            }
            tar::EntryType::Directory | tar::EntryType::Symlink | tar::EntryType::Link => {
                Vec::new()
            }
            // pax / GNU long-name headers are consumed by the crate; what
            // reaches here with another type is not a file tree.
            other => {
                return Err(hostile(format!(
                    "unsupported entry type {other:?} for {}",
                    path.display()
                )))
            }
        };
        entries.push(Entry {
            path,
            kind,
            mode,
            data,
        });
    }
    let strip = root_strip_of(
        entries
            .iter()
            .map(|e| (e.path.as_path(), e.kind == tar::EntryType::Directory)),
    );
    let mut made: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    ensure_dir(dest, &mut made)?;
    for e in &entries {
        let stripped: PathBuf = e
            .path
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .skip(strip)
            .collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&stripped);
        if e.kind == tar::EntryType::Directory {
            ensure_dir(&out, &mut made)?;
            continue;
        }
        if let Some(p) = out.parent() {
            ensure_dir(p, &mut made)?;
        }
        std::fs::write(&out, &e.data).map_err(Error::io(&out))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(e.mode))
                .map_err(Error::io(&out))?;
        }
    }
    Ok(())
}

/// A symlink target must be relative and stay lexically inside the extracted
/// root (the classic attack: `link -> ../../../../etc/passwd`).
fn check_symlink_target(
    link_rel: &Path,
    target: &str,
    dest: &Path,
    symlinked: &std::collections::HashSet<PathBuf>,
) -> Result<()> {
    let hostile = |reason: String| Error::HostileArchive {
        dest: dest.to_path_buf(),
        reason,
    };
    let target_path = Path::new(target);
    if target_path.is_absolute() {
        return Err(hostile(format!(
            "absolute symlink: {link_rel:?} -> {target}"
        )));
    }
    // Resolved lexically from the link's own directory, keeping the components
    // and not just a depth: they are what says whether the target crosses a
    // symlink this archive created, whose real place on disk is elsewhere.
    let mut parts: Vec<std::ffi::OsString> = link_rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_owned()),
            _ => None,
        })
        .collect();
    parts.pop(); // the link's own name: what remains is its directory
    let comps: Vec<Component<'_>> = target_path.components().collect();
    for (i, c) in comps.iter().enumerate() {
        match c {
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(hostile(format!(
                        "symlink escaping the archive: {link_rel:?} -> {target}"
                    )));
                }
            }
            Component::Normal(n) => {
                parts.push((*n).to_owned());
                // Crossing a symlink this archive created is what makes the
                // lexical resolution a lie; pointing AT one is harmless, so
                // only a component with something after it counts.
                if i + 1 < comps.len() {
                    let crossed: PathBuf = parts.iter().collect();
                    if symlinked.contains(&crossed) {
                        return Err(hostile(format!(
                            "symlink {link_rel:?} -> {target} passes through the symlink {}",
                            crossed.display()
                        )));
                    }
                }
            }
            Component::CurDir => {}
            _ => {
                return Err(hostile(format!(
                    "invalid symlink: {link_rel:?} -> {target}"
                )))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tar_tests {
    use super::*;

    fn tgz(entries: &[(&str, tar::EntryType, u32, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, kind, mode, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(*kind);
            h.set_mode(*mode);
            h.set_size(data.len() as u64);
            h.set_mtime(0);
            match kind {
                tar::EntryType::Symlink | tar::EntryType::Link => {
                    b.append_link(&mut h, path, "target").expect("link")
                }
                // The builder refuses `..` and absolute paths: raw header
                // for the hostile cases.
                _ if path.contains("..") || path.starts_with('/') => {
                    let gnu = h.as_gnu_mut().expect("gnu");
                    gnu.name[..path.len()].copy_from_slice(path.as_bytes());
                    h.set_cksum();
                    b.append(&h, *data).expect("raw")
                }
                _ => b.append_data(&mut h, path, *data).expect("data"),
            }
        }
        let raw = b.into_inner().expect("tar");
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut e, &raw).expect("gz");
        e.finish().expect("gz")
    }

    #[test]
    fn strips_single_root_keeps_modes_and_flattens_links() {
        use tar::EntryType as T;
        let d = tempfile::tempdir().expect("tmp");
        let out = d.path().join("out");
        extract_tar(
            &tgz(&[
                ("package/a.txt", T::Regular, 0o664, b"a"),
                ("package/bin/x", T::Regular, 0o755, b"#!"),
                ("package/dir/", T::Directory, 0o700, b""),
                ("package/link", T::Symlink, 0o777, b""),
            ]),
            &out,
        )
        .expect("extract");
        assert_eq!(std::fs::read(out.join("a.txt")).expect("a"), b"a");
        assert!(out.join("dir").is_dir());
        let link = std::fs::symlink_metadata(out.join("link")).expect("link");
        assert!(
            link.is_file() && link.len() == 0,
            "a link becomes an empty file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: &str| {
                std::fs::metadata(out.join(p))
                    .expect(p)
                    .permissions()
                    .mode()
                    & 0o7777
            };
            assert_eq!(mode("a.txt"), 0o664);
            assert_eq!(mode("bin/x"), 0o755);
            assert_eq!(mode("link"), 0o777);
            assert_ne!(mode("dir"), 0o700, "directory modes are not applied");
        }
    }

    #[test]
    fn refuses_escapes_and_devices() {
        use tar::EntryType as T;
        let d = tempfile::tempdir().expect("tmp");
        for (path, kind) in [
            ("package/../escape", T::Regular),
            ("/abs", T::Regular),
            ("package/dev", T::Char),
        ] {
            let r = extract_tar(&tgz(&[(path, kind, 0o644, b"x")]), &d.path().join("o"));
            assert!(
                matches!(r, Err(Error::HostileArchive { .. })),
                "{path} should be refused: {r:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn build_zip(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content, mode) in entries {
            let mut opts = SimpleFileOptions::default();
            if let Some(m) = mode {
                opts = opts.unix_permissions(*m);
            }
            if name.ends_with('/') {
                w.add_directory(name.trim_end_matches('/'), opts)
                    .expect("dir");
            } else {
                w.start_file(*name, opts).expect("start");
                w.write_all(content).expect("write");
            }
        }
        w.finish().expect("finish").into_inner()
    }

    fn tmpdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tmpdir")
    }

    /// Entries in order: a link when `link` is true (`add_symlink`, what a
    /// zipball carries), an ordinary file otherwise.
    fn build_mixed(entries: &[(&str, &str, bool)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, payload, link) in entries {
            if *link {
                w.add_symlink(*name, *payload, SimpleFileOptions::default())
                    .expect("symlink");
            } else {
                w.start_file(*name, SimpleFileOptions::default())
                    .expect("start");
                w.write_all(payload.as_bytes()).expect("write");
            }
        }
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn a_chain_of_symlinks_cannot_escape() {
        // Reported by a third-party review of v0.18.0, reproduced: `a -> .`
        // stays inside, and `a/b -> ..` resolves to the root LEXICALLY (its
        // parent being `a`), so both passed the target check — yet on disk `a`
        // IS the root, so `a/b` points outside and a file written under it
        // escaped the extraction directory. `unzip`, which
        // `ZipDownloader::extract` runs, refuses the same thing ("exists but is
        // not directory"), so nothing of Composer's behaviour is given up.
        let d = tmpdir();
        let dest = d.path().join("pkg");
        let zip = build_mixed(&[
            ("pkg/a", ".", true),
            ("pkg/a/b", "..", true),
            ("pkg/a/b/pwned.txt", "ESCAPED", false),
        ]);
        let err = extract_zip(&zip, &dest).expect_err("must be refused");
        assert!(
            format!("{err}").contains("through the symlink"),
            "unexpected error: {err}"
        );
        assert!(
            !d.path().join("pwned.txt").exists(),
            "a file was written outside the extraction directory"
        );
    }

    #[test]
    fn a_symlink_target_may_not_cross_a_symlink_of_the_archive() {
        // The same family, one step earlier: nothing is written through `a`
        // here, but `z`'s target crosses it, so the link left on disk would
        // point outside once followed.
        let d = tmpdir();
        let dest = d.path().join("pkg");
        let zip = build_mixed(&[("pkg/a", ".", true), ("pkg/z", "a/../evil", true)]);
        let err = extract_zip(&zip, &dest).expect_err("must be refused");
        assert!(
            format!("{err}").contains("passes through the symlink"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_plain_symlink_inside_the_package_still_works() {
        // The guard must not cost the ordinary case: a link next to its target.
        let d = tmpdir();
        let dest = d.path().join("pkg");
        let zip = build_mixed(&[
            ("pkg/src/Real.php", "<?php // real", false),
            ("pkg/src/Alias.php", "Real.php", true),
        ]);
        extract_zip(&zip, &dest).expect("a legitimate link is extracted");
        let alias = dest.join("src/Alias.php");
        let meta = std::fs::symlink_metadata(&alias).expect("alias");
        if meta.file_type().is_symlink() {
            assert_eq!(
                std::fs::read_to_string(&alias).expect("read through the link"),
                "<?php // real"
            );
        } else {
            // Windows without unzip/7z: an ordinary file holding the target.
            assert_eq!(
                std::fs::read_to_string(&alias).expect("read the file"),
                "Real.php"
            );
        }
    }

    #[test]
    fn strips_root_and_preserves_exec_bit() {
        let zip = build_zip(&[
            ("root-abc/", b"", None),
            ("root-abc/src/a.php", b"<?php", None),
            (
                "root-abc/bin/tool",
                b"#!/usr/bin/env php\n<?php",
                Some(0o100755),
            ),
        ]);
        let d = tmpdir();
        extract_zip(&zip, d.path()).expect("extract");
        assert!(d.path().join("src/a.php").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(d.path().join("bin/tool"))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit lost");
        }
    }

    #[test]
    fn strips_only_a_single_top_level_directory() {
        // Single directory without an explicit directory entry: strip.
        let single = build_zip(&[("pkg/a.txt", b"a", None), ("pkg/sub/b.txt", b"b", None)]);
        let d = tmpdir();
        extract_zip(&single, d.path()).expect("extract");
        assert!(d.path().join("a.txt").is_file() && d.path().join("sub/b.txt").is_file());

        // File at the root + directory: nothing is stripped, nothing is lost.
        let mixed = build_zip(&[("README", b"r", None), ("src/a.php", b"<?php", None)]);
        let d = tmpdir();
        extract_zip(&mixed, d.path()).expect("extract");
        assert!(d.path().join("README").is_file(), "root file lost");
        assert!(
            d.path().join("src/a.php").is_file(),
            "directory wrongly flattened"
        );

        // Two top-level directories: nothing is stripped.
        let two = build_zip(&[("a/x", b"x", None), ("b/y", b"y", None)]);
        let d = tmpdir();
        extract_zip(&two, d.path()).expect("extract");
        assert!(d.path().join("a/x").is_file() && d.path().join("b/y").is_file());

        // A single file at the root: not a directory, no strip.
        let file = build_zip(&[("only.txt", b"o", None)]);
        let d = tmpdir();
        extract_zip(&file, d.path()).expect("extract");
        assert!(d.path().join("only.txt").is_file(), "single file lost");

        // Top-level .DS_Store: ignored for the count (the single directory is
        // stripped, it disappears); without a single directory, extracted like
        // the rest.
        let ds = build_zip(&[(".DS_Store", b"junk", None), ("pkg/a.txt", b"a", None)]);
        let d = tmpdir();
        extract_zip(&ds, d.path()).expect("extract");
        assert!(d.path().join("a.txt").is_file());
        assert!(!d.path().join(".DS_Store").exists());
        let ds2 = build_zip(&[(".DS_Store", b"junk", None), ("a.txt", b"a", None)]);
        let d = tmpdir();
        extract_zip(&ds2, d.path()).expect("extract");
        assert!(d.path().join("a.txt").is_file() && d.path().join(".DS_Store").is_file());
    }

    fn build_zip_with_symlink(link_name: &str, target: &str) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.add_directory("r", SimpleFileOptions::default())
            .expect("dir");
        w.start_file("r/real.txt", SimpleFileOptions::default())
            .expect("start");
        w.write_all(b"x").expect("write");
        w.add_symlink(link_name, target, SimpleFileOptions::default())
            .expect("symlink");
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn valid_symlink_is_recreated_hostile_ones_rejected() {
        let ok = build_zip_with_symlink("r/sub/link", "../real.txt");
        let d = tmpdir();
        extract_zip(&ok, d.path()).expect("extract");
        let meta = d.path().join("sub/link").symlink_metadata().expect("meta");
        #[cfg(unix)]
        assert!(meta.file_type().is_symlink());
        #[cfg(not(unix))]
        {
            // A link when unzip/7z is on the PATH and links may be created
            // (the GitHub runner), else — like PHP ZipArchive — an ordinary
            // file containing the target.
            if meta.file_type().is_symlink() {
                assert_eq!(
                    std::fs::read_link(d.path().join("sub/link")).expect("target"),
                    std::path::PathBuf::from("../real.txt")
                );
            } else {
                assert!(meta.file_type().is_file());
                assert_eq!(
                    std::fs::read(d.path().join("sub/link")).expect("read"),
                    b"../real.txt"
                );
            }
        }

        for target in ["../../etc/passwd", "/etc/passwd", "../../../x"] {
            let bad = build_zip_with_symlink("r/link", target);
            let d = tmpdir();
            assert!(
                extract_zip(&bad, d.path()).is_err(),
                "hostile symlink accepted: {target}"
            );
        }
    }

    #[test]
    fn zip_slip_paths_are_rejected() {
        // The writer sanitises names: we forge the `..` by byte-patching
        // (same length), as a crafted archive would.
        let benign = build_zip(&[("r/", b"", None), ("r/AA/evil.txt", b"x", None)]);
        let patched: Vec<u8> = {
            let needle = b"r/AA/evil.txt";
            let replacement = b"r/../evil.txt";
            let mut bytes = benign.clone();
            let mut i = 0;
            while i + needle.len() <= bytes.len() {
                if &bytes[i..i + needle.len()] == needle {
                    bytes[i..i + needle.len()].copy_from_slice(replacement);
                }
                i += 1;
            }
            bytes
        };
        assert_ne!(benign, patched, "the patch replaced nothing");
        let d = tmpdir();
        assert!(
            extract_zip(&patched, d.path()).is_err(),
            "zip-slip accepted"
        );
    }

    #[test]
    fn bzip2_entry_decompresses_through_the_pure_rust_backend() {
        // The bzip2 backend is libbz2-rs-sys (workspace Cargo.toml) so an
        // embedder's link stays free of BZ2_* symbols; no fixture carries a
        // bzip2-compressed entry, so the decoder is exercised here.
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Bzip2);
        w.start_file("pkg/src/a.php", opts).expect("start");
        let body = "<?php // enough bytes for a real bzip2 block\n".repeat(200);
        w.write_all(body.as_bytes()).expect("write");
        let bytes = w.finish().expect("finish").into_inner();

        let d = tmpdir();
        extract_zip(&bytes, d.path()).expect("extract");
        let out = std::fs::read_to_string(d.path().join("src/a.php")).expect("read");
        assert_eq!(out, body);
    }
}

/// The bytes of one file of a zip dist, addressed as the extracted tree
/// addresses it: the same single-root strip, then a `/`-separated relative
/// path, and then the same semantics a read of the laid-out file has.
///
/// That last part is the whole difficulty, and three things follow from it:
/// the extraction writes every entry in order, so a name appearing twice
/// ends up holding the LAST one's bytes; `extract_zip` turns a symlink entry
/// into a real symlink, so reading it yields its target's bytes; and on a
/// case-insensitive filesystem — macOS and Windows — a read of
/// `src/AcmeBundle.php` finds an entry named `src/acmebundle.php`. A match
/// that only differs in case is accepted after an exact one fails: on a
/// case-sensitive host that can only over-answer, and the caller's
/// over-answer is a needless hand-over.
///
/// `None` when the archive holds no such file: a directory entry, a missing
/// name, or a symlink whose target is absent or escapes the package (an
/// archive `extract_zip` would refuse outright).
pub fn read_zip_entry(zip_bytes: &[u8], rel: &str) -> Result<Option<Vec<u8>>> {
    let dest = Path::new("<archive>");
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(Error::zip(dest))?;
    let mut entries: Vec<(PathBuf, bool, u32)> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(Error::zip(dest))?;
        entries.push((
            entry_path(&entry, dest)?,
            entry.is_dir(),
            entry.unix_mode().unwrap_or(0),
        ));
    }
    let strip = root_strip_of(entries.iter().map(|(p, d, _)| (p.as_path(), *d)));
    // Later entries win, as the extraction's writes do.
    let mut by_path: Vec<(Vec<std::ffi::OsString>, usize, bool)> = Vec::new();
    for (i, (path, dir, mode)) in entries.iter().enumerate() {
        if *dir {
            continue;
        }
        let Some(comps) = stripped(path, strip) else {
            continue;
        };
        let link = mode & S_IFMT == S_IFLNK;
        if let Some(slot) = by_path.iter_mut().find(|(c, _, _)| *c == comps) {
            *slot = (comps, i, link);
        } else {
            by_path.push((comps, i, link));
        }
    }
    let find = |wanted: &[std::ffi::OsString]| -> Option<(usize, bool)> {
        by_path
            .iter()
            .find(|(c, _, _)| c == wanted)
            .or_else(|| {
                by_path
                    .iter()
                    .find(|(c, _, _)| same_ignoring_case(c, wanted))
            })
            .map(|(_, i, link)| (*i, *link))
    };
    let mut wanted = components(Path::new(rel));
    // `ELOOP`: a symlink chain the filesystem would refuse to follow.
    for _ in 0..8 {
        let Some((index, link)) = find(&wanted) else {
            return Ok(None);
        };
        let bytes = read_zip_index(&mut archive, index, dest, rel)?;
        if !link {
            return Ok(Some(bytes));
        }
        let Ok(target) = String::from_utf8(bytes) else {
            return Ok(None);
        };
        let Some(next) = resolve_link(&wanted, &target) else {
            return Ok(None);
        };
        wanted = next;
    }
    Ok(None)
}

fn read_zip_index(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    index: usize,
    dest: &Path,
    rel: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive.by_index(index).map_err(Error::zip(dest))?;
    if entry.size() > MAX_UNCOMPRESSED {
        return Err(Error::HostileArchive {
            dest: dest.to_path_buf(),
            reason: format!("entry {rel} is absurdly large"),
        });
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).map_err(Error::io(dest))?;
    Ok(buf)
}

/// A symlink's target, resolved lexically against the link's own path, as
/// the filesystem resolves it. `None` when it leaves the package root, which
/// `check_symlink_target` refuses at extraction time, or when it is absolute.
fn resolve_link(link: &[std::ffi::OsString], target: &str) -> Option<Vec<std::ffi::OsString>> {
    let target = Path::new(target);
    if target.is_absolute() {
        return None;
    }
    let mut out: Vec<std::ffi::OsString> = link[..link.len().saturating_sub(1)].to_vec();
    for c in target.components() {
        match c {
            Component::Normal(n) => out.push(n.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop()?;
            }
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The same for a tar dist. The stream is not seekable and the strip needs
/// every path, so the archive is read once, keeping every candidate — the
/// last one wins, as the extraction's writes do. A symlink is NOT followed
/// here: `PharData::extractTo`, which `TarDownloader` uses, writes a symlink
/// entry as an empty file, and an empty file answers like `None` to the one
/// question this serves.
pub fn read_tar_entry(tgz_bytes: &[u8], rel: &str) -> Result<Option<Vec<u8>>> {
    let dest = Path::new("<archive>");
    let wanted = components(Path::new(rel));
    let mut paths: Vec<(PathBuf, bool)> = Vec::new();
    let mut found: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let decoder = flate2::read::GzDecoder::new(tgz_bytes);
    let mut archive = tar::Archive::new(decoder);
    let mut total: u64 = 0;
    for entry in archive.entries().map_err(Error::io(dest))? {
        let mut entry = entry.map_err(Error::io(dest))?;
        let kind = entry.header().entry_type();
        let path = {
            let raw = entry.path_bytes();
            let text = std::str::from_utf8(&raw).map_err(|_| Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: "entry path is not UTF-8".to_owned(),
            })?;
            let p = Path::new(text);
            if p.is_absolute()
                || !p
                    .components()
                    .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
            {
                return Err(Error::HostileArchive {
                    dest: dest.to_path_buf(),
                    reason: format!("invalid entry path: {text:?}"),
                });
            }
            p.to_path_buf()
        };
        paths.push((path.clone(), kind.is_dir()));
        if !kind.is_file() {
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNCOMPRESSED {
            return Err(Error::HostileArchive {
                dest: dest.to_path_buf(),
                reason: "decompressed size beyond the limit".to_owned(),
            });
        }
        // The strip is only known at the end, so every name that could be
        // the one — stripped or not, whatever its case — is kept.
        if matches_stripped(&path, 0, &wanted) || matches_stripped(&path, 1, &wanted) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf).map_err(Error::io(dest))?;
            found.push((path, buf));
        }
    }
    let strip = root_strip_of(paths.iter().map(|(p, d)| (p.as_path(), *d)));
    // Last match wins; an exact one wins over one that only differs in case.
    let pick = |exact: bool| {
        found.iter().rev().find(|(p, _)| {
            stripped(p, strip).is_some_and(|c| {
                if exact {
                    c == wanted
                } else {
                    same_ignoring_case(&c, &wanted)
                }
            })
        })
    };
    Ok(pick(true).or_else(|| pick(false)).map(|(_, b)| b.clone()))
}

/// An entry path's components, its first `strip` of them dropped; `None`
/// when it has no more than that (the stripped root itself).
fn stripped(path: &Path, strip: usize) -> Option<Vec<std::ffi::OsString>> {
    let got = components(path);
    (got.len() > strip).then(|| got[strip..].to_vec())
}

fn components(p: &Path) -> Vec<std::ffi::OsString> {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_owned()),
            _ => None,
        })
        .collect()
}

/// The same path but for ASCII case, which is what a case-insensitive
/// filesystem folds in every name a PHP class file can have.
fn same_ignoring_case(a: &[std::ffi::OsString], b: &[std::ffi::OsString]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| match (x.to_str(), y.to_str()) {
                (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
                _ => x == y,
            })
}

fn matches_stripped(path: &Path, strip: usize, wanted: &[std::ffi::OsString]) -> bool {
    stripped(path, strip).is_some_and(|c| c == wanted || same_ignoring_case(&c, wanted))
}
