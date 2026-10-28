//! Where each package of the lock gets installed: `vendor/<name>[/<target-dir>]`
//! by LibraryInstaller, or the path composer/installers gives it when that
//! plugin is locked, allowed (`config.allow-plugins`) and ported
//! (`installers::table_for`). A single pass, before touching the disk;
//! anything not reproducible byte for byte becomes an `issue` (falls back to
//! Composer).
//!
//! The path is relative to the project root and normalised (`normalizePath`,
//! no trailing slash); that is the form Composer normalises before computing
//! `install-path` (FilesystemRepository::write).

use crate::custom_installers::{self, Placer};
use crate::dirs::Dirs;
use crate::installers::{self, Placement};
use crate::lock::{Lock, LockPackage};
use crate::pathutil::{find_shortest_path, normalize_path};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Layout {
    /// Project root, absolute (as given, not canonicalised: the relative paths
    /// derived from it do not depend on symlinks).
    root: PathBuf,
    /// `config.vendor-dir` / `bin-dir`, project-relative.
    dirs: Dirs,
    /// name -> project-relative path (absent for a metapackage).
    paths: BTreeMap<String, String>,
    /// Emulated composer/installers tag, if the plugin is active.
    pub installers_tag: Option<String>,
    /// Installed packages (installed.json) to remove: name -> relative
    /// directory to delete (`vendor/<name>` or the plugin's target), after
    /// checking that Composer would recompute the same path today.
    removals: BTreeMap<String, String>,
    /// name -> `getPackageBasePath` (project-relative): the install path
    /// without its `target-dir`, what a re-layout empties first.
    bases: BTreeMap<String, String>,
    /// getkirby/composer-installer's `postInstall`: packages whose own
    /// `vendor/` is removed once laid out.
    strip_vendor: BTreeSet<String>,
    /// composer/installers' `Installer::uninstall` (and installers derived
    /// from it): name -> the stdout line written once the package is gone.
    removal_notes: BTreeMap<String, String>,
    /// A directory Composer leaves empty: `FileDownloader::download`
    /// creates the path the installer active at download time gives
    /// (LibraryInstaller's, the installer plugin being installed by the same
    /// transaction), and `cleanup` only removes that path and
    /// `<vendor>/<vendor name>` when empty — with a target-dir, the
    /// directories above it stay.
    download_leftovers: BTreeMap<String, String>,
}

/// `allow-plugins` verdict for a package, like PluginManager in
/// non-interactive mode.
#[derive(Debug, PartialEq, Eq)]
pub enum PluginVerdict {
    Allowed,
    /// Explicitly refused: Composer skips the plugin with a warning.
    Blocked,
    /// No rule covers it: Composer stops with an error.
    Unlisted,
}

fn absolutize(dir: &Path) -> PathBuf {
    if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    }
}

/// `BasePackage::packageNameToRegexp`: `{^<quote(pattern) with * -> .*>$}i`.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    let p = pattern.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == n;
    }
    let mut rest = n.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(pos) = rest.find(part) else {
                return false;
            };
            rest = &rest[pos + part.len()..];
        }
    }
    true
}

/// `Config::merge` for `allow-plugins`: the project value replaces the global
/// one unless both are objects, in which case
/// `array_merge($project, $global, $project)`: the project keys first, in
/// their order, then those only the global config brings. Order matters: the
/// first matching rule decides.
pub fn merged_allow_plugins(project: Option<&Value>, global: Option<&Value>) -> Option<Value> {
    match (project, global) {
        (Some(Value::Object(p)), Some(Value::Object(g))) => {
            let mut m = p.clone();
            for (k, v) in g {
                if !m.contains_key(k) {
                    m.insert(k.clone(), v.clone());
                }
            }
            Some(Value::Object(m))
        }
        (Some(p), _) => Some(p.clone()),
        (None, Some(g)) => Some(g.clone()),
        (None, None) => None,
    }
}

/// `PluginManager::parseAllowedPlugins` + `isPluginAllowed` (non-interactive).
pub fn plugin_verdict(allow: Option<&Value>, package: &str) -> PluginVerdict {
    match allow {
        Some(Value::Bool(true)) => PluginVerdict::Allowed,
        Some(Value::Bool(false)) => PluginVerdict::Blocked,
        Some(Value::Object(rules)) => {
            for (pattern, v) in rules {
                if pattern_matches(pattern, package) {
                    return if v == &Value::Bool(true) {
                        PluginVerdict::Allowed
                    } else {
                        PluginVerdict::Blocked
                    };
                }
            }
            PluginVerdict::Unlisted
        }
        _ => PluginVerdict::Unlisted,
    }
}

/// Verdict for `package` under the project config merged with the global one.
pub fn plugin_allowed(manifest: &Value, package: &str) -> PluginVerdict {
    let allow = merged_allow_plugins(
        manifest.get("config").and_then(|c| c.get("allow-plugins")),
        global_allow_plugins().as_ref(),
    );
    plugin_verdict(allow.as_ref(), package)
}

/// `config.allow-plugins` from COMPOSER_HOME/config.json.
pub fn global_allow_plugins() -> Option<Value> {
    global_config_value("allow-plugins")
}

/// `allow-plugins` of the global Composer (`Factory::createGlobalComposer`,
/// whose manifest is COMPOSER_HOME/composer.json): config.json's value
/// merged with that manifest's `config` (`Config::merge`), the rules its
/// plugins are judged by.
pub fn global_composer_allow_plugins() -> Option<Value> {
    let manifest = crate::jsonfile::read(&crate::fetch::composer_home()?.join("composer.json"));
    merged_allow_plugins(
        manifest
            .as_ref()
            .and_then(|m| m.get("config"))
            .and_then(|c| c.get("allow-plugins")),
        global_allow_plugins().as_ref(),
    )
}

/// The packages of the global Composer's installed.json (COMPOSER_HOME,
/// with its manifest's vendor-dir), when `Factory::createGlobalComposer`
/// builds one (COMPOSER_HOME/composer.json exists).
fn global_installed_packages() -> Vec<Value> {
    let Some(home) = crate::fetch::composer_home() else {
        return Vec::new();
    };
    let Some(manifest) = crate::jsonfile::read(&home.join("composer.json")) else {
        return Vec::new();
    };
    match Dirs::resolve(&manifest) {
        Ok(dirs) => installed_packages_at(&dirs.composer_dir(&home)),
        Err(_) => Vec::new(),
    }
}

/// One `config.<key>` value of COMPOSER_HOME/config.json — the global
/// layer of `Config::merge`, below the root composer.json.
pub fn global_config_value(key: &str) -> Option<Value> {
    let path = crate::fetch::composer_home()?.join("config.json");
    let v = crate::jsonfile::read(&path)?;
    v.get("config")?.get(key).cloned()
}

/// Project-relative path of a package handled by LibraryInstaller:
/// `<vendor-dir>/<name>[/<target-dir>]`.
fn vendor_rel(vendor: &str, name: &str, target_dir: Option<&str>) -> String {
    match target_dir {
        Some(t) => format!("{vendor}/{name}/{t}"),
        None => format!("{vendor}/{name}"),
    }
}

/// The active installers of a run: composer/installers' table, whether
/// roots/wordpress-core-installer places `wordpress-core`, mnsami's
/// installer, and the other custom installers (`custom_installers`).
#[derive(Default)]
struct Placers<'a> {
    table: Option<&'a installers::Table>,
    wp_core: bool,
    /// mnsami/composer-custom-directory-installer places `library` and
    /// `composer-plugin` packages named in `extra.installer-paths`.
    custom_dirs: bool,
    custom: Vec<Placer>,
    root_type: String,
    /// The installer plugins this run's transaction installs (absent from
    /// installed.json): not active yet when the packages are downloaded.
    fresh: BTreeSet<&'static str>,
}

impl Placers<'_> {
    fn any(&self) -> bool {
        self.table.is_some() || self.wp_core || self.custom_dirs || !self.custom.is_empty()
    }
}

/// Who placed a package, when that matters after the layout: the stdout
/// line of composer/installers' `uninstall`, kirby's `postInstall`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Who {
    /// composer/installers, 2.x (absolute path in its line) or 1.x.
    Installers {
        absolute: bool,
    },
    /// oomphinc: composer/installers' `uninstall`, its own relative path.
    Oomphinc,
    Kirby,
    /// johnpbloch/wordpress-core-installer: its `getInstallPath` refuses a
    /// directory another package took in the same process.
    Johnpbloch,
    Other,
}

impl Who {
    /// The installer returns a relative path: `BinaryInstaller` then
    /// `realpath`s each binary (BinaryInstaller.php:77-82), symlinks
    /// resolved. composer/installers 2.x prefixes `getcwd()`.
    fn relative(self) -> bool {
        !matches!(self, Who::Installers { absolute: true })
    }
}

/// A placement: the project-relative path (normalised), the path as the
/// winning installer built it, and the installer — `None` when several
/// installers take the type and agree on the path (which one Composer asks
/// follows the plugins' activation order). `whos` lists every installer
/// that takes the type, the winner among them.
struct Placed {
    rel: String,
    raw: Option<String>,
    who: Option<Who>,
    whos: Vec<Who>,
    /// Every installer taking the type is activated by this transaction:
    /// the package was downloaded to LibraryInstaller's path.
    all_fresh: bool,
}

impl Placed {
    /// Several installers take the type: the path they agree on is right,
    /// what each does besides (kirby's `postInstall`, composer/installers'
    /// `uninstall` line, johnpbloch's directory check) depends on which one
    /// Composer asks.
    fn contested(&self) -> bool {
        self.whos.len() > 1
    }
}

/// One installer taking a package's type: who, its label, its path as
/// built (`None`: LibraryInstaller's).
type Candidate<'a> = (Who, &'a str, Result<Option<String>, String>);

/// Decision for a package (its lock or installed.json entry) under the
/// current configuration: `InstallationManager::getInstaller` returns the
/// last installer added that `supports()` the type; vivacity does not
/// replay the activation order — when several active installers take the
/// type, their paths must agree.
fn place(
    vendor: &str,
    placers: &Placers<'_>,
    root_extra: Option<&Value>,
    pkg: &Map<String, Value>,
) -> Result<Placed, String> {
    let name = pkg.get("name").and_then(Value::as_str).unwrap_or_default();
    let package_type = pkg.get("type").and_then(Value::as_str).unwrap_or("library");
    let package_extra = pkg.get("extra");
    let target_dir = pkg
        .get("target-dir")
        .and_then(Value::as_str)
        .map(|t| t.trim_matches('/'))
        .filter(|t| !t.is_empty());
    let default = vendor_rel(vendor, name, target_dir);
    let mut cands: Vec<Candidate<'_>> = Vec::new();
    let mut plugins: Vec<&'static str> = Vec::new();
    if placers.custom_dirs && matches!(package_type, "library" | "composer-plugin") {
        cands.push((
            Who::Other,
            "custom-directory-installer",
            Ok(custom_dir(root_extra, name, package_extra)),
        ));
        plugins.push(CUSTOM_DIRS_PLUGIN);
    }
    // roots/wordpress-core-installer's `getInstallPath` for its type.
    if placers.wp_core && package_type == WP_CORE_TYPE {
        cands.push((
            Who::Other,
            "wordpress-core-installer",
            wp_core_dir(root_extra, name, package_extra, vendor)
                .map(Some)
                .map_err(|e| format!("wordpress-core-installer: {e}")),
        ));
        plugins.push(WP_CORE_PLUGIN);
    }
    if let Some(table) = placers.table {
        if installers::supports_type(table, root_extra, package_type) {
            cands.push((
                Who::Installers {
                    absolute: table.absolute_paths(),
                },
                "installers",
                match installers::placement(table, root_extra, name, package_type, package_extra) {
                    Ok(Placement::Vendor) => Ok(None),
                    Ok(Placement::Custom(p)) => Ok(Some(p)),
                    Err(e) => Err(format!("installers: {name} ({package_type}): {e}")),
                },
            ));
            plugins.push(INSTALLERS_PLUGIN);
        }
    }
    let ctx = custom_installers::Ctx {
        root_extra,
        root_type: &placers.root_type,
        vendor,
    };
    for c in &placers.custom {
        if c.supports(package_type) {
            let who = match c {
                Placer::Oomphinc(..) => Who::Oomphinc,
                Placer::Kirby => Who::Kirby,
                Placer::Johnpbloch => Who::Johnpbloch,
                _ => Who::Other,
            };
            cands.push((who, c.plugin(), c.install_path(&ctx, pkg)));
            plugins.push(c.plugin());
        }
    }
    let mut resolved: Vec<(Who, String, Option<String>)> = Vec::new();
    for (who, label, path) in cands {
        let (rel, raw) = match path? {
            None => (default.clone(), None),
            Some(p) => {
                let rel = custom_target(vendor, name, &p, label)?;
                // `AutoloadGenerator::parseAutoloads` cuts `strlen('/' .
                // targetDir)` characters off the install path, and
                // `getPackageBasePath` a target-dir suffix: right only for an
                // installer that appends the target-dir (LibraryInstaller,
                // davidbarratt's).
                if let Some(t) = target_dir {
                    // `Package::getTargetDir` rewrites `.`/`..` segments.
                    if t.contains('\\')
                        || t.split('/').any(|c| c.is_empty() || c == "." || c == "..")
                    {
                        return Err(format!(
                            "{label}: {name} has target-dir `{t}` (not emulated)"
                        ));
                    }
                    if !rel.ends_with(&format!("/{t}")) {
                        return Err(format!(
                            "{label}: {name} has target-dir `{t}` that its install path `{p}` does not end with (Composer's autoloader cuts it anyway; not emulated)"
                        ));
                    }
                }
                (rel, Some(p))
            }
        };
        resolved.push((who, rel, raw));
    }
    let whos: Vec<Who> = resolved.iter().map(|(w, _, _)| *w).collect();
    let all_fresh = !plugins.is_empty() && plugins.iter().all(|p| placers.fresh.contains(p));
    match resolved.len() {
        0 => Ok(Placed {
            rel: default,
            raw: None,
            who: None,
            whos,
            all_fresh,
        }),
        1 => {
            let (who, rel, raw) = resolved.remove(0);
            Ok(Placed {
                rel,
                raw,
                who: Some(who),
                whos,
                all_fresh,
            })
        }
        _ => {
            if resolved.iter().all(|(_, rel, _)| *rel == resolved[0].1) {
                let (_, rel, _) = resolved.remove(0);
                Ok(Placed {
                    rel,
                    raw: None,
                    who: None,
                    whos,
                    all_fresh,
                })
            } else {
                Err(format!(
                    "{name} ({package_type}) is taken by several installers that place it differently ({}): the installer precedence follows the plugins' activation order (not emulated)",
                    resolved.iter().map(|(_, r, _)| format!("`{r}`")).collect::<Vec<_>>().join(", ")
                ))
            }
        }
    }
}

/// A custom install path's checks: relative, not the project root, not
/// outside it, not the vendor directory itself nor Composer's own
/// `vendor/composer` (a target elsewhere under vendor/ is checked against
/// the other packages' paths once all are known).
fn custom_target(vendor: &str, name: &str, p: &str, who: &str) -> Result<String, String> {
    if crate::pathutil::is_absolute_path(p) {
        return Err(format!(
            "{who}: {name} would install at an absolute path `{p}`"
        ));
    }
    let rel = normalize_path(p);
    if rel.is_empty() || rel == "." {
        return Err(format!("{who}: {name} would install at the project root"));
    }
    if rel.starts_with("../") || rel == ".." {
        return Err(format!(
            "{who}: {name} would install outside the project (`{p}`)"
        ));
    }
    if rel == vendor
        || rel == format!("{vendor}/composer")
        || rel.starts_with(&format!("{vendor}/composer/"))
    {
        return Err(format!(
            "{who}: {name} targets `{p}`, the vendor directory or Composer's own files (not emulated)"
        ));
    }
    Ok(rel)
}

/// `LibraryInstaller::getPackageBasePath`: the install path without the
/// `target-dir` it ends with (`preg_replace('{/*<target-dir>/?$}', '')`).
fn base_path(rel: &str, target_dir: Option<&str>) -> String {
    match target_dir {
        Some(t) => rel
            .strip_suffix(t)
            .map(|r| r.trim_end_matches('/'))
            .filter(|r| r.len() < rel.len() && !r.is_empty())
            .unwrap_or(rel)
            .to_owned(),
        None => rel.to_owned(),
    }
}

pub const CUSTOM_DIRS_PLUGIN: &str = "mnsami/composer-custom-directory-installer";

/// mnsami/composer-custom-directory-installer (docs/reference/plugins/
/// composer-custom-directory-installer/, read at 2.0.0):
/// `PackageUtils::getPackageInstallPath` — the first `extra.installer-paths`
/// entry of the root whose list names the package (exact pretty name),
/// with `{$name}` / `{$vendor}` filled in (`extra.installer-name` of the
/// package overrides the name); `None` (the default path) otherwise.
fn custom_dir(
    root_extra: Option<&Value>,
    pretty_name: &str,
    package_extra: Option<&Value>,
) -> Option<String> {
    let paths = root_extra
        .and_then(|e| e.get("installer-paths"))
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty())?;
    let path = paths.iter().find_map(|(path, names)| {
        names
            .as_array()
            .is_some_and(|l| l.iter().any(|n| n.as_str() == Some(pretty_name)))
            .then(|| path.clone())
    })?;
    let (vendor, mut name) = match pretty_name.split_once('/') {
        Some((v, n)) => (v.to_owned(), n.to_owned()),
        None => (String::new(), pretty_name.to_owned()),
    };
    if let Some(n) = package_extra
        .and_then(|e| e.get("installer-name"))
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty() && *n != "0")
    {
        name = n.to_owned();
    }
    Some(path.replace("{$name}", &name).replace("{$vendor}", &vendor))
}

pub const WP_CORE_PLUGIN: &str = "roots/wordpress-core-installer";

const INSTALLERS_PLUGIN: &str = "composer/installers";
pub const WP_CORE_TYPE: &str = "wordpress-core";

/// Where roots/wordpress-core-installer (and johnpbloch's 2.0.0, which
/// behaves the same here; its rule on two packages sharing a directory is
/// refused in `custom_installers`) puts a `wordpress-core` package.
///
/// Written from the plugin's observed behaviour only, measured against
/// Composer by `harness/wp-core.sh` and `harness/custom-installers.sh`: the
/// plugin is GPL-2.0-or-later, so none of its source is read, copied or
/// vendored here (DECISIONS 2026-10-28).
pub fn wp_core_install_dir(
    root_extra: Option<&Value>,
    pretty_name: &str,
    package_extra: Option<&Value>,
    vendor: &str,
) -> Result<String, String> {
    wp_core_dir(root_extra, pretty_name, package_extra, vendor)
}

/// The observed rules, in order:
/// 1. the root's `extra.wordpress-install-dir`: a string or number is the
///    directory; an object is looked up by the package's pretty name;
/// 2. otherwise the package's own `extra.wordpress-install-dir`;
/// 3. otherwise `wordpress`.
///
/// A value that is unset, `null`, `false`, `""`, `"0"`, `0` or an empty
/// array/object counts as absent and the next rule applies; any other
/// array, object or `false` found at a rule yields nothing there either.
/// `true`, `.` and the vendor directory make Composer fail with
/// "Warning! <dir> is an invalid WordPress install directory (from
/// <package>)!" (`true` printed as `1`): returned as an error.
fn wp_core_dir(
    root_extra: Option<&Value>,
    pretty_name: &str,
    package_extra: Option<&Value>,
    vendor: &str,
) -> Result<String, String> {
    enum Setting {
        Dir(String),
        Invalid,
    }
    fn unset(v: &Value) -> bool {
        match v {
            Value::Null => true,
            Value::Bool(b) => !b,
            Value::String(s) => s.is_empty() || s == "0",
            Value::Number(n) => n.as_f64() == Some(0.0),
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
        }
    }
    fn setting(v: &Value) -> Option<Setting> {
        match v {
            Value::String(s) => Some(Setting::Dir(s.clone())),
            Value::Number(n) => Some(Setting::Dir(n.to_string())),
            Value::Bool(true) => Some(Setting::Invalid),
            _ => None,
        }
    }
    fn present(extra: Option<&Value>) -> Option<&Value> {
        extra
            .and_then(|e| e.get("wordpress-install-dir"))
            .filter(|v| !unset(v))
    }
    let from_root = present(root_extra).and_then(|v| match v {
        Value::Object(by_name) => by_name
            .get(pretty_name)
            .filter(|v| !unset(v))
            .and_then(setting),
        other => setting(other),
    });
    let chosen = from_root
        .or_else(|| present(package_extra).and_then(setting))
        .unwrap_or_else(|| Setting::Dir("wordpress".to_owned()));
    let invalid = |shown: &str| {
        Err(format!(
            "Warning! {shown} is an invalid WordPress install directory (from {pretty_name})!"
        ))
    };
    match chosen {
        Setting::Invalid => invalid("1"),
        Setting::Dir(d) if d == "." || d == vendor => invalid(&d),
        Setting::Dir(d) => Ok(d),
    }
}

impl Layout {
    /// Everything in vendor/ (no layout plugin), for tests and code paths
    /// that have no plugin-aware lock.
    pub fn vendor_only(project_dir: &Path, lock: &Lock, with_dev: bool) -> Layout {
        Self::vendor_only_with(project_dir, lock, with_dev, Dirs::default())
    }

    /// `vendor_only` under the given `config.vendor-dir` / `bin-dir`.
    pub fn vendor_only_with(project_dir: &Path, lock: &Lock, with_dev: bool, dirs: Dirs) -> Layout {
        let mut paths = BTreeMap::new();
        let mut bases = BTreeMap::new();
        for p in lock.wanted_packages(with_dev) {
            if !p.is_virtual(false) {
                paths.insert(
                    p.name().to_owned(),
                    vendor_rel(dirs.vendor_rel(), p.name(), p.target_dir()),
                );
                bases.insert(
                    p.name().to_owned(),
                    format!("{}/{}", dirs.vendor_rel(), p.name()),
                );
            }
        }
        Layout {
            root: absolutize(project_dir),
            dirs,
            paths,
            installers_tag: None,
            removals: BTreeMap::new(),
            bases,
            strip_vendor: BTreeSet::new(),
            removal_notes: BTreeMap::new(),
            download_leftovers: BTreeMap::new(),
        }
    }

    /// The full pass: plugin, allow-plugins, paths, refused targets, and the
    /// removal plan for installed.json packages that went away.
    pub fn resolve(
        project_dir: &Path,
        lock: &Lock,
        manifest: &Value,
        with_dev: bool,
        plugins_enabled: bool,
    ) -> Result<Layout, Vec<String>> {
        let root = absolutize(project_dir);
        let dirs = Dirs::resolve(manifest).map_err(|e| vec![e])?;
        let vendor = dirs.vendor_rel().to_owned();
        let mut issues: Vec<String> = Vec::new();
        let wanted: Vec<&LockPackage> = lock.wanted_packages(with_dev).collect();
        let previous = installed_packages(&root, &dirs);
        let has_state = dirs.composer_dir(&root).join("installed.json").is_file();

        // Is the plugin active? Composer loads it from installed.json
        // (PluginManager::loadInstalledPlugins) and installs it first in the
        // transaction; vivacity only emulates the states where both views
        // agree. A plugin present on one side only (added, removed, or in
        // require-dev with --no-dev) is a transition left to Composer.
        let lock_plugin = wanted.iter().find(|p| p.name() == "composer/installers");
        let prev_plugin = previous
            .iter()
            .find(|p| p["name"].as_str() == Some("composer/installers"));
        let mut table: Option<&installers::Table> = None;
        let mut fresh: BTreeSet<&'static str> = BTreeSet::new();
        let mut installers_tag = None;
        if plugins_enabled && (lock_plugin.is_some() || prev_plugin.is_some()) {
            let allow = merged_allow_plugins(
                manifest.get("config").and_then(|c| c.get("allow-plugins")),
                global_allow_plugins().as_ref(),
            );
            match plugin_verdict(allow.as_ref(), "composer/installers") {
                PluginVerdict::Allowed => {
                    let version = lock_plugin
                        .map(|p| p.version().to_owned())
                        .or_else(|| {
                            prev_plugin.and_then(|p| p["version"].as_str().map(str::to_owned))
                        })
                        .unwrap_or_default();
                    let Some(t) = installers::table_for(&version) else {
                        return Err(vec![format!(
                            "composer/installers {version} is not a ported version (ported: {})",
                            installers::ported_versions().collect::<Vec<_>>().join(", ")
                        )]);
                    };
                    if has_state && lock_plugin.is_some() != prev_plugin.is_some() {
                        let root_extra = manifest.get("extra");
                        let taken = |name: &str, ty: &str, extra: Option<&Value>| {
                            !matches!(
                                installers::placement(t, root_extra, name, ty, extra),
                                Ok(Placement::Vendor)
                            )
                        };
                        let any_taken = wanted
                            .iter()
                            .any(|p| taken(p.name(), p.package_type(), p.raw.get("extra")))
                            || previous.iter().any(|p| {
                                taken(
                                    p["name"].as_str().unwrap_or(""),
                                    p["type"].as_str().unwrap_or("library"),
                                    p.get("extra"),
                                )
                            });
                        if any_taken {
                            let how = if lock_plugin.is_some() {
                                "added to"
                            } else {
                                "removed from"
                            };
                            return Err(vec![format!(
                                "composer/installers is being {how} an existing install (installed.json and composer.lock disagree): let Composer handle this transition"
                            )]);
                        }
                    }
                    let prev_version = prev_plugin.and_then(|p| p["version"].as_str());
                    let one_x =
                        |v: &str| installers::table_for(v).is_some_and(|t| !t.absolute_paths());
                    if !t.absolute_paths() || prev_version.is_some_and(one_x) {
                        if let Some(issue) = installers_one_x_issue(
                            lock_plugin.map(|p| p.version()),
                            prev_version,
                            &wanted,
                            &previous,
                            &allow,
                        ) {
                            return Err(vec![issue]);
                        }
                    }
                    if lock_plugin.is_some() {
                        installers_tag = Some(t.tag.clone());
                        table = Some(t);
                        if prev_plugin.is_none() {
                            fresh.insert(INSTALLERS_PLUGIN);
                        }
                    }
                }
                PluginVerdict::Blocked => {} // Composer ignores it: everything in vendor/
                PluginVerdict::Unlisted => {
                    return Err(vec![
                        "composer/installers is a plugin not covered by config.allow-plugins (Composer would refuse to run it)"
                            .to_owned(),
                    ]);
                }
            }
        }

        // roots/wordpress-core-installer: the same two views must agree
        // (installed.json and the lock), like composer/installers above.
        let wp_lock = wanted.iter().any(|p| p.name() == WP_CORE_PLUGIN);
        let wp_prev = previous
            .iter()
            .any(|p| p["name"].as_str() == Some(WP_CORE_PLUGIN));
        let mut wp_core = false;
        if plugins_enabled && (wp_lock || wp_prev) {
            let allow = merged_allow_plugins(
                manifest.get("config").and_then(|c| c.get("allow-plugins")),
                global_allow_plugins().as_ref(),
            );
            match plugin_verdict(allow.as_ref(), WP_CORE_PLUGIN) {
                PluginVerdict::Allowed => {
                    let any_core = wanted.iter().any(|p| p.package_type() == WP_CORE_TYPE)
                        || previous
                            .iter()
                            .any(|p| p["type"].as_str() == Some(WP_CORE_TYPE));
                    if has_state && wp_lock != wp_prev && any_core {
                        return Err(vec![format!(
                            "{WP_CORE_PLUGIN} is being {} an existing install (installed.json and composer.lock disagree): let Composer handle this transition",
                            if wp_lock { "added to" } else { "removed from" }
                        )]);
                    }
                    wp_core = wp_lock;
                    if wp_lock && !wp_prev {
                        fresh.insert(WP_CORE_PLUGIN);
                    }
                }
                PluginVerdict::Blocked => {}
                PluginVerdict::Unlisted => {
                    return Err(vec![format!(
                        "{WP_CORE_PLUGIN} is a plugin not covered by config.allow-plugins (Composer would refuse to run it)"
                    )]);
                }
            }
        }

        // mnsami/composer-custom-directory-installer: same views rule;
        // with composer/installers also active the installer order
        // follows the plugins' activation order — not reproduced.
        let cd_lock = wanted.iter().any(|p| p.name() == CUSTOM_DIRS_PLUGIN);
        let cd_prev = previous
            .iter()
            .any(|p| p["name"].as_str() == Some(CUSTOM_DIRS_PLUGIN));
        let mut custom_dirs = false;
        if plugins_enabled && (cd_lock || cd_prev) {
            let allow = merged_allow_plugins(
                manifest.get("config").and_then(|c| c.get("allow-plugins")),
                global_allow_plugins().as_ref(),
            );
            match plugin_verdict(allow.as_ref(), CUSTOM_DIRS_PLUGIN) {
                PluginVerdict::Allowed => {
                    if table.is_some() {
                        return Err(vec![format!(
                            "{CUSTOM_DIRS_PLUGIN} together with composer/installers: the installer precedence follows the plugins' activation order (not emulated)"
                        )]);
                    }
                    let root_extra = manifest.get("extra");
                    let mapped = |name: &str, ty: &str, extra: Option<&Value>| {
                        matches!(ty, "library" | "composer-plugin")
                            && custom_dir(root_extra, name, extra).is_some()
                    };
                    let any_mapped = wanted
                        .iter()
                        .any(|p| mapped(p.name(), p.package_type(), p.raw.get("extra")))
                        || previous.iter().any(|p| {
                            mapped(
                                p["name"].as_str().unwrap_or(""),
                                p["type"].as_str().unwrap_or("library"),
                                p.get("extra"),
                            )
                        });
                    if has_state && cd_lock != cd_prev && any_mapped {
                        return Err(vec![format!(
                            "{CUSTOM_DIRS_PLUGIN} is being {} an existing install (installed.json and composer.lock disagree): let Composer handle this transition",
                            if cd_lock { "added to" } else { "removed from" }
                        )]);
                    }
                    custom_dirs = cd_lock;
                    if cd_lock && !cd_prev {
                        fresh.insert(CUSTOM_DIRS_PLUGIN);
                    }
                }
                PluginVerdict::Blocked => {}
                PluginVerdict::Unlisted => {
                    return Err(vec![format!(
                        "{CUSTOM_DIRS_PLUGIN} is a plugin not covered by config.allow-plugins (Composer would refuse to run it)"
                    )]);
                }
            }
        }

        let root_extra = manifest.get("extra");
        let root_type = manifest
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("library")
            .to_owned();
        let mut placers = Placers {
            table,
            wp_core,
            custom_dirs,
            custom: Vec::new(),
            root_type,
            fresh,
        };
        if plugins_enabled {
            let allow = merged_allow_plugins(
                manifest.get("config").and_then(|c| c.get("allow-plugins")),
                global_allow_plugins().as_ref(),
            );
            let raw_of = |p: &&LockPackage| p.raw.clone();
            let lock_entries: Vec<Map<String, Value>> = wanted.iter().map(raw_of).collect();
            let prev_entries: Vec<Map<String, Value>> = previous
                .iter()
                .filter_map(|p| p.as_object().cloned())
                .collect();
            // A global installer plugin registers after the project's
            // (PluginManager::loadInstalledPlugins), so it wins its types.
            let global = global_installed_packages();
            for (plugin, _) in custom_installers::READ_VERSIONS {
                if global.iter().any(|p| p["name"].as_str() == Some(*plugin)) {
                    return Err(vec![format!(
                        "{plugin} is installed globally (COMPOSER_HOME): its installer registers after the project's plugins (not emulated)"
                    )]);
                }
            }
            for (plugin, _) in custom_installers::READ_VERSIONS {
                let lock_p = wanted.iter().find(|p| p.name() == *plugin);
                let prev_p = previous
                    .iter()
                    .find(|p| p["name"].as_str() == Some(*plugin));
                if lock_p.is_none() && prev_p.is_none() {
                    continue;
                }
                match plugin_verdict(allow.as_ref(), plugin) {
                    PluginVerdict::Blocked => continue,
                    PluginVerdict::Unlisted => {
                        return Err(vec![format!(
                            "{plugin} is a plugin not covered by config.allow-plugins (Composer would refuse to run it)"
                        )]);
                    }
                    PluginVerdict::Allowed => {}
                }
                // The versions on both sides: the plugin loaded from
                // installed.json, and the one the transaction installs.
                let versions = [
                    lock_p.map(|p| p.version().to_owned()),
                    prev_p.and_then(|p| p["version"].as_str().map(str::to_owned)),
                ];
                for v in versions.iter().flatten() {
                    if let Some(issue) = custom_installers::version_issue(plugin, v) {
                        return Err(vec![issue]);
                    }
                }
                let placer =
                    match custom_placer(plugin, manifest, &lock_entries, &prev_entries, &wanted) {
                        Ok(p) => p,
                        Err(e) => return Err(vec![e]),
                    };
                let Some(placer) = placer else { continue };
                let taken = |e: &Map<String, Value>| {
                    placer.supports(e.get("type").and_then(Value::as_str).unwrap_or("library"))
                };
                if has_state
                    && lock_p.is_some() != prev_p.is_some()
                    && (lock_entries.iter().any(taken) || prev_entries.iter().any(taken))
                {
                    return Err(vec![format!(
                        "{plugin} is being {} an existing install (installed.json and composer.lock disagree): let Composer handle this transition",
                        if lock_p.is_some() { "added to" } else { "removed from" }
                    )]);
                }
                if lock_p.is_some() {
                    if prev_p.is_none() {
                        placers.fresh.insert(placer.plugin());
                    }
                    placers.custom.push(placer);
                }
            }
        }

        let flex_packs = lock.flex_packs(manifest, with_dev, plugins_enabled);
        let mut paths: BTreeMap<String, String> = BTreeMap::new();
        let mut bases: BTreeMap<String, String> = BTreeMap::new();
        let mut customs: BTreeSet<String> = BTreeSet::new();
        let mut strip_vendor: BTreeSet<String> = BTreeSet::new();
        // A package the transaction leaves alone: same version and
        // references in installed.json (no install, no update, so no
        // installer is asked to lay it out).
        let reference =
            |v: &Map<String, Value>, k: &str| v.get(k).and_then(|d| d.get("reference")).cloned();
        let unchanged = |p: &LockPackage| {
            previous.iter().filter_map(Value::as_object).any(|e| {
                e.get("name").and_then(Value::as_str) == Some(p.name())
                    && e.get("version") == p.raw.get("version")
                    && reference(e, "dist") == reference(&p.raw, "dist")
                    && reference(e, "source") == reference(&p.raw, "source")
            })
        };
        let mut johnpbloch_dirs: BTreeMap<String, String> = BTreeMap::new();
        let mut download_leftovers: BTreeMap<String, String> = BTreeMap::new();
        let vendor_composer =
            normalize_path(&format!("{}/{vendor}/composer", root.to_string_lossy()));
        let root_norm = normalize_path(&root.to_string_lossy());
        for p in &wanted {
            if p.is_virtual(flex_packs) {
                continue;
            }
            match place(&vendor, &placers, root_extra, &p.raw) {
                Ok(placed) => {
                    let default = vendor_rel(&vendor, p.name(), p.target_dir());
                    if placed.rel != default {
                        customs.insert(p.name().to_owned());
                    }
                    // LibraryInstaller::updateCode renames the install path
                    // of the initial package to the target's, both computed
                    // by the current installers: a package whose own
                    // metadata moves it (`installer-name`, its autoload for
                    // neos) — not emulated. A path that moves because the
                    // configuration did, on a package left unchanged, is a
                    // fresh install (`isInstalled` misses it, the old
                    // directory stays), which the transaction does. The same
                    // configuration move on a package that is also updated
                    // has no harness case: handed over too.
                    if placers.any() && !unchanged(p) {
                        let entry = previous
                            .iter()
                            .find(|e| e["name"].as_str() == Some(p.name()));
                        // The initial package's path under the current
                        // installers, then where installed.json says it is.
                        let recomputed = entry
                            .and_then(Value::as_object)
                            .and_then(|e| place(&vendor, &placers, root_extra, e).ok())
                            .map(|b| b.rel);
                        let recorded = entry
                            .and_then(|e| e.get("install-path"))
                            .and_then(Value::as_str)
                            .map(|ip| {
                                if crate::pathutil::is_absolute_path(ip) {
                                    normalize_path(ip)
                                } else {
                                    normalize_path(&format!("{vendor_composer}/{ip}"))
                                }
                            })
                            .and_then(|abs| {
                                abs.strip_prefix(&format!("{root_norm}/"))
                                    .map(str::to_owned)
                            });
                        if let Some(before) = recomputed
                            .into_iter()
                            .chain(recorded)
                            .find(|b| *b != placed.rel)
                        {
                            issues.push(format!(
                                "installed package {} moves from `{before}` to `{}` on update: let Composer handle this move",
                                p.name(),
                                placed.rel
                            ));
                        }
                    }
                    if placed.contested() && placed.whos.contains(&Who::Johnpbloch) {
                        issues.push(format!(
                            "{} ({}) is taken by {} and another installer: which one Composer asks follows the plugins' activation order (not emulated)",
                            p.name(),
                            p.package_type(),
                            custom_installers::JOHNPBLOCH
                        ));
                    }
                    if placed.whos.contains(&Who::Johnpbloch) {
                        johnpbloch_dirs.insert(placed.rel.clone(), p.name().to_owned());
                    }
                    if placed.contested()
                        && placed.whos.contains(&Who::Kirby)
                        && custom_installers::kirby_strips_vendor(&p.raw)
                        && !unchanged(p)
                    {
                        issues.push(format!(
                            "{} ({}) is taken by {} and another installer: whether its vendor/ is removed (`Installer::postInstall`) follows the plugins' activation order (not emulated)",
                            p.name(),
                            p.package_type(),
                            custom_installers::KIRBY
                        ));
                    }
                    // BinaryInstaller `realpath`s the binaries of a package
                    // an installer placed at a relative path: through a
                    // path repository's symlink, the proxy points at the
                    // source (not emulated).
                    if placed.rel != default
                        && placed.whos.iter().any(|w| w.relative())
                        && p.dist_kind() == crate::lock::DistKind::Path
                        && !p.bins().is_empty()
                    {
                        issues.push(format!(
                            "{} is a path package with binaries placed at a relative path (`{}`): BinaryInstaller resolves them with realpath (not emulated)",
                            p.name(),
                            placed.rel
                        ));
                    }
                    if placed.all_fresh
                        && placed.rel != default
                        && matches!(
                            p.dist_kind(),
                            crate::lock::DistKind::Zip | crate::lock::DistKind::Tar
                        )
                    {
                        // `<vendor>/<name>/<target-dir>` without its last
                        // component: what `ensureDirectoryExists` made above
                        // the path `cleanup` removes.
                        if let Some((above, _)) =
                            p.target_dir().and_then(|_| default.rsplit_once('/'))
                        {
                            download_leftovers.insert(p.name().to_owned(), above.to_owned());
                        }
                    }
                    if placed.who == Some(Who::Kirby)
                        && custom_installers::kirby_strips_vendor(&p.raw)
                    {
                        if p.dist_kind() == crate::lock::DistKind::Path {
                            issues.push(format!(
                                "{}: {} is a path package, whose source's own vendor/ the installer would remove (not emulated)",
                                custom_installers::KIRBY,
                                p.name()
                            ));
                        }
                        // `postInstall` runs after BinaryInstaller: a binary
                        // in the removed vendor/ keeps a proxy to nothing.
                        if let Some(bin) = p
                            .bins()
                            .into_iter()
                            .find(|b| normalize_path(b).starts_with("vendor/"))
                        {
                            issues.push(format!(
                                "{}: {} has a binary `{bin}` in the vendor/ the installer removes (not emulated)",
                                custom_installers::KIRBY,
                                p.name()
                            ));
                        }
                        if let Some(dir) = autoload_reaches_vendor(&p.raw) {
                            issues.push(format!(
                                "{}: {} autoloads `{dir}`, which reaches the package's own vendor/ that the installer removes (not emulated)",
                                custom_installers::KIRBY,
                                p.name()
                            ));
                        }
                        strip_vendor.insert(p.name().to_owned());
                    }
                    let base = if placed.rel == default {
                        format!("{vendor}/{}", p.name())
                    } else {
                        base_path(&placed.rel, p.target_dir())
                    };
                    bases.insert(p.name().to_owned(), base);
                    paths.insert(p.name().to_owned(), placed.rel);
                }
                Err(e) => issues.push(e),
            }
        }

        // Conflicting targets: two packages at the same place, or one under
        // the other (a placed package against every path).
        if placers.any() {
            let mut by_path: BTreeMap<&str, &str> = BTreeMap::new();
            for (name, rel) in &paths {
                if let Some(other) = by_path.insert(rel.as_str(), name.as_str()) {
                    issues.push(format!("{name} and {other} would both install at `{rel}`"));
                }
            }
            let bin_rel = dirs.bin_rel().to_owned();
            for name in &customs {
                let Some(rel) = paths.get(name) else { continue };
                // The binaries' directory and the autoloader vivacity writes
                // under vendor/ are not a package's to share.
                let autoload = format!("{vendor}/autoload.php");
                if *rel == bin_rel
                    || rel.starts_with(&format!("{bin_rel}/"))
                    || bin_rel.starts_with(&format!("{rel}/"))
                    || *rel == autoload
                {
                    issues.push(format!(
                        "{name} at `{rel}` would share `{bin_rel}` or `{autoload}` (not emulated)"
                    ));
                }
                for (other, other_rel) in &paths {
                    if other == name {
                        continue;
                    }
                    if other_rel.starts_with(&format!("{rel}/")) {
                        issues.push(format!(
                            "{name} at `{rel}` would contain {other} at `{other_rel}`"
                        ));
                    } else if rel.starts_with(&format!("{other_rel}/")) {
                        issues.push(format!(
                            "{name} at `{rel}` would be inside {other} at `{other_rel}`"
                        ));
                    }
                }
            }
        }

        // Removal plan: Composer recomputes the path of a removed package with
        // the current configuration; we only delete if that path is the one
        // where the package was laid out (installed.json), else fallback.
        let mut removals = BTreeMap::new();
        let mut removal_notes = BTreeMap::new();
        let wanted_names: std::collections::BTreeSet<&str> =
            wanted.iter().map(|p| p.name()).collect();
        for prev in &previous {
            let name = prev["name"].as_str().unwrap_or("");
            if name.is_empty() || wanted_names.contains(name) {
                continue;
            }
            let Some(old_ip) = prev.get("install-path").and_then(Value::as_str) else {
                continue; // metapackage
            };
            let old_abs = if crate::pathutil::is_absolute_path(old_ip) {
                normalize_path(old_ip)
            } else {
                normalize_path(&format!("{vendor_composer}/{old_ip}"))
            };
            let Some(old_rel) = old_abs
                .strip_prefix(&format!("{root_norm}/"))
                .filter(|r| !r.is_empty())
            else {
                issues.push(format!(
                    "installed package {name} lives outside the project (`{old_ip}`): not removing it"
                ));
                continue;
            };
            let Some(entry) = prev.as_object() else {
                continue;
            };
            let target_dir = prev
                .get("target-dir")
                .and_then(Value::as_str)
                .map(|t| t.trim_matches('/'))
                .filter(|t| !t.is_empty());
            match place(&vendor, &placers, root_extra, entry) {
                Ok(placed) if placed.rel == old_rel => {
                    let default = vendor_rel(&vendor, name, target_dir);
                    // LibraryInstaller::removeCode deletes getPackageBasePath:
                    // the install path without its target-dir.
                    let dir = if placed.rel == default {
                        format!("{vendor}/{name}")
                    } else {
                        base_path(&placed.rel, target_dir)
                    };
                    // composer/installers' `Installer::uninstall`: "Deleting
                    // <getPackageBasePath> - deleted" on stdout — the path as
                    // the installer returns it (2.x: `getcwd() . '/' . $path`).
                    // WordPressCoreInstaller::getInstallPath: "Two packages
                    // (…) cannot share the same directory!" when the removed
                    // package's directory is one an installed one takes.
                    if placed.whos.contains(&Who::Johnpbloch) {
                        if let Some(other) = johnpbloch_dirs.get(&placed.rel) {
                            issues.push(format!(
                                "{}: Two packages ({other} and {name}) cannot share the same directory!",
                                custom_installers::JOHNPBLOCH
                            ));
                        }
                    }
                    match (placed.who, &placed.raw) {
                        (Some(Who::Installers { absolute }), Some(raw)) => {
                            let shown = base_path_raw(raw, target_dir);
                            let shown = if absolute {
                                format!("{}/{shown}", real_cwd(&root))
                            } else {
                                shown
                            };
                            removal_notes.insert(name.to_owned(), format!("Deleting {shown} - deleted"));
                        }
                        (Some(Who::Oomphinc), Some(raw)) => {
                            removal_notes.insert(
                                name.to_owned(),
                                format!("Deleting {} - deleted", base_path_raw(raw, target_dir)),
                            );
                        }
                        _ if placed.contested()
                            && placed
                                .whos
                                .iter()
                                .any(|w| matches!(w, Who::Installers { .. } | Who::Oomphinc)) =>
                        {
                            issues.push(format!(
                                "removal of {name}: several installers take its type, and which one removes it (and what it prints) follows the plugins' activation order (not emulated)"
                            ));
                        }
                        _ => {}
                    }
                    removals.insert(name.to_owned(), dir);
                }
                Ok(placed) => issues.push(format!(
                    "installed package {name} is at `{old_rel}` but the current layout puts it at `{}`: let Composer handle this removal",
                    placed.rel
                )),
                Err(e) => issues.push(format!("removal of {name}: {e}")),
            }
        }

        if issues.is_empty() {
            Ok(Layout {
                root,
                dirs,
                paths,
                installers_tag,
                removals,
                bases,
                strip_vendor,
                removal_notes,
                download_leftovers,
            })
        } else {
            Err(issues)
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }

    /// `<root>/<vendor-dir>` (not canonicalised).
    pub fn vendor_dir(&self) -> PathBuf {
        self.dirs.vendor_dir(&self.root)
    }

    /// `<root>/<bin-dir>`.
    pub fn bin_dir(&self) -> PathBuf {
        self.dirs.bin_dir(&self.root)
    }

    /// `<root>/<vendor-dir>/composer`.
    pub fn composer_dir(&self) -> PathBuf {
        self.dirs.composer_dir(&self.root)
    }

    /// The root package's `install_path` in installed.php: relative to
    /// `<vendor-dir>/composer` like the packages' (`'../../'` by default).
    pub fn root_install_path(&self) -> String {
        let root = self.root.to_string_lossy();
        find_shortest_path(
            &format!("{root}/{}/composer", self.dirs.vendor_rel()),
            &root,
            true,
        )
    }

    /// Project-relative path (None: metapackage or unknown package).
    pub fn rel(&self, name: &str) -> Option<&str> {
        self.paths.get(name).map(String::as_str)
    }

    /// Absolute install path.
    pub fn abs(&self, name: &str) -> Option<PathBuf> {
        self.rel(name).map(|r| self.root.join(r))
    }

    /// Root to empty before laying out the package: `getPackageBasePath`,
    /// `vendor/<name>` (target-dir included) for LibraryInstaller, the
    /// installer's path without its target-dir otherwise.
    pub fn package_root(&self, name: &str) -> Option<PathBuf> {
        self.rel(name)?;
        self.bases.get(name).map(|b| self.root.join(b))
    }

    /// getkirby/composer-installer removes this package's own `vendor/`
    /// once it is laid out (`Installer::postInstall`).
    pub fn strips_vendor(&self, name: &str) -> bool {
        self.strip_vendor.contains(name)
    }

    /// The empty directory Composer's download leaves for this package
    /// (`download_leftovers`), absolute.
    pub fn download_leftover(&self, name: &str) -> Option<PathBuf> {
        self.download_leftovers.get(name).map(|r| self.root.join(r))
    }

    /// The stdout line the installer writes once this removed package is
    /// gone (composer/installers' `Installer::uninstall`).
    pub fn removal_note(&self, name: &str) -> Option<&str> {
        self.removal_notes.get(name).map(String::as_str)
    }

    /// `install-path` of installed.json / installed.php: relative to
    /// vendor/composer (`findShortestPath($repoDir, $path, true)`).
    pub fn install_path(&self, name: &str) -> Option<String> {
        let rel = self.rel(name)?;
        let root = self.root.to_string_lossy();
        Some(find_shortest_path(
            &format!("{root}/{}/composer", self.dirs.vendor_rel()),
            &format!("{root}/{rel}"),
            true,
        ))
    }

    /// installed.json packages to remove, with their absolute path.
    pub fn removals(&self) -> impl Iterator<Item = (&str, PathBuf)> {
        self.removals
            .iter()
            .map(|(n, rel)| (n.as_str(), self.root.join(rel)))
    }
}

/// The placer of a custom-installer plugin under this configuration:
/// `None` when it places nothing (craftcms without a `craft-plugin`
/// package, contao-components without a component), `Err` when Composer
/// would do something vivacity does not reproduce.
fn custom_placer(
    plugin: &str,
    manifest: &Value,
    lock_entries: &[Map<String, Value>],
    prev_entries: &[Map<String, Value>],
    wanted: &[&LockPackage],
) -> Result<Option<Placer>, String> {
    use custom_installers as ci;
    let root_extra = manifest.get("extra");
    let any_of_type = |t: &str| {
        lock_entries
            .iter()
            .chain(prev_entries)
            .any(|e| e.get("type").and_then(Value::as_str) == Some(t))
    };
    Ok(match plugin {
        ci::KIRBY => Some(Placer::Kirby),
        ci::NEOS => Some(Placer::Neos),
        ci::JOHNPBLOCH => Some(Placer::Johnpbloch),
        ci::DAVIDBARRATT => Some(Placer::DavidBarratt(ci::DavidBarrattConfig::from_root(
            root_extra,
        )?)),
        ci::THELIA => Some(Placer::Thelia(ci::thelia3(root_extra)?)),
        ci::CRAFT => {
            // `Installer::addPlugin`/`savePlugins` (vendor/craftcms/
            // plugins.php) runs for a `craft-plugin` package or root only.
            let root_type = manifest.get("type").and_then(Value::as_str);
            if root_type == Some("craft-plugin") || any_of_type("craft-plugin") {
                return Err(format!(
                    "{}: a craft-plugin package writes vendor/craftcms/plugins.php (not emulated)",
                    ci::CRAFT
                ));
            }
            None
        }
        ci::CONTAO_COMPONENTS => {
            // `Plugin::hasComponentDir`.
            let config_has = |k: &str| {
                manifest.get("config").and_then(|c| c.get(k)).is_some()
                    || global_config_value(k).is_some()
            };
            match root_extra.and_then(|e| e.get("contao-component-dir")) {
                Some(Value::String(dir)) => Some(Placer::ContaoComponents(dir.clone())),
                Some(Value::Null) | None => {
                    if config_has("contao-component-dir") || config_has("component-dir") {
                        return Err(format!(
                            "{}: config.component-dir / config.contao-component-dir (a deprecation line, or an exception; not emulated)",
                            ci::CONTAO_COMPONENTS
                        ));
                    }
                    // NoopInstaller: a component is recorded, never laid out.
                    if any_of_type("contao-component") {
                        return Err(format!(
                            "{}: no extra.contao-component-dir — its NoopInstaller records contao-component packages without laying them out (not emulated)",
                            ci::CONTAO_COMPONENTS
                        ));
                    }
                    None
                }
                Some(_) => {
                    return Err(format!(
                        "{}: extra.contao-component-dir is not a string",
                        ci::CONTAO_COMPONENTS
                    ))
                }
            }
        }
        ci::OOMPHINC => {
            // Its installer extends composer/installers' `Installer` (the
            // `BaseInstaller` of the version installed), loaded through its
            // own autoloader whatever allow-plugins says of composer/installers:
            // that version is guarded here, on both sides of the transaction.
            let installers_version = |e: &Map<String, Value>| {
                (e.get("name").and_then(Value::as_str) == Some("composer/installers")).then(|| {
                    e.get("version")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
            };
            let lock_v = lock_entries.iter().find_map(installers_version);
            let prev_v = prev_entries.iter().find_map(installers_version);
            if prev_v.is_some() && prev_v != lock_v {
                return Err(format!(
                    "{}: composer/installers changes version under it (installed.json and composer.lock disagree): let Composer handle this transition",
                    ci::OOMPHINC
                ));
            }
            let Some(table) = wanted
                .iter()
                .find(|p| p.name() == "composer/installers")
                .and_then(|p| installers::table_for(p.version()))
            else {
                return Err(format!(
                    "{}: composer/installers is not installed in a ported version",
                    ci::OOMPHINC
                ));
            };
            if !table.absolute_paths() {
                // Compiling 1.x's Installer.php prints a PHP 8.4 deprecation.
                return Err(format!(
                    "{}: on top of composer/installers 1.x (its classes, loaded by the extender, print deprecations; not emulated)",
                    ci::OOMPHINC
                ));
            }
            let types = match root_extra.and_then(|e| e.get("installer-types")) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) if a.iter().all(Value::is_string) => a
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                Some(_) => {
                    return Err(format!(
                        "{}: extra.installer-types is not a list of strings",
                        ci::OOMPHINC
                    ))
                }
            };
            Some(Placer::Oomphinc(types, table))
        }
        _ => None,
    })
}

/// composer/installers 1.x: compiling its `Installer.php` on PHP 8.4+
/// raises deprecations Composer's `ErrorHandler` prints (the first, then
/// "More deprecation notices were hidden" — every one, with a stack trace,
/// under -v), a process-wide state vivacity reproduces only alone: no
/// version change on either side, not under -v, and no other plugin that
/// could raise its own first.
fn installers_one_x_issue(
    lock_version: Option<&str>,
    prev_version: Option<&str>,
    wanted: &[&LockPackage],
    previous: &[Value],
    allow: &Option<Value>,
) -> Option<String> {
    if let (Some(l), Some(p)) = (lock_version, prev_version) {
        if l != p {
            return Some(format!(
                "composer/installers changes from {p} to {l}: a 1.x on either side prints PHP deprecations at a point vivacity does not reproduce (let Composer handle this transition)"
            ));
        }
    }
    if cfg!(windows) {
        return Some(
            "composer/installers 1.x on Windows: the file path its PHP deprecation names is not reproduced (not emulated)".to_owned(),
        );
    }
    if crate::io::level() >= crate::io::Verbosity::Verbose {
        return Some(
            "composer/installers 1.x under -v: its PHP deprecations come with stack traces (not emulated)".to_owned(),
        );
    }
    // The plugins whose code vivacity has read on PHP 8.4: none raises a
    // deprecation of its own.
    let quiet = |name: &str| {
        name == "composer/installers"
            || name == WP_CORE_PLUGIN
            || name == CUSTOM_DIRS_PLUGIN
            || custom_installers::READ_VERSIONS
                .iter()
                .any(|(p, _)| *p == name)
    };
    let plugin = |name: &str, ty: &str| {
        matches!(ty, "composer-plugin" | "composer-installer")
            && !quiet(name)
            && plugin_verdict(allow.as_ref(), name) != PluginVerdict::Blocked
    };
    let other = wanted
        .iter()
        .find(|p| plugin(p.name(), p.package_type()))
        .map(|p| p.name().to_owned())
        .or_else(|| {
            previous.iter().find_map(|p| {
                let name = p["name"].as_str().unwrap_or_default();
                plugin(name, p["type"].as_str().unwrap_or("library")).then(|| name.to_owned())
            })
        });
    other.map(|o| {
        format!(
            "composer/installers 1.x next to {o}: which plugin prints the first PHP deprecation follows the load order (not emulated)"
        )
    })
}

/// The autoload path of a package that reaches its own `vendor/` (or the
/// whole package): what getkirby/composer-installer removes would still be
/// scanned from vivacity's store.
fn autoload_reaches_vendor(pkg: &Map<String, Value>) -> Option<String> {
    let autoload = pkg.get("autoload")?.as_object()?;
    let mut dirs: Vec<&str> = Vec::new();
    for (kind, v) in autoload {
        match (kind.as_str(), v) {
            ("psr-0" | "psr-4", Value::Object(m)) => {
                for d in m.values() {
                    match d {
                        Value::String(s) => dirs.push(s),
                        Value::Array(a) => dirs.extend(a.iter().filter_map(Value::as_str)),
                        _ => {}
                    }
                }
            }
            ("classmap" | "files", Value::Array(a)) => {
                dirs.extend(a.iter().filter_map(Value::as_str))
            }
            _ => {}
        }
    }
    dirs.into_iter()
        .find(|d| {
            let n = normalize_path(d);
            // A wildcard (classmap `*`, `exclude`-like globs) may reach it.
            n.is_empty() || n == "." || n == "vendor" || n.starts_with("vendor/") || n.contains('*')
        })
        .map(str::to_owned)
}

/// `getPackageBasePath` on the path as the installer built it (trailing
/// slash kept, as composer/installers' line prints it).
fn base_path_raw(raw: &str, target_dir: Option<&str>) -> String {
    let Some(t) = target_dir else {
        return raw.to_owned();
    };
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    match trimmed.strip_suffix(t) {
        Some(r) if !r.is_empty() => r.trim_end_matches('/').to_owned(),
        _ => raw.to_owned(),
    }
}

/// PHP's `getcwd()` at the project root: the real path.
fn real_cwd(root: &Path) -> String {
    let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let s = real.to_string_lossy().into_owned();
    // Windows: no `\\?\` prefix in PHP's form.
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
}

fn installed_packages(root: &Path, dirs: &Dirs) -> Vec<Value> {
    installed_packages_at(&dirs.composer_dir(root))
}

/// The `packages` of `<dir>/installed.json`, raw, or nothing.
pub fn installed_packages_at(composer_dir: &Path) -> Vec<Value> {
    let Some(v) = crate::jsonfile::read(&composer_dir.join("installed.json")) else {
        return Vec::new();
    };
    v.get("packages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The project's installed.json packages, raw, under the manifest's
/// vendor-dir (or `vendor/` when that configuration is one the layout
/// refuses — it will say so).
pub fn installed_packages_of(root: &Path, manifest: &Value) -> Vec<Value> {
    let dirs = Dirs::resolve(manifest).unwrap_or_default();
    installed_packages(root, &dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lock_with(packages: Value) -> Lock {
        Lock::parse(
            &json!({"packages": packages, "packages-dev": [], "plugin-api-version": "2.6.0"})
                .to_string(),
        )
        .expect("lock")
    }

    fn pkg(name: &str, ty: &str, version: &str) -> Value {
        json!({"name": name, "version": version, "type": ty,
               "dist": {"type": "zip", "url": "https://x/y.zip", "reference": "r"}})
    }

    fn root() -> PathBuf {
        // On Windows `/proj` is not absolute (no drive) and would be
        // absolutized under the cwd; an explicit drive keeps the test stable.
        PathBuf::from(if cfg!(windows) { "C:/proj" } else { "/proj" })
    }

    #[test]
    fn allow_plugins_patterns_and_merge() {
        assert!(pattern_matches("composer/*", "composer/installers"));
        assert!(pattern_matches(
            "Composer/Installers",
            "composer/installers"
        ));
        assert!(pattern_matches("*", "anything/here"));
        assert!(!pattern_matches("composer/*", "other/installers"));
        assert!(pattern_matches("*/installers", "composer/installers"));
        let rules = json!({"composer/*": false, "composer/installers": true});
        // First matching rule: `composer/*` -> refused.
        assert_eq!(
            plugin_verdict(Some(&rules), "composer/installers"),
            PluginVerdict::Blocked
        );
        assert_eq!(
            plugin_verdict(Some(&json!(true)), "x/y"),
            PluginVerdict::Allowed
        );
        assert_eq!(
            plugin_verdict(Some(&json!({})), "x/y"),
            PluginVerdict::Unlisted
        );
        assert_eq!(plugin_verdict(None, "x/y"), PluginVerdict::Unlisted);
        let merged = merged_allow_plugins(
            Some(&json!({"a/b": false})),
            Some(&json!({"a/b": true, "c/d": true})),
        );
        assert_eq!(merged, Some(json!({"a/b": false, "c/d": true})));
    }

    #[test]
    fn vendor_dir_moves_every_derived_path() {
        let lock = lock_with(json!([pkg("a/b", "library", "1.0.0")]));
        let manifest = json!({"config": {"vendor-dir": "./lib/vendor/", "bin-dir": "bin"}});
        let l = Layout::resolve(&root(), &lock, &manifest, true, true).expect("layout");
        assert_eq!(l.rel("a/b"), Some("lib/vendor/a/b"));
        assert_eq!(l.abs("a/b"), Some(root().join("lib/vendor/a/b")));
        assert_eq!(
            l.package_root("a/b"),
            Some(root().join("lib/vendor").join("a/b"))
        );
        assert_eq!(l.install_path("a/b").as_deref(), Some("../a/b"));
        assert_eq!(l.root_install_path(), "../../../");
        assert_eq!(l.vendor_dir(), root().join("lib/vendor"));
        assert_eq!(l.bin_dir(), root().join("bin"));
        assert_eq!(l.composer_dir(), root().join("lib/vendor/composer"));
        let l = Layout::resolve(&root(), &lock, &json!({}), true, true).expect("layout");
        assert_eq!(l.root_install_path(), "../../");
        assert_eq!(l.bin_dir(), root().join("vendor/bin"));
        // A refused form is a layout issue, not a silent default.
        let manifest = json!({"config": {"vendor-dir": "../shared/vendor"}});
        let err = Layout::resolve(&root(), &lock, &manifest, true, true).unwrap_err();
        assert!(err[0].contains("outside the project"), "{err:?}");
    }

    #[test]
    fn without_plugin_everything_goes_to_vendor() {
        let lock = lock_with(json!([
            pkg("a/b", "wordpress-plugin", "1.0.0"),
            pkg("a/meta", "metapackage", "1.0.0")
        ]));
        let manifest =
            json!({"extra": {"installer-paths": {"web/{$name}": ["type:wordpress-plugin"]}}});
        let l = Layout::resolve(&root(), &lock, &manifest, true, true).expect("layout");
        assert_eq!(l.rel("a/b"), Some("vendor/a/b"));
        assert_eq!(l.rel("a/meta"), None);
        assert_eq!(l.install_path("a/b").as_deref(), Some("../a/b"));
        assert!(l.installers_tag.is_none());
    }

    #[test]
    fn plugin_allowed_places_packages_and_blocked_keeps_vendor() {
        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.3.0"),
            pkg("wpackagist-plugin/akismet", "wordpress-plugin", "5.3"),
            pkg(
                "wpackagist-theme/twentytwentyfour",
                "wordpress-theme",
                "1.0"
            ),
            pkg("monolog/monolog", "library", "3.0.0"),
            pkg("composer/pcre", "library", "3.0.0"),
        ]));
        let manifest = json!({
            "config": {"allow-plugins": {"composer/installers": true}},
            "extra": {"installer-paths": {"web/app/plugins/{$name}/": ["type:wordpress-plugin"]}}
        });
        let l = Layout::resolve(&root(), &lock, &manifest, true, true).expect("layout");
        assert_eq!(l.installers_tag.as_deref(), Some("v2.3.0"));
        assert_eq!(
            l.rel("wpackagist-plugin/akismet"),
            Some("web/app/plugins/akismet")
        );
        assert_eq!(
            l.rel("wpackagist-theme/twentytwentyfour"),
            Some("wp-content/themes/twentytwentyfour")
        );
        assert_eq!(l.rel("monolog/monolog"), Some("vendor/monolog/monolog"));
        assert_eq!(
            l.rel("composer/installers"),
            Some("vendor/composer/installers")
        );
        assert_eq!(
            l.install_path("wpackagist-plugin/akismet").as_deref(),
            Some("../../web/app/plugins/akismet")
        );
        assert_eq!(l.install_path("composer/pcre").as_deref(), Some("./pcre"));
        assert_eq!(
            l.install_path("monolog/monolog").as_deref(),
            Some("../monolog/monolog")
        );
        assert_eq!(
            l.package_root("wpackagist-plugin/akismet"),
            Some(root().join("web/app/plugins/akismet"))
        );

        let blocked = json!({"config": {"allow-plugins": {"composer/installers": false}}});
        let l = Layout::resolve(&root(), &lock, &blocked, true, true).expect("layout");
        assert_eq!(
            l.rel("wpackagist-plugin/akismet"),
            Some("vendor/wpackagist-plugin/akismet")
        );

        let unlisted = json!({"config": {"allow-plugins": {"other/x": true}}});
        let err = Layout::resolve(&root(), &lock, &unlisted, true, true).expect_err("fallback");
        assert!(err[0].contains("allow-plugins"), "{err:?}");

        let missing = json!({});
        assert!(Layout::resolve(&root(), &lock, &missing, true, true).is_err());
    }

    #[test]
    fn custom_dir_like_package_utils() {
        use serde_json::json;
        let root = json!({"installer-paths": {"modules/Log": ["psr/log"], "lib/{$vendor}-{$name}": ["psr/container", "acme/x"]}});
        assert_eq!(
            custom_dir(Some(&root), "psr/log", None).unwrap(),
            "modules/Log"
        );
        assert_eq!(
            custom_dir(Some(&root), "psr/container", None).unwrap(),
            "lib/psr-container"
        );
        assert_eq!(
            custom_dir(
                Some(&root),
                "acme/x",
                Some(&json!({"installer-name": "renamed"}))
            )
            .unwrap(),
            "lib/acme-renamed"
        );
        assert!(custom_dir(Some(&root), "psr/other", None).is_none());
        assert!(custom_dir(None, "psr/log", None).is_none());
    }

    #[test]
    fn wordpress_core_dir_like_the_plugin() {
        use serde_json::json;
        let d = |root: Option<serde_json::Value>, own: Option<serde_json::Value>| {
            wp_core_dir(
                root.as_ref(),
                "roots/wordpress-no-content",
                own.as_ref(),
                "vendor",
            )
        };
        assert_eq!(d(None, None).unwrap(), "wordpress");
        assert_eq!(
            d(Some(json!({"wordpress-install-dir": "web/wp"})), None).unwrap(),
            "web/wp"
        );
        // A map by pretty name; a missing or empty entry falls through.
        assert_eq!(
            d(
                Some(json!({"wordpress-install-dir": {"roots/wordpress-no-content": "cms"}})),
                None
            )
            .unwrap(),
            "cms"
        );
        assert_eq!(
            d(
                Some(json!({"wordpress-install-dir": {"other/pkg": "cms"}})),
                Some(json!({"wordpress-install-dir": "own"}))
            )
            .unwrap(),
            "own"
        );
        // PHP `empty()`: "" and "0" do not count.
        assert_eq!(
            d(Some(json!({"wordpress-install-dir": ""})), None).unwrap(),
            "wordpress"
        );
        assert!(d(Some(json!({"wordpress-install-dir": "."})), None)
            .unwrap_err()
            .contains("invalid WordPress install directory"));
        assert!(d(Some(json!({"wordpress-install-dir": "vendor"})), None).is_err());
    }

    #[test]
    fn unported_version_custom_framework_and_dangerous_targets_are_issues() {
        let manifest = json!({"config": {"allow-plugins": true}});
        let old = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v1.11.0"),
            pkg("a/b", "drupal-module", "1.0")
        ]));
        let err = Layout::resolve(&root(), &old, &manifest, true, true).expect_err("v1");
        assert!(err[0].contains("not a ported version"), "{err:?}");

        let cake = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.2.0"),
            pkg("a/b", "cakephp-plugin", "1.0")
        ]));
        let err = Layout::resolve(&root(), &cake, &manifest, true, true).expect_err("cake");
        assert!(err[0].contains("custom path logic"), "{err:?}");

        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.3.0"),
            pkg("a/b", "drupal-module", "1.0"),
            pkg("a/c", "drupal-module", "1.0")
        ]));
        for (paths, needle) in [
            (json!({"{$name}/../../x": ["a/b"]}), "outside the project"),
            (
                json!({"vendor/composer/{$name}": ["a/b"]}),
                "Composer's own files",
            ),
            (json!({"vendor/bin/{$name}": ["a/b"]}), "would share"),
            (json!({"/abs/{$name}": ["a/b"]}), "absolute path"),
            (json!({"same/": ["a/b", "a/c"]}), "would both install"),
            (json!({"modules/": ["a/b"]}), "would contain"),
        ] {
            let m = json!({"config": {"allow-plugins": true}, "extra": {"installer-paths": paths}});
            let err = Layout::resolve(&root(), &lock, &m, true, true).expect_err(needle);
            assert!(err.iter().any(|e| e.contains(needle)), "{needle}: {err:?}");
        }
        // The root project: empty template.
        let m =
            json!({"config": {"allow-plugins": true}, "extra": {"installer-paths": {"": ["a/b"]}}});
        let err = Layout::resolve(&root(), &lock, &m, true, true).expect_err("root");
        assert!(err.iter().any(|e| e.contains("project root")), "{err:?}");
    }

    fn with(mut v: Value, key: &str, val: Value) -> Value {
        v[key] = val;
        v
    }

    #[test]
    #[cfg(not(windows))]
    fn several_installers_on_one_type() {
        let allow = json!({"config": {"allow-plugins": true}});
        // kirby and composer/installers 1.x agree on a pluginkit's path, but
        // only kirby removes its vendor/: which one Composer asks follows
        // the activation order.
        let kp = with(
            pkg("acme/kp", "kirby-plugin", "1.0"),
            "require",
            json!({"getkirby/composer-installer": "^1.2"}),
        );
        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v1.12.0"),
            pkg("getkirby/composer-installer", "composer-plugin", "1.2.1"),
            kp
        ]));
        let err = Layout::resolve(&root(), &lock, &allow, true, true).expect_err("kirby");
        assert!(err.iter().any(|e| e.contains("postInstall")), "{err:?}");

        // thelia's and composer/installers' template paths agree; nothing
        // else differs on install.
        let thelia3 = json!({"config": {"allow-plugins": true}, "extra": {"installer-paths": {
            "vendor/{$vendor}/modules/{$name}": ["type:thelia-module"]}}});
        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.3.0"),
            pkg("thelia/installer", "composer-plugin", "1.6"),
            pkg("acme/front", "thelia-frontoffice-template", "1.0"),
            pkg("acme/cheque", "thelia-module", "1.0")
        ]));
        let l = Layout::resolve(&root(), &lock, &thelia3, true, true).expect("thelia");
        assert_eq!(l.rel("acme/front"), Some("templates/frontOffice/front"));
        assert_eq!(l.rel("acme/cheque"), Some("vendor/acme/modules/cheque"));

        // Paths that disagree.
        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v1.12.0"),
            pkg("getkirby/composer-installer", "composer-plugin", "1.2.1"),
            pkg("acme/legacy", "kirby-plugin", "1.0")
        ]));
        let err = Layout::resolve(&root(), &lock, &allow, true, true).expect_err("differ");
        assert!(
            err.iter().any(|e| e.contains("place it differently")),
            "{err:?}"
        );
    }

    #[test]
    fn contested_removal_and_johnpbloch_directory() {
        let dir = tempfile::tempdir().expect("tmp");
        let vc = dir.path().join("vendor/composer");
        std::fs::create_dir_all(&vc).expect("mkdir");
        let installed = |extra: Value| {
            let mut packages = vec![
                json!({"name": "composer/installers", "version": "v2.3.0", "type": "composer-plugin", "install-path": "./installers"}),
                json!({"name": "thelia/installer", "version": "1.6", "type": "composer-plugin", "install-path": "../thelia/installer"}),
                json!({"name": "johnpbloch/wordpress-core-installer", "version": "2.0.0", "type": "composer-plugin", "install-path": "../johnpbloch/wordpress-core-installer"}),
            ];
            packages.push(extra);
            std::fs::write(
                vc.join("installed.json"),
                json!({"packages": packages, "dev": true, "dev-package-names": []}).to_string(),
            )
            .expect("write");
        };
        let plugins = || {
            vec![
                pkg("composer/installers", "composer-plugin", "v2.3.0"),
                pkg("thelia/installer", "composer-plugin", "1.6"),
                pkg(
                    "johnpbloch/wordpress-core-installer",
                    "composer-plugin",
                    "2.0.0",
                ),
            ]
        };
        let allow = json!({"config": {"allow-plugins": true}});
        // A template both installers take: composer/installers' uninstall
        // prints a line, thelia's does not.
        installed(
            json!({"name": "acme/front", "version": "1.0", "type": "thelia-frontoffice-template", "install-path": "../../templates/frontOffice/front"}),
        );
        let err = Layout::resolve(dir.path(), &lock_with(json!(plugins())), &allow, true, true)
            .expect_err("contested removal");
        assert!(
            err.iter().any(|e| e.contains("removal of acme/front")),
            "{err:?}"
        );

        // wordpress-core swapped for another at the same directory.
        installed(
            json!({"name": "acme/wpa", "version": "1.0", "type": "wordpress-core", "install-path": "../../wordpress"}),
        );
        let mut locked = plugins();
        locked.push(pkg("acme/wpb", "wordpress-core", "1.0"));
        let err = Layout::resolve(dir.path(), &lock_with(json!(locked)), &allow, true, true)
            .expect_err("johnpbloch");
        assert!(
            err.iter()
                .any(|e| e.contains("cannot share the same directory")),
            "{err:?}"
        );
    }

    #[test]
    fn relative_placement_of_a_path_package_with_binaries() {
        let allow = json!({"config": {"allow-plugins": true}});
        let mut kp = with(
            pkg("acme/kp", "kirby-plugin", "1.0"),
            "require",
            json!({"getkirby/composer-installer": "^1.2"}),
        );
        kp["dist"] = json!({"type": "path", "url": "packages/kp", "reference": "r"});
        kp["bin"] = json!(["bin/tool"]);
        let lock = lock_with(json!([
            pkg("getkirby/composer-installer", "composer-plugin", "1.2.1"),
            kp
        ]));
        let err = Layout::resolve(&root(), &lock, &allow, true, true).expect_err("bins");
        assert!(err.iter().any(|e| e.contains("realpath")), "{err:?}");
    }

    #[test]
    #[cfg(windows)]
    fn installers_one_x_is_handed_over_on_windows() {
        let allow = json!({"config": {"allow-plugins": true}});
        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v1.12.0"),
            pkg("a/b", "wordpress-plugin", "1.0")
        ]));
        let err = Layout::resolve(&root(), &lock, &allow, true, true).expect_err("windows");
        assert!(err.iter().any(|e| e.contains("1.x on Windows")), "{err:?}");
    }

    #[test]
    #[cfg(not(windows))]
    fn installers_one_x_alone_only() {
        let allow = json!({"config": {"allow-plugins": true}});
        let one = pkg("composer/installers", "composer-plugin", "v1.12.0");
        let lock = lock_with(json!([one.clone(), pkg("a/b", "wordpress-plugin", "1.0")]));
        let l = Layout::resolve(&root(), &lock, &allow, true, true).expect("1.12");
        assert_eq!(l.rel("a/b"), Some("wp-content/plugins/b"));
        // Another plugin could raise the first deprecation.
        let lock = lock_with(json!([
            one.clone(),
            pkg("symfony/flex", "composer-plugin", "v1.21.5"),
            pkg("a/b", "wordpress-plugin", "1.0")
        ]));
        let err = Layout::resolve(&root(), &lock, &allow, true, true).expect_err("flex");
        assert!(
            err.iter().any(|e| e.contains("next to symfony/flex")),
            "{err:?}"
        );
        // oomphinc on top of 1.x compiles its classes.
        let lock = lock_with(json!([
            one,
            pkg(
                "oomphinc/composer-installers-extender",
                "composer-plugin",
                "2.0.1"
            ),
            pkg("a/b", "wordpress-plugin", "1.0")
        ]));
        let err = Layout::resolve(&root(), &lock, &allow, true, true).expect_err("oomphinc");
        assert!(
            err.iter().any(|e| e.contains("composer/installers 1.x")),
            "{err:?}"
        );
    }

    #[test]
    fn target_dir_bins_and_install_dir_edge_cases() {
        let allow = json!({"config": {"allow-plugins": true}});
        let pluginkit = |name: &str| {
            with(
                pkg(name, "kirby-plugin", "1.0"),
                "require",
                json!({"getkirby/composer-installer": "^1.2"}),
            )
        };
        let kirby = pkg("getkirby/composer-installer", "composer-plugin", "1.2.1");
        // kirby does not append the target-dir Composer's autoloader cuts.
        let td = with(pluginkit("acme/shop"), "target-dir", json!("op"));
        let err = Layout::resolve(
            &root(),
            &lock_with(json!([kirby.clone(), td])),
            &allow,
            true,
            true,
        )
        .expect_err("target-dir");
        assert!(err.iter().any(|e| e.contains("target-dir `op`")), "{err:?}");
        // A binary in the vendor/ kirby removes.
        let bin = with(pluginkit("acme/tool"), "bin", json!(["vendor/bin/tool"]));
        let err = Layout::resolve(&root(), &lock_with(json!([kirby, bin])), &allow, true, true)
            .expect_err("bin");
        assert!(
            err.iter().any(|e| e.contains("binary `vendor/bin/tool`")),
            "{err:?}"
        );
        // `wordpress-install-dir: true` makes the installer throw.
        let wp =
            json!({"config": {"allow-plugins": true}, "extra": {"wordpress-install-dir": true}});
        let lock = lock_with(json!([
            pkg(
                "johnpbloch/wordpress-core-installer",
                "composer-plugin",
                "2.0.0"
            ),
            pkg("acme/wp", "wordpress-core", "1.0")
        ]));
        let err = Layout::resolve(&root(), &lock, &wp, true, true).expect_err("true");
        assert!(
            err.iter()
                .any(|e| e.contains("1 is an invalid WordPress install directory")),
            "{err:?}"
        );
        // oomphinc: a path that templates to "0" is LibraryInstaller's.
        let oo = json!({"config": {"allow-plugins": true}, "extra": {
            "installer-types": ["npm-asset"], "installer-paths": {"{$name}": ["type:npm-asset"]}}});
        let lock = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.3.0"),
            pkg(
                "oomphinc/composer-installers-extender",
                "composer-plugin",
                "2.0.1"
            ),
            pkg("acme/0", "npm-asset", "1.0")
        ]));
        let l = Layout::resolve(&root(), &lock, &oo, true, true).expect("zero");
        assert_eq!(l.rel("acme/0"), Some("vendor/acme/0"));
    }

    #[test]
    fn removals_follow_installed_json_when_paths_agree() {
        let dir = tempfile::tempdir().expect("tmp");
        let vc = dir.path().join("vendor/composer");
        std::fs::create_dir_all(&vc).expect("mkdir");
        let plugin_entry = json!({"name": "composer/installers", "version": "v2.3.0", "type": "composer-plugin", "install-path": "./installers"});
        std::fs::write(
            vc.join("installed.json"),
            json!({"packages": [
                plugin_entry,
                {"name": "gone/lib", "version": "1.0", "type": "library", "install-path": "../gone/lib"},
                {"name": "gone/legacy", "version": "1.0", "type": "library", "target-dir": "Acme/Legacy", "install-path": "../gone/legacy/Acme/Legacy"},
                {"name": "gone/plugin", "version": "1.0", "type": "wordpress-plugin", "install-path": "../../wp-content/plugins/plugin"},
                {"name": "moved/plugin", "version": "1.0", "type": "wordpress-plugin", "install-path": "../../old/plugin"},
                {"name": "gone/meta", "version": "1.0", "type": "metapackage", "install-path": null}
            ], "dev": true, "dev-package-names": []})
            .to_string(),
        )
        .expect("write");
        let lock = lock_with(json!([pkg(
            "composer/installers",
            "composer-plugin",
            "v2.3.0"
        )]));
        let manifest = json!({"config": {"allow-plugins": true}});
        let err = Layout::resolve(dir.path(), &lock, &manifest, true, true).expect_err("moved");
        assert!(
            err.iter()
                .any(|e| e.contains("moved/plugin") && e.contains("old/plugin")),
            "{err:?}"
        );

        // Without the moved package, the plan is accepted; a target-dir package
        // is deleted at vendor/<name> (getPackageBasePath), not at the sub-path.
        std::fs::write(
            vc.join("installed.json"),
            json!({"packages": [
                plugin_entry,
                {"name": "gone/lib", "version": "1.0", "type": "library", "install-path": "../gone/lib"},
                {"name": "gone/legacy", "version": "1.0", "type": "library", "target-dir": "Acme/Legacy", "install-path": "../gone/legacy/Acme/Legacy"},
                {"name": "gone/plugin", "version": "1.0", "type": "wordpress-plugin", "install-path": "../../wp-content/plugins/plugin"}
            ], "dev": true, "dev-package-names": []})
            .to_string(),
        )
        .expect("write");
        let l = Layout::resolve(dir.path(), &lock, &manifest, true, true).expect("layout");
        let removals: Vec<(String, PathBuf)> =
            l.removals().map(|(n, p)| (n.to_owned(), p)).collect();
        assert_eq!(
            removals,
            vec![
                (
                    "gone/legacy".to_owned(),
                    l.root().join("vendor/gone/legacy")
                ),
                ("gone/lib".to_owned(), l.root().join("vendor/gone/lib")),
                (
                    "gone/plugin".to_owned(),
                    l.root().join("wp-content/plugins/plugin")
                ),
            ]
        );
    }

    #[test]
    fn plugin_present_on_one_side_only_is_a_transition_for_composer() {
        let dir = tempfile::tempdir().expect("tmp");
        let vc = dir.path().join("vendor/composer");
        std::fs::create_dir_all(&vc).expect("mkdir");
        let manifest = json!({"config": {"allow-plugins": true}});
        let with_plugin = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.3.0"),
            pkg("a/wp", "wordpress-plugin", "1.0"),
        ]));
        let without_plugin = lock_with(json!([pkg("a/wp", "wordpress-plugin", "1.0")]));
        let libs_only = lock_with(json!([
            pkg("composer/installers", "composer-plugin", "v2.3.0"),
            pkg("a/lib", "library", "1.0"),
        ]));

        // Addition: installed.json without the plugin, lock with it, and an affected package.
        std::fs::write(
            vc.join("installed.json"),
            json!({"packages": [{"name": "a/wp", "version": "1.0", "type": "wordpress-plugin", "install-path": "../a/wp"}], "dev": true, "dev-package-names": []}).to_string(),
        )
        .expect("write");
        let err =
            Layout::resolve(dir.path(), &with_plugin, &manifest, true, true).expect_err("added");
        assert!(err[0].contains("added to an existing install"), "{err:?}");
        // Same addition without a package of a type taken by the plugin: nothing to transition.
        std::fs::write(
            vc.join("installed.json"),
            json!({"packages": [{"name": "a/lib", "version": "1.0", "type": "library", "install-path": "../a/lib"}], "dev": true, "dev-package-names": []}).to_string(),
        )
        .expect("write");
        assert!(Layout::resolve(dir.path(), &libs_only, &manifest, true, true).is_ok());

        // Removal: installed.json with the plugin and a package outside vendor/, lock without.
        std::fs::write(
            vc.join("installed.json"),
            json!({"packages": [
                {"name": "composer/installers", "version": "v2.3.0", "type": "composer-plugin", "install-path": "./installers"},
                {"name": "a/wp", "version": "1.0", "type": "wordpress-plugin", "install-path": "../../wp-content/plugins/wp"}
            ], "dev": true, "dev-package-names": []}).to_string(),
        )
        .expect("write");
        let err = Layout::resolve(dir.path(), &without_plugin, &manifest, true, true)
            .expect_err("removed");
        assert!(
            err[0].contains("removed from an existing install"),
            "{err:?}"
        );

        // --no-plugins: Composer ignores the plugin on both sides, everything in vendor/.
        let l =
            Layout::resolve(dir.path(), &with_plugin, &manifest, true, false).expect("no-plugins");
        assert_eq!(l.rel("a/wp"), Some("vendor/a/wp"));
        assert!(l.installers_tag.is_none());
        // No on-disk state: the lock decides (fresh install).
        std::fs::remove_file(vc.join("installed.json")).expect("rm");
        let l = Layout::resolve(dir.path(), &with_plugin, &manifest, true, true).expect("fresh");
        assert_eq!(l.rel("a/wp"), Some("wp-content/plugins/wp"));
    }
}
