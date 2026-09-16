//! Emulation of the `pestphp/pest-plugin` Composer plugin
//! (docs/reference/plugins/pest-plugin/, MIT): on `post-autoload-dump` its
//! `DumpCommand` writes `vendor/pest-plugins.json` — `json_encode(...,
//! JSON_PRETTY_PRINT)` of the `array_merge` of every installed package's
//! `extra.pest.plugins` list, in the local repository's order
//! (`getCanonicalPackages()`: aliases excluded), the root package last.
//! Identical from v1.0.0 to v5.0.0 (the four versions the corpus holds).
//!
//! The local repository's order is the caller's business. Composer's own
//! is not reproducible: on a fresh install `LibraryInstaller::install`
//! adds a package to the repository when its extraction promise resolves,
//! so the list follows the completion order of parallel `unzip`s (the
//! corpus shows the small `pest-plugin-laravel` before `pest` every time,
//! by size). vivacity writes the operation order — the previous
//! installed.json entries minus the removed and updated ones, then the
//! (re)installed ones — which `local_repository_order` builds; the
//! harnesses compare the file sorted, like `include_paths.php`.

use crate::error::{Error, Result};
use crate::phpjson::{php_json_encode_with, EncodeOptions};
use serde_json::Value;
use std::path::Path;

pub const PLUGIN_NAME: &str = "pestphp/pest-plugin";
pub const CACHE_FILE: &str = "pest-plugins.json";

/// `extra.pest.plugins` of one package config, as `array_merge` sees it:
/// a list contributes its values, anything else nothing.
fn plugins_of(extra: Option<&Value>) -> Vec<Value> {
    match extra
        .and_then(|e| e.get("pest"))
        .and_then(|p| p.get("plugins"))
    {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Object(map)) => map.values().cloned().collect(),
        _ => Vec::new(),
    }
}

/// `DumpCommand::execute`: `packages` are the installed packages' `extra`
/// values in local-repository order, `root_extra` the root manifest's.
pub fn write_pest_plugins(
    vendor_dir: &Path,
    packages: &[Option<&Value>],
    root_extra: Option<&Value>,
) -> Result<()> {
    let mut plugins: Vec<Value> = Vec::new();
    for extra in packages {
        plugins.extend(plugins_of(*extra));
    }
    plugins.extend(plugins_of(root_extra));
    // `json_encode($plugins, JSON_PRETTY_PRINT)`: slashes and unicode
    // escaped, four-space indentation, no trailing newline.
    let text = php_json_encode_with(
        &Value::Array(plugins),
        EncodeOptions {
            pretty: true,
            escape_slashes: true,
            escape_unicode: true,
        },
    )?;
    let path = vendor_dir.join(CACHE_FILE);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    std::fs::write(&path, text).map_err(Error::io(&path))?;
    Ok(())
}

/// `Manager::uninstall`: the cache file goes with the plugin.
pub fn remove_pest_plugins(vendor_dir: &Path) -> Result<()> {
    let path = vendor_dir.join(CACHE_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(&path)(e)),
    }
}

/// The order of `InstalledRepository::getCanonicalPackages()` after a
/// transaction: the packages already installed keep installed.json's
/// order (minus the removed and the updated ones), the installed and
/// updated ones follow in operation order.
pub fn local_repository_order<'a>(
    previous: &[&'a str],
    removed_or_updated: &[&str],
    installed: &[&'a str],
) -> Vec<&'a str> {
    let mut out: Vec<&str> = previous
        .iter()
        .copied()
        .filter(|n| !removed_or_updated.contains(n) && !installed.contains(n))
        .collect();
    out.extend(installed.iter().copied());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dump_like_the_plugin() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = json!({"pest": {"plugins": ["Pest\\Laravel\\Plugin"]}});
        let b = json!({"other": true});
        let c = json!({"pest": {"plugins": ["Pest\\Plugins\\Foo", "Pest\\Plugins\\Bar"]}});
        let root = json!({"pest": {"plugins": ["App\\PestPlugin"]}});
        write_pest_plugins(
            tmp.path(),
            &[Some(&a), Some(&b), None, Some(&c)],
            Some(&root),
        )
        .unwrap();
        let text = std::fs::read_to_string(tmp.path().join("pest-plugins.json")).unwrap();
        assert_eq!(
            text,
            "[\n    \"Pest\\\\Laravel\\\\Plugin\",\n    \"Pest\\\\Plugins\\\\Foo\",\n    \"Pest\\\\Plugins\\\\Bar\",\n    \"App\\\\PestPlugin\"\n]"
        );
        write_pest_plugins(tmp.path(), &[], None).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("pest-plugins.json")).unwrap(),
            "[]"
        );
    }

    #[test]
    fn order_after_a_transaction() {
        let order = local_repository_order(&["a/a", "b/b", "c/c"], &["b/b"], &["d/d", "b/b"]);
        assert_eq!(order, vec!["a/a", "c/c", "d/d", "b/b"]);
    }
}
