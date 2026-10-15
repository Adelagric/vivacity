//! Handing a command to the real Composer: before any write, with the same
//! arguments, or stopping with exit 3 under `--no-fallback`.

use super::*;

/// The resolution commands' fallback (plan v0.16, decision 1), symmetric
/// to `install`'s: an installed, allowed plugin that changes what the
/// command resolves or writes, and that vivacity does not emulate for it,
/// hands the whole command to Composer with the same arguments — before
/// any write (the caller reverts its manifest edit first). `--no-fallback`
/// stops with exit 3 instead. `Some(code)` when the command was handled
/// here; `None` when vivacity goes on natively.
pub(crate) fn resolution_fallback(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    command: vivacity_core::scope::ResolutionCommand,
    no_plugins: bool,
    no_fallback: bool,
    with_install: bool,
) -> anyhow::Result<Option<i32>> {
    let mut issues = vivacity_core::scope::resolution_issues(
        project,
        manifest,
        command,
        !no_plugins,
        with_install,
    );
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
    if issues.is_empty() {
        return Ok(None);
    }
    delegate_resolution(project, command, &issues, no_fallback).map(Some)
}

/// Prints the reasons, then re-runs the command under Composer (or stops
/// with exit 3 under `--no-fallback`). vivacity-only options are dropped;
/// `--no-scripts` is added: vivacity never runs scripts, so Composer does
/// not either.
pub(crate) fn delegate_resolution(
    project: &std::path::Path,
    command: vivacity_core::scope::ResolutionCommand,
    issues: &[vivacity_core::scope::ScopeIssue],
    no_fallback: bool,
) -> anyhow::Result<i32> {
    // The reasons are vivacity's own narrative: at the default level when
    // Composer takes over (it prints its own run; under `-q` nothing), at
    // QUIET when the run stops here, so that an exit 3 is never mute.
    let composer = which_composer();
    let stops = no_fallback || composer.is_none();
    let reason_line = |line: &str| {
        if stops {
            vivacity_core::say_always!("{line}");
        } else {
            vivacity_core::say!("{line}");
        }
    };
    reason_line(&format!(
        "vivacity: `{}` on this project is outside what vivacity handles natively:",
        command.name()
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
    let argv = ARGV.lock().map(|a| a.clone()).unwrap_or_default();
    let forwarded = forwarded_arguments(&argv[argv.len().min(1)..], command.name());
    vivacity_core::say!(
        "vivacity: delegating to `composer {}`…",
        forwarded
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut cmd = std::process::Command::new(composer);
    cmd.args(&forwarded).current_dir(project);
    with_verbosity(&mut cmd);
    let status = cmd.status().context("cannot run composer")?;
    Ok(status.code().unwrap_or(1))
}

/// The arguments a command hands to Composer: the command line after the
/// program name, without vivacity's own options — `--no-fallback`,
/// `--check-scope`, `--offline` (the caller sets `COMPOSER_DISABLE_NETWORK`),
/// `--run-scripts`, and `--no-autoloader` on `require`/`remove`, where
/// Composer has no such option — and with `--no-scripts` unless
/// `--run-scripts` asked Composer to run them: vivacity's contract holds
/// through the fallback. Everything after `--` is left as it is.
pub(crate) fn forwarded_arguments(
    tokens: &[std::ffi::OsString],
    command: &str,
) -> Vec<std::ffi::OsString> {
    let has = |flag: &str| tokens.iter().take_while(|t| *t != "--").any(|t| t == flag);
    let mut dropped = vec![
        "--no-fallback",
        "--check-scope",
        "--offline",
        "--run-scripts",
    ];
    if matches!(command, "require" | "remove") {
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

pub(crate) fn fallback_or_fail(
    args: &InstallArgs,
    project: &std::path::Path,
    scope: &vivacity_core::scope::ScopeReport,
) -> anyhow::Result<i32> {
    let composer = which_composer();
    let stops = args.no_fallback || args.check_scope || composer.is_none();
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
    vivacity_core::say!("vivacity: delegating to `composer install`…");
    let mut cmd = std::process::Command::new(composer);
    // The contract holds through the fallback: vivacity never runs
    // scripts, so Composer does not either — unless `--run-scripts`, where
    // Composer runs them itself, in its own order; the plugin regime
    // follows.
    cmd.arg("install").current_dir(project);
    with_verbosity(&mut cmd);
    if !args.run_scripts || args.no_scripts {
        cmd.arg("--no-scripts");
    }
    if args.no_plugins {
        cmd.arg("--no-plugins");
    }
    if args.no_dev {
        cmd.arg("--no-dev");
    }
    if args.no_autoloader {
        cmd.arg("--no-autoloader");
    }
    if args.optimize_autoloader {
        cmd.arg("--optimize-autoloader");
    }
    if args.classmap_authoritative {
        cmd.arg("--classmap-authoritative");
    }
    if args.ignore_platform_reqs {
        cmd.arg("--ignore-platform-reqs");
    }
    for req in &args.ignore_platform_req {
        cmd.arg(format!("--ignore-platform-req={req}"));
    }
    #[cfg(unix)]
    if !args.spawn_fallback {
        use std::os::unix::process::CommandExt as _;
        let err = cmd.exec();
        return Err(err).context("cannot exec composer");
    }
    let status = cmd.status().context("cannot run composer")?;
    Ok(status.code().unwrap_or(1))
}

/// A plugin Composer would load in this project: installed (project or
/// `COMPOSER_HOME`) or locked — `install` activates it as soon as it is laid
/// out — and allowed by `allow-plugins`. Under `-v`, Composer writes a line
/// per plugin listener it dispatches (`EventDispatcher.php:230`:
/// `> post-autoload-dump: Class->method`), and the emulated plugins log on
/// top of it (bamarni's `Logger.php:34`, composer-merge-plugin's
/// `Logger.php:68`, Flex's configurators, dealerdirect's `Plugin.php`):
/// vivacity reproduces their effect, not those lines, so such a run goes to
/// Composer. `None` under `--no-plugins`, or without a readable
/// composer.json (the command reports that itself).
pub(crate) fn active_plugin(project: &std::path::Path, plugins_enabled: bool) -> Option<String> {
    if !plugins_enabled {
        return None;
    }
    let read = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(project.join(name)).ok()?).ok()
    };
    let manifest = read("composer.json")?;
    let mut names: Vec<String> = vivacity_core::scope::active_plugins(project, &manifest, true);
    if let Some(lock) = read("composer.lock") {
        for key in ["packages", "packages-dev"] {
            for p in lock
                .get(key)
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                if p.get("type").and_then(serde_json::Value::as_str) != Some("composer-plugin") {
                    continue;
                }
                if let Some(n) = p.get("name").and_then(serde_json::Value::as_str) {
                    let n = n.to_ascii_lowercase();
                    if !names.contains(&n)
                        && matches!(
                            vivacity_core::layout::plugin_allowed(&manifest, &n),
                            vivacity_core::layout::PluginVerdict::Allowed
                        )
                    {
                        names.push(n);
                    }
                }
            }
        }
    }
    names.into_iter().next()
}

/// The whole command handed to Composer because of its verbosity, before
/// anything is read or written: `tokens` is the command line after the
/// program name. vivacity's own options are dropped (`--no-fallback`,
/// `--check-scope`, `--offline` — which becomes `COMPOSER_DISABLE_NETWORK=1`
/// — and `--run-scripts`, whose absence becomes `--no-scripts`: vivacity's
/// contract holds through the fallback). Composer runs as a child process
/// in the same directory, so a relative `--working-dir` means the same
/// thing; never `exec`, which would replace an embedder's process.
/// `--no-fallback` (or `--check-scope`) stops with exit 3 instead.
/// The whole command handed to Composer because of its verbosity, before
/// anything is read or written: `tokens` is the command line after the
/// program name, forwarded through [`forwarded_arguments`]; `--offline`
/// becomes `COMPOSER_DISABLE_NETWORK=1`. Composer runs as a child process in
/// the same directory, so a relative `--working-dir` means the same thing;
/// never `exec`, which would replace an embedder's process. `--no-fallback`
/// stops with exit 3 instead.
pub(crate) fn delegate_for_verbosity(tokens: &[std::ffi::OsString], reason: &str) -> i32 {
    let has = |flag: &str| tokens.iter().take_while(|t| *t != "--").any(|t| t == flag);
    let command = tokens
        .iter()
        .map(|t| t.to_string_lossy())
        .find(|t| !t.starts_with('-'))
        .unwrap_or_default()
        .into_owned();
    if has("--no-fallback") {
        vivacity_core::say_always!(
            "vivacity: `{command}` with {reason} is outside what vivacity reproduces."
        );
        vivacity_core::say_always!("--no-fallback given: stopping here (nothing was written).");
        return 3;
    }
    let Some(composer) = which_composer() else {
        vivacity_core::say_always!(
            "composer not found for the fallback — install Composer or drop {reason}."
        );
        return 3;
    };
    let forwarded = forwarded_arguments(tokens, &command);
    vivacity_core::say!(
        "vivacity: delegating to `composer {}` ({reason})…",
        forwarded
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut cmd = std::process::Command::new(composer);
    cmd.args(&forwarded);
    with_verbosity(&mut cmd);
    if has("--offline") {
        cmd.env("COMPOSER_DISABLE_NETWORK", "1");
    }
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
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
