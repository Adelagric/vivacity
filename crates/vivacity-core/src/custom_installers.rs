//! Plugins that register a custom installer: where a package type is laid
//! out (docs/plans/custom-installers.md). Each one is a placer — the
//! `supports($packageType)` of its installer and its `getInstallPath` —
//! read at the versions of [`READ_VERSIONS`]; another version, or a
//! configuration the plugin would reject (or turn into something PHP
//! coerces), is a named reason for Composer to take the command, before
//! anything is written.
//!
//! The placers only answer the path, relative to the project as the plugin
//! builds it; `layout` normalises it, refuses unsafe targets, arbitrates
//! between installers and plans the removals.

use crate::installers::{self, Table};
use serde_json::{Map, Value};

pub const KIRBY: &str = "getkirby/composer-installer";
pub const CRAFT: &str = "craftcms/plugin-installer";
pub const NEOS: &str = "neos/composer-plugin";
pub const DAVIDBARRATT: &str = "davidbarratt/custom-installer";
pub const JOHNPBLOCH: &str = "johnpbloch/wordpress-core-installer";
pub const THELIA: &str = "thelia/installer";
pub const CONTAO_COMPONENTS: &str = "contao-components/installer";
pub const OOMPHINC: &str = "oomphinc/composer-installers-extender";

/// The plugins, and the normalized versions whose installer source was read
/// (tags whose sources are identical to the one read, `git diff --quiet`).
pub const READ_VERSIONS: &[(&str, &[&str])] = &[
    (KIRBY, &["1.2.1.0"]),
    (CRAFT, &["1.6.0.0"]),
    (NEOS, &["2.1.1.0", "2.1.2.0", "2.1.3.0"]),
    (DAVIDBARRATT, &["1.1.0.0"]),
    (JOHNPBLOCH, &["2.0.0.0"]),
    (THELIA, &["1.6.0.0"]),
    (CONTAO_COMPONENTS, &["1.4.0.0", "1.4.1.0"]),
    (OOMPHINC, &["2.0.0.0", "2.0.1.0"]),
];

/// `None` when `version` of `plugin` was read; the reason otherwise.
pub fn version_issue(plugin: &str, version: &str) -> Option<String> {
    let (_, read) = READ_VERSIONS.iter().find(|(n, _)| *n == plugin)?;
    let normalized = crate::version::normalize_pretty(version).unwrap_or_default();
    if read.contains(&normalized.as_str()) {
        return None;
    }
    Some(format!(
        "{plugin} {version} is not emulated in this version (versions read: {})",
        read.iter()
            .map(|v| v.strip_suffix(".0").unwrap_or(v))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// What the placers read of the root package and the configuration.
pub struct Ctx<'a> {
    pub root_extra: Option<&'a Value>,
    pub root_type: &'a str,
    /// `config.vendor-dir`, project-relative (`Config::RELATIVE_PATHS`).
    pub vendor: &'a str,
}

/// An active custom installer.
#[derive(Debug, Clone)]
pub enum Placer {
    Kirby,
    Neos,
    DavidBarratt(DavidBarrattConfig),
    /// johnpbloch: roots/wordpress-core-installer's `getInstallPath`.
    Johnpbloch,
    /// thelia/installer: the Thelia 3 layout (`true`) or the Thelia 2 one.
    Thelia(bool),
    /// contao-components/installer with `extra.contao-component-dir`.
    ContaoComponents(String),
    /// oomphinc: the types of `extra.installer-types`, placed with
    /// composer/installers' `BaseInstaller`.
    Oomphinc(Vec<String>, &'static Table),
}

impl Placer {
    pub fn plugin(&self) -> &'static str {
        match self {
            Placer::Kirby => KIRBY,
            Placer::Neos => NEOS,
            Placer::DavidBarratt(_) => DAVIDBARRATT,
            Placer::Johnpbloch => JOHNPBLOCH,
            Placer::Thelia(_) => THELIA,
            Placer::ContaoComponents(_) => CONTAO_COMPONENTS,
            Placer::Oomphinc(..) => OOMPHINC,
        }
    }

    /// The installer's `supports($packageType)`.
    pub fn supports(&self, package_type: &str) -> bool {
        match self {
            // CmsInstaller / PluginInstaller::supports.
            Placer::Kirby => matches!(package_type, "kirby-cms" | "kirby-plugin"),
            // Installer::supports → getFlowPackageType !== false.
            Placer::Neos => flow_package_type(package_type).is_some(),
            // CustomInstaller::supports.
            Placer::DavidBarratt(c) => {
                !matches!(package_type, "metapackage" | "composer-plugin")
                    && c.types.iter().any(|(t, _)| t == package_type)
            }
            Placer::Johnpbloch => package_type == crate::layout::WP_CORE_TYPE,
            // TheliaInstaller::supports: array_key_exists in its locations.
            Placer::Thelia(v3) => thelia_location(*v3, package_type).is_some(),
            // `new LibraryInstaller($io, $composer, 'contao-component')`.
            Placer::ContaoComponents(_) => package_type == "contao-component",
            // Installer::supports: in_array in `installer-types`.
            Placer::Oomphinc(types, _) => types.iter().any(|t| t == package_type),
        }
    }

    /// The installer's `getInstallPath` for a package it supports: the path
    /// as the plugin builds it (relative to the project), `None` for
    /// LibraryInstaller's own path.
    pub fn install_path(
        &self,
        ctx: &Ctx<'_>,
        pkg: &Map<String, Value>,
    ) -> Result<Option<String>, String> {
        let pretty = pkg.get("name").and_then(Value::as_str).unwrap_or_default();
        let package_type = pkg.get("type").and_then(Value::as_str).unwrap_or("library");
        let extra = pkg.get("extra");
        match self {
            Placer::Kirby => kirby_path(ctx, pkg, pretty, package_type),
            Placer::Neos => neos_path(pkg, package_type).map(Some),
            Placer::DavidBarratt(c) => Ok(c.path(pkg, pretty, package_type)),
            Placer::Johnpbloch => {
                crate::layout::wp_core_install_dir(ctx.root_extra, pretty, extra, ctx.vendor)
                    .map(Some)
            }
            Placer::Thelia(v3) => thelia_path(*v3, pretty, package_type, extra).map(Some),
            Placer::ContaoComponents(dir) => Ok(Some(format!("{dir}/{}", php_basename(pretty)))),
            Placer::Oomphinc(..) => {
                match installers::custom_paths_only(ctx.root_extra, pretty, package_type, extra) {
                    Ok(Some(p)) => Ok(Some(p)),
                    // BaseInstaller: no `installer-paths` entry and no
                    // location → InvalidArgumentException.
                    Ok(None) => Err(format!(
                        "{OOMPHINC}: no extra.installer-paths entry for {pretty} ({package_type}) — Composer throws \"Package type \\\"{package_type}\\\" is not supported\""
                    )),
                    Err(e) => Err(format!("{OOMPHINC}: {pretty}: {e:?}")),
                }
            }
        }
    }
}

/// PHP `empty()`.
fn php_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !*b,
        Some(Value::String(s)) => s.is_empty() || s == "0",
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
    }
}

/// PHP `basename()` on a package name (no trailing slashes to strip).
fn php_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// `explode('/', $prettyName)[1]` (the second piece only), or the whole name.
fn after_slash(pretty: &str) -> &str {
    match pretty.split('/').nth(1) {
        Some(n) => n,
        None => pretty,
    }
}

// --- getkirby/composer-installer 1.2.1 ------------------------------------

/// `PluginInstaller::supportsPluginkit`: the package requires the installer.
pub fn kirby_pluginkit(pkg: &Map<String, Value>) -> bool {
    pkg.get("require")
        .and_then(Value::as_object)
        .is_some_and(|r| r.keys().any(|k| k.eq_ignore_ascii_case(KIRBY)))
}

/// `Installer::postInstall` runs for this package: a `kirby-cms`, or a
/// `kirby-plugin` that requires the installer (`PluginInstaller::postInstall`).
pub fn kirby_strips_vendor(pkg: &Map<String, Value>) -> bool {
    match pkg.get("type").and_then(Value::as_str) {
        Some("kirby-cms") => true,
        Some("kirby-plugin") => kirby_pluginkit(pkg),
        _ => false,
    }
}

fn kirby_path(
    ctx: &Ctx<'_>,
    pkg: &Map<String, Value>,
    pretty: &str,
    package_type: &str,
) -> Result<Option<String>, String> {
    let root = |key: &str| ctx.root_extra.and_then(|e| e.get(key));
    if package_type == "kirby-cms" {
        // CmsInstaller::getInstallPath: `isset($extra['kirby-cms-path'])`.
        let path = match root("kirby-cms-path") {
            None | Some(Value::Null) => "kirby".to_owned(),
            Some(Value::String(s)) => s.clone(),
            // `is_string($path) !== true`: LibraryInstaller's path.
            Some(_) => return Ok(None),
        };
        if path == ctx.vendor || path == "." {
            return Err(format!(
                "{KIRBY}: The path {path} is an unsafe installation directory for {pretty}."
            ));
        }
        return Ok(Some(path));
    }
    // PluginInstaller::getInstallPath.
    if !kirby_pluginkit(pkg) {
        return Ok(None);
    }
    let base = match root("kirby-plugin-path") {
        None | Some(Value::Null) => "site/plugins".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(format!("{KIRBY}: Invalid \"kirby-plugin-path\" option")),
    };
    let own = pkg.get("extra").and_then(|e| e.get("installer-name"));
    let name = if php_empty(own) {
        after_slash(pretty).to_owned()
    } else {
        match own {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "{KIRBY}: Invalid \"installer-name\" option in plugin {pretty}"
                ))
            }
        }
    };
    Ok(Some(format!("{base}/{name}")))
}

// --- neos/composer-plugin 2.1.1–2.1.3 ---------------------------------------

/// `Installer::getFlowPackageType`: the type after `typo3-flow-` or `neos-`.
/// An empty remainder would map to `Packages//…`: not taken.
fn flow_package_type(package_type: &str) -> Option<&str> {
    ["typo3-flow-", "neos-"]
        .iter()
        .find_map(|p| package_type.strip_prefix(p))
}

/// PHP `ucfirst` (ASCII, as PHP 8 does).
fn ucfirst(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// `Installer::isPackageKeyValid`: `/^[a-z0-9]+\.(?:[a-z0-9][\.a-z0-9]*)+$/i`.
fn neos_package_key_valid(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    let Some((head, tail)) = k.split_once('.') else {
        return false;
    };
    !head.is_empty()
        && head.bytes().all(|b| b.is_ascii_alphanumeric())
        && tail
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && tail.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
}

/// The first key of an autoload map, as `key()` gives it: a JSON object's
/// first key; a list or an empty map is refused (PHP would cast an int key
/// or pass `null` to `str_replace`, a deprecation Composer prints).
fn first_autoload_key<'a>(v: &'a Value, which: &str) -> Result<&'a str, String> {
    match v.as_object().and_then(|m| m.keys().next()) {
        Some(k) => Ok(k),
        None => Err(format!("{NEOS}: autoload.{which} is not a non-empty map")),
    }
}

/// `Installer::deriveFlowPackageName`.
fn neos_package_name(pkg: &Map<String, Value>) -> Result<String, String> {
    let extra = pkg.get("extra");
    let autoload = pkg.get("autoload");
    // `isset($extras['installer-name'])`.
    match extra.and_then(|e| e.get("installer-name")) {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => return Ok(s.clone()),
        Some(_) => return Err(format!("{NEOS}: extra.installer-name is not a string")),
    }
    let is_array = |v: Option<&Value>| matches!(v, Some(Value::Object(_) | Value::Array(_)));
    if let Some(psr0) = autoload
        .and_then(|a| a.get("psr-0"))
        .filter(|v| is_array(Some(v)))
    {
        return Ok(first_autoload_key(psr0, "psr-0")?.replace('\\', "."));
    }
    if let Some(psr4) = autoload
        .and_then(|a| a.get("psr-4"))
        .filter(|v| is_array(Some(v)))
    {
        return Ok(first_autoload_key(psr4, "psr-4")?
            .replace('\\', ".")
            .trim_end_matches('.')
            .to_owned());
    }
    let key = match extra.and_then(|e| e.get("neos")) {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => match m.get("package-key") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => neos_package_key_valid(s).then(|| s.clone()),
            Some(_) => return Err(format!("{NEOS}: extra.neos.package-key is not a string")),
        },
        Some(_) => return Err(format!("{NEOS}: extra.neos is not a map")),
    };
    let name = key.unwrap_or_else(|| {
        pkg.get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase()
    });
    Ok(name
        .split('/')
        .map(|part| part.split('-').map(ucfirst).collect::<Vec<_>>().join("."))
        .collect::<Vec<_>>()
        .join("."))
}

/// `Installer::getInstallPath`: the type's template, placeholders replaced
/// in `compact()` order.
fn neos_path(pkg: &Map<String, Value>, package_type: &str) -> Result<String, String> {
    let Some(flow_type) = flow_package_type(package_type).filter(|t| !t.is_empty()) else {
        return Err(format!(
            "{NEOS}: package type {package_type} has no Flow type"
        ));
    };
    let camel: String = flow_type.split('-').map(ucfirst).collect();
    let name = neos_package_name(pkg)?;
    let template = match flow_type {
        "plugin" => "Packages/Plugins/{flowPackageName}",
        "site" => "Packages/Sites/{flowPackageName}",
        "boilerplate" => "Packages/Boilerplates/{flowPackageName}",
        "build" => "Build/{flowPackageName}",
        "package" => "Packages/Application/{flowPackageName}",
        "package-collection" => "Packages/{flowPackageName}",
        _ => "Packages/{camelCasedType}/{flowPackageName}",
    };
    Ok(template
        .replace("{flowPackageType}", flow_type)
        .replace("{camelCasedType}", &camel)
        .replace("{flowPackageName}", &name))
}

// --- davidbarratt/custom-installer 1.1.0 ------------------------------------

/// `Configuration` (or `ConfigurationAlpha1`) of the root's
/// `extra.custom-installer`, in insertion order (a later spec of the same
/// key overrides an earlier one, keeping its place).
#[derive(Debug, Clone, Default)]
pub struct DavidBarrattConfig {
    types: Vec<(String, String)>,
    packages: Vec<(String, String)>,
}

fn set(list: &mut Vec<(String, String)>, key: &str, value: &str) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value.to_owned(),
        None => list.push((key.to_owned(), value.to_owned())),
    }
}

impl DavidBarrattConfig {
    /// `CustomInstaller::getPluginConfiguration`.
    pub fn from_root(root_extra: Option<&Value>) -> Result<DavidBarrattConfig, String> {
        let mut c = DavidBarrattConfig::default();
        let Some(v) = root_extra.and_then(|e| e.get("custom-installer")) else {
            return Ok(c);
        };
        let map = match v {
            Value::Null => return Ok(c),
            Value::Object(m) => m,
            _ => {
                return Err(format!(
                    "{DAVIDBARRATT}: extra.custom-installer is not a map (foreach on a scalar: a warning Composer turns into an exception)"
                ))
            }
        };
        let legacy = !map
            .values()
            .any(|v| matches!(v, Value::Array(_) | Value::Object(_)));
        for (key, val) in map {
            if legacy {
                // ConfigurationAlpha1: `$this->types = $extra['custom-installer']`.
                match val {
                    Value::String(pattern) => set(&mut c.types, key, pattern),
                    _ => {
                        return Err(format!(
                            "{DAVIDBARRATT}: extra.custom-installer.{key} is not a string"
                        ))
                    }
                }
                continue;
            }
            // Configuration::convert: `foreach ($specs as $spec)`.
            let specs: Vec<&Value> = match val {
                Value::Array(a) => a.iter().collect(),
                Value::Object(o) => o.values().collect(),
                _ => {
                    return Err(format!(
                        "{DAVIDBARRATT}: extra.custom-installer.{key} mixes the two configuration forms"
                    ))
                }
            };
            for spec in specs {
                let Some(spec) = spec.as_str().filter(|s| !s.contains('\n')) else {
                    return Err(format!(
                        "{DAVIDBARRATT}: a specification under `{key}` is not a string"
                    ));
                };
                match spec.strip_prefix("type:") {
                    Some(t) => set(&mut c.types, t, key),
                    None => set(&mut c.packages, spec, key),
                }
            }
        }
        Ok(c)
    }

    /// `CustomInstaller::getInstallPath`: the pattern by name, pretty name,
    /// then type; a falsy pattern is LibraryInstaller's path.
    fn path(&self, pkg: &Map<String, Value>, pretty: &str, package_type: &str) -> Option<String> {
        let lower = pretty.to_ascii_lowercase();
        let find = |list: &Vec<(String, String)>, k: &str| {
            list.iter()
                .find(|(key, _)| key == k)
                .map(|(_, p)| p.clone())
        };
        let pattern = find(&self.packages, &lower)
            .or_else(|| find(&self.packages, pretty))
            .or_else(|| find(&self.types, package_type))?;
        if pattern.is_empty() || pattern == "0" {
            return None;
        }
        let (vendor, name) = if pretty.contains('/') {
            let mut pieces = pretty.split('/');
            (
                pieces.next().unwrap_or_default().to_owned(),
                pieces.next().unwrap_or_default().to_owned(),
            )
        } else {
            (String::new(), pretty.to_owned())
        };
        let base = php_strtr(
            &pattern,
            &[
                ("{$type}", package_type),
                ("{$vendor}", &vendor),
                ("{$name}", &name),
            ],
        );
        let target = pkg
            .get("target-dir")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Some(if target.is_empty() || target == "0" {
            base
        } else {
            format!("{base}/{target}")
        })
    }
}

/// PHP `strtr($s, $pairs)`: one left-to-right pass, the longest key first.
fn php_strtr(s: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let hit = pairs
            .iter()
            .filter(|(k, _)| !k.is_empty() && rest.starts_with(k))
            .max_by_key(|(k, _)| k.len());
        match hit {
            Some((k, v)) => {
                out.push_str(v);
                i += k.len();
            }
            None => {
                let ch = rest.chars().next().unwrap_or_default();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

// --- thelia/installer 1.6 ----------------------------------------------------

/// `TheliaInstaller::rootRoutesModulesThroughInstallerPaths`: a
/// `type:thelia-module` matcher in the root's `installer-paths`.
pub fn thelia3(root_extra: Option<&Value>) -> Result<bool, String> {
    let paths = match root_extra.and_then(|e| e.get("installer-paths")) {
        None | Some(Value::Null) => return Ok(false),
        Some(Value::Object(m)) => m.values().collect::<Vec<_>>(),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(_) => return Err(format!("{THELIA}: extra.installer-paths is not a map")),
    };
    Ok(paths.iter().any(|matchers| match matchers {
        // `(array) $matchers`.
        Value::Array(a) => a.iter().any(|m| m.as_str() == Some("type:thelia-module")),
        Value::Object(o) => o.values().any(|m| m.as_str() == Some("type:thelia-module")),
        other => other.as_str() == Some("type:thelia-module"),
    }))
}

/// `THELIA3_LOCATIONS` / `THELIA2_LOCATIONS`.
fn thelia_location(v3: bool, package_type: &str) -> Option<&'static str> {
    match package_type {
        "thelia-frontoffice-template" => Some("templates/frontOffice/"),
        "thelia-backoffice-template" => Some("templates/backOffice/"),
        "thelia-email-template" => Some("templates/email/"),
        "thelia-pdf-template" => Some("templates/pdf/"),
        "thelia-local" => Some("local/"),
        "thelia-module" if !v3 => Some("local/modules/"),
        "thelia-core" if !v3 => Some(""),
        _ => None,
    }
}

/// `TheliaInstaller::getInstallPath`.
fn thelia_path(
    v3: bool,
    pretty: &str,
    package_type: &str,
    extra: Option<&Value>,
) -> Result<String, String> {
    let base = thelia_location(v3, package_type).unwrap_or_default();
    let own = extra.and_then(|e| e.get("installer-name"));
    let name = if php_empty(own) {
        after_slash(pretty).to_owned()
    } else {
        match own {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(format!(
                    "{THELIA}: extra.installer-name of {pretty} is not a string"
                ))
            }
        }
    };
    Ok(format!("{base}{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pkg(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn ctx(extra: &Value) -> Ctx<'_> {
        Ctx {
            root_extra: Some(extra),
            root_type: "project",
            vendor: "vendor",
        }
    }

    #[test]
    fn versions_read_and_the_others() {
        assert!(version_issue(KIRBY, "1.2.1").is_none());
        assert!(version_issue(NEOS, "2.1.2").is_none());
        assert!(version_issue(THELIA, "1.6").is_none());
        assert!(version_issue(KIRBY, "1.2.0")
            .unwrap()
            .contains("versions read: 1.2.1"));
        assert!(version_issue(NEOS, "dev-main").is_some());
    }

    #[test]
    fn kirby_paths() {
        let e = json!({});
        let cms = pkg(json!({"name": "getkirby/cms", "type": "kirby-cms"}));
        assert_eq!(
            Placer::Kirby
                .install_path(&ctx(&e), &cms)
                .unwrap()
                .as_deref(),
            Some("kirby")
        );
        let e2 = json!({"kirby-cms-path": false});
        assert_eq!(Placer::Kirby.install_path(&ctx(&e2), &cms).unwrap(), None);
        let e3 = json!({"kirby-cms-path": "."});
        assert!(Placer::Kirby.install_path(&ctx(&e3), &cms).is_err());
        let plug = pkg(
            json!({"name": "acme/shop", "type": "kirby-plugin", "require": {"getkirby/composer-installer": "^1"}}),
        );
        assert_eq!(
            Placer::Kirby
                .install_path(&ctx(&e), &plug)
                .unwrap()
                .as_deref(),
            Some("site/plugins/shop")
        );
        let legacy = pkg(json!({"name": "acme/old", "type": "kirby-plugin"}));
        assert_eq!(Placer::Kirby.install_path(&ctx(&e), &legacy).unwrap(), None);
        assert!(
            !kirby_strips_vendor(&legacy)
                && kirby_strips_vendor(&plug)
                && kirby_strips_vendor(&cms)
        );
    }

    #[test]
    fn neos_names() {
        let p = |v: Value| neos_path(&pkg(v.clone()), v["type"].as_str().unwrap()).unwrap();
        assert_eq!(
            p(
                json!({"name": "a/b", "type": "neos-site", "autoload": {"psr-4": {"Acme\\Site\\": "Classes/"}}})
            ),
            "Packages/Sites/Acme.Site"
        );
        assert_eq!(
            p(
                json!({"name": "a/b", "type": "neos-package", "autoload": {"psr-0": {"Acme\\Legacy\\": "Classes/"}}})
            ),
            "Packages/Application/Acme.Legacy."
        );
        assert_eq!(
            p(
                json!({"name": "acme/neos-tools", "type": "neos-package-collection", "extra": {"neos": {"package-key": "Acme.Tools"}}})
            ),
            "Packages/Acme.Tools"
        );
        assert_eq!(
            p(json!({"name": "acme/neos-build-ess", "type": "neos-build"})),
            "Build/Acme.Neos.Build.Ess"
        );
        assert_eq!(
            p(json!({"name": "acme/x", "type": "neos-my-extra"})),
            "Packages/MyExtra/Acme.X"
        );
        assert_eq!(
            p(
                json!({"name": "acme/x", "type": "typo3-flow-plugin", "extra": {"installer-name": "Acme.Plug"}})
            ),
            "Packages/Plugins/Acme.Plug"
        );
        // An invalid package key falls back to the name.
        assert_eq!(
            p(
                json!({"name": "acme/x-y", "type": "neos-package", "extra": {"neos": {"package-key": "nodot"}}})
            ),
            "Packages/Application/Acme.X.Y"
        );
        assert!(neos_path(
            &pkg(json!({"name": "a/b", "type": "neos-package", "autoload": {"psr-4": {}}})),
            "neos-package"
        )
        .is_err());
        assert!(!Placer::Neos.supports("library") && Placer::Neos.supports("neos-package"));
    }

    #[test]
    fn davidbarratt_both_forms() {
        let e = json!({"custom-installer": {"vendor/{$vendor}/{$name}/": ["type:library"], "web/{$type}/{$name}": ["acme/lang-help", "type:acme-addon"]}});
        let c = DavidBarrattConfig::from_root(Some(&e)).unwrap();
        let p = Placer::DavidBarratt(c.clone());
        assert!(p.supports("library") && p.supports("acme-addon") && !p.supports("acme-theme"));
        let help = pkg(json!({"name": "acme/lang-help", "type": "library"}));
        assert_eq!(
            c.path(&help, "acme/lang-help", "library").as_deref(),
            Some("web/library/lang-help")
        );
        let lib = pkg(json!({"name": "acme/lib", "type": "library"}));
        assert_eq!(
            c.path(&lib, "acme/lib", "library").as_deref(),
            Some("vendor/acme/lib/")
        );
        let addon = pkg(json!({"name": "acme/addon-x", "type": "acme-addon", "target-dir": "Sub"}));
        assert_eq!(
            c.path(&addon, "acme/addon-x", "acme-addon").as_deref(),
            Some("web/acme-addon/addon-x/Sub")
        );
        let legacy = DavidBarrattConfig::from_root(Some(
            &json!({"custom-installer": {"acme-addon": "legacy/{$name}"}}),
        ))
        .unwrap();
        assert_eq!(
            legacy.path(&addon, "acme/addon-x", "acme-addon").as_deref(),
            Some("legacy/addon-x/Sub")
        );
        assert!(DavidBarrattConfig::from_root(Some(
            &json!({"custom-installer": {"a": ["type:x"], "b": "y"}})
        ))
        .is_err());
        assert!(DavidBarrattConfig::from_root(Some(&json!({"custom-installer": "x"}))).is_err());
    }

    #[test]
    fn strtr_is_one_pass() {
        assert_eq!(
            php_strtr(
                "{$name}{$vendor}",
                &[("{$name}", "{$vendor}"), ("{$vendor}", "v")]
            ),
            "{$vendor}v"
        );
    }

    #[test]
    fn thelia_layouts() {
        let v3 = json!({"installer-paths": {"vendor/{$vendor}/modules/{$name}": ["type:thelia-module"]}});
        assert!(thelia3(Some(&v3)).unwrap());
        assert!(!thelia3(Some(&json!({}))).unwrap());
        assert!(
            Placer::Thelia(false).supports("thelia-module")
                && !Placer::Thelia(true).supports("thelia-module")
        );
        assert_eq!(
            thelia_path(
                true,
                "acme/front",
                "thelia-frontoffice-template",
                Some(&json!({"installer-name": "front"}))
            )
            .unwrap(),
            "templates/frontOffice/front"
        );
        assert_eq!(
            thelia_path(false, "acme/core", "thelia-core", None).unwrap(),
            "core"
        );
    }

    #[test]
    fn contao_component_basename() {
        let p = Placer::ContaoComponents("assets".into());
        let e = json!({});
        assert_eq!(
            p.install_path(
                &ctx(&e),
                &pkg(json!({"name": "contao-components/ace", "type": "contao-component"}))
            )
            .unwrap()
            .as_deref(),
            Some("assets/ace")
        );
    }
}
