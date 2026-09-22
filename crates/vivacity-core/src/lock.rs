//! Reading composer.lock. Minimal typed view over the raw JSON:
//! `installed.json`/`installed.php` will have to serve the entries back
//! **unchanged**, so each package keeps its raw value (`raw`) and only exposes
//! as typed fields what the installer needs.

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Lock {
    pub content_hash: Option<String>,
    pub packages: Vec<LockPackage>,
    pub packages_dev: Vec<LockPackage>,
    /// Project platform constraints (php, ext-*, lib-*) -> constraint.
    pub platform: Vec<(String, String)>,
    pub platform_dev: Vec<(String, String)>,
    pub plugin_api_version: Option<String>,
    pub aliases: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct LockPackage {
    pub raw: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistKind {
    Zip,
    /// `TarDownloader` (asset-packagist's npm tarballs).
    Tar,
    /// A `path` repository package: `dist.url` is the source directory.
    Path,
    Other,
    Missing,
}

impl LockPackage {
    fn str_field(&self, key: &str) -> Option<&str> {
        self.raw.get(key).and_then(Value::as_str)
    }

    pub fn name(&self) -> &str {
        self.str_field("name").unwrap_or("")
    }

    pub fn version(&self) -> &str {
        self.str_field("version").unwrap_or("")
    }

    /// Package type, Composer default: "library".
    pub fn package_type(&self) -> &str {
        self.str_field("type").unwrap_or("library")
    }

    pub fn dist_url(&self) -> Option<&str> {
        self.raw.get("dist")?.get("url")?.as_str()
    }

    /// `Package::getDistUrls`: a dist url holding `%` goes through
    /// `ComposerMirror::processUrl` — `%package%`, `%version%` (normalized),
    /// `%reference%`, `%type%` and `%prettyVersion%` are substituted (a
    /// `package` repository entry such as
    /// `https://host/archive/%prettyVersion%.zip`).
    pub fn dist_url_expanded(&self) -> Option<String> {
        let url = self.dist_url()?;
        if !url.contains('%') {
            return Some(url.to_owned());
        }
        let version = crate::version::normalize_pretty(self.version())
            .unwrap_or_else(|_| self.version().to_owned());
        let kind = self
            .raw
            .get("dist")
            .and_then(|d| d.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("");
        Some(
            url.replace("%package%", self.name())
                .replace("%version%", &version)
                .replace("%reference%", self.dist_reference().unwrap_or(""))
                .replace("%type%", kind)
                .replace("%prettyVersion%", self.version()),
        )
    }

    pub fn dist_reference(&self) -> Option<&str> {
        self.raw.get("dist")?.get("reference")?.as_str()
    }

    /// sha1 shasum of the dist if non-empty (often empty on Packagist).
    pub fn dist_shasum(&self) -> Option<&str> {
        self.raw
            .get("dist")?
            .get("shasum")?
            .as_str()
            .filter(|s| !s.is_empty())
    }

    /// What identifies the dist's content in the store: its reference,
    /// or — a `tar` dist from asset-packagist has none — the sha1 of its
    /// URL, as Composer's own cache keys it (two URLs for one version must
    /// not share an entry).
    pub fn store_reference(&self) -> Option<String> {
        match self.dist_reference() {
            Some(r) => Some(r.to_owned()),
            None => self.dist_url().map(|u| {
                use sha1::Digest as _;
                sha1::Sha1::digest(u.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            }),
        }
    }

    pub fn dist_kind(&self) -> DistKind {
        match self
            .raw
            .get("dist")
            .and_then(|d| d.get("type"))
            .and_then(Value::as_str)
        {
            Some("zip") => DistKind::Zip,
            Some("tar") => DistKind::Tar,
            Some("path") => DistKind::Path,
            Some(_) => DistKind::Other,
            None => DistKind::Missing,
        }
    }

    /// `target-dir` (legacy PSR-0): the package installs into
    /// vendor/<name>/<target-dir>.
    pub fn target_dir(&self) -> Option<&str> {
        self.str_field("target-dir")
            .map(|t| t.trim_matches('/'))
            .filter(|t| !t.is_empty())
    }

    /// Install path relative to vendor/ (`name` or `name/target-dir`).
    pub fn install_subpath(&self) -> String {
        match self.target_dir() {
            Some(t) => format!("{}/{}", self.name(), t),
            None => self.name().to_owned(),
        }
    }

    pub fn is_metapackage(&self) -> bool {
        self.package_type() == "metapackage"
    }

    /// Installed nowhere: a metapackage, or a `symfony-pack` when Flex is
    /// active (its `SymfonyPackInstaller extends MetapackageInstaller`).
    pub fn is_virtual(&self, flex_packs: bool) -> bool {
        self.is_metapackage() || (flex_packs && self.package_type() == "symfony-pack")
    }

    pub fn bins(&self) -> Vec<&str> {
        self.raw
            .get("bin")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
}

fn parse_packages(v: Option<&Value>) -> Vec<LockPackage> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_object())
                .map(|m| LockPackage { raw: m.clone() })
                .collect()
        })
        .unwrap_or_default()
}

/// `platform` is `{}` or `{"php": ">=8.2", "ext-mbstring": "*"}` and, PHP
/// encoding quirk, sometimes `[]` (empty array) when there is nothing.
fn parse_platform(v: Option<&Value>) -> Vec<(String, String)> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

impl Lock {
    pub fn parse(text: &str) -> Result<Self> {
        let v: Value = serde_json::from_str(text).map_err(|source| Error::Json {
            context: "composer.lock".to_owned(),
            source,
        })?;
        Ok(Self::from_value(&v))
    }

    /// From an already parsed lock (a caller that also needs the `Value`
    /// parses once).
    pub fn from_value(v: &Value) -> Self {
        Lock {
            content_hash: v
                .get("content-hash")
                .and_then(Value::as_str)
                .map(str::to_owned),
            packages: parse_packages(v.get("packages")),
            packages_dev: parse_packages(v.get("packages-dev")),
            platform: parse_platform(v.get("platform")),
            platform_dev: parse_platform(v.get("platform-dev")),
            plugin_api_version: v
                .get("plugin-api-version")
                .and_then(Value::as_str)
                .map(str::to_owned),
            aliases: v
                .get("aliases")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Packages to install according to --no-dev.
    /// Whether `symfony-pack` packages are virtual for this install: Flex
    /// is in the lock, plugins are enabled and Flex is allowed (Composer
    /// loads it and it registers the pack installer).
    pub fn flex_packs(&self, root_manifest: &Value, with_dev: bool, plugins_enabled: bool) -> bool {
        plugins_enabled
            && self
                .wanted_packages(with_dev)
                .any(|p| p.name() == "symfony/flex")
            && matches!(
                crate::layout::plugin_allowed(root_manifest, "symfony/flex"),
                crate::layout::PluginVerdict::Allowed
            )
    }

    pub fn wanted_packages(&self, with_dev: bool) -> impl Iterator<Item = &LockPackage> {
        self.packages
            .iter()
            .chain(
                self.packages_dev
                    .iter()
                    .take(if with_dev { usize::MAX } else { 0 }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_lock() {
        let lock = Lock::parse(
            r#"{"content-hash":"abc","packages":[{"name":"a/b","version":"1.0.0",
                "dist":{"type":"zip","url":"https://x/y.zip","reference":"deadbeef","shasum":""},
                "type":"library","bin":["bin/tool"]}],
               "packages-dev":[],"platform":{"php":">=8.1"},"platform-dev":[]}"#,
        )
        .expect("parse");
        assert_eq!(lock.content_hash.as_deref(), Some("abc"));
        let p = &lock.packages[0];
        assert_eq!(p.name(), "a/b");
        assert_eq!(p.dist_kind(), DistKind::Zip);
        assert_eq!(p.dist_shasum(), None); // empty -> None
        assert_eq!(p.bins(), vec!["bin/tool"]);
        assert_eq!(lock.platform, vec![("php".to_owned(), ">=8.1".to_owned())]);
        assert_eq!(lock.wanted_packages(false).count(), 1);
    }

    #[test]
    fn empty_platform_as_array() {
        let lock = Lock::parse(r#"{"packages":[],"platform":[]}"#).expect("parse");
        assert!(lock.platform.is_empty());
    }
}

#[cfg(test)]
mod placeholder_tests {
    use super::*;

    #[test]
    fn dist_url_placeholders_like_composer_mirror() {
        let p = LockPackage {
            raw: serde_json::from_str(r#"{"name": "ssddanbrown/asserthtml", "version": "v3.2.0",
                "dist": {"type": "zip", "url": "https://codeberg.org/api/v1/repos/%package%/archive/%prettyVersion%.zip?r=%reference%&t=%type%&v=%version%", "reference": "0811b5c"}}"#).unwrap(),
        };
        assert_eq!(
            p.dist_url_expanded().unwrap(),
            "https://codeberg.org/api/v1/repos/ssddanbrown/asserthtml/archive/v3.2.0.zip?r=0811b5c&t=zip&v=3.2.0.0"
        );
    }
}
