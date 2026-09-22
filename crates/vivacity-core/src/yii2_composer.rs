//! Emulation of the `yiisoft/yii2-composer` plugin
//! (docs/reference/plugins/yii2-composer/, BSD-3-Clause), read at 2.0.11:
//! - `Plugin::activate` (every command that loads the plugin, `install`
//!   and `dump-autoload` here): `<vendor-dir>/yiisoft/extensions.php` is
//!   created as `return [];` when missing;
//! - `Installer` handles the `yii2-extension` type — the default library
//!   path, then after each install / update / uninstall the extensions
//!   file is loaded, the entry replaced or removed, and the map saved
//!   with `var_export` (`$vendorDir . '…'` for paths under vendor/).
//!
//! An entry: `name`, `version` (normalized), `alias` when the package's
//! `psr-0` / `psr-4` give one (`@yii/foo` → path, a psr-4 list skipped),
//! `bootstrap` from `extra.bootstrap`. The map's order is Composer's
//! local-repository order — the order in which the extraction promises
//! resolved, which is not reproducible (see `pest_plugin`): vivacity
//! writes the previous order minus the removed and updated entries, then
//! the operations' order; the harness compares the file with its keys
//! sorted. Not emulated: `yiisoft/yii2-dev` (three `Yii.php` shims into
//! `yiisoft/yii2`), refused at scope time; the `postInstall` /
//! `postCreateProject` statics are scripts, never run by vivacity.

use crate::error::{Error, Result};
use crate::lock::LockPackage;
use serde_json::{Map, Value};
use std::path::Path;

pub const PLUGIN_NAME: &str = "yiisoft/yii2-composer";
pub const EXTENSION_FILE: &str = "yiisoft/extensions.php";
pub const EXTENSION_TYPE: &str = "yii2-extension";
/// The one package the installer special-cases (`linkBaseYiiFiles`).
pub const YII2_DEV: &str = "yiisoft/yii2-dev";

/// `Plugin::activate`: the file exists after any command, `return [];`
/// when nothing wrote it yet.
pub fn ensure_extensions_file(vendor: &Path) -> Result<()> {
    let file = vendor.join(EXTENSION_FILE);
    if file.is_file() {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(Error::io(dir))?;
    }
    std::fs::write(&file, "<?php\n\nreturn [];\n").map_err(Error::io(&file))
}

/// `Installer::generateDefaultAlias` + `addPackage`: one entry of the
/// map, as `var_export` will print it; paths under vendor/ carry the
/// `<vendor-dir>` marker `saveExtensions` rewrites.
pub fn extension_entry(p: &LockPackage) -> Value {
    let mut entry = Map::new();
    entry.insert("name".to_owned(), Value::String(p.name().to_owned()));
    let version =
        crate::version::normalize_pretty(p.version()).unwrap_or_else(|_| p.version().to_owned());
    entry.insert("version".to_owned(), Value::String(version));
    let mut aliases = Map::new();
    let autoload = p.raw.get("autoload");
    let pretty_name = p.raw.get("name").and_then(Value::as_str).unwrap_or("");
    // `$fs->normalizePath($this->vendorDir)`: the marker stands for it.
    let alias_path = |path: &str| -> String {
        let full = if crate::pathutil::is_absolute_path(path) {
            path.to_owned()
        } else {
            format!("<vendor-dir>/{pretty_name}/{path}")
        };
        crate::pathutil::normalize_path(&full)
    };
    for (key, psr0) in [("psr-0", true), ("psr-4", false)] {
        let Some(map) = autoload.and_then(|a| a.get(key)).and_then(Value::as_object) else {
            continue;
        };
        for (ns, path) in map {
            // psr-4 lists are ambiguous, skipped; psr-0 would TypeError.
            let Some(path) = path.as_str() else {
                continue;
            };
            let name = ns.trim_matches('\\').replace('\\', "/");
            let mut target = alias_path(path);
            if psr0 {
                target = format!("{target}/{name}");
            }
            aliases.insert(format!("@{name}"), Value::String(target));
        }
    }
    if !aliases.is_empty() {
        entry.insert("alias".to_owned(), Value::Object(aliases));
    }
    if let Some(b) = p.raw.get("extra").and_then(|e| e.get("bootstrap")) {
        entry.insert("bootstrap".to_owned(), b.clone());
    }
    Value::Object(entry)
}

/// `Installer::saveExtensions` over `entries` (package name → entry, in
/// local-repository order).
pub fn write_extensions(vendor: &Path, entries: &[(&str, Value)]) -> Result<()> {
    let mut map = Map::new();
    for (name, entry) in entries {
        map.insert((*name).to_owned(), entry.clone());
    }
    let exported = crate::runtime_stub::php_var_export(&Value::Object(map), 0)
        .replace("'<vendor-dir>", "$vendorDir . '");
    let file = vendor.join(EXTENSION_FILE);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(Error::io(dir))?;
    }
    std::fs::write(
        &file,
        format!("<?php\n\n$vendorDir = dirname(__DIR__);\n\nreturn {exported};\n"),
    )
    .map_err(Error::io(&file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Lock;
    use serde_json::json;

    fn pkg(v: Value) -> LockPackage {
        let lock =
            Lock::parse(&json!({"packages": [v], "packages-dev": []}).to_string()).expect("lock");
        lock.packages.into_iter().next().expect("one")
    }

    #[test]
    fn entry_like_the_installer() {
        let p = pkg(json!({
            "name": "yiisoft/yii2-bootstrap5", "version": "2.0.51", "type": "yii2-extension",
            "autoload": {"psr-4": {"yii\\bootstrap5\\": "src", "yii\\multi\\": ["a", "b"]}},
            "extra": {"bootstrap": "yii\\bootstrap5\\i18n\\TranslationBootstrap"}
        }));
        let e = extension_entry(&p);
        assert_eq!(
            e,
            json!({"name": "yiisoft/yii2-bootstrap5", "version": "2.0.51.0",
                   "alias": {"@yii/bootstrap5": "<vendor-dir>/yiisoft/yii2-bootstrap5/src"},
                   "bootstrap": "yii\\bootstrap5\\i18n\\TranslationBootstrap"})
        );
        // psr-0: the namespace appended to the path; an empty psr-4 path
        // is the package directory itself (`normalizePath` strips `/`).
        let p = pkg(json!({
            "name": "acme/ext", "version": "dev-main", "type": "yii2-extension",
            "autoload": {"psr-0": {"Acme\\Ext\\": "lib/"}, "psr-4": {"Acme\\": ""}}
        }));
        let e = extension_entry(&p);
        assert_eq!(e["version"], "dev-main");
        assert_eq!(
            e["alias"]["@Acme/Ext"],
            "<vendor-dir>/acme/ext/lib/Acme/Ext"
        );
        assert_eq!(e["alias"]["@Acme"], "<vendor-dir>/acme/ext");
    }

    #[test]
    fn file_like_save_extensions() {
        let d = tempfile::tempdir().expect("tmp");
        ensure_extensions_file(d.path()).expect("activate");
        assert_eq!(
            std::fs::read_to_string(d.path().join(EXTENSION_FILE)).expect("read"),
            "<?php\n\nreturn [];\n"
        );
        let p = pkg(json!({
            "name": "yiisoft/yii2-gii", "version": "2.2.7", "type": "yii2-extension",
            "autoload": {"psr-4": {"yii\\gii\\": "src"}}
        }));
        write_extensions(d.path(), &[("yiisoft/yii2-gii", extension_entry(&p))]).expect("write");
        let text = std::fs::read_to_string(d.path().join(EXTENSION_FILE)).expect("read");
        assert_eq!(
            text,
            "<?php\n\n$vendorDir = dirname(__DIR__);\n\nreturn array (\n  'yiisoft/yii2-gii' => \n  array (\n    'name' => 'yiisoft/yii2-gii',\n    'version' => '2.2.7.0',\n    'alias' => \n    array (\n      '@yii/gii' => $vendorDir . '/yiisoft/yii2-gii/src',\n    ),\n  ),\n);\n"
        );
        write_extensions(d.path(), &[]).expect("empty");
        assert_eq!(
            std::fs::read_to_string(d.path().join(EXTENSION_FILE)).expect("read"),
            "<?php\n\n$vendorDir = dirname(__DIR__);\n\nreturn array (\n);\n"
        );
    }
}
