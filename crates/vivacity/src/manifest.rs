//! The root manifest as every command reads it: Composer's own refusals,
//! the config keys that change a run, and wikimedia/composer-merge-plugin's
//! merged root.

use super::*;

/// `config.<key>` of the manifest, as `Config::get` reads a boolean.
pub(crate) fn config_bool(manifest: &serde_json::Value, key: &str) -> bool {
    manifest
        .get("config")
        .and_then(|c| c.get(key))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// `InstallCommand` + `Installer::setClassMapAuthoritative`: the option,
/// `config.optimize-autoloader`, or anything that makes the class map
/// authoritative — which turns optimization on.
pub(crate) fn effective_optimize(
    manifest: &serde_json::Value,
    optimize: bool,
    authoritative: bool,
) -> bool {
    optimize
        || authoritative
        || config_bool(manifest, "classmap-authoritative")
        || config_bool(manifest, "optimize-autoloader")
}

/// `wikimedia/composer-merge-plugin` on this run: `Some` when the plugin is
/// locked (wanted for the mode), allowed, plugins enabled and a merge
/// configuration declared — the merged root (`extra.branch-alias` kept
/// from the original: `RootPackageLoader` reads it before the plugin
/// runs). `Err`: the reason vivacity cannot emulate this project.
pub(crate) fn merge_plugin_root(
    project: &std::path::Path,
    lock: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> Result<Option<vivacity_resolver::merge_plugin::Merged>, String> {
    use vivacity_resolver::merge_plugin;
    // `PluginManager::loadInstalledPlugins` activates the plugin from
    // installed.json whatever the mode (a dev-only plugin still merges at
    // INIT on an `install --no-dev` over a dev vendor), and an install
    // from the lock activates it as it lands.
    let present = lock
        .wanted_packages(with_dev)
        .any(|p| p.name() == merge_plugin::PLUGIN_NAME)
        || installed_packages(project, manifest)
            .iter()
            .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(merge_plugin::PLUGIN_NAME));
    merge_root_when(project, manifest, with_dev, plugins_enabled && present)
}

/// The merge itself, once the caller knows whether Composer would load
/// the plugin on this run (`present`): `None` unless it is allowed and a
/// merge configuration is declared.
pub(crate) fn merge_root_when(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    with_dev: bool,
    present: bool,
) -> Result<Option<vivacity_resolver::merge_plugin::Merged>, String> {
    use vivacity_resolver::merge_plugin::{self, Settings};
    if !present
        || !matches!(
            vivacity_core::layout::plugin_allowed(manifest, merge_plugin::PLUGIN_NAME),
            vivacity_core::layout::PluginVerdict::Allowed
        )
        || !Settings::declared(manifest)
    {
        return Ok(None);
    }
    let root = vivacity_core::state::RootPackage::detect(manifest, project, with_dev);
    let mut merged = merge_plugin::merge(
        project,
        manifest,
        &root.version,
        &root.pretty_version,
        with_dev,
    )?;
    let original_alias = manifest
        .get("extra")
        .and_then(|e| e.get("branch-alias"))
        .cloned();
    if let Some(extra) = merged
        .manifest
        .get_mut("extra")
        .and_then(|e| e.as_object_mut())
    {
        match original_alias {
            Some(a) => {
                extra.insert("branch-alias".to_owned(), a);
            }
            None => {
                extra.remove("branch-alias");
            }
        }
    }
    Ok(Some(merged))
}

/// composer-merge-plugin required, allowed and configured but not
/// installed yet: Composer does not load it (`PluginManager` reads
/// installed.json), resolves the root unmerged, then — once the plugin
/// lands — runs the plugin's implicit `update` of the merged names, a
/// second resolution. Not emulated: refused with the reason.
pub(crate) fn refuse_merge_plugin_resolution(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_resolver::merge_plugin::{self, Settings};
    let required = ["require", "require-dev"].iter().any(|k| {
        manifest
            .get(k)
            .and_then(|m| m.get(merge_plugin::PLUGIN_NAME))
            .is_some()
    });
    let installed = vivacity_core::scope::active_plugins(project, manifest, plugins_enabled)
        .iter()
        .any(|p| p == merge_plugin::PLUGIN_NAME);
    if plugins_enabled
        && required
        && !installed
        && matches!(
            vivacity_core::layout::plugin_allowed(manifest, merge_plugin::PLUGIN_NAME),
            vivacity_core::layout::PluginVerdict::Allowed
        )
        && Settings::declared(manifest)
    {
        anyhow::bail!(
            "wikimedia/composer-merge-plugin is not installed yet: Composer would resolve without the merge, then run the plugin's implicit update once it is installed (not emulated) — run `composer update` for this project"
        );
    }
    Ok(())
}

/// `<vendor-dir>/composer` of a project before its layout is resolved
/// (reading installed.json): the configured directory, or `vendor/composer`
/// when the configuration is one vivacity refuses (the layout will say so).
pub(crate) fn composer_dir_of(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> std::path::PathBuf {
    vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .composer_dir(project)
}

/// `COMPOSER=<file>` points Composer at another manifest, and at the lock
/// beside it. `remove` and `require` already refuse it; `install`,
/// `dump-autoload` and `update` read `composer.json` regardless — so they
/// worked on a file Composer never looks at, and the root-manifest refusals
/// below were decided on the wrong one.
pub(crate) fn refuse_alternate_manifest() -> anyhow::Result<()> {
    if let Some(f) = std::env::var("COMPOSER").ok().map(|f| f.trim().to_owned()) {
        if !f.is_empty() && f != "composer.json" && f != "./composer.json" {
            anyhow::bail!("COMPOSER={f}: an alternate manifest is not supported yet");
        }
    }
    Ok(())
}

/// `RootPackageLoader::load`'s own refusals, which Composer raises for EVERY
/// command because `Factory::createComposer` loads the root package: an invalid
/// root name, a package that requires itself, an invalid link name. vivacity
/// only built a root package for a resolution, so `install` and
/// `dump-autoload` accepted manifests the reference refuses.
///
/// The message is Composer's, printed without Symfony Console's box — the
/// deviation already taken for path repositories, and measured: the box is
/// wrapped to the terminal width, followed by the command synopsis, and
/// sometimes chained. Exit 1, as Composer, and nothing written.
pub(crate) fn refuse_invalid_root(manifest: &serde_json::Value) -> Option<i32> {
    let err = vivacity_resolver::root::manifest_error(manifest)?;
    eprintln!("{err}");
    Some(1)
}

/// PHP's truthiness: `false`, `0`, `0.0`, `""`, `"0"`, `[]` and `null` are
/// false, everything else is true.
pub(crate) fn php_truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        serde_json::Value::String(s) => !s.is_empty() && s != "0",
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

/// `Config::get('lock')`: project then global config; `"false"` and PHP's
/// falsy values disable it.
pub(crate) fn config_lock_enabled(manifest_text: &str) -> bool {
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap_or_default();
    match require::config_value(&manifest, "lock") {
        None => true,
        Some(serde_json::Value::String(s)) => s != "false" && !(s.is_empty() || s == "0"),
        Some(serde_json::Value::Bool(b)) => b,
        Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Array(a)) => !a.is_empty(),
        Some(serde_json::Value::Object(m)) => !m.is_empty(),
    }
}
