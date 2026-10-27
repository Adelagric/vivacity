//! The order in which Composer calls the plugins' listeners of one event.
//!
//! `EventDispatcher::addListener` keeps the listeners of a priority in the
//! order they were added (EventDispatcher.php:537-596), and a plugin adds
//! its own when it is activated (`PluginManager::addPlugin` →
//! `addSubscriber`, PluginManager.php:412-439). Every printing listener
//! vivacity emulates is at priority 0, so their order is the activation
//! order of their plugins:
//!
//! 1. `Factory::createComposer` loads the installed plugins
//!    (`loadInstalledPlugins`, Factory.php:448) before it purges the
//!    packages whose directory is gone (`:457`): the project's installed.json
//!    as written, then `COMPOSER_HOME`'s. `loadRepository`
//!    (PluginManager.php:495-535) sorts each repository with
//!    `PackageSorter::sortPackages` — a dev package with a branch alias is
//!    there twice (`ArrayLoader::load` returns the alias, `addPackage` adds
//!    the aliased package after it), `composer/installers` and the plugins
//!    with `extra.plugin-modifies-install-path` weigh -10000 — and
//!    registers the `composer-plugin` / `composer-installer` packages in
//!    that order. A plugin whose classes are not on disk is skipped
//!    silently (`registerPackage` with `$failOnMissingClasses` false).
//! 2. The transaction then runs one plugin operation per batch, in order
//!    (InstallationManager.php:300-323): `PluginInstaller::install`
//!    registers the plugin after laying it out, `update` deactivates the
//!    old one and registers the new one — at the end —, `uninstall` removes
//!    its listeners (PluginInstaller.php:80-120).
//!
//! A plugin `allow-plugins` refuses is never registered; the emulations
//! check that themselves, so it may stay in the list.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use vivacity_resolver::package::Package;
use vivacity_resolver::transaction::Operation;

const PLUGIN_TYPES: [&str; 2] = ["composer-plugin", "composer-installer"];

/// The packages of an installed.json (`{"packages": [...]}` or the old bare
/// list), as written — nothing purged.
pub(crate) fn installed_json_packages(composer_dir: &Path) -> Vec<Value> {
    let Some(v) = vivacity_core::jsonfile::read(&composer_dir.join("installed.json")) else {
        return Vec::new();
    };
    v.get("packages")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| v.as_array().cloned())
        .unwrap_or_default()
}

/// A `composer-plugin` / `composer-installer` package of an installed.json,
/// as `loadRepository` meets it.
#[derive(Clone)]
pub(crate) struct InstalledPlugin {
    pub(crate) name: String,
    /// Its directory is on disk (the classes can be loaded).
    pub(crate) present: bool,
    /// `extra.plugin-optional` is `true`.
    pub(crate) optional: bool,
    /// Of type `composer-plugin` (not the old `composer-installer`).
    pub(crate) composer_plugin: bool,
    /// Its `composer-plugin-api` requirement, as written.
    pub(crate) plugin_api: Option<String>,
    /// Its version, as written.
    pub(crate) version: String,
}

/// `loadRepository`'s walk of one installed.json: its plugins in
/// `PackageSorter` order, each once. `composer_dir` is the directory
/// installed.json lives in (its `install-path`s are relative to it).
pub(crate) fn sorted_plugins(packages: &[Value], composer_dir: &Path) -> Vec<InstalledPlugin> {
    struct Entry {
        name: String,
        requires: Vec<String>,
        plugin: Option<InstalledPlugin>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut weights: BTreeMap<String, i64> = BTreeMap::new();
    for p in packages {
        let Some(obj) = p.as_object() else { continue };
        let Some(name) = obj.get("name").and_then(Value::as_str) else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        let ty = obj.get("type").and_then(Value::as_str).unwrap_or("library");
        let requires: Vec<String> = obj
            .get("require")
            .and_then(Value::as_object)
            .map(|m| m.keys().map(|k| k.to_ascii_lowercase()).collect())
            .unwrap_or_default();
        if ty == "composer-plugin"
            && (name == "composer/installers"
                || obj
                    .get("extra")
                    .and_then(|e| e.get("plugin-modifies-install-path"))
                    == Some(&Value::Bool(true)))
        {
            weights.insert(name.clone(), -10000);
        }
        // The class files are on disk: the plugin's directory exists, the
        // test `purgePackages` makes too (an entry without `install-path`,
        // as Composer 1 wrote installed.json, counts as present).
        let plugin = PLUGIN_TYPES.contains(&ty).then(|| InstalledPlugin {
            name: name.clone(),
            present: obj
                .get("install-path")
                .and_then(Value::as_str)
                .is_none_or(|rel| composer_dir.join(rel).exists()),
            optional: crate::plugin_allow::optional(obj),
            composer_plugin: ty == "composer-plugin",
            plugin_api: obj
                .get("require")
                .and_then(|r| r.get("composer-plugin-api"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            version: obj
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
        // The branch alias: the same links once more in the usage table;
        // never registered (`CompleteAliasPackage` is no `CompletePackage`).
        if vivacity_resolver::loader::branch_alias(obj).is_some() {
            entries.push(Entry {
                name: name.clone(),
                requires: requires.clone(),
                plugin: None,
            });
        }
        entries.push(Entry {
            name,
            requires,
            plugin,
        });
    }
    let sortable: Vec<vivacity_autoload::sorter::SortablePackage<'_>> = entries
        .iter()
        .map(|e| vivacity_autoload::sorter::SortablePackage {
            name: &e.name,
            requires: e.requires.iter().map(String::as_str).collect(),
        })
        .collect();
    let weights: BTreeMap<&str, i64> = weights.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let mut out: Vec<InstalledPlugin> = Vec::new();
    for i in vivacity_autoload::sorter::sort_packages_weighted(&sortable, &weights) {
        if let Some(p) = &entries[i].plugin {
            if !out.iter().any(|o| o.name == p.name) {
                out.push(p.clone());
            }
        }
    }
    out
}

/// `loadRepository` on one installed.json: the plugins it registers, in
/// activation order — those whose classes are on disk (`registerPackage`
/// returns early for a name already registered).
fn load_repository(packages: &[Value], composer_dir: &Path) -> Vec<String> {
    sorted_plugins(packages, composer_dir)
        .into_iter()
        .filter(|p| p.present)
        .map(|p| p.name)
        .collect()
}

/// Step 1: the plugins `Factory::createComposer` activates, in order — the
/// project's (`installed`, read from `composer_dir`, its vendor/composer),
/// then the global ones (`global_dir`: `COMPOSER_HOME/vendor/composer`).
/// The caller skips it under `--no-plugins`.
pub(crate) fn loaded_plugins(
    installed: &[Value],
    composer_dir: &Path,
    global_dir: Option<&Path>,
) -> Vec<String> {
    let mut out = load_repository(installed, composer_dir);
    if let Some(global) = global_dir {
        for name in load_repository(&installed_json_packages(global), global) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// The global installed.json's directory: `Factory::createGlobalComposer`
/// builds a `Composer` only when `COMPOSER_HOME/composer.json` exists (it
/// gives up silently otherwise, Factory.php:489-501), with that manifest's
/// vendor-dir (a `vendor-dir` set in `COMPOSER_HOME/config.json` only is
/// not read: the default then).
pub(crate) fn global_composer_dir() -> Option<std::path::PathBuf> {
    let home = vivacity_core::fetch::composer_home()?;
    let manifest = vivacity_core::jsonfile::read(&home.join("composer.json"))?;
    let dirs = vivacity_core::dirs::Dirs::resolve(&manifest).ok()?;
    Some(dirs.composer_dir(&home))
}

/// Step 2: the transaction's plugin operations applied to the loaded list —
/// an uninstalled plugin leaves it, an updated one leaves and comes back at
/// the end, an installed one comes at the end.
pub(crate) fn activation_order(
    mut loaded: Vec<String>,
    arena: &[Package],
    operations: &[Operation],
) -> Vec<String> {
    let is_plugin = |p: &Package| PLUGIN_TYPES.contains(&p.package_type.as_str());
    for op in operations {
        match *op {
            Operation::Uninstall(p) => loaded.retain(|n| *n != arena[p].name),
            Operation::Update(i, t) => {
                loaded.retain(|n| *n != arena[i].name);
                if is_plugin(&arena[t]) {
                    loaded.push(arena[t].name.clone());
                }
            }
            // `registerPackage` returns early for a name already registered.
            Operation::Install(p) if is_plugin(&arena[p]) && !loaded.contains(&arena[p].name) => {
                loaded.push(arena[p].name.clone());
            }
            _ => {}
        }
    }
    loaded
}

/// The listeners of `names` (the emulated plugins of one event), in the
/// order `order` gives; a name `order` does not know (a plugin vivacity
/// sees as active that the list missed) comes last, in `names`' order.
pub(crate) fn listeners_in_order<'a>(order: &[String], names: &[&'a str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = order
        .iter()
        .filter_map(|n| names.iter().find(|m| **m == n.as_str()).copied())
        .collect();
    for n in names {
        if !out.contains(n) {
            // Not expected: an emulated plugin vivacity sees as installed
            // that the activation order does not know.
            if std::env::var_os("VIVACITY_TRACE").is_some() {
                eprintln!("trace: listener of {n} outside the activation order");
            }
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_installed(dir: &Path, packages: Value) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("installed.json"),
            json!({ "packages": packages }).to_string(),
        )
        .unwrap();
    }

    fn loaded(dir: &Path, global: Option<&Path>) -> Vec<String> {
        loaded_plugins(&installed_json_packages(dir), dir, global)
    }

    fn plugin(dir: &Path, name: &str, requires: Value) -> Value {
        std::fs::create_dir_all(dir.join("..").join(name)).unwrap();
        json!({"name": name, "version": "1.0.0", "type": "composer-plugin",
               "require": requires, "install-path": format!("../{name}")})
    }

    #[test]
    fn sorted_by_usage_then_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vendor/composer");
        let packages = json!([
            plugin(&dir, "codeception/c3", json!({})),
            plugin(&dir, "dealerdirect/phpcodesniffer-composer-installer", json!({})),
            plugin(&dir, "phpstan/extension-installer", json!({})),
            {"name": "slevomat/coding-standard", "version": "8.0.0", "type": "phpcodesniffer-standard",
             "require": {"dealerdirect/phpcodesniffer-composer-installer": "^1"}, "install-path": "../slevomat/coding-standard"},
        ]);
        write_installed(&dir, packages);
        assert_eq!(
            loaded(&dir, None),
            [
                "dealerdirect/phpcodesniffer-composer-installer",
                "codeception/c3",
                "phpstan/extension-installer"
            ]
        );
    }

    #[test]
    fn installers_weigh_first_and_a_missing_plugin_is_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vendor/composer");
        let mut gone = plugin(&dir, "a/gone", json!({}));
        gone["install-path"] = json!("../a/never-there");
        let packages = json!([
            gone,
            plugin(&dir, "b/plugin", json!({})),
            plugin(&dir, "z/paths", json!({})),
        ]);
        let mut packages = packages;
        packages[2]["extra"] = json!({"plugin-modifies-install-path": true});
        write_installed(&dir, packages);
        assert_eq!(loaded(&dir, None), ["z/paths", "b/plugin"]);
    }

    #[test]
    fn a_branch_alias_counts_its_links_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vendor/composer");
        // z/aliased is used by x/dev and by its branch alias: -2; m/once by
        // one package: -1. Without the alias, both -1 and m/once first.
        let packages = json!([
            plugin(&dir, "m/once", json!({})),
            plugin(&dir, "z/aliased", json!({})),
            {"name": "x/dev", "version": "dev-main", "type": "library", "install-path": "../x/dev",
             "require": {"z/aliased": "*"}, "extra": {"branch-alias": {"dev-main": "1.0.x-dev"}}},
            {"name": "y/one", "version": "1.0.0", "type": "library", "install-path": "../y/one",
             "require": {"m/once": "*"}},
        ]);
        write_installed(&dir, packages);
        assert_eq!(loaded(&dir, None), ["z/aliased", "m/once"]);
    }

    #[test]
    fn global_plugins_after_the_project_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p/vendor/composer");
        let global = tmp.path().join("home/vendor/composer");
        let local = json!([plugin(&dir, "z/local", json!({}))]);
        write_installed(&dir, local);
        let g = json!([plugin(&global, "a/global", json!({}))]);
        write_installed(&global, g);
        assert_eq!(loaded(&dir, Some(&global)), ["z/local", "a/global"]);
    }

    fn pkg(name: &str, ty: &str) -> Package {
        let mut p = Package::new(
            name,
            "1.0.0.0",
            "1.0.0",
            vivacity_resolver::package::Origin::Locked,
        );
        p.package_type = ty.to_owned();
        p
    }

    #[test]
    fn operations_move_plugins_to_the_end() {
        let arena = vec![
            pkg("a/c3", "composer-plugin"),
            pkg("a/c3", "composer-plugin"),
            pkg("b/gone", "composer-plugin"),
            pkg("c/new", "composer-plugin"),
            pkg("d/lib", "library"),
        ];
        let loaded: Vec<String> = ["a/c3", "b/gone", "e/stays"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let ops = [
            Operation::Update(0, 1),
            Operation::Uninstall(2),
            Operation::Install(4),
            Operation::Install(3),
        ];
        assert_eq!(
            activation_order(loaded, &arena, &ops),
            ["e/stays", "a/c3", "c/new"]
        );
    }

    #[test]
    fn an_entry_without_install_path_is_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vendor/composer");
        write_installed(
            &dir,
            json!([{"name": "a/old-format", "version": "1.0.0", "type": "composer-plugin"}]),
        );
        assert_eq!(loaded(&dir, None), ["a/old-format"]);
    }

    #[test]
    fn installing_a_loaded_plugin_keeps_its_place() {
        let arena = vec![pkg("a/first", "composer-plugin")];
        let loaded: Vec<String> = ["a/first", "b/second"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(
            activation_order(loaded, &arena, &[Operation::Install(0)]),
            ["a/first", "b/second"]
        );
    }

    #[test]
    fn unknown_listeners_come_last() {
        let order: Vec<String> = ["b", "x", "a"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(
            listeners_in_order(&order, &["a", "b", "c"]),
            ["b", "a", "c"]
        );
    }
}
