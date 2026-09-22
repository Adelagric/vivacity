//! Emulation of `composer/package-versions-deprecated`
//! (docs/reference/plugins/package-versions-deprecated/, MIT), read at
//! 1.11.99.5: on `POST_AUTOLOAD_DUMP` its `Installer` rewrites
//! `<vendor-dir>/composer/package-versions-deprecated/src/PackageVersions/Versions.php`
//! — the shipped file is a stub that says so — with the root package's
//! name and a `name => version@reference` map of everything the lock
//! holds. Without it, a project that reads `PackageVersions\Versions`
//! gets the stub's fallback instead of its own versions.
//!
//! The map, in `Installer::getVersions` order (an array built by a
//! generator, so a later key overwrites an earlier one): the lock's
//! `packages`, then `packages-dev` unless the run is `--no-dev`
//! (`COMPOSER_DEV_MODE`), each as `version@(source.reference ??
//! dist.reference ?? '')`; then the root's `replace` targets
//! (`self.version` resolved to the root's pretty version) at the root's
//! source reference; then the root itself. The file is written 0664
//! through a temporary file and a rename, and only when its directory
//! exists — a package scheduled for removal is skipped.

use crate::error::{Error, Result};
use crate::lock::Lock;
use serde_json::Value;
use std::path::Path;

pub const PLUGIN_NAME: &str = "composer/package-versions-deprecated";
/// `Installer::$generatedClassTemplate`, verbatim.
const TEMPLATE: &str = include_str!("../assets/package-versions-template.php");

/// `getVersions`, as `iterator_to_array` flattens it: in order, with a
/// later entry winning.
pub fn versions(
    lock: &Lock,
    with_dev: bool,
    root_name: &str,
    root_pretty_version: &str,
    root_source_reference: &str,
    root_replaces: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |name: String, value: String| match out.iter_mut().find(|(n, _)| *n == name) {
        Some(slot) => slot.1 = value,
        None => out.push((name, value)),
    };
    for p in lock.wanted_packages(with_dev) {
        let reference = p
            .raw
            .get("source")
            .and_then(|s| s.get("reference"))
            .and_then(Value::as_str)
            .or_else(|| {
                p.raw
                    .get("dist")
                    .and_then(|d| d.get("reference"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("");
        push(p.name().to_owned(), format!("{}@{reference}", p.version()));
    }
    for (target, constraint) in root_replaces {
        let version = if constraint == "self.version" {
            root_pretty_version
        } else {
            constraint
        };
        push(target.clone(), format!("{version}@{root_source_reference}"));
    }
    push(
        root_name.to_owned(),
        format!("{root_pretty_version}@{root_source_reference}"),
    );
    out
}

/// `generateVersionsClass`: the template with the class declaration, the
/// root name and `var_export` of the map.
pub fn render(root_name: &str, versions: &[(String, String)]) -> String {
    let map = Value::Object(
        versions
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    );
    let exported = crate::runtime_stub::php_var_export(&map, 0);
    // `sprintf`: the class declaration (split in the source to dodge
    // regex-based parsers), the root name, the exported map.
    TEMPLATE
        .replacen("%s", "final class Versions", 1)
        .replacen("%s", &php_single_quoted(root_name), 1)
        .replacen("%s", &exported, 1)
}

/// The `'%s'` of the template is already quoted: only the content is
/// escaped, as PHP's own single-quoted strings are.
fn php_single_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// `writeVersionClassToFile`: 0664, temporary file then rename, and
/// nothing at all when the package's directory is gone.
pub fn write(vendor: &Path, content: &str) -> Result<bool> {
    let dir = vendor
        .join("composer")
        .join("package-versions-deprecated")
        .join("src")
        .join("PackageVersions");
    if !dir.is_dir() {
        return Ok(false);
    }
    let target = dir.join("Versions.php");
    let tmp = dir.join(format!("Versions.php_tmp{}", std::process::id()));
    std::fs::write(&tmp, content).map_err(Error::io(&tmp))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o664))
            .map_err(Error::io(&tmp))?;
    }
    std::fs::rename(&tmp, &target).map_err(Error::io(&target))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lock_of(packages: Value, dev: Value) -> Lock {
        Lock::parse(&json!({"packages": packages, "packages-dev": dev}).to_string()).expect("lock")
    }

    #[test]
    fn versions_follow_the_generator() {
        let lock = lock_of(
            json!([
                {"name": "a/b", "version": "1.0.0", "source": {"reference": "src1"}, "dist": {"reference": "dist1"}},
                {"name": "c/d", "version": "2.0.0", "dist": {"reference": "dist2"}},
                {"name": "e/f", "version": "3.0.0"}
            ]),
            json!([{"name": "g/h", "version": "4.0.0", "source": {"reference": "src4"}}]),
        );
        let replaces = [
            ("x/y".to_owned(), "self.version".to_owned()),
            ("z/w".to_owned(), "1.2".to_owned()),
        ];
        let v = versions(&lock, true, "root/pkg", "dev-main", "rootref", &replaces);
        assert_eq!(
            v,
            vec![
                ("a/b".to_owned(), "1.0.0@src1".to_owned()),
                ("c/d".to_owned(), "2.0.0@dist2".to_owned()),
                ("e/f".to_owned(), "3.0.0@".to_owned()),
                ("g/h".to_owned(), "4.0.0@src4".to_owned()),
                ("x/y".to_owned(), "dev-main@rootref".to_owned()),
                ("z/w".to_owned(), "1.2@rootref".to_owned()),
                ("root/pkg".to_owned(), "dev-main@rootref".to_owned()),
            ],
            "source reference first, then dist, then empty; dev last; replaces then root"
        );
        // `--no-dev`: `COMPOSER_DEV_MODE=0` drops packages-dev.
        let v = versions(&lock, false, "root/pkg", "dev-main", "rootref", &[]);
        assert!(!v.iter().any(|(n, _)| n == "g/h"));
    }

    #[test]
    fn rendered_file_is_the_template() {
        let out = render("root/pkg", &[("a/b".to_owned(), "1.0.0@ref".to_owned())]);
        assert!(out.starts_with("<?php\n\ndeclare(strict_types=1);"));
        assert!(out.contains("final class Versions\n{"));
        assert!(out.contains("const ROOT_PACKAGE_NAME = 'root/pkg';"));
        assert!(
            out.contains("const VERSIONS          = array (\n  'a/b' => '1.0.0@ref',\n);"),
            "{out}"
        );
        assert!(!out.contains("%s"), "every placeholder is filled");
        // A quote in the root name is escaped like PHP's var_export.
        assert!(render("o'brien/pkg", &[]).contains("ROOT_PACKAGE_NAME = 'o\\'brien/pkg';"));
    }
}
