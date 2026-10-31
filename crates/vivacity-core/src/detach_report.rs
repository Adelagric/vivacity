//! The report of the one-time detach pass (`installer::detach_once`): which
//! files of a vendor directory laid out by vivacity up to 0.26.0 still
//! shared their inode when the pass found them, what each held, and how
//! that compared with the file of the same path in the package's archive.
//!
//! Once a file is detached, which names shared its inode can no longer be
//! told: the report is the only trace of it. It is written in vivacity's
//! cache, never printed (the run's output stays Composer's), and only ever
//! read by a person. It describes what one pass observed, at one time: it
//! does not say the files are unlinked now (an older binary run since may
//! have linked them again), and clearing the cache loses it.
//!
//! A file that differs from its archive is not "corrupted": it may be a
//! local patch made on purpose, in this project or in another one that
//! shared the inode. The report says "different", nothing more. The
//! comparison never decides the detachment: a file is detached whether or
//! not its archive can be read.
//!
//! The expected content comes from the archive only, never from an entry of
//! an older store layout (the one that may have been written through a
//! link), and never from the network:
//! - the store entry this very run extracted from the archive (the bytes it
//!   read, their sha1 noted), when there is one;
//! - otherwise the archive in Composer's files cache, extracted aside for
//!   the report — the same extraction as the store's, so strip, symlinks,
//!   duplicated names and Windows renaming all come out the same — and
//!   refused when the lock's `dist.shasum` is not empty and differs.

use crate::clone::LinkedFile;
use crate::store::{DistKind, Store};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Bumped when a field changes meaning.
const FORMAT: u64 = 1;

/// The archive a store entry was extracted from by this run.
#[derive(Debug, Clone)]
pub struct ArchiveId {
    pub url: String,
    /// Where Composer's files cache keeps it (`fetch::dist_cache_path_of`).
    pub cache_path: PathBuf,
    /// Read from that cache, or fetched by this run (and written there).
    pub from_cache: bool,
    /// The lock's `dist.shasum`, which the fetch checked the extracted
    /// bytes against; `None` when the lock has none (the archive's sha1 is
    /// then taken from the cache file, and only when the report needs it:
    /// nothing is hashed for a project with nothing linked).
    pub checked_shasum: Option<String>,
}

/// Where the expected content of a package's files comes from.
#[derive(Debug, Clone)]
pub enum Reference {
    /// The store entry this run extracted from `archive`.
    ExtractedThisRun { entry: PathBuf, archive: ArchiveId },
    /// The archive in Composer's files cache.
    ComposerCache {
        url: String,
        path: PathBuf,
        kind: DistKind,
    },
    /// Nothing to compare with, and why.
    Unavailable(String),
}

/// An installed package the files may belong to.
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub dist_reference: Option<String>,
    /// `dist.shasum` when not empty.
    pub dist_shasum: Option<String>,
    /// Where it is installed (target-dir included): what the store entry's
    /// root maps to.
    pub dir: PathBuf,
    pub reference: Reference,
}

/// The counts of an observation, for the trace line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub linked: usize,
    pub detached: usize,
    pub detach_failed: usize,
    pub identical: usize,
    pub different: usize,
    pub not_compared: usize,
}

impl std::fmt::Display for Counts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} linked ({} detached, {} failed); {} identical, {} different, {} not compared",
            self.linked,
            self.detached,
            self.detach_failed,
            self.identical,
            self.different,
            self.not_compared
        )
    }
}

/// The observation of one pass: `files` as the walk recorded them, mapped
/// onto `packages` (the deepest install path that holds a file wins).
pub fn observe(
    store: &Store,
    vendor: &Path,
    files: &[LinkedFile],
    packages: &[Package],
    complete: bool,
    now: std::time::SystemTime,
) -> (Value, Counts) {
    let mut counts = Counts::default();
    // Package index (`packages.len()`: under no package, last) -> files.
    let mut groups: std::collections::BTreeMap<usize, Vec<&LinkedFile>> =
        std::collections::BTreeMap::new();
    for f in files {
        let owner = packages
            .iter()
            .enumerate()
            .filter(|(_, p)| f.path.starts_with(&p.dir))
            .max_by_key(|(_, p)| p.dir.components().count())
            .map_or(packages.len(), |(i, _)| i);
        groups.entry(owner).or_default().push(f);
    }
    let mut out = Vec::new();
    for (owner, mut group) in groups {
        group.sort_by(|a, b| a.path.cmp(&b.path));
        let package = packages.get(owner);
        let (resolved, scratch) = match package {
            Some(p) => resolve(store, p),
            None => (
                Err("the file is under no installed package".to_owned()),
                None,
            ),
        };
        let root: Option<&Path> = match &resolved {
            Ok((root, _)) => Some(root.as_path()),
            Err(_) => None,
        };
        let mut entries = Vec::new();
        for f in group {
            counts.linked += 1;
            let rel = match package {
                Some(p) => f.path.strip_prefix(&p.dir).unwrap_or(&f.path),
                None => f.path.strip_prefix(vendor).unwrap_or(&f.path),
            };
            let rel_text = slashed(rel);
            let detachment = match &f.detached {
                Ok(()) => {
                    counts.detached += 1;
                    json!({ "status": "detached" })
                }
                Err(why) => {
                    counts.detach_failed += 1;
                    json!({ "status": "failed", "reason": why })
                }
            };
            let comparison = compare(f, root, rel, &resolved);
            match comparison["status"].as_str() {
                Some("identical") => counts.identical += 1,
                Some("different") => counts.different += 1,
                _ => counts.not_compared += 1,
            }
            let mut e = Map::new();
            e.insert("path".to_owned(), json!(rel_text));
            e.insert("links".to_owned(), json!(f.links));
            e.insert(
                "file_id".to_owned(),
                json!(f.file_id.map(|(dev, ino)| format!("{dev}:{ino}"))),
            );
            e.insert(
                "current_sha1".to_owned(),
                json!(f.current_sha1.as_ref().ok()),
            );
            e.insert("detachment".to_owned(), detachment);
            e.insert("comparison".to_owned(), comparison);
            entries.push(Value::Object(e));
        }
        drop(scratch);
        let mut g = Map::new();
        g.insert("name".to_owned(), json!(package.map(|p| &p.name)));
        g.insert("version".to_owned(), json!(package.map(|p| &p.version)));
        g.insert(
            "dist_reference".to_owned(),
            json!(package.and_then(|p| p.dist_reference.as_ref())),
        );
        g.insert(
            "install_path".to_owned(),
            json!(package.map(|p| p.dir.to_string_lossy())),
        );
        match resolved {
            Ok((_, identity)) => {
                g.insert("reference".to_owned(), identity);
                g.insert("not_compared".to_owned(), Value::Null);
            }
            Err(why) => {
                g.insert("reference".to_owned(), Value::Null);
                g.insert("not_compared".to_owned(), json!(why));
            }
        }
        g.insert("files".to_owned(), Value::Array(entries));
        out.push(Value::Object(g));
    }
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let report = json!({
        "format": FORMAT,
        "what": "One observation of vivacity's one-time detach pass: the files of this vendor directory that shared their inode (hard links laid out by vivacity up to 0.26.0) when the pass ran, and how each compared with its archive. A past observation: it does not say the files are unlinked now, and \"different\" may be a deliberate local patch.",
        "observed_at": rfc3339(secs),
        "vivacity_version": env!("CARGO_PKG_VERSION"),
        "vendor_dir": crate::pathutil::canonicalize(vendor)
            .unwrap_or_else(|_| vendor.to_path_buf())
            .to_string_lossy(),
        "pass_complete": complete,
        "counts": {
            "linked": counts.linked,
            "detached": counts.detached,
            "detach_failed": counts.detach_failed,
            "identical": counts.identical,
            "different": counts.different,
            "not_compared": counts.not_compared,
        },
        "packages": out,
    });
    (report, counts)
}

type Resolved = std::result::Result<(PathBuf, Value), String>;

/// The tree the package's expected files are read from, and the identity of
/// that reference; the temporary extraction, if one was made, lives as long
/// as the second value.
fn resolve(store: &Store, p: &Package) -> (Resolved, Option<tempfile::TempDir>) {
    let shasum_check = |sha1: &str| -> Value {
        match &p.dist_shasum {
            Some(s) => json!(s.eq_ignore_ascii_case(sha1)),
            None => Value::Null,
        }
    };
    match &p.reference {
        Reference::Unavailable(why) => (Err(why.clone()), None),
        Reference::ExtractedThisRun { entry, archive } => {
            let (sha1, sha1_from) = match &archive.checked_shasum {
                Some(s) => (
                    Some(s.to_ascii_lowercase()),
                    "dist.shasum, checked by the fetch against the bytes extracted".to_owned(),
                ),
                None => match std::fs::read(&archive.cache_path) {
                    Ok(bytes) => (
                        Some(hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes))),
                        "the archive in Composer's files cache, read when the report was written"
                            .to_owned(),
                    ),
                    Err(e) => (None, format!("unavailable: {e}")),
                },
            };
            let identity = json!({
                "source": "store entry extracted by this run from the archive",
                "entry": entry.to_string_lossy(),
                "archive_url": archive.url,
                "archive_cache_path": archive.cache_path.to_string_lossy(),
                "archive_obtained": if archive.from_cache { "read from Composer's files cache" } else { "fetched by this run" },
                "archive_sha1": sha1,
                "archive_sha1_from": sha1_from,
                "dist_shasum": p.dist_shasum,
                "matches_dist_shasum": sha1.as_deref().map_or(Value::Null, shasum_check),
            });
            (Ok((entry.clone(), identity)), None)
        }
        Reference::ComposerCache { url, path, kind } => {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return (
                        Err(format!(
                            "no archive in Composer's files cache ({})",
                            path.display()
                        )),
                        None,
                    )
                }
                Err(e) => {
                    return (
                        Err(format!(
                            "the archive in Composer's files cache could not be read ({}): {e}",
                            path.display()
                        )),
                        None,
                    )
                }
            };
            let sha1 = hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes));
            if let Some(expected) = &p.dist_shasum {
                if !expected.eq_ignore_ascii_case(&sha1) {
                    return (
                        Err(format!(
                            "the archive in Composer's files cache ({}) has sha1 {sha1}, not the dist.shasum {expected}",
                            path.display()
                        )),
                        None,
                    );
                }
            }
            let scratch = match store.extract_staged(&p.name, &p.version, &bytes, *kind) {
                Ok(t) => t,
                Err(e) => {
                    return (
                        Err(format!(
                        "the archive in Composer's files cache ({}) could not be extracted: {e}",
                        path.display()
                    )),
                        None,
                    )
                }
            };
            let identity = json!({
                "source": "archive in Composer's files cache, extracted aside for this report",
                "archive_url": url,
                "archive_cache_path": path.to_string_lossy(),
                "archive_sha1": sha1,
                "archive_sha1_from": "the bytes extracted",
                "dist_shasum": p.dist_shasum,
                "matches_dist_shasum": shasum_check(&sha1),
            });
            (Ok((scratch.path().to_path_buf(), identity)), Some(scratch))
        }
    }
}

fn compare(f: &LinkedFile, root: Option<&Path>, rel: &Path, resolved: &Resolved) -> Value {
    let root = match (root, resolved) {
        (Some(root), _) => root,
        (None, Err(why)) => return json!({ "status": "not_compared", "reason": why }),
        (None, Ok(_)) => return json!({ "status": "not_compared", "reason": "no reference" }),
    };
    let current = match &f.current_sha1 {
        Ok(s) => s,
        Err(why) => {
            return json!({
                "status": "not_compared",
                "reason": format!("the current content could not be read: {why}"),
            })
        }
    };
    let expected = match std::fs::read(root.join(rel)) {
        Ok(bytes) => hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return json!({ "status": "not_compared", "reason": "no such file in the archive" })
        }
        Err(e) => {
            return json!({
                "status": "not_compared",
                "reason": format!("the archive's file could not be read: {e}"),
            })
        }
    };
    let status = if &expected == current {
        "identical"
    } else {
        "different"
    };
    json!({ "status": status, "expected_sha1": expected })
}

fn slashed(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Writes `report` as a new file of `dir` (one per pass: a pass never
/// rewrites an earlier one, so two processes cannot lose each other's).
/// Returns its path.
pub fn write(
    dir: &Path,
    report: &Value,
    now: std::time::SystemTime,
) -> std::result::Result<PathBuf, String> {
    let since = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let stamp: String = rfc3339(since.as_secs())
        .trim_end_matches('Z')
        .chars()
        .filter(|c| *c != '-' && *c != ':')
        .collect();
    // To the nanosecond: two passes of one process (an embedder) never
    // share a name, and a rename never replaces an earlier report.
    let name = format!(
        "{stamp}.{:09}Z-{}.json",
        since.subsec_nanos(),
        std::process::id()
    );
    let path = dir.join(&name);
    let tmp = dir.join(format!(".{name}.tmp"));
    let mut bytes = serde_json::to_vec(report).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let made = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&tmp, &bytes))
        .and_then(|()| std::fs::rename(&tmp, &path));
    match made {
        Ok(()) => Ok(path),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("{}: {e}", path.display()))
        }
    }
}

/// Whether an earlier pass left a report in `dir`.
pub fn has_reports(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| {
        d.any(|e| {
            e.is_ok_and(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.ends_with(".json") && !name.starts_with('.')
            })
        })
    })
}

/// `YYYY-MM-DDTHH:MM:SSZ` (UTC).
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's days -> civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body) in files {
            w.start_file(format!("root/{name}"), SimpleFileOptions::default())
                .expect("start");
            w.write_all(body).expect("write");
        }
        w.finish().expect("finish").into_inner()
    }

    fn sha1_hex(b: &[u8]) -> String {
        hex::encode(<sha1::Sha1 as sha1::Digest>::digest(b))
    }

    fn linked(path: PathBuf, body: &[u8]) -> LinkedFile {
        LinkedFile {
            path,
            links: Some(3),
            file_id: Some((1, 42)),
            current_sha1: Ok(sha1_hex(body)),
            detached: Ok(()),
        }
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        store: Store,
        vendor: PathBuf,
        archive: PathBuf,
    }

    fn setup() -> Setup {
        let tmp = tempfile::tempdir().expect("tmp");
        let store = Store::at(tmp.path().join("store"));
        let vendor = tmp.path().join("p/vendor");
        let archive = tmp.path().join("composer-cache/files/acme/lib/0123.zip");
        std::fs::create_dir_all(archive.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &archive,
            zip_of(&[
                ("src/A.php", b"original a\n"),
                ("src/B.php", b"original b\n"),
            ]),
        )
        .expect("zip");
        Setup {
            _tmp: tmp,
            store,
            vendor,
            archive,
        }
    }

    fn package(s: &Setup, reference: Reference, shasum: Option<String>) -> Package {
        Package {
            name: "acme/lib".to_owned(),
            version: "1.0.0".to_owned(),
            dist_reference: Some("abcdef".to_owned()),
            dist_shasum: shasum,
            dir: s.vendor.join("acme/lib"),
            reference,
        }
    }

    fn cache_ref(s: &Setup) -> Reference {
        Reference::ComposerCache {
            url: "https://example.org/lib.zip".to_owned(),
            path: s.archive.clone(),
            kind: DistKind::Zip,
        }
    }

    fn file<'a>(report: &'a Value, pkg: usize, path: &str) -> &'a Value {
        report["packages"][pkg]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == path)
            .unwrap_or_else(|| panic!("{path} not in {report}"))
    }

    /// One file identical to the archive's, one different (a local patch
    /// or a propagated edit: the report says "different", nothing more):
    /// both hashes and the archive's identity are recorded, and no
    /// temporary extraction is left behind.
    #[test]
    fn identical_and_different_against_composers_cache() {
        let s = setup();
        let dir = s.vendor.join("acme/lib");
        let files = vec![
            linked(dir.join("src/B.php"), b"patched b\n"),
            linked(dir.join("src/A.php"), b"original a\n"),
        ];
        let p = package(&s, cache_ref(&s), None);
        let (report, counts) = observe(
            &s.store,
            &s.vendor,
            &files,
            &[p],
            true,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_761_912_000),
        );
        assert_eq!(
            counts,
            Counts {
                linked: 2,
                detached: 2,
                identical: 1,
                different: 1,
                ..Counts::default()
            }
        );
        assert_eq!(report["observed_at"], "2025-10-31T12:00:00Z");
        assert_eq!(report["vivacity_version"], env!("CARGO_PKG_VERSION"));
        let a = file(&report, 0, "src/A.php");
        assert_eq!(a["comparison"]["status"], "identical");
        assert_eq!(a["comparison"]["expected_sha1"], sha1_hex(b"original a\n"));
        assert_eq!(a["file_id"], "1:42");
        let b = file(&report, 0, "src/B.php");
        assert_eq!(b["comparison"]["status"], "different");
        assert_eq!(b["current_sha1"], sha1_hex(b"patched b\n"));
        assert_eq!(b["comparison"]["expected_sha1"], sha1_hex(b"original b\n"));
        assert_eq!(b["detachment"]["status"], "detached");
        let r = &report["packages"][0]["reference"];
        assert_eq!(
            r["archive_sha1"],
            sha1_hex(&std::fs::read(&s.archive).unwrap())
        );
        assert_eq!(
            r["archive_cache_path"],
            s.archive.to_string_lossy().as_ref()
        );
        assert_eq!(r["matches_dist_shasum"], Value::Null);
        assert_eq!(report["packages"][0]["dist_reference"], "abcdef");
        // Files sorted by path.
        assert_eq!(report["packages"][0]["files"][0]["path"], "src/A.php");
        let staging = s.store_root().join(".staging");
        assert_eq!(
            std::fs::read_dir(&staging).map(|d| d.count()).unwrap_or(0),
            0
        );
    }

    impl Setup {
        fn store_root(&self) -> PathBuf {
            self._tmp.path().join("store")
        }
    }

    /// No archive in Composer's cache: nothing is compared, the reason is
    /// said, and the detachment outcomes are reported all the same.
    #[test]
    fn archive_missing_is_not_compared() {
        let s = setup();
        std::fs::remove_file(&s.archive).expect("rm");
        let dir = s.vendor.join("acme/lib");
        let mut failed = linked(dir.join("src/A.php"), b"original a\n");
        failed.detached = Err("Permission denied".to_owned());
        let files = vec![failed, linked(dir.join("src/B.php"), b"original b\n")];
        let (report, counts) = observe(
            &s.store,
            &s.vendor,
            &files,
            &[package(&s, cache_ref(&s), None)],
            false,
            std::time::SystemTime::now(),
        );
        assert_eq!(
            counts,
            Counts {
                linked: 2,
                detached: 1,
                detach_failed: 1,
                not_compared: 2,
                ..Counts::default()
            }
        );
        let g = &report["packages"][0];
        assert_eq!(g["reference"], Value::Null);
        assert!(g["not_compared"]
            .as_str()
            .unwrap()
            .starts_with("no archive in Composer's files cache"));
        let a = file(&report, 0, "src/A.php");
        assert_eq!(a["detachment"]["status"], "failed");
        assert_eq!(a["comparison"]["status"], "not_compared");
        assert_eq!(a["current_sha1"], sha1_hex(b"original a\n"));
        assert_eq!(report["pass_complete"], false);
    }

    /// An archive whose sha1 is not the lock's `dist.shasum` is not a
    /// reference.
    #[test]
    fn an_archive_with_the_wrong_shasum_is_not_a_reference() {
        let s = setup();
        let dir = s.vendor.join("acme/lib");
        let files = vec![linked(dir.join("src/A.php"), b"original a\n")];
        let p = package(&s, cache_ref(&s), Some("0".repeat(40)));
        let (report, counts) = observe(
            &s.store,
            &s.vendor,
            &files,
            &[p],
            true,
            std::time::SystemTime::now(),
        );
        assert_eq!(counts.not_compared, 1);
        assert!(report["packages"][0]["not_compared"]
            .as_str()
            .unwrap()
            .contains("not the dist.shasum"));
        // The right one is.
        let right = sha1_hex(&std::fs::read(&s.archive).unwrap());
        let p = package(&s, cache_ref(&s), Some(right));
        let (report, counts) = observe(
            &s.store,
            &s.vendor,
            &files,
            &[p],
            true,
            std::time::SystemTime::now(),
        );
        assert_eq!(counts.identical, 1);
        assert_eq!(
            report["packages"][0]["reference"]["matches_dist_shasum"],
            true
        );
    }

    /// The entry this run extracted is read in place of the archive; a file
    /// under no package and a file the archive lacks are said so.
    #[test]
    fn entry_extracted_this_run_and_files_without_reference() {
        let s = setup();
        let bytes = std::fs::read(&s.archive).unwrap();
        let (entry, made) = s
            .store
            .ensure_made("acme/lib", "1.0.0", Some("abcdef"), &bytes, DistKind::Zip)
            .expect("extract");
        assert!(made);
        let dir = s.vendor.join("acme/lib");
        let files = vec![
            linked(dir.join("src/A.php"), b"original a\n"),
            linked(dir.join("src/Extra.php"), b"extra\n"),
            linked(s.vendor.join("stray.txt"), b"stray\n"),
        ];
        let archive = ArchiveId {
            url: "https://example.org/lib.zip".to_owned(),
            cache_path: s.archive.clone(),
            from_cache: true,
            checked_shasum: None,
        };
        let p = package(
            &s,
            Reference::ExtractedThisRun {
                entry: entry.clone(),
                archive,
            },
            None,
        );
        let (report, counts) = observe(
            &s.store,
            &s.vendor,
            &files,
            &[p],
            true,
            std::time::SystemTime::now(),
        );
        assert_eq!(counts.identical, 1);
        assert_eq!(counts.not_compared, 2);
        // The package first, the files under no package last.
        let r = &report["packages"][0]["reference"];
        assert_eq!(r["entry"], entry.to_string_lossy().as_ref());
        // No shasum in the lock: the archive's sha1 is the cache file's.
        assert_eq!(r["archive_sha1"], sha1_hex(&bytes));
        assert_eq!(
            file(&report, 0, "src/Extra.php")["comparison"]["reason"],
            "no such file in the archive"
        );
        let stray = &report["packages"][1];
        assert_eq!(stray["name"], Value::Null);
        assert_eq!(stray["files"][0]["path"], "stray.txt");
        assert_eq!(
            stray["files"][0]["comparison"]["reason"],
            "the file is under no installed package"
        );
    }

    /// One file per pass, never a rewrite; a pass with nothing written
    /// leaves no report.
    #[test]
    fn each_pass_writes_its_own_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path().join("reports");
        assert!(!has_reports(&dir));
        let first = write(&dir, &json!({"n": 1}), std::time::UNIX_EPOCH).expect("write");
        assert!(has_reports(&dir));
        let later = std::time::UNIX_EPOCH + std::time::Duration::from_secs(60);
        let second = write(&dir, &json!({"n": 2}), later).expect("write");
        assert_ne!(first, second);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let v: Value = serde_json::from_slice(&std::fs::read(&first).unwrap()).unwrap();
        assert_eq!(v["n"], 1);
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("19700101T000000.000000000Z-"));
    }

    #[test]
    fn rfc3339_matches_known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_761_912_000), "2025-10-31T12:00:00Z");
    }
}
