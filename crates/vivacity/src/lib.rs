//! vivacity: a fast, Composer-compatible installer.
//! stdout: nothing for now (reserved for machine-readable output); all
//! narrative goes to stderr. Exit codes: 0 ok, 1 runtime error,
//! 2 usage (clap), 3 out of scope with no fallback possible, 4 platform.
//!
//! The `vivacity` binary is just a call to [`run`]: another program can
//! embed the commands as they are (`vivacity::run(["vivacity", "install", …])`)
//! and get the same exit code.

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
mod dump;
use dump::*;
mod remove;
use remove::*;
mod flex;
mod require;
mod scripts;

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
    /// Internal: set once the fallback has run Composer, so that the
    /// command around this install does not print over Composer's own
    /// report (`InstallPhase`).
    #[arg(skip)]
    handed_over: std::cell::Cell<bool>,
    /// Internal: install following a resolution (`doInstall` with
    /// `alreadySolved`); the lock pool has already been filtered.
    #[arg(skip)]
    after_update: bool,
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
    // before anything is read or written: `-vv`/`-vvv`, and `-v` when an
    // emulated plugin whose output changes with it takes part.
    // `--check-scope` asks what vivacity would do with the lock, not for a
    // run: it is answered natively whatever the verbosity.
    let tail = &argv[1..];
    let check_scope = tail.iter().any(|t| t == "--check-scope");
    if requested >= 3 && !check_scope {
        return delegate_for_verbosity(tail, "-vv/-vvv, Composer's debug output");
    }
    if requested == 2 && !check_scope {
        if let Ok(project) = project_dir(cli.working_dir()) {
            if let Some(plugin) = active_plugin(&project, cli.plugins_enabled()) {
                return delegate_for_verbosity(
                    tail,
                    &format!("-v while {plugin} is active (its listeners' lines)"),
                );
            }
        }
    }
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            vivacity_core::say_always!("Error: {e:?}");
            exit_code_of(&e)
        }
    }
}

/// A session error on its way to the CLI: a transport failure keeps its nature
/// so the exit code is Composer's 100, everything else reads as before.
fn session_error(e: vivacity_resolver::session::SessionError) -> anyhow::Error {
    if e.kind == vivacity_resolver::session::SessionErrorKind::Transport {
        anyhow::Error::new(TransportFailure(e.message))
    } else {
        anyhow::anyhow!("{}", e.message)
    }
}

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
            Some(vivacity_core::Error::Http { .. })
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
        }
    }

    fn plugins_enabled(&self) -> bool {
        !match self {
            Cli::Install(a) => a.no_plugins,
            Cli::DumpAutoload(a) => a.no_plugins,
            Cli::Update(a) => a.no_plugins,
            Cli::Remove(a) => a.no_plugins,
            Cli::Require(a) => a.no_plugins,
        }
    }
}

fn dispatch(cli: Cli) -> anyhow::Result<i32> {
    match cli {
        Cli::Install(args) => run_install(&args),
        Cli::DumpAutoload(args) => run_dump(&args),
        Cli::Update(args) => run_update(&args),
        Cli::Remove(args) => run_remove(&args),
        Cli::Require(args) => require::run_require(&args),
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
