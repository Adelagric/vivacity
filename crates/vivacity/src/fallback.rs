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
    eprintln!(
        "vivacity: `{}` on this project is outside what vivacity handles natively:",
        command.name()
    );
    for issue in issues {
        eprintln!("  - {issue}");
    }
    if no_fallback {
        eprintln!("--no-fallback given: stopping here (nothing was written).");
        return Ok(3);
    }
    let Some(composer) = which_composer() else {
        eprintln!(
            "composer not found for the fallback — install Composer or run with --no-plugins."
        );
        return Ok(3);
    };
    let argv = ARGV.lock().map(|a| a.clone()).unwrap_or_default();
    let forwarded: Vec<std::ffi::OsString> = argv
        .iter()
        .skip(1)
        .filter(|a| a.to_string_lossy() != "--no-fallback" && a.to_string_lossy() != "--offline")
        .cloned()
        .collect();
    let mut forwarded = forwarded;
    if !forwarded.iter().any(|a| a == "--no-scripts") {
        forwarded.push("--no-scripts".into());
    }
    eprintln!(
        "vivacity: delegating to `composer {}`…",
        forwarded
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut cmd = std::process::Command::new(composer);
    cmd.args(&forwarded).current_dir(project);
    let status = cmd.status().context("cannot run composer")?;
    Ok(status.code().unwrap_or(1))
}

pub(crate) fn fallback_or_fail(
    args: &InstallArgs,
    project: &std::path::Path,
    scope: &vivacity_core::scope::ScopeReport,
) -> anyhow::Result<i32> {
    eprintln!("vivacity: this lock is outside what vivacity handles natively:");
    for issue in &scope.issues {
        eprintln!("  - {issue}");
    }
    if args.no_fallback || args.check_scope {
        eprintln!("--no-fallback given: stopping here (no partial vendor/ was written).");
        return Ok(3);
    }
    let composer = which_composer();
    let Some(composer) = composer else {
        eprintln!(
            "composer not found for the fallback — install Composer or remove the unsupported items."
        );
        return Ok(3);
    };
    eprintln!("vivacity: delegating to `composer install`…");
    let mut cmd = std::process::Command::new(composer);
    // The contract holds through the fallback: vivacity never runs
    // scripts, so Composer does not either — unless `--run-scripts`, where
    // Composer runs them itself, in its own order; the plugin regime
    // follows.
    cmd.arg("install").current_dir(project);
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
