//! Port of `Symfony\Flex\PackageFilter` (symfony/flex 2.11): Flex's
//! `PRE_POOL_CREATE` listener (`Flex::truncatePackages`) that removes from
//! the pool, before it is created, the versions of the packages listed in
//! `symfony/symfony` that do not match `extra.symfony.require`
//! (`SYMFONY_REQUIRE` wins; a trailing `.x` becomes `.x-dev`). The data
//! comes from the Flex endpoints' `index.json`, key `versions`
//! (`splits`: package → versions it ships in; `next`: the branch `.x`
//! resolves to), merged first endpoint wins. Fetching and caching are the
//! command layer's business (`vivacity::flex`); this module is pure.
//!
//! `ignorePreleases` (Composer < 2.9) is not ported: on 2.9+ Flex sets
//! `COMPOSER_PREFER_DEV_OVER_PRERELEASE` instead, which the session applies
//! to the policy.

use crate::constraint::{Constraint, Op};
use crate::intervals;
use crate::package::Package;
use crate::version;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// The `versions` entry of the Flex index.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlexVersions {
    /// `splits`: package name → the `<major>.<minor>` (or `<major>.x`)
    /// versions it is split from (insertion order kept for the pruning).
    pub splits: Vec<(String, Vec<String>)>,
    /// `next`: what `.x` stands for.
    pub next: String,
}

impl FlexVersions {
    /// From the merged `versions` object (`splits` required — Flex throws
    /// `The Flex index is missing a "splits" entry` otherwise).
    pub fn from_value(v: &Value) -> Result<FlexVersions, String> {
        let Some(splits) = v.get("splits").and_then(Value::as_object) else {
            return Err("The Flex index is missing a \"splits\" entry. Did you forget to add \"flex://defaults\" in the \"extra.symfony.endpoint\" array of your composer.json?".to_owned());
        };
        let mut out = Vec::new();
        for (name, vers) in splits {
            let list: Vec<String> = vers
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            out.push((name.clone(), list));
        }
        Ok(FlexVersions {
            splits: out,
            next: v
                .get("next")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        })
    }

    /// `Downloader::initialize`'s `self::$versions += $config['versions']`
    /// over the endpoints in order: the first endpoint's keys win.
    pub fn merge_endpoints(indexes: &[Value]) -> Value {
        let mut merged = serde_json::Map::new();
        for index in indexes {
            if let Some(v) = index.get("versions").and_then(Value::as_object) {
                for (k, val) in v {
                    merged.entry(k.clone()).or_insert_with(|| val.clone());
                }
            }
        }
        Value::Object(merged)
    }

    /// `PackageFilter::getVersions`: the splits pruned to the versions
    /// whose `<v>.N.0` (`.x` → `next`) matches the constraint, up to 60
    /// patch levels; an entry whose list is empty or unchanged is dropped
    /// — a package absent from `splits` is then kept by the filter.
    pub fn pruned(&self, symfony: &Constraint) -> FlexVersions {
        let mut ok: HashMap<&str, bool> = HashMap::new();
        let mut splits = Vec::new();
        for (name, vers) in &self.splits {
            let mut kept: Vec<String> = Vec::new();
            for v in vers {
                let is_ok = *ok.entry(v.as_str()).or_insert_with(|| {
                    let w = if v.ends_with(".x") {
                        self.next.as_str()
                    } else {
                        v.as_str()
                    };
                    (0..60).any(|j| symfony.matches_version(&norm_or_raw(&format!("{w}.{j}.0"))))
                });
                if is_ok {
                    kept.push(v.clone());
                }
            }
            if !kept.is_empty() && &kept != vers {
                splits.push((name.clone(), kept));
            }
        }
        FlexVersions {
            splits,
            next: self.next.clone(),
        }
    }

    fn has(&self, name: &str) -> bool {
        self.splits.iter().any(|(n, _)| n == name)
    }
}

fn norm_or_raw(v: &str) -> String {
    version::normalize(v, None).unwrap_or_else(|_| v.to_owned())
}

/// `Flex::activate`: the requirement, `SYMFONY_REQUIRE` over
/// `extra.symfony.require`, `.x` → `.x-dev`. `None` when empty (no filter).
pub fn symfony_require(manifest: &Value) -> Option<String> {
    let from_env = std::env::var("SYMFONY_REQUIRE")
        .ok()
        .filter(|s| !s.is_empty());
    let raw = from_env.or_else(|| {
        manifest
            .get("extra")?
            .get("symfony")?
            .get("require")?
            .as_str()
            .map(str::to_owned)
    })?;
    if raw.is_empty() {
        return None;
    }
    Some(match raw.strip_suffix(".x") {
        Some(base) => format!("{base}.x-dev"),
        None => raw,
    })
}

/// `PackageFilter::removeLegacyPackages`. `packages` are pool candidates
/// (arena indices, aliases included); `root_constraints` is
/// `getRequires() + getDevRequires()` (require wins on a name in both);
/// `locked` the versions of `getFixedOrLockedPackages()` by name (an
/// alias contributes the aliased version too); `versions` the index as
/// fetched — pruned here (`getVersions`), so a package whose every split
/// version matches the requirement, or none does, is not filtered.
/// Returns the kept indices in order, and whether the `Restricting
/// packages…` notice is due (once, when a package other than
/// `symfony/symfony` loses a version to the constraint).
pub fn remove_legacy_packages(
    packages: &[usize],
    arena: &[Package],
    root_constraints: &BTreeMap<String, Constraint>,
    locked: &BTreeMap<String, Vec<String>>,
    symfony: &Constraint,
    versions: &FlexVersions,
) -> (Vec<usize>, bool) {
    let versions = versions.pruned(symfony);
    let mut kept: Vec<usize> = Vec::new();
    let mut symfony_packages: Vec<usize> = Vec::new();
    let mut one_symfony = false;
    let mut restricting = false;
    for &idx in packages {
        let p = &arena[idx];
        let name = p.name.as_str();
        let mut candidate_versions: Vec<String> = vec![p.version.clone()];
        if let Some(base) = p.alias_of {
            candidate_versions.push(arena[base].version.clone());
        }
        let locked_hit = locked
            .get(name)
            .is_some_and(|l| candidate_versions.iter().any(|v| l.contains(v)));
        let root_disjoint = root_constraints
            .get(name)
            .is_some_and(|c| !intervals::have_intersections(symfony, c));
        let bridge =
            name == "symfony/psr-http-message-bridge" && php_float(&candidate_versions[0]) < 6.4;
        if name != "symfony/symfony"
            && (locked_hit || !versions.has(name) || root_disjoint || bridge)
        {
            kept.push(idx);
            continue;
        }
        // `$package->getExtra()['branch-alias'][$package->getVersion()]`
        if let Some(alias) = p
            .raw
            .get("extra")
            .and_then(|e| e.get("branch-alias"))
            .and_then(|b| b.get(&p.version))
            .and_then(Value::as_str)
        {
            candidate_versions.push(norm_or_raw(alias));
        }
        if candidate_versions.iter().any(|v| {
            symfony.matches(&Constraint::Single {
                op: Op::Eq,
                version: v.clone(),
            })
        }) {
            kept.push(idx);
            one_symfony = one_symfony || name == "symfony/symfony";
            continue;
        }
        if name == "symfony/symfony" {
            symfony_packages.push(idx);
        } else {
            restricting = true;
        }
    }
    if !symfony_packages.is_empty() && !one_symfony {
        kept.extend(symfony_packages);
    }
    (kept, restricting)
}

/// PHP's `6.4 > $versions[0]`: the normalized version string cast to
/// float (`"7.1.0.0"` → 7.1, `"dev-main"` → 0).
fn php_float(version: &str) -> f64 {
    let mut end = 0;
    let bytes = version.as_bytes();
    let mut seen_dot = false;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_digit() {
            end += 1;
        } else if b == b'.' && !seen_dot {
            seen_dot = true;
            end += 1;
        } else {
            break;
        }
    }
    version[..end].parse::<f64>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint::parse_constraints;
    use crate::package::Origin;
    use serde_json::json;

    fn cons(s: &str) -> Constraint {
        parse_constraints(s).expect("constraint").constraint
    }

    fn pkg(name: &str, version: &str) -> Package {
        let normalized = norm_or_raw(version);
        Package::new(name, &normalized, version, Origin::Repository(0))
    }

    #[test]
    fn symfony_require_env_wins_and_x_becomes_x_dev() {
        let m = json!({"extra": {"symfony": {"require": "7.1.*"}}});
        std::env::remove_var("SYMFONY_REQUIRE");
        assert_eq!(symfony_require(&m).as_deref(), Some("7.1.*"));
        let m = json!({"extra": {"symfony": {"require": "7.x"}}});
        assert_eq!(symfony_require(&m).as_deref(), Some("7.x-dev"));
        assert_eq!(symfony_require(&json!({})), None);
    }

    #[test]
    fn pruning_keeps_the_matching_splits_and_drops_unchanged_entries() {
        let v = FlexVersions {
            splits: vec![
                (
                    "symfony/console".into(),
                    vec!["6.4".into(), "7.0".into(), "7.1".into()],
                ),
                ("symfony/only-old".into(), vec!["5.4".into()]),
                ("symfony/next".into(), vec!["7.x".into(), "7.1".into()]),
                ("symfony/all-ok".into(), vec!["7.1".into()]),
            ],
            next: "7.2".into(),
        };
        let pruned = v.pruned(&cons("7.1.*"));
        // console: 7.1 kept only; only-old: nothing → dropped; next: 7.x →
        // 7.2 no, 7.1 yes → kept as [7.1]; all-ok: unchanged → dropped.
        assert_eq!(
            pruned.splits,
            vec![
                ("symfony/console".to_string(), vec!["7.1".to_string()]),
                ("symfony/next".to_string(), vec!["7.1".to_string()]),
            ]
        );
    }

    #[test]
    fn legacy_packages_are_removed_and_the_notice_is_due() {
        let arena = vec![
            pkg("symfony/console", "v7.0.3"),
            pkg("symfony/console", "v7.1.0"),
            pkg("symfony/not-split", "v1.0.0"),
            pkg("symfony/locked", "v6.4.0"),
            pkg("symfony/symfony", "v7.0.0"),
            pkg("symfony/psr-http-message-bridge", "v2.3.1"),
            pkg("symfony/all-in-range", "v6.0.0"),
        ];
        let packages: Vec<usize> = (0..arena.len()).collect();
        let versions = FlexVersions {
            splits: vec![
                (
                    "symfony/console".into(),
                    vec!["6.4".into(), "7.0".into(), "7.1".into()],
                ),
                ("symfony/locked".into(), vec!["6.4".into(), "7.1".into()]),
                (
                    "symfony/psr-http-message-bridge".into(),
                    vec!["6.4".into(), "7.1".into()],
                ),
                // Every split version matches: pruned away, never filtered.
                ("symfony/all-in-range".into(), vec!["7.1".into()]),
            ],
            next: "7.2".into(),
        };
        let mut locked = BTreeMap::new();
        locked.insert("symfony/locked".to_string(), vec!["6.4.0.0".to_string()]);
        let (kept, notice) = remove_legacy_packages(
            &packages,
            &arena,
            &BTreeMap::new(),
            &locked,
            &cons("7.1.*"),
            &versions,
        );
        // console 7.0.3 dropped; 7.1.0 kept; not-split kept (absent from
        // splits); locked kept (locked version); bridge < 6.4 kept;
        // all-in-range kept (pruned away); symfony/symfony 7.0.0 appended
        // because no symfony/symfony matched.
        assert_eq!(kept, vec![1, 2, 3, 5, 6, 4]);
        assert!(notice);
    }

    #[test]
    fn root_constraint_disjoint_from_symfony_require_keeps_the_package() {
        let arena = vec![pkg("symfony/console", "v6.4.0")];
        let versions = FlexVersions {
            splits: vec![("symfony/console".into(), vec!["6.4".into(), "7.1".into()])],
            next: "7.2".into(),
        };
        let mut root = BTreeMap::new();
        root.insert("symfony/console".to_string(), cons("^6.4"));
        let (kept, notice) = remove_legacy_packages(
            &[0],
            &arena,
            &root,
            &BTreeMap::new(),
            &cons("7.1.*"),
            &versions,
        );
        assert_eq!(kept, vec![0]);
        assert!(!notice);
    }

    #[test]
    fn merge_endpoints_first_wins() {
        let a = json!({"versions": {"next": "7.2", "splits": {"x": ["7.1"]}}});
        let b = json!({"versions": {"next": "9.9", "lts": "6.4"}});
        let m = FlexVersions::merge_endpoints(&[a, b]);
        assert_eq!(m["next"], "7.2");
        assert_eq!(m["lts"], "6.4");
        assert!(FlexVersions::from_value(&m).is_ok());
        assert!(FlexVersions::from_value(&json!({"next": "7.2"})).is_err());
    }

    #[test]
    fn php_float_cast() {
        assert_eq!(php_float("7.1.0.0"), 7.1);
        assert_eq!(php_float("2.3.1.0"), 2.3);
        assert_eq!(php_float("dev-main"), 0.0);
        assert_eq!(php_float("9999999-dev"), 9999999.0);
    }
}
