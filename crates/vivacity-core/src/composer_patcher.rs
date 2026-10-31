//! Emulation of `mlocati/composer-patcher` 1.2.6
//! (docs/reference/plugins/composer-patcher/, MIT;
//! docs/plans/remaining-fallbacks.md). The plugin applies patch files to
//! installed packages with the `patch` command; vivacity runs **the same
//! command lines, in the same order**, so the files it leaves are the ones
//! that binary leaves — nothing of a patch is interpreted here.
//!
//! - `PatchCollector::collectPatches`: the root's `extra.patches`, then,
//!   when `extra.allow-subpatches` is `true` or lists them, the installed
//!   packages' (in the local repository's order); one "Gathering patches
//!   from <name> (extra.patches)." line per source. A key is
//!   `<name>[:<constraint>]`, a value `{description: path}` or
//!   `{description: {path, levels}}`; a relative path is resolved against
//!   the source's install path (the working directory for the root).
//! - `Plugin::postOperation`: for each package of the local repository,
//!   each patch that targets it (`Patch::isForPackage`: the name, then the
//!   constraint against the normalized version): "Applying patch
//!   <source>/<description> to <name>... " and, on the same line, "done.",
//!   "patch was already applied." or the error. Per patch level (default
//!   `-p1 -p0 -p2 -p4`): `patch <level> [--no-backup-if-mismatch] -N -d
//!   <dir> -i <file> -r <tmp>/<n>.rej --dry-run`, which reads as "already
//!   applied" on exit code 1 with "Skipping patch." and "N out of N hunk(s)
//!   ignored", then the same without `--dry-run`. `patch --version` once,
//!   `patch --help` once (for `--no-backup-if-mismatch`). `<tmp>` is a
//!   directory `tempnam(sys_get_temp_dir(), 'PCH')`, removed at the end.
//! - Every patch of a package applied (or already applied): its
//!   `extra.patches_applied` = `{hash, list}`, the hash `sha1` of the
//!   sorted `sha1_file`s joined by a space, and installed.json written
//!   again.
//!
//! A package directory that holds a `.git` directory is patched as
//! `GitPatcher` does: the same dry run with `patch`, then `git -C <dir>
//! apply <level> --check -v <file>` and `git -C <dir> apply <level> <file>`.
//!
//! What is handed over before anything is written is decided by
//! [`config_issue`] and the caller (`effect_plugins`).

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};

pub const PLUGIN_NAME: &str = "mlocati/composer-patcher";
const DEFAULT_LEVELS: [&str; 4] = ["-p1", "-p0", "-p2", "-p4"];

/// One patch (`ComposerPatcher\Patch`).
#[derive(Debug, Clone)]
pub struct Patch {
    /// The source package's name (`getFromPackage()->getName()`).
    pub from_name: String,
    /// `getFromPackage()->getPrettyString()`.
    pub from_pretty: String,
    pub for_name: String,
    /// The constraint of the key, `None` for `*`.
    pub for_constraint: Option<String>,
    /// The path as configured (`getOriginalPath()`).
    pub original_path: String,
    pub local_path: PathBuf,
    pub description: String,
    pub levels: Vec<String>,
}

/// A source of patches: a package entry (installed.json / lock shape) and
/// the directory its relative paths are resolved against.
pub struct Source<'a> {
    pub package: &'a Map<String, Value>,
    pub dir: PathBuf,
}

/// The collection, with the lines its collection printed.
#[derive(Debug, Default)]
pub struct Collection {
    pub lines: Vec<String>,
    pub patches: Vec<Patch>,
}

fn name_of(p: &Map<String, Value>) -> &str {
    p.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// `isset($extra[$key])`.
fn isset<'a>(extra: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    extra.and_then(|e| e.get(key)).filter(|v| !v.is_null())
}

/// The root configuration the emulation does not cover, or a shape the
/// plugin turns into an error line or a fatal error (`handleException`,
/// the missing `new` in `resolveExplicitList`).
pub fn config_issue(root: &Value) -> Option<String> {
    let extra = root.get("extra");
    if isset(extra, "patch-temporary-folder").is_some_and(|v| v != &Value::String(String::new())) {
        return Some("extra.patch-temporary-folder".to_owned());
    }
    if let Some(v) = isset(extra, "patch-errors-as-warnings") {
        if !crate::effect_plugins::php_truthy(v) {
            return Some("extra.patch-errors-as-warnings off".to_owned());
        }
    }
    if isset(extra, "patches-file").is_some() {
        return Some("extra.patches-file".to_owned());
    }
    match isset(extra, "allow-subpatches") {
        None | Some(Value::Bool(_)) => None,
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => None,
        Some(_) => {
            Some("extra.allow-subpatches that is not a boolean or a list of names".to_owned())
        }
    }
}

/// The packages whose patches are collected besides the root's.
fn allowed_sources(root: &Value) -> AllowedSources {
    match isset(root.get("extra"), "allow-subpatches") {
        Some(Value::Bool(true)) => AllowedSources::All,
        Some(Value::Array(a)) => AllowedSources::Listed(
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        _ => AllowedSources::None,
    }
}

enum AllowedSources {
    None,
    All,
    Listed(Vec<String>),
}

impl AllowedSources {
    fn allows(&self, name: &str) -> bool {
        match self {
            AllowedSources::None => false,
            AllowedSources::All => true,
            AllowedSources::Listed(l) => l.iter().any(|n| n == name),
        }
    }
}

/// `Patch::parsePackageVersion`.
fn parse_key(key: &str) -> (String, Option<String>) {
    let mut parts = key.splitn(2, ':');
    let first = parts.next().unwrap_or_default();
    let version = parts.next().map_or("*", str::trim);
    if version.is_empty() {
        return (key.to_owned(), None);
    }
    (
        first.to_owned(),
        (version != "*").then(|| version.to_owned()),
    )
}

/// `PathResolver::resolveLocalPath` (a remote path was refused).
fn resolve_local(path: &str, base: &Path) -> PathBuf {
    let path = path
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("file://"))
        .map_or(path, |_| &path[7..]);
    let normalized = crate::pathutil::normalize_path(path);
    if crate::pathutil::is_absolute_path(&normalized) {
        return PathBuf::from(normalized);
    }
    let base = crate::pathutil::normalize_path(&base.to_string_lossy());
    PathBuf::from(format!("{base}/{normalized}"))
}

/// `^\w\w+://` (after an optional `file://`).
fn is_remote(path: &str) -> bool {
    let path = path
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("file://"))
        .map_or(path, |_| &path[7..]);
    let Some(pos) = path.find("://") else {
        return false;
    };
    pos >= 2
        && path[..pos]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The patches of one source's `extra.patches`, or why the run is handed
/// over (a shape that prints an error line, a remote path, a constraint
/// vivacity does not read).
fn explicit_list(source: &Source<'_>, patches: &Value) -> std::result::Result<Vec<Patch>, String> {
    // `getName()` (lower case) in the lines and messages,
    // `getPrettyString()` (pretty name and version) in `patches_applied`.
    let from_name = name_of(source.package).to_ascii_lowercase();
    let from_pretty = format!(
        "{} {}",
        name_of(source.package),
        source
            .package
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    if patches.as_array().is_some_and(Vec::is_empty) {
        // `[]` decodes to PHP's empty array: nothing to collect.
        return Ok(Vec::new());
    }
    let Some(map) = patches.as_object() else {
        return Err(format!("{from_name}'s extra.patches is not an object"));
    };
    let mut out = Vec::new();
    for (key, list) in map {
        let Some(list) = list.as_object() else {
            return Err(format!(
                "{from_name}'s extra.patches.{key} is not an object"
            ));
        };
        let (for_name, for_constraint) = parse_key(key);
        if let Some(c) = &for_constraint {
            if crate::constraint::Constraint::parse(c).is_err() {
                return Err(format!(
                    "{from_name}'s patch key {key}: a constraint vivacity does not read"
                ));
            }
        }
        for (description, data) in list {
            let (path, levels) = match data {
                Value::String(s) => (s.clone(), None),
                Value::Object(o) => {
                    let Some(path) = o.get("path").and_then(Value::as_str) else {
                        return Err(format!(
                            "{from_name}'s patch \"{description}\" has no string path"
                        ));
                    };
                    let levels = match o.get("levels") {
                        None | Some(Value::Null) => None,
                        Some(Value::Array(a))
                            if !a.is_empty() && a.iter().all(Value::is_string) =>
                        {
                            Some(
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect::<Vec<_>>(),
                            )
                        }
                        Some(_) => {
                            return Err(format!(
                                "{from_name}'s patch \"{description}\" has levels that are not a list of strings"
                            ))
                        }
                    };
                    (path.to_owned(), levels)
                }
                _ => {
                    return Err(format!(
                        "{from_name}'s patch \"{description}\" is neither a path nor an object"
                    ))
                }
            };
            if path.is_empty() || path.eq_ignore_ascii_case("file://") {
                return Err(format!(
                    "{from_name}'s patch \"{description}\" has an empty path"
                ));
            }
            if is_remote(&path) {
                return Err(format!(
                    "{from_name}'s patch \"{description}\" is downloaded from {path}"
                ));
            }
            out.push(Patch {
                from_name: from_name.clone(),
                from_pretty: from_pretty.clone(),
                for_name: for_name.clone(),
                for_constraint: for_constraint.clone(),
                original_path: path.clone(),
                local_path: resolve_local(&path, &source.dir),
                description: description.clone(),
                levels: levels
                    .unwrap_or_else(|| DEFAULT_LEVELS.iter().map(|l| (*l).to_owned()).collect()),
            });
        }
    }
    Ok(out)
}

/// The root package as the plugin sees it (`$composer->getPackage()`):
/// its name (`__root__` without one) and its pretty version — the alias's
/// when the root is aliased (`RootAliasPackage`) —, with its `extra`.
pub fn root_entry(manifest: &Value, root: &crate::state::RootPackage) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(root.name.clone()));
    m.insert(
        "version".into(),
        Value::String(
            root.aliases
                .first()
                .cloned()
                .unwrap_or_else(|| root.pretty_version.clone()),
        ),
    );
    if let Some(extra) = manifest.get("extra") {
        m.insert("extra".into(), extra.clone());
    }
    m
}

/// `collectPatches($rootPackage, $packages)`: `packages` in the local
/// repository's order.
pub fn collect(
    root: &Value,
    root_source: &Source<'_>,
    packages: &[Source<'_>],
) -> std::result::Result<Collection, String> {
    let mut c = Collection::default();
    let from = |source: &Source<'_>, c: &mut Collection| -> std::result::Result<(), String> {
        if let Some(patches) = isset(source.package.get("extra"), "patches") {
            c.lines.push(format!(
                "Gathering patches from {} (extra.patches).",
                name_of(source.package).to_ascii_lowercase()
            ));
            c.patches.extend(explicit_list(source, patches)?);
        }
        Ok(())
    };
    from(root_source, &mut c)?;
    let allowed = allowed_sources(root);
    for p in packages {
        if allowed.allows(&name_of(p.package).to_ascii_lowercase()) {
            from(p, &mut c)?;
        }
    }
    Ok(c)
}

impl Collection {
    /// `getPatchesFor($package)`, in collection order.
    pub fn for_package(&self, name: &str, version: &str) -> Vec<&Patch> {
        self.patches
            .iter()
            .filter(|p| {
                p.for_name == name
                    && p.for_constraint
                        .as_deref()
                        .is_none_or(|c| crate::constraint::satisfies(version, c).unwrap_or(false))
            })
            .collect()
    }

    /// The names some patch targets (whatever its constraint).
    pub fn targets(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.patches.iter().map(|p| p.for_name.as_str()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The packages whose vendor copy a patch may have changed: every name a
/// key of some `extra.patches` targets (the root's, the lock's packages'),
/// and every installed package that records `patches_applied`. Their
/// classes are scanned from the files, never from the store entry's cache.
pub fn patched_copies<'a>(
    root: &Value,
    packages: impl Iterator<Item = &'a Map<String, Value>>,
    installed: &[Value],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut keys = |extra: Option<&Value>| {
        if let Some(m) = extra
            .and_then(|e| e.get("patches"))
            .and_then(Value::as_object)
        {
            for k in m.keys() {
                out.push(parse_key(k).0);
            }
        }
    };
    keys(root.get("extra"));
    for p in packages {
        keys(p.get("extra"));
    }
    for p in installed.iter().filter_map(Value::as_object) {
        if p.get("extra")
            .and_then(|e| e.get("patches_applied"))
            .is_some_and(|v| !v.is_null())
        {
            out.push(name_of(p).to_owned());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `PatchCollection::getHash()`; `None` when a patch file cannot be read
/// (`sha1_file` fails: Composer's error handler throws).
pub fn hash(patches: &[&Patch]) -> Option<String> {
    let mut hashes = Vec::new();
    for p in patches {
        let bytes = std::fs::read(&p.local_path).ok()?;
        hashes.push(hex::encode(Sha1::digest(&bytes)));
    }
    hashes.sort();
    Some(hex::encode(Sha1::digest(hashes.join(" ").as_bytes())))
}

/// `Plugin::mustUninstallPackage` for one installed (non-alias) package:
/// it has patches applied and patches to apply, and the recorded hash is
/// empty or differs. `None` when a hash cannot be computed.
pub fn must_uninstall(entry: &Map<String, Value>, patches: &[&Patch]) -> Option<bool> {
    let applied = entry.get("extra").and_then(|e| e.get("patches_applied"));
    let has_applied = !crate::effect_plugins::php_empty(applied);
    if !has_applied || patches.is_empty() {
        return Some(false);
    }
    let recorded = applied
        .and_then(|a| a.get("hash"))
        .filter(|h| !crate::effect_plugins::php_empty(Some(h)))
        .and_then(Value::as_str);
    match recorded {
        None => Some(true),
        Some(h) => Some(h != hash(patches)?),
    }
}

/// `extra.patches_applied` for a package whose patches all passed.
pub fn applied_record(patches: &[&Patch]) -> Option<Value> {
    let mut list = Vec::new();
    for p in patches {
        let mut item = Map::new();
        item.insert("from-package".into(), Value::String(p.from_pretty.clone()));
        item.insert("path".into(), Value::String(p.original_path.clone()));
        item.insert("description".into(), Value::String(p.description.clone()));
        list.push(Value::Object(item));
    }
    let mut record = Map::new();
    record.insert("hash".into(), Value::String(hash(patches)?));
    record.insert("list".into(), Value::Array(list));
    Some(Value::Object(record))
}

/// `$extra['patches_applied'] = …; $package->setExtra($extra)`: the key
/// keeps its place when it exists, goes last otherwise.
pub fn set_applied(entry: &mut Map<String, Value>, record: Value) {
    let extra = entry
        .entry("extra")
        .or_insert_with(|| Value::Object(Map::new()));
    if !extra.is_object() {
        *extra = Value::Object(Map::new());
    }
    if let Some(o) = extra.as_object_mut() {
        o.insert("patches_applied".into(), record);
    }
}

/// What one patch did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    AlreadyApplied,
    /// The exception's message (`<error>…</error>`, tags stripped).
    Failed(String),
}

impl Outcome {
    /// The rest of the "Applying patch …" line.
    pub fn text(&self) -> &str {
        match self {
            Outcome::Done => "done.",
            Outcome::AlreadyApplied => "patch was already applied.",
            Outcome::Failed(m) => m,
        }
    }
}

/// `ComposerPatcher\Patcher` with its `PatchPatcher` and the volatile
/// directory, for one run of the listener.
pub struct Patcher {
    /// `PATH` for the commands: the bin directory first, as
    /// `EventDispatcher::ensureBinDirIsInPath` leaves it.
    path_env: Option<std::ffi::OsString>,
    /// `checkCommandExists('patch')`: `None` until tried, then its result.
    command: Option<std::result::Result<(), String>>,
    no_backup: Option<bool>,
    /// `new GitPatcher` → `checkCommandExists('git')`, then its own
    /// `PatchPatcher`'s `checkCommandExists('patch')`.
    git: Option<std::result::Result<(), String>>,
    volatile: Option<tempfile::TempDir>,
    counter: usize,
}

/// `ProcessExecutor::escape` on POSIX.
fn escape(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

impl Patcher {
    pub fn new(path_env: Option<std::ffi::OsString>) -> Self {
        Patcher {
            path_env,
            command: None,
            no_backup: None,
            git: None,
            volatile: None,
            counter: 0,
        }
    }

    /// `sh -c <command>`, stdin closed: `(exit code, stdout, stderr)`.
    fn run(&self, command: &str) -> (i32, String, String) {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdin(std::process::Stdio::null());
        if let Some(path) = &self.path_env {
            cmd.env("PATH", path);
        }
        match cmd.output() {
            Ok(o) => (
                o.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
            ),
            Err(e) => (-1, String::new(), e.to_string()),
        }
    }

    /// `VolatileDirectory::getNewPath($suffix)`.
    fn new_path(&mut self, suffix: &str) -> Result<String> {
        if self.volatile.is_none() {
            let parent = std::env::temp_dir();
            let parent = std::fs::canonicalize(&parent).unwrap_or(parent);
            self.volatile = Some(
                tempfile::Builder::new()
                    .prefix("PCH")
                    .tempdir_in(&parent)
                    .map_err(Error::io(&parent))?,
            );
        }
        let dir = self
            .volatile
            .as_ref()
            .map(|d| d.path().to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let path = format!("{}/{}{}", dir.trim_end_matches('/'), self.counter, suffix);
        self.counter += 1;
        Ok(path)
    }

    fn build(&mut self, file: &Path, base: &Path, level: &str, dry_run: bool) -> Result<String> {
        let no_backup = match self.no_backup {
            Some(b) => b,
            None => {
                let (_, out, err) = self.run("patch --help");
                let b = out.contains("--no-backup-if-mismatch")
                    || err.contains("--no-backup-if-mismatch");
                self.no_backup = Some(b);
                b
            }
        };
        let rej = self.new_path(".rej")?;
        let mut chunks = vec!["patch".to_owned(), escape(level)];
        if no_backup {
            chunks.push("--no-backup-if-mismatch".to_owned());
        }
        chunks.push("-N".to_owned());
        chunks.push("-d".to_owned());
        chunks.push(escape(&base.to_string_lossy()));
        chunks.push("-i".to_owned());
        chunks.push(escape(&file.to_string_lossy()));
        chunks.push("-r".to_owned());
        chunks.push(escape(&rej));
        if dry_run {
            chunks.push("--dry-run".to_owned());
        }
        Ok(chunks.join(" "))
    }

    fn already_applied(&mut self, patch: &Patch, base: &Path, level: &str) -> Result<bool> {
        let command = self.build(&patch.local_path, base, level, true)?;
        let (rc, out, err) = self.run(&command);
        let text = if err.is_empty() { out } else { err };
        Ok(rc == 1 && text.contains("Skipping patch.") && hunks_ignored(&text))
    }

    /// `checkCommandExists($name)`: `CommandNotFound`'s message on failure.
    fn command_exists(&self, name: &str) -> std::result::Result<(), String> {
        let (rc, _, err) = self.run(&format!("{name} --version"));
        if rc == 0 {
            Ok(())
        } else if err.is_empty() {
            Err(format!("Unable to find the command \"{name}\"."))
        } else {
            Err(err)
        }
    }

    /// `Patcher::applyPatch` without its line: the outcome. `GitPatcher`
    /// when the package directory holds a `.git` directory, `PatchPatcher`
    /// otherwise; each built once, its command checked then.
    pub fn apply(&mut self, patch: &Patch, base: &Path) -> Result<Outcome> {
        let git = base.join(".git").is_dir();
        if git {
            if self.git.is_none() {
                let checked = self
                    .command_exists("git")
                    .and_then(|()| self.command_exists("patch"));
                self.git = Some(checked);
            }
            if let Some(Err(e)) = &self.git {
                return Ok(Outcome::Failed(e.clone()));
            }
        } else {
            // `new PatchPatcher` → `checkCommandExists('patch')`.
            if self.command.is_none() {
                self.command = Some(self.command_exists("patch"));
            }
            if let Some(Err(e)) = &self.command {
                return Ok(Outcome::Failed(e.clone()));
            }
        }
        let mut first_error: Option<String> = None;
        for level in &patch.levels {
            if self.already_applied(patch, base, level)? {
                return Ok(Outcome::AlreadyApplied);
            }
            let failure = if git {
                self.git_apply(patch, base, level)
            } else {
                self.patch_apply(patch, base, level)?
            };
            match failure {
                None => return Ok(Outcome::Done),
                Some(reason) => {
                    first_error.get_or_insert(format!(
                        "Unable to apply the patch \"{}\" provided by \"{}\": {reason}",
                        patch.description, patch.from_name
                    ));
                }
            }
        }
        Ok(Outcome::Failed(first_error.unwrap_or_default()))
    }

    /// `PatchPatcher::applyPatchLevel` after the dry run: `None` when
    /// applied, the reason otherwise.
    fn patch_apply(&mut self, patch: &Patch, base: &Path, level: &str) -> Result<Option<String>> {
        let command = self.build(&patch.local_path, base, level, false)?;
        let (rc, out, err) = self.run(&command);
        if rc == 0 {
            return Ok(None);
        }
        let reason = if err.is_empty() { out } else { err };
        Ok(Some(format!(
            "failed to apply the patch with the patch command: {reason}"
        )))
    }

    /// `GitPatcher::applyPatchLevel` after the dry run: `git -C <dir>
    /// apply <level> --check -v <file>`, then without `--check -v`; a
    /// failure (its stderr alone) or a stderr that starts with `Skipped`
    /// ends it.
    fn git_apply(&mut self, patch: &Patch, base: &Path, level: &str) -> Option<String> {
        for testing in [true, false] {
            // `implode(' ', ['git', '-C ', …])`: two spaces after `-C`.
            let mut command = format!(
                "git -C  {} apply {}",
                escape(&base.to_string_lossy()),
                escape(level)
            );
            if testing {
                command.push_str(" --check -v");
            }
            command.push(' ');
            command.push_str(&escape(&patch.local_path.to_string_lossy()));
            let (rc, _, err) = self.run(&command);
            if rc != 0 {
                return Some(format!("failed to apply the patch with GIT: {err}"));
            }
            if err.starts_with("Skipped") {
                return None;
            }
        }
        None
    }
}

/// `preg_match('/^(\d+) out of \1 hunks? ignored/m', …)`.
fn hunks_ignored(text: &str) -> bool {
    text.lines().any(|line| {
        let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return false;
        }
        let rest = &line[digits.len()..];
        let Some(rest) = rest.strip_prefix(" out of ") else {
            return false;
        };
        let Some(rest) = rest.strip_prefix(digits.as_str()) else {
            return false;
        };
        rest.starts_with(" hunks ignored") || rest.starts_with(" hunk ignored")
    })
}

/// Before `patch` writes into a package: a project laid out by a version
/// that linked vendor/ to its store (up to 0.26.0) may still share inodes
/// with it until the one-time pass of `installer::detach_once` has run; the
/// patch must never reach the store entry. Kept as a belt to that pass.
pub use crate::clone::detach;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_and_paths_like_the_plugin() {
        assert_eq!(parse_key("a/b"), ("a/b".into(), None));
        assert_eq!(parse_key("a/b:1.0.0"), ("a/b".into(), Some("1.0.0".into())));
        assert_eq!(parse_key("a/b: ^2 "), ("a/b".into(), Some("^2".into())));
        assert_eq!(parse_key("a/b:*"), ("a/b".into(), None));
        assert_eq!(parse_key("a/b:"), ("a/b:".into(), None));
        assert!(is_remote("https://x/y.patch"));
        assert!(is_remote("file://https://x"));
        assert!(!is_remote("p/x.patch"));
        assert!(!is_remote("c://x"));
        assert_eq!(
            resolve_local("p/../q/x.patch", Path::new("/v/acme/p")),
            PathBuf::from("/v/acme/p/q/x.patch")
        );
        assert_eq!(
            resolve_local("file:///abs/x.patch", Path::new("/v")),
            PathBuf::from("/abs/x.patch")
        );
        assert_eq!(escape("it's"), r"'it'\''s'");
    }

    #[test]
    fn hunks_ignored_regex() {
        assert!(hunks_ignored(
            "x\n1 out of 1 hunk ignored -- saving rejects to file a.rej\n"
        ));
        assert!(hunks_ignored("2 out of 2 hunks ignored--saving rejects"));
        assert!(!hunks_ignored("1 out of 2 hunks ignored"));
        assert!(!hunks_ignored(" 1 out of 1 hunk ignored"));
        assert!(!hunks_ignored("1 out of 1 hunk FAILED"));
    }

    #[test]
    fn collection_and_matching() {
        let root = json!({"name": "acme/app", "extra": {"allow-subpatches": ["acme/patches"]}});
        let provider = json!({"name": "acme/patches", "version": "1.0.0", "extra": {"patches": {
            "acme/lib:1.0.0": {"One": "p/one.patch"},
            "acme/lib2": {"Two": {"path": "p/two.patch", "levels": ["-p0"]}},
            "acme/lib:^2": {"Never": "p/one.patch"}
        }}});
        let other = json!({"name": "acme/other", "version": "1.0.0", "extra": {"patches": {"acme/lib": {"X": "x"}}}});
        let root_map = root.as_object().unwrap();
        let sources = [
            Source {
                package: provider.as_object().unwrap(),
                dir: PathBuf::from("/v/acme/patches"),
            },
            Source {
                package: other.as_object().unwrap(),
                dir: PathBuf::from("/v/acme/other"),
            },
        ];
        let c = collect(
            &root,
            &Source {
                package: root_map,
                dir: PathBuf::from("/p"),
            },
            &sources,
        )
        .unwrap();
        assert_eq!(
            c.lines,
            vec!["Gathering patches from acme/patches (extra.patches)."]
        );
        assert_eq!(c.patches.len(), 3);
        assert_eq!(c.for_package("acme/lib", "1.0.0").len(), 1);
        assert_eq!(c.for_package("acme/lib", "2.1.0").len(), 1);
        assert_eq!(c.for_package("acme/lib", "1.5.0").len(), 0);
        let two = c.for_package("acme/lib2", "3.0.0");
        assert_eq!(two[0].levels, vec!["-p0"]);
        assert_eq!(two[0].from_pretty, "acme/patches 1.0.0");
        assert_eq!(
            two[0].local_path,
            PathBuf::from("/v/acme/patches/p/two.patch")
        );
        assert_eq!(c.targets(), vec!["acme/lib", "acme/lib2"]);
        // Remote and malformed: handed over.
        let remote = json!({"name": "r/r", "version": "1", "extra": {"patches": {"a/b": {"X": "https://e/x.patch"}}}});
        let rs = [Source {
            package: remote.as_object().unwrap(),
            dir: PathBuf::from("/v"),
        }];
        let all = json!({"extra": {"allow-subpatches": true}});
        assert!(collect(
            &all,
            &Source {
                package: root_map,
                dir: PathBuf::from("/p")
            },
            &rs
        )
        .is_err());
        assert!(config_issue(&json!({"extra": {"patches-file": "p.json"}})).is_some());
        assert!(config_issue(&json!({"extra": {"allow-subpatches": "yes"}})).is_some());
        assert!(config_issue(&json!({"extra": {"patch-errors-as-warnings": false}})).is_some());
        assert!(config_issue(&json!({"extra": {"patch-errors-as-warnings": true}})).is_none());
    }

    #[test]
    fn hash_record_and_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.patch"), "a").unwrap();
        std::fs::write(tmp.path().join("b.patch"), "b").unwrap();
        let mk = |f: &str| Patch {
            from_name: "acme/p".into(),
            from_pretty: "acme/p 1.0.0".into(),
            for_name: "acme/lib".into(),
            for_constraint: None,
            original_path: f.into(),
            local_path: tmp.path().join(f),
            description: f.into(),
            levels: vec!["-p1".into()],
        };
        let (a, b) = (mk("a.patch"), mk("b.patch"));
        // sha1("a") = 86f7e437…, sha1("b") = e9d71f5e…: sorted, joined.
        let expected = hex::encode(Sha1::digest(
            "86f7e437faa5a7fce15d1ddcb9eaeaea377667b8 e9d71f5ee7c92d6dc9e92ffdad17b8bd49418f98"
                .as_bytes(),
        ));
        assert_eq!(hash(&[&b, &a]).as_deref(), Some(expected.as_str()));
        let record = applied_record(&[&a]).unwrap();
        assert_eq!(record["list"][0]["from-package"], "acme/p 1.0.0");
        let mut entry = json!({"name": "acme/lib", "extra": {"k": 1}})
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(must_uninstall(&entry, &[&a]), Some(false));
        set_applied(&mut entry, record.clone());
        assert_eq!(
            entry["extra"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["k", "patches_applied"]
        );
        assert_eq!(must_uninstall(&entry, &[&a]), Some(false));
        assert_eq!(must_uninstall(&entry, &[&a, &b]), Some(true));
        assert_eq!(must_uninstall(&entry, &[]), Some(false));
    }
}
