//! Emulation of `dealerdirect/phpcodesniffer-composer-installer`
//! (docs/reference/plugins/phpcodesniffer-composer-installer/ for v1.2.1,
//! `…-v0.7.2/` for v0.7.2, MIT): on `post-install-cmd` /
//! `post-update-cmd`, when `squizlabs/php_codesniffer` is installed, the
//! plugin registers every coding standard it finds as PHP_CodeSniffer's
//! `installed_paths`, through `phpcs --config-set installed_paths <a>,<b>`:
//!
//! - the search paths: the packages of type `phpcodesniffer-standard`
//!   (aliases excluded), and the project itself — always for v0.7.2
//!   (`$searchPaths = array($this->cwd)`), only when the root package is a
//!   standard for 1.x;
//! - `Finder::files()->name('ruleset.xml')` at a depth between `min` (0 for
//!   phpcs >= 3.0.0, 1 before) and `max` (`extra.phpcodesniffer-search-depth`,
//!   3 by default) below each search path; symfony/finder v5.4.45 as
//!   Composer 2.10.3 bundles it: VCS directories skipped, any path
//!   component starting with a dot skipped (`ignoreDotFiles`, matched on
//!   the path relative to the search path), symbolic links to directories
//!   not descended into (a search path that is a link is read), anything
//!   but a directory accepted as a file;
//! - a standard's path is the ruleset's directory, then its parent unless
//!   it is the project itself, made relative to the phpcs package with
//!   `findShortestPath(..., directories: true)`;
//! - the paths already registered are kept when their directory still
//!   exists, the new ones appended, the whole sorted (`sort()`, bytewise)
//!   and written by phpcs into `<phpcs>/CodeSniffer.conf`:
//!   `<?php\n $phpCodeSnifferConfig = ` + `var_export` + `;\n?>`;
//! - nothing happens when nothing changed, or when no search path exists;
//! - when it saves, `saveInstalledPaths` (Plugin.php:263-315) writes one
//!   line on stdout, "PHP CodeSniffer Config installed_paths set to <the
//!   sorted paths, comma-joined>" or "… installed_paths delete" — returned
//!   here as a [`ConfigChange`] for the caller to print.
//!
//! Versions (docs/plans/plugin-lines.md, docs/plans/chamilo-dealerdirect.md):
//! v0.7.2 and v1.0.0 to v1.2.0 keep a registered relative path that exists
//! relative to the working directory *or* to phpcs (`cleanInstalledPaths`'
//! `$alternativePath`); v1.2.1 resolves it against phpcs only. v0.7.2 reads
//! the configuration through the `phpcs` of the `PATH` (the bin-dir first,
//! `EventDispatcher::ensureBinDirIsInPath`) — the same `CodeSniffer.conf`.
//! The other 0.x and v1.1.0 (phpcs's path from `InstalledVersions`) are
//! handed over by `scope`, and so are the configurations [`scope_issue`]
//! names.

use crate::error::{Error, Result};
use crate::layout::Layout;
use crate::lock::LockPackage;
use crate::pathutil::{find_shortest_path, normalize_path};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const PLUGIN_NAME: &str = "dealerdirect/phpcodesniffer-composer-installer";
const PHPCS: &str = "squizlabs/php_codesniffer";
const STANDARD_TYPE: &str = "phpcodesniffer-standard";
const VCS_DIRS: &[&str] = &[
    ".svn",
    "_svn",
    "CVS",
    "_darcs",
    ".arch-params",
    ".monotone",
    ".bzr",
    ".git",
    ".hg",
];

/// What `saveInstalledPaths` did, for its line.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigChange {
    /// Nothing to save (or no phpcs): no line at the normal level.
    Unchanged,
    /// `--config-set installed_paths <paths>`.
    Set(String),
    /// `--config-delete installed_paths`: no path left.
    Deleted,
}

impl ConfigChange {
    /// The line `saveInstalledPaths` writes on success (the `<info>` and
    /// `<comment>` tags removed, as an undecorated output prints them).
    pub fn line(&self) -> Option<String> {
        match self {
            ConfigChange::Unchanged => None,
            ConfigChange::Set(paths) => Some(format!(
                "PHP CodeSniffer Config installed_paths set to {paths}"
            )),
            ConfigChange::Deleted => {
                Some("PHP CodeSniffer Config installed_paths delete".to_owned())
            }
        }
    }
}

/// What differs between the versions read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    /// The project is always a search path (v0.7.2), not only when the
    /// root package is a standard (1.x).
    pub search_project: bool,
    /// A registered relative path is kept when it exists relative to the
    /// working directory (`$alternativePath`, v0.7.2 and v1.0.0 to v1.2.0).
    pub cwd_relative: bool,
}

/// The [`Variant`] of a plugin version (one of `scope`'s versions read).
pub fn variant(plugin_version: &str) -> Variant {
    let below = |bound: &str| {
        crate::version::normalize_pretty(plugin_version)
            .ok()
            .and_then(|v| crate::version::Version::parse(&v).ok())
            .zip(crate::version::Version::parse(bound).ok())
            .is_some_and(|(v, b)| v < b)
    };
    Variant {
        search_project: below("1.0.0.0"),
        cwd_relative: below("1.2.1.0"),
    }
}

/// The configurations the emulation does not reproduce, for a plugin
/// version read, with phpcs's version (`None`: phpcs not installed, the
/// listener does nothing) and whether the lock aliases phpcs:
///
/// - phpcs not at a stable version, or aliased: `isPHPCodeSnifferInstalled
///   ('>= 3.0.0')` (the minimum depth) matches aliases and dev branches as
///   Composer's constraints do, which is not reproduced;
/// - phpcs < 3.0.0 under a version before v1.2.0 (v0.7.2 to v1.1.2):
///   `./scripts/phpcs` writes the configuration (not measured);
/// - `extra.phpcodesniffer-search-depth` present and not a JSON integer
///   greater than the minimum depth: `getMaxDepth` throws (Composer stops),
///   or casts a string, neither of which is reproduced.
pub fn scope_issue(
    plugin_version: &str,
    phpcs: Option<(&str, bool)>,
    root_manifest: &Value,
) -> Option<String> {
    let (phpcs_version, aliased) = phpcs?;
    let stable = crate::version::normalize_pretty(phpcs_version)
        .ok()
        .filter(|v| !v.contains('-'))
        .and_then(|v| crate::version::Version::parse(&v).ok());
    let Some(stable) = stable.filter(|_| !aliased) else {
        return Some(format!(
            "plugin {PLUGIN_NAME} with {PHPCS} {phpcs_version}{} (the minimum search depth follows Composer's constraint matching, not emulated)",
            if aliased { " aliased" } else { "" }
        ));
    };
    let three = crate::version::Version::parse("3.0.0.0").expect("version");
    let before_120 = crate::version::normalize_pretty(plugin_version)
        .ok()
        .and_then(|v| crate::version::Version::parse(&v).ok())
        .is_some_and(|v| v < crate::version::Version::parse("1.2.0.0").expect("version"));
    if before_120 && stable < three {
        return Some(format!(
            "plugin {PLUGIN_NAME} {plugin_version} with {PHPCS} {phpcs_version} (phpcs 2 writes its configuration through ./scripts/phpcs, not measured)"
        ));
    }
    let min_depth = if stable >= three { 0 } else { 1 };
    let depth = root_manifest
        .get("extra")
        .and_then(|e| e.get("phpcodesniffer-search-depth"))?;
    if depth.as_i64().is_some_and(|d| d > min_depth) {
        return None;
    }
    Some(format!(
        "plugin {PLUGIN_NAME} with extra.phpcodesniffer-search-depth {depth} (anything but an integer greater than {min_depth} is thrown or cast by the plugin, not emulated)"
    ))
}

/// v0.7.2 reads the configuration with `phpcs --config-show`, the `phpcs`
/// of the `PATH`, and the proxy in the bin-dir runs `#!/usr/bin/env php`.
/// `EventDispatcher::ensureBinDirIsInPath` puts the bin-dir (its realpath)
/// first only when it is not in the `PATH` already: when it is, a `phpcs`
/// in an entry before it answers instead (measured: Composer then writes
/// that phpcs's paths along with its own and prints "Failed to set …").
/// That case, and a `PATH` without `php`, are not reproduced (vivacity
/// reads `CodeSniffer.conf` itself). `path` is the `PATH` value.
pub fn path_issue(project_dir: &Path, root_manifest: &Value, path: &str) -> Option<String> {
    let windows = cfg!(windows);
    let sep = if windows { ';' } else { ':' };
    let names: &[&str] = if windows {
        &["phpcs.bat", "phpcs.exe", "phpcs.cmd", "phpcs"]
    } else {
        &["phpcs"]
    };
    let executable = |dir: &str, name: &str| {
        let f = Path::new(dir).join(name);
        std::fs::metadata(&f).is_ok_and(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                m.is_file()
            }
        })
    };
    let entries: Vec<&str> = path.split(sep).collect();
    let dirs = crate::dirs::Dirs::resolve(root_manifest).ok()?;
    let real_project = std::fs::canonicalize(project_dir).ok()?;
    let bin = normalize_path(&real_project.join(dirs.bin_rel()).to_string_lossy());
    if let Some(at) = entries.iter().position(|e| *e == bin) {
        if let Some(other) = entries[..at]
            .iter()
            .find(|e| names.iter().any(|n| executable(e, n)))
        {
            return Some(format!(
                "plugin {PLUGIN_NAME} v0.7.2 with the bin-dir already in the PATH after {other}, which holds another phpcs (the plugin would read that phpcs's configuration, not emulated)"
            ));
        }
    }
    let php = if windows { "php.exe" } else { "php" };
    if !entries.iter().any(|e| executable(e, php)) {
        return Some(format!(
            "plugin {PLUGIN_NAME} v0.7.2 without php in the PATH (the bin-dir's phpcs runs `env php`, not emulated)"
        ));
    }
    None
}

/// `Plugin::onDependenciesChangedEvent` after an install: `packages` are
/// the installed packages (the local repository), `root_manifest` the
/// project's composer.json, `variant` the plugin version's [`variant`].
pub fn register_standards(
    project_dir: &Path,
    layout: &Layout,
    packages: &[&LockPackage],
    root_manifest: &Value,
    variant: Variant,
) -> Result<ConfigChange> {
    let cwd_relative = variant.cwd_relative;
    let Some(phpcs) = packages.iter().find(|p| p.name() == PHPCS) else {
        return Ok(ConfigChange::Unchanged);
    };
    let Some(phpcs_dir) = layout.abs(phpcs.name()) else {
        return Ok(ConfigChange::Unchanged);
    };
    let cwd = normalize_path(&project_dir.to_string_lossy());
    let phpcs_path = normalize_path(&phpcs_dir.to_string_lossy());
    let conf = phpcs_dir.join("CodeSniffer.conf");

    // loadInstalledPaths (`--config-show`, the value trimmed, nothing when
    // empty) + cleanInstalledPaths
    let mut config = read_config(&conf)?;
    let mut installed: Vec<String> = config
        .iter()
        .find(|(k, _)| k == "installed_paths")
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let mut changed = false;
    installed.retain(|p| {
        let is_dir = |dir: PathBuf| std::fs::canonicalize(dir).is_ok_and(|d| d.is_dir());
        let keep = if crate::pathutil::is_absolute_path(p) {
            is_dir(PathBuf::from(p))
        } else {
            // `is_dir($path)` on a relative path reads the working
            // directory, the project (v1.0.0 to v1.2.0 only).
            is_dir(phpcs_dir.join(p)) || (cwd_relative && is_dir(project_dir.join(p)))
        };
        if !keep {
            changed = true;
        }
        keep
    });

    // updateInstalledPaths
    let mut search: Vec<PathBuf> = Vec::new();
    if variant.search_project
        || root_manifest.get("type").and_then(Value::as_str) == Some(STANDARD_TYPE)
    {
        search.push(project_dir.to_path_buf());
    }
    for p in packages {
        if p.package_type() == STANDARD_TYPE {
            if let Some(dir) = layout.abs(p.name()) {
                search.push(dir);
            }
        }
    }
    if !search.is_empty() {
        let min_depth = if crate::version::normalize_pretty(phpcs.version())
            .ok()
            .is_some_and(|v| {
                crate::version::Version::parse(&v)
                    .is_ok_and(|v| v >= crate::version::Version::parse("3.0.0.0").expect("version"))
            }) {
            0
        } else {
            1
        };
        let max_depth = root_manifest
            .get("extra")
            .and_then(|e| e.get("phpcodesniffer-search-depth"))
            .and_then(Value::as_u64)
            .map(|d| d as usize)
            .unwrap_or(3);
        let mut rulesets: Vec<PathBuf> = Vec::new();
        for base in &search {
            find_rulesets(base, 0, min_depth, max_depth, &mut rulesets);
        }
        for ruleset in rulesets {
            let mut standards = normalize_path(&ruleset.to_string_lossy());
            if standards != cwd {
                standards = crate::pathutil::php_dirname(&standards);
            }
            let relative = find_shortest_path(&phpcs_path, &standards, true);
            if !installed.contains(&relative) {
                installed.push(relative);
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(ConfigChange::Unchanged);
    }
    // saveInstalledPaths: `--config-set` (sorted, comma-joined) or
    // `--config-delete`; phpcs rewrites the whole file either way.
    config.retain(|(k, _)| k != "installed_paths");
    let change = if installed.is_empty() {
        ConfigChange::Deleted
    } else {
        installed.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        let paths = installed.join(",");
        config.push(("installed_paths".to_owned(), paths.clone()));
        ConfigChange::Set(paths)
    };
    write_config(&conf, &config)?;
    Ok(change)
}

/// The `ruleset.xml` files at a depth in `[min, max]` below `base`
/// (Finder: depth 0 is a direct child), as symfony/finder v5.4.45 walks
/// it for `files()->ignoreVCS(true)` with its default `ignoreDotFiles`:
/// a component starting with a dot (`#(^|/)\..+(/|$)#` on the path
/// relative to `base`) skipped, VCS directories skipped, a symbolic link
/// to a directory not descended into (`hasChildren(false)`), anything that
/// is not a directory accepted (`FileTypeFilterIterator` only rejects
/// `isDir()`, which follows links: a dangling link passes), unreadable
/// directories ignored; the directory holding each ruleset.
fn find_rulesets(dir: &Path, depth: usize, min: usize, max: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<(std::ffi::OsString, bool)> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let link = e.file_type().ok()?.is_symlink();
            Some((e.file_name(), link))
        })
        .collect();
    found.sort();
    for (name, link) in found {
        let n = name.to_string_lossy();
        if n.starts_with('.') {
            continue;
        }
        let path = dir.join(&name);
        let is_dir = path.is_dir();
        if is_dir {
            if link || VCS_DIRS.contains(&n.as_ref()) {
                continue;
            }
            if depth < max {
                find_rulesets(&path, depth + 1, min, max, out);
            }
        } else if name == "ruleset.xml" && depth >= min && depth <= max {
            out.push(dir.to_path_buf());
        }
    }
}

/// `CodeSniffer.conf` as `getAllConfigData` reads it: the string entries of
/// `$phpCodeSnifferConfig`, in order. Absent file: nothing.
fn read_config(conf: &Path) -> Result<Vec<(String, String)>> {
    let Ok(text) = std::fs::read_to_string(conf) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let re = regex_lite(&text);
    for (k, v) in re {
        out.push((k, v));
    }
    Ok(out)
}

/// `'key' => 'value',` pairs of a var_export'ed array of strings.
fn regex_lite(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('\'') else {
            continue;
        };
        let Some((key, rest)) = rest.split_once("' => '") else {
            continue;
        };
        let Some(value) = rest.strip_suffix("',") else {
            continue;
        };
        out.push((
            key.replace("\\'", "'").replace("\\\\", "\\"),
            value.replace("\\'", "'").replace("\\\\", "\\"),
        ));
    }
    out
}

fn write_config(conf: &Path, config: &[(String, String)]) -> Result<()> {
    let mut map = serde_json::Map::new();
    for (k, v) in config {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    let exported = crate::runtime_stub::php_var_export(&Value::Object(map), 0);
    let text = format!("<?php\n $phpCodeSnifferConfig = {exported};\n?>");
    // Inside a package cloned from the store (a hardlink on Linux): a new
    // inode through a temporary file and a rename, never an in-place write.
    let tmp = conf.with_extension("conf.vivacity-tmp");
    std::fs::write(&tmp, text).map_err(Error::io(&tmp))?;
    std::fs::rename(&tmp, conf).map_err(Error::io(conf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip_like_phpcs() {
        let tmp = tempfile::tempdir().expect("tmp");
        let conf = tmp.path().join("CodeSniffer.conf");
        write_config(
            &conf,
            &[("installed_paths".into(), "../../a/b,../../c/d".into())],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            "<?php\n $phpCodeSnifferConfig = array (\n  'installed_paths' => '../../a/b,../../c/d',\n);\n?>"
        );
        assert_eq!(
            read_config(&conf).unwrap(),
            vec![(
                "installed_paths".to_owned(),
                "../../a/b,../../c/d".to_owned()
            )]
        );
    }

    #[test]
    fn version_rule_for_relative_paths() {
        let v = |s: &str| (variant(s).search_project, variant(s).cwd_relative);
        assert_eq!(v("v0.7.2"), (true, true));
        assert_eq!(v("v1.0.0"), (false, true));
        assert_eq!(v("v1.2.0"), (false, true));
        assert_eq!(v("v1.2.1"), (false, false));
        assert_eq!(v("dev-main"), (false, false));
        assert_eq!(ConfigChange::Unchanged.line(), None);
        assert_eq!(
            ConfigChange::Set("../../a,../../b".into())
                .line()
                .as_deref(),
            Some("PHP CodeSniffer Config installed_paths set to ../../a,../../b")
        );
        assert_eq!(
            ConfigChange::Deleted.line().as_deref(),
            Some("PHP CodeSniffer Config installed_paths delete")
        );
    }

    #[test]
    fn rulesets_by_depth() {
        let tmp = tempfile::tempdir().expect("tmp");
        let p = tmp.path();
        std::fs::create_dir_all(p.join("Std/.git")).unwrap();
        std::fs::write(p.join("ruleset.xml"), "").unwrap(); // depth 0
        std::fs::write(p.join("Std/ruleset.xml"), "").unwrap(); // depth 1
        std::fs::write(p.join("Std/.git/ruleset.xml"), "").unwrap(); // skipped
        let mut out = Vec::new();
        find_rulesets(p, 0, 1, 3, &mut out);
        assert_eq!(out, vec![p.join("Std")]);
        let mut out = Vec::new();
        find_rulesets(p, 0, 0, 3, &mut out);
        assert_eq!(out, vec![p.join("Std"), p.to_path_buf()]);
    }

    /// v0.7.2's `phpcs` of the `PATH`: the bin-dir already in the `PATH`
    /// after another phpcs, or no `php` at all, is handed over.
    #[cfg(unix)]
    #[test]
    fn path_phpcs_and_php() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().expect("tmp");
        let project = std::fs::canonicalize(tmp.path()).unwrap().join("p");
        let foreign = std::fs::canonicalize(tmp.path()).unwrap().join("foreign");
        let tools = std::fs::canonicalize(tmp.path()).unwrap().join("tools");
        for d in [&project, &foreign, &tools] {
            std::fs::create_dir_all(d).unwrap();
        }
        for f in [foreign.join("phpcs"), tools.join("php")] {
            std::fs::write(&f, "").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let bin = project.join("vendor/bin");
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let m = serde_json::json!({});
        let path = |parts: &[String]| parts.join(":");
        // Not in the PATH: Composer puts it first.
        assert_eq!(
            path_issue(&project, &m, &path(&[s(&foreign), s(&tools)])),
            None
        );
        // In the PATH, first: its own phpcs answers.
        assert_eq!(
            path_issue(&project, &m, &path(&[s(&bin), s(&foreign), s(&tools)])),
            None
        );
        // In the PATH after another phpcs.
        assert!(
            path_issue(&project, &m, &path(&[s(&foreign), s(&bin), s(&tools)]))
                .is_some_and(|i| i.contains("another phpcs"))
        );
        // A non-executable phpcs is no phpcs.
        std::fs::set_permissions(
            foreign.join("phpcs"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(
            path_issue(&project, &m, &path(&[s(&foreign), s(&bin), s(&tools)])),
            None
        );
        // No php anywhere.
        assert!(path_issue(&project, &m, &path(&[s(&foreign)]))
            .is_some_and(|i| i.contains("without php")));
    }

    /// Finder's rules (docs/plans/chamilo-dealerdirect.md): a component
    /// starting with a dot is skipped relative to the search path only, a
    /// link to a directory is not descended into unless it is the search
    /// path, a dangling `ruleset.xml` link is a file.
    #[cfg(unix)]
    #[test]
    fn rulesets_as_the_finder_walks() {
        let tmp = tempfile::tempdir().expect("tmp");
        let base = tmp.path().join(".dotted-parent/proj");
        let outside = tmp.path().join("outside");
        for d in [".github/Std", "a/.hidden/Std", "cfg/Std", "x/b/c/d"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
            std::fs::write(base.join(d).join("ruleset.xml"), "").unwrap();
        }
        std::fs::create_dir_all(outside.join("Std")).unwrap();
        std::fs::write(outside.join("Std/ruleset.xml"), "").unwrap();
        std::os::unix::fs::symlink(&outside, base.join("linked")).unwrap();
        std::fs::create_dir_all(base.join("dangling")).unwrap();
        std::os::unix::fs::symlink(base.join("nowhere"), base.join("dangling/ruleset.xml"))
            .unwrap();
        let mut out = Vec::new();
        find_rulesets(&base, 0, 0, 3, &mut out);
        assert_eq!(out, vec![base.join("cfg/Std"), base.join("dangling")]);
        // The link as a search path itself is read.
        let mut out = Vec::new();
        find_rulesets(&base.join("linked"), 0, 0, 3, &mut out);
        assert_eq!(out, vec![base.join("linked/Std")]);
        // Depth 4 reached with a larger maximum.
        let mut out = Vec::new();
        find_rulesets(&base, 0, 0, 4, &mut out);
        assert!(out.contains(&base.join("x/b/c/d")));
    }
}
