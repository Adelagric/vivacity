//! php-http/discovery's `POST_UPDATE_CMD` listener, `Plugin::postUpdate`
//! (src/Composer/Plugin.php:145-240 at 1.20.0): whether it acts — edits
//! composer.json and runs a cloned `Installer` — on a local repository.
//! vivacity hands such a run to Composer; this module only answers the
//! question, by porting `getMissingRequires` (:243-366) and the
//! `array_filter` that follows it. The file is the same from 1.19.0 to
//! 1.20.0 but for `updateComposerLock`, which runs after the decision.
//!
//! PHP details kept, each covered by a test below:
//! - `$allPackages[$name] = true`, then the replaced names with `false`
//!   (`getNames(false)`: no `provide`), and `isset` is true for `false`;
//! - `$devPackages += $names` adds integer keys: no name ever enters;
//! - a package requiring php-http/discovery adds its `require` to the root
//!   one (or the dev one, when it is a dev package) — `+=`, keys only;
//! - the root's lists count only when its `require` names php-http/discovery;
//! - `findPackage` by exact name, alias versions included;
//! - `$missingRequires[1] = array_diff_key($missingRequires[1],
//!   $missingRequires[0])` before the lists are flattened.
//!
//! The Phalcon check (`class_exists(Phalcon\Http\Message\RequestFactory)`)
//! can only stop it from acting: not read, at worst a hand-over too many.

use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) const DISCOVERY: &str = "php-http/discovery";

/// The versions whose `getMissingRequires` and `postUpdate` decision were
/// read (identical from 1.19.0 to 1.20.0).
pub(crate) const PORTED_VERSIONS: &[&str] =
    &["1.19.0", "1.19.1", "1.19.2", "1.19.3", "1.19.4", "1.20.0"];

/// The older versions whose `postUpdate` was read without being ported:
/// a plugin from 1.15.0, the same five abstractions up to 1.18.1. There,
/// anything to look after hands the command over.
const READ_VERSIONS: &[&str] = &[
    "1.15.0", "1.15.1", "1.15.2", "1.15.3", "1.16.0", "1.17.0", "1.18.0", "1.18.1",
];

/// The hand-over reason when php-http/discovery (`version`, active at
/// `POST_UPDATE_CMD`) would act on `repo`: the ported decision for the
/// versions read in full; for the older ones, as soon as it looks after an
/// abstraction (`watches`); for any other, always. `what` ends the reason.
pub(crate) fn post_update_issue(
    version: Option<&str>,
    repo: &[RepoPackage],
    dev_names: &BTreeSet<String>,
    root: &Value,
    watches: impl FnOnce() -> bool,
    what: &str,
) -> Option<vivacity_core::scope::ScopeIssue> {
    let bare = version.map(|v| v.trim_start_matches('v'));
    let why = if bare.is_some_and(|v| PORTED_VERSIONS.contains(&v)) {
        if !would_act(repo, dev_names, root) {
            return None;
        }
        format!("would add or move a PSR implementation at post-update-cmd, edit composer.json and update again ({what})")
    } else if bare.is_some_and(|v| READ_VERSIONS.contains(&v)) {
        if !watches() {
            return None;
        }
        format!("looks for PSR implementations in the installed packages at post-update-cmd, and may edit composer.json and update again ({what})")
    } else {
        format!(
            "{} listens to post-update-cmd in a version not read (versions read: {}, {}; {what})",
            version.unwrap_or("?"),
            READ_VERSIONS.join(", "),
            PORTED_VERSIONS.join(", ")
        )
    };
    Some(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
        DISCOVERY.to_owned(),
        why,
    ))
}

type Candidates = &'static [(&'static str, &'static [&'static str])];

/// `Plugin::PROVIDE_RULES`, in order.
const PROVIDE_RULES: &[(&str, Candidates)] = &[
    (
        "php-http/async-client-implementation",
        &[
            (
                "symfony/http-client:>=6.3",
                &[
                    "guzzlehttp/promises",
                    "psr/http-factory-implementation",
                    "php-http/httplug",
                ],
            ),
            (
                "symfony/http-client",
                &[
                    "guzzlehttp/promises",
                    "php-http/message-factory",
                    "psr/http-factory-implementation",
                    "php-http/httplug",
                ],
            ),
            ("php-http/guzzle7-adapter", &[]),
            ("php-http/guzzle6-adapter", &[]),
            ("php-http/curl-client", &[]),
            ("php-http/react-adapter", &[]),
        ],
    ),
    (
        "php-http/client-implementation",
        &[
            (
                "symfony/http-client:>=6.3",
                &["psr/http-factory-implementation", "php-http/httplug"],
            ),
            (
                "symfony/http-client",
                &[
                    "php-http/message-factory",
                    "psr/http-factory-implementation",
                    "php-http/httplug",
                ],
            ),
            ("php-http/guzzle7-adapter", &[]),
            ("php-http/guzzle6-adapter", &[]),
            ("php-http/cakephp-adapter", &[]),
            ("php-http/curl-client", &[]),
            ("php-http/react-adapter", &[]),
            ("php-http/buzz-adapter", &[]),
            ("php-http/artax-adapter", &[]),
            ("kriswallsmith/buzz:^1", &[]),
        ],
    ),
    (
        "psr/http-client-implementation",
        &[
            (
                "symfony/http-client",
                &["psr/http-factory-implementation", "psr/http-client"],
            ),
            ("guzzlehttp/guzzle", &[]),
            ("kriswallsmith/buzz:^1", &[]),
        ],
    ),
    (
        "psr/http-message-implementation",
        &[("php-http/discovery", &["psr/http-factory-implementation"])],
    ),
    (
        "psr/http-factory-implementation",
        &[
            ("nyholm/psr7", &[]),
            ("guzzlehttp/psr7:>=2", &[]),
            ("slim/psr7", &[]),
            ("laminas/laminas-diactoros", &[]),
            ("phalcon/cphalcon:^4", &[]),
            ("http-interop/http-factory-guzzle", &[]),
            ("http-interop/http-factory-diactoros", &[]),
            ("http-interop/http-factory-slim", &[]),
            ("httpsoft/http-message", &[]),
        ],
    ),
];

/// `Plugin::INTERFACE_MAP`: the abstractions `extra.discovery` can pin.
const INTERFACE_MAP: &[(&str, &[&str])] = &[
    (
        "php-http/async-client-implementation",
        &["Http\\Client\\HttpAsyncClient"],
    ),
    (
        "php-http/client-implementation",
        &["Http\\Client\\HttpClient"],
    ),
    (
        "psr/http-client-implementation",
        &["Psr\\Http\\Client\\ClientInterface"],
    ),
    (
        "psr/http-factory-implementation",
        &[
            "Psr\\Http\\Message\\RequestFactoryInterface",
            "Psr\\Http\\Message\\ResponseFactoryInterface",
            "Psr\\Http\\Message\\ServerRequestFactoryInterface",
            "Psr\\Http\\Message\\StreamFactoryInterface",
            "Psr\\Http\\Message\\UploadedFileFactoryInterface",
            "Psr\\Http\\Message\\UriFactoryInterface",
        ],
    ),
];

fn rules_of(abstraction: &str) -> Option<Candidates> {
    PROVIDE_RULES
        .iter()
        .find(|(a, _)| *a == abstraction)
        .map(|(_, c)| *c)
}

/// One package of the local repository as `getPackages()` returns it: an
/// alias is a package of its own (same name, its own version).
#[derive(Debug, Clone)]
pub(crate) struct RepoPackage {
    pub(crate) name: String,
    /// Normalized version (`getVersion()`).
    pub(crate) version: String,
    pub(crate) replaces: Vec<String>,
    pub(crate) requires: Vec<String>,
}

impl RepoPackage {
    /// A package entry of a lock or installed.json, and its branch alias
    /// when it has one (loaded first, as `ArrayLoader` returns it).
    pub(crate) fn from_entry(p: &Value) -> Vec<RepoPackage> {
        let Some(obj) = p.as_object() else {
            return Vec::new();
        };
        let Some(name) = obj.get("name").and_then(Value::as_str) else {
            return Vec::new();
        };
        let keys = |field: &str| -> Vec<String> {
            obj.get(field)
                .and_then(Value::as_object)
                .map(|m| m.keys().map(|k| k.to_ascii_lowercase()).collect())
                .unwrap_or_default()
        };
        let pretty = obj
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version = obj
            .get("version_normalized")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| vivacity_resolver::version::normalize(pretty, None).ok())
            .unwrap_or_else(|| pretty.to_owned());
        let package = RepoPackage {
            name: name.to_ascii_lowercase(),
            version,
            replaces: keys("replace"),
            requires: keys("require"),
        };
        let mut out = Vec::new();
        if let Some(alias) = vivacity_resolver::loader::branch_alias(obj) {
            out.push(RepoPackage {
                version: alias,
                ..package.clone()
            });
        }
        out.push(package);
        out
    }
}

/// An ordered PHP array with string keys: `$a[$k] = $v` keeps the key's
/// place when it exists, appends otherwise.
#[derive(Default)]
struct Ordered<V>(Vec<(String, V)>);

impl<V> Ordered<V> {
    fn get_mut(&mut self, k: &str) -> Option<&mut V> {
        self.0.iter_mut().find(|(key, _)| key == k).map(|(_, v)| v)
    }
    fn contains(&self, k: &str) -> bool {
        self.0.iter().any(|(key, _)| key == k)
    }
    fn set(&mut self, k: &str, v: V) {
        match self.get_mut(k) {
            Some(slot) => *slot = v,
            None => self.0.push((k.to_owned(), v)),
        }
    }
}

impl Ordered<Vec<String>> {
    fn push(&mut self, k: &str, v: &str) {
        match self.get_mut(k) {
            Some(slot) => slot.push(v.to_owned()),
            None => self.0.push((k.to_owned(), vec![v.to_owned()])),
        }
    }
}

/// `$repo->findPackage($name, $versionParser->parseConstraints($c))`.
fn find_package(repo: &[RepoPackage], name: &str, constraint: &str) -> bool {
    let Ok(parsed) = vivacity_resolver::constraint::parse_constraints(constraint) else {
        return false;
    };
    repo.iter()
        .any(|p| p.name == name && parsed.constraint.matches_version(&p.version))
}

/// php-http/discovery after an update with an install: the one of the
/// solved lock, active when `allow-plugins` allows it (loaded at start, or
/// registered by the install), acting on the local repository the install
/// leaves. Decided before the lock is written.
pub(crate) fn resolved_issue(
    manifest: &Value,
    lock: &Value,
    with_dev: bool,
) -> Option<vivacity_core::scope::ScopeIssue> {
    if !matches!(
        vivacity_core::layout::plugin_allowed(manifest, DISCOVERY),
        vivacity_core::layout::PluginVerdict::Allowed
    ) {
        return None;
    }
    let parsed = vivacity_core::lock::Lock::from_value(lock);
    let version = parsed
        .wanted_packages(with_dev)
        .find(|p| p.name().eq_ignore_ascii_case(DISCOVERY))?
        .version()
        .to_owned();
    let entries: Vec<Value> = parsed
        .wanted_packages(with_dev)
        .map(|p| Value::Object(p.raw.clone()))
        .collect();
    let (repo, dev_names) = installed_from_lock(lock, with_dev);
    post_update_issue(
        Some(&version),
        &repo,
        &dev_names,
        manifest,
        || vivacity_core::scope::discovery_watches(manifest, &entries),
        "not emulated",
    )
}

/// The local repository an install leaves for `POST_UPDATE_CMD`: the
/// lock's packages the install wanted (`with_dev`), their branch aliases
/// and the lock's inline aliases; its dev names are the whole lock's
/// (`setDevPackageNames($locker->getDevPackageNames())`).
pub(crate) fn installed_from_lock(
    lock: &Value,
    with_dev: bool,
) -> (Vec<RepoPackage>, BTreeSet<String>) {
    let list = |key: &str| -> Vec<Value> {
        lock.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let dev = list("packages-dev");
    let mut entries = list("packages");
    if with_dev {
        entries.extend(dev.iter().cloned());
    }
    let mut repo: Vec<RepoPackage> = entries.iter().flat_map(RepoPackage::from_entry).collect();
    for alias in lock
        .get("aliases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(version)) = (
            alias.get("package").and_then(Value::as_str),
            alias.get("alias_normalized").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        if let Some(p) = repo.iter().find(|p| p.name == name).cloned() {
            repo.push(RepoPackage {
                version: version.to_owned(),
                ..p
            });
        }
    }
    let dev_names = dev
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect();
    (repo, dev_names)
}

/// `isset($pinned[$key])`: present and not null.
fn pinned(extra_discovery: Option<&Value>, key: &str) -> bool {
    extra_discovery
        .and_then(|d| d.get(key))
        .is_some_and(|v| !v.is_null())
}

/// `postUpdate`'s pinned abstractions (`extra.discovery`).
fn pinned_abstractions(root: &Value) -> BTreeSet<&'static str> {
    let extra = root.get("extra").and_then(|e| e.get("discovery"));
    INTERFACE_MAP
        .iter()
        .filter(|(abstraction, interfaces)| {
            pinned(extra, abstraction) || interfaces.iter().all(|i| pinned(extra, i))
        })
        .map(|(a, _)| *a)
        .collect()
}

/// `postUpdate` up to its `if (!$missingRequires = array_filter(...))
/// return;`: true when it goes on to edit composer.json. `repo` is the
/// local repository at `POST_UPDATE_CMD`, `dev_names` its
/// `getDevPackageNames()`, `root` the root package's manifest.
pub(crate) fn would_act(repo: &[RepoPackage], dev_names: &BTreeSet<String>, root: &Value) -> bool {
    let keys = |field: &str| -> Vec<String> {
        root.get(field)
            .and_then(Value::as_object)
            .map(|m| m.keys().map(|k| k.to_ascii_lowercase()).collect())
            .unwrap_or_default()
    };
    let is_project = root.get("type").and_then(Value::as_str) == Some("project");
    let pinned = pinned_abstractions(root);
    let mut requires: [BTreeSet<String>; 2] = [
        keys("require").into_iter().collect(),
        keys("require-dev").into_iter().collect(),
    ];
    if !requires[0].contains(DISCOVERY) {
        requires = [BTreeSet::new(), BTreeSet::new()];
    }
    let mut all_packages: BTreeSet<String> = BTreeSet::new();
    for p in repo {
        all_packages.insert(p.name.clone());
        all_packages.extend(p.replaces.iter().cloned());
        if p.requires.iter().any(|r| r == DISCOVERY) {
            let dev = usize::from(dev_names.contains(&p.name));
            requires[dev].extend(p.requires.iter().cloned());
        }
    }
    let present = |n: &str| all_packages.contains(n);
    let is_dev = |n: &str| dev_names.contains(n);
    let split = |c: &'static str| -> (&'static str, Option<&'static str>) {
        match c.split_once(':') {
            Some((n, v)) => (n, Some(v)),
            None => (c, None),
        }
    };
    let mut missing: [Ordered<Vec<String>>; 3] = Default::default();
    for (dev, wanted) in requires.iter().enumerate() {
        let is_dev_list = dev == 1;
        let mut abstractions: Vec<&'static str> = Vec::new();
        let mut rules: Ordered<Candidates> = Ordered(
            PROVIDE_RULES
                .iter()
                .filter(|(a, _)| wanted.contains(*a))
                .map(|(a, c)| ((*a).to_owned(), *c))
                .collect(),
        );
        while !rules.0.is_empty() {
            let (abstraction, candidates) = rules.0.remove(0);
            let Some((abstraction, _)) = PROVIDE_RULES.iter().find(|(a, _)| *a == abstraction)
            else {
                continue;
            };
            if pinned.contains(abstraction) {
                continue;
            }
            abstractions.push(abstraction);
            for (spec, deps) in candidates {
                let (candidate, version) = split(spec);
                if !present(candidate) {
                    continue;
                }
                if version.is_some_and(|v| !find_package(repo, candidate, v)) {
                    continue;
                }
                if is_project && !is_dev_list && is_dev(candidate) {
                    missing[0].set(abstraction, vec![candidate.to_owned()]);
                    missing[2].set(abstraction, vec![candidate.to_owned()]);
                } else {
                    missing[dev].set(abstraction, Vec::new());
                }
                for dep in *deps {
                    if let Some(more) = rules_of(dep) {
                        rules.set(dep, more);
                    } else if !present(dep) {
                        missing[dev].push(abstraction, dep);
                    } else if is_project && !is_dev_list && is_dev(dep) {
                        missing[0].push(abstraction, dep);
                        missing[2].push(abstraction, dep);
                    }
                }
                break;
            }
        }
        'abstraction: for abstraction in abstractions {
            if missing[dev].contains(abstraction) {
                continue;
            }
            let Some(candidates) = rules_of(abstraction) else {
                continue;
            };
            for (spec, _) in candidates {
                let (candidate, version) = split(spec);
                if version.is_some_and(|v| !find_package(repo, candidate, v)) {
                    continue;
                }
                if present(candidate) && (!is_project || is_dev_list || !is_dev(candidate)) {
                    continue 'abstraction;
                }
            }
            // `STICKYNESS_RULES` only choose which candidate: one is required.
            missing[dev].set(abstraction, vec![String::new()]);
        }
    }
    let [zero, one, two] = missing;
    let one: Vec<(String, Vec<String>)> = one
        .0
        .into_iter()
        .filter(|(k, _)| !zero.contains(k))
        .collect();
    [&zero.0, &one, &two.0]
        .iter()
        .any(|list| list.iter().any(|(_, names)| !names.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repo(entries: &[Value]) -> Vec<RepoPackage> {
        entries.iter().flat_map(RepoPackage::from_entry).collect()
    }
    fn pkg(name: &str, version: &str) -> Value {
        json!({"name": name, "version": version})
    }
    fn devs(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }
    const ROOT_REQ: &str = "psr/http-client-implementation";

    fn root(ty: &str, require: &[&str], require_dev: &[&str]) -> Value {
        let map = |l: &[&str]| {
            l.iter()
                .map(|n| ((*n).to_owned(), json!("*")))
                .collect::<serde_json::Map<_, _>>()
        };
        json!({"type": ty, "require": map(require), "require-dev": map(require_dev)})
    }

    #[test]
    fn nothing_to_do_without_the_root_opting_in() {
        let r = repo(&[pkg("php-http/discovery", "1.20.0")]);
        assert!(!would_act(
            &r,
            &devs(&[]),
            &root("library", &[ROOT_REQ], &[])
        ));
    }

    #[test]
    fn an_installed_implementation_satisfies_it() {
        let r = repo(&[
            pkg("php-http/discovery", "1.20.0"),
            pkg("guzzlehttp/guzzle", "7.10.0"),
        ]);
        let m = root("project", &[DISCOVERY, ROOT_REQ], &[]);
        assert!(!would_act(&r, &devs(&[]), &m));
        // None installed: one is required.
        let r = repo(&[pkg("php-http/discovery", "1.20.0")]);
        assert!(would_act(&r, &devs(&[]), &m));
    }

    #[test]
    fn a_project_moves_a_dev_implementation_to_require() {
        let r = repo(&[
            pkg("php-http/discovery", "1.20.0"),
            pkg("guzzlehttp/guzzle", "7.10.0"),
        ]);
        let project = root("project", &[DISCOVERY, ROOT_REQ], &[]);
        assert!(would_act(&r, &devs(&["guzzlehttp/guzzle"]), &project));
        // A library keeps it where it is.
        let library = root("library", &[DISCOVERY, ROOT_REQ], &[]);
        assert!(!would_act(&r, &devs(&["guzzlehttp/guzzle"]), &library));
    }

    #[test]
    fn a_package_requiring_discovery_brings_its_abstractions() {
        // php-http/mock-client: requires discovery and the client
        // implementation it provides itself, unknown to PROVIDE_RULES.
        let mock = json!({"name": "php-http/mock-client", "version": "1.6.1",
            "require": {"php-http/discovery": "^1.16", "php-http/client-common": "^2.0", "psr/http-factory-implementation": "^1.0"}});
        let r = repo(&[pkg("php-http/discovery", "1.20.0"), mock.clone()]);
        assert!(would_act(&r, &devs(&[]), &root("library", &[], &[])));
        let r = repo(&[
            pkg("php-http/discovery", "1.20.0"),
            mock,
            pkg("nyholm/psr7", "1.8.2"),
        ]);
        assert!(!would_act(&r, &devs(&[]), &root("library", &[], &[])));
    }

    #[test]
    fn a_replaced_name_counts_as_present_a_provided_one_does_not() {
        let replacer =
            json!({"name": "acme/http", "version": "1.0.0", "replace": {"nyholm/psr7": "*"}});
        let provider =
            json!({"name": "acme/http", "version": "1.0.0", "provide": {"nyholm/psr7": "*"}});
        let m = root(
            "library",
            &[DISCOVERY, "psr/http-factory-implementation"],
            &[],
        );
        assert!(!would_act(&repo(&[replacer]), &devs(&[]), &m));
        assert!(would_act(&repo(&[provider]), &devs(&[]), &m));
    }

    #[test]
    fn a_replaced_name_never_counts_as_dev() {
        // `$devPackages += $names` adds integer keys: a project whose dev
        // package replaces the implementation is left alone.
        let replacer =
            json!({"name": "acme/http", "version": "1.0.0", "replace": {"nyholm/psr7": "*"}});
        let m = root(
            "project",
            &[DISCOVERY, "psr/http-factory-implementation"],
            &[],
        );
        assert!(!would_act(&repo(&[replacer]), &devs(&["acme/http"]), &m));
    }

    #[test]
    fn a_versioned_candidate_needs_a_matching_package_alias_included() {
        let m = root(
            "library",
            &[DISCOVERY, "psr/http-factory-implementation"],
            &[],
        );
        // guzzlehttp/psr7:>=2 — 1.9 does not satisfy it.
        assert!(would_act(
            &repo(&[pkg("guzzlehttp/psr7", "1.9.1")]),
            &devs(&[]),
            &m
        ));
        assert!(!would_act(
            &repo(&[pkg("guzzlehttp/psr7", "2.7.0")]),
            &devs(&[]),
            &m
        ));
        let branch = json!({"name": "guzzlehttp/psr7", "version": "dev-main",
            "extra": {"branch-alias": {"dev-main": "2.7-dev"}}});
        assert!(!would_act(&repo(&[branch]), &devs(&[]), &m));
    }

    #[test]
    fn a_pinned_abstraction_is_left_alone() {
        let mut m = root("library", &[DISCOVERY, ROOT_REQ], &[]);
        assert!(would_act(&repo(&[]), &devs(&[]), &m));
        m["extra"] = json!({"discovery": {"Psr\\Http\\Client\\ClientInterface": "Acme\\Client"}});
        assert!(!would_act(&repo(&[]), &devs(&[]), &m));
        m["extra"] = json!({"discovery": {ROOT_REQ: null}});
        assert!(would_act(&repo(&[]), &devs(&[]), &m));
    }

    #[test]
    fn an_abstraction_in_both_lists() {
        // Decided once per list (`array_diff_key` then drops the dev
        // entry): satisfied in both.
        let r = repo(&[pkg("guzzlehttp/guzzle", "7.10.0")]);
        let m = root("library", &[DISCOVERY, ROOT_REQ], &[ROOT_REQ]);
        assert!(!would_act(&r, &devs(&[]), &m));
    }

    #[test]
    fn a_dependency_rule_is_followed() {
        // symfony/http-client for the PSR-18 client needs a PSR-17 factory
        // and psr/http-client.
        let r = repo(&[
            pkg("symfony/http-client", "7.3.0"),
            pkg("psr/http-client", "1.0.3"),
        ]);
        let m = root("library", &[DISCOVERY, ROOT_REQ], &[]);
        assert!(would_act(&r, &devs(&[]), &m));
        let r = repo(&[
            pkg("symfony/http-client", "7.3.0"),
            pkg("psr/http-client", "1.0.3"),
            pkg("nyholm/psr7", "1.8.2"),
        ]);
        assert!(!would_act(&r, &devs(&[]), &m));
    }
}
