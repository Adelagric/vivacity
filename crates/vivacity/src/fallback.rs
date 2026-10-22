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
pub(crate) fn resolution_fallback(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    command: vivacity_core::scope::ResolutionCommand,
    no_plugins: bool,
    no_fallback: bool,
    with_install: bool,
    post_update_cmd: bool,
) -> anyhow::Result<Option<i32>> {
    let mut issues = vivacity_core::scope::resolution_issues(
        project,
        manifest,
        command,
        !no_plugins,
        with_install,
        post_update_cmd,
    );
    // Without an install, `POST_UPDATE_CMD` still runs the loaded plugins'
    // listeners on the local repository: what vivacity cannot reproduce
    // there goes to Composer now, before anything is written.
    if !with_install && post_update_cmd && !no_plugins {
        issues.extend(crate::no_install::issues_before_resolution(
            project, manifest,
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
    if issues.is_empty() {
        return Ok(None);
    }
    delegate_resolution(&issues, no_fallback).map(Some)
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
