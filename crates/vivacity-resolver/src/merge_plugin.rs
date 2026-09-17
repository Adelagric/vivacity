//! Emulation of `wikimedia/composer-merge-plugin` v2.1.0 (v2.0.1 alike;
//! docs/reference/plugins/composer-merge-plugin/, MIT) for an `install`
//! and an autoload dump: the `composer.json` files named by
//! `extra.merge-plugin.include` / `require` (PHP `glob()` patterns, no
//! flags) are merged into the root package — `require` / `require-dev`,
//! `autoload` / `autoload-dev` (paths re-based on each file's directory),
//! `conflict` / `replace` / `provide`, `extra` with `merge-extra` — in the
//! order and with the duplicate rules of `ExtraPackage::mergeInto` /
//! `mergeDevInto`, recursively (`recurse`).
//!
//! What vivacity does not do: the plugin's implicit `composer update` of
//! the merged requirements on the run that installs the plugin itself
//! (`onPostPackageInstall` → `onPostInstallOrUpdate`), which rewrites
//! `composer.lock` against the live repositories (DECISIONS, 2026-09-17);
//! `merge-scripts`, suggests, aliases and references, the prepended
//! repositories (resolution only).
//!
//! The merged `require` links are structured: a duplicate key without
//! `ignore-duplicates` or `replace` is Composer's conjunctive
//! `MultiConstraint` (`mergeConstraints`, with the `Intervals::isSubsetOf`
//! short-cuts), pretty `"<old>, <new>"`. The textual `require` of the
//! merged manifest carries an equivalent constraint text (the conjunction
//! distributed over `||`), for the consumers that parse text (the
//! platform check); the lock check takes the structured links.

use crate::constraint::{parse_constraints, Constraint};
use crate::intervals::is_subset_of;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;
use vivacity_core::glob::glob_plain;
use vivacity_core::phparray::{array_merge, array_merge_recursive, merge_deep};

pub const PLUGIN_NAME: &str = "wikimedia/composer-merge-plugin";

/// `PluginState::loadSettings`.
#[derive(Debug, Clone)]
pub struct Settings {
    pub includes: Vec<String>,
    pub requires: Vec<String>,
    pub recurse: bool,
    pub replace: bool,
    pub ignore_duplicates: bool,
    pub merge_dev: bool,
    pub merge_extra: bool,
    pub merge_extra_deep: bool,
    pub merge_replace: bool,
}

fn patterns(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn flag(cfg: Option<&Value>, key: &str, default: bool) -> bool {
    // `(bool)$config[$key]`: PHP truthiness.
    match cfg.and_then(|c| c.get(key)) {
        None | Some(Value::Null) => default,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !(s.is_empty() || s == "0"),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

impl Settings {
    pub fn from_manifest(manifest: &Value) -> Settings {
        let cfg = manifest.get("extra").and_then(|e| e.get("merge-plugin"));
        Settings {
            includes: patterns(cfg.and_then(|c| c.get("include"))),
            requires: patterns(cfg.and_then(|c| c.get("require"))),
            recurse: flag(cfg, "recurse", true),
            replace: flag(cfg, "replace", false),
            ignore_duplicates: flag(cfg, "ignore-duplicates", false),
            merge_dev: flag(cfg, "merge-dev", true),
            merge_extra: flag(cfg, "merge-extra", false),
            merge_extra_deep: flag(cfg, "merge-extra-deep", false),
            merge_replace: flag(cfg, "merge-replace", true),
        }
    }

    /// `extra.merge-plugin` names at least one pattern.
    pub fn declared(manifest: &Value) -> bool {
        let s = Settings::from_manifest(manifest);
        !(s.includes.is_empty() && s.requires.is_empty())
    }
}

/// A merged `require` / `require-dev` link, as `Link` would carry it.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedLink {
    pub target: String,
    pub constraint: Constraint,
    /// `getPrettyConstraint()`: what an error line shows.
    pub pretty: String,
    /// A constraint text that parses to `constraint` (the pretty string,
    /// or the conjunction distributed over `||`), for the manifest.
    pub text: String,
}

/// The root package after the merge.
#[derive(Debug, Clone)]
pub struct Merged {
    /// The root manifest with the merged sections (`require`,
    /// `require-dev` as equivalent text; `autoload`, `autoload-dev`,
    /// `conflict`, `replace`, `provide`, `extra`).
    pub manifest: Value,
    pub requires: Vec<MergedLink>,
    pub requires_dev: Vec<MergedLink>,
    /// `getMergedRequirements()`: the names added, replaced or merged.
    pub merged_names: Vec<String>,
    /// The files merged, in order.
    pub files: Vec<String>,
}

struct Merger<'a> {
    project_dir: &'a Path,
    settings: Settings,
    with_dev: bool,
    root_version: String,
    root_pretty_version: String,
    root: Value,
    requires: BTreeMap<String, MergedLink>,
    requires_dev: BTreeMap<String, MergedLink>,
    /// Insertion order of the link maps (`$origin[$name]` keeps the
    /// original position, a new name is appended).
    require_order: Vec<String>,
    require_dev_order: Vec<String>,
    merged_names: Vec<String>,
    loaded: Vec<String>,
    files: Vec<String>,
}

/// `MergePlugin::onInit` + `onInstallUpdateOrDump` in one pass: the root
/// manifest, the root version (`RootPackage`: `version` or the VCS guess,
/// normalised and pretty), the dev mode of the run.
pub fn merge(
    project_dir: &Path,
    manifest: &Value,
    root_version: &str,
    root_pretty_version: &str,
    with_dev: bool,
) -> Result<Merged, String> {
    let settings = Settings::from_manifest(manifest);
    let mut m = Merger {
        project_dir,
        settings: settings.clone(),
        with_dev: with_dev && settings.merge_dev,
        root_version: root_version.to_owned(),
        root_pretty_version: root_pretty_version.to_owned(),
        root: manifest.clone(),
        requires: BTreeMap::new(),
        requires_dev: BTreeMap::new(),
        require_order: Vec::new(),
        require_dev_order: Vec::new(),
        merged_names: Vec::new(),
        loaded: Vec::new(),
        files: Vec::new(),
    };
    for key in ["require", "require-dev"] {
        let (map, order) = if key == "require" {
            (&mut m.requires, &mut m.require_order)
        } else {
            (&mut m.requires_dev, &mut m.require_dev_order)
        };
        if let Some(links) = manifest.get(key).and_then(Value::as_object) {
            for (target, pretty) in links {
                let Some(pretty) = pretty.as_str() else {
                    continue;
                };
                let name = target.to_lowercase();
                let parsed = parse_constraints(pretty)
                    .map_err(|e| format!("composer.json {key}.{target}: {e}"))?;
                map.insert(
                    name.clone(),
                    MergedLink {
                        target: name.clone(),
                        constraint: parsed.constraint,
                        pretty: pretty.to_owned(),
                        text: pretty.to_owned(),
                    },
                );
                order.push(name);
            }
        }
    }
    let includes = settings.includes.clone();
    let requires = settings.requires.clone();
    m.merge_files(&includes, false)?;
    m.merge_files(&requires, true)?;
    // The textual require sections of the merged manifest.
    for (key, map, order) in [
        ("require", &m.requires, &m.require_order),
        ("require-dev", &m.requires_dev, &m.require_dev_order),
    ] {
        if map.is_empty() {
            continue;
        }
        let mut out = Map::new();
        for name in order {
            if let Some(l) = map.get(name) {
                out.insert(name.clone(), Value::String(l.text.clone()));
            }
        }
        m.root[key] = Value::Object(out);
    }
    Ok(Merged {
        manifest: m.root,
        requires: m
            .require_order
            .iter()
            .filter_map(|n| m.requires.get(n).cloned())
            .collect(),
        requires_dev: m
            .require_dev_order
            .iter()
            .filter_map(|n| m.requires_dev.get(n).cloned())
            .collect(),
        merged_names: m.merged_names,
        files: m.files,
    })
}

impl Merger<'_> {
    /// `mergeFiles`: each pattern globbed (flags 0), a `require` pattern
    /// must match; the matches in glob order, each merged once.
    fn merge_files(&mut self, patterns: &[String], required: bool) -> Result<(), String> {
        let mut paths: Vec<String> = Vec::new();
        for pattern in patterns {
            let found = glob_plain(pattern, self.project_dir);
            if required && found.is_empty() {
                return Err(format!(
                    "merge-plugin: No files matched required '{pattern}'"
                ));
            }
            paths.extend(found);
        }
        for path in paths {
            self.merge_file(&path)?;
        }
        Ok(())
    }

    /// `mergeFile`: a path is merged once (by its spelling).
    fn merge_file(&mut self, path: &str) -> Result<(), String> {
        if self.loaded.iter().any(|p| p == path) {
            return Ok(());
        }
        let text = std::fs::read_to_string(self.project_dir.join(path))
            .map_err(|e| format!("merge-plugin: {path}: {e}"))?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("merge-plugin: \"{path}\" does not contain valid JSON\n{e}"))?;
        if !json.is_object() {
            return Err(format!(
                "merge-plugin: \"{path}\" does not contain valid JSON"
            ));
        }
        let base = {
            let d = vivacity_core::pathutil::php_dirname(path);
            if d == "." {
                String::new()
            } else {
                format!("{d}/")
            }
        };
        let name = json
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("merge-plugin/{}", path.replace('/', "-")));
        self.files.push(path.to_owned());
        self.loaded.push(path.to_owned());

        // mergeInto
        self.merge_requires("require", &json, &name)?;
        self.merge_package_links("conflict", &json)?;
        if self.settings.merge_replace {
            self.merge_package_links("replace", &json)?;
        }
        self.merge_package_links("provide", &json)?;
        self.merge_autoload("autoload", &json, &base);
        self.merge_extra(&json);
        if self.with_dev {
            // mergeDevInto
            self.merge_requires("require-dev", &json, &name)?;
            self.merge_autoload("autoload-dev", &json, &base);
        }
        if self.settings.recurse {
            let inner = Settings::from_manifest(&json);
            let rebase = |p: &String| format!("{base}{p}");
            let includes: Vec<String> = inner.includes.iter().map(rebase).collect();
            let requires: Vec<String> = inner.requires.iter().map(rebase).collect();
            self.merge_files(&includes, false)?;
            self.merge_files(&requires, true)?;
        }
        Ok(())
    }

    /// `replaceSelfVersionDependencies` for one link: `self.version` becomes
    /// the root's constraint on a package named like the include (looked up
    /// by the link's source, as the plugin does) or the root's version.
    fn self_version(&self, key: &str, source: &str, pretty: &str) -> (Constraint, String) {
        if pretty != "self.version" {
            let c = parse_constraints(pretty)
                .map(|p| p.constraint)
                .unwrap_or(Constraint::MatchAll);
            return (c, pretty.to_owned());
        }
        let root_links = self.root.get(key).and_then(Value::as_object);
        if let Some(existing) = root_links
            .and_then(|m| m.get(&source.to_lowercase()))
            .and_then(Value::as_str)
        {
            let c = parse_constraints(existing)
                .map(|p| p.constraint)
                .unwrap_or(Constraint::MatchAll);
            return (c, existing.to_owned());
        }
        let c = parse_constraints(&self.root_version)
            .map(|p| p.constraint)
            .unwrap_or(Constraint::MatchAll);
        (c, self.root_pretty_version.clone())
    }

    /// `mergeRequires` + `mergeOrDefer` on `require` or `require-dev`.
    fn merge_requires(&mut self, key: &str, json: &Value, source: &str) -> Result<(), String> {
        let Some(links) = json.get(key).and_then(Value::as_object) else {
            return Ok(());
        };
        if links.is_empty() {
            return Ok(());
        }
        let ignore = self.settings.ignore_duplicates;
        let replace = self.settings.replace;
        for (target, pretty) in links {
            let Some(pretty) = pretty.as_str() else {
                continue;
            };
            let name = target.to_lowercase();
            let (constraint, pretty) = self.self_version(key, source, pretty);
            let incoming = MergedLink {
                target: name.clone(),
                constraint,
                text: pretty.clone(),
                pretty,
            };
            let (map, order) = if key == "require" {
                (&mut self.requires, &mut self.require_order)
            } else {
                (&mut self.requires_dev, &mut self.require_dev_order)
            };
            match map.get(&name) {
                Some(_) if ignore => {}
                Some(_) if replace => {
                    map.insert(name.clone(), incoming);
                    self.merged_names.push(name);
                }
                Some(origin) => {
                    let merged = merge_constraints(origin, &incoming);
                    map.insert(name.clone(), merged);
                    self.merged_names.push(name);
                }
                None => {
                    map.insert(name.clone(), incoming);
                    order.push(name.clone());
                    self.merged_names.push(name);
                }
            }
        }
        Ok(())
    }

    /// `mergePackageLinks`: `array_merge($root->getX(), $links)` keyed by
    /// target — the include overwrites a duplicate key in place, new keys
    /// append; `self.version` replaced.
    fn merge_package_links(&mut self, key: &str, json: &Value) -> Result<(), String> {
        let Some(links) = json.get(key).and_then(Value::as_object) else {
            return Ok(());
        };
        if links.is_empty() {
            return Ok(());
        }
        let source = json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mut out: Map<String, Value> = self
            .root
            .get(key)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (target, pretty) in links {
            let Some(pretty) = pretty.as_str() else {
                continue;
            };
            let (_, pretty) = self.self_version(key, &source, pretty);
            let name = target.to_lowercase();
            // The root's key may be spelled with capitals: same package.
            if let Some(existing) = out.keys().find(|k| k.to_lowercase() == name).cloned() {
                out.insert(existing, Value::String(pretty));
            } else {
                out.insert(name, Value::String(pretty));
            }
        }
        self.root[key] = Value::Object(out);
        Ok(())
    }

    /// `mergeAutoload`: `array_merge_recursive($root->getAutoload(),
    /// fixRelativePaths($autoload))`.
    fn merge_autoload(&mut self, key: &str, json: &Value, base: &str) {
        let Some(autoload) = json.get(key) else {
            return;
        };
        if !autoload.is_object() || autoload.as_object().is_some_and(|o| o.is_empty()) {
            return;
        }
        let rebased = rebase(autoload, base);
        let current = self
            .root
            .get(key)
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        self.root[key] = array_merge_recursive(&current, &rebased);
    }

    /// `mergeExtra`: only with `merge-extra`; the root wins unless
    /// `replace`; deep with `merge-extra-deep`; the include's own
    /// `merge-plugin` block stripped first.
    fn merge_extra(&mut self, json: &Value) {
        let Some(extra) = json.get("extra").and_then(Value::as_object) else {
            return;
        };
        let mut extra = extra.clone();
        extra.remove("merge-plugin");
        if !self.settings.merge_extra || extra.is_empty() {
            return;
        }
        let extra = Value::Object(extra);
        let root_extra = self
            .root
            .get("extra")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let (first, second) = if self.settings.replace {
            (&root_extra, &extra)
        } else {
            (&extra, &root_extra)
        };
        self.root["extra"] = if self.settings.merge_extra_deep {
            merge_deep(first, second)
        } else {
            array_merge(first, second)
        };
    }
}

/// `mergeConstraints`: the subset short-cuts, else the conjunction with
/// the pretty `"<old>, <new>"`.
fn merge_constraints(origin: &MergedLink, incoming: &MergedLink) -> MergedLink {
    if is_subset_of(&origin.constraint, &incoming.constraint) {
        return origin.clone();
    }
    if is_subset_of(&incoming.constraint, &origin.constraint) {
        return incoming.clone();
    }
    MergedLink {
        target: origin.target.clone(),
        constraint: Constraint::create(
            vec![origin.constraint.clone(), incoming.constraint.clone()],
            true,
        ),
        pretty: format!("{}, {}", origin.pretty, incoming.pretty),
        text: conjunction_text(&origin.text, &incoming.text),
    }
}

/// `fixRelativePaths`: every leaf string prefixed by the include's
/// directory (`array_walk_recursive`).
fn rebase(v: &Value, base: &str) -> Value {
    match v {
        Value::String(s) => Value::String(format!("{base}{s}")),
        Value::Array(a) => Value::Array(a.iter().map(|x| rebase(x, base)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), rebase(x, base)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The text of `a AND b` that Composer's parser reads as such: it splits
/// on `||` first, so `"A || B, C"` would mean `A || (B, C)`; the
/// conjunction is distributed over the alternatives of each side
/// (`(A || B), C` → `A, C || B, C`) — the same set, for the consumers
/// that parse text (the platform check).
fn conjunction_text(a: &str, b: &str) -> String {
    let mut out = Vec::new();
    for x in split_or(a) {
        for y in split_or(b) {
            out.push(format!("{x}, {y}"));
        }
    }
    out.join(" || ")
}

/// Top-level `||` / `|` alternatives of a constraint text.
fn split_or(text: &str) -> Vec<&str> {
    text.split("||")
        .flat_map(|s| s.split('|'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, rel: &str, v: &Value) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(v).unwrap()).unwrap();
    }

    fn root() -> Value {
        json!({
            "name": "acme/app",
            "require": {"monolog/monolog": "^2.8", "psr/log": "^3.0"},
            "require-dev": {"phpunit/phpunit": "^10"},
            "autoload": {"psr-4": {"App\\": "src/"}, "files": ["helpers.php"]},
            "conflict": {"old/lib": "<1.0"},
            "extra": {"merge-plugin": {"include": ["modules/*/composer.json", "composer.ext.json"], "require": ["required/composer.json"]}, "keep": 1, "shared": {"a": 1}}
        })
    }

    #[test]
    fn merges_every_section_like_the_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            "modules/Base/composer.json",
            &json!({
                "name": "acme/base",
                "require": {"monolog/monolog": "^2.11 || ^1.0", "guzzlehttp/guzzle": "^7.5", "acme/self": "self.version"},
                "require-dev": {"mockery/mockery": "^1.6"},
                "autoload": {"psr-4": {"App\\": "src/", "Base\\": "lib/"}, "files": ["boot.php"], "exclude-from-classmap": ["tests/"]},
                "autoload-dev": {"psr-4": {"BaseTests\\": "tests/"}},
                "provide": {"psr/log-implementation": "3.0"},
                "replace": {"acme/base-compat": "self.version"},
                "conflict": {"old/lib": "<2.0", "bad/lib": "*"},
                "extra": {"merge-plugin": {"include": ["nested/composer.json"]}, "shared": {"b": 2}, "only-include": true}
            }),
        );
        write(
            d,
            "modules/Base/nested/composer.json",
            &json!({
                "require": {"nested/dep": "^1.0"},
                "autoload": {"classmap": ["Legacy/"]}
            }),
        );
        write(
            d,
            "composer.ext.json",
            &json!({"require": {"psr/log": "^3.0"}}),
        );
        write(
            d,
            "required/composer.json",
            &json!({"require": {"symfony/yaml": "^7.4"}}),
        );
        let m = merge(d, &root(), "1.2.3.0", "1.2.3", true).unwrap();
        assert_eq!(
            m.files,
            vec![
                "modules/Base/composer.json",
                "modules/Base/nested/composer.json",
                "composer.ext.json",
                "required/composer.json"
            ]
        );
        // require: the duplicate monolog key is the conjunction (neither is a subset).
        let mono = m
            .requires
            .iter()
            .find(|l| l.target == "monolog/monolog")
            .unwrap();
        assert_eq!(mono.pretty, "^2.8, ^2.11 || ^1.0");
        assert!(mono.constraint.matches_version("2.11.0.0"));
        assert!(
            !mono.constraint.matches_version("1.5.0.0"),
            "1.x is outside ^2.8"
        );
        assert!(!mono.constraint.matches_version("2.10.0.0"), "below ^2.11");
        // Its text parses to the same set: distributed over ||.
        assert_eq!(
            m.manifest["require"]["monolog/monolog"],
            json!("^2.8, ^2.11 || ^2.8, ^1.0")
        );
        let reparsed = parse_constraints("^2.8, ^2.11 || ^2.8, ^1.0")
            .unwrap()
            .constraint;
        for v in ["2.11.0.0", "2.10.0.0", "1.5.0.0", "3.0.0.0"] {
            assert_eq!(
                reparsed.matches_version(v),
                mono.constraint.matches_version(v),
                "{v}"
            );
        }
        // psr/log ^3.0 twice: a subset of itself → unchanged pretty.
        assert_eq!(m.manifest["require"]["psr/log"], json!("^3.0"));
        // self.version → the root's version.
        assert_eq!(m.manifest["require"]["acme/self"], json!("1.2.3"));
        assert_eq!(m.manifest["replace"]["acme/base-compat"], json!("1.2.3"));
        // New keys appended in order: guzzle, acme/self, nested, symfony/yaml.
        let order: Vec<&str> = m.requires.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(
            order,
            vec![
                "monolog/monolog",
                "psr/log",
                "guzzlehttp/guzzle",
                "acme/self",
                "nested/dep",
                "symfony/yaml"
            ]
        );
        assert_eq!(
            m.requires_dev
                .iter()
                .map(|l| l.target.as_str())
                .collect::<Vec<_>>(),
            vec!["phpunit/phpunit", "mockery/mockery"]
        );
        // A merged duplicate counts as merged even when the subset short-cut kept one side (psr/log).
        assert_eq!(
            m.merged_names,
            vec![
                "monolog/monolog",
                "guzzlehttp/guzzle",
                "acme/self",
                "mockery/mockery",
                "nested/dep",
                "psr/log",
                "symfony/yaml"
            ]
        );
        // autoload: paths re-based, duplicate PSR-4 key becomes a list.
        assert_eq!(
            m.manifest["autoload"],
            json!({
                "psr-4": {"App\\": ["src/", "modules/Base/src/"], "Base\\": "modules/Base/lib/"},
                "files": ["helpers.php", "modules/Base/boot.php"],
                "exclude-from-classmap": ["modules/Base/tests/"],
                "classmap": ["modules/Base/nested/Legacy/"]
            })
        );
        assert_eq!(
            m.manifest["autoload-dev"],
            json!({"psr-4": {"BaseTests\\": "modules/Base/tests/"}})
        );
        // links: the include overwrites a duplicate conflict key in place.
        assert_eq!(
            m.manifest["conflict"],
            json!({"old/lib": "<2.0", "bad/lib": "*"})
        );
        assert_eq!(
            m.manifest["provide"],
            json!({"psr/log-implementation": "3.0"})
        );
        // extra untouched without merge-extra (the include's block ignored).
        assert_eq!(m.manifest["extra"]["keep"], json!(1));
        assert!(m.manifest["extra"].get("only-include").is_none());
    }

    #[test]
    fn duplicate_rules_dev_off_and_extra() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            "inc/composer.json",
            &json!({
                "require": {"monolog/monolog": "^2.11"},
                "require-dev": {"x/y": "*"},
                "autoload-dev": {"psr-4": {"T\\": "t/"}},
                "extra": {"shared": {"b": 2}, "keep": 2, "new": 3}
            }),
        );
        let mut r = root();
        r["extra"]["merge-plugin"] = json!({"include": ["inc/composer.json"], "ignore-duplicates": true, "merge-dev": false, "merge-extra": true});
        let m = merge(d, &r, "1.0.0.0", "1.0.0", true).unwrap();
        assert_eq!(
            m.manifest["require"]["monolog/monolog"],
            json!("^2.8"),
            "ignored duplicate"
        );
        assert!(m.merged_names.is_empty());
        assert!(
            m.requires_dev.iter().all(|l| l.target != "x/y"),
            "merge-dev false"
        );
        assert!(m.manifest.get("autoload-dev").is_none());
        // merge-extra shallow, root wins.
        assert_eq!(m.manifest["extra"]["keep"], json!(1));
        assert_eq!(m.manifest["extra"]["shared"], json!({"a": 1}));
        assert_eq!(m.manifest["extra"]["new"], json!(3));
        // replace: the include wins; deep: maps merge.
        r["extra"]["merge-plugin"] = json!({"include": ["inc/composer.json"], "replace": true, "merge-extra": true, "merge-extra-deep": true});
        let m = merge(d, &r, "1.0.0.0", "1.0.0", false).unwrap();
        assert_eq!(m.manifest["require"]["monolog/monolog"], json!("^2.11"));
        assert_eq!(m.manifest["extra"]["keep"], json!(2));
        assert_eq!(m.manifest["extra"]["shared"], json!({"a": 1, "b": 2}));
        // Subset short-cut: ^2.8 then ^2.9 → ^2.9 (the narrower side).
        write(
            d,
            "inc/composer.json",
            &json!({"require": {"monolog/monolog": "^2.9"}}),
        );
        r["extra"]["merge-plugin"] = json!({"include": ["inc/composer.json"]});
        let m = merge(d, &r, "1.0.0.0", "1.0.0", false).unwrap();
        assert_eq!(m.manifest["require"]["monolog/monolog"], json!("^2.9"));
    }

    #[test]
    fn required_pattern_must_match_and_invalid_json_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mut r = root();
        r["extra"]["merge-plugin"] = json!({"require": ["nowhere/*.json"]});
        assert_eq!(
            merge(d, &r, "1.0.0.0", "1.0.0", true).unwrap_err(),
            "merge-plugin: No files matched required 'nowhere/*.json'"
        );
        std::fs::create_dir_all(d.join("inc")).unwrap();
        std::fs::write(d.join("inc/composer.json"), "{ nope").unwrap();
        r["extra"]["merge-plugin"] = json!({"include": "inc/composer.json"});
        assert!(merge(d, &r, "1.0.0.0", "1.0.0", true)
            .unwrap_err()
            .contains("does not contain valid JSON"));
        // No match on an include: nothing merged, manifest identical.
        r["extra"]["merge-plugin"] = json!({"include": ["absent/*.json"]});
        let m = merge(d, &r, "1.0.0.0", "1.0.0", true).unwrap();
        assert_eq!(m.manifest, r);
        assert!(m.files.is_empty());
    }
}
