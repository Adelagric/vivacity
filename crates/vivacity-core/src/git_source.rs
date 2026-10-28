//! Packages that only exist as a git repository — a lock entry with a
//! `source` of type `git` and no `dist` at all — installed the way
//! Composer 2.10.3 installs them: a clone left in vendor/ with its `.git`
//! directory (`Composer\Downloader\GitDownloader`, `VcsDownloader`,
//! `Composer\Util\Git`). docs/plans/git-sources.md.
//!
//! Parity of a `.git` directory is only reachable by construction: this
//! module runs the `git` binary with the very commands, arguments, order
//! and environment `GitDownloader` uses, so the repository git writes is
//! the one Composer's run would have written (the stat cache in `index`,
//! the reflog timestamps and the bytes of a repacked pack excepted — they
//! differ between two Composer runs as well).
//!
//! Everything the port does not cover is a named reason ([`plan`]) that
//! hands the run to Composer before anything is written: other VCS, other
//! URL schemes (`http`, `git`, ssh, credentials), a configured cache or
//! GitHub protocol list, network disabled, Windows.
//!
//! A cloned repository is untrusted input: commands are given as argument
//! vectors (never a shell), every URL behind `--` as Composer does, the
//! reference and the branch derived from the version are restricted to a
//! plain character set, and nothing beyond Composer's own commands runs —
//! no submodule recursion, no hook of the repository (git never takes hooks
//! or configuration from a remote).

use crate::lock::LockPackage;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// What `GitDownloader` needs to install one package, decided before any
/// write.
#[derive(Debug, Clone)]
pub struct GitSource {
    pub name: String,
    /// `getSourceUrl()`: the URL as the lock names it.
    pub source_url: String,
    /// The URL `VcsDownloader::prepareUrls` hands to the downloader: the
    /// lock's for a remote repository, `realpath`'d for a local one.
    pub url: String,
    /// `getSourceReference()`.
    pub reference: String,
    /// `getPrettyVersion()`.
    pub pretty_version: String,
    /// `cache-vcs-dir/<sanitized url>/` (trailing slash included, as
    /// Composer builds it) for a remote repository; `None` for a local one,
    /// which `GitDownloader::doDownload` never caches.
    pub cache_path: Option<String>,
    /// `process-timeout` (`ProcessExecutor::setTimeout`); `None` for 0.
    pub timeout: Option<Duration>,
}

impl GitSource {
    /// `Filesystem::isLocalPath($url)` was false: the mirror in the cache
    /// is synced first (`  - Syncing … into cache`), the clone made from it.
    pub fn is_remote(&self) -> bool {
        self.cache_path.is_some()
    }
}

/// Decides whether a package without dist is installed natively from its
/// git source, and how; `Err` is the reason it is not, for the scope report.
pub fn plan(
    p: &LockPackage,
    project_dir: &Path,
    root_manifest: &Value,
) -> std::result::Result<GitSource, String> {
    if !cfg!(unix) {
        return Err("git sources are installed by Composer on Windows".to_owned());
    }
    let source = p
        .raw
        .get("source")
        .filter(|s| !s.is_null())
        .ok_or_else(|| "no dist and no source".to_owned())?;
    let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
    if kind != "git" {
        return Err(format!(
            "source type `{kind}` (only git is installed natively)"
        ));
    }
    let source_url = source
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| "git source without url".to_owned())?;
    let reference = source
        .get("reference")
        .and_then(Value::as_str)
        .unwrap_or("");
    // `VcsDownloader::install`: `!$package->getSourceReference()` throws,
    // and PHP reads "0" as false.
    if !plain_ref(reference) || reference == "0" {
        return Err(format!("source reference `{}`", printable(reference)));
    }
    let pretty_version = p.version();
    let branch = derived_branch(pretty_version);
    if !plain_ref(&branch) {
        return Err(format!(
            "version `{}` (as a branch name)",
            printable(pretty_version)
        ));
    }
    // `getSourceUrls()` adds the mirrors, tried in turn on failure.
    if source
        .get("mirrors")
        .is_some_and(|m| m.as_array().is_some_and(|a| !a.is_empty()) || m.is_object())
    {
        return Err("source mirrors".to_owned());
    }
    if p.raw
        .get("transport-options")
        .and_then(|t| t.get("git"))
        .and_then(|g| g.get("single_use_clone"))
        .is_some_and(php_truthy)
    {
        return Err("transport-options.git.single_use_clone".to_owned());
    }
    match git_version() {
        None => return Err("git was not found in PATH".to_owned()),
        Some(v) if v < (2, 3, 0) => {
            return Err(format!("git {}.{}.{} is older than 2.3.0", v.0, v.1, v.2))
        }
        Some(_) => {}
    }
    let timeout = process_timeout(root_manifest)?;
    // `doInstall` without the cache throws under COMPOSER_DISABLE_NETWORK,
    // and a local repository is never cached: any git source is Composer's
    // then.
    if network_disabled() {
        return Err("COMPOSER_DISABLE_NETWORK is set".to_owned());
    }
    let (url, cache_path) = if is_local_path(source_url) {
        (local_url(source_url, project_dir)?, None)
    } else {
        if !plain_https(source_url) {
            return Err(format!(
                "source url {} (only https:// without credentials, or a local path)",
                sanitize_for_message(source_url)
            ));
        }
        for key in ["github-protocols", "github-domains"] {
            if config_value(root_manifest, key).is_some() {
                return Err(format!("config {key}"));
            }
        }
        let base = cache_vcs_dir(root_manifest)?;
        (
            source_url.to_owned(),
            Some(format!("{base}/{}/", cache_key(source_url))),
        )
    };
    Ok(GitSource {
        name: p.name().to_owned(),
        source_url: source_url.to_owned(),
        url,
        reference: reference.to_owned(),
        pretty_version: pretty_version.to_owned(),
        cache_path,
        timeout,
    })
}

/// The line `GitDownloader::doDownload` writes before syncing the mirror.
pub fn syncing_line(name: &str, full_pretty_version: &str) -> String {
    format!("  - Syncing {name} ({full_pretty_version}) into cache")
}

/// What `GitDownloader::doInstall` appends to `  - Installing x (v): `.
pub fn install_appendix(reference: &str, remote: bool, verbose: bool) -> String {
    let short = short_hash(reference, verbose);
    if remote {
        format!(": Cloning {short} from cache")
    } else {
        format!(": Cloning {short}")
    }
}

/// `GitDownloader::getShortHash`: ten characters of a full sha1 unless
/// verbose; any other reference as is.
pub fn short_hash(reference: &str, verbose: bool) -> String {
    if !verbose && is_sha1(reference) {
        reference[..10].to_owned()
    } else {
        reference.to_owned()
    }
}

/// `Filesystem::isLocalPath` (non-Windows branch):
/// `{^(file://|/|/?[a-z]:[\\/]|\.\.[\\/]|[a-z0-9_.-]+[\\/])}i`.
pub fn is_local_path(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("file://") || lower.starts_with('/') {
        return true;
    }
    let b = lower.as_bytes();
    let drive = |s: &[u8]| {
        s.len() >= 3 && s[0].is_ascii_lowercase() && s[1] == b':' && (s[2] == b'/' || s[2] == b'\\')
    };
    if drive(b) || (b.first() == Some(&b'/') && drive(&b[1..])) {
        return true;
    }
    if lower.starts_with("../") || lower.starts_with("..\\") {
        return true;
    }
    let run = b
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
        .count();
    run > 0 && matches!(b.get(run), Some(b'/') | Some(b'\\'))
}

/// `GitDownloader::doDownload` for a remote repository:
/// `Git::fetchRefOrSyncMirror` into the cache. `Err` when the mirror could
/// not be synced or still lacks the reference — Composer would then clone
/// from the network without the cache, or fail: either way the run is
/// handed to it, before any write to the project.
pub fn sync(src: &GitSource, project_dir: &Path) -> std::result::Result<(), String> {
    let Some(dir) = src.cache_path.as_deref() else {
        return Ok(());
    };
    let git = Git::new(src.timeout, project_dir);
    let dir_path = Path::new(dir);
    if check_ref_in_mirror(&git, dir_path, &src.reference)? {
        if is_sha1(&src.reference) {
            let branch = derived_branch(&src.pretty_version);
            let branches = git.run(&["branch"], dir_path)?;
            let tags = git.run(&["tag"], dir_path)?;
            // `fetchRefOrSyncMirror`: a branch or tag named like the version
            // means the mirror is current enough; otherwise sync again.
            if branches.ok
                && tags.ok
                && !branches
                    .stdout
                    .lines()
                    .any(|l| strip_marks(l) == branch || strip_marks(l) == format!("v{branch}"))
                && !tags.stdout.lines().any(|l| strip_marks(l) == branch)
            {
                // Composer goes on with the mirror it has when this fails,
                // possibly after protocols or credentials vivacity does not
                // try: a failure is handed over rather than guessed.
                if !sync_mirror(&git, &src.url, dir_path)? {
                    return Err(format!(
                        "git mirror sync of {} failed",
                        sanitize_for_message(&src.url)
                    ));
                }
            }
        }
        return Ok(());
    }
    if !sync_mirror(&git, &src.url, dir_path)? {
        return Err(format!(
            "git mirror sync of {} failed",
            sanitize_for_message(&src.url)
        ));
    }
    if check_ref_in_mirror(&git, dir_path, &src.reference)? {
        Ok(())
    } else {
        Err(format!(
            "reference {} of {} is not in its repository",
            src.reference, src.name
        ))
    }
}

/// `GitDownloader::doInstall` then `updateToCommit`, into `path` (a
/// directory that does not exist yet), from the cache for a remote
/// repository ([`sync`] made sure it holds the reference:
/// `cachedPackages`), from the repository itself for a local one. Every
/// command is the one Composer runs, in its order, so the repository
/// written is the one it would write.
pub fn clone_into(
    src: &GitSource,
    path: &Path,
    project_dir: &Path,
) -> std::result::Result<(), String> {
    let git = Git::new(src.timeout, project_dir);
    let path_str = path
        .to_str()
        .ok_or_else(|| format!("non UTF-8 install path {}", path.display()))?;
    let sanitized = strip_credentials(&src.url);
    // The first command runs from Composer's working directory
    // (`runCommands(..., initialClone: true)`), the rest inside the clone.
    let (first, rest): (Vec<&str>, Vec<Vec<&str>>) = match src.cache_path.as_deref() {
        Some(cache) => (
            vec![
                "clone",
                "--no-checkout",
                cache,
                path_str,
                "--dissociate",
                "--reference",
                cache,
            ],
            vec![
                vec!["remote", "set-url", "origin", "--", &sanitized],
                vec!["remote", "add", "composer", "--", &sanitized],
            ],
        ),
        None => (
            vec!["clone", "--no-checkout", "--", &src.url, path_str],
            vec![
                vec!["remote", "add", "composer", "--", &src.url],
                vec!["fetch", "composer"],
                vec!["remote", "set-url", "origin", "--", &sanitized],
                vec!["remote", "set-url", "composer", "--", &sanitized],
            ],
        ),
    };
    git.run(&first, project_dir)?.require(&first)?;
    for cmd in &rest {
        git.run(cmd, path)?.require(cmd)?;
    }
    if src.url != src.source_url {
        // `updateOriginUrl`: its own commands are not checked by Composer.
        git.run(
            &["remote", "set-url", "origin", "--", &src.source_url],
            path,
        )?;
        set_push_url(&git, path, &src.source_url)?;
    } else {
        set_push_url(&git, path, &src.url)?;
    }
    update_to_commit(&git, path, &src.reference, &src.pretty_version)
}

/// `GitDownloader::setPushUrl` with the default `github-domains` and
/// `github-protocols` ([`plan`] hands any other configuration over).
fn set_push_url(git: &Git, path: &Path, url: &str) -> std::result::Result<(), String> {
    let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))
    else {
        return Ok(());
    };
    // `/([^/]+)/([^/]+?)(?:\.git)?$`: owner and repository, nothing after.
    let Some((owner, repo)) = rest.split_once('/') else {
        return Ok(());
    };
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return Ok(());
    }
    let repo = match repo.strip_suffix(".git") {
        Some(r) if !r.is_empty() => r,
        _ => repo,
    };
    // `github-protocols` contains `ssh` by default.
    let push_url = format!("git@github.com:{owner}/{repo}.git");
    git.run(
        &["remote", "set-url", "--push", "origin", "--", &push_url],
        path,
    )?;
    Ok(())
}

/// `GitDownloader::updateToCommit` (2.10.3: never returns a new reference).
fn update_to_commit(
    git: &Git,
    path: &Path,
    reference: &str,
    pretty_version: &str,
) -> std::result::Result<(), String> {
    let mut branch = derived_branch(pretty_version);
    let branches = git.run(&["branch", "-r"], path)?;
    let branches = branches.ok.then_some(branches.stdout);
    let remote_branch = |name: &str| {
        branches.as_deref().is_some_and(|b| {
            b.lines().any(|l| {
                l.starts_with(char::is_whitespace) && l.trim_start() == format!("composer/{name}")
            })
        })
    };
    let ok = |args: &[&str]| -> std::result::Result<bool, String> { Ok(git.run(args, path)?.ok) };
    if !is_sha1(reference) && remote_branch(reference) {
        let target = format!("composer/{reference}");
        if ok(&["checkout", "-B", &branch, &target, "--"])?
            && ok(&["reset", "--hard", &target, "--"])?
        {
            return Ok(());
        }
    }
    if is_sha1(reference) {
        if branches.is_some() && !remote_branch(&branch) && remote_branch(&format!("v{branch}")) {
            branch = format!("v{branch}");
        }
        let fallback_target = format!("composer/{branch}");
        if (ok(&["checkout", &branch, "--"])?
            || ok(&["checkout", "-B", &branch, &fallback_target, "--"])?)
            && ok(&["reset", "--hard", reference, "--"])?
        {
            return Ok(());
        }
    }
    if ok(&["checkout", reference, "--"])? && ok(&["reset", "--hard", reference, "--"])? {
        return Ok(());
    }
    Err(format!(
        "git checkout {reference} failed for {}",
        path.display()
    ))
}

/// `Git::checkRefIsInMirror`.
fn check_ref_in_mirror(
    git: &Git,
    dir: &Path,
    reference: &str,
) -> std::result::Result<bool, String> {
    if !is_mirror(git, dir)? {
        return Ok(false);
    }
    let rev = format!("{reference}^{{commit}}");
    Ok(git
        .run(&["rev-parse", "--quiet", "--verify", &rev], dir)?
        .ok)
}

/// `is_dir($dir) && git rev-parse --git-dir` answers `.` from inside it.
fn is_mirror(git: &Git, dir: &Path) -> std::result::Result<bool, String> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let out = git.run(&["rev-parse", "--git-dir"], dir)?;
    if out.stderr.contains("detected dubious ownership") {
        return Err(format!("{} is owned by another user", dir.display()));
    }
    Ok(out.ok && out.stdout.trim() == ".")
}

/// `Git::syncMirror` (network allowed: [`plan`] refuses otherwise). `Ok(false)`
/// where Composer catches the failure of an update, `Err` where it throws.
fn sync_mirror(git: &Git, url: &str, dir: &Path) -> std::result::Result<bool, String> {
    let sanitized = strip_credentials(url);
    let dir_str = dir
        .to_str()
        .ok_or_else(|| format!("non UTF-8 cache path {}", dir.display()))?;
    if is_mirror(git, dir)? {
        let mut ok = true;
        for cmd in [
            vec!["remote", "set-url", "origin", "--", url],
            vec!["remote", "update", "--prune", "origin"],
            vec!["gc", "--auto"],
        ] {
            if !git.run(&cmd, dir)?.ok {
                ok = false;
                break;
            }
        }
        // `finally`: the credentials-free URL is put back either way.
        let restored = git.run(&["remote", "set-url", "origin", "--", &sanitized], dir)?;
        return Ok(ok && restored.ok);
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("cannot remove {dir_str}: {e}"))?;
    }
    let clone = ["clone", "--mirror", "--", url, dir_str];
    git.run(&clone, &git.cwd)?.require(&clone)?;
    let restore = ["remote", "set-url", "origin", "--", &sanitized];
    git.run(&restore, dir)?.require(&restore)?;
    Ok(true)
}

struct GitOutput {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl GitOutput {
    fn require(self, args: &[&str]) -> std::result::Result<GitOutput, String> {
        if self.ok {
            Ok(self)
        } else {
            Err(format!(
                "git {} failed: {}",
                args.join(" "),
                self.stderr.trim()
            ))
        }
    }
}

/// `ProcessExecutor` for git: `Git::cleanEnv`'s environment, the
/// `process-timeout`, output captured (Composer prints none of it).
struct Git {
    timeout: Option<Duration>,
    /// Composer's working directory: where a command without `cwd` runs.
    cwd: PathBuf,
}

impl Git {
    fn new(timeout: Option<Duration>, cwd: &Path) -> Git {
        Git {
            timeout,
            cwd: cwd.to_path_buf(),
        }
    }

    fn run(&self, args: &[&str], cwd: &Path) -> std::result::Result<GitOutput, String> {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        clean_env(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot run git {}: {e}", args.join(" ")))?;
        let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_end(&mut buf);
                }
                String::from_utf8_lossy(&buf).into_owned()
            })
        };
        let out = read(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        );
        let err = read(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        );
        let deadline = self.timeout.map(|t| Instant::now() + t);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => return Err(format!("git {}: {e}", args.join(" "))),
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "git {} exceeded the process timeout",
                    args.join(" ")
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        Ok(GitOutput {
            ok: status.success(),
            stdout: out.join().unwrap_or_default(),
            stderr: err.join().unwrap_or_default(),
        })
    }
}

/// `Git::cleanEnv` for git ≥ 2.3 (the only versions [`plan`] accepts).
fn clean_env(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("LANGUAGE", "C")
        .env_remove("DYLD_LIBRARY_PATH");
}

/// `Git::getVersion`, once per process.
fn git_version() -> Option<(u32, u32, u32)> {
    static VERSION: OnceLock<Option<(u32, u32, u32)>> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let out = Command::new("git")
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_git_version(&String::from_utf8_lossy(&out.stdout))
    })
}

/// `/^git version (\d+(?:\.\d+)+)/m`.
fn parse_git_version(text: &str) -> Option<(u32, u32, u32)> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("git version ") else {
            continue;
        };
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = digits.split('.').filter(|s| !s.is_empty());
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        return Some((major, minor, patch));
    }
    None
}

/// The branch `updateToCommit` and `fetchRefOrSyncMirror` derive from the
/// pretty version: `preg_replace('{(?:^dev-|(?:\.x)?-dev$)}i', '', …)`.
pub fn derived_branch(pretty_version: &str) -> String {
    let mut s = pretty_version;
    if s.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("dev-")) {
        s = &s[4..];
    }
    // ASCII suffixes: the cut points are character boundaries.
    let lower = s.to_ascii_lowercase();
    if lower.ends_with(".x-dev") {
        s = &s[..s.len() - 6];
    } else if lower.ends_with("-dev") {
        s = &s[..s.len() - 4];
    }
    s.to_owned()
}

/// A reference or branch name passed to git as an argument: non-empty,
/// `[A-Za-z0-9._/-]`, not an option.
fn plain_ref(r: &str) -> bool {
    !r.is_empty()
        && !r.starts_with('-')
        && r.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b'-'))
}

/// `^[a-f0-9]{40}$`.
fn is_sha1(r: &str) -> bool {
    r.len() == 40 && r.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// `^[\s*]*` stripped: a `git branch` / `git tag` line's marks.
fn strip_marks(line: &str) -> &str {
    line.trim_start_matches(|c: char| c.is_whitespace() || c == '*')
}

/// An `https://` URL without user information, query, fragment or
/// whitespace: `Url::sanitize` and `Url::stripCredentials` leave it as is.
fn plain_https(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    !authority.is_empty()
        && !authority.contains('@')
        && !url.contains(['?', '#', '%'])
        && !url.chars().any(char::is_whitespace)
}

/// `Url::stripCredentials`: `{://[^/\s?#]+@}` → `://`.
fn strip_credentials(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let authority_end = rest
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(rest.len());
        if let Some(at) = rest[..authority_end].rfind('@') {
            return format!("{scheme}://{}", &rest[at + 1..]);
        }
    }
    url.to_owned()
}

/// For a message: `Url::sanitize`'s masking (no user information, an
/// `access_token` query value hidden), control characters made visible.
fn sanitize_for_message(url: &str) -> String {
    let mut out = strip_credentials(url);
    for marker in ["?access_token=", "&access_token="] {
        if let Some(start) = out.find(marker) {
            let value = start + marker.len();
            let end = out[value..].find('&').map_or(out.len(), |i| value + i);
            out.replace_range(value..end, "***");
        }
    }
    printable(&out)
}

/// A lock value in a message: control characters replaced.
fn printable(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// The cache directory name: `preg_replace('{[^a-z0-9.]}i', '-', $url)`
/// (bytewise, as PHP's non-UTF-8 pattern).
fn cache_key(url: &str) -> String {
    url.bytes()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == b'.' {
                c as char
            } else {
                '-'
            }
        })
        .collect()
}

/// `VcsDownloader::prepareUrls` for a local URL: `file://` set aside,
/// `realpath` from Composer's working directory, `file://` put back.
fn local_url(url: &str, project_dir: &Path) -> std::result::Result<String, String> {
    if url.contains('%') {
        return Err(format!(
            "local source url {} (percent-encoded)",
            printable(url)
        ));
    }
    let (prefix, path) = match url.strip_prefix("file://") {
        Some(p) => ("file://", p),
        None => ("", url),
    };
    let resolved = std::fs::canonicalize(project_dir.join(path))
        .map_err(|_| format!("local git repository {} does not exist", printable(url)))?;
    let resolved = resolved
        .to_str()
        .ok_or_else(|| format!("local git repository {url} (non UTF-8 path)"))?;
    Ok(format!("{prefix}{resolved}"))
}

/// The project's `config` key, else the global one (`Config::merge` order).
fn config_value(root_manifest: &Value, key: &str) -> Option<Value> {
    root_manifest
        .get("config")
        .and_then(|c| c.get(key))
        .cloned()
        .or_else(|| crate::layout::global_config_value(key))
}

/// PHP truthiness of a configuration value.
fn php_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty() && s != "0",
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `COMPOSER_DISABLE_NETWORK` (`Platform::getEnv`, truthy).
fn network_disabled() -> bool {
    std::env::var("COMPOSER_DISABLE_NETWORK").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// `Config::get('process-timeout')`: `COMPOSER_PROCESS_TIMEOUT`, else the
/// configuration, else 300; 0 is no timeout. A value `(int)` would read
/// differently from a plain integer is handed over.
fn process_timeout(root_manifest: &Value) -> std::result::Result<Option<Duration>, String> {
    let seconds: u64 = if let Ok(v) = std::env::var("COMPOSER_PROCESS_TIMEOUT") {
        v.parse()
            .map_err(|_| format!("COMPOSER_PROCESS_TIMEOUT={v}"))?
    } else {
        match config_value(root_manifest, "process-timeout") {
            None => 300,
            Some(v) => v
                .as_u64()
                .ok_or_else(|| format!("config process-timeout {v}"))?,
        }
    };
    Ok((seconds > 0).then(|| Duration::from_secs(seconds)))
}

/// `Config::get('cache-vcs-dir')` for the cases vivacity reads:
/// `{$cache-dir}/vcs` with the cache directory `Factory::getCacheDir`
/// computes. Any configured `cache-dir` / `cache-vcs-dir`,
/// `COMPOSER_CACHE_VCS_DIR`, or a path `expandPath`/`realpath` would
/// rewrite, is handed over: the path ends up in the clone's reflog.
fn cache_vcs_dir(root_manifest: &Value) -> std::result::Result<String, String> {
    if std::env::var_os("COMPOSER_CACHE_VCS_DIR").is_some() {
        return Err("COMPOSER_CACHE_VCS_DIR is set".to_owned());
    }
    for key in ["cache-dir", "cache-vcs-dir"] {
        if config_value(root_manifest, key).is_some() {
            return Err(format!("config {key}"));
        }
    }
    // Set but empty, `Config::get('cache-dir')` takes "" (the project
    // directory) where `Factory::getCacheDir` moves on: handed over.
    for key in ["COMPOSER_CACHE_DIR", "COMPOSER_HOME"] {
        if std::env::var_os(key).is_some_and(|v| v.is_empty()) {
            return Err(format!("{key} is set and empty"));
        }
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let base = if let Some(dir) = env("COMPOSER_CACHE_DIR") {
        if dir == "0" {
            return Err("COMPOSER_CACHE_DIR=0".to_owned());
        }
        dir
    } else if let Some(home) = env("COMPOSER_HOME") {
        if home == "0" {
            return Err("COMPOSER_HOME=0".to_owned());
        }
        format!("{home}/cache")
    } else {
        if env("XDG_CACHE_HOME").is_some_and(|x| x.ends_with('/')) {
            return Err("XDG_CACHE_HOME ends with a slash".to_owned());
        }
        crate::fetch::composer_cache_dir()
            .to_str()
            .ok_or_else(|| "non UTF-8 cache directory".to_owned())?
            .to_owned()
    };
    let base = base.trim_end_matches('/');
    if !base.starts_with('/') {
        return Err(format!("cache directory {base} is not absolute"));
    }
    let vcs = format!("{base}/vcs");
    // `Cache::isUsable`.
    if vcs
        .split('/')
        .any(|seg| matches!(seg, "$null" | "nul" | "NUL"))
        || vcs.starts_with("/dev/null")
    {
        return Err(format!("cache directory {vcs} is not usable"));
    }
    Ok(vcs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_branch_follows_the_pattern() {
        assert_eq!(derived_branch("dev-master"), "master");
        assert_eq!(derived_branch("DEV-Main"), "Main");
        assert_eq!(derived_branch("1.x-dev"), "1");
        assert_eq!(derived_branch("2.0-dev"), "2.0");
        assert_eq!(derived_branch("dev-1.x-dev"), "1");
        assert_eq!(derived_branch("dev-dev"), "dev");
        assert_eq!(derived_branch("1.1.5"), "1.1.5");
        // `dev---detach` would become an option of `git checkout`.
        assert!(!plain_ref(&derived_branch("dev---detach")));
    }

    #[test]
    fn local_paths_as_composer_sees_them() {
        for local in [
            "/abs/repo.git",
            "file:///abs",
            "../up",
            "repos/lib.git",
            "c:/x",
            "/c:\\x",
        ] {
            assert!(is_local_path(local), "{local}");
        }
        for remote in [
            "https://github.com/a/b",
            "git@github.com:a/b.git",
            "ssh://h/x",
            "lib.git",
        ] {
            assert!(!is_local_path(remote), "{remote}");
        }
    }

    #[test]
    fn short_hash_and_appendix() {
        let sha = "7aefa0a840ed9bf7e541b5ce2b88fe41ea55de5f";
        assert_eq!(short_hash(sha, false), "7aefa0a840");
        assert_eq!(short_hash(sha, true), sha);
        assert_eq!(short_hash("master", false), "master");
        assert_eq!(
            install_appendix(sha, true, false),
            ": Cloning 7aefa0a840 from cache"
        );
        assert_eq!(install_appendix("master", false, false), ": Cloning master");
    }

    #[test]
    fn cache_key_matches_composer() {
        assert_eq!(
            cache_key("https://git.example.invalid/acme/lib.git"),
            "https---git.example.invalid-acme-lib.git"
        );
        assert_eq!(
            cache_key("https://github.com/Kovah/netscape-bookmark-parser"),
            "https---github.com-Kovah-netscape-bookmark-parser"
        );
    }

    #[test]
    fn urls_handed_over() {
        assert!(plain_https("https://github.com/a/b.git"));
        assert!(!plain_https("https://u:p@github.com/a/b.git"));
        assert!(!plain_https("http://github.com/a/b.git"));
        assert!(!plain_https("https://h/x?access_token=1"));
        assert_eq!(strip_credentials("https://u:p@h/x"), "https://h/x");
        assert_eq!(strip_credentials("https://h/x"), "https://h/x");
    }

    #[test]
    fn plain_refs_only() {
        assert!(plain_ref("master"));
        assert!(plain_ref("7aefa0a840ed9bf7e541b5ce2b88fe41ea55de5f"));
        assert!(plain_ref("feature/x-1.2_3"));
        assert!(!plain_ref(""));
        assert!(!plain_ref("-x"));
        assert!(!plain_ref("a b"));
        assert!(!plain_ref("a;b"));
    }

    #[test]
    fn git_version_parse() {
        assert_eq!(parse_git_version("git version 2.54.0\n"), Some((2, 54, 0)));
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39, 5))
        );
        assert_eq!(parse_git_version("git version 2.3"), Some((2, 3, 0)));
        assert_eq!(parse_git_version("nope"), None);
    }

    #[test]
    fn plan_refuses_what_is_not_ported() {
        let manifest = serde_json::json!({});
        let pkg = |source: Value| {
            let mut raw = serde_json::Map::new();
            raw.insert("name".into(), "a/b".into());
            raw.insert("version".into(), "1.0.0".into());
            raw.insert("source".into(), source);
            LockPackage { raw }
        };
        let dir = std::path::PathBuf::from("/nonexistent-vivacity-git");
        let hg = pkg(serde_json::json!({"type": "hg", "url": "https://h/x", "reference": "abc"}));
        assert!(plan(&hg, &dir, &manifest)
            .unwrap_err()
            .contains("source type `hg`"));
        let ssh = pkg(
            serde_json::json!({"type": "git", "url": "git@github.com:a/b.git", "reference": "abc"}),
        );
        let err = plan(&ssh, &dir, &manifest).unwrap_err();
        assert!(
            err.contains("only https://") || err.contains("git was not found"),
            "{err}"
        );
        let opt =
            pkg(serde_json::json!({"type": "git", "url": "https://h/x", "reference": "--orphan"}));
        assert!(plan(&opt, &dir, &manifest)
            .unwrap_err()
            .contains("source reference"));
    }
}
