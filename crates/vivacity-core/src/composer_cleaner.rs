//! Emulation of `dg/composer-cleaner` v2.2.2 (docs/reference/plugins/
//! composer-cleaner/, BSD-3-Clause). Its `POST_INSTALL_CMD` /
//! `POST_UPDATE_CMD` listener (`Plugin::clean`) runs `Cleaner::clean` on the
//! vendor directory: for each `<vendor-dir>/<a>/<b>` directory that is not a
//! symlink, the package's own composer.json decides what stays — the
//! `exclude-from-classmap` entries are removed, then every top-level entry
//! that is neither the first segment of an autoload source or a `bin`, nor
//! `composer.json`/`license*`/`LICENSE*`/`.phpstorm.meta.php`, nor matched
//! by the root's `cleaner-ignore` (`fnmatch`). One line on stdout:
//! `Composer cleaner: Removed N files or directories.`
//!
//! The walk is the real tree (`FilesystemIterator`), not the local
//! repository. Its VERBOSE lines come in `readdir` order: `-v` is handed to
//! Composer (`verbose_listeners`).

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const PLUGIN_NAME: &str = "dg/composer-cleaner";

/// `Cleaner::$alwaysIgnore`.
const ALWAYS_IGNORE: [&str; 4] = [
    "composer.json",
    "license*",
    "LICENSE*",
    ".phpstorm.meta.php",
];

/// One package's rule from `cleaner-ignore`: skip it (`true`), or keep the
/// entries these masks match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ignore {
    Skip,
    Masks(Vec<String>),
}

/// `$extra['cleaner-ignore'] ?? (array) $config->get('cleaner-ignore')`,
/// per package name. `Err`: a shape the plugin's strict-typed code would
/// throw on, or a mask this `fnmatch` does not reproduce (the libc's differ).
pub fn ignore_paths(manifest: &Value) -> std::result::Result<Vec<(String, Ignore)>, String> {
    let from_extra = manifest
        .get("extra")
        .and_then(|e| e.get("cleaner-ignore"))
        .filter(|v| !v.is_null());
    let value = match from_extra {
        Some(v) => match v {
            // `Cleaner::clean(string, array $ignorePaths)` under
            // `strict_types`: anything but an array is a TypeError.
            Value::Object(_) | Value::Array(_) => v.clone(),
            _ => {
                return Err(
                    "extra.cleaner-ignore is not an object (TypeError in the plugin)".to_owned(),
                )
            }
        },
        None => {
            let project = manifest
                .get("config")
                .and_then(|c| c.get("cleaner-ignore"))
                .cloned();
            match project.or_else(|| crate::layout::global_config_value("cleaner-ignore")) {
                // `(array)` of a scalar: one integer key, never a package.
                Some(Value::Object(m)) => Value::Object(m),
                _ => Value::Object(Map::new()),
            }
        }
    };
    let Value::Object(map) = value else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (name, rule) in map {
        let rule = match rule {
            Value::Null => continue,
            Value::Bool(true) => Ignore::Skip,
            Value::String(s) => Ignore::Masks(vec![s]),
            Value::Array(a) => Ignore::Masks(strings(&a, &name)?),
            Value::Object(o) => {
                Ignore::Masks(strings(&o.values().cloned().collect::<Vec<_>>(), &name)?)
            }
            _ => {
                return Err(format!(
                    "cleaner-ignore of {name} is neither true nor masks (TypeError in the plugin)"
                ))
            }
        };
        if let Ignore::Masks(masks) = &rule {
            for m in masks {
                if !fnmatch_portable(m) {
                    return Err(format!(
                        "cleaner-ignore mask `{m}` of {name} is matched differently by each libc's fnmatch"
                    ));
                }
            }
        }
        out.push((name, rule));
    }
    Ok(out)
}

fn strings(values: &[Value], name: &str) -> std::result::Result<Vec<String>, String> {
    values
        .iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.clone()),
            _ => Err(format!(
                "cleaner-ignore of {name} holds a mask that is not a string (TypeError in the plugin)"
            )),
        })
        .collect()
}

/// The autoload shapes `Cleaner::getSources` / `getExcludes` read without a
/// TypeError (`ltrim`, `rtrim`, `strtr` on strings under `strict_types`),
/// checked on a package's lock entry — the dist's composer.json it will read.
pub fn shape_issue(package: &Map<String, Value>) -> Option<String> {
    let name = package.get("name").and_then(Value::as_str).unwrap_or("?");
    let bad = || {
        Some(format!(
            "package {name}: an autoload or bin entry of a shape dg/composer-cleaner throws on"
        ))
    };
    let string_or_list = |v: &Value| match v {
        Value::String(_) => true,
        Value::Array(a) => a.iter().all(Value::is_string),
        Value::Object(o) => o.values().all(Value::is_string),
        _ => false,
    };
    if let Some(bin) = package.get("bin") {
        if !bin.is_null() && !string_or_list(bin) {
            return bad();
        }
    }
    let autoload = package.get("autoload")?;
    match autoload {
        Value::Object(types) => {
            for (ty, items) in types {
                let ok = match ty.as_str() {
                    "psr-0" | "psr-4" => match items {
                        Value::Object(m) => m.values().all(string_or_list),
                        Value::Array(a) => a.iter().all(string_or_list),
                        _ => false,
                    },
                    "classmap" | "files" | "exclude-from-classmap" => string_or_list(items),
                    _ => true,
                };
                if !ok {
                    return bad();
                }
            }
            None
        }
        Value::Array(_) | Value::Null => None,
        _ => bad(),
    }
}

/// What one `clean` did: the count of the closing line, and the lines it
/// wrote on stderr (an unknown autoload type, an invalid composer.json),
/// in the order written.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub removed: usize,
    pub errors: Vec<String>,
}

/// `Cleaner::clean($vendorDir, $ignorePaths)`.
pub fn clean(vendor_dir: &Path, ignore: &[(String, Ignore)]) -> Result<Cleaned> {
    let mut out = Cleaned::default();
    for vendor in read_dir(vendor_dir)? {
        // `$packageVendor->isDir()` follows a symlink.
        if !vendor.is_dir() {
            continue;
        }
        let vendor_name = file_name(&vendor);
        for package in read_dir(&vendor)? {
            if !package.is_dir() {
                continue;
            }
            // `isSymlinkedDirectory`: a `path` package laid out as a link.
            if is_link(&package) {
                continue;
            }
            let name = format!("{vendor_name}/{}", file_name(&package));
            match ignore.iter().find(|(n, _)| *n == name).map(|(_, r)| r) {
                Some(Ignore::Skip) => {}
                Some(Ignore::Masks(m)) => process_package(&package, m.clone(), &mut out)?,
                None => process_package(&package, Vec::new(), &mut out)?,
            }
        }
    }
    Ok(out)
}

/// `Cleaner::processPackage`.
fn process_package(dir: &Path, mut ignore: Vec<String>, out: &mut Cleaned) -> Result<()> {
    let file = dir.join("composer.json");
    // `loadComposerJson`: absent (a VERBOSE line), or not an object.
    if !file.is_file() {
        return Ok(());
    }
    let data = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let Some(Value::Object(data)) = data else {
        out.errors
            .push(format!("Composer cleaner: Invalid {}.", file.display()));
        return Ok(());
    };
    // `in_array($type, [null, 'library', 'composer-plugin'], true)`.
    match data.get("type") {
        None | Some(Value::Null) => {}
        Some(Value::String(t)) if t == "library" || t == "composer-plugin" => {}
        _ => return Ok(()),
    }
    for exclude in excludes(&data) {
        let dir_rel = exclude.trim_start_matches('.').trim_matches('/');
        // `if ($dir && …)`: "" and "0" are false.
        if php_truthy_str(dir_rel) && !dir_rel.contains("..") && !match_mask(dir_rel, &ignore) {
            remove(&dir.join(dir_rel))?;
            out.removed += 1;
        }
    }
    for source in sources(&data, out) {
        let s = format!(
            "{}/",
            source.trim_start_matches('.').trim_start_matches('/')
        );
        let first = s.split('/').next().unwrap_or_default();
        ignore.push(first.to_owned());
    }
    if ignore.is_empty() || match_mask("", &ignore) {
        return Ok(());
    }
    ignore.extend(ALWAYS_IGNORE.iter().map(|s| (*s).to_owned()));
    for entry in read_dir(dir)? {
        let name = file_name(&entry);
        if !match_mask(&name, &ignore) {
            remove(&entry)?;
            out.removed += 1;
        }
    }
    Ok(())
}

/// `(array)` of a decoded value, as the strings it holds.
fn as_strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::Object(o) => o
            .values()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// PHP's `empty()` of a decoded value (`json_decode` without `assoc`: an
/// object is a `stdClass`, never empty).
fn php_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => !php_truthy_str(s),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(_)) => false,
    }
}

fn php_truthy_str(s: &str) -> bool {
    !s.is_empty() && s != "0"
}

/// `Cleaner::getExcludes`.
fn excludes(data: &Map<String, Value>) -> Vec<String> {
    let v = data
        .get("autoload")
        .and_then(Value::as_object)
        .and_then(|a| a.get("exclude-from-classmap"));
    if php_empty(v) {
        return Vec::new();
    }
    v.map(as_strings).unwrap_or_default()
}

/// `Cleaner::getSources`: an unknown autoload type writes its line and
/// drops every source.
fn sources(data: &Map<String, Value>, out: &mut Cleaned) -> Vec<String> {
    let autoload = data.get("autoload");
    if php_empty(autoload) {
        return Vec::new();
    }
    let mut sources: Vec<String> = data.get("bin").map(as_strings).unwrap_or_default();
    let entries: Vec<(String, &Value)> = match autoload {
        Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Some(Value::Array(a)) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    };
    for (ty, items) in entries {
        match ty.as_str() {
            "psr-0" => {
                for (ns, paths) in members(items) {
                    // `strtr($namespace, '\_', '//')`.
                    let ns: String = ns
                        .chars()
                        .map(|c| if c == '\\' || c == '_' { '/' } else { c })
                        .collect();
                    for p in as_strings(paths) {
                        sources.push(format!("{}/{ns}", p.trim_end_matches(['\\', '/'])));
                    }
                }
            }
            "psr-4" => {
                for (_, paths) in members(items) {
                    sources.extend(as_strings(paths));
                }
            }
            "classmap" | "files" => sources.extend(as_strings(items)),
            "exclude-from-classmap" => {}
            _ => {
                out.errors.push(format!("unknown autoload type {ty}"));
                return Vec::new();
            }
        }
    }
    sources
}

fn members(v: &Value) -> Vec<(String, &Value)> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    }
}

/// `Cleaner::matchMask`.
fn match_mask(name: &str, masks: &[String]) -> bool {
    masks.iter().any(|m| fnmatch(m.as_bytes(), name.as_bytes()))
}

/// Composer's `Filesystem::remove`: a directory (followed) goes with its
/// tree — a symlink to one is unlinked (`unlinkSymlinkedDirectory`) —, an
/// existing file (followed) is unlinked, anything else (a dangling link) is
/// left; the caller counts it either way.
fn remove(path: &Path) -> Result<()> {
    if path.is_dir() {
        if is_link(path) {
            std::fs::remove_file(path).map_err(Error::io(path))?;
        } else {
            std::fs::remove_dir_all(path).map_err(Error::io(path))?;
        }
    } else if path.exists() {
        std::fs::remove_file(path).map_err(Error::io(path))?;
    }
    Ok(())
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `FilesystemIterator` (`SKIP_DOTS`): the entries in `readdir` order.
fn read_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(Error::io(dir))? {
        out.push(e.map_err(Error::io(dir))?.path());
    }
    Ok(out)
}

/// Whether `fnmatch(3)` without flags agrees across the C libraries PHP is
/// built against for this mask: an unclosed bracket expression, a trailing
/// backslash and the `[: :]` / `[= =]` / `[. .]` forms are not (measured on
/// macOS against glibc's documented behaviour).
pub fn fnmatch_portable(mask: &str) -> bool {
    let b = mask.as_bytes();
    if b.last() == Some(&b'\\') && trailing_backslashes(b) % 2 == 1 {
        return false;
    }
    if mask.contains("[:") || mask.contains("[=") || mask.contains("[.") {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            // A `\` inside a bracket expression: the C libraries differ.
            b'[' => match bracket_end(b, i) {
                Some(end) if !b[i..end].contains(&b'\\') => i = end + 1,
                _ => return false,
            },
            _ => i += 1,
        }
    }
    true
}

fn trailing_backslashes(b: &[u8]) -> usize {
    b.iter().rev().take_while(|c| **c == b'\\').count()
}

/// The index of the `]` closing the bracket expression opened at `start`.
fn bracket_end(p: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    if i < p.len() && (p[i] == b'!' || p[i] == b'^') {
        i += 1;
    }
    // A `]` right after the opening (or the negation) is a member.
    if i < p.len() && p[i] == b']' {
        i += 1;
    }
    while i < p.len() {
        if p[i] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `fnmatch($pattern, $string)` with no flag, over bytes: `*` and `?` match
/// `/` and a leading dot, `[…]` with `!`/`^` negation and ranges, `\`
/// escapes. Callers only pass masks [`fnmatch_portable`] accepts.
pub fn fnmatch(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0usize, 0usize);
    // The last `*` and the string position it was tried at.
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        let step = if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star = Some((pi, si));
                    pi += 1;
                    continue;
                }
                b'?' => Some(pi + 1),
                b'[' => match bracket_end(p, pi) {
                    Some(end) if bracket_matches(&p[pi + 1..end], s[si]) => Some(end + 1),
                    Some(_) => None,
                    None if s[si] == b'[' => Some(pi + 1),
                    None => None,
                },
                b'\\' if pi + 1 < p.len() => (p[pi + 1] == s[si]).then_some(pi + 2),
                c => (c == s[si]).then_some(pi + 1),
            }
        } else {
            None
        };
        match step {
            Some(next) => {
                pi = next;
                si += 1;
            }
            None => match star {
                Some((sp, ss)) => {
                    pi = sp + 1;
                    si = ss + 1;
                    star = Some((sp, ss + 1));
                }
                None => return false,
            },
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// The members between `[` and `]` (exclusive) against one byte.
fn bracket_matches(set: &[u8], c: u8) -> bool {
    let (negate, set) = match set.first() {
        Some(b'!' | b'^') => (true, &set[1..]),
        _ => (false, set),
    };
    let mut i = 0;
    let mut hit = false;
    while i < set.len() {
        let mut lo = set[i];
        if lo == b'\\' && i + 1 < set.len() {
            i += 1;
            lo = set[i];
        }
        if i + 2 < set.len() && set[i + 1] == b'-' {
            let mut hi = set[i + 2];
            let mut skip = 3;
            if hi == b'\\' && i + 3 < set.len() {
                hi = set[i + 3];
                skip = 4;
            }
            if lo <= c && c <= hi {
                hit = true;
            }
            i += skip;
        } else {
            if lo == c {
                hit = true;
            }
            i += 1;
        }
    }
    hit != negate
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fnmatch_like_the_c_library() {
        // `php -r 'var_dump(fnmatch($p, $s));'` (PHP 8.5, macOS).
        for (p, s, want) in [
            ("doc*", "docs", true),
            ("*", "", true),
            ("", "", true),
            ("", "x", false),
            ("license*", "LICENSE", false),
            ("LICENSE*", "LICENSE.txt", true),
            ("*", "a/b", true),
            ("a?c", "a/c", true),
            ("*.md", ".x.md", true),
            ("[a-c]x", "bx", true),
            ("[!a-c]x", "bx", false),
            ("[^a-c]x", "dx", true),
            ("[]]", "]", true),
            ("\\*", "*", true),
            ("\\*", "a", false),
            ("src", "src", true),
            ("a*b*c", "aXbYbc", true),
            ("a*b*c", "aXbY", false),
        ] {
            assert_eq!(fnmatch(p.as_bytes(), s.as_bytes()), want, "{p} {s}");
        }
        assert!(!fnmatch_portable("[a"));
        assert!(!fnmatch_portable("a\\"));
        assert!(!fnmatch_portable("[[:alpha:]]"));
        assert!(fnmatch_portable("doc*"));
        assert!(!fnmatch_portable("[\\]*"));
        assert!(fnmatch_portable("[a-c]*"));
        assert!(fnmatch_portable("a\\\\"));
    }

    #[test]
    fn ignore_paths_follow_the_plugin() {
        let m = json!({"extra": {"cleaner-ignore": {"a/b": true, "c/d": ["x*"], "e/f": "y", "g/h": null}}});
        assert_eq!(
            ignore_paths(&m).unwrap(),
            vec![
                ("a/b".to_owned(), Ignore::Skip),
                ("c/d".to_owned(), Ignore::Masks(vec!["x*".to_owned()])),
                ("e/f".to_owned(), Ignore::Masks(vec!["y".to_owned()])),
            ]
        );
        assert!(ignore_paths(&json!({"extra": {"cleaner-ignore": "x"}})).is_err());
        assert!(ignore_paths(&json!({"extra": {"cleaner-ignore": {"a/b": false}}})).is_err());
        assert!(ignore_paths(&json!({"extra": {"cleaner-ignore": {"a/b": ["[x"]}}})).is_err());
        let from_config =
            json!({"extra": {"cleaner-ignore": null}, "config": {"cleaner-ignore": {"a/b": true}}});
        assert_eq!(
            ignore_paths(&from_config).unwrap(),
            vec![("a/b".to_owned(), Ignore::Skip)]
        );
    }

    #[test]
    fn cleans_like_the_plugin() {
        let d = tempfile::tempdir().expect("tmp");
        let v = d.path().join("vendor");
        let p = v.join("acme/lib");
        for f in [
            "src/A.php",
            "Tests/T.php",
            "docs/a.md",
            "LICENSE.txt",
            "License.md",
            "bin/tool",
            ".gitattributes",
        ] {
            std::fs::create_dir_all(p.join(f).parent().unwrap()).unwrap();
            std::fs::write(p.join(f), "x").unwrap();
        }
        std::fs::write(
            p.join("composer.json"),
            r#"{"autoload": {"psr-4": {"A\\": "src/"}, "exclude-from-classmap": ["/Tests/", "/gone/"]}, "bin": ["bin/tool"]}"#,
        )
        .unwrap();
        let root = v.join("acme/root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("composer.json"),
            r#"{"autoload": {"psr-4": {"R\\": ""}}}"#,
        )
        .unwrap();
        std::fs::write(root.join("junk"), "x").unwrap();
        let odd = v.join("acme/odd");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("composer.json"), r#"{"autoload": {"psr-9": []}}"#).unwrap();
        let cleaned = clean(&v, &[]).unwrap();
        // Tests, gone (absent, counted), docs, License.md, .gitattributes.
        assert_eq!(cleaned.removed, 5);
        assert_eq!(
            cleaned.errors,
            vec!["unknown autoload type psr-9".to_owned()]
        );
        assert!(
            p.join("src/A.php").exists()
                && p.join("LICENSE.txt").exists()
                && p.join("bin/tool").exists()
        );
        assert!(
            !p.join("docs").exists() && !p.join("License.md").exists() && !p.join("Tests").exists()
        );
        assert!(root.join("junk").exists());
    }

    #[test]
    fn shapes_the_plugin_throws_on() {
        let ok = json!({"name": "a/b", "autoload": {"psr-4": {"A\\": ["src/", "lib/"]}, "psr-5": 3}, "bin": "x"});
        assert_eq!(shape_issue(ok.as_object().unwrap()), None);
        let bad = json!({"name": "a/b", "autoload": {"classmap": [1]}});
        assert!(shape_issue(bad.as_object().unwrap()).is_some());
    }
}
