//! Emulation of `dealerdirect/phpcodesniffer-composer-installer`
//! (docs/reference/plugins/phpcodesniffer-composer-installer/, MIT; 0.7.2
//! to 1.2.1 behave alike on an install): on `post-install-cmd` /
//! `post-update-cmd`, when `squizlabs/php_codesniffer` is installed, the
//! plugin registers every coding standard found in the packages of type
//! `phpcodesniffer-standard` (and in the project when the root package is
//! one) as PHP_CodeSniffer's `installed_paths`, through
//! `phpcs --config-set installed_paths <a>,<b>`:
//!
//! - `Finder::files()->name('ruleset.xml')` at a depth between `min` (0 for
//!   phpcs >= 3.0.0, 1 before) and `max` (`extra.phpcodesniffer-search-depth`,
//!   3 by default) below each search path, VCS directories skipped;
//! - a standard's path is the ruleset's directory, then its parent unless
//!   it is the project itself, made relative to the phpcs package with
//!   `findShortestPath(..., directories: true)`;
//! - the paths already registered are kept when their directory still
//!   exists, the new ones appended, the whole sorted (`sort()`, bytewise)
//!   and written by phpcs into `<phpcs>/CodeSniffer.conf`:
//!   `<?php\n $phpCodeSnifferConfig = ` + `var_export` + `;\n?>`;
//! - nothing happens when nothing changed, or when no search path exists.

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

/// `Plugin::onDependenciesChangedEvent` after an install: `packages` are
/// the installed packages (the local repository), `root_manifest` the
/// project's composer.json.
pub fn register_standards(
    project_dir: &Path,
    layout: &Layout,
    packages: &[&LockPackage],
    root_manifest: &Value,
) -> Result<()> {
    let Some(phpcs) = packages.iter().find(|p| p.name() == PHPCS) else {
        return Ok(());
    };
    let Some(phpcs_dir) = layout.abs(phpcs.name()) else {
        return Ok(());
    };
    let cwd = normalize_path(&project_dir.to_string_lossy());
    let phpcs_path = normalize_path(&phpcs_dir.to_string_lossy());
    let conf = phpcs_dir.join("CodeSniffer.conf");

    // loadInstalledPaths + cleanInstalledPaths
    let mut config = read_config(&conf)?;
    let mut installed: Vec<String> = config
        .iter()
        .find(|(k, _)| k == "installed_paths")
        .map(|(_, v)| v.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let mut changed = false;
    installed.retain(|p| {
        let dir = if crate::pathutil::is_absolute_path(p) {
            PathBuf::from(p)
        } else {
            phpcs_dir.join(p)
        };
        let keep = std::fs::canonicalize(&dir).is_ok_and(|d| d.is_dir());
        if !keep {
            changed = true;
        }
        keep
    });

    // updateInstalledPaths
    let mut search: Vec<PathBuf> = Vec::new();
    if root_manifest.get("type").and_then(Value::as_str) == Some(STANDARD_TYPE) {
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
        return Ok(());
    }
    // saveInstalledPaths: `--config-set` (sorted, comma-joined) or
    // `--config-delete`; phpcs rewrites the whole file either way.
    config.retain(|(k, _)| k != "installed_paths");
    if !installed.is_empty() {
        installed.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        config.push(("installed_paths".to_owned(), installed.join(",")));
    }
    write_config(&conf, &config)
}

/// The `ruleset.xml` files at a depth in `[min, max]` below `base`
/// (Finder: depth 0 is a direct child), VCS directories skipped, unreadable
/// directories ignored; the directory holding each ruleset.
fn find_rulesets(dir: &Path, depth: usize, min: usize, max: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<std::ffi::OsString> = entries
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    names.sort();
    for name in names {
        let path = dir.join(&name);
        if path.is_dir() {
            let n = name.to_string_lossy();
            if VCS_DIRS.contains(&n.as_ref()) {
                continue;
            }
            if depth < max {
                find_rulesets(&path, depth + 1, min, max, out);
            }
        } else if name == "ruleset.xml" && depth >= min && depth <= max && path.is_file() {
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
    std::fs::write(conf, text).map_err(Error::io(conf))
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
}
