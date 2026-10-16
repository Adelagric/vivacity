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
            // `Downloader::parseJson` caches a response only when it can be
            // revalidated (`last-modified` or `etag`; the fetcher keeps the
            // former).
            let cacheable = last_modified.is_some();
            if let Some(lm) = last_modified {
                headers.insert(
                    "last-modified".to_owned(),
                    Value::Array(vec![Value::String(lm)]),
                );
            }
            let record = serde_json::json!({"body": body, "headers": headers});
            if cacheable && std::fs::create_dir_all(cache_dir()).is_ok() {
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
    let indexes: Vec<Value> = fetch_indexes(project, manifest, offline)?
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    let merged = FlexVersions::merge_endpoints(&indexes);
    FlexVersions::from_value(&merged).map_err(|e| anyhow::anyhow!("{e}"))
}

/// The endpoints' indexes, in order, with their URLs.
fn fetch_indexes(
    project: &Path,
    manifest: &Value,
    offline: bool,
) -> anyhow::Result<Vec<(String, Value)>> {
    let urls = endpoints(manifest).map_err(|e| anyhow::anyhow!("{e}"))?;
    let runtime = tokio::runtime::Runtime::new().context("cannot start the async runtime")?;
    let fetcher = Arc::new(vivacity_core::fetch::Fetcher::new(
        vivacity_core::fetch::composer_cache_dir(),
        vivacity_core::fetch::Auth::load(project),
    )?);
    let mut indexes = Vec::new();
    for url in &urls {
        indexes.push((url.clone(), fetch_index(url, &runtime, &fetcher, offline)?));
    }
    Ok(indexes)
}

/// `Downloader::$index` and `$endpoints` after `initialize`: package name ->
/// the recipe versions the endpoints offer, first endpoint winning a
/// package entirely (`$this->index[$package] ?? array_fill_keys($versions,
/// $endpoint)`), each version with the endpoint that serves it; each
/// endpoint's own configuration (`_links`, `branch`, `is_contrib`); and the
/// `recipe-conflicts` of every endpoint, which `getRecipes` applies to the
/// merged index before choosing ([`RecipeIndex::apply_conflicts`]).
#[derive(Debug, Default, Clone)]
pub struct RecipeIndex {
    recipes: std::collections::BTreeMap<String, Vec<(String, usize)>>,
    conflicts: Vec<Value>,
    pub endpoints: Vec<(String, Value)>,
}

pub fn recipe_index(
    project: &Path,
    manifest: &Value,
    offline: bool,
) -> anyhow::Result<RecipeIndex> {
    let mut index = RecipeIndex::default();
    for (i, (url, body)) in fetch_indexes(project, manifest, offline)?
        .into_iter()
        .enumerate()
    {
        if let Some(map) = body.get("recipes").and_then(Value::as_object) {
            for (name, versions) in map {
                if index.recipes.contains_key(name) {
                    continue;
                }
                // `array_fill_keys`: a repeated version keeps its first place.
                let mut list: Vec<(String, usize)> = Vec::new();
                for v in versions.as_array().into_iter().flatten() {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    if !list.iter().any(|(x, _)| *x == v) {
                        list.push((v, i));
                    }
                }
                index.recipes.insert(name.clone(), list);
            }
        }
        index
            .conflicts
            .push(body.get("recipe-conflicts").cloned().unwrap_or(Value::Null));
        let mut config = body;
        if let Some(m) = config.as_object_mut() {
            for k in ["recipes", "recipe-conflicts", "versions", "aliases"] {
                m.remove(k);
            }
        }
        index.endpoints.push((url, config));
    }
    Ok(index)
}

/// What `getRecipes` makes of one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// No version at or below the package's: no recipe (the auto-generated
    /// bundle recipe may still apply).
    None,
    /// The recipe version, the endpoint serving it, and the `major.minor`
    /// the `symfony.lock` entry records.
    Recipe {
        version: String,
        endpoint: usize,
        lock_version: String,
    },
    /// The index lists the package but every version was removed by a
    /// `recipe-conflicts` entry: `Skipping recipe for …`.
    AllConflicted,
    /// A step vivacity cannot take exactly (the reason).
    Unsure(String),
}

impl RecipeIndex {
    /// Is the package in the index at all (`$this->index[$name] ?? null`)?
    pub fn lists(&self, name: &str) -> bool {
        self.recipes.contains_key(name)
    }

    /// `getRecipes`' first block: each endpoint's `recipe-conflicts`, a
    /// version removed from the merged index when the locked repository
    /// (`getLockedRepository(true)`, `findPackage` by name and constraint)
    /// holds a conflicting package. `found(name, constraint)` answers that
    /// question; an unparsable constraint is an error, as in Flex.
    pub fn apply_conflicts(
        &mut self,
        found: &dyn Fn(&str, &str) -> Result<bool, String>,
    ) -> Result<(), String> {
        for conflicts in std::mem::take(&mut self.conflicts) {
            let Value::Object(by_package) = conflicts else {
                if conflicts.is_null() || conflicts.as_array().is_some_and(|a| a.is_empty()) {
                    continue;
                }
                return Err("recipe-conflicts is not an object".to_owned());
            };
            for (package, by_version) in by_package {
                let Value::Object(by_version) = by_version else {
                    return Err(format!("recipe-conflicts.{package} is not an object"));
                };
                for (version, conflicting) in by_version {
                    let Value::Object(conflicting) = conflicting else {
                        return Err(format!(
                            "recipe-conflicts.{package}.{version} is not an object"
                        ));
                    };
                    for (other, constraint) in conflicting {
                        let constraint = constraint.as_str().ok_or_else(|| {
                            format!("recipe-conflicts.{package}.{version}.{other} is not a string")
                        })?;
                        if found(other.as_str(), constraint)? {
                            if let Some(list) = self.recipes.get_mut(&package) {
                                list.retain(|(v, _)| *v != version);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `Downloader::getRecipes` for one package: the pretty version, a
    /// `dev-*` branch replaced by its `branch-alias` (Flex's own preference
    /// order), stripped of `dev-`/`v`/`.x-dev`/`-dev`, cut to `major.minor`
    /// (`minor` defaulting to `9999999`), then the highest recipe version
    /// that is not above it. Only plain dotted numbers are compared — any
    /// other shape is `Unsure`, never a guess.
    pub fn select(&self, name: &str, pretty_version: &str, extra: Option<&Value>) -> Selection {
        let Some(versions) = self.recipes.get(name) else {
            return Selection::None;
        };
        if versions.is_empty() {
            return Selection::AllConflicted;
        }
        let mut version = pretty_version.to_owned();
        if version.starts_with("dev-") {
            if let Some(aliases) = extra.and_then(|e| e.get("branch-alias")) {
                const PREFERRED: [&str; 10] = [
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
                ];
                // `isset($a[$k]) && $alias = $a[$k]`: set AND truthy.
                let truthy = |v: &Value| {
                    !matches!(v, Value::Null | Value::Bool(false))
                        && v.as_str().is_none_or(|s| !s.is_empty() && s != "0")
                };
                let keys = std::iter::once(version.as_str()).chain(PREFERRED);
                let alias = keys
                    .filter_map(|k| aliases.get(k))
                    .find(|v| truthy(v))
                    .cloned();
                match alias {
                    None => {}
                    Some(Value::String(a)) => version = a,
                    Some(other) => {
                        return Selection::Unsure(format!(
                            "{name}'s branch-alias {other} is not a string"
                        ))
                    }
                }
            }
        }
        // `array_reverse` renumbers integer keys, and `array_fill_keys`
        // made an integer-like version one: the loop would then compare
        // positions, not versions.
        if let Some((v, _)) = versions.iter().find(|(v, _)| {
            let d = v.strip_prefix('-').unwrap_or(v);
            !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())
        }) {
            return Selection::Unsure(format!(
                "{name}'s recipe version {v} is an integer key for PHP"
            ));
        }
        let stripped = strip_version(&version);
        let mut parts = stripped.split('.');
        let major = parts.next().unwrap_or("").to_owned();
        let minor = parts.next().unwrap_or("9999999").to_owned();
        let cut = format!("{major}.{minor}");
        for (v, endpoint) in versions.iter().rev() {
            match php_version_compare(&cut, v) {
                Some(std::cmp::Ordering::Less) => continue,
                Some(_) => {
                    return Selection::Recipe {
                        version: v.clone(),
                        endpoint: *endpoint,
                        lock_version: cut,
                    }
                }
                None => {
                    return Selection::Unsure(format!(
                        "cannot compare {name}'s version {cut} with its recipe version {v}"
                    ))
                }
            }
        }
        Selection::None
    }
}

/// `preg_replace('/^dev-|^v|\.x-dev$|-dev$/', '', $version)`: a GLOBAL
/// replacement, so a prefix (`dev-`, else `v`) and a suffix (`.x-dev`,
/// else `-dev`) both go — `v1.x-dev` is `1`. The suffix is searched after
/// the prefix, as the scan resumes where the first match ended.
fn strip_version(v: &str) -> String {
    let rest = v
        .strip_prefix("dev-")
        .or_else(|| v.strip_prefix('v'))
        .unwrap_or(v);
    rest.strip_suffix(".x-dev")
        .or_else(|| rest.strip_suffix("-dev"))
        .unwrap_or(rest)
        .to_owned()
}

/// `version_compare` on dotted decimals: part by part, numerically; when
/// one side runs out, the side with a further numeric part is greater
/// (`version_compare('5.3', '5.3.0') === -1`). `None` for anything that is
/// not dotted decimal digits, which PHP would canonicalize and compare by
/// rules this does not reproduce.
fn php_version_compare(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parse = |s: &str| -> Option<Vec<u64>> {
        s.split('.')
            .map(|p| {
                if p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()) {
                    None
                } else {
                    p.parse::<u64>().ok()
                }
            })
            .collect()
    };
    let (a, b) = (parse(a)?, parse(b)?);
    for (x, y) in a.iter().zip(&b) {
        if x != y {
            return Some(x.cmp(y));
        }
    }
    Some(a.len().cmp(&b.len()))
}

/// `Downloader::getRecipes`' URL of a recipe file:
/// `recipe_template_relative` resolved against the endpoint's own URL
/// (`preg_replace('{[^/\?]*+(?=\?|$)}', $relative, $endpoint, 1)`: the last
/// path segment, before any query string), else `recipe_template`; then
/// `strtr` of the placeholders.
pub fn recipe_url(
    endpoint_url: &str,
    links: &Value,
    package: &str,
    version: &str,
) -> Result<String, String> {
    let template = match links.get("recipe_template_relative") {
        Some(Value::String(relative)) => {
            let (path, query) = match endpoint_url.find('?') {
                Some(q) => endpoint_url.split_at(q),
                None => (endpoint_url, ""),
            };
            let cut = path.rfind('/').map(|i| i + 1).unwrap_or(0);
            format!("{}{relative}{query}", &path[..cut])
        }
        None | Some(Value::Null) => match links.get("recipe_template") {
            Some(Value::String(t)) => t.clone(),
            _ => return Err("the endpoint has no recipe_template".to_owned()),
        },
        Some(other) => return Err(format!("recipe_template_relative is {other}")),
    };
    Ok(template
        .replace("{package_dotted}", &package.replace('/', "."))
        .replace("{package}", package)
        .replace("{version}", version))
}

/// One recipe file, fetched like `Downloader::get` (the conditional GET and
/// Flex's cache, shared with Composer).
pub fn fetch_recipe(project: &Path, url: &str, offline: bool) -> anyhow::Result<Value> {
    let runtime = tokio::runtime::Runtime::new().context("cannot start the async runtime")?;
    let fetcher = vivacity_core::fetch::Fetcher::new(
        vivacity_core::fetch::composer_cache_dir(),
        vivacity_core::fetch::Auth::load(project),
    )?;
    fetch_index(url, &runtime, &fetcher, offline)
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
/// classes), reading from wherever the file stands: every candidate class
/// whose file is a bundle, in the order Flex finds them (a class can come
/// back once per autoload path; the caller's map keeps the first). Empty:
/// no auto-generated recipe.
pub fn bundle_classes_from(
    package: &Value,
    source: &ClassSource<'_>,
) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for (class, rel) in bundle_candidates(package) {
        if read_class_file(source, &rel)?.is_some_and(|c| declares_bundle(&c)) {
            out.push(class);
        }
    }
    Ok(out)
}

/// The files `isBundleClass` would open for this package, relative to the
/// package root, in the order it opens them. Computed from the lock entry
/// alone: an EMPTY list means the answer is no with nothing to read — and
/// that has to be settled before a dist is fetched, or a package with no
/// dist at all (a metapackage) would hand the command over for nothing.
pub fn bundle_candidate_paths(package: &Value) -> Vec<String> {
    bundle_candidates(package)
        .into_iter()
        .map(|(_, path)| path)
        .collect()
}

/// `getClassNames`' candidates: each class `extractClassNames` yields for
/// each autoload path, with the file `isBundleClass` opens for it.
fn bundle_candidates(package: &Value) -> Vec<(String, String)> {
    let autoload = package.get("autoload");
    let is_sylius_plugin = package.get("type").and_then(Value::as_str) == Some("sylius-plugin");
    let mut out = Vec::new();
    for (key, is_psr4) in [("psr-4", true), ("psr-0", false)] {
        let Some(map) = autoload.and_then(|a| a.get(key)).and_then(Value::as_object) else {
            continue;
        };
        for (namespace, paths) in map {
            // `if (!is_array($paths)) { $paths = [$paths]; }`, and each is
            // then used as a string: a scalar is a path, not a skipped
            // namespace.
            let paths: Vec<String> = match paths {
                Value::Array(a) => a.iter().map(php_string).collect(),
                other => vec![php_string(other)],
            };
            for path in paths {
                for class in extract_class_names(namespace, is_sylius_plugin) {
                    let file = class_file_path(&class, &path, is_psr4);
                    out.push((class, file));
                }
            }
        }
    }
    out
}

/// A JSON value where PHP would use a string.
fn php_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(true) => "1".to_owned(),
        Value::Bool(false) => String::new(),
        other => other.to_string(),
    }
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
    // `substr($suffix, -6)` counts BYTES and the comparison is byte-wise:
    // slicing the &str would panic on a multi-byte boundary.
    let bytes = last.as_bytes();
    let end_of_word = &bytes[bytes.len().saturating_sub(6)..];
    let mut suffix = last.to_owned();
    if is_sylius_plugin {
        if end_of_word != b"Bundle" && end_of_word != b"Plugin" {
            suffix.push_str("Bundle");
        }
    } else if end_of_word != b"Bundle" {
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

/// `symfony.lock`'s path: `SYMFONY_LOCKFILE`, else next to composer.lock
/// (`Flex::activate`).
pub fn lock_path(project: &Path) -> PathBuf {
    match std::env::var_os("SYMFONY_LOCKFILE") {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => project.join("symfony.lock"),
    }
}

/// The names `symfony.lock` holds (`Lock::all`).
pub fn lock_names(project: &Path) -> std::collections::BTreeSet<String> {
    let Some(value) = vivacity_core::jsonfile::read(&lock_path(project)) else {
        return Default::default();
    };
    value
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// `recordOperations` normalizes every `symfony.lock` version
/// (`VersionParser::normalize`, `9999999.9999999` first patched to
/// `99999.9999999`): an entry it cannot read throws there, before any
/// recipe. The first such entry, or `None`.
pub fn unreadable_lock_entry(project: &Path) -> Option<String> {
    let value = vivacity_core::jsonfile::read(&lock_path(project))?;
    let map = value.as_object()?;
    for (name, info) in map {
        let version = match info.get("version") {
            Some(Value::String(v)) if v == "9999999.9999999" => "99999.9999999".to_owned(),
            Some(Value::String(v)) => v.clone(),
            _ => return Some(format!("symfony.lock's {name} has no string version")),
        };
        if vivacity_resolver::version::normalize(&version, None).is_err() {
            return Some(format!(
                "symfony.lock's {name} has the version {version:?}, which Flex cannot normalize"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(entries: &[(&str, &[&str])]) -> RecipeIndex {
        RecipeIndex {
            recipes: entries
                .iter()
                .map(|(n, vs)| {
                    (
                        (*n).to_owned(),
                        vs.iter().map(|v| ((*v).to_owned(), 0)).collect(),
                    )
                })
                .collect(),
            conflicts: Vec::new(),
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn recipe_version_selection_follows_the_downloader() {
        let index = index(&[("a/b", &["3.3", "5.4", "7.0"]), ("c/d", &["9.9"])]);
        let pick = |n: &str, v: &str, e: Option<&Value>| match index.select(n, v, e) {
            Selection::Recipe {
                version,
                lock_version,
                ..
            } => Some((version, lock_version)),
            Selection::None => None,
            other => panic!("{n} {v}: {other:?}"),
        };
        let some = |a: &str, b: &str| Some((a.to_owned(), b.to_owned()));
        // The highest recipe version at or below major.minor.
        assert_eq!(pick("a/b", "v7.1.2", None), some("7.0", "7.1"));
        assert_eq!(pick("a/b", "5.4.0", None), some("5.4", "5.4"));
        assert_eq!(pick("a/b", "3.3", None), some("3.3", "3.3"));
        // Below every recipe version, or not in the index: none.
        assert_eq!(pick("a/b", "3.2.9", None), None);
        assert_eq!(pick("c/d", "1.0.0", None), None);
        assert_eq!(pick("x/y", "9.9.9", None), None);
        // `minor` defaults to 9999999, which the lock entry records.
        assert_eq!(pick("c/d", "9", None), some("9.9", "9.9999999"));
        // `dev-` with a branch-alias takes the alias; `.x-dev` is stripped.
        let extra = serde_json::json!({"branch-alias": {"dev-main": "7.1.x-dev"}});
        assert_eq!(pick("a/b", "dev-main", Some(&extra)), some("7.0", "7.1"));
        let low = serde_json::json!({"branch-alias": {"dev-main": "3.2.x-dev"}});
        assert_eq!(pick("a/b", "dev-main", Some(&low)), None);
        // An empty alias is not taken (`isset && $alias = …`).
        let empty =
            serde_json::json!({"branch-alias": {"dev-feature": "", "dev-master": "5.4.x-dev"}});
        assert_eq!(pick("a/b", "dev-feature", Some(&empty)), some("5.4", "5.4"));
        // No alias and a non-numeric branch: not compared, never guessed.
        assert!(matches!(
            index.select("a/b", "dev-feature", None),
            Selection::Unsure(_)
        ));
    }

    #[test]
    fn strip_version_replaces_globally() {
        // Measured with preg_replace('/^dev-|^v|\.x-dev$|-dev$/', '', …).
        for (v, want) in [
            ("v1.x-dev", "1"),
            ("v8.1.0", "8.1.0"),
            ("dev-main", "main"),
            ("2.x-dev", "2"),
            ("1.0-dev", "1.0"),
            ("dev-v1-dev", "v1"),
            ("v-dev", ""),
        ] {
            assert_eq!(strip_version(v), want, "{v}");
        }
    }

    #[test]
    fn version_compare_matches_php_on_dotted_numbers() {
        use std::cmp::Ordering::*;
        // Measured with PHP's version_compare.
        assert_eq!(php_version_compare("5.3", "5.3.0"), Some(Less));
        assert_eq!(php_version_compare("5.10", "5.9"), Some(Greater));
        assert_eq!(php_version_compare("8.9999999", "8.1"), Some(Greater));
        assert_eq!(php_version_compare("1.0", "1.0"), Some(Equal));
        assert_eq!(php_version_compare("1.x", "1.0"), None);
    }

    #[test]
    fn recipe_conflicts_remove_versions_before_the_choice() {
        let mut idx = index(&[("a/b", &["5.3", "6.4"]), ("c/d", &["1.0"])]);
        idx.conflicts = vec![serde_json::json!({
            "a/b": {"6.4": {"x/y": "<2.0"}},
            "c/d": {"1.0": {"x/y": ">=1.0"}},
        })];
        // x/y 1.5 is locked: both entries conflict.
        idx.apply_conflicts(&|n, c| Ok(n == "x/y" && (c == "<2.0" || c == ">=1.0")))
            .expect("conflicts");
        assert!(matches!(
            idx.select("a/b", "6.4.0", None),
            Selection::Recipe { ref version, .. } if version == "5.3"
        ));
        assert_eq!(idx.select("c/d", "1.0.0", None), Selection::AllConflicted);
    }

    #[test]
    fn recipe_urls_follow_the_downloader() {
        let links = serde_json::json!({
            "recipe_template": "https://raw.githubusercontent.com/symfony/recipes/flex/main/{package_dotted}.{version}.json",
            "recipe_template_relative": "{package_dotted}.{version}.json",
        });
        assert_eq!(
            recipe_url(
                "http://127.0.0.1:8000/configurators/recipes/index.json",
                &links,
                "symfony/console",
                "5.3"
            )
            .expect("url"),
            "http://127.0.0.1:8000/configurators/recipes/symfony.console.5.3.json"
        );
        assert_eq!(
            recipe_url(
                "https://api.github.com/repos/Sylius/SyliusRecipes/contents/index.json?ref=flex/main",
                &links,
                "sylius/sylius",
                "1.13"
            )
            .expect("url"),
            "https://api.github.com/repos/Sylius/SyliusRecipes/contents/sylius.sylius.1.13.json?ref=flex/main"
        );
        let absolute = serde_json::json!({"recipe_template": "https://x/{package}/{version}.json"});
        assert_eq!(
            recipe_url("https://y/index.json", &absolute, "a/b", "1.0").expect("url"),
            "https://x/a/b/1.0.json"
        );
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
            !bundle_classes_from(p, &ClassSource::LaidOut(&root))
                .expect("no error on a disk read")
                .is_empty()
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
                !bundle_classes_from(&package, &ClassSource::LaidOut(&root))
                    .expect("disk")
                    .is_empty(),
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
                !bundle_classes_from(
                    &package,
                    &ClassSource::Dist {
                        bytes: &zip_bytes,
                        kind: vivacity_core::store::DistKind::Zip,
                    },
                )
                .expect("dist read")
                .is_empty(),
                "{tag}: the dist does not answer like the laid-out package"
            );
            // Laid out somewhere else: Flex reads vendor/<name> and finds
            // nothing there.
            assert!(bundle_classes_from(&package, &ClassSource::Elsewhere)
                .expect("elsewhere")
                .is_empty());
        }
    }

    #[test]
    fn no_candidate_file_means_no_read_at_all() {
        // The guard that keeps a package with no dist to fetch — a
        // metapackage, a symfony-pack — from handing the command over for a
        // read `getClassNames` would never do.
        for empty in [
            serde_json::json!({}),
            serde_json::json!({"type": "metapackage"}),
            serde_json::json!({"autoload": {"files": ["src/fn.php"]}}),
            serde_json::json!({"autoload": {"classmap": ["src/"]}}),
            serde_json::json!({"autoload": {"psr-4": []}}),
        ] {
            assert!(
                bundle_candidate_paths(&empty).is_empty(),
                "{empty} should need no read"
            );
        }
        // `extractClassNames` yields several names per namespace, so several
        // files: the first is the one PHP opens first.
        let one = serde_json::json!({"autoload": {"psr-4": {"Acme\\FooBundle\\": "src"}}});
        assert_eq!(
            bundle_candidate_paths(&one),
            vec!["src/FooBundle.php", "src/AcmeFooBundle.php"]
        );
    }

    #[test]
    fn autoload_paths_follow_php_coercion() {
        // `if (!is_array($paths)) { $paths = [$paths]; }`, then used as a
        // string: a scalar is a path, not a reason to skip the namespace.
        let cases: [(serde_json::Value, &str); 3] = [
            (
                serde_json::json!({"autoload": {"psr-4": {"Acme\\": 5}}}),
                "5/AcmeBundle.php",
            ),
            (
                serde_json::json!({"autoload": {"psr-4": {"Acme\\": null}}}),
                "AcmeBundle.php",
            ),
            (
                serde_json::json!({"autoload": {"psr-4": {"Acme\\": ["lib", 7]}}}),
                "lib/AcmeBundle.php",
            ),
        ];
        for (package, first) in cases {
            let paths = bundle_candidate_paths(&package);
            assert_eq!(paths.first().map(String::as_str), Some(first), "{package}");
        }
    }

    #[test]
    fn a_multibyte_namespace_does_not_panic() {
        // `substr($suffix, -6)` counts bytes: on `ééééa` the sixth byte from
        // the end is a continuation byte, and slicing the &str panicked.
        for ns in ["Ééééa\\", "Bundle\\", "Ébundle\\", "é\\"] {
            let package = serde_json::json!({"autoload": {"psr-4": {ns: "src"}}});
            let paths = bundle_candidate_paths(&package);
            assert!(!paths.is_empty(), "{ns} yields a candidate");
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
