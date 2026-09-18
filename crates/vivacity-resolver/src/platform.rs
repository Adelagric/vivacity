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

/// Identity of a file the probe's result depends on: its path and, when
/// it exists, its mtime (nanoseconds) and size. An absent file is recorded
/// as absent, so its appearance invalidates the cache too.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FileId {
    path: String,
    mtime_ns: Option<i128>,
    size: Option<u64>,
}

impl FileId {
    fn of(path: &str) -> FileId {
        let meta = std::fs::metadata(path).ok();
        let mtime_ns = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_nanos() as i128)
        });
        FileId {
            path: path.to_owned(),
            mtime_ns,
            size: meta.map(|m| m.len()),
        }
    }
}

/// The environment the probe's output depends on (xdebug handling in the
/// probe, PHP's own ini discovery), captured by value.
const PROBE_ENV: &[&str] = &["PHPRC", "PHP_INI_SCAN_DIR", "XDEBUG_MODE", "XDEBUG_CONFIG"];

/// One cached probe: valid while the php binary, every ini file PHP read
/// (and the directory it scans), and the relevant environment are the same.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ProbeCache {
    php: FileId,
    inis: Vec<FileId>,
    env: Vec<(String, Option<String>)>,
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
/// disk cache: a php process costs 30–60 ms, most of a no-op install. The
/// cache is keyed on what the result depends on — the php binary (path,
/// mtime, size), every ini file PHP loaded or scanned plus the scan
/// directory, and PHPRC / PHP_INI_SCAN_DIR / XDEBUG_MODE / XDEBUG_CONFIG.
/// `VIVACITY_NO_PLATFORM_CACHE=1` bypasses it.
pub fn probe() -> Result<Vec<Value>, PlatformError> {
    let php = std::env::var("VIVACITY_PHP").unwrap_or_else(|_| "php".to_owned());
    if std::env::var_os("VIVACITY_NO_PLATFORM_CACHE").is_some() {
        return run_probe(&php);
    }
    let Some(php_path) = locate_php(&php) else {
        return run_probe(&php);
    };
    let php_id = FileId::of(&php_path.to_string_lossy());
    let env = probe_env();
    let cache_file = probe_cache_file();
    if let Ok(bytes) = std::fs::read(&cache_file) {
        if let Ok(cached) = serde_json::from_slice::<ProbeCache>(&bytes) {
            if cached.php == php_id
                && cached.env == env
                && cached.inis.iter().all(|f| FileId::of(&f.path) == *f)
            {
                return Ok(cached.probed);
            }
        }
    }
    let probed = run_probe(&php)?;
    let cache = ProbeCache {
        php: php_id,
        inis: ini_dependencies(&probed),
        env,
        probed: probed.clone(),
    };
    if let Some(dir) = cache_file.parent() {
        if std::fs::create_dir_all(dir).is_ok() {
            if let Ok(json) = serde_json::to_vec(&cache) {
                let tmp = cache_file.with_extension(format!("json.{}.tmp", std::process::id()));
                if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &cache_file).is_err()
                {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
    }
    Ok(probed)
}

/// One real run of assets/platform-probe.php.
fn run_probe(php: &str) -> Result<Vec<Value>, PlatformError> {
    let dir = tempfile::tempdir().map_err(|e| PlatformError(e.to_string()))?;
    let script = dir.path().join("platform-probe.php");
    std::fs::write(&script, PROBE).map_err(|e| PlatformError(e.to_string()))?;
    let out = Command::new(php)
        .arg(&script)
        .output()
        .map_err(|e| PlatformError(format!("cannot run {php}: {e}")))?;
    if !out.status.success() {
        return Err(PlatformError(format!(
            "platform probe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| PlatformError(format!("platform probe output: {e}")))
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
    use super::*;

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
