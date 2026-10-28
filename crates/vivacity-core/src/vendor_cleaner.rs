//! Emulation of `liborm85/composer-vendor-cleaner` 1.7.1 (docs/reference/
//! plugins/composer-vendor-cleaner/, MIT). With `extra.dev-files` (package
//! glob → patterns), `Plugin::cleanup` runs twice on an install or update:
//! at `PRE_AUTOLOAD_DUMP` it cleans every package of the local repository
//! (`Cleaner::cleanupPackages`: the entries the patterns select, in
//! descending order, a directory only once empty; then every empty
//! directory of every package), at `POST_INSTALL_CMD` / `POST_UPDATE_CMD`
//! the bin directory (`cleanupBinary`) and the summary (`finishCleanup`).
//! On `dump-autoload` the three run at `PRE_AUTOLOAD_DUMP`. Its lines go to
//! stdout (`$io->write`); the per-file ones are VERBOSE (handed over).
//!
//! The patterns are Symfony's `Glob::toRegex($glob, false)` (the copy the
//! plugin ships, `Finder/Glob.php`), matched with PCRE (`preg_grep`), `i`
//! when `config.dev-files.match-case` is false.

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use std::path::Path;

pub const PLUGIN_NAME: &str = "liborm85/composer-vendor-cleaner";

/// `extra.dev-files` and `config.dev-files`, as `Plugin::activate` reads
/// them.
#[derive(Debug, Clone)]
pub struct Config {
    /// Package glob → patterns, in the object's order.
    pub dev_files: Vec<(String, Vec<String>)>,
    pub match_case: bool,
    pub remove_empty_dirs: bool,
    pub no_dev_only: bool,
}

/// PHP's `(bool)` of a decoded value.
fn php_bool(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty() && s != "0",
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A key PHP turns into an integer (`"7"`, `"-3"`, not `"07"`).
pub fn php_int_key(k: &str) -> bool {
    let digits = k.strip_prefix('-').unwrap_or(k);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
        && !(k.starts_with('-') && digits == "0")
        && k.parse::<i64>().is_ok()
}

impl Config {
    /// `Plugin::activate`: `Ok(None)` when the cleaner is not configured
    /// (`if ($devFiles)` false); `Err` for what vivacity hands over — an
    /// external JSON file (`dev-files` a string), a shape the plugin throws
    /// on, a pattern PCRE refuses, `config.dev-files` only in the global
    /// config.
    pub fn read(manifest: &Value) -> std::result::Result<Option<Config>, String> {
        let dev_files = match manifest.get("extra").and_then(|e| e.get("dev-files")) {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::String(_)) => {
                return Err("extra.dev-files names an external JSON file (not emulated)".to_owned())
            }
            Some(v) if !php_bool(v) => return Ok(None),
            Some(Value::Object(m)) => m,
            Some(_) => {
                return Err(
                    "extra.dev-files is not an object of pattern lists (the plugin throws)"
                        .to_owned(),
                )
            }
        };
        let mut globs = Vec::new();
        for (glob, patterns) in dev_files {
            let Value::Array(list) = patterns else {
                return Err(format!(
                    "extra.dev-files `{glob}` is not a list of patterns (the plugin throws)"
                ));
            };
            if php_int_key(glob) {
                return Err(format!(
                    "extra.dev-files key `{glob}` becomes an integer in PHP"
                ));
            }
            let mut out = Vec::new();
            for p in list {
                match p {
                    Value::String(s) => out.push(s.clone()),
                    _ => {
                        return Err(format!(
                            "extra.dev-files `{glob}` holds a pattern that is not a string"
                        ))
                    }
                }
            }
            globs.push((glob.clone(), out));
        }
        let project = manifest.get("config").and_then(|c| c.get("dev-files"));
        if project.is_none() && crate::layout::global_config_value("dev-files").is_some() {
            return Err("config.dev-files is set in the global config (not read)".to_owned());
        }
        let flag = |key: &str, default: bool| match project {
            // `isset($pluginConfig[$key])` on an array; a string or a scalar
            // has no such offset.
            Some(Value::Object(m)) => match m.get(key) {
                None | Some(Value::Null) => default,
                Some(v) => php_bool(v),
            },
            _ => default,
        };
        let config = Config {
            dev_files: globs,
            match_case: flag("match-case", true),
            remove_empty_dirs: flag("remove-empty-dirs", true),
            no_dev_only: flag("no-dev-only", false),
        };
        // Every regex the plugin will build compiles (`preg_grep` returns
        // false on one that does not, then a TypeError).
        for (glob, patterns) in &config.dev_files {
            for g in package_globs(glob) {
                config
                    .regex(&g)
                    .map_err(|e| format!("extra.dev-files `{glob}`: {e}"))?;
            }
            for p in patterns {
                let (_, g) = filter_glob(p);
                config
                    .regex(&g)
                    .map_err(|e| format!("extra.dev-files pattern `{p}`: {e}"))?;
            }
        }
        Ok(Some(config))
    }

    /// `GlobFilter::globPatternToRegexPattern`.
    fn regex(&self, glob: &str) -> std::result::Result<pcre2::bytes::Regex, String> {
        let r = crate::path_install::glob_to_regex_with(glob, false);
        let inner = &r[1..r.len() - 1];
        pcre2::bytes::RegexBuilder::new()
            .caseless(!self.match_case)
            .build(inner)
            .map_err(|e| e.to_string())
    }

    /// `DevFilesFinder::isGlobPatternForPackage`.
    fn glob_matches(&self, name: &str, glob: &str) -> bool {
        package_globs(glob).iter().any(|g| {
            self.regex(g)
                .ok()
                .and_then(|r| r.is_match(name.as_bytes()).ok())
                .unwrap_or(false)
        })
    }

    /// `DevFilesFinder::getGlobPatternsForPackage`.
    pub fn patterns_for(&self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (glob, patterns) in &self.dev_files {
            if self.glob_matches(name, glob) {
                out.extend(patterns.iter().cloned());
            }
        }
        out
    }

    /// `DevFilesFinder::getFilteredEntries`: the entries the patterns
    /// select (an include, and no `!` exclude), sorted descending (`rsort`
    /// of strings that all start with `/`: byte order).
    pub fn filtered(&self, entries: &[String], patterns: &[String]) -> Vec<String> {
        let mut includes = Vec::new();
        let mut excludes = Vec::new();
        for p in patterns {
            let (exclude, glob) = filter_glob(p);
            if let Ok(r) = self.regex(&glob) {
                if exclude {
                    excludes.push(r);
                } else {
                    includes.push(r);
                }
            }
        }
        if entries.is_empty() || includes.is_empty() {
            return Vec::new();
        }
        let hit = |rs: &[pcre2::bytes::Regex], e: &str| {
            rs.iter().any(|r| r.is_match(e.as_bytes()).unwrap_or(false))
        };
        let mut out: Vec<String> = Vec::new();
        for e in entries {
            if hit(&includes, e) && !hit(&excludes, e) && !out.contains(e) {
                out.push(e.clone());
            }
        }
        out.sort_by(|a, b| b.as_bytes().cmp(a.as_bytes()));
        out
    }
}

/// The globs `isGlobPatternForPackage` includes for one key.
fn package_globs(glob: &str) -> Vec<String> {
    let g = glob.trim_end_matches('/');
    if g.is_empty() {
        vec!["*".to_owned(), "*/*".to_owned()]
    } else if !g.contains('/') {
        vec![g.to_owned(), format!("{g}/*")]
    } else {
        vec![g.to_owned()]
    }
}

/// `DevFilesFinder::buildGlobPatternForFilter` for one pattern: whether it
/// excludes, and the glob.
fn filter_glob(pattern: &str) -> (bool, String) {
    let (exclude, p) = match pattern.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let prefix = if p.starts_with('/') { "" } else { "/**/" };
    let suffix = if p.ends_with('/') { "**" } else { "" };
    let glob = format!(
        "/{}",
        format!("{prefix}{p}{suffix}").trim_start_matches('/')
    );
    (exclude, glob)
}

/// The counts `finishCleanup` reports.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub files: usize,
    pub dirs: usize,
    pub empty_dirs: usize,
    pub packages: usize,
}

/// `Directory::getEntries`: every entry under `root`, `/`-prefixed, a
/// directory with a trailing `/` (the `dir/.` the iterator yields), a
/// symlink as a leaf (`RecursiveDirectoryIterator` does not follow it).
pub fn entries(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk(root, "", &mut out, true)?;
    Ok(out)
}

/// `Directory::getDirectories`.
fn directories(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk(root, "", &mut out, false)?;
    Ok(out)
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>, files: bool) -> Result<()> {
    if !rel.is_empty() {
        out.push(format!("/{rel}/"));
    }
    for e in std::fs::read_dir(dir).map_err(Error::io(dir))? {
        let e = e.map_err(Error::io(dir))?;
        let name = e.file_name().to_string_lossy().into_owned();
        let sub = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        let ft = e.file_type().map_err(Error::io(dir))?;
        if ft.is_dir() {
            walk(&e.path(), &sub, out, files)?;
        } else if files {
            out.push(format!("/{sub}"));
        }
    }
    Ok(())
}

/// `Filesystem::isEmptyDirectory` (`RecursiveDirectoryIterator` with
/// `SKIP_DOTS`, followed through a symlink).
fn is_empty_dir(path: &Path) -> Result<bool> {
    Ok(std::fs::read_dir(path)
        .map_err(Error::io(path))?
        .next()
        .is_none())
}

/// Composer's `Filesystem::removeDirectory`: a symlink is unlinked, a
/// directory removed with its tree.
fn remove_dir(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(Error::io(path))?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(path).map_err(Error::io(path))
    } else {
        std::fs::remove_dir_all(path).map_err(Error::io(path))
    }
}

/// The plugin's `Cleaner`, across its two calls.
pub struct Cleaner<'a> {
    pub config: &'a Config,
    pub counts: Counts,
}

impl<'a> Cleaner<'a> {
    pub fn new(config: &'a Config) -> Cleaner<'a> {
        Cleaner {
            config,
            counts: Counts::default(),
        }
    }

    /// `cleanupPackages`: `packages` is the local repository, each with its
    /// install path (`None`: a metapackage). Returns the two lines it
    /// prints first.
    pub fn cleanup_packages(
        &mut self,
        packages: &[(String, Option<std::path::PathBuf>)],
    ) -> Result<()> {
        self.counts.packages = packages.len();
        for (name, path) in packages {
            let patterns = self.config.patterns_for(name);
            let Some(path) = path.as_ref().filter(|_| !patterns.is_empty()) else {
                continue;
            };
            let all = entries(path)?;
            let chosen = self.config.filtered(&all, &patterns);
            self.remove_files(path, &chosen)?;
        }
        if self.config.remove_empty_dirs {
            for (_, path) in packages {
                if let Some(path) = path {
                    self.remove_empty_directories(path)?;
                }
            }
        }
        Ok(())
    }

    /// `cleanupBinary`: `false` when the bin directory does not exist (no
    /// line).
    pub fn cleanup_binary(&mut self, bin_dir: &Path) -> Result<bool> {
        if !bin_dir.exists() {
            return Ok(false);
        }
        let patterns = self.config.patterns_for("bin");
        if !patterns.is_empty() {
            let all = entries(bin_dir)?;
            let chosen = self.config.filtered(&all, &patterns);
            self.remove_files(bin_dir, &chosen)?;
        }
        if self.config.remove_empty_dirs {
            self.remove_empty_directories(bin_dir)?;
        }
        Ok(true)
    }

    /// `finishCleanup`'s summary line (without the analysis of unused
    /// patterns, unreachable here: docs/plans/plugins-round2.md).
    pub fn summary(&self) -> String {
        let c = &self.counts;
        if c.empty_dirs > 0 {
            format!(
                "Composer vendor cleaner: Removed {} files and {} (of which {} are empty) directories from {} packages",
                c.files, c.dirs, c.empty_dirs, c.packages
            )
        } else {
            format!(
                "Composer vendor cleaner: Removed {} files and {} directories from {} packages",
                c.files, c.dirs, c.packages
            )
        }
    }

    /// `Cleaner::removeFiles`.
    fn remove_files(&mut self, root: &Path, chosen: &[String]) -> Result<()> {
        let base = root.to_string_lossy();
        for rel in chosen {
            let path = std::path::PathBuf::from(format!("{base}{rel}"));
            // `is_dir` follows a symlink.
            if path.is_dir() {
                if !is_empty_dir(&path)? {
                    continue;
                }
                remove_dir(&path)?;
                self.counts.dirs += 1;
            } else {
                std::fs::remove_file(&path).map_err(Error::io(&path))?;
                self.counts.files += 1;
            }
        }
        Ok(())
    }

    /// `Cleaner::removeEmptyDirectories`.
    fn remove_empty_directories(&mut self, root: &Path) -> Result<()> {
        let base = root.to_string_lossy();
        let mut dirs = directories(root)?;
        dirs.sort_by(|a, b| b.as_bytes().cmp(a.as_bytes()));
        for rel in dirs {
            let path = std::path::PathBuf::from(format!("{base}{rel}"));
            if !is_empty_dir(&path)? {
                continue;
            }
            remove_dir(&path)?;
            self.counts.dirs += 1;
            self.counts.empty_dirs += 1;
        }
        if is_empty_dir(root)? {
            remove_dir(root)?;
            self.counts.dirs += 1;
            self.counts.empty_dirs += 1;
        }
        Ok(())
    }
}

/// Whether the patterns that apply to a package select one of its declared
/// `bin` files, or the `bin` patterns one of the proxies: Composer would
/// then re-create or report the binary after the dump
/// (`ensureBinariesPresence`), which vivacity does not reproduce.
pub fn touches_bin(config: &Config, package: &Map<String, Value>) -> Option<String> {
    let name = package.get("name").and_then(Value::as_str)?;
    let bins: Vec<&str> = match package.get("bin") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => return None,
    };
    let own = config.patterns_for(name);
    let proxies = config.patterns_for("bin");
    for b in bins {
        let entry = format!("/{}", b.trim_start_matches("./").trim_start_matches('/'));
        let proxy = format!("/{}", b.rsplit('/').next().unwrap_or(b));
        let hit = |patterns: &[String], e: &str| {
            !patterns.is_empty() && !config.filtered(&[e.to_owned()], patterns).is_empty()
        };
        // A pattern ending in `/` selects the directory entry too.
        let dir_entry = entry
            .rsplit_once('/')
            .map(|(d, _)| format!("{d}/"))
            .filter(|d| d != "/");
        if hit(&own, &entry)
            || dir_entry.as_deref().is_some_and(|d| hit(&own, d))
            || hit(&proxies, &proxy)
        {
            return Some(format!(
                "liborm85/composer-vendor-cleaner would remove the binary {b} of {name} (Composer reinstalls or reports it after the dump; not emulated)"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(v: Value) -> Config {
        Config::read(&json!({"extra": {"dev-files": v}}))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn reads_the_configuration_like_activate() {
        assert!(Config::read(&json!({})).unwrap().is_none());
        assert!(Config::read(&json!({"extra": {"dev-files": {}}}))
            .unwrap()
            .is_none());
        assert!(Config::read(&json!({"extra": {"dev-files": "x.json"}})).is_err());
        assert!(Config::read(&json!({"extra": {"dev-files": ["tests/"]}})).is_err());
        assert!(Config::read(&json!({"extra": {"dev-files": {"/": "tests/"}}})).is_err());
        let c = Config::read(&json!({"extra": {"dev-files": {"/": ["x"]}}, "config": {"dev-files": {"match-case": "false", "no-dev-only": 1}}})).unwrap().unwrap();
        // `(bool) "false"` is true.
        assert!(c.match_case && c.no_dev_only && c.remove_empty_dirs);
    }

    #[test]
    fn selects_entries_like_the_plugin() {
        let c = config(
            json!({"/": ["tests/", ".travis.yml"], "acme/x": ["res/", "!res/keep.txt"], "bin": ["*.bat"]}),
        );
        assert_eq!(c.patterns_for("acme/y"), vec!["tests/", ".travis.yml"]);
        assert_eq!(c.patterns_for("acme/x").len(), 4);
        assert_eq!(
            c.patterns_for("bin"),
            vec!["tests/", ".travis.yml", "*.bat"]
        );
        let all: Vec<String> = [
            "/src/",
            "/src/A.php",
            "/src/tests/",
            "/src/tests/T.php",
            "/tests",
            "/.travis.yml",
            "/res/",
            "/res/keep.txt",
            "/res/a",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(
            c.filtered(&all, &c.patterns_for("acme/x")),
            vec![
                "/src/tests/T.php",
                "/src/tests/",
                "/res/a",
                "/res/",
                "/.travis.yml"
            ]
        );
        let ci = Config {
            match_case: false,
            ..config(json!({"/": ["docs/"]}))
        };
        assert_eq!(
            ci.filtered(&["/Docs/".to_owned()], &["docs/".to_owned()]),
            vec!["/Docs/"]
        );
    }

    #[test]
    fn int_keys_like_php() {
        assert!(php_int_key("7") && php_int_key("-3") && php_int_key("0"));
        assert!(!php_int_key("07") && !php_int_key("-0") && !php_int_key("a") && !php_int_key(""));
    }

    #[test]
    fn cleans_a_tree() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("acme/x");
        for f in ["src/A.php", "src/tests/T.php", "tests/a", "keep/k"] {
            std::fs::create_dir_all(p.join(f).parent().unwrap()).unwrap();
            std::fs::write(p.join(f), "x").unwrap();
        }
        std::fs::create_dir_all(p.join("empty/deep")).unwrap();
        let c = config(json!({"/": ["tests/"]}));
        let mut cl = Cleaner::new(&c);
        cl.cleanup_packages(&[
            ("acme/x".to_owned(), Some(p.clone())),
            ("acme/meta".to_owned(), None),
        ])
        .unwrap();
        assert_eq!(
            cl.counts,
            Counts {
                files: 2,
                dirs: 4,
                empty_dirs: 2,
                packages: 2
            }
        );
        assert_eq!(cl.summary(), "Composer vendor cleaner: Removed 2 files and 4 (of which 2 are empty) directories from 2 packages");
        assert!(
            p.join("src/A.php").exists() && p.join("keep/k").exists() && !p.join("empty").exists()
        );
    }

    #[test]
    fn bins_the_patterns_would_touch() {
        let c = config(json!({"/": ["bin/"]}));
        let p = json!({"name": "a/b", "bin": ["bin/x"]});
        assert!(touches_bin(&c, p.as_object().unwrap()).is_some());
        let c = config(json!({"bin": ["x"]}));
        assert!(touches_bin(&c, p.as_object().unwrap()).is_some());
        let c = config(json!({"/": ["tests/"], "bin": ["*.bat"]}));
        assert!(touches_bin(&c, p.as_object().unwrap()).is_none());
    }
}
