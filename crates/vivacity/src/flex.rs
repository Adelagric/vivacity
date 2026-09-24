//! symfony/flex at resolution time: the index its `PRE_POOL_CREATE` filter
//! reads (`vivacity_resolver::flex_filter`), fetched the way Flex's
//! `Downloader` does — the endpoints of `extra.symfony.endpoint` /
//! `SYMFONY_ENDPOINT` (default: the two recipe indexes on GitHub), one
//! conditional GET each, cached under `<cache-repo-dir>/flex/<key>` in
//! Flex's own format (`{"body": …, "headers": {…}}`), so Composer with
//! Flex and vivacity share the cache. A `file://` endpoint is read directly
//! (the harness serves a captured index that way).

use anyhow::Context as _;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vivacity_resolver::flex_filter::FlexVersions;

const DEFAULT_ENDPOINTS: [&str; 2] = [
    "https://raw.githubusercontent.com/symfony/recipes/flex/main/index.json",
    "https://raw.githubusercontent.com/symfony/recipes-contrib/flex/main/index.json",
];

/// `Downloader::__construct`: the endpoint list. `extra.symfony.endpoint`
/// as a string containing `.json` (plus `flex://defaults`), an array, or
/// `flex://defaults`; `SYMFONY_ENDPOINT` prepended; `flex://defaults`
/// expanded in place. A legacy endpoint (no `.json`) is not an index
/// list — Flex then reads `<endpoint>/versions.json`, not emulated.
pub fn endpoints(manifest: &Value) -> Result<Vec<String>, String> {
    let configured = manifest
        .get("extra")
        .and_then(|e| e.get("symfony"))
        .and_then(|s| s.get("endpoint"));
    let mut list: Option<Vec<String>> = match configured {
        None => Some(DEFAULT_ENDPOINTS.iter().map(|s| (*s).to_owned()).collect()),
        Some(Value::Array(a)) => Some(
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        Some(Value::String(s)) if s.contains(".json") => {
            Some(vec![s.clone(), "flex://defaults".to_owned()])
        }
        Some(Value::String(s)) if s == "flex://defaults" => Some(vec![s.clone()]),
        Some(Value::String(s)) => {
            return Err(format!(
            "extra.symfony.endpoint {s:?} is a legacy Flex endpoint (versions.json), not emulated"
        ))
        }
        Some(other) => {
            return Err(format!(
                "extra.symfony.endpoint {other} is not a string or a list"
            ))
        }
    };
    if let Ok(env) = std::env::var("SYMFONY_ENDPOINT") {
        if env.contains(".json") || env == "flex://defaults" {
            let l = list
                .get_or_insert_with(|| DEFAULT_ENDPOINTS.iter().map(|s| (*s).to_owned()).collect());
            l.insert(0, env);
        } else if !env.is_empty() {
            return Err(format!(
                "SYMFONY_ENDPOINT {env:?} is a legacy Flex endpoint (versions.json), not emulated"
            ));
        }
    }
    let mut out: Vec<String> = Vec::new();
    for e in list.unwrap_or_default() {
        if e == "flex://defaults" {
            out.extend(DEFAULT_ENDPOINTS.iter().map(|s| (*s).to_owned()));
        } else {
            out.push(e);
        }
    }
    // `array_fill_keys`: duplicates collapse to the first occurrence.
    let mut seen = std::collections::HashSet::new();
    out.retain(|e| seen.insert(e.clone()));
    Ok(out)
}

/// `Downloader::generateCacheKey`.
fn cache_key(url: &str) -> String {
    let mut u = url.to_owned();
    if let Some(rest) = u.strip_prefix("https://api.github.com/repos/") {
        // `([^/]++/[^/]++)/contents/` → `$1/`
        let mut parts = rest.splitn(3, '/');
        if let (Some(owner), Some(repo), Some(tail)) = (parts.next(), parts.next(), parts.next()) {
            if let Some(after) = tail.strip_prefix("contents/") {
                u = format!("{owner}/{repo}/{after}");
            }
        }
    } else if let Some(rest) = u.strip_prefix("https://raw.githubusercontent.com/") {
        let mut parts = rest.splitn(3, '/');
        if let (Some(owner), Some(repo), Some(tail)) = (parts.next(), parts.next(), parts.next()) {
            u = format!("{owner}/{repo}/{tail}");
        }
    }
    let key: String = u
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if key.len() > 140 {
        use md5::Digest as _;
        let digest = md5::Md5::digest(url.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    } else {
        key
    }
}

fn cache_dir() -> PathBuf {
    vivacity_core::fetch::composer_cache_dir()
        .join("repo")
        .join("flex")
}

/// One endpoint's index: the cached body under `If-Modified-Since`, else
/// the fetched one (cache written in Flex's format). `file://` read as is.
fn fetch_index(
    url: &str,
    runtime: &tokio::runtime::Runtime,
    fetcher: &vivacity_core::fetch::Fetcher,
    offline: bool,
) -> anyhow::Result<Value> {
    if let Some(path) = url.strip_prefix("file://") {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read Flex index {path}"))?;
        return serde_json::from_str(&text)
            .with_context(|| format!("Flex index {path}: invalid JSON"));
    }
    let cache_file = cache_dir().join(cache_key(url));
    let cached: Option<Value> = vivacity_core::jsonfile::read(&cache_file).map(|v| (*v).clone());
    let cached_body = cached.as_ref().and_then(|c| c.get("body").cloned());
    let last_modified = cached
        .as_ref()
        .and_then(|c| c.get("headers"))
        .and_then(|h| h.get("last-modified"))
        .and_then(|v| v.get(0))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if offline {
        return cached_body.context(format!("offline: Flex index {url} is not cached"));
    }
    let response = runtime
        .block_on(fetcher.metadata_fetch(url, last_modified.as_deref()))
        .with_context(|| format!("cannot fetch Flex index {url}"))?;
    use vivacity_core::fetch::MetadataResponse as R;
    match response {
        R::NotModified => {
            cached_body.context(format!("Flex index {url}: 304 without a cached body"))
        }
        R::NotFound => anyhow::bail!("Flex index {url}: not found"),
        R::Body {
            bytes,
            last_modified,
        } => {
            let body: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("Flex index {url}: invalid JSON"))?;
            let mut headers = serde_json::Map::new();
            if let Some(lm) = last_modified {
                headers.insert(
                    "last-modified".to_owned(),
                    Value::Array(vec![Value::String(lm)]),
                );
            }
            let record = serde_json::json!({"body": body, "headers": headers});
            if std::fs::create_dir_all(cache_dir()).is_ok() {
                let tmp = cache_file.with_extension(format!("tmp.{}", std::process::id()));
                if let Ok(text) = serde_json::to_string(&record) {
                    if std::fs::write(&tmp, text).is_ok()
                        && std::fs::rename(&tmp, &cache_file).is_err()
                    {
                        let _ = std::fs::remove_file(&tmp);
                    }
                }
            }
            Ok(record["body"].clone())
        }
    }
}

/// The merged `versions` of the endpoints, as `Downloader::getVersions`
/// gives them to the filter.
pub fn versions(project: &Path, manifest: &Value, offline: bool) -> anyhow::Result<FlexVersions> {
    let merged = FlexVersions::merge_endpoints(&fetch_indexes(project, manifest, offline)?);
    FlexVersions::from_value(&merged).map_err(|e| anyhow::anyhow!("{e}"))
}

/// The endpoints' indexes, in order.
fn fetch_indexes(project: &Path, manifest: &Value, offline: bool) -> anyhow::Result<Vec<Value>> {
    let urls = endpoints(manifest).map_err(|e| anyhow::anyhow!("{e}"))?;
    let runtime = tokio::runtime::Runtime::new().context("cannot start the async runtime")?;
    let fetcher = Arc::new(vivacity_core::fetch::Fetcher::new(
        vivacity_core::fetch::composer_cache_dir(),
        vivacity_core::fetch::Auth::load(project),
    )?);
    let mut indexes = Vec::new();
    for url in &urls {
        indexes.push(fetch_index(url, &runtime, &fetcher, offline)?);
    }
    Ok(indexes)
}

/// `Downloader::$index`: package name -> the recipe versions the
/// endpoints offer, first endpoint winning a package entirely
/// (`$this->index[$package] ?? array_fill_keys($versions, $endpoint)`).
/// `recipe-conflicts` are NOT applied: they only ever REMOVE entries, so
/// ignoring them can make vivacity believe a recipe exists where Flex
/// finds none — the safe direction (a needless hand-over, never a missed
/// recipe).
#[derive(Debug, Default, Clone)]
pub struct RecipeIndex {
    recipes: std::collections::BTreeMap<String, Vec<String>>,
}

pub fn recipe_index(
    project: &Path,
    manifest: &Value,
    offline: bool,
) -> anyhow::Result<RecipeIndex> {
    let mut recipes: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for index in fetch_indexes(project, manifest, offline)? {
        let Some(map) = index.get("recipes").and_then(Value::as_object) else {
            continue;
        };
        for (name, versions) in map {
            let list: Vec<String> = versions
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            recipes.entry(name.clone()).or_insert(list);
        }
    }
    Ok(RecipeIndex { recipes })
}

impl RecipeIndex {
    /// `Downloader::getRecipes` for one package: is there a recipe version
    /// at or below the package's `major.minor`? The version is the pretty
    /// one, a `dev-*` branch replaced by its `branch-alias` (Flex's own
    /// preference order), then stripped of `dev-`/`v`/`.x-dev`/`-dev` and
    /// cut to `major.minor` (`minor` defaulting to `9999999`).
    /// A comparison vivacity cannot make numerically answers `true`: a
    /// hand-over costs a delegation, a miss would skip a recipe.
    pub fn applies_to(&self, name: &str, pretty_version: &str, extra: Option<&Value>) -> bool {
        let Some(versions) = self.recipes.get(name) else {
            return false;
        };
        let mut version = pretty_version.to_owned();
        if version.starts_with("dev-") {
            if let Some(aliases) = extra.and_then(|e| e.get("branch-alias")) {
                const PREFERRED: [&str; 11] = [
                    "dev-main",
                    "dev-trunk",
                    "dev-develop",
                    "dev-default",
                    "dev-latest",
                    "dev-next",
                    "dev-current",
                    "dev-support",
                    "dev-tip",
                    "dev-master",
                    // (the branch itself comes first, handled below)
                    "",
                ];
                let own = aliases.get(&version).and_then(Value::as_str);
                let alias = own.or_else(|| {
                    PREFERRED
                        .iter()
                        .filter(|k| !k.is_empty())
                        .find_map(|k| aliases.get(*k).and_then(Value::as_str))
                });
                if let Some(a) = alias {
                    version = a.to_owned();
                }
            }
        }
        let stripped = strip_version(&version);
        let mut parts = stripped.split('.');
        let major = parts.next().unwrap_or("").to_owned();
        let minor = parts.next().unwrap_or("9999999").to_owned();
        let cut = format!("{major}.{minor}");
        versions
            .iter()
            .rev()
            .any(|v| match compare_dotted(&cut, v) {
                Some(std::cmp::Ordering::Less) => false,
                // Equal, Greater, or not comparable numerically.
                _ => true,
            })
    }
}

/// `preg_replace('/^dev-|^v|\.x-dev$|-dev$/', '', $version)` — one
/// replacement per alternative, in that order, first match only.
fn strip_version(v: &str) -> String {
    let out = v.to_owned();
    for (prefix, pattern) in [
        (true, "dev-"),
        (true, "v"),
        (false, ".x-dev"),
        (false, "-dev"),
    ] {
        if prefix {
            if let Some(rest) = out.strip_prefix(pattern) {
                return rest.to_owned();
            }
        } else if let Some(rest) = out.strip_suffix(pattern) {
            return rest.to_owned();
        }
    }
    out
}

/// `version_compare` restricted to the shapes this uses (dotted decimal):
/// `None` when either side has a part that is not a plain number, which
/// the caller treats as "assume a recipe applies".
fn compare_dotted(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parse =
        |s: &str| -> Option<Vec<u64>> { s.split('.').map(|p| p.parse::<u64>().ok()).collect() };
    let (a, b) = (parse(a)?, parse(b)?);
    let len = a.len().max(b.len());
    for i in 0..len {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return Some(x.cmp(&y));
        }
    }
    Some(std::cmp::Ordering::Equal)
}

/// Where the candidate class file is read. `isBundleClass` always reads
/// `<vendor-dir>/<package name>/…`, whatever installer laid the package out
/// somewhere else, and it reads it at `POST_UPDATE_CMD` — i.e. the version
/// the install has just extracted, not the one vendor/ holds now.
pub enum ClassSource<'a> {
    /// vendor/ already holds the package at the version the lock names.
    LaidOut(&'a Path),
    /// The dist that would lay it out, read without extracting it.
    Dist {
        bytes: &'a [u8],
        kind: vivacity_core::store::DistKind,
    },
    /// Nothing will stand at `<vendor-dir>/<name>` — an installer puts the
    /// package elsewhere — so Flex's read finds no file and no candidate.
    Elsewhere,
}

/// `SymfonyBundle::getClassNames` for an install (`$uninstall` false, so
/// each candidate class must exist and mention one of the two Bundle base
/// classes): does this package register a bundle, i.e. would Flex build an
/// auto-generated recipe for it? Reading from wherever the file stands.
pub fn has_bundle_class_from(package: &Value, source: &ClassSource<'_>) -> anyhow::Result<bool> {
    let autoload = package.get("autoload");
    let is_sylius_plugin = package.get("type").and_then(Value::as_str) == Some("sylius-plugin");
    for (key, is_psr4) in [("psr-4", true), ("psr-0", false)] {
        let Some(map) = autoload.and_then(|a| a.get(key)).and_then(Value::as_object) else {
            continue;
        };
        for (namespace, paths) in map {
            let paths: Vec<&str> = match paths {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                _ => continue,
            };
            for path in paths {
                for class in extract_class_names(namespace, is_sylius_plugin) {
                    let rel = class_file_path(&class, path, is_psr4);
                    if read_class_file(source, &rel)?.is_some_and(|c| declares_bundle(&c)) {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

/// The file `isBundleClass` opens, relative to the package root.
fn class_file_path(class: &str, path: &str, is_psr4: bool) -> String {
    let parts: Vec<&str> = class.split('\\').collect();
    let last = parts.last().copied().unwrap_or("");
    let mut rel = path.trim_end_matches('/').to_owned();
    if !is_psr4 {
        let joined: String = parts[..parts.len() - 1].join("/").replace('\\', "");
        if !joined.is_empty() {
            if !rel.is_empty() {
                rel.push('/');
            }
            rel.push_str(&joined);
        }
    }
    if !rel.is_empty() {
        rel.push('/');
    }
    rel.push_str(&format!("{}.php", last.replace('\\', "/")));
    rel
}

fn read_class_file(source: &ClassSource<'_>, rel: &str) -> anyhow::Result<Option<Vec<u8>>> {
    Ok(match source {
        ClassSource::Elsewhere => None,
        ClassSource::LaidOut(root) => std::fs::read(root.join(rel)).ok(),
        ClassSource::Dist { bytes, kind } => match kind {
            vivacity_core::store::DistKind::Zip => {
                vivacity_core::extract::read_zip_entry(bytes, rel)?
            }
            vivacity_core::store::DistKind::Tar => {
                vivacity_core::extract::read_tar_entry(bytes, rel)?
            }
        },
    })
}

/// `isBundleClass`'s two literal needles, on bytes: `file_get_contents` and
/// `str_contains` do not care whether the file is valid UTF-8.
fn declares_bundle(contents: &[u8]) -> bool {
    [
        b"Symfony\\Component\\HttpKernel\\Bundle\\Bundle".as_slice(),
        b"Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle".as_slice(),
    ]
    .iter()
    .any(|needle| contents.windows(needle.len()).any(|w| w == *needle))
}

/// `SymfonyBundle::extractClassNames`.
fn extract_class_names(namespace: &str, is_sylius_plugin: bool) -> Vec<String> {
    let namespace = namespace.trim_matches('\\');
    let class = format!("{namespace}\\");
    let parts: Vec<&str> = namespace.split('\\').collect();
    let last = parts.last().copied().unwrap_or("");
    let end_of_word = if last.len() >= 6 {
        &last[last.len() - 6..]
    } else {
        last
    };
    let mut suffix = last.to_owned();
    if is_sylius_plugin {
        if end_of_word != "Bundle" && end_of_word != "Plugin" {
            suffix.push_str("Bundle");
        }
    } else if end_of_word != "Bundle" {
        suffix.push_str("Bundle");
    }
    let mut classes = vec![format!("{class}{suffix}")];
    let mut acc = String::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        if *part == "Bundle" || (is_sylius_plugin && *part == "Plugin") {
            continue;
        }
        classes.push(format!("{class}{part}{suffix}"));
        acc.push_str(part);
        classes.push(format!("{class}{acc}{suffix}"));
    }
    // `array_unique`: first occurrence kept, order preserved.
    let mut seen = std::collections::HashSet::new();
    classes.retain(|c| seen.insert(c.clone()));
    classes
}

/// A runtime and a fetcher for the reads the bundle question needs, built
/// once per command.
pub struct DistReader {
    runtime: tokio::runtime::Runtime,
    fetcher: vivacity_core::fetch::Fetcher,
}

impl DistReader {
    pub fn new(project: &Path) -> anyhow::Result<DistReader> {
        Ok(DistReader {
            runtime: tokio::runtime::Runtime::new().context("cannot start the async runtime")?,
            fetcher: vivacity_core::fetch::Fetcher::new(
                vivacity_core::fetch::composer_cache_dir(),
                vivacity_core::fetch::Auth::load(project),
            )?,
        })
    }

    /// The dist bytes of a package: Composer's own files cache when it
    /// holds them, the network otherwise — the very fetch the install would
    /// do, into the very cache it would fill, so nothing is transferred
    /// twice whatever this command decides.
    pub fn dist_bytes(
        &self,
        package: &vivacity_core::lock::LockPackage,
        offline: bool,
    ) -> anyhow::Result<(Vec<u8>, vivacity_core::store::DistKind)> {
        let kind = if package.dist_kind() == vivacity_core::lock::DistKind::Tar {
            vivacity_core::store::DistKind::Tar
        } else {
            vivacity_core::store::DistKind::Zip
        };
        let dist_type = match kind {
            vivacity_core::store::DistKind::Zip => "zip",
            vivacity_core::store::DistKind::Tar => "tar",
        };
        let url = package
            .dist_url_expanded()
            .context("package without a dist url")?;
        let (bytes, _) = self
            .runtime
            .block_on(self.fetcher.dist_bytes_of(
                package.name(),
                &url,
                dist_type,
                package.dist_shasum(),
                offline,
            ))
            .with_context(|| format!("cannot read the dist of {}", package.name()))?;
        Ok((bytes, kind))
    }
}

/// The names `symfony.lock` holds (`Lock::all`). Its path:
/// `SYMFONY_LOCKFILE`, else next to composer.lock (`Flex::activate`).
pub fn lock_names(project: &Path) -> std::collections::BTreeSet<String> {
    let path = match std::env::var_os("SYMFONY_LOCKFILE") {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => project.join("symfony.lock"),
    };
    let Some(value) = vivacity_core::jsonfile::read(&path) else {
        return Default::default();
    };
    value
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_version_selection_follows_the_downloader() {
        let index = RecipeIndex {
            recipes: [
                (
                    "a/b".to_owned(),
                    vec!["3.3".to_owned(), "5.4".to_owned(), "7.0".to_owned()],
                ),
                ("c/d".to_owned(), vec!["9.9".to_owned()]),
            ]
            .into_iter()
            .collect(),
        };
        // major.minor at or above a recipe version: a recipe applies.
        assert!(index.applies_to("a/b", "v7.1.2", None));
        assert!(index.applies_to("a/b", "5.4.0", None));
        assert!(index.applies_to("a/b", "3.3", None));
        // Below every recipe version: none.
        assert!(!index.applies_to("a/b", "3.2.9", None));
        assert!(!index.applies_to("c/d", "1.0.0", None));
        // Unknown package: none.
        assert!(!index.applies_to("x/y", "9.9.9", None));
        // `minor` defaults to 9999999, so a bare major is above everything.
        assert!(index.applies_to("c/d", "9", None));
        // `dev-` with a branch-alias takes the alias; `.x-dev` is stripped.
        let extra = serde_json::json!({"branch-alias": {"dev-main": "7.1.x-dev"}});
        assert!(index.applies_to("a/b", "dev-main", Some(&extra)));
        let low = serde_json::json!({"branch-alias": {"dev-main": "3.2.x-dev"}});
        assert!(!index.applies_to("a/b", "dev-main", Some(&low)));
        // No alias and a non-numeric branch: not comparable -> assume yes.
        assert!(index.applies_to("a/b", "dev-feature", None));
    }

    #[test]
    fn bundle_class_names_follow_symfony_bundle() {
        let names = extract_class_names("Acme\\FooBundle", false);
        assert_eq!(
            names,
            vec![
                "Acme\\FooBundle\\FooBundle",
                "Acme\\FooBundle\\AcmeFooBundle"
            ]
        );
        // A namespace that does not end in Bundle gets the suffix.
        let names = extract_class_names("Acme\\Foo", false);
        assert_eq!(
            names,
            vec!["Acme\\Foo\\FooBundle", "Acme\\Foo\\AcmeFooBundle"],
            "the accumulator and the part give the same class here"
        );
        // Three parts: the accumulator differs from the part.
        let names = extract_class_names("A\\B\\C", false);
        assert_eq!(
            names,
            vec![
                "A\\B\\C\\CBundle",
                "A\\B\\C\\ACBundle",
                "A\\B\\C\\BCBundle",
                "A\\B\\C\\ABCBundle"
            ]
        );
        // A sylius-plugin keeps a Plugin suffix and skips `Plugin` parts.
        let names = extract_class_names("Acme\\Plugin\\ShopPlugin", true);
        assert_eq!(
            names,
            vec![
                "Acme\\Plugin\\ShopPlugin\\ShopPlugin",
                "Acme\\Plugin\\ShopPlugin\\AcmeShopPlugin"
            ]
        );
    }

    #[test]
    fn a_bundle_is_found_only_when_the_file_says_so() {
        let d = tempfile::tempdir().expect("tmp");
        let vendor = d.path();
        let src = vendor.join("acme/foo-bundle/src");
        std::fs::create_dir_all(&src).expect("dirs");
        let package = serde_json::json!({"autoload": {"psr-4": {"Acme\\FooBundle\\": "src"}}});
        let root = vendor.join("acme/foo-bundle");
        let found = |p: &Value| {
            has_bundle_class_from(p, &ClassSource::LaidOut(&root)).expect("no error on a disk read")
        };
        // No file at all.
        assert!(!found(&package));
        // A file that is not a bundle.
        std::fs::write(src.join("FooBundle.php"), "<?php class FooBundle {}").expect("w");
        assert!(!found(&package));
        // The heuristic: either base class mentioned anywhere in the file.
        std::fs::write(
            src.join("FooBundle.php"),
            "<?php use Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle;",
        )
        .expect("w");
        assert!(found(&package));
    }

    #[test]
    fn the_dist_answers_the_same_as_the_laid_out_package() {
        // The same tree, once on disk and once as the zip that would lay it
        // out: `isBundleClass` must not care which it reads.
        let bundle = "<?php use Symfony\\Component\\HttpKernel\\Bundle\\Bundle;";
        let cases: [(&str, serde_json::Value, &str); 3] = [
            (
                "psr-4",
                serde_json::json!({"autoload": {"psr-4": {"Acme\\FooBundle\\": "src"}}}),
                "src/FooBundle.php",
            ),
            (
                // psr-0 appends the namespace's own directories under the
                // path, `implode('/', array_slice($parts, 0, -1))`.
                "psr-0",
                serde_json::json!({"autoload": {"psr-0": {"Acme\\Foo": "lib"}}}),
                "lib/Acme/Foo/FooBundle.php",
            ),
            (
                "root-path",
                serde_json::json!({"autoload": {"psr-4": {"Acme\\FooBundle\\": ""}}}),
                "FooBundle.php",
            ),
        ];
        for (tag, package, rel) in cases {
            let d = tempfile::tempdir().expect("tmp");
            let root = d.path().join("acme/foo-bundle");
            let file = root.join(rel);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
            std::fs::write(&file, bundle).expect("w");
            assert!(
                has_bundle_class_from(&package, &ClassSource::LaidOut(&root)).expect("disk"),
                "{tag}: not found on disk, so the path is wrong"
            );
            // One root directory, as a Packagist zipball has.
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.add_directory("foo-bundle-abc123", opts).expect("dir");
            w.start_file(format!("foo-bundle-abc123/{rel}"), opts)
                .expect("file");
            std::io::Write::write_all(&mut w, bundle.as_bytes()).expect("write");
            let zip_bytes = w.finish().expect("finish").into_inner();
            assert!(
                has_bundle_class_from(
                    &package,
                    &ClassSource::Dist {
                        bytes: &zip_bytes,
                        kind: vivacity_core::store::DistKind::Zip,
                    },
                )
                .expect("dist read"),
                "{tag}: the dist does not answer like the laid-out package"
            );
            // Laid out somewhere else: Flex reads vendor/<name> and finds
            // nothing there.
            assert!(!has_bundle_class_from(&package, &ClassSource::Elsewhere).expect("elsewhere"));
        }
    }

    #[test]
    fn cache_keys_match_flex() {
        assert_eq!(
            cache_key("https://raw.githubusercontent.com/symfony/recipes/flex/main/index.json"),
            "symfony-recipes-flex-main-index.json"
        );
        assert_eq!(
            cache_key(
                "https://raw.githubusercontent.com/symfony/recipes-contrib/flex/main/index.json"
            ),
            "symfony-recipes-contrib-flex-main-index.json"
        );
        assert_eq!(
            cache_key("https://example.com/a b.json"),
            "https---example.com-a-b.json"
        );
    }

    #[test]
    fn endpoint_forms() {
        std::env::remove_var("SYMFONY_ENDPOINT");
        let d = endpoints(&serde_json::json!({})).expect("defaults");
        assert_eq!(d.len(), 2);
        let one = endpoints(
            &serde_json::json!({"extra": {"symfony": {"endpoint": "https://x/index.json"}}}),
        )
        .expect("string");
        assert_eq!(one[0], "https://x/index.json");
        assert_eq!(one.len(), 3);
        let arr = endpoints(
            &serde_json::json!({"extra": {"symfony": {"endpoint": ["file:///tmp/i.json"]}}}),
        )
        .expect("array");
        assert_eq!(arr, vec!["file:///tmp/i.json"]);
        assert!(endpoints(
            &serde_json::json!({"extra": {"symfony": {"endpoint": "https://flex.symfony.com"}}})
        )
        .is_err());
    }
}
