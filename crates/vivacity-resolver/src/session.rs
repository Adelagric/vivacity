//! Setup of an `update`: what `Factory::createComposer` then
//! `Installer::doUpdate` do before `createPool`: root, platform,
//! repositories (global config + composer.json, same merge rules as
//! `Config::merge`), lock repository, `Request`. Rust-side replica of
//! tools/oracle-pool.php.

use crate::constraint::{Constraint, Op};
use crate::lockfile::{dump_package, lock_data, lock_packages, LockInput};
use crate::optimizer::PoolOptimizer;
use crate::package::{Origin, Package};
use crate::platform::{platform_packages, probe};
use crate::platform_filter::PlatformRequirementFilter;
use crate::policy::DefaultPolicy;
use crate::pool::{OrderedMap, Pool, PoolError, Repository, RepositorySet, Request};
use crate::repository::{
    locked_repository, ComposerRepository, FileTransport, HttpTransport, HttpTransports,
};
use crate::root::RootPackage;
use crate::solver::{SolveError, Solver};
use crate::transaction::LockTransaction;
use crate::version::{parse_stability, regex, stability_rank};
use pcre2::bytes::Regex;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SessionError {
    pub message: String,
    pub kind: SessionErrorKind,
    /// A line Composer writes on STDOUT next to the error (`$io->write`
    /// after `$io->writeError`), as `UpdateCommand` does for a temporary
    /// constraint that misses the root's.
    pub stdout: Option<String>,
    /// The message as Composer writes it, piece by piece with each piece's
    /// level (`IOInterface::QUIET` for the headline of an unsolvable set and
    /// the `--no-dev` warning, the default level for the problems). Empty
    /// when the whole message has one level, the caller's.
    pub parts: Vec<(vivacity_core::io::Verbosity, String)>,
    /// What Composer hands to `GithubActionError::emit` for this error,
    /// when it is not the message itself: the headline and the problems of
    /// an unsolvable set (`$err."\n".$prettyProblem`), without the
    /// `--no-dev` warning or the dev extraction's two hint lines.
    pub annotation: Option<String>,
    /// Lines Composer has written before it throws, which the error must
    /// not lose: the warnings a degraded repository held while the list
    /// filter of `install`'s verification ran (`ComposerRepository` writes
    /// them as it loads, before the exception of another fetch).
    pub preface: Box<[String]>,
}

/// What Composer does with the error: an unsolvable set
/// (`SolverProblemsException`) means exit code 2 from `Installer::run`,
/// everything else is an exception that propagates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionErrorKind {
    Other,
    Unsolvable,
    /// Composer would carry this as a `TransportException`, whose code it
    /// rewrites to 100 (`Application.php:502-506`, `Installer.php:92`).
    Transport,
    /// What vivacity does not reproduce (a `path` package versioned by hg,
    /// fossil or svn): the message is the reason, and the command goes to
    /// Composer.
    Unsupported,
    /// The command prints the message itself and returns a code, with no
    /// `Error:` prefix and nothing written: `UpdateCommand`'s temporary
    /// constraint refusal (1), `Installer::run`'s partial update without a
    /// lock (3).
    Printed(i32),
}

impl SessionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: SessionErrorKind::Other,
            stdout: None,
            parts: Vec::new(),
            annotation: None,
            preface: Box::default(),
        }
    }
}

/// What vivacity does not reproduce: the reason, for the caller to name.
fn unsupported(message: String) -> SessionError {
    SessionError {
        kind: SessionErrorKind::Unsupported,
        ..SessionError::new(message)
    }
}

/// `RuleSetGenerator`'s same-name rule (`getNames(false)`: the name and the
/// `replace` targets, RuleSetGenerator.php:176-179 and 243-248) is not
/// filtered by `--ignore-platform-reqs`: a package replacing a platform name
/// conflicts with the fixed platform package of that name when the PHP has
/// it. Without PHP that is unknowable: the first such package, named.
fn replaced_platform_name(
    packages: &[usize],
    arena: &[Package],
    overrides: &Map<String, Value>,
) -> Option<String> {
    packages.iter().find_map(|&idx| {
        let p = &arena[idx];
        if p.origin == Origin::Platform {
            return None;
        }
        p.replaces
            .iter()
            .find(|l| !crate::platform::known_without_php(&l.target, overrides))
            .map(|l| {
                format!(
                    "{} replaces {}, which conflicts with the platform package of that name when the PHP has it, and only a PHP can say",
                    p.pretty_name, l.target
                )
            })
    })
}

impl From<PoolError> for SessionError {
    fn from(e: PoolError) -> SessionError {
        let transport = e.is_transport();
        SessionError {
            message: e.0,
            kind: if transport {
                SessionErrorKind::Transport
            } else {
                SessionErrorKind::Other
            },
            stdout: None,
            parts: Vec::new(),
            annotation: None,
            preface: Box::default(),
        }
    }
}

/// The newtype errors of the other layers: a message, and nothing to
/// distinguish — none of them is a transport failure.
macro_rules! session_error_from {
    ($($ty:path),+ $(,)?) => {
        $(impl From<$ty> for SessionError {
            fn from(e: $ty) -> SessionError {
                SessionError::new(e.0)
            }
        })+
    };
}

session_error_from!(
    crate::loader::LoadError,
    crate::platform::PlatformError,
    crate::policy_config::PolicyError,
    crate::root::RootError,
);

/// A filter error, on the other hand, can be either.
impl From<crate::pool_filters::FilterError> for SessionError {
    fn from(e: crate::pool_filters::FilterError) -> SessionError {
        SessionError {
            kind: if e.1 == crate::repository::RepoErrorKind::Transport {
                SessionErrorKind::Transport
            } else {
                SessionErrorKind::Other
            },
            message: e.0,
            stdout: None,
            parts: Vec::new(),
            annotation: None,
            preface: Box::default(),
        }
    }
}

/// A repository error keeps its nature: Composer rewrites a
/// `TransportException`'s code to `Installer::ERROR_TRANSPORT_EXCEPTION`
/// (100) before Symfony turns it into an exit code, so a failure to FETCH
/// something does not exit like a failure to understand it.
impl From<crate::repository::RepoError> for SessionError {
    fn from(e: crate::repository::RepoError) -> SessionError {
        SessionError {
            kind: match e.1 {
                crate::repository::RepoErrorKind::Transport => SessionErrorKind::Transport,
                crate::repository::RepoErrorKind::Unsupported => SessionErrorKind::Unsupported,
                crate::repository::RepoErrorKind::Data => SessionErrorKind::Other,
            },
            message: e.0,
            stdout: None,
            parts: Vec::new(),
            annotation: None,
            preface: Box::default(),
        }
    }
}

/// `Config::$repositories` after merging: (name or index, definition).
#[derive(Debug, Clone, PartialEq)]
pub struct RepoConfig {
    pub key: RepoKey,
    pub definition: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoKey {
    Named(String),
    Indexed(u64),
}

/// `Config::merge` for the `repositories` key, applied in order (defaults,
/// global config, composer.json).
pub fn merge_repositories(current: &mut Vec<RepoConfig>, new: &Value) {
    static PACKAGIST: OnceLock<Regex> = OnceLock::new();
    let entries: Vec<(RepoKey, Value)> = match new {
        Value::Array(list) => list
            .iter()
            .enumerate()
            .map(|(i, v)| (RepoKey::Indexed(i as u64), v.clone()))
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let key = match k.parse::<u64>() {
                    Ok(i) if i.to_string() == *k => RepoKey::Indexed(i),
                    _ => RepoKey::Named(k.clone()),
                };
                (key, v.clone())
            })
            .collect(),
        _ => return,
    };
    if entries.is_empty() {
        return;
    }
    current.reverse();
    // `disableRepoByName((string) $name)`: a numeric string falls back to
    // the integer key of the PHP array.
    let disable = |current: &mut Vec<RepoConfig>, name: &str| {
        let key = match name.parse::<u64>() {
            Ok(i) if i.to_string() == name => RepoKey::Indexed(i),
            _ => RepoKey::Named(name.to_owned()),
        };
        if current.iter().any(|r| r.key == key) {
            current.retain(|r| r.key != key);
        } else if name == "packagist" {
            current.retain(|r| r.key != RepoKey::Named("packagist.org".into()));
        }
    };
    for (key, repository) in entries.into_iter().rev() {
        if repository == Value::Bool(false) {
            let name = match &key {
                RepoKey::Named(n) => n.clone(),
                RepoKey::Indexed(i) => i.to_string(),
            };
            disable(current, &name);
            continue;
        }
        if let Some(obj) = repository.as_object() {
            if obj.len() == 1 && obj.values().next() == Some(&Value::Bool(false)) {
                disable(current, obj.keys().next().map(String::as_str).unwrap_or(""));
                continue;
            }
        }
        if repository.get("type").and_then(Value::as_str) == Some("composer") {
            if let Some(url) = repository.get("url").and_then(Value::as_str) {
                let re = regex(
                    &PACKAGIST,
                    r"^https?://(?:[a-z0-9-.]+\.)?packagist.org(/|$)",
                    false,
                );
                if re.is_match(url.as_bytes()).unwrap_or(false) {
                    disable(current, "packagist.org");
                }
            }
        }
        match key {
            RepoKey::Indexed(i) => {
                if current.iter().any(|r| r.key == RepoKey::Indexed(i)) {
                    let next = current
                        .iter()
                        .filter_map(|r| match r.key {
                            RepoKey::Indexed(j) => Some(j + 1),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(0);
                    current.push(RepoConfig {
                        key: RepoKey::Indexed(next),
                        definition: repository,
                    });
                } else {
                    current.push(RepoConfig {
                        key: RepoKey::Indexed(i),
                        definition: repository,
                    });
                }
            }
            RepoKey::Named(name) => {
                let name = if name == "packagist" {
                    "packagist.org".to_owned()
                } else {
                    name
                };
                let key = RepoKey::Named(name);
                if let Some(slot) = current.iter_mut().find(|r| r.key == key) {
                    slot.definition = repository;
                } else {
                    current.push(RepoConfig {
                        key,
                        definition: repository,
                    });
                }
            }
        }
    }
    current.reverse();
}

/// Merged configuration relevant to the resolver.
#[derive(Debug, Clone, Default)]
pub struct MergedConfig {
    pub repositories: Vec<RepoConfig>,
    /// `config.platform` (the last definition replaces the previous one).
    pub platform: Map<String, Value>,
    /// `config.policy` and `config.audit` merged (`Config::merge`).
    pub policy: crate::policy_config::RawPolicyConfig,
}

impl MergedConfig {
    /// Defaults + `COMPOSER_HOME/config.json` + composer.json, like
    /// `Factory::createConfig` then `Config::merge($localConfig)`.
    pub fn load(
        manifest: &Value,
        composer_home: Option<&Path>,
    ) -> Result<MergedConfig, SessionError> {
        let mut cfg = MergedConfig {
            repositories: vec![RepoConfig {
                key: RepoKey::Named("packagist.org".into()),
                definition: serde_json::json!({"type": "composer", "url": "https://repo.packagist.org"}),
            }],
            platform: Map::new(),
            policy: crate::policy_config::RawPolicyConfig::default(),
        };
        if let Some(home) = composer_home {
            let global = home.join("config.json");
            if global.is_file() {
                let text = std::fs::read_to_string(&global)
                    .map_err(|e| SessionError::new(format!("{}: {e}", global.display())))?;
                let v: Value = serde_json::from_str(&text)
                    .map_err(|e| SessionError::new(format!("{}: {e}", global.display())))?;
                cfg.merge(&v);
            }
        }
        cfg.merge(manifest);
        Ok(cfg)
    }

    fn merge(&mut self, config: &Value) {
        // `$this->config['platform'] = $val`: any value replaces the
        // previous one (`[]` or `null` clear it).
        if let Some(platform) = config.get("config").and_then(|c| c.get("platform")) {
            self.platform = platform.as_object().cloned().unwrap_or_default();
        }
        if let Some(repos) = config.get("repositories") {
            merge_repositories(&mut self.repositories, repos);
        }
        if let Some(cfg) = config.get("config").and_then(Value::as_object) {
            self.policy.merge(cfg);
        }
    }
}

/// Result of a `solve`: the transaction and what is needed to compare it
/// with Composer.
pub struct SolveReport {
    pub transaction: LockTransaction,
    /// Decided literals, in order.
    pub decisions: Vec<i64>,
    /// `getRuleSetSize()`.
    pub rules: usize,
    /// Learned rules (conflicts encountered).
    pub learned: usize,
    /// What `Installer::doUpdate` writes after a successful resolution and
    /// before the lock operations, in order, each line with its level: the
    /// resolution's duration, the `Analyzed …` counts, `Nothing to modify in
    /// lock file`, the dev extraction's duration. Held rather than written so
    /// that the command prints them once it knows the run stays here.
    pub events: Vec<(vivacity_core::io::Verbosity, String)>,
}

/// Options of an `update` (`Installer::setUpdateAllowList`,
/// `setUpdateAllowTransitiveDependencies`).
#[derive(Debug, Clone, Default)]
pub struct UpdateOptions {
    /// `composer update a/b c/*`: patterns, lowercased and deduplicated.
    pub allow_list: Vec<String>,
    pub transitive: Option<crate::pool::UpdateMode>,
    /// `--no-blocking` / `--no-security-blocking`.
    pub no_blocking: bool,
    /// Dry run of `require`/`remove`: the root package is patched in
    /// memory, composer.json is not touched.
    pub root_patch: Option<crate::root::RootPatch>,
    /// composer-merge-plugin active: the root as its INIT /
    /// `PRE_UPDATE_CMD` merges left it (the merged manifest, the
    /// included files' own requirements for the stability flags).
    pub merged: Option<crate::merge_plugin::Merged>,
    /// `update --with a/b:^1` (and the `update a/b:^1` shorthand), as
    /// written: name → constraint text, in command order. Parsed and
    /// checked against the root requirements by `prepare_update`.
    pub temporary_requirements: Vec<(String, String)>,
    /// `config.lock: false`: `Factory` gives the `Locker` `/dev/null`, so
    /// `isLocked()` is false and the update runs without a locked
    /// repository, whatever composer.lock holds (Factory.php:423-429).
    pub ignore_lock: bool,
    /// No PHP can be started and the command ignores every platform
    /// requirement (docs/plans/update-without-php.md): no probe, the
    /// platform is the `composer*` packages and `config.platform`'s
    /// overrides, and what would depend on the real platform is refused
    /// as [`SessionErrorKind::Unsupported`], named.
    pub php_absent: bool,
}

impl UpdateOptions {
    /// `Installer::setUpdateAllowList`: `strtolower` + `array_unique`.
    pub fn partial(packages: &[String], transitive: crate::pool::UpdateMode) -> UpdateOptions {
        let mut allow_list: Vec<String> = Vec::new();
        for p in packages {
            let l = p.to_lowercase();
            if !allow_list.contains(&l) {
                allow_list.push(l);
            }
        }
        UpdateOptions {
            allow_list,
            transitive: Some(transitive),
            ..Default::default()
        }
    }
}

/// `UpdateCommand::execute`'s temporary-constraint block: the root's
/// references and stability flags take the requirements in, a name with
/// `*` expands over the root requirements, and every constraint must
/// intersect the one in composer.json — otherwise the command fails with
/// its own message (exit 1), before anything is written.
fn temporary_constraints(
    reqs: &[(String, String)],
    root: &mut RootPackage,
) -> Result<BTreeMap<String, crate::pool::TemporaryConstraint>, SessionError> {
    if reqs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let pairs: Vec<(String, String)> = reqs.to_vec();
    crate::root::extract_references(&pairs, &mut root.references);
    let minimum = root.minimum_stability.clone();
    crate::root::extract_stability_flags(&pairs, &minimum, &mut root.stability_flags);
    // `$rootRequirements = array_merge(getRequires(), getDevRequires())`:
    // composer.json's order, which is the order a wildcard reports its
    // first mismatch in.
    let root_requires: Vec<(String, String, Constraint)> = root
        .all_requires()
        .iter()
        .map(|l| {
            (
                l.target.clone(),
                l.pretty_constraint.clone(),
                l.constraint.clone(),
            )
        })
        .collect();
    let fail = |message: String, stdout: Option<String>| SessionError {
        message,
        kind: SessionErrorKind::Printed(1),
        stdout,
        parts: Vec::new(),
        annotation: None,
        preface: Box::default(),
    };
    let mut out: BTreeMap<String, crate::pool::TemporaryConstraint> = BTreeMap::new();
    for (name, text) in &pairs {
        let name = name.to_lowercase();
        let parsed = crate::pool::TemporaryConstraint {
            pretty: text.clone(),
            constraint: crate::constraint::parse_constraints(text)
                .map_err(|e| SessionError::new(format!("{name}: {e}")))?
                .constraint,
        };
        if name.contains('*') {
            let re = crate::pool::package_name_regexp(&name);
            for (target, pretty, constraint) in &root_requires {
                if !re.is_match(target.as_bytes()).unwrap_or(false) {
                    continue;
                }
                out.insert(target.clone(), parsed.clone());
                if !crate::intervals::have_intersections(&parsed.constraint, constraint) {
                    return Err(fail(
                        format!(
                            "The temporary constraint \"{text}\" for \"{name}\" matching \"{target}\" must be a subset of the constraint in your composer.json ({pretty})"
                        ),
                        None,
                    ));
                }
            }
        } else {
            out.insert(name.clone(), parsed.clone());
            if let Some((_, pretty, constraint)) = root_requires.iter().find(|(t, _, _)| *t == name)
            {
                if !crate::intervals::have_intersections(&parsed.constraint, constraint) {
                    return Err(fail(
                        format!(
                            "The temporary constraint \"{text}\" for \"{name}\" must be a subset of the constraint in your composer.json ({pretty})"
                        ),
                        Some(format!(
                            "Run `composer require {name}` or `composer require {name}:{text}` instead to replace the constraint"
                        )),
                    ));
                }
            }
        }
    }
    Ok(out)
}

/// Everything `Installer::doUpdate` has at hand right before `createPool`.
/// symfony/flex active for this resolution: its requirement and index.
#[derive(Debug, Clone)]
pub struct FlexSession {
    pub symfony_require: String,
    pub symfony: Constraint,
    pub versions: crate::flex_filter::FlexVersions,
}

pub struct UpdateSession {
    /// `None`: no Flex filter at `PRE_POOL_CREATE`.
    pub flex: Option<FlexSession>,
    pub arena: Vec<Package>,
    pub root: RootPackage,
    /// Arena indices of the fixed root (requires emptied) and of its alias.
    pub fixed_root: usize,
    pub fixed_root_alias: Option<usize>,
    pub platform: Vec<usize>,
    pub locked: Option<Vec<usize>>,
    pub set: RepositorySet,
    pub request: Request,
    pub config: MergedConfig,
    pub dev_mode: bool,
    /// `--prefer-stable` / `--prefer-lowest` from the command line.
    pub prefer_stable: bool,
    pub prefer_lowest: bool,
    /// `PHP_MAJOR.MINOR.RELEASE` of the probed PHP (`ext-*` rule of the
    /// `VersionSelector`).
    pub php_version: String,
    /// The pool blocking policies (`createPolicyConfig`).
    pub policy_config: crate::policy_config::PolicyConfig,
    /// `XdebugHandler::getAllIniFiles()` of the probed PHP (extension hint
    /// of an unsolvable set).
    pub ini_files: Vec<String>,
    /// `ext-*` loaded by the probed PHP, before `config.platform`.
    pub loaded_extensions: BTreeSet<String>,
    /// `Installer::$devMode` (`--no-dev` / `--update-no-dev` clear it):
    /// only the `--no-dev` warning of an unsolvable set reads it.
    pub installer_dev_mode: bool,
    /// [`UpdateOptions::php_absent`]: the platform was not probed.
    pub php_absent: bool,
}

impl UpdateSession {
    pub fn prepare(
        project_dir: &Path,
        composer_home: Option<&Path>,
        dev_mode: bool,
    ) -> Result<UpdateSession, SessionError> {
        Self::prepare_with(project_dir, composer_home, dev_mode, None)
    }

    /// Like `prepare`, with a network transport for `https://` repositories.
    pub fn prepare_with(
        project_dir: &Path,
        composer_home: Option<&Path>,
        dev_mode: bool,
        http: Option<HttpTransports>,
    ) -> Result<UpdateSession, SessionError> {
        Self::prepare_full(project_dir, composer_home, dev_mode, http, None)
    }

    /// Like `prepare_with`, with Composer's `cache-repo-dir` directory for
    /// the metadata cache (read and written in Composer's format).
    pub fn prepare_full(
        project_dir: &Path,
        composer_home: Option<&Path>,
        dev_mode: bool,
        http: Option<HttpTransports>,
        cache_repo_dir: Option<&Path>,
    ) -> Result<UpdateSession, SessionError> {
        Self::prepare_update(
            project_dir,
            composer_home,
            dev_mode,
            http,
            cache_repo_dir,
            &UpdateOptions::default(),
        )
    }

    /// Like `prepare_full`, with the allow list of a partial update
    /// (`composer update a/b`).
    pub fn prepare_update(
        project_dir: &Path,
        composer_home: Option<&Path>,
        dev_mode: bool,
        http: Option<HttpTransports>,
        cache_repo_dir: Option<&Path>,
        options: &UpdateOptions,
    ) -> Result<UpdateSession, SessionError> {
        let partial_update = !options.allow_list.is_empty();
        let manifest_path = project_dir.join("composer.json");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| SessionError::new(format!("{}: {e}", manifest_path.display())))?;
        let manifest: Value = serde_json::from_str(&manifest_text)
            .map_err(|e| SessionError::new(format!("{}: {e}", manifest_path.display())))?;
        let config = MergedConfig::load(&manifest, composer_home)?;
        let mut policy_config = crate::policy_config::PolicyConfig::from_raw(&config.policy)
            .map_err(SessionError::from)?;
        policy_config
            .apply_no_blocking(options.no_blocking)
            .map_err(SessionError::from)?;
        let mut root = RootPackage::load(&manifest, project_dir).map_err(SessionError::from)?;
        if let Some(merged) = &options.merged {
            // `RootPackageLoader` ran on the file: its stability flags are
            // the starting point. The plugin then merges each file's
            // links, aliases and references into the root; the flags of a
            // merged file come from its own `require` (and `require-dev`
            // in dev mode), `mergeStabilityFlags` — not from the merged
            // constraint text, which the loader never sees.
            let flags = std::mem::take(&mut root.stability_flags);
            root = RootPackage::load(&merged.manifest, project_dir).map_err(SessionError::from)?;
            root.stability_flags = flags;
            // (`require_dev` is recorded only when the merge ran in dev
            // mode — `mergeDevInto`, a second pass over the files.)
            for include in &merged.include_links {
                crate::root::merge_plugin_stability_flags(
                    &mut root.stability_flags,
                    &root.minimum_stability,
                    &include.require,
                );
            }
            for include in &merged.include_links {
                crate::root::merge_plugin_stability_flags(
                    &mut root.stability_flags,
                    &root.minimum_stability,
                    &include.require_dev,
                );
            }
        }
        if let Some(patch) = &options.root_patch {
            root.apply_patch(patch).map_err(SessionError::from)?;
        }
        // `UpdateCommand`: the temporary requirements feed the root's
        // references and stability flags before anything else reads them,
        // then become the pool's temporary constraints.
        let temporary_constraints =
            temporary_constraints(&options.temporary_requirements, &mut root)?;
        // `PoolBuilder` applies a temporary constraint to the fixed platform
        // packages too (PoolBuilder.php:292-318): one that drops a probed
        // package is Composer's `LogicException`, which no PHP means no
        // way to reproduce.
        if options.php_absent {
            if let Some(name) = temporary_constraints
                .keys()
                .find(|n| !crate::platform::known_without_php(n, &config.platform))
            {
                return Err(unsupported(format!(
                    "the temporary constraint on {name} applies to the platform package of that name, which only a PHP can report"
                )));
            }
        }
        let probed = if options.php_absent {
            None
        } else {
            Some(probe().map_err(SessionError::from)?)
        };
        // `PHP_MAJOR_VERSION.PHP_MINOR_VERSION.PHP_RELEASE_VERSION` of the
        // actual PHP (no `config.platform.php` here): the first three
        // numbers of PHP_VERSION.
        let php_version = probed
            .iter()
            .flatten()
            .find(|p| p.get("name").and_then(Value::as_str) == Some("php"))
            .and_then(|p| p.get("version").and_then(Value::as_str))
            .map(|v| {
                v.split(['.', '-', '+'])
                    .take(3)
                    .map(|part| part.trim_end_matches(|c: char| !c.is_ascii_digit()))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .unwrap_or_default();
        let platform_pkgs =
            platform_packages(probed.as_deref().unwrap_or_default(), &config.platform)
                .map_err(SessionError::from)?;

        let mut arena: Vec<Package> = Vec::new();

        // `$fixedRootPackage = clone $package; setRequires([]); setDevRequires([])`.
        let mut fixed = root.package.clone();
        fixed.requires = Default::default();
        fixed.dev_requires = Default::default();
        arena.push(fixed);
        let fixed_root = 0;
        let fixed_root_alias = root.branch_alias.as_ref().map(|(normalized, pretty)| {
            let a = arena[fixed_root].alias(fixed_root, normalized, pretty);
            arena.push(a);
            arena.len() - 1
        });
        let root_members: Vec<usize> = fixed_root_alias
            .into_iter()
            .chain(std::iter::once(fixed_root))
            .collect();

        let mut platform: Vec<usize> = Vec::new();
        for p in platform_pkgs {
            arena.push(p);
            platform.push(arena.len() - 1);
        }

        // `Locker::isLocked()` = file present and `isset($data['packages'])`;
        // an unreadable lock is ignored for a full update (`doUpdate`
        // swallows the ParsingException), fatal for a partial one.
        let lock_path = project_dir.join("composer.lock");
        let lock: Option<Value> = if lock_path.is_file() && !options.ignore_lock {
            let text = std::fs::read_to_string(&lock_path)
                .map_err(|e| SessionError::new(format!("{}: {e}", lock_path.display())))?;
            match serde_json::from_str::<Value>(&text) {
                Ok(v) if v.get("packages").is_some_and(|p| !p.is_null()) => Some(v),
                Ok(_) => None,
                Err(e) => {
                    if partial_update {
                        return Err(SessionError::new(format!(
                            "\"{}\" does not contain valid JSON\n{e}",
                            lock_path.display()
                        )));
                    }
                    None
                }
            }
        } else {
            None
        };
        let locked = match &lock {
            Some(v) => Some(locked_repository(v, &mut arena).map_err(SessionError::from)?),
            None => None,
        };

        // `$stabilityFlags[$package->getName()] = STABILITIES[parseStability($package->getVersion())]`:
        // the version seen is that of the root alias if there is one.
        let mut stability_flags = root.stability_flags.clone();
        let root_version = match fixed_root_alias {
            Some(a) => arena[a].version.clone(),
            None => arena[fixed_root].version.clone(),
        };
        stability_flags.insert(
            root.package.name.clone(),
            stability_rank(parse_stability(&root_version)),
        );

        // `createRepositorySet(forUpdate)` and `requirePackagesForUpdate(..., true)`
        // always take require + require-dev, `--no-dev` or not; and with a
        // root alias, `$this->package` is the RootAliasPackage, whose
        // `self.version` links target the alias version.
        let mut root_requires: OrderedMap<Constraint> = OrderedMap::default();
        let requires = match &root.branch_alias {
            Some((normalized, pretty)) => {
                let aliased = root.package.alias(0, normalized, pretty);
                let mut all = aliased.requires.clone();
                for l in aliased.dev_requires.iter() {
                    all.insert(l.clone());
                }
                all
            }
            None => root.all_requires(),
        };
        for link in requires.iter() {
            root_requires.insert(&link.target, link.constraint.clone());
        }

        let mut set = RepositorySet::new(
            &root.minimum_stability,
            stability_flags,
            &root.aliases,
            root.references.clone(),
            root_requires,
            temporary_constraints,
        );
        set.add_repository(Repository::Root(root_members));
        set.add_repository(Repository::Platform(platform.clone()));
        for repo in &config.repositories {
            let origin = Origin::Repository(set.repositories.len());
            set.add_repository(open_repository(
                repo,
                http.as_ref(),
                cache_repo_dir,
                project_dir,
                origin,
                &mut arena,
            )?);
        }
        if let Some(ids) = &locked {
            set.add_repository(Repository::Locked(ids.clone()));
        }

        // `Installer::run`: a partial update requires a lock — the message
        // alone on stderr, exit 3.
        if partial_update && locked.is_none() {
            return Err(SessionError {
                message: "Cannot update only a partial set of packages without a lock file present. Run `composer update` to generate a lock file.".to_owned(),
                kind: SessionErrorKind::Printed(3),
                stdout: None,
                // `Installer.php:508`: at QUIET.
                parts: vec![(
                    vivacity_core::io::Verbosity::Quiet,
                    "Cannot update only a partial set of packages without a lock file present. Run `composer update` to generate a lock file.".to_owned(),
                )],
                annotation: None,
                preface: Box::default(),
            });
        }
        let mut request = Request::new(locked.clone());
        if partial_update {
            request.set_update_allow_list(
                options.allow_list.clone(),
                options
                    .transitive
                    .unwrap_or(crate::pool::UpdateMode::OnlyListed),
            );
        }
        if let Some(a) = fixed_root_alias {
            request.fix_package(a);
        }
        request.fix_package(fixed_root);
        for &p in &platform {
            let provided = arena[fixed_root]
                .provides
                .get(&arena[p].name)
                .map(|l| l.constraint.clone());
            let provided_here = provided
                .is_some_and(|c| c.matches(&Constraint::new(Op::Eq, arena[p].version.clone())));
            if !provided_here {
                request.fix_package(p);
            }
        }
        for link in requires.iter() {
            request.require_name_pretty(
                &link.target,
                Some(link.constraint.clone()),
                &link.pretty_constraint,
            )?;
        }

        Ok(UpdateSession {
            flex: None,
            arena,
            root,
            fixed_root,
            fixed_root_alias,
            platform,
            locked,
            set,
            request,
            config,
            dev_mode,
            prefer_stable: false,
            prefer_lowest: false,
            php_version,
            policy_config,
            ini_files: probed
                .as_deref()
                .map(crate::platform::ini_files)
                .unwrap_or_default(),
            loaded_extensions: probed
                .as_deref()
                .map(crate::platform::loaded_extensions)
                .unwrap_or_default(),
            installer_dev_mode: true,
            php_absent: options.php_absent,
        })
    }

    pub fn create_pool(&mut self) -> Result<Pool, SessionError> {
        let pre_pool = self.flex.as_ref().map(|f| {
            // `$rootPackage->getRequires() + $rootPackage->getDevRequires()`:
            // a name in both keeps the `require` constraint.
            let mut root_constraints = BTreeMap::new();
            for link in self
                .root
                .package
                .requires
                .0
                .iter()
                .chain(self.root.package.dev_requires.0.iter())
            {
                root_constraints
                    .entry(link.target.clone())
                    .or_insert_with(|| link.constraint.clone());
            }
            crate::pool::PrePoolFilter {
                symfony_require: f.symfony_require.clone(),
                symfony: f.symfony.clone(),
                root_constraints,
                versions: f.versions.clone(),
            }
        });
        Ok(self.set.create_pool(
            &mut self.request,
            &mut self.arena,
            pre_pool.as_ref(),
            crate::pool::INSTALLER_IGNORED_TYPES,
        )?)
    }

    /// `Installer::createPolicy(true, ...)` without `--minimal-changes`.
    /// With Flex active, `COMPOSER_PREFER_DEV_OVER_PRERELEASE` is what Flex
    /// sets before Composer builds the policy.
    pub fn policy(&self) -> DefaultPolicy {
        let mut policy = DefaultPolicy::new(
            self.prefer_stable || self.root.prefer_stable,
            self.prefer_lowest,
            None,
        );
        if self.flex.is_some() {
            policy.prefer_dev_over_prerelease = true;
        }
        policy
    }

    /// `Installer::extractDevPackages`: second solve without the
    /// require-dev, on only the packages kept by the first, to classify
    /// `packages` / `packages-dev`.
    pub fn extract_dev_packages(
        &mut self,
        transaction: &mut LockTransaction,
        policy: &mut DefaultPolicy,
        filter: &PlatformRequirementFilter,
        events: &mut Vec<(vivacity_core::io::Verbosity, String)>,
    ) -> Result<(), SessionError> {
        if self.root.package.dev_requires.is_empty() {
            return Ok(());
        }
        // `$resultRepo`: each package reloaded from its dump (`load`, one at
        // a time), branch aliases recreated -> [alias, base].
        let dumps: Vec<Value> = transaction
            .new_lock_packages(&self.arena, false)
            .iter()
            .map(|&idx| Value::Object(dump_package(&self.arena[idx])))
            .collect();
        let result_ids =
            crate::loader::load_packages(&dumps, Origin::Result, &mut self.arena, false)
                .map_err(SessionError::from)?;
        // createPoolWithAllPackages: root, platform, result, with the root
        // aliases applied along the way.
        let mut members: Vec<usize> = Vec::new();
        members.extend(self.fixed_root_alias);
        members.push(self.fixed_root);
        members.extend(self.platform.iter().copied());
        members.extend(result_ids);
        let mut pool_packages: Vec<usize> = Vec::new();
        for idx in members {
            pool_packages.push(idx);
            let (name, version) = (
                self.arena[idx].name.clone(),
                self.arena[idx].version.clone(),
            );
            if let Some((alias, alias_normalized)) = self
                .set
                .root_aliases
                .get(&name)
                .and_then(|m| m.get(&version))
            {
                let mut base = idx;
                while let Some(b) = self.arena[base].alias_of {
                    base = b;
                }
                let mut a = self.arena[base].alias(base, alias_normalized, alias);
                a.root_package_alias = true;
                a.origin = Origin::Detached;
                self.arena.push(a);
                pool_packages.push(self.arena.len() - 1);
            }
        }
        let pool = Pool::new(pool_packages, Vec::new(), &self.arena);
        // createRequest (without lock) + requirePackagesForUpdate(..., false).
        let mut request = Request::new(None);
        if let Some(a) = self.fixed_root_alias {
            request.fix_package(a);
        }
        request.fix_package(self.fixed_root);
        for &p in &self.platform {
            let provided = self.arena[self.fixed_root]
                .provides
                .get(&self.arena[p].name)
                .map(|l| l.constraint.clone());
            let provided_here = provided.is_some_and(|c| {
                c.matches(&Constraint::new(Op::Eq, self.arena[p].version.clone()))
            });
            if !provided_here {
                request.fix_package(p);
            }
        }
        let requires = match &self.root.branch_alias {
            Some((normalized, pretty)) => self.root.package.alias(0, normalized, pretty).requires,
            None => self.root.package.requires.clone(),
        };
        for link in requires.iter() {
            request.require_name_pretty(
                &link.target,
                Some(link.constraint.clone()),
                &link.pretty_constraint,
            )?;
        }
        let mut solver = Solver::new(&pool, &self.arena);
        let before = std::time::Instant::now();
        let solved = solver.solve(&request, policy, filter);
        // `Solver::solve`: the duration at VERBOSE, before any problem —
        // held with the caller's lines on success.
        events.push(resolution_completed(before));
        let non_dev = match solved {
            Ok(t) => t,
            Err(SolveError::Problems(problems)) => {
                drop(solver);
                if let Some(reason) = self.extension_problem_without_php(&problems) {
                    return Err(unsupported(reason));
                }
                // `Installer::extractDevPackages`: `$isDevExtraction`, on a
                // request without lock built for this solve.
                let pretty = {
                    let mut ctx = crate::problem::MessageContext {
                        arena: &mut self.arena,
                        set: &self.set,
                        pool: &pool,
                        request: &request,
                        is_verbose: vivacity_core::io::shows(vivacity_core::io::Verbosity::Verbose),
                        ini_files: &self.ini_files,
                        loaded_extensions: &self.loaded_extensions,
                    };
                    crate::problem::pretty_string(&mut ctx, &problems, true)
                };
                // `Installer::extractDevPackages`: the headline at QUIET, the
                // rest at the default level.
                let head = "Unable to find a compatible set of packages based on your non-dev requirements alone.";
                let rest = format!("Your requirements can be resolved successfully when require-dev packages are present.\nYou may need to move packages from require-dev or some of their dependencies to require.\n{pretty}");
                return Err(SessionError {
                    message: format!("{head}\n{rest}"),
                    kind: SessionErrorKind::Unsolvable,
                    stdout: None,
                    parts: vec![
                        (vivacity_core::io::Verbosity::Quiet, head.to_owned()),
                        (vivacity_core::io::Verbosity::Normal, rest),
                    ],
                    // `$ghe->emit($err."\n".$prettyProblem)` (Installer.php:735).
                    annotation: Some(format!("{head}\n{pretty}")),
                    preface: Box::default(),
                });
            }
            Err(SolveError::Bug(b)) => return Err(SessionError::new(b)),
        };
        transaction.set_non_dev_packages(&self.arena, &non_dev);
        Ok(())
    }

    /// Without PHP, an unsolvable set whose explanation would end with the
    /// extension hint (`getExtensionProblems`: a rule requiring a name that
    /// starts with `ext-`), which lists the probed PHP's ini files.
    fn extension_problem_without_php(
        &self,
        problems: &[crate::solver::SolvedProblem],
    ) -> Option<String> {
        if !self.php_absent {
            return None;
        }
        crate::problem::extension_problem(&self.arena, problems).map(|name| {
            format!("the explanation of this unsolvable set (a requirement on {name}) lists the PHP's ini files, which only a PHP can report")
        })
    }

    /// `extractPlatformRequirements($links)`.
    fn platform_requirements(links: &crate::package::Links) -> Map<String, Value> {
        let mut out = Map::new();
        for l in links.iter() {
            if crate::platform::is_platform_package(&l.target) {
                out.insert(l.target.clone(), Value::String(l.pretty_constraint.clone()));
            }
        }
        out
    }

    /// `Locker::setLockData(...)`: the lock JSON to write.
    pub fn lock_json(
        &self,
        transaction: &LockTransaction,
        manifest_text: &str,
    ) -> Result<Value, SessionError> {
        let content_hash = vivacity_core::content_hash::content_hash(manifest_text)
            .map_err(|e| SessionError::new(e.to_string()))?;
        let packages = lock_packages(
            &self.arena,
            &transaction.new_lock_packages(&self.arena, false),
        )
        .map_err(SessionError::new)?;
        let packages_dev = lock_packages(
            &self.arena,
            &transaction.new_lock_packages(&self.arena, true),
        )
        .map_err(SessionError::new)?;
        let (requires, dev_requires) = match &self.root.branch_alias {
            Some((normalized, pretty)) => {
                let a = self.root.package.alias(0, normalized, pretty);
                (a.requires, a.dev_requires)
            }
            None => (
                self.root.package.requires.clone(),
                self.root.package.dev_requires.clone(),
            ),
        };
        Ok(lock_data(LockInput {
            content_hash: &content_hash,
            packages,
            packages_dev: Some(packages_dev),
            platform: Self::platform_requirements(&requires),
            platform_dev: Self::platform_requirements(&dev_requires),
            aliases: &transaction.aliases(&self.arena, &self.root.aliases),
            minimum_stability: &self.root.minimum_stability,
            stability_flags: &self.root.stability_flags,
            prefer_stable: self.prefer_stable || self.root.prefer_stable,
            prefer_lowest: self.prefer_lowest,
            platform_overrides: &self.config.platform,
        }))
    }

    /// Full `composer update --no-install`: solve, dev package extraction,
    /// lock data. Returns the lock JSON and the report of the first solve.
    pub fn update(
        &mut self,
        manifest_text: &str,
        filter: &PlatformRequirementFilter,
    ) -> Result<(Value, SolveReport), SessionError> {
        let trace = std::env::var_os("VIVACITY_TRACE").is_some();
        let t = std::time::Instant::now();
        let lap = |label: &str, t: &std::time::Instant| {
            if trace {
                eprintln!(
                    "trace: {label:<22} {:>7.1} ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
        };
        let mut policy = self.policy();
        let pool = self.create_filtered_pool()?;
        if self.php_absent {
            if let Some(reason) =
                replaced_platform_name(&pool.packages, &self.arena, &self.config.platform)
            {
                return Err(unsupported(reason));
            }
        }
        // `Installer::doUpdate`: the pool is built (Flex's notice comes out
        // of it), then `Updating dependencies`, then the policy filters'
        // warnings as they happened.
        if let Some(notice) = &pool.flex_notice {
            vivacity_core::say!("{notice}");
        }
        vivacity_core::say!("Updating dependencies");
        for w in &pool.warnings {
            vivacity_core::say!("{w}");
        }
        lap("pool", &t);
        let pool = if std::env::var("COMPOSER_POOL_OPTIMIZER").as_deref() == Ok("0") {
            pool
        } else {
            PoolOptimizer::new().optimize(&self.request, &pool, &self.arena, &mut policy)
        };
        lap("optimize", &t);
        let before = std::time::Instant::now();
        let solved = self.solve(&pool, &mut policy, filter);
        let completed = resolution_completed(before);
        let mut report = match solved {
            Ok(mut r) => {
                r.events.push(completed);
                r
            }
            Err(SolveError::Problems(problems)) => {
                if let Some(reason) = self.extension_problem_without_php(&problems) {
                    return Err(unsupported(reason));
                }
                // `Solver::solve` wrote its duration before throwing.
                say_event(&completed);
                // `Installer::doUpdate`: the headline, the pretty problems,
                // the `--no-dev` warning.
                // The headline and the warning at QUIET, the problems at the
                // default level.
                let pretty = self.pretty_problems(&pool, &problems, false);
                let head =
                    "Your requirements could not be resolved to an installable set of packages.";
                // `$ghe->emit($err."\n".$prettyProblem)` (Installer.php:554).
                let annotation = Some(format!("{head}\n{pretty}"));
                let mut parts = vec![
                    (vivacity_core::io::Verbosity::Quiet, head.to_owned()),
                    (vivacity_core::io::Verbosity::Normal, pretty),
                ];
                if !self.installer_dev_mode {
                    parts.push((vivacity_core::io::Verbosity::Quiet, "Running update with --no-dev does not mean require-dev is ignored, it just means the packages will not be installed. If dev requirements are blocking the update you have to resolve those problems.".to_owned()));
                }
                let message = parts
                    .iter()
                    .map(|(_, p)| p.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(SessionError {
                    message,
                    kind: SessionErrorKind::Unsolvable,
                    stdout: None,
                    parts,
                    annotation,
                    preface: Box::default(),
                });
            }
            Err(SolveError::Bug(b)) => {
                say_event(&completed);
                return Err(SessionError::new(b));
            }
        };
        lap("solve", &t);
        // `ValidatingArrayLoader::validatePackage` on every kept package
        // (`LockTransaction::setResultPackages`, inside `Solver::solve`): a
        // SecurityException stops the update before anything else is written.
        for &idx in &report.transaction.all {
            if let Err(e) = crate::lockfile::validate_package(&self.arena[idx]) {
                say_events(&report.events);
                return Err(SessionError::new(e));
            }
        }
        // `Installer::doUpdate`, in its order: the pool and rule set sizes
        // at VERBOSE, `Nothing to modify in lock file`, then the dev
        // extraction (its own resolution line).
        use vivacity_core::io::Verbosity;
        report.events.push((
            Verbosity::Verbose,
            format!("Analyzed {} packages to resolve dependencies", pool.len()),
        ));
        report.events.push((
            Verbosity::Verbose,
            format!("Analyzed {} rules to resolve dependencies", report.rules),
        ));
        if report.transaction.transaction.operations.is_empty() {
            report.events.push((
                Verbosity::Normal,
                "Nothing to modify in lock file".to_owned(),
            ));
        }
        drop(pool);
        let mut transaction = std::mem::replace(&mut report.transaction, LockTransaction::empty());
        if let Err(e) =
            self.extract_dev_packages(&mut transaction, &mut policy, filter, &mut report.events)
        {
            // Composer wrote the lines above before the extraction failed.
            say_events(&report.events);
            return Err(e);
        }
        lap("extract dev", &t);
        let lock = self.lock_json(&transaction, manifest_text)?;
        lap("lock data", &t);
        report.transaction = transaction;
        Ok((lock, report))
    }

    /// `SolverProblemsException::getPrettyString` for problems found on
    /// `pool` with this session's request.
    pub fn pretty_problems(
        &mut self,
        pool: &Pool,
        problems: &[crate::solver::SolvedProblem],
        is_dev_extraction: bool,
    ) -> String {
        let mut ctx = crate::problem::MessageContext {
            arena: &mut self.arena,
            set: &self.set,
            pool,
            request: &self.request,
            is_verbose: vivacity_core::io::shows(vivacity_core::io::Verbosity::Verbose),
            ini_files: &self.ini_files,
            loaded_extensions: &self.loaded_extensions,
        };
        crate::problem::pretty_string(&mut ctx, problems, is_dev_extraction)
    }

    /// `createPool` with the PoolOptimizer (unless `COMPOSER_POOL_OPTIMIZER=0`),
    /// like `Installer::doUpdate`.
    /// `RepositorySet::findPackages($name)` on the `CompositeRepository` of
    /// `require` (platform then project repositories, all merged) with a
    /// `RepositorySet` reduced to `minimum-stability` (no flags);
    /// `ignore_stability` means `ALLOW_UNACCEPTABLE_STABILITIES`.
    pub fn find_packages_for_require(
        &mut self,
        name: &str,
        ignore_stability: bool,
    ) -> Result<Vec<usize>, SessionError> {
        let name = name.to_lowercase();
        let mut acceptable = BTreeMap::new();
        let min = crate::version::stability_rank(&self.root.minimum_stability);
        for st in ["stable", "RC", "beta", "alpha", "dev"] {
            let rank = crate::version::stability_rank(st);
            if ignore_stability || rank <= min {
                acceptable.insert(st.to_owned(), rank);
            }
        }
        let flags = BTreeMap::new();
        let already = BTreeMap::new();
        let map = vec![(name.clone(), Constraint::MatchAll)];
        let mut found: Vec<usize> = Vec::new();
        let (_, ids) = crate::pool::array_repository_load_packages(
            &self.platform,
            &map,
            &acceptable,
            &flags,
            &already,
            &self.arena,
        );
        found.extend(ids);
        // `PackageDiscoveryTrait::getRepos`: the platform, then the
        // manifest's repositories in order — `path` ones included, read as
        // the array repositories they are.
        for (i, repo) in self.set.repositories.iter().enumerate() {
            match repo {
                Repository::Composer(repo) => {
                    let (_, ids) = repo
                        .load_packages(
                            &map,
                            &acceptable,
                            &flags,
                            &already,
                            Origin::Repository(i),
                            &mut self.arena,
                        )
                        .map_err(SessionError::from)?;
                    found.extend(ids);
                }
                Repository::Path(repo) => {
                    let (_, ids) = crate::pool::array_repository_load_packages(
                        &repo.members,
                        &map,
                        &acceptable,
                        &flags,
                        &already,
                        &self.arena,
                    );
                    found.extend(ids);
                }
                Repository::Root(_) | Repository::Platform(_) | Repository::Locked(_) => {}
            }
        }
        Ok(found)
    }

    pub fn create_optimized_pool(
        &mut self,
        policy: &mut DefaultPolicy,
    ) -> Result<Pool, SessionError> {
        let pool = self.create_filtered_pool()?;
        if std::env::var("COMPOSER_POOL_OPTIMIZER").as_deref() == Ok("0") {
            return Ok(pool);
        }
        Ok(PoolOptimizer::new().optimize(&self.request, &pool, &self.arena, policy))
    }

    /// `buildPool` up to the policy filters (advisories, lists), without the
    /// optimizer; their warnings go into `pool.warnings`.
    pub fn create_filtered_pool(&mut self) -> Result<Pool, SessionError> {
        let mut pool = self.create_pool()?;
        let before = pool.len();
        // The policy filters rebuild the pool: what PRE_POOL_CREATE noted
        // rides along.
        let flex_notice = pool.flex_notice.take();
        let mut warnings = Vec::new();
        pool = crate::pool_filters::security_advisory_filter(
            pool,
            &self.arena,
            &self.set.repositories,
            &self.request,
            &self.policy_config,
            &mut warnings,
        )
        .map_err(SessionError::from)?;
        pool = crate::pool_filters::filter_list_filter(
            pool,
            &self.arena,
            &self.set.repositories,
            &self.request,
            &self.policy_config,
            "update",
            &mut warnings,
        )
        .map_err(SessionError::from)?;
        pool.warnings.extend(warnings);
        pool.flex_notice = flex_notice;
        if std::env::var_os("VIVACITY_TRACE").is_some() {
            eprintln!(
                "trace: policy filters      {before} → {} package versions ({} removed by lists)",
                pool.len(),
                pool.filter_list_removed
                    .values()
                    .map(Vec::len)
                    .sum::<usize>()
            );
        }
        Ok(pool)
    }

    /// `Solver::solve` on this pool; returns the transaction and the
    /// decisions (pool literals, in order) with the rule set size.
    pub fn solve(
        &self,
        pool: &Pool,
        policy: &mut DefaultPolicy,
        filter: &PlatformRequirementFilter,
    ) -> Result<SolveReport, SolveError> {
        let mut solver = Solver::new(pool, &self.arena);
        let transaction = solver.solve(&self.request, policy, filter)?;
        let decisions = solver.decisions.queue.iter().map(|d| d.literal).collect();
        Ok(SolveReport {
            learned: solver
                .rules
                .ids_of_type(crate::rule::RuleType::Learned)
                .len(),
            rules: solver.rule_set_size(),
            decisions,
            transaction,
            events: Vec::new(),
        })
    }
}

/// `Solver.php:231`: `Dependency resolution completed in %.3f seconds` at
/// VERBOSE, after every resolution, before its problems are thrown.
fn resolution_completed(before: std::time::Instant) -> (vivacity_core::io::Verbosity, String) {
    (
        vivacity_core::io::Verbosity::Verbose,
        format!(
            "Dependency resolution completed in {:.3} seconds",
            before.elapsed().as_secs_f64()
        ),
    )
}

/// A session event, written at its level.
fn say_event((level, line): &(vivacity_core::io::Verbosity, String)) {
    vivacity_core::say_at!(*level, "{line}");
}

/// The held lines, in order — on the error paths, where the command never
/// gets a report to print them from.
fn say_events(events: &[(vivacity_core::io::Verbosity, String)]) {
    events.iter().for_each(say_event);
}

/// What `verify_install` reads, gathered by `install` before it starts the
/// verification on its own thread.
pub struct InstallVerifyInput {
    pub project_dir: std::path::PathBuf,
    pub composer_home: Option<std::path::PathBuf>,
    pub http: Option<HttpTransports>,
    pub cache_repo_dir: Option<std::path::PathBuf>,
    /// composer.json as written: the merged config (repositories, `policy`)
    /// the list filter reads.
    pub manifest_text: String,
    /// The root package Composer works with (merged by composer-merge-plugin
    /// when it is active): its platform requirements and the fixed root.
    pub root_manifest: Value,
    /// `Installer::$devMode`.
    pub dev_mode: bool,
    /// `--no-blocking` / `--no-security-blocking`.
    pub no_blocking: bool,
    /// `--ignore-platform-reqs`.
    pub ignore_platform_reqs: bool,
    /// `--ignore-platform-req`.
    pub ignore_platform_req: Vec<String>,
    /// composer-merge-plugin's `prependRepositories`: the repositories its
    /// merged files declare, at the head of the manager — the list filter
    /// asks them too.
    pub prepended_repositories: Vec<Value>,
    /// Set by `install` when it stops before the join (missing
    /// requirements, a hand-over): the thread then skips what is left after
    /// its network wait — in a program that embeds vivacity it would
    /// otherwise keep probing and solving after the command returned.
    pub abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// What `Installer::doInstall` concluded about the lock.
#[derive(Debug, PartialEq, Eq)]
pub enum InstallVerdict {
    /// The solve kept the lock as it is.
    Installable,
    /// `SolverProblemsException::getPrettyString`.
    Problems(String),
    /// The solve succeeded with operations: the lock cannot be installed
    /// without changes.
    NeedsChanges,
    /// A kept package failed `ValidatingArrayLoader::validatePackage`
    /// (`LockTransaction`, inside the solve): the exception's message.
    Invalid(String),
}

#[derive(Debug)]
pub struct InstallVerification {
    /// The list filter's warnings (unreachable sources ignored per policy).
    pub warnings: Vec<String>,
    /// php could not be probed: the lock was solved without php, its
    /// extensions and libraries (their requirements ignored), the
    /// `composer-*` packages kept.
    pub php_missing: bool,
    pub verdict: InstallVerdict,
    /// `runSat`'s duration (`Solver::sat_time`): the
    /// `Dependency resolution completed` line of `doInstall`, which leaves
    /// out the list filter, the platform, the pool and the rules.
    pub resolution: std::time::Duration,
}

/// The lock's settings `Installer` reads when it does not update
/// (`createRepositorySet(false)`, `createPolicy(false)`,
/// `createPlatformRepo(false)`).
struct LockSettings {
    minimum_stability: String,
    stability_flags: BTreeMap<String, i32>,
    /// `null` for a lock written before Composer recorded them: the root's
    /// `prefer-stable` then, and `--prefer-lowest` (which `install` does not
    /// have).
    prefer_stable: Option<bool>,
    prefer_lowest: Option<bool>,
}

impl LockSettings {
    fn of(lock: &Value) -> LockSettings {
        // `$lockData[$key] ?? null`, then PHP's truthiness.
        let truthy = |key: &str| match lock.get(key)? {
            Value::Null => None,
            Value::Bool(b) => Some(*b),
            Value::Number(n) => Some(n.as_f64() != Some(0.0)),
            Value::String(s) => Some(!(s.is_empty() || s == "0")),
            Value::Array(a) => Some(!a.is_empty()),
            Value::Object(o) => Some(!o.is_empty()),
        };
        LockSettings {
            minimum_stability: lock
                .get("minimum-stability")
                .and_then(Value::as_str)
                .unwrap_or("stable")
                .to_owned(),
            stability_flags: lock
                .get("stability-flags")
                .and_then(Value::as_object)
                .map(|flags| {
                    flags
                        .iter()
                        .filter_map(|(n, v)| Some((n.clone(), i32::try_from(v.as_i64()?).ok()?)))
                        .collect()
                })
                .unwrap_or_default(),
            prefer_stable: truthy("prefer-stable"),
            prefer_lowest: truthy("prefer-lowest"),
        }
    }
}

/// `$this->package->getRequires()`, plus `getDevRequires()` in dev mode:
/// `array_merge` keyed by target (a `require-dev` link replaces the
/// `require` one at its position).
fn root_requires(root: &Package, dev_mode: bool) -> crate::package::Links {
    let mut links = root.requires.clone();
    if dev_mode {
        for link in root.dev_requires.iter() {
            links.insert(link.clone());
        }
    }
    links
}

/// `Installer::createRequest` and what `doInstall` adds to it before the
/// solve: the root (and its alias), the platform packages the root does not
/// provide at their version, every locked package fixed; then
/// `requireName` for the root's platform requirements, and for the lock's
/// (`platform`, plus `platform-dev` in dev mode) the root does not declare.
fn install_request(
    arena: &[Package],
    (fixed_root, fixed_root_alias): (usize, Option<usize>),
    platform: &[usize],
    locked: &[usize],
    root_links: &crate::package::Links,
    lock: &Value,
    dev_mode: bool,
) -> Result<Request, SessionError> {
    use crate::package::LinkType;
    let mut request = Request::new(Some(locked.to_vec()));
    if let Some(a) = fixed_root_alias {
        request.fix_package(a);
    }
    request.fix_package(fixed_root);
    for &p in platform {
        let provided_here = arena[fixed_root]
            .provides
            .get(&arena[p].name)
            .is_some_and(|l| {
                l.constraint
                    .matches(&Constraint::new(Op::Eq, arena[p].version.clone()))
            });
        if !provided_here {
            request.fix_package(p);
        }
    }
    for &idx in locked {
        request.fix_locked_package(idx);
    }
    for link in root_links.iter() {
        if crate::platform::is_platform_package(&link.target) {
            request.require_name_pretty(
                &link.target,
                Some(link.constraint.clone()),
                &link.pretty_constraint,
            )?;
        }
    }
    // `Locker::getPlatformRequirements`: `parseLinks('__root__', '1.0.0', …)`
    // of `platform`, `array_merge`d with `platform-dev` in dev mode.
    let parse = |key: &str| {
        crate::loader::parse_links("__root__", "1.0.0", LinkType::Require, lock.get(key), false)
            .map_err(|e| SessionError::new(format!("composer.lock: {}", e.0)))
    };
    let mut lock_links = parse("platform")?;
    if dev_mode {
        for link in parse("platform-dev")?.iter() {
            lock_links.insert(link.clone());
        }
    }
    for link in lock_links.iter() {
        if root_links.get(&link.target).is_none() {
            request.require_name_pretty(
                &link.target,
                Some(link.constraint.clone()),
                &link.pretty_constraint,
            )?;
        }
    }
    Ok(request)
}

/// `Installer::createRepositorySet(false, …)`: the lock's stability, the
/// root's stability flag, the locked versions as root requirements
/// (`=` constraints, filtered by the platform requirement filter), and the
/// root, platform and lock repositories.
fn install_repository_set(
    arena: &[Package],
    root: &RootPackage,
    root_members: Vec<usize>,
    platform: &[usize],
    locked: &[usize],
    settings: &LockSettings,
    filter: &PlatformRequirementFilter,
) -> RepositorySet {
    let mut requires: OrderedMap<Constraint> = OrderedMap::default();
    for &idx in locked {
        let p = &arena[idx];
        if filter.is_ignored(&p.name) {
            continue;
        }
        let constraint = Constraint::new(Op::Eq, p.version.clone());
        let constraint = filter.filter_constraint(&p.name, &constraint, true);
        requires.insert(&p.name, constraint);
    }
    // `$stabilityFlags[$this->package->getName()]`: the version of the
    // root alias if there is one.
    let mut stability_flags = settings.stability_flags.clone();
    let root_version = match &root.branch_alias {
        Some((normalized, _)) => normalized.clone(),
        None => root.package.version.clone(),
    };
    stability_flags.insert(
        root.package.name.clone(),
        stability_rank(parse_stability(&root_version)),
    );
    let mut set = RepositorySet::new(
        &settings.minimum_stability,
        stability_flags,
        &[],
        root.references.clone(),
        requires,
        BTreeMap::new(),
    );
    set.add_repository(Repository::Root(root_members));
    set.add_repository(Repository::Platform(platform.to_vec()));
    set.add_repository(Repository::Locked(locked.to_vec()));
    set
}

/// `createPlatformRepo(false)`: the probed platform under the overrides
/// recorded in the lock (`platform-overrides`), not the manifest's
/// `config.platform`.
fn locked_platform(probed: &[Value], lock: &Value) -> Result<Vec<Package>, SessionError> {
    let empty = Map::new();
    let overrides = lock
        .get("platform-overrides")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    platform_packages(probed, overrides).map_err(SessionError::from)
}

/// The fixed root (`$fixedRootPackage`: requires emptied) and its alias,
/// pushed into the arena; the root repository's members, alias first.
fn push_fixed_root(arena: &mut Vec<Package>, root: &RootPackage) -> (usize, Option<usize>) {
    let mut fixed = root.package.clone();
    fixed.requires = Default::default();
    fixed.dev_requires = Default::default();
    arena.push(fixed);
    let fixed_root = arena.len() - 1;
    let alias = root.branch_alias.as_ref().map(|(normalized, pretty)| {
        let a = arena[fixed_root].alias(fixed_root, normalized, pretty);
        arena.push(a);
        arena.len() - 1
    });
    (fixed_root, alias)
}

/// The locked pool's ingredients that do not depend on the root version:
/// the lock, its packages and the platform's, already in the arena.
struct LockedPool<'a> {
    lock: &'a Value,
    settings: LockSettings,
    locked: Vec<usize>,
    platform: Vec<usize>,
    dev_mode: bool,
    /// `--ignore-platform-req(s)` (`IgnoreAll` without php).
    filter: PlatformRequirementFilter,
}

/// What the final solve needs from the list filter: the arena indices of
/// the locked packages it removed and their records.
type Filtered = (BTreeSet<usize>, crate::pool::FilterListRemoved);

impl LockedPool<'_> {
    /// The repository set, the request and the pool (no optimizer, no
    /// filter yet) around `root`, pushed into the arena as the fixed root.
    fn build(
        &self,
        arena: &mut Vec<Package>,
        root: &RootPackage,
    ) -> Result<(RepositorySet, Request, Pool), SessionError> {
        let (fixed_root, alias) = push_fixed_root(arena, root);
        let members: Vec<usize> = alias.into_iter().chain([fixed_root]).collect();
        let set = install_repository_set(
            arena,
            root,
            members,
            &self.platform,
            &self.locked,
            &self.settings,
            &self.filter,
        );
        let mut request = install_request(
            arena,
            (fixed_root, alias),
            &self.platform,
            &self.locked,
            &root_requires(&root.package, self.dev_mode),
            self.lock,
            self.dev_mode,
        )?;
        let pool = set.create_pool(
            &mut request,
            arena,
            None,
            crate::pool::INSTALLER_IGNORED_TYPES,
        )?;
        Ok((set, request, pool))
    }

    /// The pool around the real root, less what the list filter removed,
    /// then `Solver::solve` and, on failure, `getPrettyString`; with the
    /// SAT's duration, measured whatever the verdict (Composer writes the
    /// line before it throws).
    fn solve(
        &self,
        arena: &mut Vec<Package>,
        root: &RootPackage,
        filtered: Option<Filtered>,
        probed: Option<&[Value]>,
    ) -> Result<(InstallVerdict, std::time::Duration), SessionError> {
        let (set, request, mut pool) = self.build(arena, root)?;
        if let Some((removed, records)) = filtered {
            // The filter saw every locked package; the pool holds those the
            // lock's stability accepts. What it removed among them leaves,
            // with their records (`filterListRemovedVersions`) — and only
            // theirs: Composer never filtered the others.
            let removed_here: Vec<usize> = pool
                .packages
                .iter()
                .copied()
                .filter(|idx| removed.contains(idx))
                .collect();
            if !removed_here.is_empty() {
                let mut kept_records = crate::pool::FilterListRemoved::new();
                for &idx in &removed_here {
                    let p = &arena[idx];
                    for name in p.names(false) {
                        let Some(record) = records
                            .get(&name)
                            .and_then(|versions| versions.iter().find(|(v, _)| *v == p.version))
                        else {
                            continue;
                        };
                        let versions = kept_records.entry(name).or_default();
                        if !versions.iter().any(|(v, _)| *v == p.version) {
                            versions.push(record.clone());
                        }
                    }
                }
                let kept: Vec<usize> = pool
                    .packages
                    .iter()
                    .copied()
                    .filter(|idx| !removed.contains(idx))
                    .collect();
                pool = pool.with_packages(kept, arena);
                pool.filter_list_removed = kept_records;
            }
        }
        // `createPolicy(false)`: the lock's preferences, the root's
        // `prefer-stable` for a lock without them.
        let mut policy = DefaultPolicy::new(
            self.settings.prefer_stable.unwrap_or(root.prefer_stable),
            self.settings.prefer_lowest.unwrap_or(false),
            None,
        );
        let (solved, sat_time) = {
            let mut solver = Solver::new(&pool, arena);
            let solved = solver.solve(&request, &mut policy, &self.filter);
            (solved, solver.sat_time)
        };
        let verdict = match solved {
            Ok(transaction) => {
                // `LockTransaction::setResultPackages` validates every kept
                // package, as in `update`.
                for &idx in &transaction.all {
                    if let Err(e) = crate::lockfile::validate_package(&arena[idx]) {
                        return Ok((InstallVerdict::Invalid(e), sat_time));
                    }
                }
                Ok(if transaction.transaction.operations.is_empty() {
                    InstallVerdict::Installable
                } else {
                    InstallVerdict::NeedsChanges
                })
            }
            Err(SolveError::Problems(problems)) => {
                let ini_files = probed.map(crate::platform::ini_files).unwrap_or_default();
                let loaded_extensions = probed
                    .map(crate::platform::loaded_extensions)
                    .unwrap_or_default();
                let mut ctx = crate::problem::MessageContext {
                    arena,
                    set: &set,
                    pool: &pool,
                    request: &request,
                    is_verbose: vivacity_core::io::shows(vivacity_core::io::Verbosity::Verbose),
                    ini_files: &ini_files,
                    loaded_extensions: &loaded_extensions,
                };
                Ok(InstallVerdict::Problems(crate::problem::pretty_string(
                    &mut ctx, &problems, false,
                )))
            }
            Err(SolveError::Bug(b)) => Err(SessionError::new(b)),
        }?;
        Ok((verdict, sat_time))
    }
}

/// `Installer::doInstall` before it lays out a lock it did not just solve:
/// the request of `install_request`, the pool on [root, platform, lock]
/// through the filter list filter in `install` scope, then
/// `Solver::solve` under the `--ignore-platform-req(s)` filter. One solve,
/// one list of problems: the root's platform requirements, the lock's,
/// the conflicts of the fixed root and the locked versions a list blocks.
///
/// Runs on its own thread while `install` reads the local repository, in
/// the order that keeps the network wait first: the list filter runs on
/// the locked packages alone (it never judges the root or the platform),
/// then the platform is probed and the root version — detected by the
/// command for its missing requirements, git runs once — received; the
/// pool built around them loses what the filter removed.
pub fn verify_install(
    input: InstallVerifyInput,
    root_version: std::sync::mpsc::Receiver<vivacity_core::root_version::RootVersion>,
) -> Result<InstallVerification, SessionError> {
    let manifest: Value = serde_json::from_str(&input.manifest_text)
        .map_err(|e| SessionError::new(format!("composer.json: {e}")))?;
    let config = MergedConfig::load(&manifest, input.composer_home.as_deref())?;
    let mut policy =
        crate::policy_config::PolicyConfig::from_raw(&config.policy).map_err(SessionError::from)?;
    policy
        .apply_no_blocking(input.no_blocking)
        .map_err(SessionError::from)?;
    let lock_text = std::fs::read_to_string(input.project_dir.join("composer.lock"))
        .map_err(|e| SessionError::new(format!("composer.lock: {e}")))?;
    let lock: Value = serde_json::from_str(&lock_text)
        .map_err(|e| SessionError::new(format!("composer.lock: {e}")))?;

    let mut arena: Vec<Package> = Vec::new();
    let locked = crate::repository::locked_repository_with(&lock, &mut arena, input.dev_mode)
        .map_err(SessionError::from)?;

    let mut warnings = Vec::new();
    let filtered: Option<Filtered> = if policy.malware_blocks("install") {
        // Only `composer` repositories carry lists; the other types (which
        // `install` otherwise accepts) are left aside here.
        let mut repositories: Vec<Repository> = Vec::new();
        let prepended: Vec<RepoConfig> = input
            .prepended_repositories
            .iter()
            .enumerate()
            .map(|(i, definition)| RepoConfig {
                key: RepoKey::Indexed(i as u64),
                definition: definition.clone(),
            })
            .collect();
        for repo in prepended.iter().chain(&config.repositories) {
            if repo.definition.get("type").and_then(Value::as_str) != Some("composer") {
                continue;
            }
            // The constructor does no I/O: an error here is a configuration
            // error, fatal as in Composer.
            let mut opened = open_repository_with(
                repo,
                input.http.as_ref(),
                input.cache_repo_dir.as_deref(),
                true,
            )?;
            if let Repository::Composer(r) = &mut opened {
                r.hold_output();
            }
            repositories.push(opened);
        }
        let pool = Pool::new(locked.clone(), Vec::new(), &arena);
        let before = pool.packages.clone();
        // (In `install` scope the filter does not read the request.)
        let request = Request::new(Some(locked.clone()));
        let mut filter_warnings = Vec::new();
        let pool = match crate::pool_filters::filter_list_filter(
            pool,
            &arena,
            &repositories,
            &request,
            &policy,
            "install",
            &mut filter_warnings,
        ) {
            Ok(pool) => pool,
            // What the repositories wrote while loading was written before
            // the exception (a degraded repository, then another that
            // fails): kept with the error.
            Err(e) => {
                let mut e = SessionError::from(e);
                let mut preface = Vec::new();
                for repo in &repositories {
                    if let Repository::Composer(r) = repo {
                        preface.extend(r.take_held());
                    }
                }
                e.preface = preface.into_boxed_slice();
                return Err(e);
            }
        };
        // What the repositories wrote while loading came first.
        for repo in &repositories {
            if let Repository::Composer(r) = repo {
                warnings.extend(r.take_held());
            }
        }
        warnings.extend(filter_warnings);
        let removed: BTreeSet<usize> = before
            .into_iter()
            .filter(|idx| pool.id_of(*idx).is_none())
            .collect();
        Some((removed, pool.filter_list_removed))
    } else {
        None
    };
    let abandoned = || {
        if input.abandoned.load(std::sync::atomic::Ordering::Relaxed) {
            Err(SessionError::new("the lock verification was abandoned"))
        } else {
            Ok(())
        }
    };
    abandoned()?;

    // `createPlatformRepo(false)`. Without php (no reference: Composer
    // cannot run), the `composer-*` packages and the lock's overrides only,
    // and the requirements on php, its extensions and libraries ignored.
    let probed = probe().ok();
    let packages = match &probed {
        Some(probed) => locked_platform(probed, &lock)?,
        None => locked_platform(&[], &lock)?,
    };
    let mut platform: Vec<usize> = Vec::new();
    for p in packages {
        arena.push(p);
        platform.push(arena.len() - 1);
    }
    let filter = if input.ignore_platform_reqs {
        PlatformRequirementFilter::IgnoreAll
    } else if probed.is_none() {
        let mut list = input.ignore_platform_req.clone();
        list.extend(["php", "php-*", "hhvm", "ext-*", "lib-*"].map(str::to_owned));
        PlatformRequirementFilter::from_list(&list)
    } else if !input.ignore_platform_req.is_empty() {
        PlatformRequirementFilter::from_list(&input.ignore_platform_req)
    } else {
        PlatformRequirementFilter::IgnoreNothing
    };
    let locked_pool = LockedPool {
        lock: &lock,
        settings: LockSettings::of(&lock),
        locked,
        platform,
        dev_mode: input.dev_mode,
        filter,
    };
    let rv = root_version
        .recv()
        .map_err(|_| SessionError::new("the root version was not detected"))?;
    let root = RootPackage::load_with_version(&input.root_manifest, rv)
        .map_err(|e| SessionError::new(e.0))?;
    abandoned()?;
    let (verdict, resolution) =
        locked_pool.solve(&mut arena, &root, filtered, probed.as_deref())?;
    Ok(InstallVerification {
        warnings,
        php_missing: probed.is_none(),
        verdict,
        resolution,
    })
}

/// `RepositoryManager::createRepository` for the `composer` and `path`
/// types (`vcs` and the others are not supported yet). A `path` repository
/// reads its packages now, into the arena, as `origin`.
fn open_repository(
    repo: &RepoConfig,
    http: Option<&HttpTransports>,
    cache_repo_dir: Option<&Path>,
    project_dir: &Path,
    origin: Origin,
    arena: &mut Vec<Package>,
) -> Result<Repository, SessionError> {
    if repo.definition.get("type").and_then(Value::as_str) == Some("path") {
        if repo.definition.get("only").is_some()
            || repo.definition.get("exclude").is_some()
            || repo.definition.get("canonical").is_some()
        {
            return Err(SessionError::new(format!(
                "repository filters (only/exclude/canonical) are not supported by vivacity update yet ({})",
                key_string(&repo.key)
            )));
        }
        let path_repo = crate::path_repo::open(&repo.definition, project_dir, origin, arena)
            .map_err(SessionError::from)?;
        return Ok(Repository::Path(path_repo));
    }
    open_repository_with(repo, http, cache_repo_dir, false)
}

/// `for_policies`: a repository with `only`/`exclude`/`canonical`
/// (`FilterRepository`) is accepted (the advisory and list paths honor
/// `only`/`exclude`) where resolution still rejects it.
fn open_repository_with(
    repo: &RepoConfig,
    http: Option<&HttpTransports>,
    cache_repo_dir: Option<&Path>,
    for_policies: bool,
) -> Result<Repository, SessionError> {
    let def = &repo.definition;
    let kind = def.get("type").and_then(Value::as_str).ok_or_else(|| {
        SessionError::new(format!(
            "Repository \"{}\" ({def}) must have a type defined",
            key_string(&repo.key)
        ))
    })?;
    if kind != "composer" {
        return Err(SessionError::new(format!(
            "repository type \"{kind}\" is not supported by vivacity update yet ({})",
            key_string(&repo.key)
        )));
    }
    if !for_policies
        && (def.get("only").is_some()
            || def.get("exclude").is_some()
            || def.get("canonical").is_some())
    {
        return Err(SessionError::new(format!(
            "repository filters (only/exclude/canonical) are not supported by vivacity update yet ({})",
            key_string(&repo.key)
        )));
    }
    let url = def.get("url").and_then(Value::as_str).ok_or_else(|| {
        SessionError::new(format!("repository {} has no url", key_string(&repo.key)))
    })?;
    let transport: Box<dyn crate::repository::Transport> = if url.starts_with("file://") {
        Box::new(FileTransport)
    } else if url.starts_with("http://") || url.starts_with("https://") || !url.contains("://") {
        match http {
            Some(h) => Box::new(HttpTransport {
                fetch: h.0.clone(),
                fetch_many: h.1.clone(),
                post: h.2.clone(),
            }),
            None => {
                return Err(SessionError::new(format!(
                    "remote composer repositories need a network transport ({url})"
                )))
            }
        }
    } else {
        return Err(SessionError::new(format!(
            "unsupported repository url scheme ({url})"
        )));
    };
    let mut repo = ComposerRepository::open(url, transport).map_err(SessionError::from)?;
    if let Some(options) = def.get("options") {
        repo.options = options.clone();
    }
    repo.set_user_filter(def.get("filter"))
        .map_err(SessionError::from)?;
    if for_policies {
        repo.set_name_filter(def.get("only"), def.get("exclude"))
            .map_err(SessionError::from)?;
    }
    if let Some(dir) = cache_repo_dir {
        repo.cache = Some(crate::metacache::MetadataCache::new(dir, &repo.url));
    }
    Ok(Repository::Composer(Box::new(repo)))
}

fn key_string(key: &RepoKey) -> String {
    match key {
        RepoKey::Named(n) => n.clone(),
        RepoKey::Indexed(i) => i.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(repos: &[RepoConfig]) -> Vec<String> {
        repos.iter().map(|r| key_string(&r.key)).collect()
    }

    #[test]
    fn merges_repositories_like_composer_config() {
        let mut cfg = MergedConfig::load(&json!({}), None).unwrap();
        assert_eq!(names(&cfg.repositories), vec!["packagist.org"]);
        // global config: snapshot + packagist disabled.
        cfg.merge(&json!({"repositories": {"snapshot": {"type": "composer", "url": "file:///s"}, "packagist.org": false}}));
        assert_eq!(names(&cfg.repositories), vec!["snapshot"]);
        // composer.json: an indexed repository goes first.
        cfg.merge(&json!({"repositories": [{"type": "composer", "url": "https://packages.drupal.org/8"}]}));
        assert_eq!(names(&cfg.repositories), vec!["0", "snapshot"]);
        // two indexed ones while 0 exists: 1 takes its key, 0 is renumbered
        // (`$this->repositories[] = ...`); the new ones stay in front.
        cfg.merge(
            &json!({"repositories": [{"type": "vcs", "url": "a"}, {"type": "vcs", "url": "b"}]}),
        );
        assert_eq!(names(&cfg.repositories), vec!["2", "1", "0", "snapshot"]);
        assert_eq!(cfg.repositories[0].definition["url"], "a");
        assert_eq!(cfg.repositories[1].definition["url"], "b");
        assert_eq!(
            cfg.repositories[2].definition["url"],
            "https://packages.drupal.org/8"
        );
    }

    #[test]
    fn packagist_url_disables_default() {
        let mut cfg = MergedConfig::load(&json!({}), None).unwrap();
        cfg.merge(
            &json!({"repositories": [{"type": "composer", "url": "https://repo.packagist.org"}]}),
        );
        assert_eq!(names(&cfg.repositories), vec!["0"]);
        let mut cfg = MergedConfig::load(&json!({}), None).unwrap();
        cfg.merge(&json!({"repositories": [{"packagist": false}]}));
        assert!(cfg.repositories.is_empty());
    }

    // -- install verification ---------------------------------------------

    /// A probed PHP 8.2.5 with mbstring and intl.
    fn probed() -> Vec<Value> {
        vec![
            json!({"kind": "php", "name": "php", "version": "8.2.5", "description": "The PHP interpreter"}),
            json!({"kind": "ext", "name": "mbstring", "version": "8.2.5"}),
            json!({"kind": "ext", "name": "intl", "version": "8.2.5"}),
        ]
    }

    fn root_version() -> vivacity_core::root_version::RootVersion {
        vivacity_core::root_version::RootVersion {
            pretty_version: "1.0.0+no-version-set".to_owned(),
            version: "1.0.0.0".to_owned(),
            reference: None,
        }
    }

    /// `verify_install` without the probe, the network or the thread.
    fn verify(
        manifest: Value,
        lock: Value,
        dev_mode: bool,
        filter: PlatformRequirementFilter,
    ) -> (InstallVerdict, Request) {
        verify_flagged(manifest, lock, dev_mode, filter, &[])
    }

    /// `verify`, with the locked packages named in `flagged` removed by the
    /// list filter (a malware entry each).
    fn verify_flagged(
        manifest: Value,
        lock: Value,
        dev_mode: bool,
        filter: PlatformRequirementFilter,
        flagged: &[&str],
    ) -> (InstallVerdict, Request) {
        let mut arena = Vec::new();
        let locked = crate::repository::locked_repository_with(&lock, &mut arena, dev_mode)
            .expect("locked repository");
        let mut platform = Vec::new();
        for p in locked_platform(&probed(), &lock).expect("platform") {
            arena.push(p);
            platform.push(arena.len() - 1);
        }
        let pool = LockedPool {
            lock: &lock,
            settings: LockSettings::of(&lock),
            locked,
            platform,
            dev_mode,
            filter,
        };
        let root = RootPackage::load_with_version(&manifest, root_version()).expect("root");
        let (_, request, _) = pool.build(&mut arena.clone(), &root).expect("build");
        let filtered = (!flagged.is_empty()).then(|| {
            let mut removed = BTreeSet::new();
            let mut records = crate::pool::FilterListRemoved::new();
            for &idx in &pool.locked {
                let p = &arena[idx];
                if !flagged.contains(&p.name.as_str()) {
                    continue;
                }
                removed.insert(idx);
                let entry = crate::repository::FilterEntry {
                    package_name: p.name.clone(),
                    constraint: Constraint::MatchAll,
                    list_name: "malware".to_owned(),
                    url: None,
                    reason: None,
                    id: None,
                    source: Some("testsource".to_owned()),
                };
                records
                    .entry(p.name.clone())
                    .or_default()
                    .push((p.version.clone(), vec![entry]));
            }
            (removed, records)
        });
        let (verdict, _) = pool
            .solve(&mut arena, &root, filtered, Some(&probed()))
            .expect("solve");
        (verdict, request)
    }

    fn requires(request: &Request) -> Vec<(String, String)> {
        request
            .pretty_requires
            .iter()
            .map(|(n, p)| (n.clone(), p.clone()))
            .collect()
    }

    fn lock_with(packages: Value, dev: Value, platform: Value, platform_dev: Value) -> Value {
        json!({
            "packages": packages, "packages-dev": dev, "aliases": [],
            "minimum-stability": "stable", "stability-flags": {},
            "prefer-stable": false, "prefer-lowest": false,
            "platform": platform, "platform-dev": platform_dev,
        })
    }

    fn pair(n: &str, p: &str) -> (String, String) {
        (n.to_owned(), p.to_owned())
    }

    #[test]
    fn requires_the_roots_platform_requirements_then_the_locks_it_does_not_declare() {
        let manifest = json!({
            "name": "acme/root",
            "require": {"php": "^8.0", "ext-intl": "*", "acme/lib": "^1.0"},
            "require-dev": {"ext-intl": ">=1", "ext-mbstring": "*"},
        });
        let lock = lock_with(
            json!([]),
            json!([]),
            json!({"php": ">=8.3", "ext-mbstring": "^8", "ext-json": "*"}),
            json!({"ext-intl": "^9", "ext-zip": "*"}),
        );
        // Dev mode: `array_merge(require, require-dev)` — the `require-dev`
        // constraint at the `require` position; the lock's `php`,
        // `ext-mbstring` and `ext-intl` are the root's already.
        let (_, request) = verify(
            manifest.clone(),
            lock.clone(),
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(
            requires(&request),
            vec![
                pair("php", "^8.0"),
                pair("ext-intl", ">=1"),
                pair("ext-mbstring", "*"),
                pair("ext-json", "*"),
                pair("ext-zip", "*"),
            ]
        );
        // `--no-dev`: no `require-dev`, no `platform-dev`; the lock's
        // `ext-mbstring` is no longer the root's.
        let (_, request) = verify(
            manifest,
            lock,
            false,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(
            requires(&request),
            vec![
                pair("php", "^8.0"),
                pair("ext-intl", "*"),
                pair("ext-mbstring", "^8"),
                pair("ext-json", "*"),
            ]
        );
    }

    #[test]
    fn the_roots_constraint_wins_over_the_locks() {
        let lock = lock_with(json!([]), json!([]), json!({"php": "^99"}), json!({}));
        let (verdict, _) = verify(
            json!({"require": {"php": "^8.0"}}),
            lock.clone(),
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(verdict, InstallVerdict::Installable);
        let (verdict, _) = verify(
            json!({}),
            lock,
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        let InstallVerdict::Problems(text) = verdict else {
            panic!("{verdict:?}")
        };
        assert!(
            text.contains("Root composer.json requires php ^99 but your php version (8.2.5)"),
            "{text}"
        );
    }

    #[test]
    fn locked_packages_platform_requirements_and_the_ignore_filter() {
        let lock = lock_with(
            json!([{"name": "acme/a", "version": "1.0.0", "type": "library", "require": {"ext-gd": "*"}}]),
            json!([]),
            json!({}),
            json!({}),
        );
        let (verdict, _) = verify(
            json!({"require": {"acme/a": "^1.0"}}),
            lock.clone(),
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        let InstallVerdict::Problems(text) = verdict else {
            panic!("{verdict:?}")
        };
        assert!(
            text.contains("acme/a 1.0.0 requires ext-gd * -> it is missing from your system."),
            "{text}"
        );
        for ignored in [
            PlatformRequirementFilter::IgnoreAll,
            PlatformRequirementFilter::from_list(&["ext-gd".to_owned()]),
            PlatformRequirementFilter::from_list(&["ext-*".to_owned()]),
        ] {
            let (verdict, _) = verify(
                json!({"require": {"acme/a": "^1.0"}}),
                lock.clone(),
                true,
                ignored,
            );
            assert_eq!(verdict, InstallVerdict::Installable);
        }
    }

    #[test]
    fn platform_dev_only_in_dev_mode() {
        let lock = lock_with(
            json!([]),
            json!([{"name": "acme/dev", "version": "1.0.0", "type": "library", "require": {"ext-zip": "*"}}]),
            json!({}),
            json!({"ext-zip": "*"}),
        );
        let (verdict, _) = verify(
            json!({}),
            lock.clone(),
            false,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(verdict, InstallVerdict::Installable);
        let (verdict, _) = verify(
            json!({}),
            lock,
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert!(
            matches!(verdict, InstallVerdict::Problems(_)),
            "{verdict:?}"
        );
    }

    #[test]
    fn the_locks_platform_overrides_apply() {
        let mut lock = lock_with(
            json!([]),
            json!([]),
            json!({"php": ">=8.3", "ext-gd": "*"}),
            json!({}),
        );
        let (verdict, _) = verify(
            json!({}),
            lock.clone(),
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert!(
            matches!(verdict, InstallVerdict::Problems(_)),
            "{verdict:?}"
        );
        lock["platform-overrides"] = json!({"php": "8.3.0", "ext-gd": "2.0.0"});
        let (verdict, _) = verify(
            json!({}),
            lock,
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(verdict, InstallVerdict::Installable);
    }

    #[test]
    fn the_fixed_root_keeps_its_conflicts() {
        let lock = lock_with(
            json!([{"name": "acme/a", "version": "1.0.0", "type": "library"}]),
            json!([]),
            json!({}),
            json!({}),
        );
        let (verdict, _) = verify(
            json!({"name": "acme/root", "require": {"acme/a": "*"}, "conflict": {"acme/a": "*"}}),
            lock,
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        let InstallVerdict::Problems(text) = verdict else {
            panic!("{verdict:?}")
        };
        assert!(
            text.contains("acme/root 1.0.0+no-version-set conflicts with acme/a 1.0.0."),
            "{text}"
        );
    }

    #[test]
    fn without_php_the_composer_packages_remain() {
        let names: Vec<String> = platform_packages(&[], &Map::new())
            .expect("platform")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            ["composer", "composer-plugin-api", "composer-runtime-api"]
        );
    }

    #[test]
    fn a_lock_without_preferences_leaves_them_to_the_root() {
        let mut lock = lock_with(json!([]), json!([]), json!({}), json!({}));
        assert_eq!(LockSettings::of(&lock).prefer_stable, Some(false));
        lock.as_object_mut().expect("lock").remove("prefer-stable");
        lock.as_object_mut().expect("lock").remove("prefer-lowest");
        let settings = LockSettings::of(&lock);
        assert_eq!(
            (settings.prefer_stable, settings.prefer_lowest),
            (None, None)
        );
    }

    #[test]
    fn a_locked_package_the_locks_stability_rejects_needs_changes() {
        // Outside the pool (`unacceptableFixedOrLockedPackages`) but present:
        // the solve succeeds with its removal.
        let lock = lock_with(
            json!([{"name": "acme/dev", "version": "dev-main", "type": "library"}]),
            json!([]),
            json!({}),
            json!({}),
        );
        let (verdict, _) = verify(
            json!({"require": {"acme/dev": "dev-main"}}),
            lock.clone(),
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(verdict, InstallVerdict::NeedsChanges);
        let mut flagged = lock;
        flagged["stability-flags"] = json!({"acme/dev": 20});
        let (verdict, _) = verify(
            json!({"require": {"acme/dev": "dev-main"}}),
            flagged,
            true,
            PlatformRequirementFilter::IgnoreNothing,
        );
        assert_eq!(verdict, InstallVerdict::Installable);
    }

    #[test]
    fn what_the_list_filter_removed_is_one_problem_among_the_others() {
        let lock = lock_with(
            json!([
                {"name": "acme/bad", "version": "1.1.0", "type": "library"},
                {"name": "acme/dev", "version": "dev-main", "type": "library"},
            ]),
            json!([]),
            json!({"ext-nope": "*"}),
            json!({}),
        );
        let (verdict, _) = verify_flagged(
            json!({}),
            lock,
            true,
            PlatformRequirementFilter::IgnoreNothing,
            &["acme/bad", "acme/dev"],
        );
        let InstallVerdict::Problems(text) = verdict else {
            panic!("{verdict:?}")
        };
        // The root requirement first, then the blocked package; `acme/dev`,
        // outside the pool (its stability), was never judged by the filter.
        let nope = text.find("requires PHP extension ext-nope").expect(&text);
        let bad = text
            .find("acme/bad 1.1.0 (in the lock file) was not loaded, because it was flagged as malware reported by testsource")
            .expect(&text);
        assert!(nope < bad, "{text}");
        assert!(!text.contains("acme/dev"), "{text}");
    }

    /// `doInstall`'s `Dependency resolution completed` line times `runSat`
    /// alone (Solver.php:225-228): the wait before it — here the root
    /// version, which the command sends 200 ms after the thread started —
    /// is not in the duration `verify_install` reports.
    #[test]
    fn the_resolution_duration_is_the_sats_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let package = json!({
            "name": "acme/lib", "version": "1.0.0", "version_normalized": "1.0.0.0",
            "type": "library",
            "dist": {"type": "zip", "url": "https://example.invalid/lib.zip", "reference": "abc"},
        });
        let lock = lock_with(json!([package]), json!([]), json!({}), json!({}));
        std::fs::write(tmp.path().join("composer.lock"), lock.to_string()).expect("lock");
        let manifest = json!({"name": "acme/root", "require": {"acme/lib": "^1.0"}});
        let input = InstallVerifyInput {
            project_dir: tmp.path().to_path_buf(),
            composer_home: None,
            http: None,
            cache_repo_dir: None,
            manifest_text: manifest.to_string(),
            root_manifest: manifest,
            dev_mode: true,
            no_blocking: true,
            ignore_platform_reqs: true,
            ignore_platform_req: Vec::new(),
            prepended_repositories: Vec::new(),
            abandoned: std::sync::Arc::default(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            tx.send(root_version()).expect("send");
        });
        let verified = verify_install(input, rx).expect("verify");
        sender.join().expect("sender");
        assert_eq!(verified.verdict, InstallVerdict::Installable);
        assert!(started.elapsed() >= std::time::Duration::from_millis(200));
        assert!(
            verified.resolution < std::time::Duration::from_millis(50),
            "{:?}",
            verified.resolution
        );
    }
}
