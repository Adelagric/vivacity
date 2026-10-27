//! The lines Composer writes at `-v` for the plugin listeners it calls, and
//! the plugins whose `-v` output vivacity does not reproduce
//! (docs/plans/verbose-plugins.md).
//!
//! `EventDispatcher::doDispatch` (EventDispatcher.php:230) writes, on stderr
//! at `IOInterface::VERBOSE`, one line per plugin listener it calls:
//! `> <event>: <class>-><method>`, the class being `get_class` of the
//! subscriber, the actual class loaded. The listeners of an event run by
//! priority, then in the order they were added (`getListeners`, :583-596):
//! every listener below is at priority 0, so they follow the activation
//! order of their plugins (`plugin_order`). The events vivacity places are
//! `command` (`bamarni_command_event`), `pre-autoload-dump` and
//! `post-autoload-dump` (`AutoloadGenerator::dump`, :204 and :476),
//! `post-install-cmd` / `post-update-cmd` (`Installer::run`, :440, also under
//! `update --no-install`); none under `--dry-run` (`Installer.php:292-294`:
//! no script event at all).
//!
//! The class name: `PluginManager::registerPackage` (PluginManager.php:
//! 289-307) loads a plugin class that is already declared in the process
//! under `<class>_composer_tmp<N>`, `N` a counter of the process. That
//! happens to every class of every loaded plugin when `require` and `remove`
//! create their second `Composer` (`RequireCommand.php:409-410`,
//! `RemoveCommand.php:221-222`), and to the classes of a plugin the
//! transaction updates (`PluginInstaller::update`, :110-111). [`Renames`]
//! keeps that counter for one run.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A plugin whose listeners vivacity announces at `-v`: the versions whose
/// source was read (every tag of that range, docs/plans/verbose-plugins.md),
/// its subscriber class, and `(event, method)` per listener
/// (`getSubscribedEvents`). Listeners for events Composer never dispatches
/// on these commands (`post-create-project-cmd`) are left out.
pub(crate) struct VerbosePlugin {
    pub(crate) name: &'static str,
    /// The versions read, as a Composer constraint: one range per branch,
    /// up to the last tag of that branch when it was read — a later patch
    /// release is not accepted unread.
    pub(crate) versions: &'static str,
    pub(crate) class: &'static str,
    pub(crate) listeners: &'static [(&'static str, &'static str)],
}

pub(crate) const COMMAND: &str = "command";
pub(crate) const PRE_AUTOLOAD_DUMP: &str = "pre-autoload-dump";
pub(crate) const POST_AUTOLOAD_DUMP: &str = "post-autoload-dump";
pub(crate) const POST_INSTALL_CMD: &str = "post-install-cmd";
pub(crate) const POST_UPDATE_CMD: &str = "post-update-cmd";

/// The plugins whose `-v` output vivacity reproduces. Those without
/// listeners write nothing at `-v` (their `activate` writes at DEBUG at
/// most); their classes still count for the `_composer_tmpN` counter.
pub(crate) const VERBOSE_PLUGINS: &[VerbosePlugin] = &[
    // `Plugin` implements `PluginInterface` only (v1.0.22 – v2.3.0;
    // `Installer`, a `composer-installer`, before).
    VerbosePlugin {
        name: "composer/installers",
        versions: ">=1.0.0 <=1.0.25 || >=1.1.0 <=1.1.0 || >=1.2.0 <=1.2.0 || >=1.3.0 <=1.3.0 || >=1.4.0 <=1.4.0 || >=1.5.0 <=1.5.0 || >=1.6.0 <=1.6.0 || >=1.7.0 <=1.7.0 || >=1.8.0 <=1.8.0 || >=1.9.0 <=1.9.0 || >=1.10.0 <=1.10.0 || >=1.11.0 <=1.11.0 || >=1.12.0 <=1.12.0 || >=2.0.0 <=2.0.1 || >=2.1.0 <=2.1.1 || >=2.2.0 <=2.2.0 || >=2.3.0 <=2.3.0",
        class: "Composer\\Installers\\Plugin",
        listeners: &[],
    },
    VerbosePlugin {
        name: "roots/wordpress-core-installer",
        versions: ">=1.0.0 <=1.0.0 || >=1.1.0 <=1.1.0 || >=1.100.0 <=1.100.0 || >=2.0.0 <=2.0.0 || >=3.0.0 <=3.0.0 || >=4.0.0 <=4.0.0",
        class: "Roots\\Composer\\WordPressCorePlugin",
        listeners: &[],
    },
    VerbosePlugin {
        name: "mnsami/composer-custom-directory-installer",
        versions: ">=1.0.0 <=1.0.4 || >=1.1.0 <=1.1.1 || >=2.0.0 <=2.0.0 || >=2.1.0 <=2.1.0 || >=2.2.0 <=2.2.1 || >=2.3.0 <=2.3.0",
        class: "Composer\\CustomDirectoryInstaller\\LibraryPlugin",
        listeners: &[],
    },
    // A `CommandProvider`; `activate` / `deactivate` write at DEBUG.
    VerbosePlugin {
        name: "ibexa/post-install",
        versions: ">=1.0.0 <=1.0.16 || >=4.0.0 <=4.0.8 || >=4.1.0 <=4.1.5 || >=4.2.0 <=4.2.4 || >=4.3.0 <=4.3.5 || >=4.4.0 <=4.4.4 || >=4.5.0 <=4.5.7 || >=4.6.0 <=4.6.32 || >=5.0.0 <=5.0.10",
        class: "Ibexa\\PostInstall\\PostInstall",
        listeners: &[],
    },
    // A `CommandProvider` with an empty `activate`.
    VerbosePlugin {
        name: "ergebnis/composer-normalize",
        versions: ">=0.1.0 <=0.1.0 || >=0.2.0 <=0.2.0 || >=0.3.0 <=0.3.0 || >=0.4.0 <=0.4.1 || >=0.5.0 <=0.5.0 || >=0.6.0 <=0.6.0 || >=0.7.0 <=0.7.0 || >=0.8.0 <=0.8.0 || >=0.9.0 <=0.9.0 || >=1.0.0 <=1.0.0 || >=1.1.0 <=1.1.4 || >=1.2.0 <=1.2.0 || >=1.3.0 <=1.3.1 || >=2.0.0 <=2.0.2 || >=2.1.0 <=2.1.2 || >=2.2.0 <=2.2.4 || >=2.3.0 <=2.3.2 || >=2.4.0 <=2.4.0 || >=2.5.0 <=2.5.2 || >=2.6.0 <=2.6.1 || >=2.7.0 <=2.7.1 || >=2.8.0 <=2.8.2 || >=2.9.0 <=2.9.1 || >=2.10.0 <=2.10.0 || >=2.11.0 <=2.11.0 || >=2.12.0 <=2.12.2 || >=2.13.0 <=2.13.4 || >=2.14.0 <=2.14.0 || >=2.15.0 <=2.15.0 || >=2.16.0 <=2.16.0 || >=2.17.0 <=2.17.0 || >=2.18.0 <=2.18.0 || >=2.19.0 <=2.19.0 || >=2.20.0 <=2.20.0 || >=2.21.0 <=2.21.0 || >=2.22.0 <=2.22.0 || >=2.23.0 <=2.23.1 || >=2.24.0 <=2.24.1 || >=2.25.0 <=2.25.2 || >=2.26.0 <=2.26.0 || >=2.27.0 <=2.27.0 || >=2.28.0 <=2.28.3 || >=2.29.0 <=2.29.0 || >=2.30.0 <=2.30.2 || >=2.31.0 <=2.31.0 || >=2.32.0 <=2.32.0 || >=2.33.0 <=2.33.0 || >=2.34.0 <=2.34.0 || >=2.35.0 <=2.35.0 || >=2.36.0 <=2.36.0 || >=2.37.0 <=2.37.0 || >=2.38.0 <=2.38.0 || >=2.39.0 <=2.39.0 || >=2.40.0 <=2.40.0 || >=2.41.0 <=2.41.1 || >=2.42.0 <=2.42.0 || >=2.43.0 <=2.43.0 || >=2.44.0 <=2.44.0 || >=2.45.0 <=2.45.0 || >=2.46.0 <=2.46.0 || >=2.47.0 <=2.47.0 || >=2.48.0 <=2.48.2 || >=2.49.0 <=2.49.0 || >=2.50.0 <=2.50.0 || >=2.51.0 <=2.51.0 || >=2.52.0 <=2.52.0 || >=2.53.0 <=2.53.0 || >=2.54.0 <=2.54.0",
        class: "Ergebnis\\Composer\\Normalize\\NormalizePlugin",
        listeners: &[],
    },
    // `ComposerPlugin::getSubscribedEvents`: `updateAutoloadFile` once
    // activated (it always is when the dispatcher asks).
    VerbosePlugin {
        name: "symfony/runtime",
        versions: ">=5.3.0 <=5.3.14 || >=5.4.0 <=5.4.53 || >=6.0.0 <=6.0.19 || >=6.1.0 <=6.1.11 || >=6.2.0 <=6.2.13 || >=6.3.0 <=6.3.12 || >=6.4.0 <=6.4.41 || >=7.0.0 <=7.0.8 || >=7.1.0 <=7.1.7 || >=7.2.0 <=7.2.8 || >=7.3.0 <=7.3.8 || >=7.4.0 <=7.4.21 || >=8.0.0 <=8.0.14 || >=8.1.0 <=8.1.9",
        class: "Symfony\\Component\\Runtime\\Internal\\ComposerPlugin",
        listeners: &[(POST_AUTOLOAD_DUMP, "updateAutoloadFile")],
    },
    VerbosePlugin {
        name: "pestphp/pest-plugin",
        versions: ">=0.2.0 <=0.2.1 || >=0.3.0 <=0.3.0 || >=1.0.0 <=1.0.0 || >=1.1.0 <=1.1.0 || >=2.0.0 <=2.0.1 || >=2.1.0 <=2.1.1 || >=3.0.0 <=3.0.0 || >=4.0.0 <=4.0.0 || >=5.0.0 <=5.0.0",
        class: "Pest\\Plugin\\Manager",
        listeners: &[(POST_AUTOLOAD_DUMP, "registerPlugins")],
    },
    // 1.15 and 1.16 listen to `post-update-cmd` only.
    VerbosePlugin {
        name: "php-http/discovery",
        versions: ">=1.17.0 <=1.17.0 || >=1.18.0 <=1.18.1 || >=1.19.0 <=1.19.4 || >=1.20.0 <=1.20.0",
        class: "Http\\Discovery\\Composer\\Plugin",
        listeners: &[
            (PRE_AUTOLOAD_DUMP, "preAutoloadDump"),
            (POST_UPDATE_CMD, "postUpdate"),
        ],
    },
    VerbosePlugin {
        name: "composer/package-versions-deprecated",
        versions: ">=1.8.0 <=1.8.2 || >=1.10.0 <=1.10.99.1 || >=1.11.0 <=1.11.99.5",
        class: "PackageVersions\\Installer",
        listeners: &[(POST_AUTOLOAD_DUMP, "dumpVersionsClass")],
    },
    VerbosePlugin {
        name: "phpstan/extension-installer",
        versions: ">=1.0.0 <=1.0.6 || >=1.1.0 <=1.1.0 || >=1.2.0 <=1.2.0 || >=1.3.0 <=1.3.1 || >=1.4.0 <=1.4.3",
        class: "PHPStan\\ExtensionInstaller\\Plugin",
        listeners: &[(POST_INSTALL_CMD, "process"), (POST_UPDATE_CMD, "process")],
    },
    VerbosePlugin {
        name: "rector/extension-installer",
        versions: ">=0.10.0 <=0.10.2 || >=0.11.0 <=0.11.2 || >=1.0.0 <=1.0.0",
        class: "Rector\\RectorInstaller\\Plugin",
        listeners: &[(POST_INSTALL_CMD, "process"), (POST_UPDATE_CMD, "process")],
    },
    // Before 2.6.0, package events (`copyC3`, `askForUpdate`, `deleteC3`).
    VerbosePlugin {
        name: "codeception/c3",
        versions: ">=2.6.0 <=2.6.6 || >=2.7.0 <=2.7.0 || >=2.8.0 <=2.8.1 || >=2.9.0 <=2.9.0",
        class: "Codeception\\c3\\Installer",
        listeners: &[
            (POST_INSTALL_CMD, "copyC3V2"),
            (POST_UPDATE_CMD, "askForUpdateV2"),
        ],
    },
    VerbosePlugin {
        name: "drupal/core-project-message",
        versions: ">=8.8.0 <=8.8.12 || >=8.9.0 <=8.9.20 || >=9.0.0 <=9.0.14 || >=9.1.0 <=9.1.15 || >=9.2.0 <=9.2.21 || >=9.3.0 <=9.3.22 || >=9.4.0 <=9.4.15 || >=9.5.0 <=9.5.11 || >=10.0.0 <=10.0.11 || >=10.1.0 <=10.1.8 || >=10.2.0 <=10.2.12 || >=10.3.0 <=10.3.14 || >=10.4.0 <=10.4.10 || >=10.5.0 <=10.5.12 || >=10.6.0 <=10.6.20 || >=11.0.0 <=11.0.13 || >=11.1.0 <=11.1.10 || >=11.2.0 <=11.2.14 || >=11.3.0 <=11.3.20 || >=11.4.0 <=11.4.11 || >=11.5.0-dev <=11.5.0-beta1 || >=12.0.0-dev <=12.0.0-beta2",
        class: "Drupal\\Composer\\Plugin\\ProjectMessage\\MessagePlugin",
        listeners: &[(POST_INSTALL_CMD, "displayPostCreateMessage")],
    },
    // Its VERBOSE writes are all in a `RequireCommand` context
    // (Plugin.php:100-124), which `resolution_effect` already hands over.
    VerbosePlugin {
        name: "drupal/core-recipe-unpack",
        versions: ">=11.2.0 <=11.2.14 || >=11.3.0 <=11.3.20 || >=11.4.0 <=11.4.11 || >=11.5.0-dev <=11.5.0-beta1 || >=12.0.0-dev <=12.0.0-beta2",
        class: "Drupal\\Composer\\Plugin\\RecipeUnpack\\Plugin",
        listeners: &[(POST_UPDATE_CMD, "unpackOnRequire")],
    },
    // Its `Logger::logDebug` lines are VERBOSE: "Calling onCommandEvent()."
    // and "Calling onPostAutoloadDump()." (BamarniBinPlugin.php:88, :110),
    // written by the emulation; "Command already forwarded…" needs
    // `forward-command`, handed over at every level.
    VerbosePlugin {
        name: "bamarni/composer-bin-plugin",
        versions: ">=1.8.0 <=1.8.3 || >=1.9.0 <=1.9.1",
        class: "Bamarni\\Composer\\Bin\\BamarniBinPlugin",
        listeners: &[
            (COMMAND, "onCommandEvent"),
            (POST_AUTOLOAD_DUMP, "onPostAutoloadDump"),
        ],
    },
];

/// Plugins whose `-v` output is not reproduced, and why.
pub(crate) const VERBOSE_DELEGATED: &[(&str, &str)] = &[
    (
        "symfony/flex",
        "it listens to pre-pool-create, pre-operations-exec and the package events, and its configurators write at -v",
    ),
    (
        "dealerdirect/phpcodesniffer-composer-installer",
        "it prints the output of phpcs --config-set at -v",
    ),
    (
        "wikimedia/composer-merge-plugin",
        "it listens to init, pre-install-cmd and post-package-install, and logs at -v",
    ),
    (
        "yiisoft/yii2-composer",
        "it listens to post-package-update, dispatched between the operations",
    ),
    (
        "symfony/thanks",
        "it listens to post-package-update, dispatched between the operations",
    ),
    (
        "metasyntactical/composer-plugin-license-check",
        "it listens to command and the package events, and writes at -v",
    ),
];

/// The table entry of `name`.
pub(crate) fn plugin(name: &str) -> Option<&'static VerbosePlugin> {
    VERBOSE_PLUGINS.iter().find(|p| p.name == name)
}

/// Whether `version` (as written in the lock or installed.json) is one of
/// the versions read for `p`. A dev branch never is.
fn version_read(p: &VerbosePlugin, version: &str) -> bool {
    vivacity_core::constraint::satisfies(version, p.versions).unwrap_or(false)
}

/// Why the `-v` output of one allowed plugin is not reproduced, or `None`.
fn plugin_issue(name: &str, version: &str, run_scripts: bool) -> Option<String> {
    if let Some((_, why)) = VERBOSE_DELEGATED.iter().find(|(n, _)| *n == name) {
        return Some((*why).to_owned());
    }
    let Some(entry) = plugin(name) else {
        return Some("not on vivacity's -v list".to_owned());
    };
    if !version_read(entry, version) {
        return Some(format!(
            "{version} is not among the versions reproduced, {}",
            entry.versions
        ));
    }
    if run_scripts && !entry.listeners.is_empty() {
        return Some("composer run-script replays them under --run-scripts".to_owned());
    }
    None
}

fn is_plugin(p: &Value) -> bool {
    matches!(
        p.get("type").and_then(Value::as_str),
        Some("composer-plugin" | "composer-installer")
    )
}

fn name_version(p: &Value) -> Option<(String, String)> {
    Some((
        p.get("name")?.as_str()?.to_ascii_lowercase(),
        p.get("version")?.as_str()?.to_owned(),
    ))
}

fn allowed(manifest: &Value, name: &str) -> bool {
    matches!(
        vivacity_core::layout::plugin_allowed(manifest, name),
        vivacity_core::layout::PluginVerdict::Allowed
    )
}

/// Why `-v` on this project goes to Composer, or `None` when vivacity
/// prints it: every plugin Composer would load — installed or locked
/// (installed and target versions alike), and allowed — is in
/// [`VERBOSE_PLUGINS`] in a version reproduced; no plugin is installed
/// globally (the emulations play the project's plugins only, and Composer
/// judges those by the global `allow-plugins`); `--run-scripts` is not
/// asked while one of them has listeners (`composer run-script` replays
/// them). Under `--no-install` their `post-update-cmd` listeners are played
/// like the others (`no_install::post_update_listeners`). A plugin the
/// resolution brings in is checked on the solved lock
/// ([`resolved_issue`]). `None` under `--no-plugins` or without a readable
/// composer.json (the command reports that itself).
pub(crate) fn issue(
    project: &Path,
    global: Option<&Path>,
    plugins_enabled: bool,
    run_scripts: bool,
) -> Option<String> {
    if !plugins_enabled {
        return None;
    }
    let read = |name: &str| -> Option<Value> {
        serde_json::from_str(&std::fs::read_to_string(project.join(name)).ok()?).ok()
    };
    let manifest = read("composer.json")?;
    if let Some(global) = global {
        for p in vivacity_core::layout::installed_packages_at(global) {
            if let (true, Some((name, _))) = (is_plugin(&p), name_version(&p)) {
                return Some(format!(
                    "-v while {name} is active (its listeners' lines: it is installed globally)"
                ));
            }
        }
    }
    let mut plugins: Vec<(String, String)> = Vec::new();
    for p in vivacity_core::layout::installed_packages_of(project, &manifest) {
        if is_plugin(&p) {
            plugins.extend(name_version(&p));
        }
    }
    if let Some(lock) = read("composer.lock") {
        for key in ["packages", "packages-dev"] {
            for p in lock
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if is_plugin(p) {
                    plugins.extend(name_version(p));
                }
            }
        }
    }
    plugins
        .into_iter()
        .filter(|(name, _)| allowed(&manifest, name))
        .find_map(|(name, version)| {
            plugin_issue(&name, &version, run_scripts).map(|why| {
                format!("-v while {name} {version} is active (its listeners' lines: {why})")
            })
        })
}

/// [`issue`] on a solved lock, for the plugins the install phase would
/// install and activate (`with_dev`: the packages it lays out): the plugin
/// and why, so that the command goes to Composer before the lock is
/// written — a hand-over after it would run `composer install`, whose
/// `-v` lines differ (`post-install-cmd`, no `_composer_tmpN`).
pub(crate) fn resolved_issue(
    lock: &Value,
    manifest: &Value,
    with_dev: bool,
) -> Option<(String, String)> {
    let keys: &[&str] = if with_dev {
        &["packages", "packages-dev"]
    } else {
        &["packages"]
    };
    keys.iter()
        .filter_map(|k| lock.get(*k).and_then(Value::as_array))
        .flatten()
        .filter(|p| is_plugin(p))
        .filter_map(name_version)
        .filter(|(name, _)| allowed(manifest, name))
        .find_map(|(name, version)| {
            plugin_issue(&name, &version, false).map(|why| {
                (
                    name.clone(),
                    format!("{version} would be activated by this run, and its -v lines are not reproduced ({why})"),
                )
            })
        })
}

/// The `_composer_tmpN` state of one run: the plugin classes declared in
/// the process (lowercase, as PHP compares class names), the counter, the
/// suffix each table plugin's subscriber class carries now, the plugins
/// registered, and whether the first load was recorded.
#[derive(Default)]
struct Renames {
    counter: u32,
    declared: BTreeSet<String>,
    suffix: BTreeMap<String, u32>,
    first_load_done: bool,
}

static RENAMES: std::sync::Mutex<Option<Renames>> = std::sync::Mutex::new(None);

/// Forgets the state (the start of a run: a new process for Composer).
pub(crate) fn reset() {
    if let Ok(mut r) = RENAMES.lock() {
        *r = None;
    }
}

/// `extra.class` of a package entry: a string or a list.
fn classes_of(entry: &Value) -> Vec<String> {
    match entry.get("extra").and_then(|e| e.get("class")) {
        Some(Value::String(c)) => vec![c.clone()],
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// `registerPackage` loading `classes` of `plugin` (PluginManager.php:
/// 286-307): a class already declared (`class_exists($class, false)`) is
/// loaded as `<class>_composer_tmp<counter>`, the counter moving on; a new
/// one is declared as it is.
fn load(r: &mut Renames, plugin: &str, classes: &[String]) {
    let subscriber = self::plugin(plugin).map(|p| p.class.to_ascii_lowercase());
    for class in classes {
        let class = class.trim_start_matches('\\').to_ascii_lowercase();
        let is_subscriber = subscriber.as_deref() == Some(class.as_str());
        if r.declared.contains(&class) {
            if is_subscriber {
                r.suffix.insert(plugin.to_owned(), r.counter);
            }
            r.counter += 1;
        } else {
            r.declared.insert(class);
            if is_subscriber {
                r.suffix.remove(plugin);
            }
        }
    }
}

/// The entry of `name` among `entries`.
fn entry_named<'a>(entries: &'a [Value], name: &str) -> Option<&'a Value> {
    entries.iter().find(|p| {
        p.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// The plugins `Factory::createComposer` loads: `order` (`loaded_plugins`,
/// installed.json sorted, present on disk), the allowed ones, their classes
/// from `installed`.
fn load_all(r: &mut Renames, manifest: &Value, order: &[String], installed: &[Value]) {
    for name in order {
        if !allowed(manifest, name) {
            continue;
        }
        if let Some(entry) = entry_named(installed, name) {
            load(r, name, &classes_of(entry));
        }
    }
}

/// The first `Composer` of the run loads the installed plugins (once:
/// `require` / `remove` have recorded it already when they re-create it).
pub(crate) fn first_load(manifest: &Value, order: &[String], installed: &[Value]) {
    if let Ok(mut guard) = RENAMES.lock() {
        let r = guard.get_or_insert_with(Renames::default);
        if !r.first_load_done {
            r.first_load_done = true;
            load_all(r, manifest, order, installed);
        }
    }
}

/// `require` / `remove` create their second `Composer` (`resetComposer` +
/// `requireComposer`): every class the first one loaded is loaded again,
/// renamed, in load order. `manifest`: the rules of that load. Global
/// plugins hand `-v` over before this runs.
pub(crate) fn reload_plugins(project: &Path, manifest: &Value, plugins_enabled: bool) {
    if !plugins_enabled {
        return;
    }
    let composer_dir = crate::composer_dir_of(project, manifest);
    let installed = crate::plugin_order::installed_json_packages(&composer_dir);
    let order = crate::plugin_order::loaded_plugins(&installed, &composer_dir, None);
    first_load(manifest, &order, &installed);
    if let Ok(mut guard) = RENAMES.lock() {
        load_all(
            guard.get_or_insert_with(Renames::default),
            manifest,
            &order,
            &installed,
        );
    }
}

/// A plugin operation of the transaction registers the target package
/// (`PluginInstaller::install` / `update`, :80-111): its classes are loaded
/// — renamed when declared already (an update, or another package that
/// ships the same class). The caller skips an install of a plugin already
/// registered (`registerPackage` returns early).
pub(crate) fn plugin_registered(name: &str, target: &Value) {
    if let Ok(mut guard) = RENAMES.lock() {
        load(
            guard.get_or_insert_with(Renames::default),
            name,
            &classes_of(target),
        );
    }
}

/// The `> <event>: <class>-><method>` lines of `plugin` for `event`, at
/// `-v` (nothing at the other levels, nothing for a plugin `allow-plugins`
/// does not allow: Composer never loads it).
pub(crate) fn announce(plugin: &str, event: &str, manifest: &Value) {
    if vivacity_core::io::level() != vivacity_core::io::Verbosity::Verbose {
        return;
    }
    let Some(entry) = self::plugin(plugin) else {
        return;
    };
    if !matches!(
        vivacity_core::layout::plugin_allowed(manifest, plugin),
        vivacity_core::layout::PluginVerdict::Allowed
    ) {
        return;
    }
    let suffix = RENAMES
        .lock()
        .ok()
        .and_then(|r| r.as_ref().and_then(|r| r.suffix.get(plugin).copied()))
        .map(|n| format!("_composer_tmp{n}"))
        .unwrap_or_default();
    for (e, method) in entry.listeners {
        if *e == event {
            vivacity_core::say_verbose!("> {event}: {}{suffix}->{method}", entry.class);
        }
    }
}

/// The table plugins with a listener on `event`, for the loops that walk
/// the activation order.
pub(crate) fn listening_to(event: &str) -> Vec<&'static str> {
    VERBOSE_PLUGINS
        .iter()
        .filter(|p| p.listeners.iter().any(|(e, _)| *e == event))
        .map(|p| p.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_read_bind_the_table() {
        let discovery = plugin("php-http/discovery").expect("listed");
        assert!(version_read(discovery, "1.20.0"));
        assert!(version_read(discovery, "1.17.0"));
        assert!(!version_read(discovery, "1.15.0"));
        assert!(!version_read(discovery, "1.20.1"));
        assert!(!version_read(discovery, "1.x-dev"));
        assert!(!version_read(discovery, "dev-main"));
        let runtime = plugin("symfony/runtime").expect("listed");
        for v in [
            "v5.3.0", "v5.4.53", "v6.4.41", "v7.4.14", "v8.1.0", "v8.1.9",
        ] {
            assert!(version_read(runtime, v), "{v}");
        }
        assert!(!version_read(runtime, "v8.2.0"));
        // A patch release after the last one read, on a maintained branch.
        assert!(!version_read(runtime, "v7.4.22"));
        assert!(!version_read(
            plugin("pestphp/pest-plugin").expect("listed"),
            "v4.0.1"
        ));
        let message = plugin("drupal/core-project-message").expect("listed");
        assert!(version_read(message, "11.4.6"));
        assert!(version_read(message, "12.0.0-beta2"));
        assert!(!version_read(message, "12.0.0-beta3"));
        assert!(!version_read(message, "12.0.0"));
        assert!(!version_read(message, "11.4.12"));
        let pv = plugin("composer/package-versions-deprecated").expect("listed");
        assert!(version_read(pv, "1.11.99.5"));
        assert!(!version_read(pv, "1.7.0"));
    }

    #[test]
    fn every_entry_parses_and_no_plugin_is_listed_twice() {
        let mut names: Vec<&str> = VERBOSE_PLUGINS.iter().map(|p| p.name).collect();
        names.extend(VERBOSE_DELEGATED.iter().map(|(n, _)| *n));
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        for p in VERBOSE_PLUGINS {
            assert!(
                vivacity_core::constraint::Constraint::parse(p.versions).is_ok(),
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn a_declared_class_is_renamed_in_load_order() {
        let classes = |list: &[&str]| list.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        let mut r = Renames::default();
        let first = [
            (
                "composer/installers",
                classes(&["Composer\\Installers\\Plugin"]),
            ),
            (
                "mnsami/composer-custom-directory-installer",
                classes(&[
                    "Composer\\CustomDirectoryInstaller\\LibraryPlugin",
                    "Composer\\CustomDirectoryInstaller\\PearPlugin",
                    "Composer\\CustomDirectoryInstaller\\PluginPlugin",
                ]),
            ),
            ("pestphp/pest-plugin", classes(&["Pest\\Plugin\\Manager"])),
        ];
        // The first load declares them: no suffix, no counter.
        for (name, c) in &first {
            load(&mut r, name, c);
        }
        assert_eq!(r.counter, 0);
        assert!(r.suffix.is_empty());
        // The second `Composer` of `require`: all renamed, in order; three
        // classes move the counter three times.
        for (name, c) in &first {
            load(&mut r, name, c);
        }
        assert_eq!(r.suffix.get("composer/installers"), Some(&0));
        assert_eq!(r.suffix.get("pestphp/pest-plugin"), Some(&4));
        assert_eq!(r.counter, 5);
        // An update of pest-plugin: renamed again; a class seen for the
        // first time (another name in that version) would load as is.
        load(
            &mut r,
            "pestphp/pest-plugin",
            &classes(&["\\Pest\\Plugin\\Manager"]),
        );
        assert_eq!(r.suffix.get("pestphp/pest-plugin"), Some(&5));
        load(
            &mut r,
            "pestphp/pest-plugin",
            &classes(&["Pest\\Plugin\\Other"]),
        );
        assert_eq!(r.counter, 6);
    }

    #[test]
    fn classes_are_read_as_a_string_or_a_list() {
        assert_eq!(classes_of(&json!({"extra": {"class": "A\\B"}})), ["A\\B"]);
        assert_eq!(
            classes_of(&json!({"extra": {"class": ["A\\B", "C\\D"]}})),
            ["A\\B", "C\\D"]
        );
        assert!(classes_of(&json!({})).is_empty());
    }

    #[test]
    fn issue_names_the_plugin_and_the_reason() {
        let tmp = tempfile::tempdir().expect("tmp");
        let write = |manifest: Value, packages: Value| {
            std::fs::write(tmp.path().join("composer.json"), manifest.to_string()).expect("json");
            std::fs::write(
                tmp.path().join("composer.lock"),
                json!({"packages": packages, "packages-dev": []}).to_string(),
            )
            .expect("lock");
        };
        let allow = |names: &[&str]| {
            let map: serde_json::Map<String, Value> = names
                .iter()
                .map(|n| ((*n).to_owned(), Value::Bool(true)))
                .collect();
            json!({"config": {"allow-plugins": map}})
        };
        let plugin = |name: &str, version: &str| json!({"name": name, "version": version, "type": "composer-plugin"});
        write(
            allow(&["pestphp/pest-plugin", "php-http/discovery"]),
            json!([
                plugin("pestphp/pest-plugin", "v4.0.0"),
                plugin("php-http/discovery", "1.20.0")
            ]),
        );
        assert_eq!(issue(tmp.path(), None, true, false), None);
        assert_eq!(issue(tmp.path(), None, false, false), None);
        assert!(issue(tmp.path(), None, true, true)
            .expect("run-scripts")
            .contains("--run-scripts"));
        write(
            allow(&["php-http/discovery"]),
            json!([plugin("php-http/discovery", "1.15.0")]),
        );
        assert!(issue(tmp.path(), None, true, false)
            .expect("old")
            .starts_with(
                "-v while php-http/discovery 1.15.0 is active (its listeners' lines: 1.15.0 is not among"
            ));
        write(
            allow(&["symfony/flex"]),
            json!([plugin("symfony/flex", "v2.11.0")]),
        );
        assert!(issue(tmp.path(), None, true, false)
            .expect("flex")
            .starts_with(
                "-v while symfony/flex v2.11.0 is active (its listeners' lines: it listens"
            ));
        write(
            allow(&["phpstan/extension-installer"]),
            json!([plugin("phpstan/extension-installer", "1.4.3")]),
        );
        assert_eq!(issue(tmp.path(), None, true, false), None);
        let lock = json!({"packages": [plugin("php-http/discovery", "1.15.0")], "packages-dev": [plugin("symfony/flex", "v2.11.0")]});
        let manifest = allow(&["php-http/discovery", "symfony/flex"]);
        assert_eq!(
            resolved_issue(&lock, &manifest, true)
                .map(|(n, _)| n)
                .as_deref(),
            Some("php-http/discovery")
        );
        let lock = json!({"packages": [plugin("php-http/discovery", "1.20.0")], "packages-dev": [plugin("symfony/flex", "v2.11.0")]});
        assert_eq!(resolved_issue(&lock, &manifest, false), None);
        assert!(resolved_issue(&lock, &manifest, true).is_some());
        // Not allowed: never loaded, nothing to print (explicitly refused,
        // whatever a global `allow-plugins` on this machine says).
        write(
            json!({"config": {"allow-plugins": {"symfony/flex": false}}}),
            json!([plugin("symfony/flex", "v2.11.0")]),
        );
        assert_eq!(issue(tmp.path(), None, true, false), None);
        write(
            allow(&["acme/unknown"]),
            json!([plugin("acme/unknown", "1.0.0")]),
        );
        assert!(issue(tmp.path(), None, true, false)
            .expect("unknown")
            .contains("not on vivacity's -v list"));
    }
}
