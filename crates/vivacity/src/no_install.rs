//! `update`, `require` and `remove` with `--no-install`: the end of
//! `Installer::run` when nothing is installed (docs/plans/no-install-listeners.md).
//!
//! `Installer::run` with `setInstall(false)` skips the dump
//! (Installer.php:305-307) and returns from `doUpdate` once the lock is
//! written (:684-689), but it still prints the funding count (:412-436) and
//! dispatches `POST_UPDATE_CMD` (:438-441) — `--no-scripts` only silences
//! composer.json's scripts (EventDispatcher.php:585); `--dry-run` turns it
//! off (:292-298). The listeners are those of the plugins
//! `Factory::createComposer` loaded from installed.json
//! (`loadInstalledPlugins`, Factory.php:448), and every emulated one reads
//! `getLocalRepository()` — installed.json as it was, purged of the
//! packages whose directory is gone (`purgePackages`, :457), dev packages
//! included whatever `--no-dev` says, without the lock's inline aliases
//! (only an install re-adds them), each branch alias loaded before its
//! package (`ArrayRepository::addPackage`, ArrayRepository.php:215-231).
//! A plugin the command adds is not loaded; one it removes still is.

use super::*;

/// The local repository `POST_UPDATE_CMD`'s listeners read without an
/// install, and the order their plugins were activated in.
pub(crate) struct LocalRepository {
    /// `loadInstalledPlugins`: installed.json as written (project, then
    /// `COMPOSER_HOME`), sorted by `PackageSorter` — no transaction follows.
    pub(crate) listeners: Vec<String>,
    /// installed.json purged, as a lock whose `packages` hold everything.
    pub(crate) lock: vivacity_core::lock::Lock,
    /// The same entries, as read.
    pub(crate) entries: Vec<serde_json::Value>,
}

pub(crate) fn local_repository(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> LocalRepository {
    let composer_dir = composer_dir_of(project, manifest);
    let raw = plugin_order::installed_json_packages(&composer_dir);
    let listeners = if plugins_enabled {
        plugin_order::loaded_plugins(
            &raw,
            &composer_dir,
            plugin_order::global_composer_dir().as_deref(),
        )
    } else {
        Vec::new()
    };
    let entries = purge_missing(&composer_dir, raw);
    let lock = vivacity_core::lock::Lock::from_value(
        &serde_json::json!({"packages": entries, "packages-dev": []}),
    );
    LocalRepository {
        listeners,
        lock,
        entries,
    }
}

fn allowed(manifest: &serde_json::Value, plugin: &str) -> bool {
    matches!(
        vivacity_core::layout::plugin_allowed(manifest, plugin),
        vivacity_core::layout::PluginVerdict::Allowed
    )
}

/// The install paths of the local repository, as the listeners ask
/// `InstallationManager::getInstallPath` for them (the current manifest's
/// installers and directories).
fn local_layout(
    project: &std::path::Path,
    repo: &LocalRepository,
    manifest: &serde_json::Value,
) -> Result<vivacity_core::layout::Layout, Vec<String>> {
    vivacity_core::layout::Layout::resolve(project, &repo.lock, manifest, true, true)
}

/// `plugin`'s listener runs: Composer loaded the plugin from installed.json
/// (`repo.listeners`) and `allow-plugins` allows it. The one rule of the
/// layout check before resolving and of the listeners themselves.
fn listening(repo: &LocalRepository, manifest: &serde_json::Value, plugin: &str) -> bool {
    repo.listeners.iter().any(|p| p == plugin) && allowed(manifest, plugin)
}

/// Decided before resolving (`resolution_fallback`): a loaded, allowed
/// listener that needs install paths (dealerdirect, the extension
/// installers) on a local repository whose layout vivacity cannot compute
/// goes to Composer, named.
pub(crate) fn layout_issue(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> Option<vivacity_core::scope::ScopeIssue> {
    let repo = local_repository(project, manifest, true);
    if !vivacity_core::scope::POST_UPDATE_FILE_LISTENERS
        .iter()
        .any(|p| listening(&repo, manifest, p))
    {
        return None;
    }
    local_layout(project, &repo, manifest).err().map(|issues| {
        vivacity_core::scope::ScopeIssue::Layout(format!(
            "the install paths the post-update-cmd listeners read cannot be computed: {}",
            issues.join("; ")
        ))
    })
}

/// php-http/discovery loaded and allowed: its `postUpdate`
/// (Plugin.php:145-240 at 1.20.0) asks the local repository — not the new
/// lock — for an implementation of every abstraction it looks after, and
/// may then move a package to `require`, add one, edit composer.json and
/// run a cloned `Installer` again; measured, with the implementation absent
/// from the local repository, and with a `project` root whose only
/// implementation is a dev package: never converging under `--no-install`.
/// When it would act ([`crate::discovery::post_update_issue`]) the command
/// goes to Composer. Decided on the manifest the command resolves
/// (`require`'s edit included), before the lock is written.
pub(crate) fn discovery_issue(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> Option<vivacity_core::scope::ScopeIssue> {
    use crate::discovery::DISCOVERY;
    let repo = local_repository(project, manifest, true);
    if !repo.listeners.iter().any(|p| p == DISCOVERY) || !allowed(manifest, DISCOVERY) {
        return None;
    }
    let version = repo
        .lock
        .packages
        .iter()
        .find(|p| p.name() == DISCOVERY)
        .map(|p| p.version().to_owned());
    let packages: Vec<crate::discovery::RepoPackage> = repo
        .entries
        .iter()
        .flat_map(crate::discovery::RepoPackage::from_entry)
        .collect();
    // `InstalledFilesystemRepository::getDevPackageNames`: installed.json's
    // `dev-package-names`.
    let dev_names =
        vivacity_core::jsonfile::read(&composer_dir_of(project, manifest).join("installed.json"))
            .and_then(|v| v.get("dev-package-names").cloned())
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.to_ascii_lowercase())
            .collect();
    crate::discovery::post_update_issue(
        version.as_deref(),
        &packages,
        &dev_names,
        manifest,
        || vivacity_core::scope::discovery_watches(manifest, &repo.entries),
        "not emulated without an install",
    )
}

/// Decided before resolving, for a command that dispatches
/// `POST_UPDATE_CMD` without an install (`resolution_fallback`):
///
/// - an emulated `POST_UPDATE_CMD` listener loaded from `COMPOSER_HOME`
///   (`PluginManager::loadInstalledPlugins` loads the global repository
///   too, PluginManager.php:109-111): vivacity runs the project's
///   plugins only;
/// - an active plugin that is neither emulated nor benign (what the
///   install side hands over: composer-patches removes the packages to
///   re-patch at `PRE_UPDATE_CMD`, an installers extender moves the
///   install paths the listeners write) — inert for the lock, not for
///   the run;
/// - a local repository whose install paths vivacity cannot compute when
///   a listener that writes them is loaded.
pub(crate) fn issues_before_resolution(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    with_dev: bool,
) -> Vec<vivacity_core::scope::ScopeIssue> {
    use vivacity_core::scope::{ScopeIssue, BENIGN_PLUGINS, EMULATED_PLUGINS, RESOLUTION_INERT};
    let mut issues = Vec::new();
    let listeners = [
        vivacity_core::phpcs_installer::PLUGIN_NAME,
        extension_installers::PHPSTAN,
        extension_installers::RECTOR,
        vivacity_core::c3_plugin::PLUGIN_NAME,
    ];
    if let Some(global) = plugin_order::global_composer_dir() {
        let packages = plugin_order::installed_json_packages(&global);
        for p in &packages {
            let Some(name) = p.get("name").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if listeners.contains(&name)
                && p.get("type").and_then(serde_json::Value::as_str) == Some("composer-plugin")
            {
                issues.push(ScopeIssue::ResolutionPlugin(
                    name.to_owned(),
                    "is installed globally: its post-update-cmd listener runs on this project (not emulated)".to_owned(),
                ));
            }
        }
    }
    // (A plugin outside every list is already handed over by
    // `resolution_issues`: only the resolution-inert ones are left.)
    for name in vivacity_core::scope::active_plugins(project, manifest, true) {
        if RESOLUTION_INERT.contains(&name.as_str())
            && !EMULATED_PLUGINS.contains(&name.as_str())
            && !BENIGN_PLUGINS.contains(&name.as_str())
        {
            issues.push(ScopeIssue::ResolutionPlugin(
                name,
                "runs listeners during an update without install that vivacity does not emulate"
                    .to_owned(),
            ));
        }
    }
    issues.extend(layout_issue(project, manifest));
    // The second round of emulated plugins (docs/plans/plugins-round2.md):
    // dg/composer-cleaner, liborm85/composer-vendor-cleaner and a
    // configured laminas-skeleton-installer act at `POST_UPDATE_CMD`.
    let installed = vivacity_core::layout::installed_packages_of(project, manifest);
    let global = crate::effect_plugins::global_packages();
    let ctx = vivacity_core::effect_plugins::Context {
        project,
        manifest,
        local: installed
            .iter()
            .filter_map(serde_json::Value::as_object)
            .collect(),
        installed: &installed,
        dev_mode: with_dev,
        command: vivacity_core::effect_plugins::Command::UpdateNoInstall,
        interactive: false,
        no_autoloader: false,
        plugins_enabled: true,
        global: &global,
        aliases: false,
        // No dump without an install: drupol/composer-packages is inert.
        lock_data: None,
    };
    issues.extend(
        vivacity_core::effect_plugins::issues(&ctx)
            .into_iter()
            .map(ScopeIssue::Layout),
    );
    issues
}

/// yiisoft/yii2-composer's `Plugin::activate` (Plugin.php:46-56) creates
/// `<vendor-dir>/yiisoft/extensions.php` when it is missing, on every
/// command that loads it — without an install, and on a dry run too. The
/// plugin loaded from installed.json and allowed.
pub(crate) fn activate_yii2_composer(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_core::yii2_composer as yii;
    let repo = local_repository(project, manifest, plugins_enabled);
    if repo.listeners.iter().any(|p| p == yii::PLUGIN_NAME) && allowed(manifest, yii::PLUGIN_NAME) {
        let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
            .unwrap_or_default()
            .vendor_dir(project);
        yii::ensure_extensions_file(&vendor)?;
    }
    Ok(())
}

/// `POST_UPDATE_CMD` without an install: the emulated listeners, in their
/// plugins' activation order, on the local repository. `manifest` is the
/// root package the listeners read (`extra`, `allow-plugins` as loaded);
/// `flex` prints symfony/flex's output at its rank (already decided by the
/// resolution, `print_flex_post_update`).
pub(crate) fn post_update_listeners(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
    flex: Option<&dyn Fn() -> anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let repo = local_repository(project, manifest, plugins_enabled);
    let emulated = [
        vivacity_core::phpcs_installer::PLUGIN_NAME,
        extension_installers::PHPSTAN,
        extension_installers::RECTOR,
        vivacity_core::c3_plugin::PLUGIN_NAME,
        "symfony/flex",
    ];
    let mut layout: Option<vivacity_core::layout::Layout> = None;
    let mut flex_printed = false;
    // Every plugin Composer loaded, in activation order: under -v its
    // listener's line (`EventDispatcher::doDispatch`, written right before
    // the call), then what the emulated listener does — the two
    // interleaved, as Composer writes them.
    for plugin in repo.listeners.iter().map(String::as_str) {
        crate::verbose_listeners::announce(
            plugin,
            crate::verbose_listeners::POST_UPDATE_CMD,
            manifest,
        );
        if !emulated.contains(&plugin) {
            continue;
        }
        if plugin == "symfony/flex" {
            if let Some(print) = flex {
                print()?;
                flex_printed = true;
            }
            continue;
        }
        if !listening(&repo, manifest, plugin) {
            continue;
        }
        if plugin == vivacity_core::c3_plugin::PLUGIN_NAME {
            // `askForUpdateV2`: the installed c3's own c3.php.
            emulate_c3(project, &repo.lock, manifest, true, true, plugins_enabled)?;
            continue;
        }
        if layout.is_none() {
            // Refused before resolving when it cannot be computed
            // (`layout_issue`).
            layout = Some(local_layout(project, &repo, manifest).map_err(|issues| {
                anyhow::anyhow!(
                    "internal: the local repository's layout was not checked: {}",
                    issues.join("; ")
                )
            })?);
        }
        let Some(layout) = layout.as_ref() else {
            continue;
        };
        match plugin {
            vivacity_core::phpcs_installer::PLUGIN_NAME => {
                emulate_phpcs(project, layout, &repo.lock, manifest, true)?;
            }
            extension_installers::PHPSTAN => {
                extension_installers::phpstan(
                    layout,
                    &repo.lock,
                    &[],
                    manifest,
                    true,
                    plugins_enabled,
                )?;
            }
            extension_installers::RECTOR => {
                extension_installers::rector(layout, &repo.lock, manifest, true, plugins_enabled)?;
            }
            _ => {}
        }
    }
    // symfony/flex active for the resolution but not loadable from
    // installed.json (its directory gone): printed where it always was.
    if let (Some(print), false) = (flex, flex_printed) {
        print()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A project whose installed.json lists `packages`, each laid out
    /// under vendor/ unless its name is in `gone`.
    fn project(packages: serde_json::Value, gone: &[&str]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tmp");
        let composer = tmp.path().join("vendor/composer");
        std::fs::create_dir_all(&composer).expect("mkdir");
        let mut list = Vec::new();
        for p in packages.as_array().expect("array") {
            let name = p["name"].as_str().expect("name");
            if !gone.contains(&name) {
                std::fs::create_dir_all(tmp.path().join("vendor").join(name)).expect("pkg");
            }
            let mut p = p.clone();
            p["install-path"] = json!(format!("../{name}"));
            list.push(p);
        }
        std::fs::write(
            composer.join("installed.json"),
            json!({"packages": list, "dev": true, "dev-package-names": []}).to_string(),
        )
        .expect("installed.json");
        tmp
    }

    #[test]
    fn the_local_repository_is_installed_json_purged() {
        let tmp = project(
            json!([
                {"name": "phpstan/extension-installer", "version": "1.4.3", "type": "composer-plugin"},
                {"name": "a/here", "version": "1.0.0"},
                {"name": "a/gone", "version": "1.0.0"},
            ]),
            &["a/gone"],
        );
        let manifest = json!({"config": {"allow-plugins": {"phpstan/extension-installer": true}}});
        let repo = local_repository(tmp.path(), &manifest, true);
        let names: Vec<&str> = repo.lock.packages.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["phpstan/extension-installer", "a/here"]);
        assert!(repo
            .listeners
            .iter()
            .any(|p| p == "phpstan/extension-installer"));
        // `--no-plugins`: nothing loaded, the repository unchanged.
        assert!(local_repository(tmp.path(), &manifest, false)
            .listeners
            .is_empty());
    }

    #[test]
    fn discovery_hands_over_when_it_looks_after_an_abstraction() {
        let allowed = json!({"config": {"allow-plugins": {"php-http/discovery": true}}});
        let plugin = |version: &str| json!({"name": "php-http/discovery", "version": version, "type": "composer-plugin"});
        let sdk = json!({"name": "acme/sdk", "version": "1.0.0",
            "require": {"php-http/discovery": "^1.15", "psr/http-client-implementation": "*"}});
        // Something to look after: handed over.
        let watched = project(json!([plugin("1.20.0"), sdk.clone()]), &[]);
        assert!(discovery_issue(watched.path(), &allowed,).is_some());
        // Not allowed: never loaded.
        assert!(discovery_issue(
            watched.path(),
            &json!({"config": {"allow-plugins": {"php-http/discovery": false}}})
        )
        .is_none());
        // Nothing to look after: native.
        let idle = project(
            json!([plugin("1.20.0"), {"name": "psr/http-client", "version": "1.0.3"}]),
            &[],
        );
        assert!(discovery_issue(idle.path(), &allowed).is_none());
        // A version not read: handed over, whatever it looks after.
        let newer = project(json!([plugin("1.21.0")]), &[]);
        let issue = discovery_issue(newer.path(), &allowed)
            .map(|i| i.to_string())
            .unwrap_or_default();
        assert!(
            issue.contains("1.21.0 listens to post-update-cmd in a version not read"),
            "{issue}"
        );
        // Its directory gone: its classes are not there, not loaded.
        let gone = project(json!([plugin("1.20.0"), sdk]), &["php-http/discovery"]);
        assert!(discovery_issue(gone.path(), &allowed).is_none());
    }

    #[test]
    fn a_plugin_neither_emulated_nor_benign_is_handed_over() {
        let tmp = project(
            json!([{"name": "cweagans/composer-patches", "version": "1.7.3", "type": "composer-plugin"}]),
            &[],
        );
        let manifest = json!({"config": {"allow-plugins": {"cweagans/composer-patches": true}}});
        let issues: Vec<String> = issues_before_resolution(tmp.path(), &manifest, true)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(
            issues
                .iter()
                .any(|i| i.starts_with("plugin cweagans/composer-patches runs listeners")),
            "{issues:?}"
        );
    }

    #[test]
    fn a_plugin_allowed_but_not_installed_runs_nothing() {
        // Allowed in `allow-plugins`, absent from installed.json: Composer
        // never loads it, so no listener runs — not even the layout of the
        // local repository is computed (an absolute vendor-dir, which
        // `Layout::resolve` refuses, would otherwise fail the run after the
        // lock was written).
        let tmp = project(json!([{"name": "a/lib", "version": "1.0.0"}]), &[]);
        let manifest = json!({"config": {"vendor-dir": "/an/absolute/vendor",
            "allow-plugins": {"phpstan/extension-installer": true,
                "dealerdirect/phpcodesniffer-composer-installer": true}}});
        assert!(post_update_listeners(tmp.path(), &manifest, true, None).is_ok());
        assert!(post_update_listeners(tmp.path(), &manifest, false, None).is_ok());
    }
}
