//! What the class map cache takes as "the same file": a validation policy
//! for its records, not a proof of identical bytes.
//!
//! Under Unix a file is identified by (device, inode, size, mtime, ctime),
//! nanoseconds included. No unprivileged call sets the ctime: a write,
//! `touch -r`, a `chmod` or a new link moves it to the current time, and a
//! file replaced by a rename (`sed -i`, an editor's "safe write") has a new
//! inode. What it does not see, and the README says: the system clock moved
//! back, a privileged tool writing the disk (`debugfs`), a file system that
//! does not keep a real ctime (some FUSE and network mounts, FAT's "ctime"
//! is a creation time). The ctime is no cryptographic guarantee.
//!
//! A file written within the timestamp granularity of the moment its
//! identity was taken can change again without its identity changing ("racy"
//! entries, git's rule): [`Clock`] tells which identities are too recent to
//! be trusted by a later run.
//!
//! Windows: the standard library gives neither a ctime nor a file id, and
//! the NTFS `ChangeTime` can be set by `SetFileInformationByHandle`. No
//! identity is trusted there ([`FileIdentity::of`] is `None`): the class map
//! cache reads and hashes every file, and runs the class detection only
//! when the content changed. Correct and slower; to be measured on the
//! Windows runner.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// (device, inode, size, mtime, ctime) of a file, seconds and nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime: (i64, i64),
    pub ctime: (i64, i64),
}

impl FileIdentity {
    /// The identity of the file at `path` (symbolic links followed). `None`
    /// when it cannot be read, and always under Windows (module header).
    #[cfg(unix)]
    pub fn of(path: &Path) -> Option<FileIdentity> {
        std::fs::metadata(path)
            .ok()
            .map(|m| FileIdentity::from_metadata(&m))
    }

    #[cfg(not(unix))]
    pub fn of(_path: &Path) -> Option<FileIdentity> {
        None
    }

    #[cfg(unix)]
    fn from_metadata(m: &std::fs::Metadata) -> FileIdentity {
        use std::os::unix::fs::MetadataExt as _;
        FileIdentity {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        }
    }

    fn ctime_ns(&self) -> i128 {
        i128::from(self.ctime.0) * 1_000_000_000 + i128::from(self.ctime.1)
    }

    /// The later of the change and modification times: a file system whose
    /// ctime does not move on a write still moves the mtime.
    fn latest_ns(&self) -> i128 {
        let mtime = i128::from(self.mtime.0) * 1_000_000_000 + i128::from(self.mtime.1);
        self.ctime_ns().max(mtime)
    }

    /// Fixed-size little-endian encoding (56 bytes), for the cache records.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.dev.to_le_bytes());
        buf.extend_from_slice(&self.ino.to_le_bytes());
        buf.extend_from_slice(&self.size.to_le_bytes());
        buf.extend_from_slice(&self.mtime.0.to_le_bytes());
        buf.extend_from_slice(&self.mtime.1.to_le_bytes());
        buf.extend_from_slice(&self.ctime.0.to_le_bytes());
        buf.extend_from_slice(&self.ctime.1.to_le_bytes());
    }

    pub const ENCODED_LEN: usize = 56;

    pub fn decode(b: &[u8]) -> Option<FileIdentity> {
        if b.len() != Self::ENCODED_LEN {
            return None;
        }
        let u = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
        let s = |i: usize| i64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
        Some(FileIdentity {
            dev: u(0),
            ino: u(1),
            size: u(2),
            mtime: (s(3), s(4)),
            ctime: (s(5), s(6)),
        })
    }
}

/// The margin under which an identity is "racy": the coarsest timestamp
/// granularity in sight (FAT 2 s, HFS+ 1 s, Linux before multigrain
/// timestamps a few ms). One margin everywhere, no `statfs`: at worst a file
/// changed in the last two seconds is read once more.
const RACY_MARGIN_NS: i128 = 2_000_000_000;

/// The reference time of each file system a scan reads, taken BEFORE the
/// scan as the ctime of a probe file created on that file system — never
/// the system clock, which can differ from the file system's (a Docker
/// Desktop bind mount carries the host's clock). An identity whose ctime (or
/// mtime) is within [`RACY_MARGIN_NS`] of that time, or later, is racy: a
/// later run does not trust it alone. A device with no probe is racy
/// throughout.
#[derive(Debug, Default)]
pub struct Clock {
    /// device -> reference ctime in ns.
    now: HashMap<u64, i128>,
}

impl Clock {
    /// Probes, for each device of `devices_of`, the first of `probe_dirs` on
    /// that device: an unnamed temporary file (`O_TMPFILE` under Linux;
    /// created and unlinked at once elsewhere), its ctime read, nothing left
    /// behind. Under Windows nothing is probed (no identity is trusted).
    pub fn probe(devices_of: &[&Path], probe_dirs: &[&Path]) -> Clock {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let dev_of = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
            let probes: Vec<(u64, &Path)> = probe_dirs
                .iter()
                .filter_map(|d| dev_of(d).map(|dev| (dev, *d)))
                .collect();
            let mut now = HashMap::new();
            for path in devices_of {
                let Some(dev) = dev_of(path) else { continue };
                if now.contains_key(&dev) {
                    continue;
                }
                let Some((_, dir)) = probes.iter().find(|(d, _)| *d == dev) else {
                    continue;
                };
                let probed = tempfile::tempfile_in(dir)
                    .and_then(|f| f.metadata())
                    .ok()
                    .map(|m| FileIdentity::from_metadata(&m))
                    .filter(|id| id.dev == dev);
                if let Some(id) = probed {
                    now.insert(dev, id.ctime_ns());
                }
            }
            Clock { now }
        }
        #[cfg(not(unix))]
        {
            let _ = (devices_of, probe_dirs);
            Clock::default()
        }
    }

    /// A clock whose reference time on device `dev` is `now` (seconds,
    /// nanoseconds) — for tests, which need identities that are not racy
    /// without waiting.
    pub fn at(dev: u64, now: (i64, i64)) -> Clock {
        let mut clock = Clock::default();
        clock
            .now
            .insert(dev, i128::from(now.0) * 1_000_000_000 + i128::from(now.1));
        clock
    }

    /// Whether `id` was taken too close to (or after) its file system's
    /// reference time to be trusted by a later run.
    pub fn is_racy(&self, id: &FileIdentity) -> bool {
        match self.now.get(&id.dev) {
            Some(now) => id.latest_ns() >= now - RACY_MARGIN_NS,
            None => true,
        }
    }
}

/// A package this run laid out from a store entry: the identity of each of
/// its PHP files taken right after the placement. Within the same run, a
/// file whose identity is still that one holds the entry's bytes, and its
/// classes are the entry's record's (the class map cache's "placed" case).
#[derive(Debug)]
pub struct PlacedPackage {
    pub name: String,
    /// Canonical install directory: the root of the copy of `entry`.
    pub dest: PathBuf,
    /// The store entry it was copied from.
    pub entry: PathBuf,
    /// Canonical path (OS bytes) -> identity right after the placement.
    pub files: HashMap<Vec<u8>, FileIdentity>,
}

impl PlacedPackage {
    /// Walks `roots` under `dest` (paths relative to it, files or
    /// directories; `""` is `dest` itself) — symbolic links not followed,
    /// dot entries skipped like the class map walk — and takes the identity
    /// of every `.php`, `.inc` and `.hh` file. `None` under Windows, or when
    /// `dest` cannot be canonicalized.
    pub fn record(
        name: &str,
        dest: &Path,
        entry: &Path,
        roots: &[String],
    ) -> Option<PlacedPackage> {
        if cfg!(not(unix)) {
            return None;
        }
        let dest = crate::pathutil::canonicalize(dest).ok()?;
        let mut files = HashMap::new();
        let php = |p: &Path| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("php" | "inc" | "hh")
            )
        };
        let take = |p: &Path, files: &mut HashMap<Vec<u8>, FileIdentity>| {
            if let Some(id) = FileIdentity::of(p) {
                files.insert(p.as_os_str().as_encoded_bytes().to_vec(), id);
            }
        };
        let mut stack: Vec<PathBuf> = Vec::new();
        for root in roots {
            let path = if root.is_empty() {
                dest.clone()
            } else {
                dest.join(root)
            };
            match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() => stack.push(path),
                Ok(m) if m.is_file() && php(&path) => take(&path, &mut files),
                _ => {}
            }
        }
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                if e.file_name().as_encoded_bytes().first() == Some(&b'.') {
                    continue;
                }
                let Ok(ft) = e.file_type() else { continue };
                let path = e.path();
                if ft.is_dir() {
                    stack.push(path);
                } else if ft.is_file() && php(&path) {
                    take(&path, &mut files);
                }
            }
        }
        Some(PlacedPackage {
            name: name.to_owned(),
            dest,
            entry: entry.to_path_buf(),
            files,
        })
    }

    /// The paths, relative to a package's install directory, that the
    /// autoloader dump may scan for its `autoload` rules: the `classmap`
    /// entries, and with `psr` the PSR-0/PSR-4 directories too (an
    /// optimized dump). The whole directory (`""`) for anything this cannot
    /// place exactly — a glob, a `..`, an absolute path, a `target-dir`.
    pub fn autoload_roots(
        autoload: &serde_json::Value,
        target_dir: bool,
        psr: bool,
    ) -> Vec<String> {
        let whole = || vec![String::new()];
        if target_dir {
            return whole();
        }
        let mut roots = Vec::new();
        let mut add = |v: &serde_json::Value| -> bool {
            let paths: Vec<&serde_json::Value> = match v {
                serde_json::Value::Array(a) => a.iter().collect(),
                other => vec![other],
            };
            for p in paths {
                let Some(p) = p.as_str() else { return false };
                let p = p.trim_start_matches("./").trim_end_matches('/');
                if p.contains(['*', '?', '[', '\\'])
                    || p.starts_with('/')
                    || p.split('/').any(|c| c == "..")
                {
                    return false;
                }
                roots.push(p.trim_end_matches("/.").to_owned());
            }
            true
        };
        if let Some(cm) = autoload.get("classmap") {
            if !add(cm) {
                return whole();
            }
        }
        if psr {
            for key in ["psr-0", "psr-4"] {
                if let Some(map) = autoload.get(key).and_then(serde_json::Value::as_object) {
                    for v in map.values() {
                        if !add(v) {
                            return whole();
                        }
                    }
                }
            }
        }
        if roots.iter().any(|r| r.is_empty() || r == ".") {
            return whole();
        }
        roots
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn identity_changes_with_ctime_alone_and_with_inode_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let f = tmp.path().join("a.php");
        std::fs::write(&f, b"<?php class A {}").expect("write");
        let before = FileIdentity::of(&f).expect("id");
        // Same bytes, mtime put back: only the ctime moves.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mtime = std::fs::metadata(&f)
            .expect("meta")
            .modified()
            .expect("mtime");
        std::fs::File::options()
            .write(true)
            .open(&f)
            .and_then(|h| h.set_modified(mtime))
            .expect("touch -r");
        let after = FileIdentity::of(&f).expect("id");
        assert_eq!(
            (before.ino, before.size, before.mtime),
            (after.ino, after.size, after.mtime)
        );
        assert_ne!(before, after, "the ctime alone must change the identity");
        // Same bytes, same mtime, new inode (a rename over it).
        let g = tmp.path().join("b.php");
        std::fs::write(&g, b"<?php class A {}").expect("write");
        std::fs::File::options()
            .write(true)
            .open(&g)
            .and_then(|h| h.set_modified(mtime))
            .expect("touch -r");
        std::fs::rename(&g, &f).expect("rename");
        let renamed = FileIdentity::of(&f).expect("id");
        assert_ne!(renamed.ino, after.ino);
        assert_ne!(renamed, after);
    }

    #[test]
    fn encoding_round_trips() {
        let id = FileIdentity {
            dev: 1,
            ino: u64::MAX,
            size: 3,
            mtime: (-4, 5),
            ctime: (i64::MAX, 999_999_999),
        };
        let mut b = Vec::new();
        id.encode(&mut b);
        assert_eq!(b.len(), FileIdentity::ENCODED_LEN);
        assert_eq!(FileIdentity::decode(&b), Some(id));
        assert_eq!(FileIdentity::decode(&b[1..]), None);
    }

    #[test]
    fn racy_window_follows_the_probe_and_unknown_devices_are_racy() {
        let tmp = tempfile::tempdir().expect("tmp");
        let f = tmp.path().join("a.php");
        std::fs::write(&f, b"x").expect("write");
        let clock = Clock::probe(&[tmp.path()], &[tmp.path()]);
        let fresh = FileIdentity::of(&f).expect("id");
        assert!(clock.is_racy(&fresh), "a file written just now is racy");
        let old = FileIdentity {
            ctime: (fresh.ctime.0 - 3, fresh.ctime.1),
            mtime: (fresh.ctime.0 - 3, fresh.ctime.1),
            ..fresh
        };
        assert!(
            !clock.is_racy(&old),
            "three seconds before the probe: not racy"
        );
        let rewritten = FileIdentity {
            mtime: fresh.ctime,
            ..old
        };
        assert!(clock.is_racy(&rewritten), "a recent mtime alone: racy");
        let elsewhere = FileIdentity {
            dev: fresh.dev.wrapping_add(1),
            ..old
        };
        assert!(clock.is_racy(&elsewhere), "no probe on that device: racy");
        assert_eq!(
            std::fs::read_dir(tmp.path()).expect("ls").count(),
            1,
            "the probe leaves nothing behind"
        );
        let unprobed = Clock::probe(&[tmp.path()], &[]);
        assert!(unprobed.is_racy(&old));
    }

    #[test]
    fn placed_package_records_php_files_by_canonical_path() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dest = tmp.path().join("pkg");
        std::fs::create_dir_all(dest.join("src/.hidden")).expect("mkdir");
        std::fs::write(dest.join("src/A.php"), b"<?php").expect("w");
        std::fs::write(dest.join("src/.hidden/B.php"), b"<?php").expect("w");
        std::fs::write(dest.join("README.md"), b"x").expect("w");
        std::os::unix::fs::symlink(dest.join("src/A.php"), dest.join("src/L.php")).expect("ln");
        let p = PlacedPackage::record("a/b", &dest, Path::new("/store/e"), &[String::new()])
            .expect("placed");
        let canon = crate::pathutil::canonicalize(dest.join("src/A.php")).expect("canon");
        assert_eq!(p.files.len(), 1, "{:?}", p.files.keys());
        assert_eq!(
            p.files.get(canon.as_os_str().as_encoded_bytes()),
            FileIdentity::of(&canon).as_ref()
        );
        std::fs::create_dir_all(dest.join("lib")).expect("mkdir");
        std::fs::write(dest.join("lib/C.php"), b"<?php").expect("w");
        std::fs::write(dest.join("one.php"), b"<?php").expect("w");
        let p = PlacedPackage::record(
            "a/b",
            &dest,
            Path::new("/e"),
            &["lib".into(), "one.php".into()],
        )
        .expect("placed");
        assert_eq!(p.files.len(), 2, "only the roots: {:?}", p.files.keys());
    }

    #[test]
    fn autoload_roots_follow_the_rules_or_take_everything() {
        let roots = |v: serde_json::Value, psr: bool| PlacedPackage::autoload_roots(&v, false, psr);
        let rules = serde_json::json!({
            "psr-4": {"A\\": "src/", "B\\": ["lib", "./more/"]},
            "psr-0": {"C_": "legacy/"},
            "classmap": ["res/Map.php", "classes/"],
            "files": ["helpers.php"]
        });
        assert_eq!(roots(rules.clone(), false), ["res/Map.php", "classes"]);
        assert_eq!(
            roots(rules, true),
            ["res/Map.php", "classes", "legacy", "src", "lib", "more"]
        );
        assert_eq!(roots(serde_json::json!({"psr-4": {"A\\": ""}}), true), [""]);
        assert_eq!(
            roots(serde_json::json!({"psr-4": {"A\\": ""}}), false),
            Vec::<String>::new()
        );
        assert_eq!(
            roots(serde_json::json!({"classmap": ["src/*/x"]}), false),
            [""]
        );
        assert_eq!(
            roots(serde_json::json!({"classmap": ["../x"]}), false),
            [""]
        );
        assert_eq!(
            PlacedPackage::autoload_roots(&serde_json::json!({"psr-0": {"A": "src"}}), true, true),
            [""]
        );
    }
}
