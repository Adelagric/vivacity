//! Local content-addressed store: each (package, version, dist reference) is
//! extracted ONCE into `<cache>/store/<layout>/<vendor>/<pkg>/<key>/`, then
//! cloned into the projects' vendor/ (see clone.rs). Atomic write: extraction
//! into a staging directory then `rename`; the final directory only exists
//! complete, and two concurrent processes converge (the loser of the rename
//! discards its staging directory).
//!
//! The staging directory is `<cache>/store/.staging/`, deliberately OUTSIDE the
//! served layout: an entry is served from `<layout>/…` and nothing else is ever
//! read, so whatever an extraction writes beside its own directory — the
//! extractor refuses that now, this is the belt to its braces — cannot become a
//! package. It is also on the same filesystem as the layout, which a `rename`
//! requires.

use crate::error::{Error, Result};
use crate::extract::{extract_tar, extract_zip};

/// The archive format of a dist in the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistKind {
    Zip,
    Tar,
}
use std::path::PathBuf;

pub struct Store {
    root: PathBuf,
}

/// Segment naming the extraction semantics an entry was written with. A store
/// entry is a *tree*, not an archive, so it carries whatever rule the build
/// that wrote it applied — and when that rule changes (`v2`: the modes come
/// from the archive instead of "executable → 0755, default otherwise"; `v3`:
/// the root of the entry takes the mode of the archive's top-level directory),
/// an old entry keeps serving the old tree forever, silently. Bumping this
/// segment is what makes the change take effect; the previous trees stay on
/// disk, unused, until the cache is cleared.
///
/// `v4`: up to 0.26.0 vendor/ was laid out by hard links on file systems
/// without copy-on-write, so a file written in place in a project was written
/// in its store entry too. No entry written before the change can be trusted
/// to hold its archive's bytes: none is served again, every entry is
/// extracted afresh from its archive.
const LAYOUT: &str = "v4";

/// Beside the layouts: one empty file per vendor directory whose files laid
/// out by an older version were given inodes of their own
/// (`installer::detach_once`).
const DETACHED: &str = ".detached";

/// Beside the markers, not under them (deleting `.detached/` to run the
/// pass again keeps them): one directory per vendor directory, one JSON file
/// per pass that found linked files (`detach_report`).
const DETACH_REPORTS: &str = ".detached-reports";

/// Beside the markers: one file per vendor directory naming the packages
/// whose copy there does not come from their entry ([`Store::foreign_copies`]).
const FOREIGN: &str = ".foreign";

/// Where extractions happen, swept of orphans on the way in. Six hours is well
/// past any install, and short enough that a killed process does not leave a
/// tree there for a month: a parallel install's staging directory is minutes
/// old, never hours.
const STAGING: &str = ".staging";
const ORPHAN_AFTER: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

impl Store {
    pub fn at(root: PathBuf) -> Store {
        Store { root }
    }

    /// Removes staging directories no live process can own any more. Silent on
    /// every error: this is housekeeping, not part of the install's contract,
    /// and another process may be deleting the same entry.
    fn sweep_staging(&self) {
        let staging = self.root.join(STAGING);
        let Ok(entries) = std::fs::read_dir(&staging) else {
            return;
        };
        let now = std::time::SystemTime::now();
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .and_then(|m| now.duration_since(m).map_err(std::io::Error::other))
                .is_ok_and(|age| age > ORPHAN_AFTER);
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    pub fn default_location() -> Store {
        Store::at(crate::platform::cache_dir().join("store"))
    }

    /// Entry key: version + first 12 hex chars of the dist reference,
    /// sanitised for the file system.
    pub fn entry_path(&self, name: &str, version: &str, dist_ref: Option<&str>) -> PathBuf {
        let sane = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect()
        };
        let short_ref = dist_ref.unwrap_or("noref");
        let short_ref = &short_ref[..short_ref.len().min(12)];
        self.root
            .join(LAYOUT)
            .join(name) // vendor/pkg: two safe segments (validated by the lock)
            .join(format!("{}-{}", sane(version), sane(short_ref)))
    }

    /// Ensures the entry is present, extracting it if needed.
    /// Returns the path of the extracted tree.
    pub fn ensure(
        &self,
        name: &str,
        version: &str,
        dist_ref: Option<&str>,
        dist_bytes: &[u8],
        kind: DistKind,
    ) -> Result<PathBuf> {
        self.ensure_made(name, version, dist_ref, dist_bytes, kind)
            .map(|(path, _)| path)
    }

    /// [`Store::ensure`], and whether THIS call extracted the entry from
    /// `dist_bytes` (false: it was there already, or a concurrent process
    /// renamed its own first).
    pub fn ensure_made(
        &self,
        name: &str,
        version: &str,
        dist_ref: Option<&str>,
        dist_bytes: &[u8],
        kind: DistKind,
    ) -> Result<(PathBuf, bool)> {
        let final_path = self.entry_path(name, version, dist_ref);
        if final_path.is_dir() {
            return Ok((final_path, false));
        }
        let parent = final_path.parent().unwrap_or(&self.root).to_path_buf();
        std::fs::create_dir_all(&parent).map_err(Error::io(&parent))?;
        let tmp = self.extract_staged(name, version, dist_bytes, kind)?;
        let tmp_path = tmp.keep();
        match std::fs::rename(&tmp_path, &final_path) {
            Ok(()) => Ok((final_path, true)),
            Err(_) if final_path.is_dir() => {
                // A concurrent process won the rename: its entry is complete.
                let _ = std::fs::remove_dir_all(&tmp_path);
                Ok((final_path, false))
            }
            Err(source) => {
                let _ = std::fs::remove_dir_all(&tmp_path);
                Err(Error::Io {
                    path: final_path,
                    source,
                })
            }
        }
    }

    /// An extraction of `dist_bytes` in the staging directory, outside the
    /// served layout; removed when the returned directory is dropped.
    pub fn extract_staged(
        &self,
        name: &str,
        version: &str,
        dist_bytes: &[u8],
        kind: DistKind,
    ) -> Result<tempfile::TempDir> {
        let staging = self.root.join(STAGING);
        std::fs::create_dir_all(&staging).map_err(Error::io(&staging))?;
        self.sweep_staging();
        let tmp = tempfile::Builder::new()
            .prefix(".tmp-")
            .tempdir_in(&staging)
            .map_err(Error::io(&staging))?;
        let extracted = match kind {
            DistKind::Zip => extract_zip(dist_bytes, tmp.path()),
            DistKind::Tar => extract_tar(dist_bytes, tmp.path()),
        };
        // A refusal names the package, not `.tmp-EX4VNV` under the store.
        extracted.map_err(|e| match e {
            Error::HostileArchive { reason, .. } => Error::HostileDist {
                name: name.to_owned(),
                version: version.to_owned(),
                reason,
            },
            other => other,
        })?;
        Ok(tmp)
    }

    pub fn contains(&self, name: &str, version: &str, dist_ref: Option<&str>) -> bool {
        self.entry_path(name, version, dist_ref).is_dir()
    }

    /// The marker saying `vendor`'s files no longer share an inode with an
    /// older layout's entries: keyed by the canonical path of the directory
    /// and by this layout (a later bump runs the pass again).
    pub fn detached_marker(&self, vendor: &std::path::Path) -> PathBuf {
        self.root
            .join(DETACHED)
            .join(LAYOUT)
            .join(vendor_key(vendor))
    }

    /// Where the reports of `vendor`'s one-time pass go
    /// (`detach_report`), one file per pass that found linked files.
    pub fn detach_reports(&self, vendor: &std::path::Path) -> PathBuf {
        self.root
            .join(DETACH_REPORTS)
            .join(LAYOUT)
            .join(vendor_key(vendor))
    }

    /// The packages whose copy under `vendor` was laid out from an archive
    /// Composer downloaded and whose tree differs from their entry (point 55,
    /// DECISIONS 2026-10-31): the entry stays as it is — other projects hold
    /// copies of it, and the classmap cache keys its records on its path —
    /// and the dump must not serve the entry's records for that copy. Empty
    /// when the file is absent (the common case: one failed `open`).
    pub fn foreign_copies(&self, vendor: &std::path::Path) -> std::collections::BTreeSet<String> {
        std::fs::read_to_string(self.foreign_file(vendor))
            .map(|text| {
                text.lines()
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Records [`Store::foreign_copies`] for `vendor` (temp+rename; the file
    /// goes when the set is empty).
    pub fn set_foreign_copies(
        &self,
        vendor: &std::path::Path,
        names: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        let file = self.foreign_file(vendor);
        if names.is_empty() {
            return match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::Io {
                    path: file,
                    source: e,
                }),
                _ => Ok(()),
            };
        }
        let dir = file.parent().unwrap_or(&self.root).to_path_buf();
        std::fs::create_dir_all(&dir).map_err(Error::io(&dir))?;
        let mut text = String::new();
        for n in names {
            text.push_str(n);
            text.push('\n');
        }
        let tmp = tempfile::NamedTempFile::new_in(&dir).map_err(Error::io(&dir))?;
        std::fs::write(tmp.path(), text).map_err(Error::io(tmp.path()))?;
        tmp.persist(&file).map_err(|e| Error::Io {
            path: file.clone(),
            source: e.error,
        })?;
        Ok(())
    }

    fn foreign_file(&self, vendor: &std::path::Path) -> PathBuf {
        self.root
            .join(FOREIGN)
            .join(LAYOUT)
            .join(vendor_key(vendor))
    }
}

/// Whether two extracted trees would lay out the same copy: the same
/// relative paths, of the same kinds, the same bytes in every file, the same
/// targets for the links, the same permission bits (Unix) for files and
/// directories, the roots included. Read in full: only the desynchronized
/// path asks (an archive Composer downloaded against the entry already in
/// the store).
pub fn same_tree(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn kind_and_mode(m: &std::fs::Metadata) -> (u8, u32) {
        let kind = if m.file_type().is_symlink() {
            0
        } else if m.is_dir() {
            1
        } else {
            2
        };
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt as _;
            m.permissions().mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = u32::from(m.permissions().readonly());
        (kind, mode)
    }
    fn walk(a: &std::path::Path, b: &std::path::Path) -> std::io::Result<bool> {
        let (ma, mb) = (std::fs::symlink_metadata(a)?, std::fs::symlink_metadata(b)?);
        if kind_and_mode(&ma) != kind_and_mode(&mb) {
            return Ok(false);
        }
        if ma.file_type().is_symlink() {
            return Ok(std::fs::read_link(a)? == std::fs::read_link(b)?);
        }
        if !ma.is_dir() {
            return Ok(ma.len() == mb.len() && std::fs::read(a)? == std::fs::read(b)?);
        }
        let names = |dir: &std::path::Path| -> std::io::Result<Vec<std::ffi::OsString>> {
            let mut v: Vec<_> = std::fs::read_dir(dir)?
                .map(|e| e.map(|e| e.file_name()))
                .collect::<std::io::Result<_>>()?;
            v.sort();
            Ok(v)
        };
        let (na, nb) = (names(a)?, names(b)?);
        if na != nb {
            return Ok(false);
        }
        for n in &na {
            if !walk(&a.join(n), &b.join(n))? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    walk(a, b).unwrap_or(false)
}

/// The sha1 of the canonical path of a vendor directory.
fn vendor_key(vendor: &std::path::Path) -> String {
    use sha1::Digest as _;
    let canonical = crate::pathutil::canonicalize(vendor).unwrap_or_else(|_| vendor.to_path_buf());
    hex::encode(sha1::Sha1::digest(canonical.to_string_lossy().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn sample_zip() -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.add_directory("root-x", SimpleFileOptions::default())
            .expect("dir");
        w.start_file("root-x/composer.json", SimpleFileOptions::default())
            .expect("f");
        w.write_all(b"{}").expect("w");
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn same_tree_compares_what_a_copy_would_hold() {
        let dir = tempfile::tempdir().expect("tmp");
        let make = |name: &str, body: &[u8]| {
            let root = dir.path().join(name);
            std::fs::create_dir_all(root.join("src/empty")).expect("mkdir");
            std::fs::write(root.join("src/A.php"), body).expect("write");
            root
        };
        let a = make("a", b"<?php class A {}");
        let b = make("b", b"<?php class A {}");
        assert!(same_tree(&a, &b));
        let c = make("c", b"<?php class B {}");
        assert!(!same_tree(&a, &c), "other bytes");
        let d = make("d", b"<?php class A {}");
        std::fs::write(d.join("extra.txt"), b"x").expect("write");
        assert!(!same_tree(&a, &d), "one more file");
        let e = make("e", b"<?php class A {}");
        std::fs::remove_dir(e.join("src/empty")).expect("rmdir");
        assert!(!same_tree(&a, &e), "an empty directory less");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let f = make("f", b"<?php class A {}");
            std::fs::set_permissions(f.join("src/A.php"), std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
            assert!(!same_tree(&a, &f), "another mode");
            let g = make("g", b"<?php class A {}");
            let h = make("h", b"<?php class A {}");
            std::os::unix::fs::symlink("src/A.php", g.join("l")).expect("link");
            std::os::unix::fs::symlink("src", h.join("l")).expect("link");
            assert!(!same_tree(&g, &h), "another link target");
        }
        assert!(!same_tree(&a, &dir.path().join("absent")));
    }

    #[test]
    fn foreign_copies_are_recorded_per_vendor() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = Store::at(dir.path().join("store"));
        let vendor = dir.path().join("p/vendor");
        std::fs::create_dir_all(&vendor).expect("mkdir");
        assert!(store.foreign_copies(&vendor).is_empty());
        let names: std::collections::BTreeSet<String> =
            ["a/b".to_owned(), "c/d".to_owned()].into_iter().collect();
        store.set_foreign_copies(&vendor, &names).expect("record");
        assert_eq!(store.foreign_copies(&vendor), names);
        let other = dir.path().join("q/vendor");
        std::fs::create_dir_all(&other).expect("mkdir");
        assert!(store.foreign_copies(&other).is_empty(), "another project");
        store
            .set_foreign_copies(&vendor, &std::collections::BTreeSet::new())
            .expect("cleared");
        assert!(store.foreign_copies(&vendor).is_empty());
    }

    #[test]
    fn ensure_is_idempotent_and_atomic() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = Store::at(dir.path().join("store"));
        let zip = sample_zip();
        let p1 = store
            .ensure(
                "a/b",
                "1.0.0",
                Some("deadbeefcafe1234"),
                &zip,
                DistKind::Zip,
            )
            .expect("ensure");
        assert!(p1.join("composer.json").is_file());
        assert!(store.contains("a/b", "1.0.0", Some("deadbeefcafe1234")));
        // Second call: same bytes or not, the existing entry wins.
        let p2 = store
            .ensure(
                "a/b",
                "1.0.0",
                Some("deadbeefcafe1234"),
                b"garbage",
                DistKind::Zip,
            )
            .expect("hit");
        assert_eq!(p1, p2);
        // Different key -> other entry.
        assert!(!store.contains("a/b", "1.0.0", Some("feedfacefeed5678")));
        // The extraction happens OUTSIDE the served layout, and the staging
        // directory is left empty behind it: nothing that an extraction writes
        // beside its own tree can ever be served as a package.
        let staging = dir.path().join("store").join(".staging");
        assert!(staging.is_dir(), "the staging directory is created");
        assert_eq!(
            std::fs::read_dir(&staging).expect("read_dir").count(),
            0,
            "no staging directory survives a successful extraction"
        );
        // An orphan older than the cutoff is collected; a fresh one is not.
        let orphan = staging.join(".tmp-orphan");
        let fresh = staging.join(".tmp-fresh");
        std::fs::create_dir_all(orphan.join("inside")).expect("mkdir");
        std::fs::create_dir_all(&fresh).expect("mkdir");
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 3600);
        filetime::set_file_mtime(&orphan, filetime::FileTime::from_system_time(long_ago))
            .expect("set mtime");
        store
            .ensure(
                "c/d",
                "1.0.0",
                Some("feedfacefeed5678"),
                &zip,
                DistKind::Zip,
            )
            .expect("extract");
        assert!(!orphan.exists(), "the orphan is swept");
        assert!(fresh.exists(), "a fresh staging directory is left alone");

        // Hostile version sanitised (no traversal).
        let p3 = store
            .ensure("a/b", "../../evil", None, &zip, DistKind::Zip)
            .expect("sane");
        assert!(p3.starts_with(dir.path().join("store").join(LAYOUT).join("a/b")));
        // The layout segment is part of the key: a tree written by another
        // extraction rule is never served under this one.
        assert!(store
            .entry_path("a/b", "1.0.0", Some("deadbeefcafe1234"))
            .components()
            .any(|c| c.as_os_str() == LAYOUT));
    }
}
