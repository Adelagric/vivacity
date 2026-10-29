//! Emulation of `typo3/class-alias-loader` v2.0.1
//! (docs/reference/plugins/class-alias-loader/, MIT;
//! docs/plans/remaining-fallbacks.md). Its only listener,
//! `Plugin::onPreAutoloadDump` (`pre-autoload-dump`), runs
//! `ClassAliasMapGenerator::generateAliasMapFiles`:
//!
//! - every package of the package map (the root and the installed
//!   packages) that declares `extra.typo3/class-alias-loader.class-alias-maps`
//!   has its map files `require`d — PHP code of the package: such a run is
//!   handed over ([`declares_maps`]);
//! - without a map and without the root's `always-add-alias-loader`
//!   (`(bool)` of the value), it returns at once: nothing written;
//! - otherwise it picks a suffix — `config.autoloader-suffix`, else the one
//!   in `vendor/autoload.php` as it stands *before* Composer writes the new
//!   one, else `md5(uniqid('', true))`: on a fresh vendor/ a random one,
//!   not Composer's (the lock's content-hash) — writes
//!   `vendor/typo3/alias-loader-include.php` from its template and
//!   `vendor/composer/autoload_classaliasmap_static.php` (the two empty
//!   maps, `var_export`), adds the include file at the end of the root's
//!   `autoload.files` and prints "Generating empty class alias map file".

use crate::error::{Error, Result};
use md5::{Digest, Md5};
use serde_json::{Map, Value};
use std::path::Path;

pub const PLUGIN_NAME: &str = "typo3/class-alias-loader";
const KEY: &str = "typo3/class-alias-loader";
pub const INCLUDE_FILE: &str = "typo3/alias-loader-include.php";
const MAP_FILE: &str = "autoload_classaliasmap_static.php";

const INCLUDE_TEMPLATE: &str = "<?php\n\ndeclare(strict_types=1);\n\n$composerAutoLoader = require dirname(__DIR__) . '/autoload.php';\n$classAliasLoader = new TYPO3\\ClassAliasLoader\\ClassAliasLoader($composerAutoLoader);\nrequire_once dirname(__DIR__) . '/composer/autoload_classaliasmap_static.php';\n$classAliasLoader->setAliasMap(TYPO3\\ClassAliasLoader\\ClassAliasLoaderStaticInit{$suffix}::$aliasMap);\n$classAliasLoader->register({$prepend});\nTYPO3\\ClassAliasLoader\\ClassAliasMap::setClassAliasLoader($classAliasLoader);\n";

/// PHP's `(bool)` of a decoded value.
fn php_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !(s.is_empty() || s == "0"),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `$extra['typo3/class-alias-loader'][$key]` under `isset`: the block is
/// an array (an object, or a list — which has no string key) and the key
/// holds something other than `null`.
fn config_value<'a>(extra: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    extra
        .and_then(|e| e.get(KEY))
        .and_then(Value::as_object)
        .and_then(|block| block.get(key))
        .filter(|v| !v.is_null())
}

/// The package (or root) declares class alias maps: the plugin would
/// `require` them.
pub fn declares_maps(package: &Map<String, Value>) -> bool {
    config_value(package.get("extra"), "class-alias-maps").is_some()
}

/// The root's `always-add-alias-loader`.
pub fn always_add(root_manifest: &Value) -> bool {
    config_value(root_manifest.get("extra"), "always-add-alias-loader").is_some_and(php_truthy)
}

/// `config.autoloader-suffix` as `Config::get` returns it: `None` when
/// absent or `null`, `Err` for a value that is not a string (not measured).
pub fn configured_suffix(root_manifest: &Value) -> std::result::Result<Option<&str>, String> {
    match root_manifest
        .get("config")
        .and_then(|c| c.get("autoloader-suffix"))
    {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err("config.autoloader-suffix is not a string".to_owned()),
    }
}

/// The suffix the plugin picks. `existing_autoload`: `vendor/autoload.php`
/// as it is before the dump writes it, when readable. `random`: what
/// `md5(uniqid('', true))` stands for.
pub fn pick_suffix(
    configured: Option<&str>,
    existing_autoload: Option<&str>,
    random: impl FnOnce() -> String,
) -> String {
    let configured = configured.filter(|s| php_truthy(&Value::String((*s).to_owned())));
    let mut suffix: Option<String> = None;
    if configured.is_none() {
        // `preg_match('{ComposerAutoloaderInit([^:\s]+)::}', …)`: the first
        // match, `::` required.
        if let Some(text) = existing_autoload {
            let mut rest = text;
            while let Some(pos) = rest.find("ComposerAutoloaderInit") {
                let after = &rest[pos + "ComposerAutoloaderInit".len()..];
                let end = after
                    .find(|c: char| c == ':' || c.is_ascii_whitespace() || c == '\u{0b}')
                    .unwrap_or(after.len());
                if end > 0 && after[end..].starts_with("::") {
                    suffix = Some(after[..end].to_owned());
                    break;
                }
                rest = after;
            }
        }
    }
    // `if (!$suffix)`: "0" is false too.
    match suffix.filter(|s| php_truthy(&Value::String(s.clone()))) {
        Some(s) => s,
        None => configured.map_or_else(random, str::to_owned),
    }
}

/// `md5(uniqid('', true))` stands for any 32 hex digits: the harnesses mask
/// the value, nothing else reads it.
pub fn random_suffix() -> String {
    let mut hasher = Md5::new();
    hasher.update(format!("{:?}{}", std::time::SystemTime::now(), std::process::id()).as_bytes());
    format!("{:x}", hasher.finalize())
}

/// `vendor/typo3/alias-loader-include.php`: the template, `{$suffix}` and
/// `{$prepend}` replaced (`prepend-autoloader` strictly `false` gives
/// `false`).
pub fn include_file(suffix: &str, root_manifest: &Value) -> String {
    let prepend = root_manifest
        .get("config")
        .and_then(|c| c.get("prepend-autoloader"))
        == Some(&Value::Bool(false));
    INCLUDE_TEMPLATE
        .replace("{$suffix}", suffix)
        .replace("{$prepend}", if prepend { "false" } else { "true" })
}

/// `vendor/composer/autoload_classaliasmap_static.php` with both maps
/// empty (a run with maps is handed over).
pub fn map_file(suffix: &str) -> String {
    let mut maps = Map::new();
    maps.insert(
        "aliasToClassNameMapping".to_owned(),
        Value::Object(Map::new()),
    );
    maps.insert(
        "classNameToAliasMapping".to_owned(),
        Value::Object(Map::new()),
    );
    format!(
        "<?php\n\n// autoload_classaliasmap_static.php @generated by ClassAliasLoader\n\nnamespace TYPO3\\ClassAliasLoader;\n\nclass ClassAliasLoaderStaticInit{suffix}\n{{\n    public static $aliasMap={};\n}}\n",
        crate::runtime_stub::php_var_export(&Value::Object(maps), 0)
    )
}

/// `IncludeFile::register`: the root's `autoload.files` with the include
/// file appended (`$autoloadDefinition['files'][] = …`). `vendor_rel`:
/// `Config::get('vendor-dir', RELATIVE_PATHS)`. `Err` when `files` exists
/// and is not a list.
pub fn root_with_include(
    root_manifest: &Value,
    vendor_rel: &str,
) -> std::result::Result<Value, String> {
    let mut root = root_manifest.clone();
    let Some(obj) = root.as_object_mut() else {
        return Err("the root package is not an object".to_owned());
    };
    let autoload = obj
        .entry("autoload")
        .or_insert_with(|| Value::Object(Map::new()));
    if autoload.as_array().is_some_and(Vec::is_empty) {
        // An empty JSON list decodes to PHP's empty array, like `{}`.
        *autoload = Value::Object(Map::new());
    }
    let Some(autoload) = autoload.as_object_mut() else {
        return Err("the root's autoload is not an object".to_owned());
    };
    let files = autoload
        .entry("files")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(files) = files.as_array_mut() else {
        return Err("the root's autoload.files is not a list".to_owned());
    };
    files.push(Value::String(format!("{vendor_rel}/{INCLUDE_FILE}")));
    Ok(root)
}

/// The two files, each through a temporary file and a rename. `vendor`:
/// the vendor directory (`vendor/typo3` exists: the plugin is installed
/// there).
pub fn write_files(vendor: &Path, suffix: &str, root_manifest: &Value) -> Result<()> {
    let put = |path: &Path, contents: &str| -> Result<()> {
        if std::fs::read(path).is_ok_and(|b| b == contents.as_bytes()) {
            return Ok(());
        }
        let tmp = path.with_extension("php.vivacity-tmp");
        std::fs::write(&tmp, contents).map_err(Error::io(&tmp))?;
        std::fs::rename(&tmp, path).map_err(Error::io(path))
    };
    put(
        &vendor.join(INCLUDE_FILE),
        &include_file(suffix, root_manifest),
    )?;
    let composer = vendor.join("composer");
    std::fs::create_dir_all(&composer).map_err(Error::io(&composer))?;
    put(&composer.join(MAP_FILE), &map_file(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_suffix_comes_from_autoload_php_then_the_config_then_chance() {
        let autoload = "<?php\n\nreturn ComposerAutoloaderInitabc123::getLoader();\n";
        assert_eq!(pick_suffix(None, Some(autoload), || "r".into()), "abc123");
        assert_eq!(
            pick_suffix(Some("cfg"), Some(autoload), || "r".into()),
            "cfg"
        );
        assert_eq!(pick_suffix(None, None, || "r".into()), "r");
        // `::` required; a later occurrence may match.
        let two = "ComposerAutoloaderInitnope\nComposerAutoloaderInitok::x";
        assert_eq!(pick_suffix(None, Some(two), || "r".into()), "ok");
        // "0" is false in PHP, from the file or the config.
        assert_eq!(
            pick_suffix(None, Some("ComposerAutoloaderInit0::"), || "r".into()),
            "r"
        );
        assert_eq!(
            pick_suffix(Some("0"), Some(autoload), || "r".into()),
            "abc123"
        );
        assert_eq!(pick_suffix(Some(""), None, || "r".into()), "r");
        assert_eq!(random_suffix().len(), 32);
    }

    #[test]
    fn the_files_as_the_plugin_writes_them() {
        assert_eq!(
            map_file("s1"),
            "<?php\n\n// autoload_classaliasmap_static.php @generated by ClassAliasLoader\n\nnamespace TYPO3\\ClassAliasLoader;\n\nclass ClassAliasLoaderStaticInits1\n{\n    public static $aliasMap=array (\n  'aliasToClassNameMapping' => \n  array (\n  ),\n  'classNameToAliasMapping' => \n  array (\n  ),\n);\n}\n"
        );
        let include = include_file("s1", &json!({}));
        assert!(include.contains(
            "ClassAliasLoaderStaticInits1::$aliasMap);\n$classAliasLoader->register(true);"
        ));
        let off = include_file("s1", &json!({"config": {"prepend-autoloader": false}}));
        assert!(off.contains("->register(false);"));
        let string = include_file("s1", &json!({"config": {"prepend-autoloader": "false"}}));
        assert!(string.contains("->register(true);"));
    }

    #[test]
    fn configuration_reads() {
        assert!(always_add(
            &json!({"extra": {KEY: {"always-add-alias-loader": true}}})
        ));
        assert!(always_add(
            &json!({"extra": {KEY: {"always-add-alias-loader": "yes"}}})
        ));
        assert!(!always_add(
            &json!({"extra": {KEY: {"always-add-alias-loader": "0"}}})
        ));
        assert!(!always_add(
            &json!({"extra": {KEY: "always-add-alias-loader"}})
        ));
        let pkg = |v: Value| v.as_object().cloned().unwrap();
        assert!(declares_maps(&pkg(
            json!({"extra": {KEY: {"class-alias-maps": ["m.php"]}}})
        )));
        assert!(declares_maps(&pkg(
            json!({"extra": {KEY: {"class-alias-maps": "m.php"}}})
        )));
        assert!(!declares_maps(&pkg(
            json!({"extra": {KEY: {"class-alias-maps": null}}})
        )));
        assert!(!declares_maps(&pkg(json!({"extra": {}}))));
        assert_eq!(configured_suffix(&json!({})), Ok(None));
        assert!(configured_suffix(&json!({"config": {"autoloader-suffix": 5}})).is_err());
    }

    #[test]
    fn the_include_file_goes_last_in_the_root_files() {
        let root = root_with_include(
            &json!({"autoload": {"psr-4": {"A\\": "src/"}, "files": ["a.php"]}}),
            "vendor",
        )
        .unwrap();
        assert_eq!(
            root["autoload"]["files"],
            json!(["a.php", "vendor/typo3/alias-loader-include.php"])
        );
        let bare = root_with_include(&json!({}), "vendor").unwrap();
        assert_eq!(
            bare["autoload"],
            json!({"files": ["vendor/typo3/alias-loader-include.php"]})
        );
        assert!(root_with_include(&json!({"autoload": {"files": "a.php"}}), "vendor").is_err());
    }
}
