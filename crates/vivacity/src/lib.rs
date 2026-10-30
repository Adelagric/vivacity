//! vivacity: a fast, Composer-compatible installer.
//! Each line goes to the stream Composer writes it on: stdout for the few
//! Composer sends there (plugins' lines, GitHub Actions annotations), stderr
//! for the rest of the narrative. Exit codes: 0 ok, 1 runtime error,
//! 2 usage (clap), 3 out of scope with no fallback possible, 4 platform.
//!
//! The `vivacity` binary is just a call to [`run`]: another program can
//! embed the commands as they are (`vivacity::run(["vivacity", "install", …])`)
//! and get the same exit code.

// stdout is written through `vivacity_core::out!` only (one write per line).
#![deny(clippy::print_stdout)]

mod extension_installers;
mod install;
use install::*;
mod update;
use update::*;
mod manifest;
use manifest::*;
mod fallback;
use fallback::*;
mod flex_install;
use flex_install::*;
mod config;
use config::*;
mod dump;
use dump::*;
mod remove;
use remove::*;
mod discovery;
mod effect_plugins;
mod flex;
mod flex_recipes;
mod no_install;
mod plugin_allow;
mod plugin_order;
mod require;
mod scripts;
mod verbose_listeners;

use anyhow::Context as _;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "vivacity",
    version,
    about = "Fast, Composer-compatible installer"
)]
enum Cli {
    /// Install dependencies from composer.lock (drop-in `composer install`).
    Install(InstallArgs),
    /// Regenerate the autoloader (drop-in `composer dump-autoload`).
    #[command(name = "dump-autoload", alias = "dumpautoload")]
    DumpAutoload(DumpArgs),
    /// Resolve dependencies and write composer.lock (drop-in `composer update`).
    #[command(alias = "upgrade")]
    Update(UpdateArgs),
    /// Remove packages from composer.json, then update (drop-in `composer remove`).
    #[command(alias = "rm")]
    Remove(RemoveArgs),
    /// Add packages to composer.json, then update (drop-in `composer require`).
    #[command(alias = "r")]
    Require(RequireArgs),
    /// Print Composer's cache directory as vivacity uses it (`config --global cache-dir` only).
    Config(ConfigArgs),
}

#[derive(clap::Args, Debug)]
struct RequireArgs {
    /// Packages to require: `vendor/name`, `vendor/name:^1.0`, `vendor/name ^1.0`.
    #[arg(value_name = "PACKAGES")]
    packages: Vec<String>,
    /// Add to require-dev.
    #[arg(long)]
    dev: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    prefer_source: bool,
    #[arg(long)]
    prefer_dist: bool,
    #[arg(long)]
    prefer_install: Option<String>,
    /// Exact constraint (the version found) instead of `^x.y`.
    #[arg(long)]
    fixed: bool,
    #[arg(long)]
    no_suggest: bool,
    #[arg(long)]
    no_progress: bool,
    /// Do not update dependencies (implies --no-install).
    #[arg(long)]
    no_update: bool,
    /// Write the lock file without installing.
    #[arg(long)]
    no_install: bool,
    #[arg(long)]
    no_audit: bool,
    #[arg(long, value_name = "FORMAT")]
    audit_format: Option<String>,
    #[arg(long)]
    no_blocking: bool,
    #[arg(long)]
    no_security_blocking: bool,
    /// Run the update with --no-dev.
    #[arg(long)]
    update_no_dev: bool,
    /// Also update dependencies, except those required by the root (`-w`).
    #[arg(short = 'w', long)]
    update_with_dependencies: bool,
    /// Alias for --update-with-dependencies.
    #[arg(long)]
    with_dependencies: bool,
    /// Also update dependencies, root requirements included (`-W`).
    #[arg(short = 'W', long)]
    update_with_all_dependencies: bool,
    /// Alias for --update-with-all-dependencies.
    #[arg(long)]
    with_all_dependencies: bool,
    #[arg(short = 'm', long)]
    minimal_changes: bool,
    #[arg(long)]
    prefer_stable: bool,
    #[arg(long)]
    prefer_lowest: bool,
    /// Sort the packages of the section (also `config.sort-packages`).
    #[arg(long)]
    sort_packages: bool,
    #[arg(short = 'o', long)]
    optimize_autoloader: bool,
    #[arg(short = 'a', long)]
    classmap_authoritative: bool,
    #[arg(long)]
    apcu_autoloader: bool,
    #[arg(long, value_name = "PREFIX")]
    apcu_autoloader_prefix: Option<String>,
    #[arg(long)]
    ignore_platform_reqs: bool,
    #[arg(long = "ignore-platform-req", value_name = "REQ")]
    ignore_platform_req: Vec<String>,
    #[arg(short = 'n', long)]
    no_interaction: bool,
    #[arg(short = 'q', long)]
    quiet: bool,
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Accepted for compatibility: vivacity never writes ANSI escapes.
    #[arg(long)]
    no_ansi: bool,
    #[arg(long)]
    no_scripts: bool,
    #[arg(long)]
    no_plugins: bool,
    /// Do not generate the autoloader.
    #[arg(long)]
    no_autoloader: bool,
    #[arg(long)]
    no_fallback: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long, value_name = "DIR")]
    working_dir: Option<PathBuf>,
}

/// Composer's global options that a CI command line passes to every command
/// (`composer install --no-dev --no-interaction --no-ansi -o`), accepted where
/// a command did not declare them so that line runs instead of exiting 2 with
/// a usage error. `-n` and `--no-ansi` are true no-ops: vivacity never prompts
/// and never writes an ANSI escape. The verbosity is not read from these
/// fields: `run` computes it from the command line as Symfony Console does
/// (`vivacity_core::io::requested_verbosity`), for every command alike.
#[derive(clap::Args, Debug, Default)]
struct GlobalCompatArgs {
    /// Accepted for compatibility: vivacity is never interactive.
    #[arg(short = 'n', long)]
    no_interaction: bool,
    /// Only the lines Composer writes at QUIET (errors, the stale-lock
    /// warning).
    #[arg(short = 'q', long)]
    quiet: bool,
    /// Composer's VERBOSE lines; `-vv`/`-vvv` hand the command to Composer.
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Accepted for compatibility: vivacity never writes ANSI escapes.
    #[arg(long)]
    no_ansi: bool,
}

#[derive(clap::Args, Debug)]
struct RemoveArgs {
    /// Packages to remove; `vendor/*` patterns accepted.
    #[arg(value_name = "PACKAGES")]
    packages: Vec<String>,
    /// Remove from require-dev.
    #[arg(long)]
    dev: bool,
    /// Do not update dependencies (implies --no-install).
    #[arg(long)]
    no_update: bool,
    /// Write the lock file without installing.
    #[arg(long)]
    no_install: bool,
    /// Accepted for compatibility: vivacity does not audit (yet).
    #[arg(long)]
    no_audit: bool,
    /// Run the update with --no-dev.
    #[arg(long)]
    update_no_dev: bool,
    /// Deprecated in Composer (default behaviour).
    #[arg(short = 'w', long)]
    update_with_dependencies: bool,
    /// Also update dependencies that are root requirements (`-W`).
    #[arg(short = 'W', long)]
    update_with_all_dependencies: bool,
    /// Alias for --update-with-all-dependencies.
    #[arg(long)]
    with_all_dependencies: bool,
    /// Only update the listed packages.
    #[arg(long)]
    no_update_with_dependencies: bool,
    /// Remove every locked package that nothing requires.
    #[arg(long)]
    unused: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(short = 'm', long)]
    minimal_changes: bool,
    /// Accepted for compatibility: vivacity is never interactive, does not
    /// audit and applies no blocking policy.
    #[arg(short = 'n', long)]
    no_interaction: bool,
    #[arg(long)]
    no_blocking: bool,
    #[arg(long)]
    no_security_blocking: bool,
    #[arg(long, value_name = "FORMAT")]
    audit_format: Option<String>,
    #[arg(long)]
    apcu_autoloader: bool,
    #[arg(long, value_name = "PREFIX")]
    apcu_autoloader_prefix: Option<String>,
    #[arg(short = 'q', long)]
    quiet: bool,
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Accepted for compatibility: vivacity never writes ANSI escapes.
    #[arg(long)]
    no_ansi: bool,
    #[arg(long)]
    no_progress: bool,
    #[arg(long)]
    no_scripts: bool,
    #[arg(long)]
    no_plugins: bool,
    /// Do not generate the autoloader.
    #[arg(long)]
    no_autoloader: bool,
    #[arg(short = 'o', long)]
    optimize_autoloader: bool,
    #[arg(short = 'a', long)]
    classmap_authoritative: bool,
    #[arg(long)]
    ignore_platform_reqs: bool,
    #[arg(long = "ignore-platform-req", value_name = "REQ")]
    ignore_platform_req: Vec<String>,
    #[arg(long)]
    no_fallback: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long, value_name = "DIR")]
    working_dir: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct UpdateArgs {
    #[command(flatten)]
    compat: GlobalCompatArgs,
    /// Accepted for compatibility: the run goes to Composer
    /// (`update-with-minimal-changes` is not reproduced).
    #[arg(short = 'm', long)]
    minimal_changes: bool,
    /// Accepted for compatibility: vivacity prints no progress bar.
    #[arg(long)]
    no_progress: bool,
    /// Packages to update (the others stay locked); `vendor/*` patterns accepted.
    #[arg(value_name = "PACKAGES")]
    packages: Vec<String>,
    /// Temporary version constraint for this run, e.g. `foo/bar:1.0.0`.
    #[arg(long = "with", value_name = "REQ")]
    with: Vec<String>,
    /// Also update their dependencies, except those required by the root (`-w`).
    #[arg(short = 'w', long)]
    with_dependencies: bool,
    /// Also update their dependencies, root requirements included (`-W`).
    #[arg(short = 'W', long)]
    with_all_dependencies: bool,
    /// Write the lock file without installing.
    #[arg(long)]
    no_install: bool,
    /// Outputs the operations but will not execute anything.
    #[arg(long)]
    dry_run: bool,
    /// Do not install require-dev packages (they are still resolved).
    #[arg(long)]
    no_dev: bool,
    /// Do not generate the autoloader.
    #[arg(long)]
    no_autoloader: bool,
    #[arg(short = 'o', long)]
    optimize_autoloader: bool,
    #[arg(short = 'a', long)]
    classmap_authoritative: bool,
    /// Use APCu to cache found/not-found classes.
    #[arg(long)]
    apcu_autoloader: bool,
    /// A custom prefix for the APCu cache (implies --apcu-autoloader).
    #[arg(long, value_name = "PREFIX")]
    apcu_autoloader_prefix: Option<String>,
    /// Accepted for compatibility: vivacity never runs scripts.
    #[arg(long)]
    no_scripts: bool,
    #[arg(long)]
    no_plugins: bool,
    /// Accepted for compatibility: vivacity does not audit (yet).
    #[arg(long)]
    no_audit: bool,
    /// Disable blocking policies (security advisories, malware).
    #[arg(long)]
    no_blocking: bool,
    #[arg(long)]
    no_security_blocking: bool,
    #[arg(long)]
    prefer_stable: bool,
    #[arg(long)]
    prefer_lowest: bool,
    #[arg(long)]
    ignore_platform_reqs: bool,
    #[arg(long = "ignore-platform-req", value_name = "REQ")]
    ignore_platform_req: Vec<String>,
    #[arg(long)]
    no_fallback: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long, value_name = "DIR")]
    working_dir: Option<PathBuf>,
    /// Internal: `require`'s `--prefer-source` / `--prefer-dist` /
    /// `--prefer-install`, which its install phase reads
    /// (`RequireCommand` → `setPreferSource`/`setPreferDist`). `update`
    /// does not accept them yet; `remove` has none, as in Composer.
    #[arg(skip)]
    prefer_source: bool,
    #[arg(skip)]
    prefer_dist: bool,
    #[arg(skip)]
    prefer_install: Option<String>,
    /// Internal: the command has dispatched Composer's `COMMAND` event
    /// itself (`require` after its "Running composer update" line,
    /// `remove` before it); `update` dispatches it in `resolve_and_lock`,
    /// where `UpdateCommand::execute` does.
    #[arg(skip)]
    command_dispatched: bool,
    /// Internal: an `install` that found no usable lock (`config.lock:
    /// false`, no composer.lock, no `packages` key) and runs as an update —
    /// `Installer::run`'s line saying so, printed after `COMMAND`, before
    /// the repositories load. Its command is still `install` (no Flex
    /// `symfony/thanks` reminder).
    #[arg(skip)]
    lockless_install: Option<String>,
    /// Internal: the project's `allow-plugins` as `remove` found it, when it
    /// then took entries out (RemoveCommand.php:242-252) — after its
    /// `Composer` had loaded the plugins with these rules (:216-222), which
    /// the `PluginManager` keeps for the run: a removed plugin's
    /// `uninstall` hook runs (codeception/c3 deletes c3.php), its listeners
    /// too while another package keeps it installed.
    #[arg(skip)]
    allow_plugins_at_load: Option<serde_json::Value>,
    /// Internal: the command this resolution runs for, when it is not
    /// `update` (`require`, `remove`): some plugins act differently.
    #[arg(skip)]
    resolution_command: Option<vivacity_core::scope::ResolutionCommand>,
}

#[derive(clap::Args, Debug)]
struct DumpArgs {
    /// Do not include require-dev packages in the autoloader.
    #[arg(long)]
    no_dev: bool,
    /// Optimized classmap: every PSR directory is scanned.
    #[arg(short = 'o', long)]
    optimize: bool,
    /// Authoritative classmap (implies -o).
    #[arg(short = 'a', long)]
    classmap_authoritative: bool,
    /// Use APCu to cache found/not-found classes.
    #[arg(long = "apcu")]
    apcu_autoloader: bool,
    /// A custom prefix for the APCu cache (implies --apcu).
    #[arg(long = "apcu-prefix", value_name = "PREFIX")]
    apcu_autoloader_prefix: Option<String>,
    #[arg(long)]
    ignore_platform_reqs: bool,
    #[arg(long = "ignore-platform-req", value_name = "REQ")]
    ignore_platform_req: Vec<String>,
    /// Like Composer: no plugin, not even emulated ones (composer/installers).
    #[arg(long)]
    no_plugins: bool,
    /// Run `pre-autoload-dump` / `post-autoload-dump` through
    /// `composer run-script`.
    #[arg(long)]
    run_scripts: bool,
    /// Composer's global `--no-scripts`: accepted, and already the default
    /// here (scripts only run with `--run-scripts`).
    #[arg(long, conflicts_with = "run_scripts")]
    no_scripts: bool,
    /// Stop with exit 3 instead of handing the command to Composer (the
    /// other commands' option; `dump-autoload` stops with 3 anyway on what
    /// it cannot dump, and hands `-vv`/`-vvv` to Composer without it).
    #[arg(long)]
    no_fallback: bool,
    #[arg(long, value_name = "DIR")]
    working_dir: Option<PathBuf>,
    #[command(flatten)]
    compat: GlobalCompatArgs,
}

#[derive(clap::Args, Debug)]
struct InstallArgs {
    #[command(flatten)]
    compat: GlobalCompatArgs,
    /// Do not install require-dev packages.
    #[arg(long)]
    no_dev: bool,
    /// Install from source: refused, vivacity only lays out dists.
    #[arg(long)]
    prefer_source: bool,
    /// Install from dist: accepted for compatibility, vivacity's only way.
    #[arg(long)]
    prefer_dist: bool,
    /// `dist`, `source` or `auto`, as Composer's option.
    #[arg(long, value_name = "PREFER-INSTALL")]
    prefer_install: Option<String>,
    /// Accepted for compatibility: vivacity prints no progress bar.
    #[arg(long)]
    no_progress: bool,
    /// Do not generate the autoloader.
    #[arg(long)]
    no_autoloader: bool,
    /// Optimized classmap (`-o`).
    #[arg(short = 'o', long)]
    optimize_autoloader: bool,
    /// Authoritative classmap (`-a`, implies -o).
    #[arg(short = 'a', long)]
    classmap_authoritative: bool,
    /// Use APCu to cache found/not-found classes.
    #[arg(long)]
    apcu_autoloader: bool,
    /// A custom prefix for the APCu cache (implies --apcu-autoloader).
    #[arg(long, value_name = "PREFIX")]
    apcu_autoloader_prefix: Option<String>,
    /// Accepted for compatibility: vivacity never runs scripts.
    #[arg(long)]
    no_scripts: bool,
    /// Run the project's scripts through `composer run-script`, at the
    /// points where Composer dispatches them (pre-install-cmd,
    /// pre/post-autoload-dump, post-install-cmd).
    #[arg(long)]
    run_scripts: bool,
    /// Like Composer: no plugin, not even emulated ones (composer/installers);
    /// everything installs into vendor/.
    #[arg(long)]
    no_plugins: bool,
    /// Ignore all platform requirements.
    #[arg(long)]
    ignore_platform_reqs: bool,
    /// Ignore one specific platform requirement (repeatable, `ext-*` patterns).
    #[arg(long = "ignore-platform-req", value_name = "REQ")]
    ignore_platform_req: Vec<String>,
    /// Never delegate to composer (fail explicitly when out of scope).
    #[arg(long)]
    no_fallback: bool,
    /// Report whether this lock is installed natively (exit 0) or handed to
    /// Composer (exit 3, the reasons on stderr), and stop before any
    /// download or write. Implies --no-fallback.
    #[arg(long)]
    check_scope: bool,
    /// Only use local caches (no network access).
    #[arg(long)]
    offline: bool,
    /// Disable blocking policies (the lock's malware list).
    #[arg(long)]
    no_blocking: bool,
    #[arg(long)]
    no_security_blocking: bool,
    /// Rejected, as in Composer (`composer update --no-install`).
    #[arg(long)]
    no_install: bool,
    /// Checks (policies, scope, platform) and prints the operations
    /// without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Project directory (default: current directory).
    #[arg(long, value_name = "DIR")]
    working_dir: Option<PathBuf>,
    /// Internal: the line handed to Composer when this install goes to the
    /// fallback, where it is not the command line itself — the install
    /// phase of `update`/`require`/`remove` (`after_update_line`). `None`:
    /// the command line, forwarded.
    #[arg(skip)]
    fallback_line: Option<Vec<std::ffi::OsString>>,
    /// Internal: the lock of a dry-run update, never written — the install
    /// phase reads it instead of composer.lock (`Locker::setLockData`
    /// keeps the data in memory; `mockLocalRepositories`).
    #[arg(skip)]
    virtual_lock: Option<serde_json::Value>,
    /// Internal: the manifest as the dry run's patched root package sees
    /// it (`require`/`require-dev` of `RequireCommand`/`RemoveCommand`
    /// applied in memory).
    #[arg(skip)]
    virtual_manifest: Option<serde_json::Value>,
    /// Internal: `Some` for the install following a resolution
    /// (`doInstall` with `alreadySolved`: the lock pool has already been
    /// filtered), with what `Installer::run` prints around it — the
    /// suggestions and abandoned packages before the dump, Flex's
    /// `POST_UPDATE_CMD` after the funding — which this phase prints at
    /// Composer's points.
    #[arg(skip)]
    post_update: Option<Box<UpdateReport>>,
}

impl InstallArgs {
    /// The install following a resolution (`update`, `require`, `remove`).
    fn after_update(&self) -> bool {
        self.post_update.is_some()
    }
}

/// `VIVACITY_TRACE=1`: duration of each phase on stderr (perf diagnostics).
fn trace(label: &str, since: std::time::Instant) {
    if std::env::var_os("VIVACITY_TRACE").is_some() {
        eprintln!(
            "trace: {label:<22} {:>7.1} ms",
            since.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Run a vivacity command line (`args[0]` is the program name, as in
/// `std::env::args()`), runtime errors included: the return value is the
/// binary's exit code. A usage error (clap) prints the help or the message
/// and returns 2; `--help`/`--version` return 0.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let argv: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    // What git answered belongs to one run (forgotten when the guard
    // drops, panics included).
    let _git_answers = vivacity_core::root_version::remember_git_answers();
    run_once(argv)
}

/// [`run`], between the two moments git's answers are forgotten.
fn run_once(argv: Vec<std::ffi::OsString>) -> i32 {
    // The configuration files are read once per run (`fetch::global_json`).
    vivacity_core::fetch::forget_global_files();
    fallback::forget_manifest_restored();
    fallback::forget_hold_trigger();
    vivacity_core::io::forget_held_write();
    vivacity_resolver::repository::forget_preparsed();
    // Kept for the resolution fallback, which re-runs the whole command
    // under Composer with the same arguments.
    if let Ok(mut a) = ARGV.lock() {
        *a = argv.clone();
    }
    // The run's verbosity, computed as Symfony Console computes Composer's
    // (`Application::configureIO`), and reset when the run ends.
    let tokens: Vec<String> = argv
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let requested = vivacity_core::io::requested_verbosity(
        &tokens,
        std::env::var("SHELL_VERBOSITY").ok().as_deref(),
    );
    let _verbosity = vivacity_core::io::Scope::new(match requested {
        0 => vivacity_core::io::Verbosity::Quiet,
        1 => vivacity_core::io::Verbosity::Normal,
        _ => vivacity_core::io::Verbosity::Verbose,
    });
    let cli = match Cli::try_parse_from(argv.clone()) {
        Ok(cli) => cli,
        Err(e) => {
            // `e.exit()` writes the help to stdout, the error to stderr.
            let _ = e.print();
            return e.exit_code();
        }
    };
    // What vivacity cannot print as Composer does goes to Composer, whole,
    // before anything is read or written: `-vv`/`-vvv`, and `-v` when a
    // plugin Composer would load writes lines vivacity does not reproduce
    // (`verbose_listeners`). `--check-scope` asks what vivacity would do
    // with the lock, not for a run: it is answered natively whatever the
    // verbosity.
    let tail = &argv[1..];
    let check_scope = tail.iter().any(|t| t == "--check-scope");
    if requested >= 3 && !check_scope {
        let reason = "-vv/-vvv, Composer's debug output";
        return delegate_for_verbosity(tail, reason, &format!("drop {reason}"));
    }
    if requested == 2 && !check_scope {
        if let Ok(project) = project_dir(cli.working_dir()) {
            if let Some(reason) = active_plugin(&project, cli.plugins_enabled(), cli.run_scripts())
            {
                return delegate_for_verbosity(tail, &reason, "drop -v");
            }
        }
    }
    // An alternate manifest (`COMPOSER=`): the command is Composer's, whole,
    // before anything is read or written.
    if let Some(reason) = cli.unsupported_hand_over() {
        return delegate_for_verbosity(tail, &reason, "drop it");
    }
    // Composer's cache directory as `Config::get('cache-dir')` resolves it,
    // `disable-tls`, the minimal-changes mode: what vivacity does not
    // reproduce is Composer's, whole, before anything is read or written
    // (docs/plans/action-cache.md).
    let (cache_reason, tls_reason) = cli.startup_hand_over();
    if let Some(reason) = tls_reason {
        let remedy = if reason == "disable-tls is set" {
            "unset disable-tls".to_owned()
        } else {
            format!("fix the configuration ({reason})")
        };
        return delegate_for_verbosity(tail, &reason, &remedy);
    }
    if let Some(reason) = cache_reason {
        return delegate_for_verbosity(
            tail,
            &reason,
            &format!("change the cache directory settings ({reason})"),
        );
    }
    // The `_composer_tmpN` counter belongs to one run, like a process.
    verbose_listeners::reset();
    // Under -v, a resolution that installs may still go to Composer once
    // solved (a plugin it would activate, `verbose_listeners::resolved_issue`):
    // its lines are kept until then, so that Composer does not print them a
    // second time (released by `resolve_and_lock`, dropped by the hand-over).
    if requested == 2 && cli.plugins_enabled() && cli.resolves_then_installs() {
        vivacity_core::io::defer();
    }
    // `RootPackageLoader::load` guesses the root's version with git, then
    // hg, fossil and svn (`VersionGuesser::guessVersion`); vivacity reads git
    // only. Where one of the others would answer, the command is Composer's,
    // whole, before anything is read or written — at every level, for every
    // type of root (the version reaches installed.php). No working copy of
    // them above the project (the common case): a few `stat`s, no process.
    // `config --global` loads no root package.
    let loads_root = !matches!(cli, Cli::Config(_));
    if let (true, Ok(project)) = (loads_root, project_dir(cli.working_dir())) {
        if let Some(vcs) = foreign_vcs_root(&project) {
            return delegate_for_verbosity(
                tail,
                &format!("a root version from {vcs}"),
                "set COMPOSER_ROOT_VERSION",
            );
        }
    }
    // `secure-http`: a new `Config`, nothing warned yet, the setting read
    // only if an insecure URL shows up.
    vivacity_core::secure_http::begin(
        project_dir(cli.working_dir()).ok().as_deref(),
        cli.offline(),
    );
    match dispatch(cli) {
        Ok(code) => {
            // A command that wrote nothing else still writes the line it held
            // (unless it went to Composer, which dropped it).
            vivacity_core::io::release_held();
            code
        }
        Err(e) => {
            if let Some(ComposerException(message)) = e
                .chain()
                .find_map(|c| c.downcast_ref::<ComposerException>())
            {
                vivacity_core::say_always!("{message}");
                vivacity_core::io::github_action_error(message);
                return 1;
            }
            vivacity_core::say_always!("Error: {e:?}");
            exit_code_of(&e)
        }
    }
}

/// [`vivacity_core::root_version::root_version_from_foreign_vcs`] on the
/// project's composer.json, when there is one vivacity would read (no
/// `COMPOSER=` alternate, valid JSON).
fn foreign_vcs_root(project: &std::path::Path) -> Option<&'static str> {
    if refuse_alternate_manifest().is_err()
        || !vivacity_core::root_version::foreign_vcs_marked(project)
    {
        return None;
    }
    let text = std::fs::read_to_string(project.join("composer.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    vivacity_core::root_version::root_version_from_foreign_vcs(&manifest, project)
}

/// A session error on its way to the CLI: a transport failure keeps its nature
/// so the exit code is Composer's 100, everything else reads as before.
fn session_error(e: vivacity_resolver::session::SessionError) -> anyhow::Error {
    if e.kind == vivacity_resolver::session::SessionErrorKind::Transport {
        anyhow::Error::new(TransportFailure(e.message))
    } else if e.kind == vivacity_resolver::session::SessionErrorKind::Unsupported {
        anyhow::Error::new(HandOverReason(e.message))
    } else if e.message == vivacity_resolver::repository::LOCK_WITHOUT_DEV {
        anyhow::Error::new(ComposerException(e.message))
    } else {
        anyhow::anyhow!("{}", e.message)
    }
}

/// What vivacity does not reproduce, carried up to a caller that can hand
/// the command over (`require`'s version discovery): the reason.
#[derive(Debug)]
pub(crate) struct HandOverReason(pub(crate) String);

impl std::fmt::Display for HandOverReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HandOverReason {}

/// An exception whose message is Composer's word for word: printed as
/// vivacity prints `RootPackageLoader`'s refusals — the message alone at
/// QUIET, without Symfony Console's box or vivacity's `Error:` prefix — and
/// annotated under GitHub Actions (`Application::doRun`). Exit 1.
#[derive(Debug)]
pub(crate) struct ComposerException(pub(crate) String);

impl std::fmt::Display for ComposerException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ComposerException {}

/// A failure to fetch, carried as a typed error so `exit_code_of` finds it in
/// the chain. It exists because the resolver flattens its own errors to text
/// at the `HttpFetch` boundary, keeping only a flag: this type is where that
/// flag lands on the CLI's side. Its `Display` is the message alone, so the
/// `Error:` line stays what it was.
#[derive(Debug)]
struct TransportFailure(String);

impl std::fmt::Display for TransportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransportFailure {}

/// Composer rewrites a `TransportException`'s code to
/// `Installer::ERROR_TRANSPORT_EXCEPTION` before letting Symfony Console turn it
/// into an exit code (`Application.php:502-506`, `Installer.php:92`), so
/// anything it failed to fetch exits **100**, not 1. Measured on three shapes
/// of the same failure: a dist answering 404, an unreachable `composer`
/// repository, and a local dist that is not there.
///
/// The chain is walked rather than the outermost error inspected, because the
/// fetch error travels under whatever context the caller added.
fn exit_code_of(e: &anyhow::Error) -> i32 {
    let transport = e.chain().any(|c| {
        matches!(
            c.downcast_ref::<vivacity_core::Error>(),
            Some(vivacity_core::Error::Http { .. } | vivacity_core::Error::Transport(_))
        ) || c.downcast_ref::<TransportFailure>().is_some()
    });
    if transport {
        100
    } else {
        1
    }
}

static ARGV: std::sync::Mutex<Vec<std::ffi::OsString>> = std::sync::Mutex::new(Vec::new());

impl Cli {
    fn working_dir(&self) -> Option<&std::path::Path> {
        match self {
            Cli::Install(a) => a.working_dir.as_deref(),
            Cli::DumpAutoload(a) => a.working_dir.as_deref(),
            Cli::Update(a) => a.working_dir.as_deref(),
            Cli::Remove(a) => a.working_dir.as_deref(),
            Cli::Require(a) => a.working_dir.as_deref(),
            Cli::Config(_) => None,
        }
    }

    /// `--offline` (no network: `secure-http` neither refuses nor warns).
    fn offline(&self) -> bool {
        match self {
            Cli::Install(a) => a.offline,
            Cli::Update(a) => a.offline,
            Cli::Remove(a) => a.offline,
            Cli::Require(a) => a.offline,
            Cli::DumpAutoload(_) | Cli::Config(_) => false,
        }
    }

    /// `update`, `require` or `remove` that installs what it resolves (no
    /// `--no-install`, `--no-update` or `--dry-run`).
    fn resolves_then_installs(&self) -> bool {
        match self {
            Cli::Update(a) => !a.no_install && !a.dry_run,
            Cli::Require(a) => !a.no_install && !a.no_update && !a.dry_run,
            Cli::Remove(a) => !a.no_install && !a.no_update && !a.dry_run,
            Cli::Install(_) | Cli::DumpAutoload(_) | Cli::Config(_) => false,
        }
    }

    /// `--run-scripts` (`install` and `dump-autoload` only).
    fn run_scripts(&self) -> bool {
        match self {
            Cli::Install(a) => a.run_scripts,
            Cli::DumpAutoload(a) => a.run_scripts,
            Cli::Update(_) | Cli::Remove(_) | Cli::Require(_) | Cli::Config(_) => false,
        }
    }

    fn plugins_enabled(&self) -> bool {
        !match self {
            Cli::Install(a) => a.no_plugins,
            Cli::DumpAutoload(a) => a.no_plugins,
            Cli::Update(a) => a.no_plugins,
            Cli::Remove(a) => a.no_plugins,
            Cli::Require(a) => a.no_plugins,
            Cli::Config(_) => true,
        }
    }

    /// Why the command goes to Composer because of the cache directory, if
    /// it must: what `fetch::cache_dir_hand_over` names, a cache directory
    /// set in the project's composer.json, and a relative
    /// cache directory under `--working-dir`, which Composer resolves from
    /// there (`Config::realpath`, after `Application::doRun`'s `chdir`)
    /// where vivacity, which never changes directory, would not.
    /// `dump-autoload` reads no cache; `config` answers on its own.
    /// [`run_once`]'s whole-command hand-over: `COMPOSER=<other file>`
    /// (every command reads composer.json).
    fn unsupported_hand_over(&self) -> Option<String> {
        if let Some(f) = std::env::var("COMPOSER").ok().map(|f| f.trim().to_owned()) {
            if !f.is_empty() && f != "composer.json" && f != "./composer.json" {
                return Some(format!("COMPOSER={f} (an alternate manifest)"));
            }
        }
        None
    }

    /// `update-with-minimal-changes` for `update`, `require` and `remove`
    /// (`$input->getOption('minimal-changes') ||
    /// $config->get('update-with-minimal-changes')`): the option,
    /// `COMPOSER_MINIMAL_CHANGES` (`(bool) getenv`), or the key as
    /// `Factory` merges it under `config`, the last layer that sets it
    /// winning — the global config.json, the global auth.json,
    /// COMPOSER_AUTH, composer.json, the project's auth.json, COMPOSER_AUTH
    /// again. The key is typed boolean in the schema: anything else fails
    /// composer.json's validation and draws a warning from the other
    /// layers', so the run is Composer's too (`"false"` included).
    /// `--no-update` resolves nothing: the mode plays no part there — but
    /// Composer validates the layers on every command, so a value that is
    /// not a boolean hands those over too.
    fn minimal_changes(&self, manifest: Option<&serde_json::Value>) -> bool {
        let (flag, no_update) = match self {
            Cli::Require(a) => (a.minimal_changes, a.no_update),
            Cli::Remove(a) => (a.minimal_changes, a.no_update),
            Cli::Update(a) => (a.minimal_changes, false),
            _ => return false,
        };
        let env =
            std::env::var("COMPOSER_MINIMAL_CHANGES").is_ok_and(|v| !v.is_empty() && v != "0");
        if !no_update && (flag || env) {
            return true;
        }
        const KEY: &str = "update-with-minimal-changes";
        // One layer: its boolean, or `Err` for any other value.
        let layer = |config: Option<&serde_json::Value>| -> Result<Option<bool>, ()> {
            match config.and_then(|c| c.get(KEY)) {
                None => Ok(None),
                Some(serde_json::Value::Bool(b)) => Ok(Some(*b)),
                Some(_) => Err(()),
            }
        };
        let env_auth = match vivacity_core::fetch::composer_auth_env(
            std::env::var("COMPOSER_AUTH").ok().as_deref(),
        ) {
            Ok(env) => env,
            // Composer reports it (`startup_hand_over` hands it over first).
            Err(_) => return true,
        };
        let home = vivacity_core::fetch::composer_home();
        // The project's auth.json, kept for the run: the transport's
        // `fetch::Auth::load` reads that copy, not the file again.
        let project = project_dir(self.working_dir()).ok();
        let mut layers: Vec<Result<Option<bool>, ()>> = Vec::with_capacity(6);
        if let Some(home) = &home {
            layers.push(vivacity_core::fetch::with_global_json(
                home,
                "config.json",
                |c| layer(c.and_then(|c| c.get("config"))),
            ));
            layers.push(vivacity_core::fetch::with_global_json(
                home,
                "auth.json",
                layer,
            ));
        }
        layers.push(layer(env_auth.as_ref()));
        layers.push(layer(manifest.and_then(|m| m.get("config"))));
        if let Some(project) = &project {
            layers.push(vivacity_core::fetch::with_global_json(
                project,
                "auth.json",
                layer,
            ));
        }
        layers.push(layer(env_auth.as_ref()));
        let mut value = false;
        for l in layers {
            match l {
                Ok(Some(b)) => value = b,
                Ok(None) => {}
                Err(()) => return true,
            }
        }
        !no_update && value
    }

    /// [`run_once`]'s whole-command hand-overs before anything: the cache
    /// directory settings vivacity does not reproduce (first), then
    /// `disable-tls`, a COMPOSER_AUTH Composer reports, or the
    /// minimal-changes mode (second) — on the files this reads anyway: the
    /// global ones, composer.json for the commands that read the cache
    /// (`dump-autoload` checks its own once it has read it).
    fn startup_hand_over(&self) -> (Option<String>, Option<String>) {
        let reads_cache = match self {
            // `Application::doRun` creates the project's `Composer` to list
            // plugin commands (`getPluginCommands`), so a composer.json's
            // `disable-tls` writes its line there (measured); without one,
            // the global layers alone (broader than Composer, which then
            // creates only a global `Composer`, if any).
            Cli::Config(_) => {
                let manifest = project_dir(self.working_dir())
                    .ok()
                    .and_then(|p| std::fs::read_to_string(p.join("composer.json")).ok())
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
                return (
                    None,
                    vivacity_core::secure_http::disable_tls_reason(
                        manifest.as_ref().and_then(|m| m.get("config")),
                    )
                    .or_else(|| {
                        crate::fallback::repository_construction_issue(
                            project_dir(self.working_dir()).ok().as_deref(),
                            manifest.as_ref(),
                        )
                    }),
                );
            }
            Cli::DumpAutoload(_) => false,
            Cli::Install(_) | Cli::Update(_) | Cli::Remove(_) | Cli::Require(_) => true,
        };
        let manifest = reads_cache
            .then(|| {
                project_dir(self.working_dir())
                    .ok()
                    .and_then(|p| std::fs::read_to_string(p.join("composer.json")).ok())
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            })
            .flatten();
        let tls = vivacity_core::secure_http::disable_tls_reason(
            manifest.as_ref().and_then(|m| m.get("config")),
        )
        .or_else(|| {
            self.minimal_changes(manifest.as_ref())
                .then(|| "update-with-minimal-changes".to_owned())
        })
        // What creating the `Composer` refuses or warns about in a
        // repository definition (`dump-autoload` checks it once it has read
        // composer.json).
        .or_else(|| {
            reads_cache
                .then(|| {
                    crate::fallback::repository_construction_issue(
                        project_dir(self.working_dir()).ok().as_deref(),
                        manifest.as_ref(),
                    )
                })
                .flatten()
        });
        if let Some(why) = vivacity_core::fetch::cache_dir_hand_over(reads_cache) {
            return (Some(why), tls);
        }
        if let Some(why) = manifest
            .as_ref()
            .and_then(vivacity_core::fetch::project_cache_config)
        {
            return (Some(why), tls);
        }
        let cache = (reads_cache
            && self.working_dir().is_some()
            && vivacity_core::fetch::cache_dir_is_relative())
        .then(|| "a relative cache-dir with --working-dir".to_owned());
        (cache, tls)
    }
}

fn dispatch(cli: Cli) -> anyhow::Result<i32> {
    match cli {
        Cli::Install(args) => run_install(&args),
        Cli::DumpAutoload(args) => run_dump(&args),
        Cli::Update(args) => run_update(&args),
        Cli::Remove(args) => run_remove(&args),
        Cli::Require(args) => require::run_require(&args),
        Cli::Config(args) => run_config(&args),
    }
}

/// Absolute project root: every relative path written into vendor/ (bin
/// proxies, install-path) derives from it and must not depend on the
/// current directory.
fn project_dir(working_dir: Option<&std::path::Path>) -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir().context("cannot determine the current directory")?;
    Ok(match working_dir {
        Some(d) if d.is_absolute() => d.to_path_buf(),
        Some(d) => cwd.join(d),
        None => cwd,
    })
}
