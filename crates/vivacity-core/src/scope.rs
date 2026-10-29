//! Out-of-scope detector: decides, BEFORE touching the disk, whether vivacity
//! can install this lock natively or must delegate to `composer install`
//! (default fallback) / fail explicitly (when Composer is not available).
//!
//! Principle (plan r1/F3-F5): never a silently divergent vendor/. An unknown
//! plugin, or one that changes the layout, is out of scope. Plugins proven
//! harmless at boot (fixture qualification) are installed like ordinary
//! libraries, with a warning.

use crate::layout::Layout;
use crate::lock::{DistKind, Lock, LockPackage};
use serde_json::Value;
use std::path::Path;

/// Plugins emulated natively by vivacity (identical files, drift test, and
/// the lines their listeners write, in their activation order —
/// docs/plans/plugin-lines.md). composer/installers (see `layout`) is,
/// under conditions checked before any write; the extension-installers and
/// dealerdirect in the versions whose source was read
/// ([`EMULATED_VERSIONS`]). drupal/core-composer-scaffold is deliberately absent: its
/// source is GPL-2.0-or-later and cannot be ported here (NOTICE.md).
pub const EMULATED_PLUGINS: &[&str] = &[
    "symfony/runtime",
    "composer/installers",
    "pestphp/pest-plugin",
    "dealerdirect/phpcodesniffer-composer-installer",
    "phpstan/extension-installer",
    "rector/extension-installer",
    // docs/plans/remaining-fallbacks.md: `POST_INSTALL_CMD` /
    // `POST_UPDATE_CMD`, 0.1.2 only.
    "infection/extension-installer",
    "wikimedia/composer-merge-plugin",
    "composer/package-versions-deprecated",
    // At install: `POST_INSTALL_CMD` copies `.env` from its `.dist` and
    // prints, `symfony-pack` packages are laid out as metapackages, and
    // nothing else fires (`fetchRecipes` is POST_UPDATE_CMD only). The
    // resolution commands have their own rules (`resolution_effect`); on
    // `update` with install, the recipes Flex applies at `POST_UPDATE_CMD`
    // are reproduced for the `bundles` (and auto-generated),
    // `copy-from-recipe`, `env`, `dotenv`, `gitignore`, `add-lines` and
    // `composer-scripts` configurators, and the Docker ones when
    // `extra.symfony.docker` is false (`vivacity::flex_recipes`); any other
    // configurator hands the command over, named.
    "symfony/flex",
    "yiisoft/yii2-composer",
    "codeception/c3",
    "bamarni/composer-bin-plugin",
    "roots/wordpress-core-installer",
    "mnsami/composer-custom-directory-installer",
    // Custom installers (docs/plans/custom-installers.md), each in the
    // versions of `custom_installers::READ_VERSIONS`, decided by `layout`.
    crate::custom_installers::KIRBY,
    crate::custom_installers::CRAFT,
    crate::custom_installers::NEOS,
    crate::custom_installers::DAVIDBARRATT,
    crate::custom_installers::JOHNPBLOCH,
    crate::custom_installers::THELIA,
    crate::custom_installers::CONTAO_COMPONENTS,
    crate::custom_installers::OOMPHINC,
    // Plugins with effects at script events (docs/plans/plugins-round2.md),
    // in the versions of `effect_plugins::READ_VERSIONS`, decided by
    // `effect_plugins::issues`.
    crate::effect_plugins::SPIRAL,
    crate::effect_plugins::SCULPIN,
    crate::effect_plugins::LAMINAS_COMPONENT,
    crate::effect_plugins::LAMINAS_SKELETON,
    crate::effect_plugins::DG_CLEANER,
    crate::effect_plugins::VENDOR_CLEANER,
    crate::effect_plugins::CAKEPHP,
    // docs/plans/duration-drupol-collector.md: `POST_AUTOLOAD_DUMP` only.
    crate::effect_plugins::DRUPOL,
    // docs/plans/remaining-fallbacks.md: `PRE_AUTOLOAD_DUMP`; and the
    // TYPO3 installers, inert under the root `typo3/cms` (observed).
    crate::effect_plugins::CLASS_ALIAS_LOADER,
    crate::effect_plugins::CMS_INSTALLERS,
    // `PRE_INSTALL_CMD` / `POST_INSTALL_CMD`: patches applied by `patch`.
    crate::effect_plugins::PATCHER,
];

/// The versions of the listeners vivacity reproduces whose code changed
/// across the plugin's history, read one by one (docs/plans/plugin-lines.md):
/// another version — older, newer, a dev branch — goes to Composer, named,
/// before anything is written. Normalized versions, and why the others
/// differ.
pub const EMULATED_VERSIONS: &[(&str, &[&str], &str)] = &[
    (
        "phpstan/extension-installer",
        &["1.4.0.0", "1.4.1.0", "1.4.2.0", "1.4.3.0"],
        "1.3.x writes no PHPSTAN_VERSION_CONSTRAINT",
    ),
    (
        "rector/extension-installer",
        &["0.11.0.0", "0.11.1.0", "0.11.2.0", "1.0.0.0"],
        "0.10.x writes another template",
    ),
    (
        "infection/extension-installer",
        &["0.1.2.0"],
        "other versions were not read",
    ),
    (
        "dealerdirect/phpcodesniffer-composer-installer",
        &[
            "0.7.2.0", "1.0.0.0", "1.1.1.0", "1.1.2.0", "1.2.0.0", "1.2.1.0",
        ],
        "the other 0.x versions were not read, v1.1.0 asks InstalledVersions where phpcs is",
    ),
    (
        "composer/package-versions-deprecated",
        &["1.11.99.4", "1.11.99.5"],
        "earlier versions generate another Versions.php",
    ),
];

/// The [`EMULATED_VERSIONS`] issue of one plugin version, if any.
fn emulated_version_issue(name: &str, version: &str) -> Option<ScopeIssue> {
    let (name, read, why) = EMULATED_VERSIONS.iter().find(|(n, _, _)| *n == name)?;
    let normalized = crate::version::normalize_pretty(version).unwrap_or_default();
    if read.contains(&normalized.as_str()) {
        return None;
    }
    Some(ScopeIssue::Layout(format!(
        "plugin {name} {version} is not emulated in this version (versions read: {}; {why})",
        read.iter()
            .map(|v| v.strip_suffix(".0").unwrap_or(v))
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// dealerdirect/phpcodesniffer-composer-installer in a version read
/// (`phpcs_installer::scope_issue`): `packages` are the installed packages
/// as (name, version) pairs, `aliased` whether the lock aliases a name.
fn phpcs_issue(
    project_dir: &Path,
    packages: &[(&str, &str)],
    aliased: impl Fn(&str) -> bool,
    root_manifest: &Value,
) -> Option<ScopeIssue> {
    use crate::phpcs_installer::{path_issue, scope_issue, variant, PLUGIN_NAME};
    let &(_, plugin) = packages.iter().find(|(n, _)| *n == PLUGIN_NAME)?;
    if emulated_version_issue(PLUGIN_NAME, plugin).is_some()
        || !matches!(
            crate::layout::plugin_allowed(root_manifest, PLUGIN_NAME),
            crate::layout::PluginVerdict::Allowed
        )
    {
        return None;
    }
    let phpcs = packages
        .iter()
        .find(|(n, _)| *n == "squizlabs/php_codesniffer")
        .map(|&(n, v)| (v, aliased(n)));
    scope_issue(plugin, phpcs, root_manifest)
        .or_else(|| {
            // The `PATH` matters to v0.7.2 only, and only with phpcs there.
            phpcs.filter(|_| variant(plugin).search_project)?;
            path_issue(
                project_dir,
                root_manifest,
                &std::env::var("PATH").unwrap_or_default(),
            )
        })
        .map(ScopeIssue::Layout)
}

/// [`phpcs_issue`] on a lock: `analyze`, and the solved lock of an update
/// before its install phase.
pub fn phpcs_lock_issue(
    project_dir: &Path,
    lock: &Lock,
    root_manifest: &Value,
    with_dev: bool,
) -> Option<ScopeIssue> {
    let wanted: Vec<(&str, &str)> = lock
        .wanted_packages(with_dev)
        .map(|p| (p.name(), p.version()))
        .collect();
    phpcs_issue(
        project_dir,
        &wanted,
        |name| {
            lock.aliases
                .iter()
                .any(|a| a.get("package").and_then(Value::as_str) == Some(name))
        },
        root_manifest,
    )
}

/// phpstan/extension-installer and rector/extension-installer write each
/// listed package's `getFullPrettyVersion()` from the *last* entry of that
/// name in the local repository (`$data[$name] = …`), and an alias is an
/// entry. Measured (docs/plans/plugin-lines.md): an inline alias of the lock
/// is re-added after the loaded package on every run (`MarkAliasInstalled`),
/// so a no-op writes the alias's version where a fresh install wrote the
/// package's — a file and lines vivacity does not reproduce; a `path`
/// package is laid out synchronously, before its alias, the other way
/// round. A default-branch alias of an archive is not (the rector
/// fixture). Such a listed package goes to Composer, named.
fn aliased_listing_issue(lock: &Lock, root_manifest: &Value, with_dev: bool) -> Option<ScopeIssue> {
    let active = |plugin: &str| {
        lock.wanted_packages(with_dev).any(|p| p.name() == plugin)
            && matches!(
                crate::layout::plugin_allowed(root_manifest, plugin),
                crate::layout::PluginVerdict::Allowed
            )
    };
    let phpstan = active("phpstan/extension-installer");
    let rector = active("rector/extension-installer");
    let infection = active("infection/extension-installer");
    if !phpstan && !rector && !infection {
        return None;
    }
    for p in lock.wanted_packages(with_dev) {
        let extra = p.raw.get("extra");
        let listed = (phpstan
            && (p.package_type() == "phpstan-extension"
                || extra.and_then(|e| e.get("phpstan")).is_some()
                || (p.name().contains("phpstan")
                    && ![
                        "phpstan/phpstan",
                        "phpstan/phpstan-shim",
                        "phpstan/phpdoc-parser",
                        "phpstan/extension-installer",
                    ]
                    .contains(&p.name()))))
            || (rector
                && (p.package_type() == "rector-extension"
                    || extra.and_then(|e| e.get("rector")).is_some()))
            // infection keys its valid and invalid extensions by name too.
            || (infection && p.package_type() == "infection-extension");
        if !listed {
            continue;
        }
        // `Locker::getLockedRepository` attaches a lock alias by name.
        let inline = lock
            .aliases
            .iter()
            .any(|a| a.get("package").and_then(Value::as_str) == Some(p.name()));
        let branch = crate::root_version::branch_alias_of(
            p.version(),
            extra,
            p.raw.get("default-branch") == Some(&Value::Bool(true)),
        );
        let default_branch_archive = branch
            .as_ref()
            .is_some_and(|(alias, _)| alias == crate::root_version::DEFAULT_BRANCH_ALIAS)
            && p.dist_kind() != crate::lock::DistKind::Path;
        if inline || (branch.is_some() && !default_branch_archive) {
            return Some(ScopeIssue::Layout(format!(
                "package {} {} is aliased: the version the extension installers write for it depends on Composer's repository order (not reproduced)",
                p.name(),
                p.version()
            )));
        }
    }
    None
}

/// Plugins proven to write nothing at install time under a Composer whose
/// plugins are active (the corpus baseline, docs/corpus/): installed as
/// libraries, reported with a note. A plugin that writes a file is either
/// emulated (EMULATED_PLUGINS) or unknown — never listed here. Removed on
/// the corpus's evidence (2026-09-16): pestphp/pest-plugin
/// (vendor/pest-plugins.json), phpstan/extension-installer and
/// rector/extension-installer (GeneratedConfig.php),
/// dealerdirect/phpcodesniffer-composer-installer (CodeSniffer.conf).
/// `metasyntactical/composer-plugin-license-check` checks each installed
/// package's licence against the root's `extra` lists and throws when one
/// is denied — Composer stops, vendor/ half written. With no list
/// configured every package passes and the plugin writes nothing: benign
/// then, handed over when it is configured.
pub const LICENSE_CHECK_PLUGIN: &str = "metasyntactical/composer-plugin-license-check";

/// Its configuration, as `ComposerConfig` reads it: `array_filter` over
/// `allow-list`, `deny-list` and `allowed-packages`, so empty (or absent)
/// lists mean no check at all.
pub fn license_check_configured(root_manifest: &Value) -> bool {
    let Some(cfg) = root_manifest
        .get("extra")
        .and_then(|e| e.get(LICENSE_CHECK_PLUGIN))
    else {
        return false;
    };
    ["allow-list", "deny-list"].iter().any(|k| {
        cfg.get(*k).and_then(Value::as_array).is_some_and(|l| {
            l.iter()
                .any(|v| !matches!(v, Value::Null | Value::Bool(false)))
        })
    })
}

/// `drupal/core-project-message` prints at `POST_INSTALL_CMD`
/// (`MessagePlugin::displayPostCreateMessage`) what `Message::getText`
/// returns for `post-install-cmd`: the `extra` message, else the lines of
/// `post-install-cmd-message.txt` (or the configured `-file`), plus the
/// root's `include-keys`. Not emulated (GPL-2.0-or-later, NOTICE.md): an
/// `install` where it would print goes to Composer
/// (docs/plans/plugin-lines.md). `update` dispatches `POST_UPDATE_CMD`,
/// which it does not listen to.
pub const CORE_PROJECT_MESSAGE: &str = "drupal/core-project-message";

/// The `install` issue of [`CORE_PROJECT_MESSAGE`], when it would print.
pub fn core_project_message_issue(
    project_dir: &Path,
    lock: &Lock,
    root_manifest: &Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> Option<ScopeIssue> {
    if !plugins_enabled
        || !lock
            .wanted_packages(with_dev)
            .any(|p| p.name() == CORE_PROJECT_MESSAGE)
        || !matches!(
            crate::layout::plugin_allowed(root_manifest, CORE_PROJECT_MESSAGE),
            crate::layout::PluginVerdict::Allowed
        )
    {
        return None;
    }
    // PHP's truthiness of a decoded JSON value.
    let truthy = |v: Option<&Value>| match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty() && s != "0",
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    };
    let config = root_manifest
        .get("extra")
        .and_then(|e| e.get("drupal-core-project-message"));
    let get = |key: &str| config.and_then(|c| c.get(key));
    let file = get("post-install-cmd-file")
        .filter(|v| truthy(Some(v)))
        .and_then(Value::as_str)
        .unwrap_or("post-install-cmd-message.txt");
    let from_file = std::fs::read(project_dir.join(file)).is_ok_and(|b| !b.is_empty());
    // `include-keys`: a line per key the root has a value for (`getName`
    // is never empty: `__root__` without a name).
    let include_keys = get("include-keys")
        .and_then(Value::as_array)
        .is_some_and(|keys| {
            keys.iter().filter_map(Value::as_str).any(|k| match k {
                "name" => true,
                "description" | "homepage" | "support" => truthy(root_manifest.get(k)),
                _ => false,
            })
        });
    if truthy(get("post-install-cmd-message")) || from_file || include_keys {
        return Some(ScopeIssue::Layout(format!(
            "plugin {CORE_PROJECT_MESSAGE} would print its post-install-cmd message (not emulated)"
        )));
    }
    None
}

pub const BENIGN_PLUGINS: &[&str] = &[
    "php-http/discovery",
    // Only listens to POST_CREATE_PROJECT_CMD / POST_INSTALL_CMD to print a
    // message (MessagePlugin::getSubscribedEvents): no disk effect — and no
    // line when no message is configured; an `install` where it would print
    // goes to Composer ([`core_project_message_issue`]).
    CORE_PROJECT_MESSAGE,
    // Only listens to POST_UPDATE_CMD / POST_CREATE_PROJECT_CMD, and only acts
    // in a `require` context (Plugin::getSubscribedEvents): inert at install.
    "drupal/core-recipe-unpack",
    LICENSE_CHECK_PLUGIN,
    // A `CommandProvider` (`ibexa setup`) whose activate/deactivate only
    // write at DEBUG verbosity; no listener (read at v5.0.10).
    "ibexa/post-install",
    // `Thanks::activate` arms its reminder only when the command is
    // `update`; `POST_PACKAGE_UPDATE` then `POST_UPDATE_CMD` print it after
    // a GitHub GraphQL call. On `install`: two commands added, nothing
    // else (read at v1.4.1).
    "symfony/thanks",
    // A `CommandProvider` (`composer normalize`) with an empty `activate`
    // and no listener (NormalizePlugin, read at 2.45.0).
    "ergebnis/composer-normalize",
];

/// php-http/discovery (a benign plugin at install, inert at resolution).
pub const DISCOVERY: &str = "php-http/discovery";

/// The abstractions php-http/discovery's `POST_UPDATE_CMD` listener looks
/// after (`Plugin::PROVIDE_RULES` keys, the same from 1.15.0 — its first
/// version as a plugin — to 1.20.0).
pub const DISCOVERY_ABSTRACTIONS: &[&str] = &[
    "php-http/async-client-implementation",
    "php-http/client-implementation",
    "psr/http-client-implementation",
    "psr/http-message-implementation",
    "psr/http-factory-implementation",
];

/// Whether php-http/discovery's `Plugin::postUpdate` has anything to look
/// after (`getMissingRequires`, Plugin.php:243-268 at 1.20.0): the root's
/// `require` and `require-dev` when the root's `require` names
/// php-http/discovery, plus the `require` of every package of the local
/// repository (`installed`, installed.json entries) that requires it — one
/// of them naming an abstraction it provides implementations for. Pinning
/// through `extra.discovery` (from 1.18.0, and only for the four
/// abstractions of `INTERFACE_MAP`) is not read: a pinned abstraction counts
/// as looked after, which only ever hands the command over.
pub fn discovery_watches(root_manifest: &Value, installed: &[Value]) -> bool {
    let keys = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_object)
            .map(|m| m.keys().map(|k| k.to_ascii_lowercase()).collect())
            .unwrap_or_default()
    };
    let mut required: Vec<String> = Vec::new();
    let root_require = keys(root_manifest.get("require"));
    if root_require.iter().any(|k| k == DISCOVERY) {
        required.extend(root_require);
        required.extend(keys(root_manifest.get("require-dev")));
    }
    for p in installed {
        let requires = keys(p.get("require"));
        if requires.iter().any(|k| k == DISCOVERY) {
            required.extend(requires);
        }
    }
    required
        .iter()
        .any(|k| DISCOVERY_ABSTRACTIONS.contains(&k.as_str()))
}

/// Plugins known to change the install layout or the package contents:
/// always out of scope.
pub const LAYOUT_PLUGINS: &[&str] = &["cweagans/composer-patches"];

/// The resolution commands (`update`, `require`, `remove`): Composer loads
/// the installed plugins before resolving (`PluginManager::loadInstalledPlugins`
/// from installed.json, global ones included), and some of them change what
/// gets resolved or written. Read in the plugins' sources (plan v0.16):
/// these have no listener that touches the resolution or the lock —
/// `POST_INSTALL/UPDATE_CMD` writers of files under vendor/ (handled at
/// install), install-path mappers, message printers.
pub const RESOLUTION_INERT: &[&str] = &[
    "composer/installers",
    "symfony/runtime",
    "pestphp/pest-plugin",
    "dealerdirect/phpcodesniffer-composer-installer",
    "phpstan/extension-installer",
    "rector/extension-installer",
    "infection/extension-installer",
    "composer/package-versions-deprecated",
    "ergebnis/composer-normalize",
    // `POST_INSTALL_CMD` / `POST_UPDATE_CMD` copy `c3.php` to the root:
    // the install side's business, emulated there.
    "codeception/c3",
    // `COMMAND` forwards `install` / `update` to vendor-bin/* when
    // `forward-command` is true — nested installs, the root lock untouched;
    // the install side refuses that configuration.
    "bamarni/composer-bin-plugin",
    // An installer for the `wordpress-core` type: install paths only.
    "roots/wordpress-core-installer",
    // `COMMAND` (a verbose-only line) and `POST_PACKAGE_INSTALL/UPDATE`,
    // where it only ever throws: the lock is untouched either way.
    "metasyntactical/composer-plugin-license-check",
    "ibexa/post-install",
    // `POST_INSTALL/UPDATE_CMD`: an in-process `require` of a PSR
    // implementation only when one is missing — with a complete lock, a
    // no-op (verified on install with the corpus).
    "php-http/discovery",
    "drupal/core-project-message",
    // `PRE_UPDATE_CMD` gathers patches, `POST_PACKAGE_*` applies them: the
    // lock is untouched; the install side already hands these to Composer.
    "cweagans/composer-patches",
    "mlocati/composer-patcher",
    "oomphinc/composer-installers-extender",
    "mnsami/composer-custom-directory-installer",
    // Installers only (docs/plans/custom-installers.md): install paths,
    // decided by the install side. craftcms writes plugins.php when it is
    // activated for a `craft-plugin` root — handed over below.
    crate::custom_installers::KIRBY,
    crate::custom_installers::CRAFT,
    crate::custom_installers::NEOS,
    crate::custom_installers::DAVIDBARRATT,
    crate::custom_installers::JOHNPBLOCH,
    crate::custom_installers::THELIA,
    crate::custom_installers::CONTAO_COMPONENTS,
    // docs/plans/plugins-round2.md: nothing at resolution. Their effects
    // belong to the install that follows (or, without one, to
    // `POST_UPDATE_CMD`), checked on the solved lock by
    // `effect_plugins::issues`.
    crate::effect_plugins::SPIRAL,
    crate::effect_plugins::SCULPIN,
    crate::effect_plugins::LAMINAS_COMPONENT,
    crate::effect_plugins::LAMINAS_SKELETON,
    crate::effect_plugins::DG_CLEANER,
    crate::effect_plugins::VENDOR_CLEANER,
    crate::effect_plugins::CAKEPHP,
    // docs/plans/duration-drupol-collector.md: `POST_AUTOLOAD_DUMP` only.
    crate::effect_plugins::DRUPOL,
    // docs/plans/remaining-fallbacks.md: `PRE_AUTOLOAD_DUMP`; and the
    // TYPO3 installers, inert under the root `typo3/cms` (observed).
    crate::effect_plugins::CLASS_ALIAS_LOADER,
    crate::effect_plugins::CMS_INSTALLERS,
    // `PRE_INSTALL_CMD` / `POST_INSTALL_CMD`: patches applied by `patch`.
    crate::effect_plugins::PATCHER,
];

/// A resolution command, for the per-plugin rules below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionCommand {
    Update,
    Require,
    Remove,
}

impl ResolutionCommand {
    pub fn name(self) -> &'static str {
        match self {
            ResolutionCommand::Update => "update",
            ResolutionCommand::Require => "require",
            ResolutionCommand::Remove => "remove",
        }
    }
}

/// What an active plugin does to `command`, as far as the lock is
/// concerned: nothing (inert, or emulated for this command), or something
/// vivacity does not emulate (the reason, for the fallback line).
/// `with_install`: the command installs after resolving (no `--no-install`).
pub fn resolution_effect(
    plugin: &str,
    command: ResolutionCommand,
    with_install: bool,
) -> Option<String> {
    if RESOLUTION_INERT.contains(&plugin) {
        return None;
    }
    Some(match plugin {
        // `PRE_POOL_CREATE` filters the pool against `extra.symfony.require`
        // (`Restricting packages listed in "symfony/symfony" to …`) —
        // emulated (`vivacity_resolver::flex_filter`) for `update` and
        // `remove` without install. `require` resolves Flex aliases and
        // `POST_UPDATE_CMD` applies recipes to the installed packages
        // (files, symfony.lock): with install, or on `require`, Composer.
        "symfony/flex" => {
            if command == ResolutionCommand::Require {
                "resolves Flex aliases and applies recipes on require (not emulated)".to_owned()
            } else if with_install && command == ResolutionCommand::Remove {
                "applies recipes to the packages it installs (not emulated; --no-install resolves natively)".to_owned()
            } else {
                // `update` with install is decided once the lock is solved
                // (`flex_install_decision`): the recipes Flex would apply
                // depend on the packages the install lays out, and are
                // either reproduced or handed over with their reason.
                return None;
            }
        }
        // `POST_PACKAGE_UPDATE` arms the reminder, `POST_UPDATE_CMD` prints
        // it after a GitHub GraphQL query (`GitHubClient`): without a token
        // that is Composer's GitHub auth prompt (an exception under
        // `--no-interaction`) — stderr and exit code differ. Only once a
        // package was actually updated, which is not known before
        // resolving: `update` with install, Composer (`activate` arms it
        // for the `update` command only: `require` / `remove` stay inert).
        "symfony/thanks" => {
            if command == ResolutionCommand::Update && with_install {
                "prints a reminder after a package update, after querying GitHub (not emulated; --no-install resolves natively)".to_owned()
            } else {
                return None;
            }
        }
        // `POST_PACKAGE_UPDATE` records updates, `POST_UPDATE_CMD` prints
        // yiisoft/yii2's UPGRADE.md notes when it was updated — only with
        // install, and only known after resolving: with install, Composer.
        "yiisoft/yii2-composer" => {
            if with_install {
                "prints yiisoft/yii2's upgrade notes after an update with install (not emulated; --no-install resolves natively)".to_owned()
            } else {
                return None;
            }
        }
        // INIT / `PRE_UPDATE_CMD` merge other manifests into the root
        // before resolving: emulated (`vivacity_resolver::merge_plugin`),
        // the merged links, stability flags, aliases and references in
        // the pool. A merged `repositories` section is the exception,
        // checked by the caller on the merge itself.
        "wikimedia/composer-merge-plugin" => return None,
        // `POST_UPDATE_CMD`, acting only in a `require` context: unpacks
        // recipes into composer.json.
        "drupal/core-recipe-unpack" => {
            if command == ResolutionCommand::Require {
                "unpacks recipes into composer.json on require (not emulated)".to_owned()
            } else {
                return None;
            }
        }
        _ => "is not on vivacity's known-plugin list".to_owned(),
    })
}

/// The plugins whose `POST_UPDATE_CMD` listener vivacity reproduces from
/// their source in the versions of [`EMULATED_VERSIONS`], and which run
/// without an install too: `update --no-install` (and `require` /
/// `remove`) still dispatch `POST_UPDATE_CMD` (Installer.php:438-441), to
/// the plugins loaded from installed.json. codeception/c3 is absent: its
/// listener is the same from 2.6.0 to 2.9.0, and older versions require
/// `composer-plugin-api ^1.0`, so Composer 2 never loads them;
/// package-versions-deprecated is absent: it listens to
/// `POST_AUTOLOAD_DUMP`, which is not dispatched without an install
/// (docs/plans/no-install-listeners.md).
pub const POST_UPDATE_FILE_LISTENERS: &[&str] = &[
    "dealerdirect/phpcodesniffer-composer-installer",
    "phpstan/extension-installer",
    "rector/extension-installer",
    "infection/extension-installer",
];

/// The plugins Composer would load for a resolution command, and what
/// each does to it: those installed (installed.json of the project and
/// of `COMPOSER_HOME`) that `allow-plugins` allows. Empty under
/// `--no-plugins`. One issue per plugin that is not inert for `command`.
/// `post_update_cmd`: the command dispatches `POST_UPDATE_CMD` (any but a
/// dry run, install or not).
pub fn resolution_issues(
    project_dir: &Path,
    root_manifest: &Value,
    command: ResolutionCommand,
    plugins_enabled: bool,
    with_install: bool,
    post_update_cmd: bool,
) -> Vec<ScopeIssue> {
    let mut issues = Vec::new();
    let installed = crate::layout::installed_packages_of(project_dir, root_manifest);
    for name in active_plugins(project_dir, root_manifest, plugins_enabled) {
        // The custom installers: inert for these commands at the versions
        // read, but for what their activation does.
        if crate::custom_installers::READ_VERSIONS
            .iter()
            .any(|(p, _)| *p == name)
        {
            let version = installed
                .iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some(name.as_str()))
                .and_then(|p| p.get("version").and_then(Value::as_str))
                .unwrap_or_default();
            if let Some(reason) = crate::custom_installers::activation_issue(
                &name,
                version,
                root_manifest,
                &installed,
            ) {
                issues.push(ScopeIssue::ResolutionPlugin(name, reason));
                continue;
            }
        }
        // craftcms/plugin-installer's `Plugin::activate` registers a
        // `craft-plugin` root in vendor/craftcms/plugins.php, whatever the
        // command.
        if name == crate::custom_installers::CRAFT
            && root_manifest.get("type").and_then(Value::as_str) == Some("craft-plugin")
        {
            issues.push(ScopeIssue::ResolutionPlugin(
                name,
                "registers the craft-plugin root in vendor/craftcms/plugins.php when it is activated (not emulated)".to_owned(),
            ));
            continue;
        }
        if let Some(effect) = resolution_effect(&name, command, with_install) {
            issues.push(ScopeIssue::ResolutionPlugin(name, effect));
        }
    }
    // An installed plugin whose listeners are emulated in other versions
    // only: the install phase would hand `composer install` over after the
    // lock is written (`POST_INSTALL_CMD` instead of `POST_UPDATE_CMD`); the
    // whole command goes now instead — whatever the command does with the
    // plugin (an update to a version read, its removal: conservative).
    // Without an install, the plugins loaded from installed.json still run
    // their `POST_UPDATE_CMD` listener on the local repository: the same
    // check, for those listeners.
    if plugins_enabled && (with_install || post_update_cmd) {
        let installed = crate::layout::installed_packages_of(project_dir, root_manifest);
        let pairs: Vec<(&str, &str)> = installed
            .iter()
            .filter_map(|p| {
                Some((
                    p.get("name").and_then(Value::as_str)?,
                    p.get("version").and_then(Value::as_str)?,
                ))
            })
            .collect();
        issues.extend(phpcs_issue(project_dir, &pairs, |_| false, root_manifest));
        for p in installed {
            let (Some(name), Some(version)) = (
                p.get("name").and_then(Value::as_str),
                p.get("version").and_then(Value::as_str),
            ) else {
                continue;
            };
            if (with_install || POST_UPDATE_FILE_LISTENERS.contains(&name))
                && matches!(
                    crate::layout::plugin_allowed(root_manifest, name),
                    crate::layout::PluginVerdict::Allowed
                )
            {
                issues.extend(emulated_version_issue(name, version));
            }
        }
    }
    issues
}

/// The plugins Composer loads for a command: `composer-plugin` packages of
/// installed.json (project and `COMPOSER_HOME`) that `allow-plugins`
/// allows. Empty under `--no-plugins`.
pub fn active_plugins(
    project_dir: &Path,
    root_manifest: &Value,
    plugins_enabled: bool,
) -> Vec<String> {
    if !plugins_enabled {
        return Vec::new();
    }
    let mut names: Vec<String> = Vec::new();
    let mut collect = |packages: Vec<Value>| {
        for p in packages {
            if p.get("type").and_then(Value::as_str) == Some("composer-plugin") {
                if let Some(n) = p.get("name").and_then(Value::as_str) {
                    if !names.iter().any(|x| x == n) {
                        names.push(n.to_owned());
                    }
                }
            }
        }
    };
    collect(crate::layout::installed_packages_of(
        project_dir,
        root_manifest,
    ));
    if let Some(home) = crate::fetch::composer_home() {
        collect(crate::layout::installed_packages_at(
            &home.join("vendor").join("composer"),
        ));
    }
    names.retain(|name| {
        matches!(
            crate::layout::plugin_allowed(root_manifest, name),
            crate::layout::PluginVerdict::Allowed
        )
    });
    names
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScopeIssue {
    /// Plugin absent from the known lists: unpredictable behaviour.
    UnknownPlugin(String),
    /// An installed, allowed plugin that changes what `update`/`require`/
    /// `remove` resolve or write (name, what it does).
    ResolutionPlugin(String, String),
    /// Plugin known to change the layout (patches, installers-extender...).
    LayoutPlugin(String),
    /// Non-reproducible layout (composer/installers: version not ported,
    /// framework with custom logic, refused target...), or a plugin
    /// listener vivacity does not reproduce for this project (a plugin
    /// version not read, an aliased extension, a message to print).
    Layout(String),
    /// Package without a usable zip dist (exotic dist, no dist nor source).
    NoUsableDist(String),
    /// A `path` package whose directory is not there (name, url).
    MissingPathSource(String, String),
    /// A package without dist whose source is not installed natively
    /// (name, reason): docs/plans/git-sources.md.
    GitSource(String, String),
    /// A package installed from its source (a git checkout) that the
    /// transaction updates or removes: Composer asks the clone for local
    /// changes and unpushed commits first (`GitDownloader::cleanChanges`).
    SourceInstalled(String),
    /// A `config` key vivacity does not read and that changes the layout
    /// (`vendor-dir`, `bin-dir`, `preferred-install: source`): Composer's
    /// output would differ, so the lock is handed over.
    Config(String),
    /// `wikimedia/composer-merge-plugin` could not be emulated for this
    /// run (a `require` pattern without match, an invalid included file,
    /// or a bare vendor whose lock misses a merged requirement — where
    /// Composer would run the plugin's implicit update).
    MergePlugin(String),
}

impl std::fmt::Display for ScopeIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeIssue::UnknownPlugin(p) => {
                write!(f, "plugin {p} is not on vivacity's known-plugin list")
            }
            ScopeIssue::LayoutPlugin(p) => {
                write!(f, "plugin {p} changes the install layout (not emulated)")
            }
            ScopeIssue::ResolutionPlugin(p, what) => write!(f, "plugin {p} {what}"),
            ScopeIssue::Layout(why) => write!(f, "{why}"),
            ScopeIssue::NoUsableDist(p) => {
                write!(
                    f,
                    "package {p} has no usable dist (no zip or tar dist, no path source)"
                )
            }
            ScopeIssue::MissingPathSource(p, url) => write!(
                f,
                "package {p} is a path package whose directory {url} does not exist"
            ),
            ScopeIssue::GitSource(p, why) => {
                write!(f, "package {p}: git source not installed natively ({why})")
            }
            ScopeIssue::SourceInstalled(p) => write!(
                f,
                "package {p} is installed from source (a git checkout): its update or removal checks the clone for local changes (not emulated)"
            ),
            ScopeIssue::Config(why) => write!(f, "config {why} is not supported natively"),
            ScopeIssue::MergePlugin(why) => write!(f, "wikimedia/composer-merge-plugin: {why}"),
        }
    }
}

#[derive(Debug, Default)]
pub struct ScopeReport {
    /// Blocking: at least one -> fallback (or error without Composer).
    pub issues: Vec<ScopeIssue>,
    /// Non-blocking: harmless plugins ignored, to report on stderr.
    pub skipped_plugins: Vec<String>,
    /// Resolved layout (None if a layout issue blocks).
    pub layout: Option<Layout>,
}

impl ScopeReport {
    pub fn is_native_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// `plugins_enabled` = no `--no-plugins`: with the flag, Composer ignores
/// every plugin, composer/installers included; everything goes into vendor/.
pub fn analyze(
    project_dir: &Path,
    lock: &Lock,
    root_manifest: &Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> ScopeReport {
    let mut report = ScopeReport::default();

    for p in lock.wanted_packages(with_dev) {
        classify_package(project_dir, root_manifest, p, plugins_enabled, &mut report);
    }
    // yii2-composer's installer writes three `Yii.php` shims for
    // `yiisoft/yii2-dev` (`linkBaseYiiFiles`): not emulated.
    if plugins_enabled
        && matches!(
            crate::layout::plugin_allowed(root_manifest, crate::yii2_composer::PLUGIN_NAME),
            crate::layout::PluginVerdict::Allowed
        )
        && lock
            .wanted_packages(with_dev)
            .any(|p| p.name() == crate::yii2_composer::PLUGIN_NAME)
        && lock
            .wanted_packages(with_dev)
            .any(|p| p.name() == crate::yii2_composer::YII2_DEV)
    {
        report.issues.push(ScopeIssue::UnknownPlugin(format!(
            "{} with {} (the Yii.php shims are not emulated); it",
            crate::yii2_composer::PLUGIN_NAME,
            crate::yii2_composer::YII2_DEV
        )));
    }
    // The licence check, configured: a denied licence stops Composer
    // mid-install, which vivacity does not reproduce.
    if plugins_enabled
        && license_check_configured(root_manifest)
        && matches!(
            crate::layout::plugin_allowed(root_manifest, LICENSE_CHECK_PLUGIN),
            crate::layout::PluginVerdict::Allowed
        )
        && lock
            .wanted_packages(with_dev)
            .any(|p| p.name() == LICENSE_CHECK_PLUGIN)
    {
        report.issues.push(ScopeIssue::UnknownPlugin(format!(
            "{LICENSE_CHECK_PLUGIN} with an allow-list or deny-list in `extra` (it stops the install on a denied licence); it"
        )));
    }
    // bamarni/composer-bin-plugin: `forward-command: true` runs nested
    // installs under vendor-bin/ (not emulated); a wrong setting type is
    // Composer's abort.
    if plugins_enabled
        && matches!(
            crate::layout::plugin_allowed(root_manifest, crate::bamarni_bin::PLUGIN_NAME),
            crate::layout::PluginVerdict::Allowed
        )
        && lock
            .wanted_packages(with_dev)
            .any(|p| p.name() == crate::bamarni_bin::PLUGIN_NAME)
    {
        match crate::bamarni_bin::config(root_manifest.get("extra")) {
            Ok(cfg) if cfg.forward_command => report.issues.push(ScopeIssue::UnknownPlugin(
                format!(
                    "{} with extra.bamarni-bin.forward-command true (forwards the command to vendor-bin/*, not emulated); it",
                    crate::bamarni_bin::PLUGIN_NAME
                ),
            )),
            Ok(_) => {}
            Err(msg) => report.issues.push(ScopeIssue::Config(format!(
                "extra.bamarni-bin ({msg})"
            ))),
        }
    }
    // The plugins whose listeners are emulated version by version, and
    // the extension installers' aliased packages.
    if plugins_enabled {
        for p in lock.wanted_packages(with_dev) {
            if matches!(
                crate::layout::plugin_allowed(root_manifest, p.name()),
                crate::layout::PluginVerdict::Allowed
            ) {
                report
                    .issues
                    .extend(emulated_version_issue(p.name(), p.version()));
            }
        }
        report
            .issues
            .extend(aliased_listing_issue(lock, root_manifest, with_dev));
        report
            .issues
            .extend(phpcs_lock_issue(project_dir, lock, root_manifest, with_dev));
    }
    report.issues.extend(
        config_issues(root_manifest, lock, with_dev)
            .into_iter()
            .map(ScopeIssue::Config),
    );
    match Layout::resolve(project_dir, lock, root_manifest, with_dev, plugins_enabled) {
        Ok(layout) => {
            report
                .issues
                .extend(crate::installer::source_install_issues(
                    lock, &layout, with_dev,
                ));
            report.layout = Some(layout);
        }
        Err(issues) => report
            .issues
            .extend(issues.into_iter().map(ScopeIssue::Layout)),
    }
    report
}

/// Blocking plugin issues alone (unknown or layout-changing plugins in the
/// lock), for commands that do not install but would still let Composer run
/// plugin listeners — `dump-autoload` and its PRE_AUTOLOAD_DUMP.
pub fn plugin_issues(lock: &Lock, with_dev: bool) -> Vec<ScopeIssue> {
    let mut report = ScopeReport::default();
    for p in lock.wanted_packages(with_dev) {
        classify_plugin(p, &mut report);
    }
    report.issues
}

/// The `config` keys of the manifest (or the global config) that vivacity
/// does not honour: a `preferred-install` that is not exactly `dist`.
///
/// `DownloadManager::resolvePackageInstallPreference` takes the FIRST pattern
/// that matches, and with no match at all it answers
/// `$package->isDev() ? 'source' : 'dist'`. So `auto` and any partial pattern
/// map install a dev-version package from **source** — a git clone where
/// vivacity extracts a dist, i.e. a different vendor/ and not a missing line.
/// Composer's own default is the string `dist` (`Config`), which is why the
/// common case needs no refusal. A map is accepted only when every value is
/// `dist` AND a `*` pattern makes it total; anything else hands over.
/// (`vendor-dir` / `bin-dir` are resolved by `dirs::Dirs`; the forms it
/// refuses come back as layout issues.)
/// `BaseCommand::getPreferredInstallOptions`: the `preferred-install` this
/// command really runs with, once `--prefer-source`, `--prefer-dist` and
/// `--prefer-install` have had their say — `None` when they say nothing and the
/// config stands.
///
/// The cascade, ported: the config's string sets one of the two flags (`auto`, a
/// map, or anything else sets neither); `--prefer-install` may not be combined
/// with either flag (`InvalidArgumentException`) and its value stands in for the
/// matching flag, `auto` clearing both; finally, if either flag is set, the two
/// flags replace whatever the config said.
///
/// Reported as the effective `preferred-install`, which is what the refusal is
/// about: `config_issues` then judges it per package, exactly as
/// `resolvePackageInstallPreference` does. `Err` is Composer's own usage
/// message, printed as text with exit 1.
pub fn effective_preferred_install(
    root_manifest: &Value,
    prefer_source: bool,
    prefer_dist: bool,
    prefer_install: Option<&str>,
) -> Result<Option<Value>, String> {
    if prefer_install.is_none() && !prefer_source && !prefer_dist {
        return Ok(None);
    }
    let config = root_manifest
        .get("config")
        .and_then(|c| c.get("preferred-install"))
        .cloned()
        .or_else(|| crate::layout::global_config_value("preferred-install"));
    let mut flag_source = prefer_source;
    let mut flag_dist = prefer_dist;
    let mut cleared = false;
    if let Some(value) = prefer_install {
        if prefer_source {
            return Err(
                "--prefer-source can not be used together with --prefer-install".to_owned(),
            );
        }
        if prefer_dist {
            return Err("--prefer-dist can not be used together with --prefer-install".to_owned());
        }
        match value {
            "dist" => flag_dist = true,
            "source" => flag_source = true,
            "auto" => cleared = true,
            other => {
                return Err(format!(
                    "--prefer-install accepts one of \"dist\", \"source\" or \"auto\", got {other}"
                ))
            }
        }
    }
    if flag_source {
        return Ok(Some(Value::String("source".to_owned())));
    }
    if flag_dist {
        return Ok(Some(Value::String("dist".to_owned())));
    }
    if cleared {
        // Both flags off: the config's string no longer applies, but its map
        // still reaches `setPreferences`, so only a string is replaced.
        return Ok(Some(match config {
            Some(Value::Object(m)) => Value::Object(m),
            _ => Value::String("auto".to_owned()),
        }));
    }
    Ok(None)
}

/// `DownloadManager::resolvePackageInstallPreference` plus `Factory`'s switch,/// `DownloadManager::resolvePackageInstallPreference` plus `Factory`'s switch,
/// for one package: would Composer install it from source?
///
/// A string is handled by the switch — `dist` forces dist, `source` forces
/// source, anything else (including `auto`) leaves both flags off and falls
/// through to the per-package rule with no patterns. A map goes to
/// `setPreferences`, where the FIRST matching pattern decides: `dist` is dist,
/// `auto` is dist only for a non-dev package, and anything else is source.
/// With no pattern matching at all: `$package->isDev() ? 'source' : 'dist'`.
fn prefers_source(preference: &Value, name: &str, is_dev: bool) -> bool {
    match preference {
        Value::String(s) => match s.as_str() {
            "dist" => false,
            "source" => true,
            _ => is_dev,
        },
        Value::Object(map) => {
            for (pattern, pref) in map {
                if pattern_matches(pattern, name) {
                    return match pref.as_str() {
                        Some("dist") => false,
                        Some("auto") => is_dev,
                        _ => true,
                    };
                }
            }
            is_dev
        }
        _ => is_dev,
    }
}

/// `'{^'.str_replace('\*', '.*', preg_quote($pattern)).'$}i'`: a whole-name
/// The pattern of `resolvePackageInstallPreference`, built as
/// `preg_quote($pattern)` with `\*` turned into `.*`, anchored, `i`: the whole
/// name, case-insensitive, `*` the only metacharacter and everything else
/// literal. Greedy left to right, as `.*` is.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let pieces: Vec<String> = pattern
        .to_ascii_lowercase()
        .split('*')
        .map(str::to_owned)
        .collect();
    if pieces.len() == 1 {
        return name == pieces[0];
    }
    let first = &pieces[0];
    let last = &pieces[pieces.len() - 1];
    if !name.starts_with(first.as_str()) || !name.ends_with(last.as_str()) {
        return false;
    }
    // The first and last pieces may not overlap in the name.
    if name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for piece in &pieces[1..pieces.len() - 1] {
        match rest.find(piece.as_str()) {
            Some(pos) => rest = &rest[pos + piece.len()..],
            None => return false,
        }
    }
    true
}

/// `Package::isDev()`: the version's stability is `dev`.
fn is_dev_version(version: &str) -> bool {
    version.starts_with("dev-") || version.ends_with("-dev")
}

pub fn config_issues(root_manifest: &Value, lock: &Lock, with_dev: bool) -> Vec<String> {
    let value = |key: &str| -> Option<Value> {
        root_manifest
            .get("config")
            .and_then(|c| c.get(key))
            .cloned()
            .or_else(|| crate::layout::global_config_value(key))
    };
    let mut out = Vec::new();
    if let Some(v) = value("preferred-install") {
        // A package without dist comes from its source whatever the
        // preference (`getAvailableSources` has nothing else to offer).
        if lock
            .wanted_packages(with_dev)
            .filter(|p| p.dist_kind() != DistKind::Missing)
            .any(|p| prefers_source(&v, p.name(), is_dev_version(p.version())))
        {
            out.push(format!("preferred-install {v}"));
        }
    }
    out
}

fn classify_package(
    project_dir: &Path,
    root_manifest: &Value,
    p: &LockPackage,
    plugins_enabled: bool,
    report: &mut ScopeReport,
) {
    // Under --no-plugins Composer never loads a plugin (PluginManager::
    // registerPackage returns at once): the package is a plain library in
    // vendor/, whatever it would do when active, and nothing is printed.
    if plugins_enabled {
        classify_plugin(p, report);
    }
    if p.is_metapackage() {
        return;
    }
    match p.dist_kind() {
        DistKind::Zip | DistKind::Tar => {}
        // A `path` package is laid out natively (symlink or mirror) on
        // Linux/macOS when its source directory is there; Windows
        // (junctions) is left to Composer.
        DistKind::Path => {
            let url = p.dist_url().unwrap_or("");
            if cfg!(unix) && !url.is_empty() && !project_dir.join(url).is_dir() {
                report.issues.push(ScopeIssue::MissingPathSource(
                    p.name().to_owned(),
                    url.to_owned(),
                ));
            } else if !cfg!(unix) || url.is_empty() {
                report
                    .issues
                    .push(ScopeIssue::NoUsableDist(p.name().to_owned()));
            }
        }
        // No dist at all: `DownloadManager::getAvailableSources` leaves
        // only the source, installed by `GitDownloader` when it is git
        // (docs/plans/git-sources.md). A dist of another type is tried
        // first by Composer, with its own fallback to the source: handed
        // over as before.
        // (`dist` absent or null: a `dist` without `type` is
        // `ArrayLoader`'s error, handed over below.)
        DistKind::Missing
            if p.raw.get("dist").is_none_or(Value::is_null)
                && p.raw.get("source").is_some_and(|s| !s.is_null()) =>
        {
            // `SymfonyPackInstaller` lays a pack out as a metapackage when
            // Flex is active: no clone, no line — not reproduced here.
            if p.package_type() == "symfony-pack" {
                report.issues.push(ScopeIssue::GitSource(
                    p.name().to_owned(),
                    "a symfony-pack".to_owned(),
                ));
            } else if let Err(why) = crate::git_source::plan(p, project_dir, root_manifest) {
                report
                    .issues
                    .push(ScopeIssue::GitSource(p.name().to_owned(), why));
            }
        }
        DistKind::Other | DistKind::Missing => report
            .issues
            .push(ScopeIssue::NoUsableDist(p.name().to_owned())),
    }
}

fn classify_plugin(p: &LockPackage, report: &mut ScopeReport) {
    if p.package_type() != "composer-plugin" {
        return;
    }
    let name = p.name().to_owned();
    if EMULATED_PLUGINS.contains(&name.as_str()) {
        // Emulated natively: nothing to report.
    } else if BENIGN_PLUGINS.contains(&name.as_str()) {
        report.skipped_plugins.push(name);
    } else if LAYOUT_PLUGINS.contains(&name.as_str()) {
        report.issues.push(ScopeIssue::LayoutPlugin(name));
    } else {
        report.issues.push(ScopeIssue::UnknownPlugin(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Lock;
    use serde_json::json;

    fn lock_with(packages: serde_json::Value) -> Lock {
        Lock::parse(&json!({ "packages": packages, "packages-dev": [] }).to_string()).expect("lock")
    }

    fn zip_pkg(name: &str, r#type: &str) -> serde_json::Value {
        json!({"name": name, "version": "1.0.0", "type": r#type,
               "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}})
    }

    fn proj() -> std::path::PathBuf {
        std::path::PathBuf::from("/nonexistent-vivacity-scope")
    }

    #[test]
    fn preferred_install_auto_hands_over_for_a_dev_package() {
        // `resolvePackageInstallPreference` with no pattern answers
        // `$package->isDev() ? 'source' : 'dist'`, so `auto` clones a dev
        // version from source — a git checkout where vivacity extracts a dist.
        // Measured against Composer 2.10.3: `  - Syncing … into cache` for the
        // dev packages, `  - Downloading …` for the stable ones.
        let dev = json!({"name": "a/b", "version": "dev-main", "type": "library",
                         "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}});
        for preference in [
            json!("auto"),
            json!("source"),
            json!({"*": "auto"}),
            json!({"a/*": "auto"}),
            // No pattern matches, so the default rule applies to a dev package.
            json!({"other/*": "dist"}),
        ] {
            let lock = lock_with(json!([dev.clone()]));
            let manifest = json!({"config": {"preferred-install": preference}});
            assert!(
                !analyze(&proj(), &lock, &manifest, true, true).is_native_ok(),
                "{preference} with a dev package must hand over"
            );
            // The same preference with a stable package is dist throughout.
            let stable = lock_with(json!([zip_pkg("a/b", "library")]));
            let native = analyze(&proj(), &stable, &manifest, true, true).is_native_ok();
            assert_eq!(
                native,
                preference != json!("source"),
                "{preference} with a stable package"
            );
        }
    }

    #[test]
    fn install_preference_patterns_match_like_pcre() {
        // `preg_quote` then `\*` -> `.*`, anchored, case-insensitive.
        for (pattern, name, want) in [
            ("a/b", "a/b", true),
            ("a/b", "a/bc", false),
            ("A/B", "a/b", true),
            ("*", "anything/at-all", true),
            ("acme/*", "acme/thing", true),
            ("acme/*", "acmex/thing", false),
            ("*/thing", "acme/thing", true),
            ("a*b", "a/xb", true),
            // `.*` matches the empty string.
            ("a*b", "ab", true),
            ("a**b", "a/b", true),
            // A dot is literal, not "any character".
            ("a.b/c", "axb/c", false),
            ("a.b/c", "a.b/c", true),
            ("acme/*-bundle", "acme/my-bundle", true),
            ("acme/*-bundle", "acme/bundle", false),
        ] {
            assert_eq!(
                pattern_matches(pattern, name),
                want,
                "pattern {pattern:?} against {name:?}"
            );
        }
    }

    #[test]
    fn plain_library_is_native() {
        let lock = lock_with(json!([zip_pkg("a/b", "library")]));
        let r = analyze(&proj(), &lock, &json!({}), true, true);
        assert!(r.is_native_ok());
        assert!(r.skipped_plugins.is_empty());
        assert_eq!(r.layout.expect("layout").rel("a/b"), Some("vendor/a/b"));
    }

    #[test]
    fn emulated_and_benign_plugins_stay_native() {
        let lock = lock_with(json!([
            zip_pkg("symfony/runtime", "composer-plugin"),
            zip_pkg("symfony/flex", "composer-plugin"),
        ]));
        let r = analyze(&proj(), &lock, &json!({}), true, true);
        assert!(r.is_native_ok());
        // Both are emulated now (symfony/flex since its POST_INSTALL_CMD
        // listener is): nothing to report as merely skipped.
        assert!(r.skipped_plugins.is_empty(), "{:?}", r.skipped_plugins);
    }

    #[test]
    fn thanks_and_normalize_are_benign_at_install() {
        let lock = lock_with(json!([
            zip_pkg("symfony/thanks", "composer-plugin"),
            zip_pkg("ergebnis/composer-normalize", "composer-plugin"),
        ]));
        let r = analyze(&proj(), &lock, &json!({}), true, true);
        assert!(r.is_native_ok(), "{:?}", r.issues);
        assert_eq!(
            r.skipped_plugins,
            vec!["symfony/thanks", "ergebnis/composer-normalize"]
        );
    }

    #[test]
    fn thanks_reminder_only_on_update_with_install() {
        use ResolutionCommand::*;
        let effect = |c, i| resolution_effect("symfony/thanks", c, i);
        assert!(effect(Update, true).is_some());
        assert!(effect(Update, false).is_none());
        assert!(effect(Require, true).is_none());
        assert!(effect(Remove, true).is_none());
        assert!(resolution_effect("ergebnis/composer-normalize", Update, true).is_none());
    }

    #[test]
    fn yii2_composer_is_emulated_except_with_yii2_dev() {
        let allow = json!({"config": {"allow-plugins": {"yiisoft/yii2-composer": true}}});
        let lock = lock_with(json!([
            zip_pkg("yiisoft/yii2-composer", "composer-plugin"),
            zip_pkg("yiisoft/yii2-gii", "yii2-extension"),
        ]));
        assert!(analyze(&proj(), &lock, &allow, true, true).is_native_ok());
        let lock = lock_with(json!([
            zip_pkg("yiisoft/yii2-composer", "composer-plugin"),
            zip_pkg("yiisoft/yii2-dev", "yii2-extension"),
        ]));
        let r = analyze(&proj(), &lock, &allow, true, true);
        assert!(!r.is_native_ok());
        assert!(
            r.issues[0].to_string().contains("yiisoft/yii2-dev"),
            "{}",
            r.issues[0]
        );
        // Under --no-plugins the shims are not written by Composer either.
        assert!(analyze(&proj(), &lock, &allow, true, false).is_native_ok());
    }

    #[test]
    fn license_check_native_unless_configured() {
        let lock = lock_with(json!([zip_pkg(
            "metasyntactical/composer-plugin-license-check",
            "composer-plugin"
        )]));
        let manifest = |extra: serde_json::Value| json!({"config": {"allow-plugins": {"metasyntactical/composer-plugin-license-check": true}}, "extra": extra});
        // No list: `ComposerConfig` filters everything away, nothing checked.
        for extra in [
            json!({}),
            json!({"metasyntactical/composer-plugin-license-check": {}}),
            json!({"metasyntactical/composer-plugin-license-check": {"allow-list": []}}),
            json!({"metasyntactical/composer-plugin-license-check": {"allowed-packages": {"a/b": "why"}}}),
        ] {
            let r = analyze(&proj(), &lock, &manifest(extra.clone()), true, true);
            assert!(
                r.is_native_ok(),
                "{extra} should stay native: {:?}",
                r.issues
            );
        }
        for key in ["allow-list", "deny-list"] {
            let extra = json!({"metasyntactical/composer-plugin-license-check": {key: ["MIT"]}});
            let r = analyze(&proj(), &lock, &manifest(extra), true, true);
            assert!(
                r.issues[0].to_string().contains("allow-list or deny-list"),
                "{key}: {:?}",
                r.issues
            );
        }
    }

    #[test]
    fn bamarni_bin_native_unless_it_forwards() {
        let lock = lock_with(json!([zip_pkg(
            "bamarni/composer-bin-plugin",
            "composer-plugin"
        )]));
        let allow = |extra: serde_json::Value| json!({"config": {"allow-plugins": {"bamarni/composer-bin-plugin": true}}, "extra": extra});
        assert!(analyze(&proj(), &lock, &allow(json!({})), true, true).is_native_ok());
        let r = analyze(
            &proj(),
            &lock,
            &allow(json!({"bamarni-bin": {"forward-command": true}})),
            true,
            true,
        );
        assert!(
            r.issues[0].to_string().contains("forward-command true"),
            "{}",
            r.issues[0]
        );
        let r = analyze(
            &proj(),
            &lock,
            &allow(json!({"bamarni-bin": {"bin-links": "no"}})),
            true,
            true,
        );
        assert!(
            r.issues[0].to_string().contains("boolean value"),
            "{}",
            r.issues[0]
        );
    }

    #[test]
    fn no_plugins_makes_every_plugin_a_plain_library() {
        let lock = lock_with(json!([
            zip_pkg("acme/mystery-plugin", "composer-plugin"),
            zip_pkg("cweagans/composer-patches", "composer-plugin"),
            zip_pkg("symfony/flex", "composer-plugin"),
        ]));
        let r = analyze(&proj(), &lock, &json!({}), true, false);
        assert!(r.is_native_ok(), "{:?}", r.issues);
        assert!(r.skipped_plugins.is_empty());
        assert_eq!(
            r.layout.expect("layout").rel("acme/mystery-plugin"),
            Some("vendor/acme/mystery-plugin")
        );
    }

    #[test]
    fn unknown_or_layout_plugin_is_out_of_scope() {
        let lock = lock_with(json!([
            zip_pkg("acme/mystery-plugin", "composer-plugin"),
            zip_pkg("cweagans/composer-patches", "composer-plugin"),
        ]));
        let r = analyze(&proj(), &lock, &json!({}), true, true);
        assert_eq!(
            r.issues,
            vec![
                ScopeIssue::UnknownPlugin("acme/mystery-plugin".into()),
                ScopeIssue::LayoutPlugin("cweagans/composer-patches".into()),
            ]
        );
    }

    #[test]
    fn installers_without_allow_plugins_and_sourceless_dist_are_out_of_scope() {
        let lock = lock_with(json!([
            {"name": "a/src-only", "version": "1.0.0", "type": "library",
             "source": {"type": "svn", "url": "https://g/x", "reference": "r"}},
            {"name": "a/meta", "version": "1.0.0", "type": "metapackage"},
            zip_pkg("composer/installers", "composer-plugin"),
        ]));
        // installer-paths alone is inert (as in Composer); the plugin without
        // allow-plugins, however, blocks.
        let manifest = json!({"extra": {"installer-paths": {"web/modules/{$name}": []}}});
        let r = analyze(&proj(), &lock, &manifest, true, true);
        assert_eq!(r.issues.len(), 2, "{:?}", r.issues);
        // Only a git source is installed natively (docs/plans/git-sources.md).
        assert_eq!(
            r.issues[0],
            ScopeIssue::GitSource(
                "a/src-only".into(),
                if cfg!(windows) {
                    "git sources are installed by Composer on Windows".into()
                } else {
                    "source type `svn` (only git is installed natively)".into()
                }
            )
        );
        assert!(matches!(&r.issues[1], ScopeIssue::Layout(m) if m.contains("allow-plugins")));
    }

    #[test]
    fn path_package_is_native_when_its_source_exists() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::create_dir_all(tmp.path().join("packages/here")).expect("mkdir");
        let lock = lock_with(json!([
            {"name": "a/here", "version": "dev-main", "type": "library",
             "dist": {"type": "path", "url": "packages/here", "reference": "r"}},
            {"name": "a/gone", "version": "dev-main", "type": "library",
             "dist": {"type": "path", "url": "packages/gone", "reference": "r"}},
        ]));
        let r = analyze(tmp.path(), &lock, &json!({}), true, true);
        let expected = if cfg!(unix) {
            vec![ScopeIssue::MissingPathSource(
                "a/gone".into(),
                "packages/gone".into(),
            )]
        } else {
            vec![
                ScopeIssue::NoUsableDist("a/here".into()),
                ScopeIssue::NoUsableDist("a/gone".into()),
            ]
        };
        assert_eq!(r.issues, expected);
    }

    #[test]
    fn unread_config_keys_are_scope_issues() {
        // `preferred-install` is judged per package, as
        // `resolvePackageInstallPreference` judges it: only a package that
        // would come from source is a reason to hand over.
        let lock = lock_with(json!([zip_pkg("acme/thing", "library")]));
        let manifest = json!({"config": {"vendor-dir": "lib/", "bin-dir": "vendor/bin",
            "preferred-install": {"acme/*": "source", "*": "dist"}}});
        let r = analyze(&proj(), &lock, &manifest, true, true);
        assert_eq!(
            r.issues,
            vec![ScopeIssue::Config(
                "preferred-install {\"acme/*\":\"source\",\"*\":\"dist\"}".into()
            ),]
        );
        // The same map with a package no source pattern matches: Composer
        // installs it from dist, so there is nothing to hand over.
        let lock = lock_with(json!([zip_pkg("a/b", "library")]));
        let manifest = json!({"config": {"vendor-dir": "vendor",
            "preferred-install": {"acme/*": "source", "*": "dist"}}});
        assert!(analyze(&proj(), &lock, &manifest, true, true).is_native_ok());
        // `auto` with a stable package is dist.
        let manifest = json!({"config": {"vendor-dir": "vendor", "preferred-install": "auto"}});
        assert!(analyze(&proj(), &lock, &manifest, true, true).is_native_ok());
    }

    #[test]
    fn no_dev_skips_dev_packages() {
        let lock = Lock::parse(
            &json!({
                "packages": [zip_pkg("a/b", "library")],
                "packages-dev": [zip_pkg("acme/mystery-plugin", "composer-plugin")]
            })
            .to_string(),
        )
        .expect("lock");
        assert!(analyze(&proj(), &lock, &json!({}), false, true).is_native_ok());
        assert!(!analyze(&proj(), &lock, &json!({}), true, true).is_native_ok());
    }

    #[test]
    fn listener_plugins_outside_the_versions_read_are_handed_over() {
        let manifest = json!({"config": {"allow-plugins": {
            "dealerdirect/phpcodesniffer-composer-installer": true,
            "phpstan/extension-installer": true}}});
        let lock = |dd: &str, phpstan: &str| {
            lock_with(json!([
                {"name": "dealerdirect/phpcodesniffer-composer-installer", "version": dd,
                 "type": "composer-plugin", "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}},
                {"name": "phpstan/extension-installer", "version": phpstan,
                 "type": "composer-plugin", "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}},
            ]))
        };
        let issues = |l: &Lock, plugins: bool| -> Vec<String> {
            analyze(&proj(), l, &manifest, true, plugins)
                .issues
                .iter()
                .map(ToString::to_string)
                .filter(|i| i.contains("in this version"))
                .collect()
        };
        assert!(issues(&lock("v1.2.1", "1.4.3"), true).is_empty());
        assert!(issues(&lock("v1.0.0", "1.4.0"), true).is_empty());
        let found = issues(&lock("v0.7.1", "1.3.1"), true);
        assert_eq!(found.len(), 2);
        assert!(found[0].starts_with(
            "plugin dealerdirect/phpcodesniffer-composer-installer v0.7.1 is not emulated in this version (versions read: 0.7.2, 1.0.0, 1.1.1, 1.1.2, 1.2.0, 1.2.1;"
        ));
        assert!(found[1].contains("1.3.x writes no PHPSTAN_VERSION_CONSTRAINT"));
        // v1.1.0, a dev branch, a version not released yet.
        assert_eq!(issues(&lock("v1.1.0", "dev-main"), true).len(), 2);
        assert_eq!(issues(&lock("v1.3.0", "1.5.0"), true).len(), 2);
        // Not loaded: --no-plugins, or not allowed.
        assert!(issues(&lock("v0.7.1", "1.3.1"), false).is_empty());
        assert!(issues(&lock("v0.7.2", "1.4.3"), true).is_empty());
        let refused = json!({"config": {"allow-plugins": {
            "dealerdirect/phpcodesniffer-composer-installer": false,
            "phpstan/extension-installer": false}}});
        assert!(
            analyze(&proj(), &lock("v0.7.1", "1.3.1"), &refused, true, true)
                .issues
                .iter()
                .all(|i| !i.to_string().contains("in this version"))
        );
    }

    #[test]
    fn dealerdirect_configurations_not_reproduced_are_handed_over() {
        let manifest = |extra: Value| {
            json!({"config": {"allow-plugins": {
                "dealerdirect/phpcodesniffer-composer-installer": true}}, "extra": extra})
        };
        let lock = |dd: &str, phpcs: Option<&str>, aliased: bool| {
            let mut packages = vec![
                json!({"name": "dealerdirect/phpcodesniffer-composer-installer",
                "version": dd, "type": "composer-plugin",
                "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}}),
            ];
            if let Some(v) = phpcs {
                packages.push(zip_pkg("squizlabs/php_codesniffer", "library"));
                packages[1]["version"] = json!(v);
            }
            let aliases = if aliased {
                json!([{"package": "squizlabs/php_codesniffer", "version": "dev-master",
                        "alias": "3.99.0", "alias_normalized": "3.99.0.0"}])
            } else {
                json!([])
            };
            Lock::parse(
                &json!({"packages": packages, "packages-dev": [], "aliases": aliases}).to_string(),
            )
            .expect("lock")
        };
        let found = |l: &Lock, m: &Value| -> Vec<String> {
            analyze(&proj(), l, m, true, true)
                .issues
                .iter()
                .map(ToString::to_string)
                .filter(|i| {
                    i.contains("phpcodesniffer-composer-installer with")
                        || i.contains(" with squizlabs/")
                })
                .collect()
        };
        let plain = manifest(json!({}));
        assert!(found(&lock("v0.7.2", Some("3.13.6"), false), &plain).is_empty());
        assert!(found(&lock("v1.2.1", Some("2.9.2"), false), &plain).is_empty());
        // No phpcs: the listener does nothing, whatever the depth.
        assert!(found(
            &lock("v0.7.2", None, false),
            &manifest(json!({"phpcodesniffer-search-depth": "x"}))
        )
        .is_empty());
        let one = |l: &Lock, m: &Value| {
            let f = found(l, m);
            assert_eq!(f.len(), 1, "{f:?}");
            f[0].clone()
        };
        assert!(one(&lock("v0.7.2", Some("2.9.2"), false), &plain).contains("./scripts/phpcs"));
        assert!(one(&lock("v1.0.0", Some("2.9.2"), false), &plain).contains("./scripts/phpcs"));
        assert!(one(&lock("v1.1.2", Some("2.9.2"), false), &plain).contains("./scripts/phpcs"));
        assert!(
            one(&lock("v1.2.1", Some("dev-master"), false), &plain).contains("constraint matching")
        );
        assert!(
            one(&lock("v1.2.1", Some("4.0.0-RC1"), false), &plain).contains("constraint matching")
        );
        assert!(one(&lock("v1.2.1", Some("3.13.6"), true), &plain).contains("3.13.6 aliased"));
        for bad in [
            json!("4"),
            json!(0),
            json!(-1),
            json!(2.0),
            json!(null),
            json!(true),
        ] {
            assert!(one(
                &lock("v0.7.2", Some("3.13.6"), false),
                &manifest(json!({"phpcodesniffer-search-depth": bad}))
            )
            .contains("extra.phpcodesniffer-search-depth"));
        }
        // phpcs 2: the minimum depth is 1, so 1 is refused and 2 read.
        assert!(!found(
            &lock("v1.2.1", Some("2.9.2"), false),
            &manifest(json!({"phpcodesniffer-search-depth": 1}))
        )
        .is_empty());
        assert!(found(
            &lock("v1.2.1", Some("2.9.2"), false),
            &manifest(json!({"phpcodesniffer-search-depth": 2}))
        )
        .is_empty());
        assert!(found(
            &lock("v0.7.2", Some("3.13.6"), false),
            &manifest(json!({"phpcodesniffer-search-depth": 1}))
        )
        .is_empty());
    }

    #[test]
    fn core_project_message_hands_over_an_install_it_would_print_on() {
        let tmp = tempfile::tempdir().expect("tmp");
        let lock = lock_with(json!([zip_pkg(CORE_PROJECT_MESSAGE, "composer-plugin")]));
        let allowed = |extra: Value| json!({"config": {"allow-plugins": {CORE_PROJECT_MESSAGE: true}}, "extra": extra});
        let issue = |m: &Value| core_project_message_issue(tmp.path(), &lock, m, true, true);
        assert!(issue(&allowed(json!({}))).is_none());
        // Only post-create-project-cmd configured: silent at install.
        let create_only = allowed(json!({"drupal-core-project-message": {
            "post-create-project-cmd-message": ["hello"]}}));
        assert!(issue(&create_only).is_none());
        for extra in [
            json!({"drupal-core-project-message": {"post-install-cmd-message": ["hi"]}}),
            json!({"drupal-core-project-message": {"include-keys": ["name"]}}),
        ] {
            assert!(issue(&allowed(extra)).is_some());
        }
        // `include-keys`: a line only for a key the root has a value for.
        let homepage =
            allowed(json!({"drupal-core-project-message": {"include-keys": ["homepage"]}}));
        assert!(issue(&homepage).is_none());
        let mut with_homepage = homepage.clone();
        with_homepage["homepage"] = json!("https://example.org");
        assert!(issue(&with_homepage).is_some());
        std::fs::write(tmp.path().join("post-install-cmd-message.txt"), "hi\n").expect("write");
        assert!(issue(&create_only).is_some());
        // Not loaded: refused, or --no-plugins.
        let refused = json!({"config": {"allow-plugins": {CORE_PROJECT_MESSAGE: false}}});
        assert!(issue(&refused).is_none());
        assert!(core_project_message_issue(tmp.path(), &lock, &create_only, true, false).is_none());
    }

    #[test]
    fn an_aliased_extension_is_handed_over_unless_a_default_branch_archive() {
        let manifest = json!({"config": {"allow-plugins": {"phpstan/extension-installer": true}}});
        let plugin = json!({"name": "phpstan/extension-installer", "version": "1.4.3",
            "type": "composer-plugin", "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}});
        let ext = |version: &str, extra: Value, dist: &str| {
            json!({"name": "a/phpstan-ext", "version": version, "type": "phpstan-extension",
                   "extra": extra, "dist": {"type": dist, "url": "https://x/y.zip", "reference": "r"}})
        };
        let issues = |lock: &Lock| -> Vec<String> {
            analyze(&proj(), lock, &manifest, true, true)
                .issues
                .iter()
                .map(ToString::to_string)
                .filter(|i| i.contains("is aliased"))
                .collect()
        };
        let plain = lock_with(json!([plugin, ext("1.0.0", json!({}), "zip")]));
        assert!(issues(&plain).is_empty());
        let mut default_branch = ext("dev-main", json!({}), "zip");
        default_branch["default-branch"] = json!(true);
        assert!(issues(&lock_with(json!([plugin, default_branch]))).is_empty());
        let branch = ext(
            "dev-main",
            json!({"branch-alias": {"dev-main": "1.0.x-dev"}}),
            "zip",
        );
        assert_eq!(issues(&lock_with(json!([plugin, branch]))).len(), 1);
        let inline = Lock::parse(
            &json!({"packages": [plugin, ext("1.0.0", json!({}), "zip")], "packages-dev": [],
                    "aliases": [{"package": "a/phpstan-ext", "version": "1.0.0.0",
                                 "alias": "1.0.99", "alias_normalized": "1.0.99.0"}]})
            .to_string(),
        )
        .expect("lock");
        assert_eq!(issues(&inline).len(), 1);
    }

    #[test]
    fn post_update_listener_versions_are_checked_without_install_too() {
        let tmp = tempfile::tempdir().expect("tmp");
        let composer = tmp.path().join("vendor/composer");
        std::fs::create_dir_all(&composer).expect("mkdir");
        std::fs::write(
            composer.join("installed.json"),
            json!({"packages": [
                {"name": "phpstan/extension-installer", "version": "1.3.1", "type": "composer-plugin"},
                {"name": "composer/package-versions-deprecated", "version": "1.11.99.1", "type": "composer-plugin"},
            ]})
            .to_string(),
        )
        .expect("installed.json");
        let manifest = json!({"config": {"allow-plugins": {
            "phpstan/extension-installer": true, "composer/package-versions-deprecated": true}}});
        let names = |with_install: bool, post_update_cmd: bool| -> Vec<String> {
            resolution_issues(
                tmp.path(),
                &manifest,
                ResolutionCommand::Update,
                true,
                with_install,
                post_update_cmd,
            )
            .iter()
            .map(ToString::to_string)
            .collect()
        };
        // With an install: every listener vivacity reproduces.
        assert_eq!(names(true, true).len(), 2);
        // `--no-install`: `POST_UPDATE_CMD` still runs, `POST_AUTOLOAD_DUMP`
        // does not — phpstan only.
        let no_install = names(false, true);
        assert_eq!(no_install.len(), 1, "{no_install:?}");
        assert!(no_install[0].contains("phpstan/extension-installer 1.3.1"));
        // A dry run dispatches nothing.
        assert!(names(false, false).is_empty());
        // `--no-plugins`: nothing is loaded.
        assert!(resolution_issues(
            tmp.path(),
            &manifest,
            ResolutionCommand::Update,
            false,
            false,
            true
        )
        .is_empty());
    }

    #[test]
    fn discovery_watches_what_postupdate_reads() {
        let impl_pkg = json!({"name": "acme/sdk", "require": {"php-http/discovery": "^1.15", "psr/http-client-implementation": "*"}});
        let plain =
            json!({"name": "acme/other", "require": {"psr/http-client-implementation": "*"}});
        // Nothing requires php-http/discovery: nothing looked after.
        assert!(!discovery_watches(
            &json!({"require": {"psr/http-client-implementation": "*"}}),
            std::slice::from_ref(&plain)
        ));
        // The root requires it (in `require`): its own links count,
        // `require-dev` included.
        assert!(discovery_watches(
            &json!({"require": {"php-http/discovery": "^1.20"}, "require-dev": {"psr/http-factory-implementation": "*"}}),
            &[]
        ));
        // Only in `require-dev`: the root's links do not count.
        assert!(!discovery_watches(
            &json!({"require-dev": {"php-http/discovery": "^1.20", "psr/http-client-implementation": "*"}}),
            &[]
        ));
        // An installed package that requires it brings its own `require`.
        assert!(discovery_watches(
            &json!({}),
            std::slice::from_ref(&impl_pkg)
        ));
        // A pin in `extra.discovery` is not read: still looked after.
        assert!(discovery_watches(
            &json!({"extra": {"discovery": {"psr/http-client-implementation": "Acme\\Client"}}}),
            &[impl_pkg]
        ));
    }
}
