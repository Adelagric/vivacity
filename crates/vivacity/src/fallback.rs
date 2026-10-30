//! Handing a command to the real Composer: before any write, with the same
//! arguments, or stopping with exit 3 under `--no-fallback`.

use super::*;

/// The resolution commands' fallback (plan v0.16, decision 1), symmetric
/// to `install`'s: an installed, allowed plugin that changes what the
/// command resolves or writes, and that vivacity does not emulate for it,
/// hands the whole command to Composer with the same arguments — before
/// any write (the caller reverts its manifest edit first). `--no-fallback`
/// stops with exit 3 instead. `Some(code)` when the command was handled
/// here; `None` when vivacity goes on natively. `post_update_cmd`: the
/// command dispatches `POST_UPDATE_CMD` (not a dry run) — without an
/// install, to the plugins loaded from installed.json
/// (docs/plans/no-install-listeners.md).
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolution_fallback(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    command: vivacity_core::scope::ResolutionCommand,
    no_plugins: bool,
    no_fallback: bool,
    with_install: bool,
    post_update_cmd: bool,
    with_dev: bool,
    start: &PoolStart<'_>,
) -> anyhow::Result<Option<i32>> {
    let mut issues = vivacity_core::scope::resolution_issues(
        project,
        manifest,
        command,
        !no_plugins,
        with_install,
        post_update_cmd,
    );
    // laminas-skeleton-installer, configured, rewrites composer.json at
    // `POST_UPDATE_CMD`; `require` and `remove` read the file again for the
    // lock's hash afterwards (docs/plans/plugins-round2.md). Decided now,
    // before the command edits composer.json.
    if !no_plugins
        && matches!(
            command,
            vivacity_core::scope::ResolutionCommand::Require
                | vivacity_core::scope::ResolutionCommand::Remove
        )
        && vivacity_core::scope::active_plugins(project, manifest, true)
            .iter()
            .any(|p| p == vivacity_core::effect_plugins::LAMINAS_SKELETON)
        && vivacity_core::effect_plugins::laminas_skeleton_configured(manifest) == Ok(true)
    {
        issues.push(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
            vivacity_core::effect_plugins::LAMINAS_SKELETON.to_owned(),
            "rewrites composer.json, which the command reads again for the lock's hash (not emulated)".to_owned(),
        ));
    }
    // Without an install, `POST_UPDATE_CMD` still runs the loaded plugins'
    // listeners on the local repository: what vivacity cannot reproduce
    // there goes to Composer now, before anything is written.
    if !with_install && post_update_cmd && !no_plugins {
        issues.extend(crate::no_install::issues_before_resolution(
            project, manifest, with_dev,
        ));
    }
    // Flex emulated for this command: its file-writing behaviours are
    // still Composer's.
    let flex_active = vivacity_core::scope::active_plugins(project, manifest, !no_plugins)
        .iter()
        .any(|p| p == "symfony/flex");
    if flex_active && !issues.iter().any(|i| matches!(i, vivacity_core::scope::ScopeIssue::ResolutionPlugin(n, _) if n == "symfony/flex")) {
        if let Some(reason) = flex_write_guard(project, manifest) {
            issues.push(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
                "symfony/flex".to_owned(),
                reason,
            ));
        }
    }
    // The merge plugin emulated: a merged file's `repositories`
    // (`prependRepositories`) is not. Dev mode does not matter here — the
    // files are merged whatever the mode, only their `require-dev` waits.
    let merge_active = vivacity_core::scope::active_plugins(project, manifest, !no_plugins)
        .iter()
        .any(|p| p == vivacity_resolver::merge_plugin::PLUGIN_NAME);
    if merge_active {
        if let Ok(Some(merged)) = merge_root_when(project, manifest, true, true) {
            if !merged.repositories.is_empty() {
                issues.push(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
                    vivacity_resolver::merge_plugin::PLUGIN_NAME.to_owned(),
                    format!(
                        "merges repositories from {} (not emulated at resolution)",
                        merged.repositories.join(", ")
                    ),
                ));
            }
        }
    }
    // `secure-http` on the `composer` repositories the resolution reads —
    // the project's and the global config.json's, merged as `Config::merge`
    // merges them: decided here, before `require` or `remove` edit
    // composer.json and before anything is printed.
    issues.extend(insecure_repository_issue(
        project,
        manifest,
        with_install,
        with_dev,
        !no_plugins,
        start,
    ));
    if issues.is_empty() {
        return Ok(None);
    }
    delegate_resolution(&issues, no_fallback).map(Some)
}

/// What a resolution's pool starts from, as the command knows it before
/// anything is printed: `update <packages>` (the allow list and its
/// transitive mode), `require` (the requirements it adds, which a lock
/// makes the allow list), `remove` (the packages it drops). Empty: a full
/// update.
#[derive(Default)]
pub(crate) struct PoolStart<'a> {
    pub allow: Vec<String>,
    pub mode: Option<vivacity_resolver::pool::UpdateMode>,
    pub added: Vec<(String, String)>,
    pub removed: Vec<String>,
    /// The lock, when the command has it in memory already (`remove`).
    pub lock: Option<&'a serde_json::Value>,
}

/// The first `composer` repository of the merged configuration (the global
/// config.json, then composer.json, `MergedConfig::load`) that `secure-http`
/// does not let through as is — refused, or judged on a setting vivacity
/// hands over — as a hand-over reason: its packages.json URL for a remote
/// one; for the local ones (`file://`), what their files name, read as the
/// pool would read them ([`vivacity_resolver::repository::local_insecure_url`]),
/// without a request. Over the common path (every repository `https`): a
/// string test each, on files the run has read already.
fn insecure_repository_issue(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    with_install: bool,
    with_dev: bool,
    plugins_enabled: bool,
    start: &PoolStart<'_>,
) -> Option<vivacity_core::scope::ScopeIssue> {
    use serde_json::Value;
    use vivacity_core::secure_http::{classify, insecure, Verdict};
    let home = vivacity_core::fetch::composer_home();
    // An unreadable global config.json is the session's to report.
    let config = vivacity_resolver::session::MergedConfig::load(manifest, home.as_deref()).ok()?;
    let why = |v: Verdict| match v {
        Verdict::Refuse => Some("which secure-http refuses".to_owned()),
        Verdict::HandOver(reason) => Some(format!("with {reason}")),
        Verdict::Allowed | Verdict::Warn { .. } => None,
    };
    let mut local: Vec<String> = Vec::new();
    for repo in &config.repositories {
        let def = &repo.definition;
        if def.get("type").and_then(Value::as_str) != Some("composer") {
            continue;
        }
        // `allow_ssl_downgrade` (=== true): without a `metadata-url`,
        // Composer reads the rest of the repository over `http` once its
        // packages.json is in (ComposerRepository.php:190, :1552-1555) —
        // refused, or warned about; not reproduced.
        if def.get("allow_ssl_downgrade") == Some(&Value::Bool(true)) {
            return Some(vivacity_core::scope::ScopeIssue::Layout(format!(
                "repository {} sets allow_ssl_downgrade (not reproduced)",
                def.get("url").and_then(Value::as_str).unwrap_or("?")
            )));
        }
        let Some(url) = def.get("url").and_then(Value::as_str) else {
            continue;
        };
        // The common path: an `https` URL is not a path either.
        if url.starts_with("https://") {
            continue;
        }
        let urls = vivacity_resolver::repository::repo_urls(url);
        if insecure(&urls.packages_json_url).is_some() {
            if let Some(why) = why(classify(&urls.packages_json_url)) {
                return Some(vivacity_core::scope::ScopeIssue::Layout(format!(
                    "repository {} is read over an insecure protocol, {why}",
                    vivacity_resolver::metacache::sanitize_url(&urls.url)
                )));
            }
            continue;
        }
        if urls.packages_json_url.starts_with("file://") {
            local.push(url.to_owned());
        }
    }
    // A partial update fixes the locked packages outside its allow list:
    // one not installed as locked is downloaded from its locked dist,
    // which the lock holds — judged now (the lock in memory, or read for a
    // local repository's check anyway).
    let partial = !start.allow.is_empty() || !start.added.is_empty();
    let read_lock;
    let lock = match start.lock {
        Some(l) => Some(l),
        None if partial && with_install && !local.is_empty() => {
            read_lock = vivacity_core::jsonfile::read(&project.join("composer.lock"));
            read_lock.as_deref()
        }
        None => None,
    };
    let fixed = lock
        .filter(|_| partial)
        .map(|l| fixed_packages(manifest, with_dev, start, l));
    if let (true, Some(lock), Some(fixed)) = (with_install, lock, &fixed) {
        let sections = if with_dev {
            &["packages", "packages-dev"][..]
        } else {
            &["packages"][..]
        };
        let entries: Vec<Value> = sections
            .iter()
            .filter_map(|k| lock.get(*k).and_then(Value::as_array))
            .flatten()
            .filter(|p| {
                p.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| fixed.names.contains(&n.to_lowercase()))
            })
            .cloned()
            .collect();
        if crate::install::any_insecure_dist(&serde_json::json!({ "packages": entries })) {
            let subset = serde_json::json!({"packages": entries, "packages-dev": []});
            let parsed = vivacity_core::lock::Lock::from_value(&subset);
            if let Some(reason) = crate::install::insecure_dist_reason(
                project,
                manifest,
                &parsed,
                true,
                plugins_enabled,
            ) {
                return Some(vivacity_core::scope::ScopeIssue::Layout(reason));
            }
        }
    }
    if local.is_empty() {
        return None;
    }
    // `policy.ignore-unreachable` off for the resolution's filters: their
    // refusal stops Composer (a policy vivacity cannot read: counted).
    let filters_fatal = vivacity_resolver::policy_config::PolicyConfig::from_raw(&config.policy)
        .map_or(true, |p| {
            !(p.ignore_unreachable.update && p.ignore_unreachable.install)
        });
    let scope = pool_scope(
        manifest,
        with_install,
        with_dev,
        start,
        fixed,
        filters_fatal,
    );
    let (found, verdict) = vivacity_resolver::repository::local_insecure_url(&local, &scope)?;
    why(verdict).map(|why| {
        vivacity_core::scope::ScopeIssue::Layout(format!(
            "a local repository names {} (an insecure protocol), {why}",
            vivacity_resolver::metacache::sanitize_url(&found)
        ))
    })
}

/// The root's requirements as `PoolBuilder` starts from them (`require`,
/// `require-dev` in dev mode, `require`'s additions, not `remove`'s drops),
/// name and constraint.
fn root_requirements(
    manifest: &serde_json::Value,
    with_dev: bool,
    start: &PoolStart<'_>,
) -> Vec<(String, String)> {
    use serde_json::Value;
    let removed: Vec<String> = start.removed.iter().map(|n| n.to_lowercase()).collect();
    let sections: &[&str] = if with_dev {
        &["require", "require-dev"]
    } else {
        &["require"]
    };
    let mut root: Vec<(String, String)> = sections
        .iter()
        .filter_map(|k| manifest.get(*k).and_then(Value::as_object))
        .flatten()
        .filter_map(|(n, c)| Some((n.to_lowercase(), c.as_str()?.to_owned())))
        .filter(|(n, _)| !removed.contains(n))
        .collect();
    for (name, constraint) in &start.added {
        let name = name.to_lowercase();
        root.retain(|(n, _)| *n != name);
        root.push((name, constraint.clone()));
    }
    root
}

/// The locked packages a partial update fixes (PoolBuilder.php:215-245):
/// those outside the allow list — widened along the lock's `require`
/// links for the transitive modes, root requirements left out for `-w`
/// (Composer widens further as it loads new versions: the local check
/// unlocks those too) — and the names they replace; and the requirements
/// the fixed packages mark for loading.
struct Fixed {
    names: std::collections::BTreeSet<String>,
    requires: Vec<(String, String)>,
}

fn fixed_packages(
    manifest: &serde_json::Value,
    with_dev: bool,
    start: &PoolStart<'_>,
    lock: &serde_json::Value,
) -> Fixed {
    use serde_json::Value;
    use vivacity_resolver::pool::UpdateMode;
    let root = root_requirements(manifest, with_dev, start);
    let mut allow: Vec<String> = start.allow.iter().map(|n| n.to_lowercase()).collect();
    allow.extend(start.added.iter().map(|(n, _)| n.to_lowercase()));
    let locked: Vec<&Value> = ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| lock.get(*k).and_then(Value::as_array))
        .flatten()
        .collect();
    let name_of = |p: &Value| {
        p.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
    };
    let patterns: Vec<_> = allow
        .iter()
        .map(|a| vivacity_resolver::pool::package_name_regexp(a))
        .collect();
    let mut allowed: std::collections::BTreeSet<String> = locked
        .iter()
        .map(|p| name_of(p))
        .filter(|n| {
            patterns
                .iter()
                .any(|re| re.is_match(n.as_bytes()).unwrap_or(false))
        })
        .collect();
    if let Some(mode) = start.mode.filter(|m| *m != UpdateMode::OnlyListed) {
        let root_names: Vec<&String> = root.iter().map(|(n, _)| n).collect();
        let mut pending: Vec<String> = allowed.iter().cloned().collect();
        while let Some(name) = pending.pop() {
            let Some(p) = locked.iter().find(|p| name_of(p) == name) else {
                continue;
            };
            for dep in p
                .get("require")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(d, _)| d.to_lowercase())
            {
                if mode == UpdateMode::ListedWithTransitiveDepsNoRootRequire
                    && root_names.contains(&&dep)
                {
                    continue;
                }
                if allowed.insert(dep.clone()) {
                    pending.push(dep);
                }
            }
        }
    }
    let mut names = std::collections::BTreeSet::new();
    let mut requires = Vec::new();
    for p in &locked {
        let name = name_of(p);
        if allowed.contains(&name) {
            continue;
        }
        names.insert(name);
        for (target, _) in p
            .get("replace")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            names.insert(target.to_lowercase());
        }
        for (dep, c) in p
            .get("require")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let Some(c) = c.as_str() {
                requires.push((dep.to_lowercase(), c.to_owned()));
            }
        }
    }
    Fixed { names, requires }
}

/// `PoolBuilder::buildPool`'s start (PoolBuilder.php:215-290) for the local
/// check: the root's requirements and the fixed packages' requirements,
/// minus what is fixed; the stabilities and aliases the root sets.
fn pool_scope(
    manifest: &serde_json::Value,
    with_install: bool,
    with_dev: bool,
    start: &PoolStart<'_>,
    fixed: Option<Fixed>,
    filters_fatal: bool,
) -> vivacity_resolver::repository::LocalScope {
    use serde_json::Value;
    use vivacity_resolver::pool::UpdateMode;
    let root = root_requirements(manifest, with_dev, start);
    // `extractStabilityFlags`: an explicit `@stability`, or a constraint
    // that names an unstable version (`dev-main`, `1.0.x-dev`, the left
    // side of an alias).
    let flagged = root
        .iter()
        .filter(|(_, c)| {
            c.contains('@')
                || c.split("||").flat_map(|a| a.split('|')).any(|alt| {
                    let left = alt.split(" as ").next().unwrap_or(alt).trim();
                    vivacity_resolver::version::parse_stability(left) != "stable"
                })
        })
        .map(|(n, _)| n.clone())
        .collect();
    // `extractAliases`: `<version> as <alias>`.
    let root_aliases = root
        .iter()
        .filter_map(|(n, c)| {
            let (from, to) = c.split_once(" as ")?;
            let norm = |v: &str| {
                vivacity_resolver::version::normalize(v.trim().trim_start_matches(['=', ' ']), None)
                    .ok()
            };
            Some((n.clone(), norm(from)?, norm(to)?))
        })
        .collect();
    let minimum_stability = match manifest
        .get("minimum-stability")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("rc") => "RC".to_owned(),
        Some(s @ ("dev" | "alpha" | "beta")) => s.to_owned(),
        _ => "stable".to_owned(),
    };
    let root_names = root.iter().map(|(n, _)| n.clone()).collect();
    let (skipped, mut requires) = match fixed {
        Some(f) => (f.names, f.requires),
        None => (std::collections::BTreeSet::new(), Vec::new()),
    };
    requires.extend(root);
    requires.retain(|(n, _)| !skipped.contains(n));
    let unlock = match start.mode {
        Some(UpdateMode::ListedWithTransitiveDeps) => Some(false),
        Some(UpdateMode::ListedWithTransitiveDepsNoRootRequire) => Some(true),
        _ => None,
    };
    vivacity_resolver::repository::LocalScope {
        requires,
        skipped,
        minimum_stability,
        flagged,
        root_aliases,
        unlock,
        root_names,
        with_dists: with_install,
        filters_fatal,
    }
}

/// Prints the reasons, then re-runs the command under Composer (or stops
/// with exit 3 under `--no-fallback`): the command line as it was given,
/// through [`forwarded_arguments`] — `--no-scripts` added, vivacity never
/// runs scripts, so Composer does not either — and [`hand_over`]. The
/// command is the one the line names: the hand-over that follows a
/// resolution (`hand_resolution_over`) runs inside `update`, `require` or
/// `remove` alike.
pub(crate) fn delegate_resolution(
    issues: &[vivacity_core::scope::ScopeIssue],
    no_fallback: bool,
) -> anyhow::Result<i32> {
    let tokens = command_line();
    let command = command_of(&tokens);
    // The reasons are vivacity's own narrative: at the default level when
    // Composer takes over (it prints its own run; under `-q` nothing), at
    // QUIET when the run stops here, so that an exit 3 is never mute.
    let composer = which_composer();
    let stops = no_fallback || composer.is_none();
    if stops {
        // The run ends here: what it kept under -v is its output.
        vivacity_core::io::release_deferred();
    } else {
        // Composer writes what vivacity held (the root-version warning), and
        // what it kept under -v.
        vivacity_core::io::drop_held();
        vivacity_core::io::drop_deferred();
    }
    let reason_line = |line: &str| {
        if stops {
            vivacity_core::say_always!("{line}");
        } else {
            vivacity_core::say!("{line}");
        }
    };
    reason_line(&format!(
        "vivacity: `{}` on this project is outside what vivacity handles natively:",
        canonical_command(&command)
    ));
    for issue in issues {
        reason_line(&format!("  - {issue}"));
    }
    if no_fallback {
        vivacity_core::say_always!("--no-fallback given: stopping here (nothing was written).");
        return Ok(3);
    }
    let Some(composer) = composer else {
        vivacity_core::say_always!(
            "composer not found for the fallback — install Composer or run with --no-plugins."
        );
        return Ok(3);
    };
    let forwarded = forwarded_arguments(&tokens, &command);
    hand_over(
        &composer,
        &forwarded,
        has_option(&tokens, "--offline"),
        None,
    )
    .context("cannot run composer")
}

/// The command line `run` was given, after the program name.
pub(crate) fn command_line() -> Vec<std::ffi::OsString> {
    let argv = ARGV.lock().map(|a| a.clone()).unwrap_or_default();
    argv.into_iter().skip(1).collect()
}

/// The command a line names: its first word that is not an option (clap
/// takes the subcommand first). An alias stays an alias.
fn command_of(tokens: &[std::ffi::OsString]) -> String {
    tokens
        .iter()
        .map(|t| t.to_string_lossy())
        .find(|t| !t.starts_with('-'))
        .unwrap_or_default()
        .into_owned()
}

/// The command's own name for an alias (clap's `r`, `rm`, `upgrade`).
fn canonical_command(command: &str) -> &str {
    match command {
        "r" => "require",
        "rm" => "remove",
        "upgrade" => "update",
        other => other,
    }
}

/// `flag` among the options, before any `--` (after it, a word is an
/// argument, whatever it looks like).
fn has_option(tokens: &[std::ffi::OsString], flag: &str) -> bool {
    tokens.iter().take_while(|t| *t != "--").any(|t| t == flag)
}

/// Runs Composer with `args`, as a child process in vivacity's own
/// directory — never `exec`, which would replace the process of a program
/// that embeds `vivacity::run` (ePHPm) — and returns its exit code (1 when a
/// signal ended it). The directory does not change so that a relative
/// `--working-dir` on the line means for Composer what it meant for
/// vivacity: Composer applies it itself (`Application::doRun`, `chdir`).
/// The child gets the run's verbosity ([`with_verbosity`]), and
/// `COMPOSER_DISABLE_NETWORK=1` for `--offline`, which Composer does not
/// have. The `delegating to` line shows the whole line handed over.
pub(crate) fn hand_over(
    composer: &std::path::Path,
    args: &[std::ffi::OsString],
    offline: bool,
    reason: Option<&str>,
) -> std::io::Result<i32> {
    // The Composer started here writes its own root-version warning: what
    // vivacity still holds is dropped, not written twice.
    vivacity_core::io::drop_held();
    // The archives this run put in Composer's files cache (an install, Flex
    // reading dists) would spare Composer their `  - Downloading` lines:
    // taken back first; one that cannot be stops the run here.
    if let Err(why) = vivacity_core::fetch::take_back_cached_dists() {
        vivacity_core::say_always!("Error: {why}");
        return Ok(1);
    }
    let line = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    match reason {
        Some(reason) => {
            vivacity_core::say!("vivacity: delegating to `composer {line}` ({reason})…")
        }
        None => vivacity_core::say!("vivacity: delegating to `composer {line}`…"),
    }
    let mut cmd = std::process::Command::new(composer);
    cmd.args(args);
    with_verbosity(&mut cmd);
    if offline {
        cmd.env("COMPOSER_DISABLE_NETWORK", "1");
    }
    Ok(cmd.status()?.code().unwrap_or(1))
}

/// The arguments a command hands to Composer: the command line after the
/// program name, without vivacity's own options — `--no-fallback`,
/// `--check-scope`, `--offline` (the caller sets `COMPOSER_DISABLE_NETWORK`),
/// `--run-scripts`, and `--no-autoloader` on `require`/`remove`, where
/// Composer has no such option — and with `--no-scripts` unless
/// `--run-scripts` asked Composer to run them: vivacity's contract holds
/// through the fallback. Everything after `--` is left as it is, and
/// `--no-scripts` goes before it. `command` is the word of the line, alias
/// included (`r`, `rm`).
pub(crate) fn forwarded_arguments(
    tokens: &[std::ffi::OsString],
    command: &str,
) -> Vec<std::ffi::OsString> {
    let has = |flag: &str| has_option(tokens, flag);
    let mut dropped = vec![
        "--no-fallback",
        "--check-scope",
        "--offline",
        "--run-scripts",
    ];
    if matches!(canonical_command(command), "require" | "remove") {
        dropped.push("--no-autoloader");
    }
    let mut before_dashes = true;
    let mut forwarded: Vec<std::ffi::OsString> = Vec::new();
    for t in tokens {
        if t == "--" {
            before_dashes = false;
        }
        if before_dashes && dropped.iter().any(|f| t == f) {
            continue;
        }
        forwarded.push(t.clone());
    }
    if !has("--run-scripts") && !has("--no-scripts") {
        let at = forwarded
            .iter()
            .position(|t| t == "--")
            .unwrap_or(forwarded.len());
        forwarded.insert(at, "--no-scripts".into());
    }
    forwarded
}

/// The run's verbosity for a Composer child process, the way Symfony
/// Console carries it (`Application::configureIO` exports
/// `SHELL_VERBOSITY`): `-1` quiet, `0` normal, `1` verbose. An explicit
/// `-q`/`-v` on the forwarded line still wins, as in Composer.
pub(crate) fn with_verbosity(cmd: &mut std::process::Command) {
    let level = match vivacity_core::io::level() {
        vivacity_core::io::Verbosity::Quiet => "-1",
        vivacity_core::io::Verbosity::Normal => "0",
        vivacity_core::io::Verbosity::Verbose => "1",
    };
    cmd.env("SHELL_VERBOSITY", level);
}

/// The `install` fallback: the reasons, then the command handed to Composer
/// before anything is written under vendor/ (or exit 3 under `--no-fallback`
/// / `--check-scope`). An `install` hands over its own command line through
/// [`forwarded_arguments`] — every option it was given, `--dry-run`
/// included; the install phase of `update`/`require`/`remove`, whose line is
/// not an install's, hands over the line [`after_update_line`] built
/// (`args.fallback_line`). Either way a child process ([`hand_over`]).
pub(crate) fn fallback_or_fail(
    args: &InstallArgs,
    scope: &vivacity_core::scope::ScopeReport,
) -> anyhow::Result<i32> {
    let composer = which_composer();
    let stops = args.no_fallback || args.check_scope || composer.is_none();
    if !stops {
        // Composer writes what vivacity held (the root-version warning).
        vivacity_core::io::drop_held();
    }
    let reason_line = |line: &str| {
        if stops {
            vivacity_core::say_always!("{line}");
        } else {
            vivacity_core::say!("{line}");
        }
    };
    reason_line("vivacity: this lock is outside what vivacity handles natively:");
    for issue in &scope.issues {
        reason_line(&format!("  - {issue}"));
    }
    if args.no_fallback || args.check_scope {
        vivacity_core::say_always!(
            "--no-fallback given: stopping here (no partial vendor/ was written)."
        );
        return Ok(3);
    }
    let Some(composer) = composer else {
        vivacity_core::say_always!(
            "composer not found for the fallback — install Composer or remove the unsupported items."
        );
        return Ok(3);
    };
    let forwarded = match &args.fallback_line {
        Some(line) => line.clone(),
        None => forwarded_arguments(&command_line(), "install"),
    };
    let code =
        hand_over(&composer, &forwarded, args.offline, None).context("cannot run composer")?;
    // The install phase of an `update` whose Flex recipes vivacity had
    // decided to apply: Composer's `install` runs Flex at
    // `POST_INSTALL_CMD`, which applies no recipe (`fetchRecipes` is
    // `POST_UPDATE_CMD` only). The lock is written already, so the
    // recipes are applied here, after it, as the update would have.
    if code == 0 {
        if let Some(plan) = args
            .post_update
            .as_ref()
            .and_then(|u| u.flex_plan.as_deref())
        {
            plan.print_and_apply()?;
        }
    }
    Ok(code)
}

/// The `composer install` line of the install phase that follows a
/// resolution (`Installer::run` with `setInstall(true)` after `doUpdate`),
/// should that phase go to Composer. Not the command line itself: by then
/// composer.lock is written and, for `require`/`remove`, composer.json is
/// edited — re-running `update` would resolve a second time, `remove` would
/// find its package already gone. Composer goes on with `doInstall` on the
/// lock it just solved; `composer install` reads that same lock. Every
/// option of the update that the install phase reads is carried, under
/// `install`'s own name (`--update-no-dev` arrives here as `no_dev`;
/// `require`'s `--prefer-*` in the internal fields of the same name).
/// Not carried: the audit — `install --audit` also sets `setErrorOnAudit`
/// (exit 5 on an advisory) where `update` audits without failing, and
/// vivacity does not audit natively either. `--no-scripts` always: none of
/// the three runs scripts in vivacity. `--dry-run` reaches a fallback only
/// through composer-merge-plugin when it is locked but not installed (the
/// install merges, the resolution did not); Composer then lists from the
/// lock on disk, not the dry run's unwritten one — but writes nothing.
pub(crate) fn after_update_line(args: &UpdateArgs) -> Vec<std::ffi::OsString> {
    let mut line: Vec<std::ffi::OsString> = vec!["install".into()];
    for (on, flag) in [
        (args.dry_run, "--dry-run"),
        (args.prefer_source, "--prefer-source"),
        (args.prefer_dist, "--prefer-dist"),
        (args.no_dev, "--no-dev"),
        (args.no_progress, "--no-progress"),
        (args.no_autoloader, "--no-autoloader"),
        (args.optimize_autoloader, "--optimize-autoloader"),
        (args.classmap_authoritative, "--classmap-authoritative"),
        (args.apcu_autoloader, "--apcu-autoloader"),
        (args.ignore_platform_reqs, "--ignore-platform-reqs"),
        (args.no_plugins, "--no-plugins"),
        (args.no_blocking, "--no-blocking"),
        (args.no_security_blocking, "--no-security-blocking"),
        (args.compat.no_interaction, "--no-interaction"),
        (args.compat.no_ansi, "--no-ansi"),
    ] {
        if on {
            line.push(flag.into());
        }
    }
    // `--name=value`, one word: Symfony would not take a separate value
    // that starts with `-` for the option's.
    let valued = |name: &str, value: &std::ffi::OsStr| {
        let mut word = std::ffi::OsString::from(format!("--{name}="));
        word.push(value);
        word
    };
    if let Some(how) = &args.prefer_install {
        line.push(valued("prefer-install", how.as_ref()));
    }
    if let Some(prefix) = &args.apcu_autoloader_prefix {
        line.push(valued("apcu-autoloader-prefix", prefix.as_ref()));
    }
    for req in &args.ignore_platform_req {
        line.push(valued("ignore-platform-req", req.as_ref()));
    }
    if let Some(dir) = &args.working_dir {
        line.push(valued("working-dir", dir.as_os_str()));
    }
    line.push("--no-scripts".into());
    line
}

/// Why `-v` on this project goes to Composer, if it does. Under `-v`,
/// Composer writes a line per plugin listener it dispatches
/// (`EventDispatcher.php:230`: `> post-autoload-dump: Class->method`), and
/// some plugins log on top of it (dealerdirect's `Plugin.php`, Flex's
/// configurators, composer-merge-plugin's `Logger.php:68`). vivacity prints
/// those lines for the plugins of `verbose_listeners::VERBOSE_PLUGINS`, in
/// the versions read; any other plugin Composer would load — installed
/// (project or `COMPOSER_HOME`) or locked, and allowed — hands the run
/// over, named (docs/plans/verbose-plugins.md). `None` under
/// `--no-plugins`, or without a readable composer.json (the command
/// reports that itself).
pub(crate) fn active_plugin(
    project: &std::path::Path,
    plugins_enabled: bool,
    run_scripts: bool,
) -> Option<String> {
    crate::verbose_listeners::issue(
        project,
        crate::plugin_order::global_composer_dir().as_deref(),
        plugins_enabled,
        run_scripts,
    )
}

/// The whole command handed to Composer before anything is read or written
/// — its verbosity, or a root version only Composer can guess: `tokens` is
/// the command line after the program name, forwarded through
/// [`forwarded_arguments`] and [`hand_over`]. `--no-fallback` stops with
/// exit 3 instead (and `--check-scope`, for the reasons it reports).
/// `remedy` completes "install Composer or …" when there is none.
pub(crate) fn delegate_for_verbosity(
    tokens: &[std::ffi::OsString],
    reason: &str,
    remedy: &str,
) -> i32 {
    let command = command_of(tokens);
    let stop = if has_option(tokens, "--no-fallback") {
        Some("--no-fallback")
    } else if has_option(tokens, "--check-scope") {
        Some("--check-scope")
    } else {
        None
    };
    if let Some(option) = stop {
        vivacity_core::say_always!(
            "vivacity: `{command}` with {reason} is outside what vivacity reproduces."
        );
        vivacity_core::say_always!("{option} given: stopping here (nothing was written).");
        return 3;
    }
    let Some(composer) = which_composer() else {
        vivacity_core::say_always!(
            "composer not found for the fallback — install Composer or {remedy}."
        );
        return 3;
    };
    let forwarded = forwarded_arguments(tokens, &command);
    match hand_over(
        &composer,
        &forwarded,
        has_option(tokens, "--offline"),
        Some(reason),
    ) {
        Ok(code) => code,
        Err(e) => {
            vivacity_core::say_always!("Error: cannot run composer: {e}");
            1
        }
    }
}

pub(crate) fn which_composer() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    // On Windows, composer installs itself as composer.bat/.cmd (a wrapper
    // around the .phar); `Command` can launch a .bat (via cmd.exe).
    let names: &[&str] = if cfg!(windows) {
        &["composer.bat", "composer.cmd", "composer.exe", "composer"]
    } else {
        &["composer"]
    };
    std::env::split_paths(&path)
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|c| c.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(tokens: &[&str], command: &str) -> Vec<String> {
        let tokens: Vec<std::ffi::OsString> = tokens.iter().map(Into::into).collect();
        forwarded_arguments(&tokens, command)
            .iter()
            .map(|t| t.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn vivacity_options_are_dropped_and_no_scripts_added_last() {
        for flag in ["--no-fallback", "--check-scope", "--offline"] {
            assert_eq!(
                forward(&["install", flag, "-o", "--working-dir", "p"], "install"),
                ["install", "-o", "--working-dir", "p", "--no-scripts"],
                "{flag}"
            );
        }
        // Composer's own options go through as they were written.
        assert_eq!(
            forward(
                &[
                    "install",
                    "--dry-run",
                    "--prefer-install=source",
                    "-q",
                    "--working-dir=p"
                ],
                "install"
            ),
            [
                "install",
                "--dry-run",
                "--prefer-install=source",
                "-q",
                "--working-dir=p",
                "--no-scripts"
            ]
        );
    }

    #[test]
    fn run_scripts_hands_the_scripts_to_composer() {
        assert_eq!(
            forward(&["install", "--run-scripts"], "install"),
            ["install"]
        );
        // Both asked: vivacity reads `--no-scripts` as winning, so does Composer.
        assert_eq!(
            forward(&["install", "--run-scripts", "--no-scripts"], "install"),
            ["install", "--no-scripts"]
        );
        // Already there: not doubled.
        assert_eq!(
            forward(&["update", "--no-scripts", "-n"], "update"),
            ["update", "--no-scripts", "-n"]
        );
    }

    #[test]
    fn nothing_after_double_dash_is_touched() {
        assert_eq!(
            forward(
                &["require", "--offline", "--", "--no-fallback", "--offline"],
                "require"
            ),
            [
                "require",
                "--no-scripts",
                "--",
                "--no-fallback",
                "--offline"
            ]
        );
        // After `--` these are arguments, not options: scripts stay off.
        assert_eq!(
            forward(
                &["require", "--", "--run-scripts", "--no-scripts"],
                "require"
            ),
            [
                "require",
                "--no-scripts",
                "--",
                "--run-scripts",
                "--no-scripts"
            ]
        );
    }

    #[test]
    fn no_autoloader_is_dropped_only_where_composer_lacks_it() {
        for command in ["require", "r", "remove", "rm"] {
            assert_eq!(
                forward(&[command, "a/b", "--no-autoloader"], command),
                [command, "a/b", "--no-scripts"],
                "{command}"
            );
        }
        for command in ["install", "update"] {
            assert_eq!(
                forward(&[command, "--no-autoloader"], command),
                [command, "--no-autoloader", "--no-scripts"],
                "{command}"
            );
        }
    }

    #[test]
    fn the_command_is_the_first_word_and_options_stop_at_double_dash() {
        let tokens: Vec<std::ffi::OsString> = ["rm", "a/b", "--", "--offline"]
            .iter()
            .map(Into::into)
            .collect();
        assert_eq!(command_of(&tokens), "rm");
        assert!(!has_option(&tokens, "--offline"));
        assert!(has_option(&tokens[..2], "a/b"));
    }

    fn update_args(argv: &[&str]) -> UpdateArgs {
        let argv = std::iter::once("vivacity").chain(argv.iter().copied());
        match Cli::try_parse_from(argv).expect("parses") {
            Cli::Update(args) => args,
            other => panic!("not an update: {other:?}"),
        }
    }

    fn line(args: &UpdateArgs) -> Vec<String> {
        after_update_line(args)
            .iter()
            .map(|t| t.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_install_phase_line_carries_what_the_install_reads() {
        assert_eq!(line(&update_args(&["update"])), ["install", "--no-scripts"]);
        // `require`'s preference, carried in the internal fields.
        let mut args = update_args(&["update"]);
        args.prefer_source = true;
        args.prefer_install = Some("auto".to_owned());
        assert_eq!(
            line(&args),
            [
                "install",
                "--prefer-source",
                "--prefer-install=auto",
                "--no-scripts"
            ]
        );
        let args = update_args(&[
            "update",
            "a/b",
            "--with",
            "c/d:1.0",
            "-W",
            "--prefer-stable",
            "--no-audit",
            "--offline",
            "--no-fallback",
            "-v",
            "--working-dir",
            "proj",
            "--no-dev",
            "-oa",
            "--apcu-autoloader",
            "--apcu-autoloader-prefix=pfx",
            "--no-progress",
            "--no-autoloader",
            "--ignore-platform-reqs",
            "--ignore-platform-req=ext-a",
            "--ignore-platform-req",
            "ext-b",
            "--no-plugins",
            "-n",
            "--no-ansi",
            "--no-blocking",
            "--no-security-blocking",
            "--dry-run",
        ]);
        // The resolution's own options (packages, `--with`, `-W`,
        // `--prefer-stable`), the audit and vivacity's options stay behind;
        // the verbosity travels as `SHELL_VERBOSITY`, `--offline` as
        // `COMPOSER_DISABLE_NETWORK`.
        assert_eq!(
            line(&args),
            [
                "install",
                "--dry-run",
                "--no-dev",
                "--no-progress",
                "--no-autoloader",
                "--optimize-autoloader",
                "--classmap-authoritative",
                "--apcu-autoloader",
                "--ignore-platform-reqs",
                "--no-plugins",
                "--no-blocking",
                "--no-security-blocking",
                "--no-interaction",
                "--no-ansi",
                "--apcu-autoloader-prefix=pfx",
                "--ignore-platform-req=ext-a",
                "--ignore-platform-req=ext-b",
                "--working-dir=proj",
                "--no-scripts",
            ]
        );
    }
}
