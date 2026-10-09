//! Port of `Composer\Repository\PlatformRepository`: the platform packages
//! (composer*, php*, ext-*, lib-*, hhvm) in Composer's exact order, with the
//! `config.platform` overrides. Probing the current PHP is done by
//! assets/platform-probe.php (a transcription of `initialize()`, raw
//! versions); normalization and its fallbacks are replayed here with the
//! exact port of VersionParser.

use crate::constraint::{Constraint, Op};
use crate::package::{Link, LinkType, Links, Origin, Package};
use crate::version::{group, normalize, regex};
use pcre2::bytes::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::OnceLock;

/// Emulated Composer version and its APIs (Composer::VERSION,
/// PluginInterface::PLUGIN_API_VERSION, Composer::RUNTIME_API_VERSION).
pub const COMPOSER_VERSION: &str = "2.10.3";
pub const PLUGIN_API_VERSION: &str = "2.9.0";
pub const RUNTIME_API_VERSION: &str = "2.2.2";

const PROBE: &str = include_str!("../assets/platform-probe.php");

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PlatformError(pub String);

/// `PlatformRepository::PLATFORM_PACKAGE_REGEX`.
pub fn is_platform_package(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        &RE,
        r"^(?:php(?:-64bit|-ipv6|-zts|-debug)?|hhvm|(?:ext|lib)-[a-z0-9](?:[_.-]?[a-z0-9]+)*|composer(?:-(?:plugin|runtime)-api)?)\z",
        true,
    );
    re.is_match(name.as_bytes()).unwrap_or(false)
}

/// A raw probe entry.
#[derive(Debug, Clone, serde::Deserialize)]
struct Probed {
    kind: String,
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    replaces: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
}

/// `XdebugHandler::getAllIniFiles()` from the probe's `ini` entry:
/// `COMPOSER_ORIGINAL_INIS` (set by a Composer restarted without xdebug)
/// wins; else `[php_ini_loaded_file()]` + the trimmed scanned files.
pub fn ini_files(probed: &[Value]) -> Vec<String> {
    if let Ok(env) = std::env::var("COMPOSER_ORIGINAL_INIS") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        return env.split(sep).map(str::to_owned).collect();
    }
    let Some(entry) = probed
        .iter()
        .find(|v| v.get("kind").and_then(Value::as_str) == Some("ini"))
    else {
        return vec![String::new()];
    };
    let mut paths = vec![entry
        .get("loaded")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()];
    if let Some(scanned) = entry.get("scanned").and_then(Value::as_str) {
        paths.extend(scanned.split(',').map(|s| s.trim().to_owned()));
    }
    paths
}

/// The extensions the probed PHP has loaded (`extension_loaded`), before
/// any `config.platform` override: `ext-<name>` keys, lowercased.
pub fn loaded_extensions(probed: &[Value]) -> std::collections::BTreeSet<String> {
    probed
        .iter()
        .filter(|v| v.get("kind").and_then(Value::as_str) == Some("ext"))
        .filter(|v| v.get("skipped").and_then(Value::as_bool) != Some(true))
        .filter_map(|v| v.get("name").and_then(Value::as_str))
        .map(|n| format!("ext-{}", n.to_lowercase()))
        .collect()
}

/// Identity of a file the probe's result depends on: its path and, when it
/// exists, its mtime (nanoseconds), size, and on unix its ctime and inode — a
/// copy made with `cp -p`, `tar` or `rsync -t` keeps the mtime, it cannot keep
/// the ctime. An absent file is recorded as absent, so its appearance
/// invalidates the cache too.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FileId {
    path: String,
    mtime_ns: Option<i128>,
    size: Option<u64>,
    ctime_ns: Option<i128>,
    ino: Option<u64>,
}

impl FileId {
    fn of(path: &str) -> FileId {
        FileId::from_meta(path, std::fs::metadata(path).ok())
    }

    fn from_meta(path: &str, meta: Option<std::fs::Metadata>) -> FileId {
        let mtime_ns = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_nanos() as i128)
        });
        #[cfg(unix)]
        let (ctime_ns, ino) = {
            use std::os::unix::fs::MetadataExt as _;
            (
                meta.as_ref()
                    .map(|m| i128::from(m.ctime()) * 1_000_000_000 + i128::from(m.ctime_nsec())),
                meta.as_ref().map(|m| m.ino()),
            )
        };
        #[cfg(not(unix))]
        let (ctime_ns, ino) = (None, None);
        FileId {
            path: path.to_owned(),
            mtime_ns,
            size: meta.map(|m| m.len()),
            ctime_ns,
            ino,
        }
    }

    fn exists(&self) -> bool {
        self.size.is_some()
    }
}

/// The environment the probe's output depends on, captured by value: the
/// xdebug handling in the probe (`COMPOSER_ALLOW_XDEBUG` was read by the probe
/// and missing here until 0.21), PHP's own ini discovery, and what the dynamic
/// linker reads to decide WHICH libraries to load — a change there loads other
/// files without touching any file the key watches.
const PROBE_ENV: &[&str] = &[
    "PHPRC",
    "PHP_INI_SCAN_DIR",
    "XDEBUG_MODE",
    "XDEBUG_CONFIG",
    "COMPOSER_ALLOW_XDEBUG",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
];

/// Bumped whenever the key's shape or meaning changes, and COMPARED on read: an
/// older cache must not validate by the accident of a field that deserialises
/// to a default.
const CACHE_FORMAT: u32 = 2;

/// One cached probe, valid while everything it was derived from is the same:
/// the probe script itself, the php binary, every ini file PHP read, the
/// environment, every image the process loaded, the package-manager links
/// those images were reached through, and the system's own markers.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ProbeCache {
    format: u32,
    script: String,
    php: FileId,
    inis: Vec<FileId>,
    env: Vec<(String, Option<String>)>,
    images: Vec<FileId>,
    links: Vec<(String, Option<String>)>,
    markers: Vec<FileId>,
    probed: Vec<Value>,
}

/// The files PHP read its configuration from, per the probe's `ini` entry:
/// the loaded php.ini, the scanned files, and the scan directory itself
/// (its mtime changes when a file is added or removed).
fn ini_dependencies(probed: &[Value]) -> Vec<FileId> {
    let Some(entry) = probed
        .iter()
        .find(|v| v.get("kind").and_then(Value::as_str) == Some("ini"))
    else {
        return Vec::new();
    };
    let mut paths: Vec<String> = Vec::new();
    for key in ["loaded", "scan_dir"] {
        if let Some(p) = entry.get(key).and_then(Value::as_str) {
            if !p.is_empty() {
                paths.push(p.to_owned());
            }
        }
    }
    if let Some(scanned) = entry.get("scanned").and_then(Value::as_str) {
        paths.extend(
            scanned
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        );
    }
    paths.iter().map(|p| FileId::of(p)).collect()
}

fn probe_env() -> Vec<(String, Option<String>)> {
    PROBE_ENV
        .iter()
        .map(|k| ((*k).to_owned(), std::env::var(k).ok()))
        .collect()
}

/// The probe script, by content: a vivacity that ships a different
/// transcription must not serve the output of the previous one.
fn script_hash() -> String {
    use sha1::Digest as _;
    let digest = sha1::Sha1::digest(PROBE.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// For every image under a Homebrew-style keg — `<prefix>/Cellar/<formula>/…` —
/// the link `<prefix>/opt/<formula>` and where it points. Measured on
/// 2026-10-08: `brew upgrade pcre2` left `Cellar/pcre2/10.47_1` untouched next
/// to `Cellar/pcre2/10.49`, and only the link moved; a key of resolved paths
/// stayed valid across the upgrade. The link's target is what moves, so it is
/// what the key holds — exact, and blind to a `brew install` of anything else.
fn keg_links(images: &[String]) -> Vec<(String, Option<String>)> {
    let mut seen = std::collections::BTreeMap::new();
    for image in images {
        let Some(at) = image.find("/Cellar/") else {
            continue;
        };
        let prefix = &image[..at];
        let Some(formula) = image[at + "/Cellar/".len()..].split('/').next() else {
            continue;
        };
        if formula.is_empty() {
            continue;
        }
        let link = format!("{prefix}/opt/{formula}");
        seen.entry(link.clone()).or_insert_with(|| read_link(&link));
    }
    seen.into_iter().collect()
}

fn read_link(path: &str) -> Option<String> {
    std::fs::read_link(path)
        .ok()
        .map(|t| t.to_string_lossy().into_owned())
}

/// Files that change when the system's libraries or their data change, whatever
/// the images say: the dynamic linker's cache, the package databases (they also
/// move for data files no image list can see — tzdata, ICU), and on macOS the
/// dyld shared cache, where `/usr/lib` and the system frameworks actually live.
fn system_markers() -> Vec<FileId> {
    let mut out: Vec<FileId> = [
        "/etc/ld.so.cache",
        "/var/lib/dpkg/status",
        "/var/lib/rpm/rpmdb.sqlite",
        "/var/lib/rpm/Packages",
        "/lib/apk/db/installed",
        "/var/lib/pacman/local",
    ]
    .iter()
    .map(|p| FileId::of(p))
    .filter(FileId::exists)
    .collect();
    for dir in [
        "/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld",
        "/System/Library/dyld",
    ] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut caches: Vec<FileId> = entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("dyld_shared_cache_")
                })
                .map(|e| FileId::from_meta(&e.path().to_string_lossy(), e.metadata().ok()))
                .collect();
            caches.sort_by(|a, b| a.path.cmp(&b.path));
            out.extend(caches);
        }
    }
    out
}

/// What raced the probe: the system markers, plus the `opt` directory of the
/// Homebrew prefix the php binary lives in, when it lives in one — any `brew`
/// operation rewrites it. Taken before and after the run; a difference means
/// something moved while PHP was answering, and the answer is not cached.
fn race_snapshot(php_path: &str) -> Vec<FileId> {
    let mut out = system_markers();
    if let Some(at) = php_path.find("/Cellar/") {
        out.push(FileId::of(&format!("{}/opt", &php_path[..at])));
    }
    out
}

/// The php binary `php` names: as given when it holds a path separator,
/// else the first executable of that name on PATH (with PATHEXT's `.exe`,
/// `.bat`, `.cmd` on Windows), canonicalized. None when there is none.
fn locate_php(php: &str) -> Option<std::path::PathBuf> {
    let candidates: Vec<std::path::PathBuf> = if php.contains('/') || php.contains('\\') {
        vec![std::path::PathBuf::from(php)]
    } else {
        let exts: &[&str] = if cfg!(windows) {
            &["", ".exe", ".bat", ".cmd"]
        } else {
            &[""]
        };
        std::env::var_os("PATH")
            .map(|p| {
                std::env::split_paths(&p)
                    .flat_map(|dir| exts.iter().map(move |e| dir.join(format!("{php}{e}"))))
                    .collect()
            })
            .unwrap_or_default()
    };
    candidates
        .into_iter()
        .find(|c| is_executable(c))
        .and_then(|c| vivacity_core::pathutil::canonicalize(c).ok())
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn probe_cache_file() -> std::path::PathBuf {
    vivacity_core::platform::cache_dir().join("platform-probe.json")
}

/// Runs the probe on the current PHP (`VIVACITY_PHP` or `php`), through a
/// disk cache: a php process costs 30–60 ms, most of a no-op install.
///
/// The cache only exists for a php whose loaded files it can SEE (DECISIONS
/// 2026-10-09): the probe must report that it is the binary that was located,
/// and on unix the images it loaded must have been listed. A wrapper script,
/// a version-manager shim, a hardened-runtime binary that drops `DYLD_*`,
/// a system without `/proc`: the probe runs every time instead, which costs
/// 40 ms where a stale cache costs a lock Composer would not write.
/// `VIVACITY_NO_PLATFORM_CACHE=1` bypasses it outright.
pub fn probe() -> Result<Vec<Value>, PlatformError> {
    let php = std::env::var("VIVACITY_PHP").unwrap_or_else(|_| "php".to_owned());
    if std::env::var_os("VIVACITY_NO_PLATFORM_CACHE").is_some() {
        return run_probe(&php).map(|r| r.entries);
    }
    probe_at(&php, &probe_cache_file())
}

/// `probe` with the php and the cache file as arguments: the seam the tests
/// use to drive a fake php against a cache of their own.
fn probe_at(php: &str, cache_file: &std::path::Path) -> Result<Vec<Value>, PlatformError> {
    let Some(php_path) = locate_php(php) else {
        return run_probe(php).map(|r| r.entries);
    };
    let php_path = php_path.to_string_lossy().into_owned();
    let php_id = FileId::of(&php_path);
    let env = probe_env();
    let script = script_hash();
    if let Ok(bytes) = std::fs::read(cache_file) {
        if let Ok(cached) = serde_json::from_slice::<ProbeCache>(&bytes) {
            if cached.format == CACHE_FORMAT
                && cached.script == script
                && cached.php == php_id
                && cached.env == env
                && cached.inis.iter().all(|f| FileId::of(&f.path) == *f)
                && cached.images.iter().all(|f| FileId::of(&f.path) == *f)
                && cached.links.iter().all(|(l, t)| read_link(l) == *t)
                && cached.markers == system_markers()
            {
                return Ok(cached.probed);
            }
        }
    }
    let before = race_snapshot(&php_path);
    let run = run_probe(php)?;
    let Some(images) = run.loaded_files() else {
        return Ok(run.entries);
    };
    // The php that answered must be the php that was located: a wrapper or a
    // shim that picks another binary would make the key watch the wrong file.
    if run.binary.as_deref() != Some(php_path.as_str()) {
        return Ok(run.entries);
    }
    if race_snapshot(&php_path) != before {
        return Ok(run.entries);
    }
    let cache = ProbeCache {
        format: CACHE_FORMAT,
        script,
        php: php_id,
        inis: ini_dependencies(&run.entries),
        env,
        links: keg_links(&images.iter().map(|f| f.path.clone()).collect::<Vec<_>>()),
        images,
        markers: system_markers(),
        probed: run.entries.clone(),
    };
    if let Some(dir) = cache_file.parent() {
        if std::fs::create_dir_all(dir).is_ok() {
            if let Ok(json) = serde_json::to_vec(&cache) {
                let tmp = cache_file.with_extension(format!("json.{}.tmp", std::process::id()));
                if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, cache_file).is_err()
                {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
    }
    Ok(run.entries)
}

/// One real run of the probe: Composer's entries, and what this php loaded —
/// the key's raw material, never the resolution's.
struct ProbeRun {
    entries: Vec<Value>,
    binary: Option<String>,
    #[cfg_attr(not(windows), allow(dead_code))]
    ext_dir: Option<String>,
    /// The images the process mapped (`/proc/self/maps` on Linux, the dyld
    /// trace on macOS), or `None` when they could not be seen.
    #[cfg_attr(windows, allow(dead_code))]
    images: Option<Vec<String>>,
}

impl ProbeRun {
    /// The files whose identity the cache holds, or `None` when this php's
    /// loaded files cannot be seen — and then nothing is cached. On unix, the
    /// images that exist on disk (the dyld shared cache is not on disk as
    /// files: it is a marker of its own). On Windows, where no image list
    /// exists, the DLLs next to `php.exe` and in `extension_dir` — the PHP
    /// distribution ships its libraries there — read with `read_dir`, whose
    /// entries carry their metadata without opening each file.
    fn loaded_files(&self) -> Option<Vec<FileId>> {
        #[cfg(unix)]
        {
            let images = self.images.as_ref()?;
            let mut files: Vec<FileId> = images
                .iter()
                .map(|p| FileId::of(p))
                .filter(FileId::exists)
                .collect();
            files.sort_by(|a, b| a.path.cmp(&b.path));
            files.dedup_by(|a, b| a.path == b.path);
            (!files.is_empty()).then_some(files)
        }
        #[cfg(not(unix))]
        {
            let binary = self.binary.as_ref()?;
            let mut dirs: Vec<std::path::PathBuf> = Vec::new();
            if let Some(parent) = std::path::Path::new(binary).parent() {
                dirs.push(parent.to_path_buf());
            }
            if let Some(ext) = self.ext_dir.as_deref().filter(|d| !d.is_empty()) {
                dirs.push(std::path::PathBuf::from(ext));
            }
            let mut files = Vec::new();
            for dir in dirs {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    if name.ends_with(".dll") || name.ends_with(".exe") {
                        files.push(FileId::from_meta(
                            &e.path().to_string_lossy(),
                            e.metadata().ok(),
                        ));
                    }
                }
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            Some(files)
        }
    }
}

/// The images dyld reports loading under `DYLD_PRINT_LIBRARIES=1`, in either
/// format seen: `dyld[pid]: <UUID> /path` (macOS 12 and later, measured on 26)
/// and `dyld: loaded: /path` (earlier). Paths may contain spaces.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn dyld_images(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|line| {
            if let Some(rest) = line.strip_prefix("dyld: loaded: ") {
                return Some(rest.to_owned());
            }
            let rest = line.strip_prefix("dyld[")?;
            let (_, rest) = rest.split_once("]: ")?;
            let rest = rest.strip_prefix('<')?;
            let (_, path) = rest.split_once("> ")?;
            path.starts_with('/').then(|| path.to_owned())
        })
        .collect()
}

/// stderr without the dyld trace, for an error message a person reads.
fn without_dyld(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.starts_with("dyld[") && !l.starts_with("dyld: "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One real run of assets/platform-probe.php.
fn run_probe(php: &str) -> Result<ProbeRun, PlatformError> {
    let dir = tempfile::tempdir().map_err(|e| PlatformError(e.to_string()))?;
    let script = dir.path().join("platform-probe.php");
    std::fs::write(&script, PROBE).map_err(|e| PlatformError(e.to_string()))?;
    let mut cmd = Command::new(php);
    cmd.arg(&script);
    // The images this php loads, at no cost on a run that happens anyway
    // (measured: 30 ms with or without). Stripped by SIP behind `/bin/sh` or
    // `/usr/bin/env` — measured on a wrapper script: 0 images — which is the
    // case `loaded_files` turns into "do not cache".
    #[cfg(target_os = "macos")]
    cmd.env("DYLD_PRINT_LIBRARIES", "1");
    let out = cmd
        .output()
        .map_err(|e| PlatformError(format!("cannot run {php}: {e}")))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(PlatformError(format!(
            "platform probe failed: {}",
            without_dyld(&stderr)
        )));
    }
    let doc: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| PlatformError(format!("platform probe output: {e}")))?;
    let entries = match doc.get("entries") {
        Some(Value::Array(a)) => a.clone(),
        _ => {
            return Err(PlatformError(
                "platform probe output: no `entries` array".to_owned(),
            ))
        }
    };
    let text = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_owned);
    #[cfg(target_os = "macos")]
    let images = {
        let seen = dyld_images(&stderr);
        (!seen.is_empty()).then_some(seen)
    };
    #[cfg(not(target_os = "macos"))]
    let images = doc.get("maps").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    Ok(ProbeRun {
        entries,
        binary: text("binary"),
        ext_dir: text("ext_dir"),
        images,
    })
}

struct Override {
    name: String,
    version: Option<String>,
}

/// The ordered list of platform packages (`getPackages()`), `probed` being
/// the probe output and `overrides` `config.platform`.
pub fn platform_packages(
    probed: &[Value],
    overrides_cfg: &Map<String, Value>,
) -> Result<Vec<Package>, PlatformError> {
    // Insertion order of `config.platform` (PHP array), not sorted.
    let mut overrides: Vec<(String, Override)> = Vec::new();
    for (name, version) in overrides_cfg {
        let v = match version {
            Value::String(s) => Some(s.clone()),
            Value::Bool(false) => None,
            other => {
                return Err(PlatformError(format!(
                    "config.platform.{name} should be a string or false, but got {other}"
                )))
            }
        };
        if name == "php" && v.is_none() {
            return Err(PlatformError(
                "config.platform.php cannot be set to false as you cannot disable php entirely."
                    .into(),
            ));
        }
        let key = name.to_lowercase();
        let o = Override {
            name: name.clone(),
            version: v,
        };
        match overrides.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = o,
            None => overrides.push((key, o)),
        }
    }
    let override_of = |name: &str| overrides.iter().find(|(k, _)| k == name).map(|(_, o)| o);

    let mut packages: Vec<Package> = Vec::new();
    let mut libraries: BTreeMap<String, bool> = BTreeMap::new();

    // addOverriddenPackage
    let add_overridden = |packages: &mut Vec<Package>,
                          o: &Override,
                          name: Option<&str>|
     -> Result<(), PlatformError> {
        let Some(pretty) = &o.version else {
            return Ok(());
        };
        let version = normalize(pretty, None).map_err(|e| PlatformError(e.0))?;
        let mut p = Package::new(name.unwrap_or(&o.name), &version, pretty, Origin::Platform);
        p.raw = serde_json::json!({"description": "Package overridden via config.platform", "extra": {"config.platform": true}});
        packages.push(p);
        Ok(())
    };
    for (_, o) in &overrides {
        if !is_platform_package(&o.name) {
            return Err(PlatformError(format!(
                "Invalid platform package name in config.platform: {}",
                o.name
            )));
        }
        if o.version.is_some() {
            add_overridden(&mut packages, o, None)?;
        }
    }
    // The actual version is appended to the override's description
    // (`, same as actual` / `, actual: x.y.z`).
    let note_actual = |packages: &mut [Package], actual: &Package| {
        if let Some(over) = packages.iter_mut().find(|q| q.name == actual.name) {
            let text = if actual.version == over.version {
                "same as actual".to_owned()
            } else {
                format!("actual: {}", actual.pretty_version)
            };
            let description = over
                .raw
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            over.raw["description"] = serde_json::Value::String(format!("{description}, {text}"));
        }
    };
    // addPackage
    let add = |packages: &mut Vec<Package>, p: Package| -> Result<(), PlatformError> {
        if let Some(o) = override_of(&p.name) {
            if o.version.is_none() {
                return Ok(()); // disabled
            }
            note_actual(packages, &p);
            return Ok(()); // already added by the override
        }
        if let Some(php) = override_of("php") {
            if p.name.starts_with("php-") {
                add_overridden(packages, php, Some(&p.pretty_name))?;
                note_actual(packages, &p);
                return Ok(());
            }
        }
        packages.push(p);
        Ok(())
    };
    let simple = |name: &str, pretty: &str, description: &str| -> Result<Package, PlatformError> {
        let version = normalize(pretty, None).map_err(|e| PlatformError(e.0))?;
        let mut p = Package::new(name, &version, pretty, Origin::Platform);
        p.raw = serde_json::json!({"description": description});
        Ok(p)
    };
    add(
        &mut packages,
        simple("composer", COMPOSER_VERSION, "Composer package")?,
    )?;
    add(
        &mut packages,
        simple(
            "composer-plugin-api",
            PLUGIN_API_VERSION,
            "The Composer Plugin API",
        )?,
    )?;
    add(
        &mut packages,
        simple(
            "composer-runtime-api",
            RUNTIME_API_VERSION,
            "The Composer Runtime API",
        )?,
    )?;

    let entries: Vec<Probed> = probed
        .iter()
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| PlatformError(format!("probe entry: {e}")))
        })
        .collect::<Result<_, _>>()?;

    static PHP_FALLBACK: OnceLock<Regex> = OnceLock::new();
    static EXT_FALLBACK: OnceLock<Regex> = OnceLock::new();
    for e in &entries {
        match e.kind.as_str() {
            "php" => {
                let (version, pretty) = match normalize(&e.version, None) {
                    Ok(v) => (v, e.version.clone()),
                    Err(_) => {
                        let re = regex(&PHP_FALLBACK, r"^([^~+-]+).*$", false);
                        let pretty = match re.captures(e.version.as_bytes()) {
                            Ok(Some(caps)) => group(&caps, 1).to_owned(),
                            _ => e.version.clone(),
                        };
                        let v = normalize(&pretty, None).map_err(|x| PlatformError(x.0))?;
                        (v, pretty)
                    }
                };
                let mut p = Package::new(&e.name, &version, &pretty, Origin::Platform);
                p.raw = serde_json::json!({"description": e.description});
                add(&mut packages, p)?;
            }
            "ext" => {
                let mut extra_description = String::new();
                let (version, pretty) = match normalize(&e.version, None) {
                    Ok(v) => (v, e.version.clone()),
                    Err(_) => {
                        extra_description = format!(" (actual version: {})", e.version);
                        let re = regex(&EXT_FALLBACK, r"^(\d+\.\d+\.\d+(?:\.\d+)?)", false);
                        let pretty = match re.captures(e.version.as_bytes()) {
                            Ok(Some(caps)) => group(&caps, 1).to_owned(),
                            _ => "0".to_owned(),
                        };
                        let v = normalize(&pretty, None).map_err(|x| PlatformError(x.0))?;
                        (v, pretty)
                    }
                };
                let package_name = format!("ext-{}", e.name.to_lowercase().replace(' ', "-"));
                let mut p = Package::new(&package_name, &version, &pretty, Origin::Platform);
                p.package_type = "php-ext".to_owned();
                p.raw = serde_json::json!({"description": format!("The {} PHP extension{extra_description}", e.name)});
                if e.name == "uuid" {
                    p.replaces.insert(Link::new(
                        "ext-uuid",
                        "lib-uuid",
                        Constraint::new(Op::Eq, version.clone()),
                        &pretty,
                        LinkType::Replace,
                    ));
                }
                add(&mut packages, p)?;
            }
            "lib" => {
                let Ok(version) = normalize(&e.version, None) else {
                    continue;
                };
                let lib_name = format!("lib-{}", e.name);
                if !is_platform_package(&lib_name) || libraries.contains_key(&lib_name) {
                    continue;
                }
                libraries.insert(lib_name.clone(), true);
                let description = e
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("The {} library", e.name));
                let mut p = Package::new(&lib_name, &version, &e.version, Origin::Platform);
                p.raw = serde_json::json!({"description": description});
                let mut replaces = Links::default();
                for r in &e.replaces {
                    let r = r.to_lowercase();
                    // PHP key = bare name, target = `lib-<name>` (addLibrary).
                    replaces.insert(Link {
                        key: Some(r.clone()),
                        source: lib_name.clone(),
                        target: format!("lib-{r}"),
                        constraint: Constraint::new(Op::Eq, version.clone()),
                        pretty_constraint: e.version.clone(),
                        kind: LinkType::Replace,
                    });
                }
                let mut provides = Links::default();
                for pr in &e.provides {
                    let pr = pr.to_lowercase();
                    provides.insert(Link {
                        key: Some(pr.clone()),
                        source: lib_name.clone(),
                        target: format!("lib-{pr}"),
                        constraint: Constraint::new(Op::Eq, version.clone()),
                        pretty_constraint: e.version.clone(),
                        kind: LinkType::Provide,
                    });
                }
                p.replaces = replaces;
                p.provides = provides;
                add(&mut packages, p)?;
            }
            _ => {}
        }
    }
    Ok(packages)
}

/// `install`'s platform verification on the full platform repository
/// (`PlatformRepository`: php, extensions, `lib-*` libraries, the
/// composer-*-api packages, `config.platform` overrides), the way
/// `Installer::doInstall` checks the lock: every platform requirement of
/// the lock's `platform`/`platform-dev` sections and of the wanted
/// packages must be satisfied by a platform package's name, or by one of
/// its `provide`/`replace` links (`lib-dom-libxml` through `lib-libxml`,
/// `php-64bit` through `php`).
pub fn check_install(
    lock: &vivacity_core::lock::Lock,
    platform: &[Package],
    with_dev: bool,
    ignored: &[String],
) -> Vec<vivacity_core::platform::PlatformFailure> {
    use vivacity_core::platform::{FailureReason, PlatformFailure};
    let mut reqs: Vec<(String, String, Option<String>)> = Vec::new();
    for (name, cons) in &lock.platform {
        reqs.push((name.clone(), cons.clone(), None));
    }
    if with_dev {
        for (name, cons) in &lock.platform_dev {
            reqs.push((name.clone(), cons.clone(), None));
        }
    }
    for p in lock.wanted_packages(with_dev) {
        if let Some(require) = p.raw.get("require").and_then(Value::as_object) {
            for (name, cons) in require {
                let lname = name.to_ascii_lowercase();
                if is_platform_package(&lname) {
                    if let Some(c) = cons.as_str() {
                        reqs.push((lname, c.to_owned(), Some(p.name().to_owned())));
                    }
                }
            }
        }
    }
    let mut failures = Vec::new();
    for (requirement, cons, required_by) in reqs {
        if vivacity_core::platform::is_ignored(&requirement, ignored) {
            continue;
        }
        // The package itself, else a link providing/replacing the name.
        let mut candidates: Vec<(String, String)> = Vec::new();
        for p in platform {
            if p.name == requirement {
                candidates.push((p.version.clone(), p.pretty_version.clone()));
            }
            for link in p.provides.iter().chain(p.replaces.iter()) {
                if link.target == requirement {
                    let v = match &link.constraint {
                        Constraint::Single {
                            op: Op::Eq,
                            version,
                        } => version.clone(),
                        _ => continue,
                    };
                    candidates.push((v, link.pretty_constraint.clone()));
                }
            }
        }
        if candidates.is_empty() {
            failures.push(PlatformFailure {
                requirement,
                constraint: cons,
                required_by,
                reason: FailureReason::Missing,
            });
            continue;
        }
        let Ok(parsed) = crate::constraint::parse_constraints(&cons) else {
            failures.push(PlatformFailure {
                requirement,
                constraint: cons,
                required_by,
                reason: FailureReason::Unsupported,
            });
            continue;
        };
        if !candidates
            .iter()
            .any(|(v, _)| parsed.constraint.matches_version(v))
        {
            failures.push(PlatformFailure {
                requirement,
                constraint: cons,
                required_by,
                reason: FailureReason::Mismatch {
                    installed: candidates[0].1.trim_start_matches("== ").to_owned(),
                },
            });
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    /// A fake `php` whose answer depends on a library reached through a
    /// Homebrew-style link (`opt/pcre2 -> ../Cellar/pcre2/<keg>`), the shape
    /// measured on 2026-10-08: the old keg stays on disk, only the link moves.
    /// It speaks the probe's format — its entries, its own path as `binary`,
    /// and the library it "loaded" both as `maps` (Linux's channel) and as a
    /// dyld trace line on stderr (macOS's) — and appends a line to `count` on
    /// every launch, so a test can tell a cache hit from a probe.
    #[cfg(unix)]
    fn fake_php(root: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        for (keg, v) in [("a", "10.47"), ("b", "10.49")] {
            let lib = root.join("Cellar/pcre2").join(keg).join("lib");
            std::fs::create_dir_all(&lib).expect("mkdir");
            std::fs::write(lib.join("libpcre2-8.0.dylib"), v).expect("lib");
        }
        std::fs::create_dir_all(root.join("opt")).expect("mkdir");
        std::os::unix::fs::symlink("../Cellar/pcre2/a", root.join("opt/pcre2")).expect("link");
        let php = root.join("php");
        std::fs::write(
            &php,
            format!(
                "#!/bin/sh\n\
                 echo x >> '{root}/count'\n\
                 lib=\"$(cd '{root}/opt/pcre2/lib' && pwd -P)/libpcre2-8.0.dylib\"\n\
                 v=$(cat \"$lib\")\n\
                 echo \"dyld[1]: <00000000-0000-0000-0000-000000000000> $lib\" >&2\n\
                 printf '{{\"entries\":[{{\"kind\":\"lib\",\"name\":\"pcre\",\"version\":\"%s\",\"description\":\"x\"}}],\"binary\":\"{php}\",\"ext_dir\":\"\",\"maps\":[\"%s\"]}}' \"$v\" \"$lib\"\n",
                root = root.display(),
                php = root.join("php").display()
            ),
        )
        .expect("php");
        std::fs::set_permissions(&php, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        php
    }

    #[cfg(unix)]
    fn launches(root: &std::path::Path) -> usize {
        std::fs::read_to_string(root.join("count"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    #[cfg(unix)]
    fn pcre_of(probed: &[Value]) -> String {
        probed
            .iter()
            .find(|v| v.get("name").and_then(Value::as_str) == Some("pcre"))
            .and_then(|v| v.get("version").and_then(Value::as_str))
            .unwrap_or("?")
            .to_owned()
    }

    /// The known issue of 0.20.0, reproduced without Homebrew: the link is
    /// repointed to a new keg while the old keg stays untouched, and the cache
    /// must notice.
    #[test]
    #[cfg(unix)]
    fn a_repointed_library_invalidates_the_cache() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = vivacity_core::pathutil::canonicalize(dir.path()).expect("canon");
        let php = fake_php(&root);
        let cache = root.join("platform-probe.json");
        let php = php.to_string_lossy().into_owned();
        assert_eq!(pcre_of(&probe_at(&php, &cache).expect("probe")), "10.47");
        assert_eq!(launches(&root), 1);
        // Same link target: a hit, no second launch — the cache still works.
        assert_eq!(pcre_of(&probe_at(&php, &cache).expect("probe")), "10.47");
        assert_eq!(launches(&root), 1, "an unchanged system must hit the cache");
        // `brew upgrade pcre2`: the link moves, the old keg is left as it was.
        std::fs::remove_file(root.join("opt/pcre2")).expect("rm link");
        std::os::unix::fs::symlink("../Cellar/pcre2/b", root.join("opt/pcre2")).expect("link");
        assert_eq!(
            pcre_of(&probe_at(&php, &cache).expect("probe")),
            "10.49",
            "the cache served the old keg's version after the link moved"
        );
        assert_eq!(launches(&root), 2);
    }

    /// A cache written by 0.20.0 or earlier has no `format`, and must not
    /// validate — not by a deserialisation that happens to fail, but by a
    /// comparison: every field is present here except `format`'s value.
    #[test]
    #[cfg(unix)]
    fn a_cache_of_an_older_format_never_validates() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = vivacity_core::pathutil::canonicalize(dir.path()).expect("canon");
        let php = fake_php(&root).to_string_lossy().into_owned();
        let cache = root.join("platform-probe.json");
        probe_at(&php, &cache).expect("probe");
        assert_eq!(launches(&root), 1);
        let mut doc: Value =
            serde_json::from_slice(&std::fs::read(&cache).expect("read")).expect("json");
        doc["format"] = Value::from(CACHE_FORMAT - 1);
        std::fs::write(&cache, serde_json::to_vec(&doc).expect("json")).expect("write");
        probe_at(&php, &cache).expect("probe");
        assert_eq!(
            launches(&root),
            2,
            "an older format must send the probe again"
        );
        // And a file without the field at all — 0.20.0's shape — likewise.
        doc.as_object_mut().expect("object").remove("format");
        std::fs::write(&cache, serde_json::to_vec(&doc).expect("json")).expect("write");
        probe_at(&php, &cache).expect("probe");
        assert_eq!(launches(&root), 3);
    }

    /// A library replaced IN PLACE (apt, dnf): same path, new content. The
    /// image's own identity has to move the key.
    #[test]
    #[cfg(unix)]
    fn a_library_replaced_in_place_invalidates_the_cache() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = vivacity_core::pathutil::canonicalize(dir.path()).expect("canon");
        let php = fake_php(&root).to_string_lossy().into_owned();
        let cache = root.join("platform-probe.json");
        assert_eq!(pcre_of(&probe_at(&php, &cache).expect("probe")), "10.47");
        // Rewritten through a new inode, as a package manager does.
        let lib = root.join("Cellar/pcre2/a/lib/libpcre2-8.0.dylib");
        let tmp = lib.with_extension("new");
        std::fs::write(&tmp, "10.48").expect("write");
        std::fs::rename(&tmp, &lib).expect("rename");
        assert_eq!(pcre_of(&probe_at(&php, &cache).expect("probe")), "10.48");
    }

    /// Fail closed: a php that cannot be seen — here a `#!/bin/sh` wrapper,
    /// which drops `DYLD_*` on macOS and reports another binary than the one
    /// located everywhere — is probed on EVERY call, never cached. Measured
    /// before this rule existed: such a wrapper kept the 0.20.0 bug intact.
    #[test]
    #[cfg(unix)]
    fn a_php_behind_a_wrapper_is_never_cached() {
        use std::os::unix::fs::PermissionsExt as _;
        let Some(real) = locate_php("php") else {
            panic!("this test needs a real php on PATH (dev and CI have one)");
        };
        let dir = tempfile::tempdir().expect("tmp");
        let root = vivacity_core::pathutil::canonicalize(dir.path()).expect("canon");
        let wrapper = root.join("php");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\necho x >> '{}/count'\nexec '{}' \"$@\"\n",
                root.display(),
                real.display()
            ),
        )
        .expect("wrapper");
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let cache = root.join("platform-probe.json");
        let w = wrapper.to_string_lossy().into_owned();
        let first = probe_at(&w, &cache).expect("probe");
        let second = probe_at(&w, &cache).expect("probe");
        assert_eq!(
            launches(&root),
            2,
            "a wrapper must not be served from the cache"
        );
        assert_eq!(first, second);
        assert!(
            !cache.exists(),
            "nothing may be cached for a php we cannot see"
        );
    }

    /// The real php, directly: its images are seen, they include its own binary
    /// and the PCRE2 library behind `lib-pcre`, and a cached answer is the
    /// uncached one — the sentinel the 0.20.0 known issue lacked.
    #[test]
    #[cfg(unix)]
    fn the_real_php_is_seen_and_its_cache_matches_a_fresh_probe() {
        let Some(real) = locate_php("php") else {
            panic!("this test needs a real php on PATH (dev and CI have one)");
        };
        let real = real.to_string_lossy().into_owned();
        let run = run_probe(&real).expect("probe");
        assert_eq!(run.binary.as_deref(), Some(real.as_str()));
        let files = run
            .loaded_files()
            .expect("the images of a direct php are seen");
        assert!(
            files.iter().any(|f| f.path == real),
            "the binary itself is among the images"
        );
        assert!(
            files.iter().any(|f| f.path.contains("pcre2")),
            "the PCRE2 library is among the images: {:?}",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
        let dir = tempfile::tempdir().expect("tmp");
        let cache = dir.path().join("platform-probe.json");
        let cold = probe_at(&real, &cache).expect("probe");
        assert!(cache.exists(), "a php we can see is cached");
        let warm = probe_at(&real, &cache).expect("probe");
        assert_eq!(cold, warm);
        assert_eq!(
            warm, run.entries,
            "the cache serves exactly what a fresh probe says"
        );
    }

    /// Both trace formats dyld has used, and a path with a space.
    #[test]
    fn dyld_trace_lines_are_read_in_both_formats() {
        let stderr = "dyld[123]: <0729711C-3D0D-3112-9FE8-5BAEC75F9439> /opt/homebrew/Cellar/php/8.5.10/bin/php\n\
                      dyld: loaded: /usr/local/lib/libfoo.dylib\n\
                      dyld[123]: <AB> /Users/x/Library/Application Support/lib.dylib\n\
                      dyld[123]: move loaded to delayed: Montreal\n\
                      PHP Warning: something\n";
        assert_eq!(
            dyld_images(stderr),
            vec![
                "/opt/homebrew/Cellar/php/8.5.10/bin/php".to_owned(),
                "/usr/local/lib/libfoo.dylib".to_owned(),
                "/Users/x/Library/Application Support/lib.dylib".to_owned(),
            ]
        );
        assert_eq!(without_dyld(stderr), "PHP Warning: something");
    }

    /// The links the key watches are the Homebrew `opt` links of the kegs the
    /// images live in, once each.
    #[test]
    fn keg_links_name_the_opt_link_of_each_formula() {
        let images = vec![
            "/opt/homebrew/Cellar/pcre2/10.49/lib/libpcre2-8.0.dylib".to_owned(),
            "/opt/homebrew/Cellar/pcre2/10.49/lib/libpcre2-posix.dylib".to_owned(),
            "/opt/homebrew/Cellar/icu4c@78/78.3/lib/libicuuc.78.3.dylib".to_owned(),
            "/usr/lib/libz.1.dylib".to_owned(),
        ];
        let links: Vec<String> = keg_links(&images).into_iter().map(|(l, _)| l).collect();
        assert_eq!(
            links,
            vec![
                "/opt/homebrew/opt/icu4c@78".to_owned(),
                "/opt/homebrew/opt/pcre2".to_owned()
            ]
        );
    }

    use super::*;

    /// A probed platform: PHP 8.2.5 (64-bit) with mbstring and intl.
    fn probed() -> Vec<Value> {
        vec![
            serde_json::json!({"kind": "php", "name": "php", "version": "8.2.5", "description": "The PHP interpreter"}),
            serde_json::json!({"kind": "php", "name": "php-64bit", "version": "8.2.5", "description": "x"}),
            serde_json::json!({"kind": "ext", "name": "mbstring", "version": "8.2.5"}),
            serde_json::json!({"kind": "ext", "name": "intl", "version": "8.2.5"}),
        ]
    }

    fn platform_with(overrides: Value) -> Vec<Package> {
        let empty = Map::new();
        let overrides = overrides.as_object().unwrap_or(&empty).clone();
        platform_packages(&probed(), &overrides).expect("platform")
    }

    fn lock(platform: Value, packages: Value) -> vivacity_core::lock::Lock {
        vivacity_core::lock::Lock::parse(
            &serde_json::json!({ "packages": packages, "platform": platform }).to_string(),
        )
        .expect("lock")
    }

    #[test]
    fn satisfied_lock_passes() {
        let l = lock(
            serde_json::json!({"php": ">=8.1", "ext-mbstring": "*"}),
            serde_json::json!([{"name":"a/b","version":"1.0","require":{"php":"^8.0","ext-intl":"*"}}]),
        );
        assert!(check_install(&l, &platform_with(Value::Null), true, &[]).is_empty());
    }

    #[test]
    fn reports_mismatch_and_missing() {
        use vivacity_core::platform::FailureReason;
        let l = lock(
            serde_json::json!({"php": ">=8.3", "ext-gd": "*", "lib-icu": ">=70"}),
            serde_json::json!([]),
        );
        let f = check_install(&l, &platform_with(Value::Null), true, &[]);
        assert_eq!(f.len(), 3, "{f:?}");
        assert_eq!(
            f[0].reason,
            FailureReason::Mismatch {
                installed: "8.2.5".to_owned()
            }
        );
        assert_eq!(f[1].reason, FailureReason::Missing);
        assert_eq!(f[2].reason, FailureReason::Missing);
    }

    #[test]
    fn package_requirements_are_checked_and_attributed() {
        let l = lock(
            serde_json::json!({}),
            serde_json::json!([{"name":"a/b","version":"1.0","require":{"php":">=8.3","some/dep":"^1.0"}}]),
        );
        let f = check_install(&l, &platform_with(Value::Null), true, &[]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].required_by.as_deref(), Some("a/b"));
        assert_eq!(f[0].requirement, "php");
    }

    #[test]
    fn ignore_patterns_work() {
        let l = lock(
            serde_json::json!({"php": ">=9.0", "ext-gd": "*"}),
            serde_json::json!([]),
        );
        let p = platform_with(Value::Null);
        assert_eq!(check_install(&l, &p, true, &[]).len(), 2);
        assert!(check_install(&l, &p, true, &["*".to_owned()]).is_empty());
        assert_eq!(check_install(&l, &p, true, &["ext-*".to_owned()]).len(), 1);
        assert_eq!(check_install(&l, &p, true, &["php".to_owned()]).len(), 1);
        assert_eq!(
            check_install(&l, &p, true, &["ext-gd+".to_owned()]).len(),
            1
        );
    }

    #[test]
    fn config_platform_overrides_apply_to_the_check() {
        let l = lock(
            serde_json::json!({"php": ">=8.3", "ext-gd": "*", "ext-intl": "*"}),
            serde_json::json!([]),
        );
        let p = platform_with(
            serde_json::json!({"php": "8.3.0", "ext-gd": "8.3.0", "ext-intl": false}),
        );
        let f = check_install(&l, &p, true, &[]);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].requirement, "ext-intl");
    }

    #[test]
    fn probe_cache_watches_every_ini_file_and_the_scan_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ini = dir.path().join("php.ini");
        std::fs::write(&ini, "memory_limit=1G\n").expect("write");
        let scan = dir.path().join("conf.d");
        std::fs::create_dir(&scan).expect("mkdir");
        let ext = scan.join("ext-mbstring.ini");
        std::fs::write(&ext, "extension=mbstring\n").expect("write");
        let probed = vec![serde_json::json!({
            "kind": "ini", "name": "ini", "version": "",
            "loaded": ini.to_string_lossy(),
            "scanned": format!("{},\n{}", ext.to_string_lossy(), ext.to_string_lossy()),
            "scan_dir": scan.to_string_lossy(),
        })];
        let deps = ini_dependencies(&probed);
        let paths: Vec<&str> = deps.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                ini.to_string_lossy().as_ref(),
                scan.to_string_lossy().as_ref(),
                ext.to_string_lossy().as_ref(),
                ext.to_string_lossy().as_ref()
            ]
        );
        assert!(deps
            .iter()
            .all(|f| f.mtime_ns.is_some() && f.size.is_some()));
        // Unchanged: every id still matches.
        assert!(deps.iter().all(|f| FileId::of(&f.path) == *f));
        // A file edited (size changes) or removed invalidates.
        std::fs::write(&ext, "extension=mbstring\nextension=intl\n").expect("write");
        assert!(deps.iter().any(|f| FileId::of(&f.path) != *f));
        std::fs::remove_file(&ext).expect("rm");
        assert_eq!(FileId::of(&ext.to_string_lossy()).mtime_ns, None);
        // No loaded php.ini ('' in the probe): nothing to watch for it, the
        // scan dir still is.
        let probed = vec![
            serde_json::json!({"kind": "ini", "loaded": "", "scanned": null,
                                            "scan_dir": scan.to_string_lossy()}),
        ];
        assert_eq!(ini_dependencies(&probed).len(), 1);
    }

    #[test]
    fn locate_php_resolves_an_explicit_path_only_when_executable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plain = dir.path().join("php");
        std::fs::write(&plain, "").expect("write");
        #[cfg(unix)]
        {
            assert_eq!(locate_php(&plain.to_string_lossy()), None);
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        assert!(locate_php(&plain.to_string_lossy()).is_some());
        assert_eq!(
            locate_php(&dir.path().join("missing").to_string_lossy()),
            None
        );
    }

    #[test]
    fn platform_names() {
        assert!(is_platform_package("php"));
        assert!(is_platform_package("ext-mbstring"));
        assert!(is_platform_package("lib-icu-cldr"));
        assert!(is_platform_package("composer-plugin-api"));
        assert!(!is_platform_package("php-http/discovery"));
        assert!(!is_platform_package("ext-"));
    }

    #[test]
    fn builds_from_a_probe_with_overrides() {
        let probed = vec![
            serde_json::json!({"kind": "php", "name": "php", "version": "8.5.10", "description": "The PHP interpreter"}),
            serde_json::json!({"kind": "php", "name": "php-64bit", "version": "8.5.10", "description": "x"}),
            serde_json::json!({"kind": "ext", "name": "Zend OPcache", "version": "8.5.10"}),
            serde_json::json!({"kind": "ext", "name": "weird", "version": "not a version"}),
            serde_json::json!({"kind": "lib", "name": "libsodium", "version": "1.0.20", "replaces": [], "provides": []}),
            serde_json::json!({"kind": "lib", "name": "libsodium", "version": "1.0.20", "replaces": [], "provides": []}),
            serde_json::json!({"kind": "lib", "name": "libxml", "version": "2.13.4", "description": "libxml library version", "replaces": [], "provides": ["dom-libxml"]}),
        ];
        let mut overrides = Map::new();
        overrides.insert("php".into(), Value::String("8.2.0".into()));
        overrides.insert("ext-weird".into(), Value::Bool(false));
        let pk = platform_packages(&probed, &overrides).unwrap();
        let names: Vec<&str> = pk.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "php",
                "composer",
                "composer-plugin-api",
                "composer-runtime-api",
                "php-64bit",
                "ext-zend-opcache",
                "lib-libsodium",
                "lib-libxml"
            ]
        );
        assert_eq!(pk[0].version, "8.2.0.0");
        assert_eq!(
            pk[4].version, "8.2.0.0",
            "php-64bit takes the overridden php version"
        );
        // PHP key without `lib-`, target with it (addLibrary).
        let link = pk[7].provides.get("dom-libxml").unwrap();
        assert_eq!(link.target, "lib-dom-libxml");
        assert_eq!(link.constraint.to_string(), "== 2.13.4.0");
        assert!(pk[7].provides.get("lib-dom-libxml").is_none());
        // Insertion order of the overrides, not alphabetical.
        let mut overrides = Map::new();
        overrides.insert("php".into(), Value::String("8.2.0".into()));
        overrides.insert("ext-mbstring".into(), Value::String("1.0".into()));
        overrides.insert("Ext-Json".into(), Value::String("2.0".into()));
        let pk = platform_packages(&probed, &overrides).unwrap();
        let names: Vec<&str> = pk.iter().take(3).map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["php", "ext-mbstring", "ext-json"]);
    }
}
