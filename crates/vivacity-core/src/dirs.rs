//! `config.vendor-dir` and `config.bin-dir`, resolved like
//! `Config::get('vendor-dir' | 'bin-dir')` (Composer 2.10.3):
//!
//! - the `COMPOSER_VENDOR_DIR` / `COMPOSER_BIN_DIR` environment first, else
//!   the project's `config`, else the global config, else the defaults
//!   `vendor` and `{$vendor-dir}/bin`;
//! - `{$vendor-dir}` / `{$bin-dir}` placeholders substituted (`process`),
//!   the result stripped of trailing `/` and `\` (`rtrim`), then made
//!   absolute against the project directory without canonicalisation
//!   (`Config::realpath`).
//!
//! Every consumer of the directory (`LibraryInstaller`, `BinaryInstaller`,
//! `AutoloadGenerator`, `FilesystemRepository`) `realpath`s it before
//! deriving a path written to disk, so the project-relative normalised form
//! kept here (`lib/vendor`, `vendor/bin`) is what those outputs depend on.
//!
//! The forms the differential harness does not cover are refused as scope
//! issues rather than half-supported: absolute paths, `..`-prefixed paths,
//! the project root itself, `~/`, `$VAR` / `%VAR%` prefixes
//! (`Platform::expandPath`), other placeholders, non-string values.

use crate::pathutil::{is_absolute_path, normalize_path};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The two directories, project-relative and normalised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    vendor_rel: String,
    bin_rel: String,
}

impl Default for Dirs {
    fn default() -> Self {
        Dirs {
            vendor_rel: "vendor".to_owned(),
            bin_rel: "vendor/bin".to_owned(),
        }
    }
}

impl Dirs {
    /// From the root manifest, the environment and the global config.
    pub fn resolve(root_manifest: &Value) -> Result<Dirs, String> {
        Self::resolve_with(
            root_manifest,
            |var| std::env::var(var).ok(),
            crate::layout::global_config_value,
        )
    }

    /// `resolve` with the environment and the global config injected.
    pub fn resolve_with(
        root_manifest: &Value,
        env: impl Fn(&str) -> Option<String>,
        global: impl Fn(&str) -> Option<Value>,
    ) -> Result<Dirs, String> {
        let raw = |key: &str, default: &str| -> Result<String, String> {
            let var = format!("COMPOSER_{}", key.to_ascii_uppercase().replace('-', "_"));
            // `getComposerEnv`: an unset or empty variable is no value.
            if let Some(v) = env(&var).filter(|v| !v.is_empty()) {
                return Ok(v);
            }
            let configured = root_manifest
                .get("config")
                .and_then(|c| c.get(key))
                .cloned()
                .or_else(|| global(key));
            match configured {
                None => Ok(default.to_owned()),
                Some(Value::String(s)) => Ok(s),
                Some(other) => Err(format!("config {key} {other} is not a string")),
            }
        };
        let vendor_raw = raw("vendor-dir", "vendor")?;
        let vendor_rel = relative("vendor-dir", &substitute("vendor-dir", &vendor_raw, None)?)?;
        let bin_raw = raw("bin-dir", "{$vendor-dir}/bin")?;
        let bin_rel = relative(
            "bin-dir",
            &substitute("bin-dir", &bin_raw, Some(&vendor_rel))?,
        )?;
        Ok(Dirs {
            vendor_rel,
            bin_rel,
        })
    }

    /// `vendor` by default.
    pub fn vendor_rel(&self) -> &str {
        &self.vendor_rel
    }

    /// `vendor/bin` by default.
    pub fn bin_rel(&self) -> &str {
        &self.bin_rel
    }

    pub fn vendor_dir(&self, root: &Path) -> PathBuf {
        root.join(&self.vendor_rel)
    }

    pub fn bin_dir(&self, root: &Path) -> PathBuf {
        root.join(&self.bin_rel)
    }

    /// `<vendor-dir>/composer`.
    pub fn composer_dir(&self, root: &Path) -> PathBuf {
        root.join(&self.vendor_rel).join("composer")
    }
}

/// `Config::process`: `{$key}` replaced by the resolved value of `key`.
/// Only `vendor-dir` is meaningful inside `bin-dir` (and the default
/// depends on it); anything else is refused.
fn substitute(key: &str, value: &str, vendor_rel: Option<&str>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("{$") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = &after[..end];
        match (name, vendor_rel) {
            ("vendor-dir", Some(v)) => out.push_str(v),
            _ => {
                return Err(format!(
                    "config {key} \"{value}\" uses the placeholder {{${name}}}, which is not supported natively"
                ))
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The project-relative normalised form, or the scope issue.
fn relative(key: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim_end_matches(['/', '\\']);
    let refuse = |why: &str| {
        Err(format!(
            "config {key} \"{value}\" is not supported natively ({why})"
        ))
    };
    if trimmed.starts_with("~/") || trimmed.starts_with("~\\") || trimmed == "~" {
        return refuse("home-relative path");
    }
    if trimmed.starts_with('$') || trimmed.starts_with('%') {
        return refuse("environment-variable prefix");
    }
    if is_absolute_path(trimmed) || (trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':') {
        return refuse("absolute path");
    }
    let rel = normalize_path(trimmed);
    if rel.is_empty() || rel == "." {
        return refuse("the project root itself");
    }
    if rel == ".." || rel.starts_with("../") {
        return refuse("outside the project");
    }
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolve(
        manifest: Value,
        env: &[(&str, &str)],
        global: Option<Value>,
    ) -> Result<Dirs, String> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Dirs::resolve_with(
            &manifest,
            |var| env.iter().find(|(k, _)| k == var).map(|(_, v)| v.clone()),
            |key| global.as_ref()?.get(key).cloned(),
        )
    }

    fn dirs(vendor: &str, bin: &str) -> Dirs {
        Dirs {
            vendor_rel: vendor.to_owned(),
            bin_rel: bin.to_owned(),
        }
    }

    #[test]
    fn defaults() {
        assert_eq!(resolve(json!({}), &[], None).unwrap(), Dirs::default());
        assert_eq!(Dirs::default(), dirs("vendor", "vendor/bin"));
    }

    #[test]
    fn project_config_trailing_slashes_and_dot_prefix() {
        let m =
            json!({"config": {"vendor-dir": "upload/system/storage/vendor/", "bin-dir": "bin/"}});
        assert_eq!(
            resolve(m, &[], None).unwrap(),
            dirs("upload/system/storage/vendor", "bin")
        );
        let m = json!({"config": {"vendor-dir": "./concrete/vendor"}});
        assert_eq!(
            resolve(m, &[], None).unwrap(),
            dirs("concrete/vendor", "concrete/vendor/bin")
        );
        let m = json!({"config": {"vendor-dir": "./vendor/composer/vendor"}});
        assert_eq!(
            resolve(m, &[], None).unwrap(),
            dirs("vendor/composer/vendor", "vendor/composer/vendor/bin")
        );
    }

    #[test]
    fn precedence_env_then_project_then_global() {
        let m = json!({"config": {"vendor-dir": "lib/vendor"}});
        let g = json!({"vendor-dir": "global/vendor", "bin-dir": "gbin"});
        assert_eq!(
            resolve(m.clone(), &[], Some(g.clone())).unwrap(),
            dirs("lib/vendor", "gbin")
        );
        assert_eq!(
            resolve(
                m.clone(),
                &[("COMPOSER_VENDOR_DIR", "env-vendor")],
                Some(g.clone())
            )
            .unwrap(),
            dirs("env-vendor", "gbin")
        );
        assert_eq!(
            resolve(
                m,
                &[("COMPOSER_VENDOR_DIR", ""), ("COMPOSER_BIN_DIR", "b/")],
                Some(g)
            )
            .unwrap(),
            dirs("lib/vendor", "b")
        );
    }

    #[test]
    fn placeholder() {
        let m = json!({"config": {"bin-dir": "{$vendor-dir}/tools"}});
        assert_eq!(
            resolve(m, &[("COMPOSER_VENDOR_DIR", "env-vendor")], None).unwrap(),
            dirs("env-vendor", "env-vendor/tools")
        );
        let m = json!({"config": {"vendor-dir": "{$home}/v"}});
        assert!(resolve(m, &[], None).unwrap_err().contains("{$home}"));
    }

    #[test]
    fn refused_forms() {
        for v in [
            "/abs/vendor",
            "C:/vendor",
            "c:vendor",
            "../vendor",
            "..",
            ".",
            "./",
            "~/vendor",
            "$HOME/v",
            "%HOME%/v",
            "",
        ] {
            let m = json!({"config": {"vendor-dir": v}});
            let err = resolve(m, &[], None).unwrap_err();
            assert!(err.contains("not supported natively"), "{v}: {err}");
        }
        let m = json!({"config": {"vendor-dir": 3}});
        assert!(resolve(m, &[], None).unwrap_err().contains("not a string"));
    }
}
