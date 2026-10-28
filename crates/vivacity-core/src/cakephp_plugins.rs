//! Emulation of `cakephp/plugin-installer` 2.0.2 (docs/reference/plugins/
//! cakephp-plugin-installer/, MIT). Two listeners around the autoloader
//! dump, both silent:
//!
//! - `PRE_AUTOLOAD_DUMP` (`Plugin::preAutoloadDump`): the application
//!   plugins under `extra.plugin-paths` (default `plugins`, relative to the
//!   parent of the real vendor directory) are added to the root's `psr-4`
//!   autoload (`<Name>\` → `<path>/src`), and to its `autoload-dev`
//!   (`<Name>\Test\` → `<path>/tests`) when they have a `tests/` — unless
//!   the root already maps that namespace.
//! - `POST_AUTOLOAD_DUMP` (`Plugin::postAutoloadDump`):
//!   `<vendor-dir>/cakephp-plugins.php`, the `cakephp-plugin` packages of
//!   the local repository by primary namespace, then the application
//!   plugins (`+=`: a package keeps its name), `ksort`ed, the root replaced
//!   by `$baseDir . '`.

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const PLUGIN_NAME: &str = "cakephp/plugin-installer";

/// `extra.plugin-paths` when not `empty()`, else `None` (the default).
/// `Err`: a shape the plugin throws on, or an absolute path (the two
/// listeners then disagree, and the file would hold the machine's path).
pub fn plugin_paths(manifest: &Value) -> std::result::Result<Option<Vec<String>>, String> {
    let v = manifest.get("extra").and_then(|e| e.get("plugin-paths"));
    let list = match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => return Ok(None),
        Some(Value::String(s)) if s.is_empty() || s == "0" => return Ok(None),
        Some(Value::Array(a)) if a.is_empty() => return Ok(None),
        Some(Value::Object(o)) if o.is_empty() => return Ok(None),
        Some(Value::Array(a)) => a.clone(),
        // `foreach` over an array with string keys: its values.
        Some(Value::Object(o)) => o.values().cloned().collect(),
        Some(_) => return Err("extra.plugin-paths is not a list (the plugin throws)".to_owned()),
    };
    let mut out = Vec::new();
    for p in &list {
        let Some(s) = p.as_str() else {
            return Err("extra.plugin-paths holds a value that is not a string".to_owned());
        };
        if is_absolute(s) {
            return Err(format!(
                "extra.plugin-paths `{s}` is absolute (not emulated)"
            ));
        }
        out.push(s.to_owned());
    }
    Ok(Some(out))
}

/// `getFullPath`'s test: `{^(?:/|[a-z]:|[a-z0-9.]+://)}i`.
fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    if b.first() == Some(&b'/') {
        return true;
    }
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true;
    }
    match p.find("://") {
        Some(i) if i > 0 => p[..i]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.'),
        _ => false,
    }
}

/// `getPrimaryNamespace`: the first `psr-4` map of the package's autoload
/// — its only key, else the key of the first `src`-like path, else the key
/// of `''` or `'.'` (the latter wins) —, trimmed of `\`. `Err`: the
/// RuntimeException (no namespace), or a shape PHP warns on.
pub fn primary_namespace(package: &Map<String, Value>) -> std::result::Result<String, String> {
    let name = package.get("name").and_then(Value::as_str).unwrap_or("?");
    let none = || format!("Unable to get primary namespace for package {name}");
    let Some(Value::Object(autoload)) = package.get("autoload") else {
        return Err(none());
    };
    let Some(map) = autoload.get("psr-4") else {
        return Err(none());
    };
    let map: Vec<(&String, &Value)> = match map {
        Value::Object(m) => m.iter().collect(),
        _ => return Err(none()),
    };
    let ns = if map.len() == 1 {
        Some(map[0].0.clone())
    } else {
        if map.iter().any(|(_, v)| !v.is_string()) {
            return Err(format!(
                "package {name}: a psr-4 path that is not a string (preg_grep warns)"
            ));
        }
        let src = map.iter().find(|(_, v)| {
            v.as_str().is_some_and(|s| {
                let s = s.strip_prefix("./").unwrap_or(s);
                s == "src" || s == "src/"
            })
        });
        match src {
            Some((k, _)) => Some((*k).clone()),
            None => {
                let mut found = None;
                for path in ["", "."] {
                    if let Some((k, _)) = map.iter().find(|(_, v)| v.as_str() == Some(path)) {
                        found = Some((*k).clone());
                    }
                }
                found
            }
        }
    };
    // `if (!$primaryNs)`: null, "" and "0".
    match ns {
        Some(ns) if !ns.is_empty() && ns != "0" => Ok(ns.trim_matches('\\').to_owned()),
        _ => Err(none()),
    }
}

/// A plugin name PHP would make an integer key (a `str_replace` on it then
/// throws under `strict_types`) or compare as a number in `ksort`
/// (`is_numeric`: optional whitespace, sign, digits with a dot and an
/// exponent).
pub fn numeric_name(name: &str) -> bool {
    crate::vendor_cleaner::php_int_key(name) || php_is_numeric(name)
}

fn php_is_numeric(s: &str) -> bool {
    const WS: [char; 6] = [' ', '\t', '\n', '\r', '\x0B', '\x0C'];
    let t = s.trim_matches(WS);
    let b = t.strip_prefix(['+', '-']).unwrap_or(t).as_bytes();
    let int = b.iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = int;
    let mut frac = 0;
    if b.get(i) == Some(&b'.') {
        frac = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        i += 1 + frac;
    }
    if int + frac == 0 {
        return false;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = b
            .get(j..)
            .map(|r| r.iter().take_while(|c| c.is_ascii_digit()).count())
            .unwrap_or(0);
        if exp == 0 {
            return false;
        }
        i = j + exp;
    }
    i == b.len()
}

/// `findAppPlugins($path, $keepLegacyDirectories)`: `(name, path)` in
/// `DirectoryIterator` order — `<Name>` with a `src/`, or `<Vendor>/<Name>`
/// below a directory without one, or that directory itself when
/// `keep_legacy` and it holds no plugin. Hidden entries and files skipped.
pub fn find_app_plugins(path: &str, keep_legacy: bool) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (name, plugin_path) in children(path)? {
        if Path::new(&format!("{plugin_path}/src")).is_dir() {
            out.push((name, plugin_path));
            continue;
        }
        let mut vendor = Vec::new();
        for (sub, sub_path) in children(&plugin_path)? {
            if Path::new(&format!("{sub_path}/src")).is_dir() {
                vendor.push((format!("{name}/{sub}"), sub_path));
            }
        }
        if !vendor.is_empty() {
            out.extend(vendor);
        } else if keep_legacy {
            out.push((name, plugin_path));
        }
    }
    Ok(out)
}

/// The non-hidden directories (followed) of `dir`, as `DirectoryIterator`
/// yields them: `(name, "<dir>/<name>")`.
fn children(dir: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let p = Path::new(dir);
    for e in std::fs::read_dir(p).map_err(Error::io(p))? {
        let e = e.map_err(Error::io(p))?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !e.path().is_dir() {
            continue;
        }
        out.push((name.clone(), format!("{dir}/{name}")));
    }
    Ok(out)
}

/// `dirname()` of a path string, as PHP computes it for an absolute path.
fn dirname(p: &str) -> String {
    crate::glob::php_dirname(p)
}

/// The real vendor directory as a string (`realpath`).
pub fn real_vendor(vendor_dir: &Path) -> Result<String> {
    let real = crate::pathutil::canonicalize(vendor_dir).map_err(Error::io(vendor_dir))?;
    Ok(real.to_string_lossy().into_owned())
}

/// `preAutoloadDump`: the root manifest with the application plugins'
/// autoload added. `real_vendor` is `realpath(vendor-dir)`.
pub fn root_with_app_plugins(
    manifest: &Value,
    real_vendor: &str,
    paths: &Option<Vec<String>>,
) -> Result<Value> {
    let root = format!("{}/", dirname(real_vendor));
    let default = vec!["plugins".to_owned()];
    let mut m = manifest.clone();
    for plugins_path in paths.as_ref().unwrap_or(&default) {
        let plugin_dir = format!("{root}{plugins_path}");
        if !Path::new(&plugin_dir).is_dir() {
            continue;
        }
        for (name, path) in find_app_plugins(&plugin_dir, false)? {
            let ns = format!("{}\\", name.replace('/', "\\"));
            let rel = path
                .get(root.len()..)
                .unwrap_or_default()
                .trim_matches('/')
                .to_owned();
            add_psr4(&mut m, "autoload", &ns, format!("{rel}/src"));
            if Path::new(&format!("{path}/tests")).is_dir() {
                add_psr4(
                    &mut m,
                    "autoload-dev",
                    &format!("{ns}Test\\"),
                    format!("{rel}/tests"),
                );
            }
        }
    }
    Ok(m)
}

/// `if (!isset($autoload['psr-4'][$ns])) $autoload['psr-4'][$ns] = $path;`
fn add_psr4(manifest: &mut Value, key: &str, ns: &str, path: String) {
    let Some(root) = manifest.as_object_mut() else {
        return;
    };
    let autoload = root.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !autoload.is_object() {
        *autoload = Value::Object(Map::new());
    }
    let Some(autoload) = autoload.as_object_mut() else {
        return;
    };
    let psr4 = autoload
        .entry("psr-4")
        .or_insert_with(|| Value::Object(Map::new()));
    if !psr4.is_object() {
        *psr4 = Value::Object(Map::new());
    }
    if let Some(psr4) = psr4.as_object_mut() {
        if psr4.get(ns).is_none_or(Value::is_null) {
            psr4.insert(ns.to_owned(), Value::String(path));
        }
    }
}

/// `postAutoloadDump` → `findPlugins` → `writeConfigFile`: the contents of
/// `<vendor-dir>/cakephp-plugins.php`. `packages` is the local repository
/// (pretty names), `real_vendor` is `realpath(vendor-dir)`.
pub fn config_file(
    packages: &[&Map<String, Value>],
    real_vendor: &str,
    paths: &Option<Vec<String>>,
) -> std::result::Result<String, String> {
    let mut plugins: Vec<(String, String)> = Vec::new();
    for p in packages {
        if p.get("type").and_then(Value::as_str) != Some("cakephp-plugin") {
            continue;
        }
        let ns = primary_namespace(p)?;
        let pretty = p.get("name").and_then(Value::as_str).unwrap_or_default();
        let path = format!("{real_vendor}/{pretty}");
        match plugins.iter_mut().find(|(k, _)| *k == ns) {
            Some(slot) => slot.1 = path,
            None => plugins.push((ns, path)),
        }
    }
    let base = dirname(real_vendor);
    let dirs: Vec<String> = match paths {
        None => vec![format!("{base}/plugins")],
        Some(list) => list.clone(),
    };
    for d in dirs {
        let full = full_path(&d, real_vendor);
        if !Path::new(&full).is_dir() {
            continue;
        }
        for (name, path) in find_app_plugins(&full, true).map_err(|e| e.to_string())? {
            if !plugins.iter().any(|(k, _)| *k == name) {
                plugins.push((name, path));
            }
        }
    }
    if let Some((name, _)) = plugins.iter().find(|(k, _)| numeric_name(k)) {
        return Err(format!(
            "cakephp plugin name `{name}` is numeric (TypeError in the plugin)"
        ));
    }
    plugins.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut data = String::new();
    for (name, path) in &plugins {
        let path = format!("{}/", path.replace('\\', "/")).replace("//", "/");
        let name = name.replace('\\', "/");
        data.push_str(&format!("        '{name}' => '{path}',\n"));
    }
    let contents = format!(
        "<?php\n$baseDir = dirname(dirname(__file__));\n\nreturn [\n    'plugins' => [\n{data}    ],\n];\n"
    )
    .replace("__file__", "__FILE__");
    let root = base.replace("//", "/").replace('\\', "/");
    Ok(contents.replace(&format!("'{root}"), "$baseDir . '"))
}

/// `getFullPath($path, $vendorDir)`.
fn full_path(path: &str, real_vendor: &str) -> String {
    if is_absolute(path) {
        return path.trim_end_matches('/').to_owned();
    }
    let p = path.strip_prefix("./").unwrap_or(path);
    format!("{}/{p}", dirname(real_vendor))
        .trim_end_matches([' ', '\t', '\n', '\r', '\0', '\x0B'])
        .to_owned()
}

/// Writes the file (`file_put_contents`, every dump).
pub fn write_config_file(vendor_dir: &Path, contents: &str) -> Result<PathBuf> {
    let path = vendor_dir.join("cakephp-plugins.php");
    std::fs::write(&path, contents).map_err(Error::io(&path))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primary_namespace_like_the_plugin() {
        let p = |a: Value| {
            let v = json!({"name": "a/b", "autoload": a});
            primary_namespace(v.as_object().unwrap())
        };
        assert_eq!(p(json!({"psr-4": {"A\\B\\": "src/"}})).unwrap(), "A\\B");
        assert_eq!(
            p(json!({"psr-4": {"A\\T\\": "tests/", "A\\": "./src"}})).unwrap(),
            "A"
        );
        assert_eq!(
            p(json!({"psr-4": {"A\\": "", "B\\": ".", "C\\": "lib"}})).unwrap(),
            "B"
        );
        assert!(p(json!({"classmap": ["x"]})).is_err());
        assert!(p(json!({"psr-4": {"A\\": ["src"], "B\\": "lib"}})).is_err());
    }

    #[test]
    fn writes_the_file_like_the_plugin() {
        let d = tempfile::tempdir().unwrap();
        let root = crate::pathutil::canonicalize(d.path()).unwrap();
        let root = root.to_string_lossy();
        let vendor = format!("{root}/vendor");
        for dir in [
            "plugins/Foo/src",
            "plugins/Vendor/Bar/src",
            "plugins/Legacy",
            "plugins/.hidden/src",
            "plugins/Acme/Plug/src",
        ] {
            std::fs::create_dir_all(format!("{root}/{dir}")).unwrap();
        }
        std::fs::create_dir_all(&vendor).unwrap();
        let pkg = json!({"name": "acme/plug", "type": "cakephp-plugin", "autoload": {"psr-4": {"Acme\\Plug\\": "src/"}}});
        let file = config_file(&[pkg.as_object().unwrap()], &vendor, &None).unwrap();
        // The package's key is `Acme\Plug`, the application plugin's
        // `Acme/Plug`: both stay, and both are written as `Acme/Plug`.
        assert_eq!(
            file,
            "<?php\n$baseDir = dirname(dirname(__FILE__));\n\nreturn [\n    'plugins' => [\n        'Acme/Plug' => $baseDir . '/plugins/Acme/Plug/',\n        'Acme/Plug' => $baseDir . '/vendor/acme/plug/',\n        'Foo' => $baseDir . '/plugins/Foo/',\n        'Legacy' => $baseDir . '/plugins/Legacy/',\n        'Vendor/Bar' => $baseDir . '/plugins/Vendor/Bar/',\n    ],\n];\n"
        );
        let m = root_with_app_plugins(
            &json!({"autoload": {"psr-4": {"App\\": "src/"}}}),
            &vendor,
            &None,
        )
        .unwrap();
        let psr4 = &m["autoload"]["psr-4"];
        assert_eq!(psr4["Foo\\"], "plugins/Foo/src");
        assert_eq!(psr4["Acme\\Plug\\"], "plugins/Acme/Plug/src");
        assert_eq!(psr4["Vendor\\Bar\\"], "plugins/Vendor/Bar/src");
        assert!(psr4.get("Legacy\\").is_none());
    }

    #[test]
    fn numeric_names_like_php() {
        // `php -r 'var_dump(is_numeric($n));'`
        for n in ["7", "-3", "1.5", "1e3", " 2", ".5", "1."] {
            assert!(numeric_name(n), "{n}");
        }
        for n in ["Foo", "inf", "NaN", "1e", "e3", "", "0x1A", "1_000"] {
            assert!(!numeric_name(n), "{n}");
        }
    }

    #[test]
    fn plugin_paths_shapes() {
        assert_eq!(plugin_paths(&json!({})).unwrap(), None);
        assert_eq!(
            plugin_paths(&json!({"extra": {"plugin-paths": []}})).unwrap(),
            None
        );
        assert!(plugin_paths(&json!({"extra": {"plugin-paths": ["/opt"]}})).is_err());
        assert!(plugin_paths(&json!({"extra": {"plugin-paths": ["C:x"]}})).is_err());
        assert!(plugin_paths(&json!({"extra": {"plugin-paths": "plugins"}})).is_err());
        assert_eq!(
            plugin_paths(&json!({"extra": {"plugin-paths": ["./p"]}})).unwrap(),
            Some(vec!["./p".to_owned()])
        );
    }
}
