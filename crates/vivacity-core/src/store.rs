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
/// from the archive instead of "executable → 0755, default otherwise"), an old
/// entry keeps serving the old tree forever, silently. Bumping this segment is
/// what makes the change take effect; the previous trees stay on disk, unused,
/// until the cache is cleared.
const LAYOUT: &str = "v2";

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
        let final_path = self.entry_path(name, version, dist_ref);
        if final_path.is_dir() {
            return Ok(final_path);
        }
        let parent = final_path.parent().unwrap_or(&self.root).to_path_buf();
        std::fs::create_dir_all(&parent).map_err(Error::io(&parent))?;
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
        let tmp_path = tmp.keep();
        match std::fs::rename(&tmp_path, &final_path) {
            Ok(()) => Ok(final_path),
            Err(_) if final_path.is_dir() => {
                // A concurrent process won the rename: its entry is complete.
                let _ = std::fs::remove_dir_all(&tmp_path);
                Ok(final_path)
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

    pub fn contains(&self, name: &str, version: &str, dist_ref: Option<&str>) -> bool {
        self.entry_path(name, version, dist_ref).is_dir()
    }
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
