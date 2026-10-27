//! `allow-plugins` when Composer loads the installed plugins, on every
//! command that creates a Composer with plugins enabled
//! (`Factory::createComposer` → `PluginManager::loadInstalledPlugins`,
//! Factory.php:448, before anything the command prints).
//!
//! `loadRepository` (PluginManager.php:495-539) walks the project's
//! installed.json as written — a plugin whose directory is gone is judged
//! too — sorted like `PackageSorter`, then the global one
//! (`COMPOSER_HOME`, only when its composer.json exists). For each plugin:
//!
//! - a project plugin outside the set the root requires
//!   (`RepositoryUtils::filterRequiredPackages`, require-dev included) and
//!   not allowed (`isPluginAllowed($name, false, true, false)`) is not
//!   loaded, with a warning;
//! - otherwise `registerPackage` asks `isPluginAllowed` again: allowed, it
//!   loads; refused by a rule, `composer/package-versions-deprecated` or
//!   `extra.plugin-optional`, it is skipped (a debug line); no rule,
//!   `PluginBlockedException` — the command stops, exit 1, nothing written.
//!
//! Rules are null — `RuntimeException` at the first plugin judged — when
//! the merged `allow-plugins` is `[]` (absent or `{}`), the lock exists and
//! its `plugin-api-version` (default `1.1.0`) is below 2.2.0
//! (`parseAllowedPlugins`, PluginManager.php:690-711). The global plugins
//! use the global Composer's rules (config.json and its composer.json's
//! `config`), never null.
//!
//! Composer is not interactive here: vivacity never prompts.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use vivacity_core::layout::{plugin_verdict, PluginVerdict};

const PACKAGE_VERSIONS: &str = "composer/package-versions-deprecated";

const OLD_LOCK: &str = "Your composer.lock was generated before the allow-plugins security feature was introduced and your composer.json does not define allow-plugins. Run \"composer update --lock\" locally and commit the updated composer.lock, then add an explicit allow-plugins section to composer.json. See https://getcomposer.org/allow-plugins";

/// What loading the installed plugins prints: the warnings, then the
/// refusal that stops the command, if any.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Load {
    pub(crate) warnings: Vec<String>,
    pub(crate) refusal: Option<String>,
    /// The plugin the refusal is about.
    pub(crate) refused: Option<String>,
    /// The project's plugins registered before the refusal, in order.
    pub(crate) loaded: Vec<String>,
    /// A plugin `registerPackage` checks before judging it (its
    /// `composer-plugin-api` requirement, symfony/flex < 1.9.8) while it is
    /// not allowed: Composer skips it or throws another exception — not
    /// emulated, the command goes to Composer.
    pub(crate) hand_over: Option<String>,
}

/// The parsed rules of one `PluginManager` side: `None` is the null rules
/// of an old lock.
pub(crate) type Rules = Option<Value>;

/// `isPluginAllowed($name, $global, $optional)`, non-interactive: allowed,
/// or skipped, or the exception's message.
fn is_allowed(rules: &Rules, name: &str, global: bool, optional: bool) -> Result<bool, String> {
    let Some(rules) = rules else {
        return Err(OLD_LOCK.to_owned());
    };
    match plugin_verdict(Some(rules), name) {
        PluginVerdict::Allowed => Ok(true),
        PluginVerdict::Blocked => Ok(false),
        PluginVerdict::Unlisted if name == PACKAGE_VERSIONS || optional => Ok(false),
        PluginVerdict::Unlisted => Err(blocked_message(name, global)),
    }
}

/// `PluginBlockedException`'s message (PluginManager.php:819-823).
fn blocked_message(name: &str, global: bool) -> String {
    let (installed, command) = if global {
        (" (installed globally)", "global ")
    } else {
        ("", "")
    };
    format!(
        "{name}{installed} contains a Composer plugin which is blocked by your allow-plugins config. You may add it to the list if you consider it safe.\n\
         You can run \"composer {command}config --no-plugins allow-plugins.{name} [true|false]\" to enable it (true) or disable it explicitly and suppress this exception (false)\n\
         See https://getcomposer.org/allow-plugins"
    )
}

/// `[] === $allowPluginsConfig`: the default `[]`, an empty object or list.
fn is_empty_config(v: Option<&Value>) -> bool {
    match v {
        None => true,
        Some(Value::Object(m)) => m.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(_) => false,
    }
}

/// `Locker::isLocked() && version_compare(getPluginApi(), '2.2.0', '<')`.
fn lock_predates_allow_plugins(lock_path: &Path) -> bool {
    let Some(lock) = vivacity_core::jsonfile::read(lock_path) else {
        return false;
    };
    if lock.get("packages").is_none_or(Value::is_null) {
        return false;
    }
    let api = lock
        .get("plugin-api-version")
        .and_then(Value::as_str)
        .unwrap_or("1.1.0");
    vivacity_resolver::phpver::version_compare(api, "2.2.0") == std::cmp::Ordering::Less
}

/// What the global configuration brings to a load.
#[derive(Default)]
pub(crate) struct Globals {
    /// `config.allow-plugins` of COMPOSER_HOME/config.json, merged under
    /// the project's.
    pub(crate) allow_plugins: Option<Value>,
    /// The global Composer, when COMPOSER_HOME/composer.json exists: its
    /// vendor/composer and its rules.
    pub(crate) composer: Option<(std::path::PathBuf, Value)>,
}

impl Globals {
    /// The global configuration of this run (`COMPOSER_HOME`).
    pub(crate) fn current() -> Globals {
        Globals {
            allow_plugins: vivacity_core::layout::global_allow_plugins(),
            composer: crate::plugin_order::global_composer_dir().map(|dir| {
                (
                    dir,
                    vivacity_core::layout::global_composer_allow_plugins()
                        .unwrap_or_else(|| Value::Array(Vec::new())),
                )
            }),
        }
    }
}

/// `parseAllowedPlugins` for the project: its `allow-plugins` merged with
/// the global config; null when that is `[]` and the lock predates 2.2.0
/// (`lock_path` `None`: `config.lock` off, the lock is /dev/null).
fn project_rules(manifest: &Value, globals: &Globals, lock_path: Option<&Path>) -> Rules {
    let merged = vivacity_core::layout::merged_allow_plugins(
        manifest.get("config").and_then(|c| c.get("allow-plugins")),
        globals.allow_plugins.as_ref(),
    );
    if is_empty_config(merged.as_ref()) && lock_path.is_some_and(lock_predates_allow_plugins) {
        return None;
    }
    Some(merged.unwrap_or_else(|| Value::Array(Vec::new())))
}

/// The names a package answers to (`getNames()`: name, provide, replace),
/// lowercased, and the names it requires.
fn names_and_requires(p: &Value) -> (Vec<String>, Vec<String>) {
    let keys = |field: &str| -> Vec<String> {
        p.get(field)
            .and_then(Value::as_object)
            .map(|m| m.keys().map(|k| k.to_ascii_lowercase()).collect())
            .unwrap_or_default()
    };
    let mut names: Vec<String> = p
        .get("name")
        .and_then(Value::as_str)
        .map(|n| vec![n.to_ascii_lowercase()])
        .unwrap_or_default();
    names.extend(keys("provide"));
    names.extend(keys("replace"));
    (names, keys("require"))
}

/// `RepositoryUtils::filterRequiredPackages($packages, $root, true)`: the
/// installed packages the root requires (require and require-dev),
/// recursively through each one's `require`. By name.
fn required_names(installed: &[Value], manifest: &Value) -> BTreeSet<String> {
    let mut wanted: BTreeSet<String> = ["require", "require-dev"]
        .iter()
        .filter_map(|f| manifest.get(*f).and_then(Value::as_object))
        .flat_map(|m| m.keys().map(|k| k.to_ascii_lowercase()))
        .collect();
    let packages: Vec<(Vec<String>, Vec<String>)> =
        installed.iter().map(names_and_requires).collect();
    let mut taken = vec![false; packages.len()];
    let mut out: BTreeSet<String> = BTreeSet::new();
    loop {
        let mut grew = false;
        for (i, (names, requires)) in packages.iter().enumerate() {
            if !taken[i] && names.iter().any(|n| wanted.contains(n)) {
                taken[i] = true;
                grew = true;
                out.extend(names.first().cloned());
                wanted.extend(requires.iter().cloned());
            }
        }
        if !grew {
            return out;
        }
    }
}

/// `registerPackage`'s checks before `isPluginAllowed`
/// (PluginManager.php:177-205, `PLUGIN_API_VERSION` 2.9.0): a
/// `composer-plugin` without a `composer-plugin-api` requirement throws,
/// one whose requirement is the bare version or does not accept 2.9.0
/// warns, symfony/flex below 1.9.8 is skipped.
fn checked_before_judgement(p: &crate::plugin_order::InstalledPlugin) -> bool {
    if !p.composer_plugin {
        return false;
    }
    let Some(api) = &p.plugin_api else {
        return true;
    };
    let accepts = vivacity_resolver::constraint::parse_constraints(api)
        .is_ok_and(|c| c.constraint.matches_version("2.9.0.0") && c.pretty != PLUGIN_API_VERSION);
    // `getVersion()`: the normalized version (`1.9.7.0` for `v1.9.7`).
    let old_flex = p.name == "symfony/flex"
        && p.version.chars().all(|c| c.is_ascii_digit() || c == '.')
        && vivacity_resolver::phpver::version_compare(&p.version, "1.9.8")
            == std::cmp::Ordering::Less;
    !accepts || old_flex
}

const PLUGIN_API_VERSION: &str = "2.9.0";

/// `loadInstalledPlugins` for a project: `composer_dir` is its
/// vendor/composer, `lock_path` its lock (`None` when `config.lock` is
/// off).
pub(crate) fn load(
    manifest: &Value,
    composer_dir: &Path,
    lock_path: Option<&Path>,
    globals: &Globals,
) -> Load {
    let mut out = Load::default();
    let rules = project_rules(manifest, globals, lock_path);
    let installed = crate::plugin_order::installed_json_packages(composer_dir);
    let required = required_names(&installed, manifest);
    let mut judged: Vec<(crate::plugin_order::InstalledPlugin, bool)> = Vec::new();
    for p in crate::plugin_order::sorted_plugins(&installed, composer_dir) {
        judged.push((p, false));
    }
    if let Some((global_dir, _)) = &globals.composer {
        let installed = crate::plugin_order::installed_json_packages(global_dir);
        for p in crate::plugin_order::sorted_plugins(&installed, global_dir) {
            judged.push((p, true));
        }
    }
    let global_rules: Rules = globals.composer.as_ref().map(|(_, r)| r.clone());
    for (p, global) in &judged {
        let rules = if *global { &global_rules } else { &rules };
        if !global && !required.contains(&p.name) {
            match is_allowed(rules, &p.name, false, true) {
                Ok(true) => {}
                Ok(false) => {
                    out.warnings.push(format!(
                        "The \"{}\" plugin was not loaded as it is not listed in allow-plugins and is not required by the root package anymore.",
                        p.name
                    ));
                    continue;
                }
                Err(e) => {
                    out.refusal = Some(e);
                    out.refused = Some(p.name.clone());
                    break;
                }
            }
        }
        let verdict = is_allowed(rules, &p.name, *global, p.optional);
        if verdict != Ok(true) && checked_before_judgement(p) {
            out.hand_over = Some(p.name.clone());
            return out;
        }
        match verdict {
            Err(e) => {
                out.refusal = Some(e);
                out.refused = Some(p.name.clone());
                break;
            }
            Ok(true) if !global && p.present => out.loaded.push(p.name.clone()),
            Ok(_) => {}
        }
    }
    out
}

/// `load` with the project's vendor/composer and lock.
pub(crate) fn load_for(project: &Path, manifest: &Value) -> Load {
    load(
        manifest,
        &composer_dir(project, manifest),
        lock_path(project, manifest).as_deref(),
        &Globals::current(),
    )
}

fn composer_dir(project: &Path, manifest: &Value) -> std::path::PathBuf {
    match vivacity_core::dirs::Dirs::resolve(manifest) {
        Ok(d) => d.composer_dir(project),
        Err(_) => project.join("vendor").join("composer"),
    }
}

fn lock_path(project: &Path, manifest: &Value) -> Option<std::path::PathBuf> {
    crate::config_lock_enabled(&manifest.to_string()).then(|| project.join("composer.lock"))
}

/// The hand-over of a plugin judged before its `allow-plugins` verdict.
pub(crate) fn hand_over_issue(name: &str) -> vivacity_core::scope::ScopeIssue {
    vivacity_core::scope::ScopeIssue::ResolutionPlugin(
        name.to_owned(),
        "is not allowed by config.allow-plugins and Composer checks its plugin API (or its version) before judging it (not emulated)".to_owned(),
    )
}

/// Root without `COMPOSER_ALLOW_SUPERUSER`, not interactive: Composer
/// disables the plugins (Application.php:232-320, outside Docker) — a
/// condition vivacity does not reproduce, so a refusal there goes to
/// Composer.
fn root_without_superuser() -> bool {
    let allowed =
        std::env::var("COMPOSER_ALLOW_SUPERUSER").is_ok_and(|v| !v.is_empty() && v != "0");
    #[cfg(unix)]
    let root = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
    };
    #[cfg(not(unix))]
    let root = false;
    root && !allowed
}

/// The hand-over of a judgement Composer skips as root.
pub(crate) fn root_issue() -> Option<vivacity_core::scope::ScopeIssue> {
    root_without_superuser().then(|| {
        vivacity_core::scope::ScopeIssue::Layout(
            "running as root without COMPOSER_ALLOW_SUPERUSER, where Composer disables the plugins and would not judge them (not emulated)".to_owned(),
        )
    })
}

/// The first `Composer` of a command (`BaseCommand::initialize`, then the
/// command's own `requireComposer`): `Some(code)` when the command stops
/// here — refused (exit 1) or handed to Composer. Otherwise the warnings
/// are held: written before the command's first line, dropped when the
/// command goes to Composer later (which writes them itself).
///
/// `edited`: the manifest `require` writes before its second load, which
/// may refuse where the first did not ([`second_load_issue`]): handed over
/// before anything is printed.
pub(crate) fn at_load(
    project: &Path,
    manifest: &Value,
    edited: Option<&Value>,
    plugins_enabled: bool,
    no_fallback: bool,
) -> anyhow::Result<Option<i32>> {
    if !plugins_enabled {
        return Ok(None);
    }
    let load = load_for(project, manifest);
    let issue = match (&load.hand_over, edited) {
        (Some(name), _) => Some(hand_over_issue(name)),
        (None, Some(edited)) if load.refusal.is_none() => second_load_issue(project, edited),
        _ => None,
    };
    let issue = match issue {
        Some(i) => Some(root_issue().unwrap_or(i)),
        None if load.refusal.is_some() => root_issue(),
        None => None,
    };
    if let Some(issue) = issue {
        return crate::delegate_resolution(&[issue], no_fallback).map(Some);
    }
    if load.refusal.is_some() {
        return refuse(project, manifest, &load, 2).map(Some);
    }
    hold_warnings(&load);
    Ok(None)
}

/// Holds the warnings of a load that went through, right after the lines
/// of the `Factory` run that printed them.
pub(crate) fn hold_warnings(load: &Load) {
    vivacity_core::io::hold_after_factory(vivacity_core::io::Verbosity::Normal, &load.warnings);
}

/// yiisoft/yii2-composer registered before a refusal: its `activate` wrote
/// extensions.php all the same.
pub(crate) fn activate_before_refusal(
    project: &Path,
    manifest: &Value,
    load: &Load,
) -> anyhow::Result<()> {
    if load
        .loaded
        .iter()
        .any(|n| n == vivacity_core::yii2_composer::PLUGIN_NAME)
    {
        crate::no_install::activate_yii2_composer(project, manifest, true)?;
    }
    Ok(())
}

/// A refused load, as Composer prints it: every `Factory` run that fails
/// is run again — `initialize` (swallowed), then what the command held
/// since (the platform-requirement environment, its first lines), the
/// command's own (`requireComposer`, which throws): the `Factory` lines
/// and the warnings before the refusal `attempts` times; then the GitHub
/// Actions annotation, `hintCommonErrors`'s run with plugins off (the
/// `Factory` lines once more), and the message, without Symfony Console's
/// box (as `refuse_invalid_root`). Exit 1; nothing written but what a
/// plugin registered before the refusal writes on activation.
pub(crate) fn refuse(
    project: &Path,
    manifest: &Value,
    load: &Load,
    attempts: usize,
) -> anyhow::Result<i32> {
    let Some(refusal) = &load.refusal else {
        return Ok(0);
    };
    activate_before_refusal(project, manifest, load)?;
    let factory = vivacity_core::io::factory_held();
    hold_warnings(load);
    vivacity_core::io::release_held();
    for _ in 1..attempts {
        for line in factory.iter().chain(&load.warnings) {
            vivacity_core::say!("{line}");
        }
    }
    vivacity_core::io::github_action_error(refusal);
    for line in &factory {
        vivacity_core::say!("{line}");
    }
    vivacity_core::say_always!("{refusal}");
    Ok(1)
}

/// `PluginInstaller::prepare` (PluginInstaller.php:55-62): a plugin the
/// install phase installs or updates is judged again; not allowed and not
/// skipped, Composer stops in the middle of the installation. `plugins`
/// are those operations' targets (name, `extra.plugin-optional`), judged
/// with the rules the run loaded (`load_manifest`, the lock as it was).
pub(crate) fn install_issue<'a>(
    project: &Path,
    load_manifest: &Value,
    plugins: impl IntoIterator<Item = (&'a str, bool)>,
) -> Option<vivacity_core::scope::ScopeIssue> {
    let issue = install_issue_with(project, load_manifest, &Globals::current(), plugins)?;
    Some(root_issue().unwrap_or(issue))
}

fn install_issue_with<'a>(
    project: &Path,
    load_manifest: &Value,
    globals: &Globals,
    plugins: impl IntoIterator<Item = (&'a str, bool)>,
) -> Option<vivacity_core::scope::ScopeIssue> {
    let rules = project_rules(
        load_manifest,
        globals,
        lock_path(project, load_manifest).as_deref(),
    );
    for (name, optional) in plugins {
        if let Err(e) = is_allowed(&rules, name, false, optional) {
            let why = if e == OLD_LOCK {
                "would be installed under a lock that predates config.allow-plugins (Composer stops in the middle of the installation; not emulated)"
            } else {
                "is not covered by config.allow-plugins and this run installs it (Composer stops in the middle of the installation; not emulated)"
            };
            return Some(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
                name.to_owned(),
                why.to_owned(),
            ));
        }
    }
    None
}

/// The plugins of a solved lock the install phase installs or updates:
/// those the local repository (installed.json, purged) does not hold as
/// locked.
pub(crate) fn installed_by(
    project: &Path,
    manifest: &Value,
    lock: &vivacity_core::lock::Lock,
    with_dev: bool,
) -> Vec<(String, bool)> {
    let installed = crate::installed_packages(project, manifest);
    lock.wanted_packages(with_dev)
        .filter(|p| {
            matches!(
                p.raw.get("type").and_then(Value::as_str),
                Some("composer-plugin" | "composer-installer")
            )
        })
        .filter(|p| !crate::installed_as_locked(&installed, p))
        .map(|p| (p.name().to_ascii_lowercase(), optional(&p.raw)))
        .collect()
}

/// `extra.plugin-optional` of a lock or installed.json entry.
pub(crate) fn optional(entry: &serde_json::Map<String, Value>) -> bool {
    entry.get("extra").and_then(|e| e.get("plugin-optional")) == Some(&Value::Bool(true))
}

/// The second load of `require`, on the edited manifest, would stop the
/// command where the first did not (the package required makes an
/// installed, uncovered plugin required): Composer then reverts
/// composer.json — not emulated, the command goes to Composer.
pub(crate) fn second_load_issue(
    project: &Path,
    edited: &Value,
) -> Option<vivacity_core::scope::ScopeIssue> {
    let load = load_for(project, edited);
    if let Some(name) = &load.hand_over {
        return Some(hand_over_issue(name));
    }
    load.refused.as_ref().map(|name| {
        vivacity_core::scope::ScopeIssue::ResolutionPlugin(
            name.clone(),
            "is not covered by config.allow-plugins and becomes required by this command (Composer refuses it after editing composer.json; not emulated)".to_owned(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(installed: Value, lock: Option<Value>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        std::fs::create_dir_all(&composer).unwrap();
        std::fs::write(
            composer.join("installed.json"),
            json!({ "packages": installed }).to_string(),
        )
        .unwrap();
        if let Some(lock) = lock {
            std::fs::write(dir.path().join("composer.lock"), lock.to_string()).unwrap();
        }
        dir
    }

    fn plugin(name: &str) -> Value {
        json!({"name": name, "version": "1.0.0", "type": "composer-plugin",
               "require": {"composer-plugin-api": "^2.0"}})
    }

    /// The load, `loaded` set aside (asserted where it matters).
    fn run(dir: &Path, manifest: Value) -> Load {
        let mut l = load(
            &manifest,
            &dir.join("vendor/composer"),
            Some(&dir.join("composer.lock")),
            &Globals::default(),
        );
        l.loaded.clear();
        l
    }

    #[test]
    fn a_required_unlisted_plugin_stops_the_command() {
        let d = project(json!([plugin("a/p")]), None);
        let l = run(d.path(), json!({"require": {"a/p": "*"}}));
        assert!(l.warnings.is_empty());
        assert_eq!(
            l.refusal.unwrap().lines().next(),
            Some("a/p contains a Composer plugin which is blocked by your allow-plugins config. You may add it to the list if you consider it safe.")
        );
    }

    #[test]
    fn an_explicit_rule_decides() {
        let d = project(json!([plugin("a/p")]), None);
        for allow in [
            json!({"a/*": true}),
            json!({"a/p": false}),
            json!(true),
            json!(false),
        ] {
            let l = run(
                d.path(),
                json!({"require": {"a/p": "*"}, "config": {"allow-plugins": allow}}),
            );
            assert_eq!(l, Load::default(), "{allow}");
        }
    }

    #[test]
    fn a_plugin_no_longer_required_is_skipped_with_a_warning() {
        let d = project(json!([plugin("a/p")]), None);
        for allow in [json!({}), json!({"a/p": false})] {
            let l = run(d.path(), json!({"config": {"allow-plugins": allow}}));
            assert_eq!(l.warnings.len(), 1);
            assert!(l.refusal.is_none());
        }
        let l = run(
            d.path(),
            json!({"config": {"allow-plugins": {"a/p": true}}}),
        );
        assert_eq!(l, Load::default());
    }

    #[test]
    fn required_through_a_package_replace_or_provide() {
        let mut lib = json!({"name": "b/lib", "version": "1.0.0", "require": {"a/p": "*"},
                             "replace": {"c/old": "*"}});
        let d = project(json!([plugin("a/p"), lib.clone()]), None);
        assert!(run(d.path(), json!({"require": {"c/old": "*"}}))
            .refusal
            .is_some());
        lib["require"] = json!({});
        let d = project(json!([plugin("a/p"), lib]), None);
        let l = run(d.path(), json!({"require": {"c/old": "*"}}));
        assert_eq!(l.warnings.len(), 1);
        assert!(l.refusal.is_none());
    }

    #[test]
    fn optional_and_package_versions_are_skipped_silently() {
        let mut p = plugin("a/p");
        p["extra"] = json!({"plugin-optional": true});
        let d = project(json!([p, plugin(PACKAGE_VERSIONS)]), None);
        let l = run(
            d.path(),
            json!({"require": {"a/p": "*", PACKAGE_VERSIONS: "*"}}),
        );
        assert_eq!(l, Load::default());
    }

    #[test]
    fn an_old_lock_without_rules_stops_at_the_first_plugin() {
        let lock = json!({"packages": [], "plugin-api-version": "2.1.0"});
        let d = project(json!([plugin(PACKAGE_VERSIONS)]), Some(lock));
        for m in [
            json!({"require": {PACKAGE_VERSIONS: "*"}}),
            json!({"config": {"allow-plugins": {}}}),
        ] {
            assert_eq!(run(d.path(), m).refusal.as_deref(), Some(OLD_LOCK));
        }
        // A rule, or a lock written since 2.2: the usual judgement.
        let l = run(
            d.path(),
            json!({"config": {"allow-plugins": {"x/y": true}}}),
        );
        assert_eq!(l.warnings.len(), 1);
        let d = project(
            json!([plugin(PACKAGE_VERSIONS)]),
            Some(json!({"packages": [], "plugin-api-version": "2.2.0"})),
        );
        assert_eq!(
            run(d.path(), json!({"require": {PACKAGE_VERSIONS: "*"}})),
            Load::default()
        );
        // No lock key at all: 1.1.0.
        let d = project(
            json!([plugin(PACKAGE_VERSIONS)]),
            Some(json!({"packages": []})),
        );
        assert!(run(d.path(), json!({})).refusal.is_some());
    }

    #[test]
    fn the_first_refusal_in_sorted_order_wins() {
        // b/q requires a/p: a/p sorts first (fewer dependents last).
        let mut q = plugin("b/q");
        q["require"]["a/p"] = json!("*");
        let d = project(json!([q, plugin("a/p")]), None);
        let l = run(d.path(), json!({"require": {"b/q": "*"}}));
        assert!(l.refusal.unwrap().starts_with("a/p contains"));
    }

    #[test]
    fn a_plugin_composer_checks_first_goes_to_composer() {
        let mut no_api = plugin("a/p");
        no_api["require"] = json!({});
        let mut old_api = plugin("b/q");
        old_api["require"] = json!({"composer-plugin-api": "^1.0"});
        let mut bare_api = plugin("c/r");
        bare_api["require"] = json!({"composer-plugin-api": "2.9.0"});
        for p in [no_api, old_api, bare_api] {
            let name = p["name"].as_str().unwrap().to_owned();
            let d = project(json!([p]), None);
            let l = run(d.path(), json!({"require": {name.clone(): "*"}}));
            assert_eq!(l.hand_over.as_deref(), Some(name.as_str()));
            // Allowed: judged as before (not this module's concern).
            let l = run(
                d.path(),
                json!({"require": {name.clone(): "*"}, "config": {"allow-plugins": {name: true}}}),
            );
            assert_eq!(l, Load::default());
        }
        // A composer-installer has no API check.
        let mut installer = plugin("d/s");
        installer["type"] = json!("composer-installer");
        installer["require"] = json!({});
        let d = project(json!([installer]), None);
        let l = run(d.path(), json!({"require": {"d/s": "*"}}));
        assert!(l.hand_over.is_none() && l.refusal.is_some());
    }

    #[test]
    fn a_gone_plugin_is_warned_about_before_any_api_check() {
        let mut p = plugin("a/p");
        p["require"] = json!({});
        let d = project(json!([p]), None);
        let l = run(d.path(), json!({}));
        assert_eq!(l.warnings.len(), 1);
        assert!(l.hand_over.is_none() && l.refusal.is_none());
    }

    #[test]
    fn the_install_phase_judges_what_it_installs() {
        let d = project(json!([]), None);
        let m = json!({"config": {"allow-plugins": {"a/ok": true}}});
        assert!(install_issue(
            d.path(),
            &m,
            [("a/ok", false), ("b/opt", true), (PACKAGE_VERSIONS, false)]
        )
        .is_none());
        assert!(
            install_issue_with(d.path(), &m, &Globals::default(), [("c/blocked", false)])
                .is_some_and(|i| i.to_string().starts_with("plugin c/blocked is not covered"))
        );
        // An old lock: even the optional ones stop it.
        let d = project(
            json!([]),
            Some(json!({"packages": [], "plugin-api-version": "2.1.0"})),
        );
        assert!(
            install_issue_with(d.path(), &json!({}), &Globals::default(), [("b/opt", true)])
                .is_some()
        );
        assert!(install_issue_with(d.path(), &m, &Globals::default(), [("b/opt", true)]).is_none());
    }

    #[test]
    fn an_old_flex_is_skipped_by_composer_on_its_normalized_version() {
        let mut flex = plugin("symfony/flex");
        flex["version"] = json!("v1.9.7");
        flex["require"] = json!({"composer-plugin-api": "^1.0|^2.0"});
        let d = project(json!([flex.clone()]), None);
        let l = run(d.path(), json!({"require": {"symfony/flex": "*"}}));
        assert_eq!(l.hand_over.as_deref(), Some("symfony/flex"));
        flex["version"] = json!("v1.9.8");
        let d = project(json!([flex]), None);
        let l = run(d.path(), json!({"require": {"symfony/flex": "*"}}));
        assert!(l.hand_over.is_none() && l.refusal.is_some());
    }

    #[test]
    fn a_global_plugin_is_judged_by_the_global_rules() {
        let d = project(json!([]), None);
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("installed.json"),
            json!({"packages": [plugin("g/p")]}).to_string(),
        )
        .unwrap();
        let globals = |rules: Value| Globals {
            allow_plugins: None,
            composer: Some((home.path().to_path_buf(), rules)),
        };
        // The project's rules do not count for it.
        let m = json!({"config": {"allow-plugins": {"g/p": true}}});
        let l = load(
            &m,
            &d.path().join("vendor/composer"),
            None,
            &globals(json!([])),
        );
        assert!(l
            .refusal
            .unwrap()
            .starts_with("g/p (installed globally) contains"));
        let l = load(
            &m,
            &d.path().join("vendor/composer"),
            None,
            &globals(json!({"g/*": true})),
        );
        assert!(l.refusal.is_none() && l.warnings.is_empty() && l.loaded.is_empty());
        // config.json's rules merge under the project's.
        let globals = Globals {
            allow_plugins: Some(json!({"a/p": true})),
            composer: None,
        };
        let d = project(json!([plugin("a/p")]), None);
        let l = load(
            &json!({"require": {"a/p": "*"}}),
            &d.path().join("vendor/composer"),
            None,
            &globals,
        );
        assert_eq!(l.loaded, vec!["a/p".to_owned()]);
        assert!(l.refusal.is_none());
    }

    #[test]
    fn the_global_message() {
        assert!(blocked_message("a/p", true)
            .contains("a/p (installed globally) contains a Composer plugin which is blocked"));
        assert!(blocked_message("a/p", true)
            .contains("\"composer global config --no-plugins allow-plugins.a/p [true|false]\""));
    }
}
