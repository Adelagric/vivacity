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

/// Plugins emulated natively by vivacity (identical output, drift test).
/// composer/installers (see `layout`) is, under conditions checked before
/// any write. drupal/core-composer-scaffold is deliberately absent: its
/// source is GPL-2.0-or-later and cannot be ported here (NOTICE.md).
pub const EMULATED_PLUGINS: &[&str] = &[
    "symfony/runtime",
    "composer/installers",
    "pestphp/pest-plugin",
    "dealerdirect/phpcodesniffer-composer-installer",
    "phpstan/extension-installer",
    "rector/extension-installer",
    "wikimedia/composer-merge-plugin",
];

/// Plugins proven to write nothing at install time under a Composer whose
/// plugins are active (the corpus baseline, docs/corpus/): installed as
/// libraries, reported with a note. A plugin that writes a file is either
/// emulated (EMULATED_PLUGINS) or unknown — never listed here. Removed on
/// the corpus's evidence (2026-09-16): pestphp/pest-plugin
/// (vendor/pest-plugins.json), phpstan/extension-installer and
/// rector/extension-installer (GeneratedConfig.php),
/// dealerdirect/phpcodesniffer-composer-installer (CodeSniffer.conf).
pub const BENIGN_PLUGINS: &[&str] = &[
    "symfony/flex",
    "composer/package-versions-deprecated",
    "php-http/discovery",
    // Only listens to POST_CREATE_PROJECT_CMD / POST_INSTALL_CMD to print a
    // message (MessagePlugin::getSubscribedEvents): no disk effect.
    "drupal/core-project-message",
    // Only listens to POST_UPDATE_CMD / POST_CREATE_PROJECT_CMD, and only acts
    // in a `require` context (Plugin::getSubscribedEvents): inert at install.
    "drupal/core-recipe-unpack",
    // `Thanks::activate` arms its reminder only when the command is
    // `update`; `POST_PACKAGE_UPDATE` then `POST_UPDATE_CMD` print it after
    // a GitHub GraphQL call. On `install`: two commands added, nothing
    // else (read at v1.4.1).
    "symfony/thanks",
    // A `CommandProvider` (`composer normalize`) with an empty `activate`
    // and no listener (NormalizePlugin, read at 2.45.0).
    "ergebnis/composer-normalize",
];

/// Plugins known to change the install layout or the package contents:
/// always out of scope.
pub const LAYOUT_PLUGINS: &[&str] = &[
    "cweagans/composer-patches",
    "oomphinc/composer-installers-extender",
    "mnsami/composer-custom-directory-installer",
];

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
    "composer/package-versions-deprecated",
    "ergebnis/composer-normalize",
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
            } else if with_install {
                "applies recipes to the packages it installs (not emulated; --no-install resolves natively)".to_owned()
            } else {
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

/// The plugins Composer would load for a resolution command, and what
/// each does to it: those installed (installed.json of the project and
/// of `COMPOSER_HOME`) that `allow-plugins` allows. Empty under
/// `--no-plugins`. One issue per plugin that is not inert for `command`.
pub fn resolution_issues(
    project_dir: &Path,
    root_manifest: &Value,
    command: ResolutionCommand,
    plugins_enabled: bool,
    with_install: bool,
) -> Vec<ScopeIssue> {
    let mut issues = Vec::new();
    for name in active_plugins(project_dir, root_manifest, plugins_enabled) {
        if let Some(effect) = resolution_effect(&name, command, with_install) {
            issues.push(ScopeIssue::ResolutionPlugin(name, effect));
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
    /// framework with custom logic, refused target...).
    Layout(String),
    /// Package without a usable zip dist (source-only, exotic dist).
    NoUsableDist(String),
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
        classify_package(project_dir, p, plugins_enabled, &mut report);
    }
    report.issues.extend(
        config_issues(root_manifest)
            .into_iter()
            .map(ScopeIssue::Config),
    );
    match Layout::resolve(project_dir, lock, root_manifest, with_dev, plugins_enabled) {
        Ok(layout) => report.layout = Some(layout),
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
/// does not honour: a `preferred-install` asking for `source` anywhere.
/// (`vendor-dir` / `bin-dir` are resolved by `dirs::Dirs`; the forms it
/// refuses come back as layout issues.)
pub fn config_issues(root_manifest: &Value) -> Vec<String> {
    let value = |key: &str| -> Option<Value> {
        root_manifest
            .get("config")
            .and_then(|c| c.get(key))
            .cloned()
            .or_else(|| crate::layout::global_config_value(key))
    };
    let mut out = Vec::new();
    if let Some(v) = value("preferred-install") {
        let wants_source = match &v {
            Value::String(s) => s == "source",
            Value::Object(m) => m.values().any(|x| x.as_str() == Some("source")),
            _ => false,
        };
        if wants_source {
            out.push(format!("preferred-install {v}"));
        }
    }
    out
}

fn classify_package(
    project_dir: &Path,
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
    let usable = match p.dist_kind() {
        DistKind::Zip | DistKind::Tar => true,
        // A `path` package is laid out natively (symlink or mirror) on
        // Linux/macOS when its source directory is there; Windows
        // (junctions) is left to Composer.
        DistKind::Path => cfg!(unix) && p.dist_url().is_some_and(|u| project_dir.join(u).is_dir()),
        DistKind::Other | DistKind::Missing => false,
    };
    if !usable {
        report
            .issues
            .push(ScopeIssue::NoUsableDist(p.name().to_owned()));
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
        assert_eq!(r.skipped_plugins, vec!["symfony/flex"]);
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
             "source": {"type": "git", "url": "https://g/x.git", "reference": "r"}},
            {"name": "a/meta", "version": "1.0.0", "type": "metapackage"},
            zip_pkg("composer/installers", "composer-plugin"),
        ]));
        // installer-paths alone is inert (as in Composer); the plugin without
        // allow-plugins, however, blocks.
        let manifest = json!({"extra": {"installer-paths": {"web/modules/{$name}": []}}});
        let r = analyze(&proj(), &lock, &manifest, true, true);
        assert_eq!(r.issues.len(), 2, "{:?}", r.issues);
        assert_eq!(r.issues[0], ScopeIssue::NoUsableDist("a/src-only".into()));
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
            vec![ScopeIssue::NoUsableDist("a/gone".into())]
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
        let lock = lock_with(json!([zip_pkg("a/b", "library")]));
        let manifest = json!({"config": {"vendor-dir": "lib/", "bin-dir": "vendor/bin",
            "preferred-install": {"acme/*": "source", "*": "dist"}}});
        let r = analyze(&proj(), &lock, &manifest, true, true);
        assert_eq!(
            r.issues,
            vec![ScopeIssue::Config(
                "preferred-install {\"acme/*\":\"source\",\"*\":\"dist\"}".into()
            ),]
        );
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
}
