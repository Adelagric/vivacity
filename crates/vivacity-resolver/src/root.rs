//! The root package as `RootPackageLoader::load` builds it
//! (docs/reference/RootPackageLoader.php): version (composer.json,
//! COMPOSER_ROOT_VERSION, git, otherwise `1.0.0+no-version-set`), links,
//! `minimum-stability`, `prefer-stable`, and the three extractions from the
//! require constraints: aliases (`X as Y`), stability flags (`@dev`,
//! branches), references (`#sha`).

use crate::constraint::parse_constraints;
use crate::loader;
use crate::package::{Links, Origin, Package};
use crate::version::{
    self, group, normalize, regex, stability_rank, VersionError, STABILITIES_REGEX,
};
use pcre2::bytes::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

pub const DEFAULT_PRETTY_VERSION: &str = "1.0.0+no-version-set";

/// `RootPackage::getAliases()`: an alias declared in a require.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootAlias {
    pub package: String,
    pub version: String,
    pub alias: String,
    pub alias_normalized: String,
}

#[derive(Debug, Clone)]
pub struct RootPackage {
    pub package: Package,
    /// `RootAliasPackage`: (normalized alias, pretty alias) if `extra.branch-alias` applies.
    pub branch_alias: Option<(String, String)>,
    pub minimum_stability: String,
    pub prefer_stable: bool,
    /// name -> stability rank (`BasePackage::STABILITIES`).
    pub stability_flags: BTreeMap<String, i32>,
    pub aliases: Vec<RootAlias>,
    /// name -> git reference.
    pub references: BTreeMap<String, String>,
    /// `config.platform`.
    pub platform_overrides: Map<String, Value>,
    pub manifest: Value,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct RootError(pub String);

/// What a dry run changes on the root package in memory instead of in
/// composer.json.
#[derive(Debug, Clone, Default)]
pub struct RootPatch {
    /// `(name, constraint)` to add (`require` or, with `dev`, `require-dev`).
    pub requirements: Vec<(String, String)>,
    pub dev: bool,
    /// `(from require-dev?, name as typed)` to remove.
    pub removals: Vec<(bool, String)>,
}

impl RootPackage {
    /// Loads composer.json (already parsed) with the root version guessed by
    /// vivacity-core (same rules as RootPackageLoader + VersionGuesser).
    pub fn load(manifest: &Value, project_dir: &Path) -> Result<RootPackage, RootError> {
        if let Some(err) = manifest_error(manifest) {
            return Err(RootError(err));
        }
        let mut config = manifest
            .as_object()
            .cloned()
            .ok_or_else(|| RootError("composer.json is not an object".into()))?;
        if !config.contains_key("name") {
            config.insert("name".into(), Value::String("__root__".into()));
        }
        let mut auto_versioned = false;
        if !config.contains_key("version") {
            let rv = vivacity_core::root_version::detect(manifest, project_dir);
            if rv.pretty_version == vivacity_core::root_version::DEFAULT_PRETTY_VERSION {
                config.insert("version".into(), Value::String("1.0.0".into()));
                auto_versioned = true;
            } else {
                config.insert("version".into(), Value::String(rv.pretty_version.clone()));
                config.insert(
                    "version_normalized".into(),
                    Value::String(rv.version.clone()),
                );
                if let Some(commit) = rv.reference {
                    let r = serde_json::json!({"type": "", "url": "", "reference": commit});
                    config.insert("source".into(), r.clone());
                    config.insert("dist".into(), r);
                }
            }
        }
        let value = Value::Object(config.clone());
        let (mut package, alias) =
            loader::load(&value, Origin::Root, false).map_err(|e| RootError(e.0))?;
        if auto_versioned {
            package.pretty_version = DEFAULT_PRETTY_VERSION.to_owned();
        }
        let minimum_stability = match config.get("minimum-stability").and_then(Value::as_str) {
            Some(s) => normalize_stability(s)?,
            None => "stable".to_owned(),
        };
        let mut aliases = Vec::new();
        let mut stability_flags = BTreeMap::new();
        let mut references = BTreeMap::new();
        for links in [&package.requires, &package.dev_requires] {
            let map: Vec<(String, String)> = links
                .iter()
                // `$link->getConstraint()->getPrettyString()`: for
                // `self.version`, the root version.
                .map(|l| {
                    let pretty = if l.pretty_constraint == "self.version" {
                        // Pretty version at parseLinks time (before
                        // `setPrettyVersion('1.0.0+no-version-set')`).
                        if auto_versioned {
                            "1.0.0".to_owned()
                        } else {
                            package.pretty_version.clone()
                        }
                    } else {
                        l.pretty_constraint.clone()
                    };
                    (l.target.clone(), pretty)
                })
                .collect();
            extract_aliases(&map, &mut aliases)?;
            extract_stability_flags(&map, &minimum_stability, &mut stability_flags);
            extract_references(&map, &mut references);
            if map.iter().any(|(n, _)| *n == package.name) {
                return Err(RootError(format!(
                    "Root package '{}' cannot require itself in its composer.json\nDid you accidentally name your root package after an external package?",
                    package.pretty_name
                )));
            }
        }
        let prefer_stable = config
            .get("prefer-stable")
            .map(|v| match v {
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_f64() != Some(0.0),
                Value::String(s) => !(s.is_empty() || s == "0"),
                Value::Null => false,
                _ => true,
            })
            .unwrap_or(false);
        let platform_overrides = config
            .get("config")
            .and_then(|c| c.get("platform"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        Ok(RootPackage {
            package,
            branch_alias: alias,
            minimum_stability,
            prefer_stable,
            stability_flags,
            aliases,
            references,
            platform_overrides,
            manifest: manifest.clone(),
        })
    }

    /// `require` + `require-dev` links (`array_merge`: dev-requires
    /// overwrite a duplicate target, at its position).
    /// The in-memory root patch of a dry run (`RequireCommand`,
    /// `RemoveCommand`): the new links are `array_merge`d onto the key's
    /// links (a duplicate key keeps its position), removed from the other
    /// key by their name, and the references / stability flags are
    /// extended; aliases are not re-extracted (the reference does not).
    pub fn apply_patch(&mut self, patch: &RootPatch) -> Result<(), RootError> {
        if !patch.requirements.is_empty() {
            let map: Map<String, Value> = patch
                .requirements
                .iter()
                .map(|(n, c)| (n.clone(), Value::String(c.clone())))
                .collect();
            let kind = if patch.dev {
                crate::package::LinkType::DevRequire
            } else {
                crate::package::LinkType::Require
            };
            let new_links = loader::parse_links(
                &self.package.name,
                &self.package.pretty_version,
                kind,
                Some(&Value::Object(map)),
                false,
            )
            .map_err(|e| RootError(e.0))?;
            let (own_links, other_links) = if kind == crate::package::LinkType::DevRequire {
                (&mut self.package.dev_requires, &mut self.package.requires)
            } else {
                (&mut self.package.requires, &mut self.package.dev_requires)
            };
            for l in new_links.iter() {
                own_links.insert(l.clone());
            }
            for (name, _) in &patch.requirements {
                other_links.remove(name);
            }
            let pairs: Vec<(String, String)> = patch.requirements.clone();
            extract_references(&pairs, &mut self.references);
            extract_stability_flags(&pairs, &self.minimum_stability, &mut self.stability_flags);
        }
        // `RemoveCommand`: `unset($links[$type][$name])` with the name as
        // typed against lowercase keys.
        for (dev, name) in &patch.removals {
            if *dev {
                self.package.dev_requires.remove(name);
            } else {
                self.package.requires.remove(name);
            }
        }
        Ok(())
    }

    pub fn all_requires(&self) -> Links {
        let mut out = self.package.requires.clone();
        for l in self.package.dev_requires.iter() {
            out.insert(l.clone());
        }
        out
    }
}

/// `VersionParser::normalizeStability`.
/// The refusals `RootPackageLoader::load` raises from the manifest alone, in
/// its order: the root package's own name, then — after the links are parsed,
/// which changes nothing for a raw manifest — a package that requires itself,
/// then the name of every link of the five `BasePackage::$supportedLinkTypes`.
///
/// Composer refuses these for EVERY command, because `Factory::createComposer`
/// loads the root package; vivacity only built a `RootPackage` for a
/// resolution, so `install` and `dump-autoload` used to accept them. They call
/// this directly — the checks that need a parsed manifest (a malformed alias,
/// an unknown `minimum-stability`, an unparsable constraint) stay inside
/// `load`, since reaching them means guessing the root version, i.e. running
/// git, on the install hot path.
///
/// The message is returned as Composer words it, prefixes included. vivacity
/// prints it without Symfony Console's box, as it already does for path
/// repositories: the box is wrapped to the terminal width, followed by the
/// command synopsis, and sometimes chained.
pub fn manifest_error(manifest: &Value) -> Option<String> {
    use crate::lockfile::package_naming_error;
    let config = manifest.as_object()?;
    // `if (!isset($config['name'])) { '__root__' } elseif ($err = …)`.
    //
    // The schema runs first, and it holds the same shape pattern for `name`
    // but CASE SENSITIVELY (`^[a-z0-9]([_.-]?[a-z0-9]+)*/…$`, measured in
    // `res/composer-schema.json`). So a root name with an uppercase letter or a
    // bad shape is refused with the schema's message, which vivacity does not
    // reproduce — only what the schema lets through is ours to report, i.e. the
    // `.json` suffix and the reserved names.
    let name = config.get("name").and_then(Value::as_str);
    if let Some(name) = name.filter(|n| schema_name_shape(n)) {
        if let Some(err) = package_naming_error(name, false) {
            return Some(format!("Your package name {err}"));
        }
    }
    // `if (isset($links[$config['name']]))`, for require then require-dev. The
    // targets are lowercased by `parseLinks`; the name passed the check above,
    // so it is already lowercase ASCII here.
    if let Some(name) = name {
        for link_type in ["require", "require-dev"] {
            let Some(links) = config.get(link_type).and_then(Value::as_object) else {
                continue;
            };
            if links.keys().any(|k| k.to_ascii_lowercase() == name) {
                return Some(format!(
                    "Root package '{name}' cannot require itself in its composer.json\nDid you accidentally name your root package after an external package?"
                ));
            }
        }
    }
    // `foreach (array_keys(BasePackage::$supportedLinkTypes) as $linkType)`,
    // in that order, first error wins.
    for link_type in ["require", "conflict", "provide", "replace", "require-dev"] {
        let Some(links) = config.get(link_type).and_then(Value::as_object) else {
            continue;
        };
        for target in links.keys() {
            if let Some(err) = package_naming_error(target, true) {
                return Some(format!("{link_type}.{err}"));
            }
        }
    }
    None
}

/// `res/composer-schema.json`'s pattern for `name`: the loader's shape rule
/// without its `i` flag. A name that fails it never reaches the loader, since
/// `Factory::createComposer` validates the schema first.
fn schema_name_shape(name: &str) -> bool {
    static SHAPE: OnceLock<pcre2::bytes::Regex> = OnceLock::new();
    let re = crate::version::regex(
        &SHAPE,
        r"^[a-z0-9](?:[_.-]?[a-z0-9]++)*+/[a-z0-9](?:(?:[_.]|-{1,2})?[a-z0-9]++)*+\z",
        false,
    );
    re.is_match(name.as_bytes()).unwrap_or(false)
}

pub fn normalize_stability(s: &str) -> Result<String, RootError> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "stable" | "beta" | "alpha" | "dev" => Ok(lower),
        "rc" => Ok("RC".to_owned()),
        _ => Err(RootError(format!(
            "Invalid stability string \"{s}\", expected one of stable, RC, beta, alpha or dev"
        ))),
    }
}

/// `RootPackageLoader::extractAliases`.
fn extract_aliases(
    requires: &[(String, String)],
    aliases: &mut Vec<RootAlias>,
) -> Result<(), RootError> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        &RE,
        r"(?:^|\| *|, *)([^,\s#|]+)(?:#[^ ]+)? +as +([^,\s|]+)(?:$| *\|| *,)",
        false,
    );
    for (name, req) in requires {
        if let Ok(Some(caps)) = re.captures(req.as_bytes()) {
            let v = group(&caps, 1);
            let a = group(&caps, 2);
            aliases.push(RootAlias {
                package: name.to_lowercase(),
                version: normalize(v, Some(req)).map_err(|e: VersionError| RootError(e.0))?,
                alias: a.to_owned(),
                alias_normalized: normalize(a, Some(req))
                    .map_err(|e: VersionError| RootError(e.0))?,
            });
        } else if req.contains(" as ") {
            return Err(RootError(format!(
                "Invalid alias definition in \"{name}\": \"{req}\". Aliases should be in the form \"exact-version as other-exact-version\"."
            )));
        }
    }
    Ok(())
}

/// `preg_split` of the constraints into "and" pieces across the "or"s.
fn split_constraints(req: &str) -> Vec<String> {
    static OR: OnceLock<Regex> = OnceLock::new();
    static AND: OnceLock<Regex> = OnceLock::new();
    let or = regex(&OR, r"\s*\|\|?\s*", false);
    let and = regex(
        &AND,
        r"(?<!^|as|[=>< ,]) *(?<!-)[, ](?!-) *(?!,|as|$)",
        false,
    );
    let mut out = Vec::new();
    for part in split(or, req.trim()) {
        out.extend(split(and, &part));
    }
    out
}

fn split(re: &Regex, subject: &str) -> Vec<String> {
    let bytes = subject.as_bytes();
    let mut out = Vec::new();
    let mut last = 0;
    for m in re.find_iter(bytes).flatten() {
        if m.start() == m.end() && m.start() == last && last == bytes.len() {
            break;
        }
        out.push(subject[last..m.start()].to_owned());
        last = m.end();
    }
    out.push(subject[last..].to_owned());
    out
}

/// composer-merge-plugin's `ExtraPackage::mergeStabilityFlags` for one
/// merged file: `StabilityFlags::extractAll` over the file's own
/// requirements (an explicit `@flag`, the most unstable across the
/// constraint's parts, else the parsed stability when it is dev and not
/// more stable than the minimum), each raised to at least the root's
/// current flag, then `array_merge`d over the root's flags.
pub fn merge_plugin_stability_flags(
    flags: &mut BTreeMap<String, i32>,
    minimum_stability: &str,
    requires: &[(String, String)],
) {
    static AT: OnceLock<Regex> = OnceLock::new();
    static AS: OnceLock<Regex> = OnceLock::new();
    let at = regex(&AT, &format!("^[^@]*?@({STABILITIES_REGEX})$"), true);
    let as_re = regex(&AS, r"^([^,\s@]+) as .+$", false);
    let minimum = stability_rank(minimum_stability);
    for (req_name, pretty) in requires {
        let name = req_name.to_lowercase();
        let mut explicit: Option<i32> = None;
        for c in split_constraints(pretty) {
            if let Ok(Some(caps)) = at.captures(c.as_bytes()) {
                let Ok(stab) = normalize_stability(group(&caps, 1)) else {
                    continue;
                };
                let rank = stability_rank(&stab);
                explicit = Some(explicit.map_or(rank, |e| e.max(rank)));
            }
        }
        let stability = match explicit {
            Some(e) => Some(e),
            None => {
                // Drop aliasing if used.
                let v = match as_re.captures(pretty.as_bytes()) {
                    Ok(Some(caps)) => group(&caps, 1).to_owned(),
                    _ => pretty.clone(),
                };
                let rank = stability_rank(version::parse_stability(&v));
                if rank == stability_rank("stable") || minimum > rank {
                    None
                } else {
                    Some(rank)
                }
            }
        };
        if let Some(st) = stability {
            let current = flags.get(&name).copied();
            flags.insert(name, current.map_or(st, |c| c.max(st)));
        }
    }
}

/// `RootPackageLoader::extractStabilityFlags`.
pub fn extract_stability_flags(
    requires: &[(String, String)],
    minimum_stability: &str,
    flags: &mut BTreeMap<String, i32>,
) {
    static AT: OnceLock<Regex> = OnceLock::new();
    static AS: OnceLock<Regex> = OnceLock::new();
    static PLAIN: OnceLock<Regex> = OnceLock::new();
    let at = regex(&AT, &format!("^[^@]*?@({STABILITIES_REGEX})$"), true);
    let as_re = regex(&AS, r"^([^,\s@]+) as .+$", false);
    let plain = regex(&PLAIN, r"^[^,\s@]+$", false);
    let minimum = stability_rank(minimum_stability);
    for (req_name, req) in requires {
        let constraints = split_constraints(req);
        let mut matched = false;
        for c in &constraints {
            if let Ok(Some(caps)) = at.captures(c.as_bytes()) {
                let name = req_name.to_lowercase();
                let Ok(stab) = normalize_stability(group(&caps, 1)) else {
                    continue;
                };
                let rank = stability_rank(&stab);
                if flags.get(&name).is_some_and(|f| *f > rank) {
                    continue;
                }
                flags.insert(name, rank);
                matched = true;
            }
        }
        if matched {
            continue;
        }
        for c in &constraints {
            let stripped = match as_re.captures(c.as_bytes()) {
                Ok(Some(caps)) => group(&caps, 1).to_owned(),
                _ => c.clone(),
            };
            if plain.is_match(stripped.as_bytes()).unwrap_or(false) {
                let stability = version::parse_stability(&stripped);
                if stability != "stable" {
                    let name = req_name.to_lowercase();
                    let rank = stability_rank(stability);
                    if flags.get(&name).is_some_and(|f| *f > rank) || minimum > rank {
                        continue;
                    }
                    flags.insert(name, rank);
                }
            }
        }
    }
}

/// `RootPackageLoader::extractReferences`.
pub fn extract_references(
    requires: &[(String, String)],
    references: &mut BTreeMap<String, String>,
) {
    static AS: OnceLock<Regex> = OnceLock::new();
    static REF: OnceLock<Regex> = OnceLock::new();
    let as_re = regex(&AS, r"^([^,\s@]+) as .+$", false);
    let re = regex(&REF, r"^[^,\s@]+?#([a-f0-9]+)$", false);
    for (name, req) in requires {
        let stripped = match as_re.captures(req.as_bytes()) {
            Ok(Some(caps)) => group(&caps, 1).to_owned(),
            _ => req.clone(),
        };
        if let Ok(Some(caps)) = re.captures(stripped.as_bytes()) {
            if version::parse_stability(&stripped) == "dev" {
                references.insert(name.to_lowercase(), group(&caps, 1).to_owned());
            }
        }
    }
}

/// Constraint of a root require (with `as`: the source part).
pub fn root_constraint(pretty: &str) -> Result<crate::constraint::Constraint, VersionError> {
    Ok(parse_constraints(pretty)?.constraint)
}

#[cfg(test)]
mod tests {
    #[test]
    fn merge_plugin_flags_follow_the_plugin() {
        use super::merge_plugin_stability_flags as flags_of;
        use std::collections::BTreeMap;
        let req = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(n, c)| ((*n).to_owned(), (*c).to_owned()))
                .collect()
        };
        // Explicit flag: the most unstable part of the constraint wins,
        // whatever the minimum stability; the name is lowercased.
        let mut f = BTreeMap::new();
        flags_of(
            &mut f,
            "stable",
            &req(&[("Acme/Lib", "^1.0@beta || ^2.0@RC")]),
        );
        assert_eq!(f.get("acme/lib"), Some(&10));
        // Parsed dev stability counts unless the minimum is more unstable
        // than it (`$this->minimumStability > $stability`), and never a
        // stable one; unlike the loader, no "single plain token" check.
        let mut f = BTreeMap::new();
        flags_of(
            &mut f,
            "stable",
            &req(&[("a/b", "dev-main || ^1.0"), ("c/d", "^1.0")]),
        );
        assert_eq!(f.get("a/b"), Some(&20));
        assert!(!f.contains_key("c/d"));
        let mut f = BTreeMap::new();
        flags_of(&mut f, "dev", &req(&[("a/b", "1.0.0-beta")]));
        assert!(f.is_empty(), "beta is more stable than the dev minimum");
        // `max($stability, current)`: an existing flag is never lowered,
        // and a requirement without a flag leaves it untouched.
        let mut f: BTreeMap<String, i32> = [("a/b".to_owned(), 20)].into();
        flags_of(
            &mut f,
            "stable",
            &req(&[("a/b", "^1.0@beta"), ("a/b", "^1.0")]),
        );
        assert_eq!(f.get("a/b"), Some(&20));
        // The alias is dropped before parsing.
        let mut f = BTreeMap::new();
        flags_of(&mut f, "stable", &req(&[("a/b", "dev-main as 1.0.0")]));
        assert_eq!(f.get("a/b"), Some(&20));
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_flags_aliases_references() {
        let reqs = vec![
            ("a/b".to_owned(), "^1.0@beta".to_owned()),
            ("c/d".to_owned(), "dev-main as 1.0.x-dev".to_owned()),
            ("e/f".to_owned(), "dev-main#abcdef".to_owned()),
            ("g/h".to_owned(), "1.x-dev || ^2.0".to_owned()),
            ("i/j".to_owned(), "^1.0".to_owned()),
        ];
        let mut flags = BTreeMap::new();
        extract_stability_flags(&reqs, "stable", &mut flags);
        assert_eq!(flags.get("a/b"), Some(&10));
        assert_eq!(flags.get("c/d"), Some(&20));
        assert_eq!(flags.get("e/f"), Some(&20));
        assert_eq!(flags.get("g/h"), Some(&20));
        assert_eq!(flags.get("i/j"), None);
        let mut aliases = Vec::new();
        extract_aliases(&reqs, &mut aliases).unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].alias_normalized, "1.0.9999999.9999999-dev");
        let mut refs = BTreeMap::new();
        extract_references(&reqs, &mut refs);
        assert_eq!(refs.get("e/f").map(String::as_str), Some("abcdef"));
    }

    #[test]
    fn loads_root_without_git() {
        let m = json!({"name": "acme/app", "require": {"php": "^8.1", "monolog/monolog": "^3"}, "minimum-stability": "RC", "prefer-stable": true});
        let r = RootPackage::load(&m, Path::new("/nonexistent-vivacity")).unwrap();
        assert_eq!(r.package.pretty_version, DEFAULT_PRETTY_VERSION);
        assert_eq!(r.package.version, "1.0.0.0");
        assert_eq!(r.minimum_stability, "RC");
        assert!(r.prefer_stable);
        assert_eq!(r.package.requires.len(), 2);
    }

    #[test]
    fn manifest_error_follows_the_loader() {
        use serde_json::json;
        // A real manifest is accepted, platform packages included.
        assert_eq!(
            manifest_error(&json!({
                "name": "acme/app",
                "require": {"php": ">=8.1", "ext-json": "*", "psr/log": "^3", "a-b/c--d": "*"},
                "require-dev": {"phpunit/phpunit": "^11"},
            })),
            None
        );
        // The root name: only what the schema lets through is ours to report.
        assert_eq!(
            manifest_error(&json!({"name": "acme/thing.json"})).as_deref(),
            Some("Your package name acme/thing.json is invalid, package names can not end in .json, consider renaming it or perhaps using a -json suffix instead.")
        );
        assert!(manifest_error(&json!({"name": "con/thing"})).is_some());
        // An uppercase or malformed root name is refused by the schema first,
        // with a message vivacity does not reproduce: silence here is
        // deliberate, not an oversight.
        assert_eq!(manifest_error(&json!({"name": "acme/ThingBaz"})), None);
        assert_eq!(manifest_error(&json!({"name": "acme"})), None);
        // No name at all is `__root__`, never an error.
        assert_eq!(manifest_error(&json!({"require": {"a/b": "*"}})), None);
        // A package that requires itself, from either section.
        for section in ["require", "require-dev"] {
            let m = json!({"name": "acme/app", section: {"acme/app": "*"}});
            assert!(
                manifest_error(&m).is_some_and(|e| e
                    .starts_with("Root package 'acme/app' cannot require itself")
                    && e.contains("\nDid you accidentally")),
                "{section}"
            );
        }
        // The five link types, each prefixed with its own name.
        for section in ["require", "conflict", "provide", "replace", "require-dev"] {
            let m = json!({"name": "acme/app", section: {"Acme/Thing": "^1"}});
            assert_eq!(
                manifest_error(&m).as_deref(),
                Some(&*format!("{section}.Acme/Thing is invalid, it should not contain uppercase characters. Please use acme/thing instead.")),
                "{section}"
            );
        }
        // Composer's order: the root name first, then self-require, then the
        // link names in the order of `BasePackage::$supportedLinkTypes`.
        let all = json!({
            "name": "acme/thing.json",
            "require": {"acme/thing.json": "*", "Bad/Name": "*"},
            "conflict": {"Other/Name": "*"},
        });
        assert!(manifest_error(&all).is_some_and(|e| e.starts_with("Your package name")));
        let links = json!({"require": {"Bad/Name": "*"}, "conflict": {"Other/Name": "*"}});
        assert!(manifest_error(&links).is_some_and(|e| e.starts_with("require.Bad/Name")));
    }
}
