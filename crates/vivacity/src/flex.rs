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
    let merged = FlexVersions::merge_endpoints(&indexes);
    FlexVersions::from_value(&merged).map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
