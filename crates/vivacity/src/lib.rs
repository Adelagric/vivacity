//! vivacity: a fast, Composer-compatible installer.
//! stdout: nothing for now (reserved for machine-readable output); all
//! narrative goes to stderr. Exit codes: 0 ok, 1 runtime error,
//! 2 usage (clap), 3 out of scope with no fallback possible, 4 platform.
//!
//! The `vivacity` binary is just a call to [`run`]: another program can
//! embed the commands as they are (`vivacity::run(["vivacity", "install", …])`)
//! and get the same exit code.

mod extension_installers;
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
    /// Packages to update (the others stay locked); `vendor/*` patterns accepted.
    #[arg(value_name = "PACKAGES")]
    packages: Vec<String>,
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
    #[arg(skip)]
    spawn_fallback: bool,
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
    #[arg(long, value_name = "DIR")]
    working_dir: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct InstallArgs {
    /// Do not install require-dev packages.
    #[arg(long)]
    no_dev: bool,
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
    /// Internal: run `composer install` as a subprocess instead of
    /// replacing the process (when the caller still has work to do after).
    /// Off Unix there is no `exec()`: the subprocess is the only mode, and
    /// this field is never read (hence the `allow` — callers still set it
    /// so there is a single construction flow).
    #[arg(skip)]
    #[cfg_attr(not(unix), allow(dead_code))]
    spawn_fallback: bool,
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
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(e) => {
            // `e.exit()` writes the help to stdout, the error to stderr.
            let _ = e.print();
            return e.exit_code();
        }
    };
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:?}");
            1
        }
    }
}

static ARGV: std::sync::Mutex<Vec<std::ffi::OsString>> = std::sync::Mutex::new(Vec::new());

/// The resolution commands' fallback (plan v0.16, decision 1), symmetric
/// to `install`'s: an installed, allowed plugin that changes what the
/// command resolves or writes, and that vivacity does not emulate for it,
/// hands the whole command to Composer with the same arguments — before
/// any write (the caller reverts its manifest edit first). `--no-fallback`
/// stops with exit 3 instead. `Some(code)` when the command was handled
/// here; `None` when vivacity goes on natively.
fn resolution_fallback(
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
fn delegate_resolution(
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

fn run_install(args: &InstallArgs) -> anyhow::Result<i32> {
    if args.no_install {
        eprintln!("Invalid option \"--no-install\". Use \"composer update --no-install\" instead if you are trying to update the composer.lock file.");
        return Ok(1);
    }
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    let manifest_path = project.join("composer.json");
    let lock_path = project.join("composer.lock");

    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("cannot read {}", manifest_path.display()))?;
    let manifest: serde_json::Value = match &args.virtual_manifest {
        Some(v) => v.clone(),
        None => serde_json::from_str(&manifest_text).context("invalid composer.json")?,
    };
    if args.virtual_lock.is_none() && !lock_path.is_file() {
        anyhow::bail!(
            "no composer.lock in {} — `install` needs one; run `vivacity update` \
             to resolve and write it (Composer's `install` resolves instead, \
             which vivacity does not do)",
            project.display()
        );
    }
    // A dry-run update installs from its unwritten lock.
    let lock_text: String = match &args.virtual_lock {
        Some(v) => {
            vivacity_core::phpjson::php_json_encode_with(v, vivacity_core::phpjson::FLAGS_JSONFILE)?
        }
        None => std::fs::read_to_string(&lock_path)
            .with_context(|| format!("cannot read {}", lock_path.display()))?,
    };
    // Parsed once: the `Value` serves the transaction, the abandoned
    // warnings and the missing-requirement check; the `Lock` everything else.
    let lock_value: serde_json::Value =
        serde_json::from_str(&lock_text).context("invalid composer.lock")?;
    let lock = vivacity_core::lock::Lock::from_value(&lock_value);
    trace("read manifests", t0);

    let with_dev = !args.no_dev && std::env::var("COMPOSER_NO_DEV").as_deref() != Ok("1");
    // `wikimedia/composer-merge-plugin`: the root package Composer works
    // with is the merged one (INIT, before any output — a merge error
    // fails Composer inside Factory::create). Everything below reads the
    // merged manifest; the content-hash keeps reading the file.
    let merged = match merge_plugin_root(&project, &lock, &manifest, with_dev, !args.no_plugins) {
        Ok(m) => m,
        Err(reason) => {
            let report = vivacity_core::scope::ScopeReport {
                issues: vec![vivacity_core::scope::ScopeIssue::MergePlugin(reason)],
                ..Default::default()
            };
            return fallback_or_fail(args, &project, &report);
        }
    };
    let manifest = match &merged {
        Some(m) => m.manifest.clone(),
        None => manifest,
    };
    // `--run-scripts`: the declared events go to `composer run-script` at
    // Composer's own points; `pre-install-cmd` fires before anything
    // (before the headline and the lock validation, Installer::run).
    let runner = if args.run_scripts && !args.no_scripts {
        match scripts::Runner::new(&project, &manifest, with_dev, which_composer()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("vivacity: {e}");
                return Ok(3);
            }
        }
    } else {
        None
    };
    if let Some(r) = &runner {
        let code = r.run(scripts::PRE_INSTALL_CMD)?;
        if code != 0 {
            return Ok(code);
        }
    }
    // bamarni/composer-bin-plugin's `COMMAND` listener (the plugin
    // already installed: loaded from installed.json before the command).
    if vivacity_core::scope::active_plugins(&project, &manifest, !args.no_plugins)
        .iter()
        .any(|p| p == vivacity_core::bamarni_bin::PLUGIN_NAME)
    {
        if let Ok(cfg) = vivacity_core::bamarni_bin::config(manifest.get("extra")) {
            vivacity_core::bamarni_bin::print_deprecations(&cfg);
        }
    }
    let config_lock = config_lock_enabled(&manifest_text);
    // `Installer::doInstall`: the headline, then the platform verification
    // notice (the lock is solved against the platform when not coming
    // straight from an update), then the freshness warning.
    if config_lock {
        eprintln!(
            "Installing dependencies from lock file{}",
            if with_dev {
                " (including require-dev)"
            } else {
                ""
            }
        );
    }
    if !args.after_update {
        eprintln!("Verifying lock file contents can be installed on current platform.");
        // Lock freshness: same behaviour as Composer, a warning.
        if let (Ok(actual), Some(expected)) = (
            vivacity_core::content_hash::content_hash(&manifest_text),
            lock.content_hash.as_deref(),
        ) {
            if actual != expected {
                eprintln!(
                    "Warning: The lock file is not up to date with the latest changes in composer.json. \
                     You may be getting outdated dependencies. It is recommended that you run `composer update` or `composer update <package name>`."
                );
            }
        }
    }

    // `Installer::doInstall` runs the lock pool through the list filter in
    // install scope: a flagged locked version (malware list) is not
    // installed. One conditional request per install (Composer's
    // `loadFilterSummary`, `packages.json` cached 600 s), the only network
    // wait of an install from lock: it runs on its own thread while the
    // platform check, the transaction and the scope are computed, and is
    // joined before anything is printed or written past this point — the
    // output order and the decision order are Composer's.
    let policy_thread = if args.after_update {
        None
    } else {
        let http = http_transport(&project, args.offline)?;
        let cache_repo_dir = vivacity_core::fetch::composer_cache_dir().join("repo");
        let composer_home = vivacity_core::fetch::composer_home();
        let project_dir = project.clone();
        let no_blocking = args.no_blocking || args.no_security_blocking;
        Some(std::thread::spawn(move || {
            vivacity_resolver::session::install_policy_problems(
                &project_dir,
                composer_home.as_deref(),
                Some(http),
                Some(&cache_repo_dir),
                with_dev,
                no_blocking,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
        }))
    };

    // Platform (computed now, reported after the policy join).
    let mut ignored = args.ignore_platform_req.clone();
    if args.ignore_platform_reqs {
        ignored.push("*".to_owned());
    }
    let platform_outcome: PlatformOutcome = if ignored.iter().all(|p| p != "*") {
        // The full platform repository (php, extensions, libraries, the
        // composer-*-api packages, `config.platform`), probed through php
        // like `update` does; without php, a warning.
        match vivacity_resolver::platform::probe().ok() {
            Some(probed) => {
                let empty = serde_json::Map::new();
                let overrides = manifest
                    .get("config")
                    .and_then(|c| c.get("platform"))
                    .and_then(serde_json::Value::as_object)
                    .unwrap_or(&empty);
                let platform = vivacity_resolver::platform::platform_packages(&probed, overrides)
                    .map_err(|e| anyhow::anyhow!("{}", e.0))?;
                PlatformOutcome::Failures(vivacity_resolver::platform::check_install(
                    &lock, &platform, with_dev, &ignored,
                ))
            }
            None => PlatformOutcome::NoPhp {
                warn: !lock.platform.is_empty() || (with_dev && !lock.platform_dev.is_empty()),
            },
        }
    } else {
        PlatformOutcome::Failures(Vec::new())
    };
    trace("platform check", t0);
    // `LocalRepoTransaction` (installed.json against the lock): the
    // `Package operations` summary, and in a dry run the operation lines
    // in transaction order, then the abandoned warnings of `Installer::run`.
    let mut arena: Vec<vivacity_resolver::package::Package> = Vec::new();
    // The local repository: installed.json purged of the packages whose
    // install path is gone (`Factory::purgePackages`).
    let installed_now = installed_packages(&project, &manifest);
    trace("installed.json", t0);
    // Nothing to do at all (the commonest install): proved on the raw
    // entries, so the two repositories are not loaded into the arena —
    // ~8 ms of constraint parsing on a 100-package lock. `None` falls back
    // to the full computation.
    let unchanged_names = unchanged_local_repository(&installed_now, &lock_value, with_dev);
    let (present, result) = if unchanged_names.is_some() {
        (Vec::new(), Vec::new())
    } else {
        let as_lock = serde_json::json!({"packages": installed_now, "packages-dev": []});
        let present =
            vivacity_resolver::repository::locked_repository_with(&as_lock, &mut arena, true)
                .map_err(|e| anyhow::anyhow!("installed.json: {}", e.0))?;
        let result = vivacity_resolver::repository::locked_repository_with(
            &lock_value,
            &mut arena,
            with_dev,
        )
        .map_err(|e| anyhow::anyhow!("composer.lock: {}", e.0))?;
        (present, result)
    };
    // `Locker::getMissingRequirementInfo`: a root requirement the lock does
    // not satisfy (a hand-edited composer.json) stops here with code 4.
    let merged_links = merged
        .as_ref()
        .map(|m| (m.requires.as_slice(), m.requires_dev.as_slice()));
    let root_pkg = vivacity_core::state::RootPackage::detect(&manifest, &project, with_dev);
    let missing = missing_requirement_info(
        &manifest,
        &lock_value,
        with_dev,
        &root_pkg.version,
        &root_pkg.pretty_version,
        merged_links,
    )?;
    trace("local repositories", t0);
    let transaction = vivacity_resolver::transaction::Transaction::new(&arena, &present, &result);
    // The scope analysis is pure (lock, manifest, installed.json, global
    // config): computed here, reported after the join.
    let scope = if args.dry_run {
        None
    } else {
        Some(vivacity_core::scope::analyze(
            &project,
            &lock,
            &manifest,
            with_dev,
            !args.no_plugins,
        ))
    };

    trace("scope analysis", t0);
    // The autoloader, planned during the network wait when the install has
    // nothing to place (every wanted package present, same identity): its
    // inputs are then all known — the local repository the install would
    // produce, the manifest, the layout — and `plan` writes nothing under
    // vendor/. Kept as a `Result` and only looked at where the dump runs
    // today, so an error surfaces at the same point with the same text.
    // Not with `--run-scripts` (pre-autoload-dump may edit sources).
    let early_dump: Option<anyhow::Result<PlannedDump>> = if args.dry_run
        || args.no_autoloader
        || runner.is_some()
        || !transaction.operations.is_empty()
    {
        None
    } else {
        scope
            .as_ref()
            .filter(|s| s.is_native_ok())
            .and_then(|s| s.layout.as_ref())
            .and_then(|layout| {
                vivacity_core::installer::local_repository_if_unchanged(&lock, layout, with_dev)
                    .map(|local| {
                        dump_autoload_plan(
                            &project,
                            &local,
                            &manifest,
                            layout,
                            with_dev,
                            args.optimize_autoloader || args.classmap_authoritative,
                            args.classmap_authoritative,
                            args.ignore_platform_reqs,
                            &args.ignore_platform_req,
                            (args.apcu_autoloader, args.apcu_autoloader_prefix.as_deref()),
                            !args.no_plugins,
                            lock.packages_dev
                                .iter()
                                .map(|p| p.name().to_owned())
                                .collect(),
                        )
                    })
            })
    };
    trace("autoload planned", t0);

    // Join: policy first (exit 2), then the platform (exit 4), then the
    // missing requirements, in `Installer::doInstall`'s order.
    if let Some(handle) = policy_thread {
        let (problems, warnings) = handle
            .join()
            .map_err(|_| anyhow::anyhow!("the policy check thread panicked"))??;
        for w in &warnings {
            eprintln!("{w}");
        }
        if !problems.is_empty() {
            // `Installer::doInstall`: the headline, then
            // `SolverProblemsException::getPrettyString` (problems
            // deduplicated and numbered, each ending with a newline).
            eprintln!("Your lock file does not contain a compatible set of packages. Please run composer update.");
            let mut text = String::from("\n");
            let mut seen: Vec<&String> = Vec::new();
            for p in &problems {
                if seen.contains(&p) {
                    continue;
                }
                seen.push(p);
                text.push_str(&format!("  Problem {}\n    {p}\n", seen.len()));
            }
            eprintln!("{text}");
            return Ok(2);
        }
        trace("policy", t0);
    }
    match platform_outcome {
        PlatformOutcome::Failures(failures) if !failures.is_empty() => {
            eprintln!("Your lock file cannot be installed on this platform:");
            for f in &failures {
                let by = f
                    .required_by
                    .as_deref()
                    .map(|p| format!(" (required by {p})"))
                    .unwrap_or_default();
                eprintln!(
                    "  - {} {}{}: {:?}",
                    f.requirement, f.constraint, by, f.reason
                );
            }
            eprintln!("Use --ignore-platform-req=<req> or --ignore-platform-reqs to bypass.");
            return Ok(4);
        }
        PlatformOutcome::NoPhp { warn: true } => {
            eprintln!(
                "Warning: php not found, platform requirements were not checked \
                 (--ignore-platform-reqs silences this warning)"
            );
        }
        _ => {}
    }
    if !missing.is_empty() {
        // On a bare vendor the plugin is not active yet in Composer's run:
        // it would install itself, then run a partial `composer update` of
        // the merged requirements and rewrite the lock. vivacity does not
        // do that on a user's behalf: the lock goes to Composer, with the
        // reason. With the plugin installed, Composer refuses like this.
        if let Some(m) = &merged {
            let plugin_installed = installed_packages(&project, &manifest).iter().any(|p| {
                p.get("name").and_then(|n| n.as_str())
                    == Some(vivacity_resolver::merge_plugin::PLUGIN_NAME)
            });
            if !plugin_installed {
                let report = vivacity_core::scope::ScopeReport {
                    issues: vec![vivacity_core::scope::ScopeIssue::MergePlugin(format!(
                        "the lock does not satisfy the merged requirements ({}); Composer would run the plugin's implicit update and rewrite composer.lock",
                        m.merged_names.join(", ")
                    ))],
                    ..Default::default()
                };
                return fallback_or_fail(args, &project, &report);
            }
        }
        for line in &missing {
            eprintln!("{line}");
        }
        if !require::truthy(require::config_value(
            &manifest,
            "allow-missing-requirements",
        )) {
            return Ok(4);
        }
    }
    {
        use vivacity_resolver::transaction::Operation;
        let ops = &transaction.operations;
        let installs = ops
            .iter()
            .filter(|o| matches!(o, Operation::Install(_)))
            .count();
        let updates = ops
            .iter()
            .filter(|o| matches!(o, Operation::Update(..)))
            .count();
        let removals = ops
            .iter()
            .filter(|o| matches!(o, Operation::Uninstall(_)))
            .count();
        if installs + updates + removals == 0 {
            eprintln!("Nothing to install, update or remove");
        } else {
            eprintln!(
                "Package operations: {installs} install{}, {updates} update{}, {removals} removal{}",
                if installs == 1 { "" } else { "s" },
                if updates == 1 { "" } else { "s" },
                if removals == 1 { "" } else { "s" }
            );
        }
    }
    if args.dry_run {
        for op in &transaction.operations {
            if let Some(line) = op.show(&arena, false) {
                eprintln!("  - {line}");
            }
        }
        // `Installer::run` after `doInstall` (an update prints its own
        // report after this phase).
        if !args.after_update {
            for line in abandoned_warnings(&lock_value) {
                eprintln!("{line}");
            }
            print_funding(&project, &manifest);
        }
        return Ok(0);
    }

    // Out of scope: exec composer as fallback (default) or fail explicitly.
    let Some(scope) = scope else {
        anyhow::bail!("internal: scope was not analysed");
    };
    if !scope.is_native_ok() {
        return fallback_or_fail(args, &project, &scope);
    }
    let Some(layout) = scope.layout.as_ref() else {
        anyhow::bail!("internal: scope is native but no layout was resolved");
    };
    if let Some(tag) = &layout.installers_tag {
        eprintln!("Note: composer/installers {tag} emulated natively (custom install paths)");
    }
    for plugin in &scope.skipped_plugins {
        eprintln!(
            "Note: plugin {plugin} installed as a plain library (vivacity never runs plugins)"
        );
    }
    trace("scope", t0);
    if args.check_scope {
        eprintln!("vivacity: this lock is installed natively");
        return Ok(0);
    }

    // The operation lines of a real install, in transaction order, with the
    // downloader's appendix (`getInstallOperationAppendix`: `: Extracting
    // archive`, `: Symlinking from …`, `: Mirroring from …`, `: Source
    // already present`) computed on the state before the transaction, like
    // Composer does right before each operation. They are printed once the
    // transaction has succeeded: the placements run in parallel, and a
    // failure is reported alone.
    let operation_lines = operation_lines(&project, &arena, &transaction.operations, layout)?;

    // Transaction.
    let store = Arc::new(vivacity_core::store::Store::default_location());
    let auth = vivacity_core::fetch::Auth::load(&project);
    let fetcher = Arc::new(vivacity_core::fetch::Fetcher::new(
        vivacity_core::fetch::composer_cache_dir(),
        auth,
    )?);
    let opts = vivacity_core::installer::InstallOptions {
        with_dev,
        offline: args.offline,
        ..Default::default()
    };
    let runtime = tokio::runtime::Runtime::new().context("cannot start the async runtime")?;
    let report = match runtime.block_on(vivacity_core::installer::install(
        &project, &lock, &manifest, layout, store, fetcher, &opts,
    )) {
        Ok(r) => r,
        // Emulation refusal detected before any write: vendor/ is intact.
        Err(vivacity_core::Error::Unsupported(msg)) => {
            let scope = vivacity_core::scope::ScopeReport {
                issues: vec![vivacity_core::scope::ScopeIssue::Layout(msg)],
                skipped_plugins: vec![],
                layout: None,
            };
            return fallback_or_fail(args, &project, &scope);
        }
        Err(vivacity_core::Error::Refused(msg)) => {
            eprintln!("{msg}");
            return Ok(1);
        }
        Err(e) => return Err(e.into()),
    };
    trace("install transaction", t0);
    for line in &operation_lines {
        eprintln!("{line}");
    }
    // `BinaryInstaller` notices belong to the operations, before the dump.
    for m in &report.messages {
        eprintln!("{m}");
    }
    // `Installer::run`: the abandoned packages of the lock (dev included)
    // before the dump, the funding count after it; an install driven by
    // `update` reports through it.
    if !args.after_update {
        for line in abandoned_warnings(&lock_value) {
            eprintln!("{line}");
        }
    }
    let mut autoload_note = String::new();
    if !args.no_autoloader {
        if let Some(r) = &runner {
            let code = r.run(scripts::PRE_AUTOLOAD_DUMP)?;
            if code != 0 {
                return Ok(code);
            }
        }
        // `Installer::doInstall`: "optimized" as soon as the effective
        // optimize flag is on — the option, `config.optimize-autoloader`,
        // or class-map authoritative (which implies it), the same
        // computation `dump_autoload_plan` makes.
        eprintln!(
            "Generating{} autoload files",
            if effective_optimize(
                &manifest,
                args.optimize_autoloader,
                args.classmap_authoritative
            ) {
                " optimized"
            } else {
                ""
            }
        );
        // `AutoloadGenerator::dump($localRepo)`: the local repository, not
        // the lock (they differ for an unchanged package whose lock entry
        // moved).
        let local = report.local_repository.as_ref().unwrap_or(&lock);
        let planned = match early_dump {
            // Planned during the network wait: the install placed nothing,
            // the inputs are what they were.
            Some(planned) => planned?,
            None => dump_autoload_plan(
                &project,
                local,
                &manifest,
                layout,
                with_dev,
                args.optimize_autoloader || args.classmap_authoritative,
                args.classmap_authoritative,
                args.ignore_platform_reqs,
                &args.ignore_platform_req,
                (args.apcu_autoloader, args.apcu_autoloader_prefix.as_deref()),
                !args.no_plugins,
                // `$localRepo->setDevPackageNames($this->locker->getDevPackageNames())`
                lock.packages_dev
                    .iter()
                    .map(|p| p.name().to_owned())
                    .collect(),
            )?,
        };
        let report = planned.finish(&project, &manifest, layout)?;
        autoload_note = format!(", autoloader with {} classes", report.classes);
        // Plugins listening to post-autoload-dump, emulated: the local
        // repository's order is the previous installed.json order minus
        // the removed and updated packages, then the operations' order.
        {
            use vivacity_resolver::transaction::Operation;
            let previous: Vec<&str> = match &unchanged_names {
                Some(names) => names.iter().map(String::as_str).collect(),
                None => present.iter().map(|&i| arena[i].name.as_str()).collect(),
            };
            let mut gone: Vec<&str> = Vec::new();
            let mut fresh: Vec<&str> = Vec::new();
            for op in &transaction.operations {
                match *op {
                    Operation::Uninstall(p) => gone.push(arena[p].name.as_str()),
                    Operation::Update(i, t) => {
                        gone.push(arena[i].name.as_str());
                        fresh.push(arena[t].name.as_str());
                    }
                    Operation::Install(p) => fresh.push(arena[p].name.as_str()),
                    _ => {}
                }
            }
            let order =
                vivacity_core::pest_plugin::local_repository_order(&previous, &gone, &fresh);
            let previously_present = previous.contains(&vivacity_core::pest_plugin::PLUGIN_NAME);
            emulate_pest_plugin(
                &project,
                local,
                &manifest,
                with_dev,
                &order,
                !args.no_plugins,
                previously_present,
            )?;
            // yiisoft/yii2-composer: `activate` then its installer's
            // per-operation rewrites of extensions.php — one write here,
            // when an extension was installed, updated or removed.
            let touched: Vec<&str> = gone.iter().chain(fresh.iter()).copied().collect();
            emulate_yii2_composer(
                &project,
                local,
                &manifest,
                with_dev,
                &order,
                &touched,
                !args.no_plugins,
            )?;
            // codeception/c3: its `uninstall` when the plugin leaves the
            // vendor (during the transaction), its `POST_INSTALL_CMD` /
            // `POST_UPDATE_CMD` copy afterwards.
            emulate_c3(
                &project,
                local,
                &manifest,
                with_dev,
                previous.contains(&vivacity_core::c3_plugin::PLUGIN_NAME),
                args.after_update,
                !args.no_plugins,
            )?;
            emulate_bamarni_bin(local, &manifest, with_dev, !args.no_plugins);
        }
        // `AutoloadGenerator::dump`: post-autoload-dump once the files are
        // written (the emulated post-autoload-dump plugins included).
        if let Some(r) = &runner {
            let code = r.run(scripts::POST_AUTOLOAD_DUMP)?;
            if code != 0 {
                return Ok(code);
            }
        }
        trace("autoload dump", t0);
    }
    // `dealerdirect/phpcodesniffer-composer-installer` listens to
    // post-install-cmd: after the transaction and the dump, plugins on.
    if !args.no_plugins {
        let local = report.local_repository.as_ref().unwrap_or(&lock);
        let installed: Vec<&vivacity_core::lock::LockPackage> =
            local.wanted_packages(with_dev).collect();
        let allowed = matches!(
            vivacity_core::layout::plugin_allowed(
                &manifest,
                vivacity_core::phpcs_installer::PLUGIN_NAME
            ),
            vivacity_core::layout::PluginVerdict::Allowed
        );
        if allowed
            && installed
                .iter()
                .any(|p| p.name() == vivacity_core::phpcs_installer::PLUGIN_NAME)
        {
            vivacity_core::phpcs_installer::register_standards(
                &project, layout, &installed, &manifest,
            )?;
        }
        extension_installers::phpstan(layout, local, &manifest, with_dev, !args.no_plugins)?;
        extension_installers::rector(layout, local, &manifest, with_dev, !args.no_plugins)?;
    }
    if !args.after_update {
        print_funding(&project, &manifest);
    }
    if let Some(r) = &runner {
        let code = r.run(scripts::POST_INSTALL_CMD)?;
        if code != 0 {
            return Ok(code);
        }
    }
    let warmed = if report.store_warmed > 0 {
        format!(", store warmed for {} packages", report.store_warmed)
    } else {
        String::new()
    };
    eprintln!(
        "vivacity: {} installed, {} unchanged, {} removed ({} from store, {} from cache, {} from network){warmed}{autoload_note} in {:.2}s",
        report.installed,
        report.unchanged,
        report.removed,
        report.store_hits,
        report.from_cache,
        report.from_network,
        t0.elapsed().as_secs_f32()
    );
    Ok(0)
}

/// `pestphp/pest-plugin`'s post-autoload-dump listener, emulated: with
/// plugins on and the plugin wanted, `vendor/pest-plugins.json` from the
/// installed packages' `extra.pest.plugins` in local-repository order
/// (`order`, package names); the file goes when the plugin leaves.
fn emulate_pest_plugin(
    project: &std::path::Path,
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    order: &[&str],
    plugins_enabled: bool,
    previously_present: bool,
) -> anyhow::Result<()> {
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .vendor_dir(project);
    let wanted: std::collections::BTreeMap<&str, &vivacity_core::lock::LockPackage> = local
        .wanted_packages(with_dev)
        .map(|p| (p.name(), p))
        .collect();
    // Like Composer: a plugin listed as `false` in allow-plugins is skipped.
    let allowed = matches!(
        vivacity_core::layout::plugin_allowed(manifest, vivacity_core::pest_plugin::PLUGIN_NAME),
        vivacity_core::layout::PluginVerdict::Allowed
    );
    if !plugins_enabled || !allowed || !wanted.contains_key(vivacity_core::pest_plugin::PLUGIN_NAME)
    {
        if previously_present {
            vivacity_core::pest_plugin::remove_pest_plugins(&vendor)?;
        }
        return Ok(());
    }
    let extras: Vec<Option<&serde_json::Value>> = order
        .iter()
        .filter_map(|n| wanted.get(n))
        .map(|p| p.raw.get("extra"))
        .collect();
    vivacity_core::pest_plugin::write_pest_plugins(&vendor, &extras, manifest.get("extra"))?;
    Ok(())
}

/// `yiisoft/yii2-composer`, emulated: with plugins on and the plugin
/// installed and allowed, `vendor/yiisoft/extensions.php` exists
/// (`activate`), and when a `yii2-extension` package was among the
/// operations (`touched`) the map is rewritten from the installed
/// extensions in local-repository order (`order`).
fn emulate_yii2_composer(
    project: &std::path::Path,
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    order: &[&str],
    touched: &[&str],
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_core::yii2_composer as yii;
    let wanted: std::collections::BTreeMap<&str, &vivacity_core::lock::LockPackage> = local
        .wanted_packages(with_dev)
        .map(|p| (p.name(), p))
        .collect();
    let allowed = matches!(
        vivacity_core::layout::plugin_allowed(manifest, yii::PLUGIN_NAME),
        vivacity_core::layout::PluginVerdict::Allowed
    );
    if !plugins_enabled || !allowed || !wanted.contains_key(yii::PLUGIN_NAME) {
        return Ok(());
    }
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .vendor_dir(project);
    yii::ensure_extensions_file(&vendor)?;
    let is_extension = |n: &str| {
        wanted
            .get(n)
            .is_some_and(|p| p.package_type() == yii::EXTENSION_TYPE)
    };
    let extension_touched = touched.iter().any(|n| {
        is_extension(n)
            || local
                .packages
                .iter()
                .chain(local.packages_dev.iter())
                .any(|p| p.name() == *n && p.package_type() == yii::EXTENSION_TYPE)
    });
    if !extension_touched {
        return Ok(());
    }
    let entries: Vec<(&str, serde_json::Value)> = order
        .iter()
        .filter(|n| is_extension(n))
        .map(|n| (*n, yii::extension_entry(wanted[n])))
        .collect();
    yii::write_extensions(&vendor, &entries)?;
    Ok(())
}

/// `codeception/c3`, emulated: with plugins on, `c3.php` copied to the
/// project root after the install (`copyC3V2`; `askForUpdateV2` after an
/// update: no "up-to-date" line), or deleted when the plugin was
/// uninstalled by this run. Messages on stdout, as `$io->write` prints.
fn emulate_c3(
    project: &std::path::Path,
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    previously_present: bool,
    after_update: bool,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_core::c3_plugin as c3;
    if !plugins_enabled {
        return Ok(());
    }
    let allowed = matches!(
        vivacity_core::layout::plugin_allowed(manifest, c3::PLUGIN_NAME),
        vivacity_core::layout::PluginVerdict::Allowed
    );
    let wanted = local
        .wanted_packages(with_dev)
        .any(|p| p.name() == c3::PLUGIN_NAME);
    if !wanted {
        if previously_present && allowed && c3::delete_c3(project)? {
            println!("[codeception/c3] Deleting c3.php from the root of your project...");
        }
        return Ok(());
    }
    if !allowed {
        return Ok(());
    }
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .vendor_dir(project);
    match c3::copy_c3(&vendor, project)? {
        c3::C3Action::UpToDate if !after_update => {
            println!("[codeception/c3] c3.php is already up-to-date");
        }
        c3::C3Action::Copied => {
            println!("[codeception/c3] Copying c3.php to the root of your project...");
            println!("[codeception/c3] Include c3.php into index.php in order to collect codecoverage from server scripts");
        }
        _ => {}
    }
    Ok(())
}

/// bamarni/composer-bin-plugin's `POST_AUTOLOAD_DUMP` listener: the
/// deprecation lines again (the plugin now installed and allowed).
fn emulate_bamarni_bin(
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
) {
    use vivacity_core::bamarni_bin as bin;
    let allowed = matches!(
        vivacity_core::layout::plugin_allowed(manifest, bin::PLUGIN_NAME),
        vivacity_core::layout::PluginVerdict::Allowed
    );
    if !plugins_enabled
        || !allowed
        || !local
            .wanted_packages(with_dev)
            .any(|p| p.name() == bin::PLUGIN_NAME)
    {
        return;
    }
    if let Ok(cfg) = bin::config(manifest.get("extra")) {
        bin::print_deprecations(&cfg);
    }
}

/// `  - <operation><appendix>` for every operation of a real install.
fn operation_lines(
    project: &std::path::Path,
    arena: &[vivacity_resolver::package::Package],
    operations: &[vivacity_resolver::transaction::Operation],
    layout: &vivacity_core::layout::Layout,
) -> anyhow::Result<Vec<String>> {
    use vivacity_resolver::transaction::Operation;
    let mut lines = Vec::new();
    for op in operations {
        let Some(shown) = op.show(arena, false) else {
            continue;
        };
        let appendix = match *op {
            Operation::Install(p) | Operation::Update(_, p) => {
                let pkg = &arena[p];
                match pkg.dist.as_ref() {
                    Some(d) if d.kind == "path" => {
                        let install_path = layout
                            .abs(&pkg.name)
                            .unwrap_or_else(|| layout.vendor_dir().join(&pkg.name));
                        match vivacity_core::path_install::install_appendix(
                            project,
                            &install_path,
                            &d.url,
                            pkg.raw.get("transport-options"),
                        ) {
                            Ok(a) => a,
                            Err(vivacity_core::Error::Refused(msg)) => anyhow::bail!("{msg}"),
                            Err(e) => return Err(e.into()),
                        }
                    }
                    Some(_) if pkg.package_type != "metapackage" => {
                        ": Extracting archive".to_owned()
                    }
                    _ => String::new(),
                }
            }
            Operation::Uninstall(p) => {
                let pkg = &arena[p];
                match pkg.dist.as_ref() {
                    Some(d) if d.kind == "path" => {
                        let install_path = layout
                            .abs(&pkg.name)
                            .unwrap_or_else(|| layout.vendor_dir().join(&pkg.name));
                        if vivacity_core::path_install::is_own_source(
                            project,
                            &install_path.to_string_lossy(),
                            &d.url,
                        ) {
                            format!(", source is still present in {}", install_path.display())
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };
        lines.push(format!("  - {shown}{appendix}"));
    }
    Ok(lines)
}

/// `config.<key>` of the manifest, as `Config::get` reads a boolean.
fn config_bool(manifest: &serde_json::Value, key: &str) -> bool {
    manifest
        .get("config")
        .and_then(|c| c.get(key))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// `InstallCommand` + `Installer::setClassMapAuthoritative`: the option,
/// `config.optimize-autoloader`, or anything that makes the class map
/// authoritative — which turns optimization on.
fn effective_optimize(manifest: &serde_json::Value, optimize: bool, authoritative: bool) -> bool {
    optimize
        || authoritative
        || config_bool(manifest, "classmap-authoritative")
        || config_bool(manifest, "optimize-autoloader")
}

/// The computing half of the dump (`vivacity_autoload::plan` with the
/// options InstallCommand derives): nothing written under vendor/.
#[allow(clippy::too_many_arguments)]
fn dump_autoload_plan(
    project: &std::path::Path,
    lock: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    dev_mode: bool,
    optimize: bool,
    authoritative: bool,
    ignore_all: bool,
    ignored: &[String],
    apcu: (bool, Option<&str>),
    plugins_enabled: bool,
    dev_package_names: Vec<String>,
) -> anyhow::Result<PlannedDump> {
    let platform_check = match manifest.get("config").and_then(|c| c.get("platform-check")) {
        Some(serde_json::Value::Bool(false)) => vivacity_autoload::PlatformCheckMode::Off,
        Some(serde_json::Value::Bool(true)) => vivacity_autoload::PlatformCheckMode::Full,
        _ => vivacity_autoload::PlatformCheckMode::PhpOnly,
    };
    // Like InstallCommand: the flags OR the composer.json config.
    let cfg_bool = |key: &str| config_bool(manifest, key);
    let authoritative = authoritative || cfg_bool("classmap-authoritative");
    let optimize = effective_optimize(manifest, optimize, authoritative);
    // `$apcu = $apcuPrefix !== null || --apcu-autoloader || config.apcu-autoloader`;
    // without a prefix Composer draws bin2hex(random_bytes(10)).
    let apcu_prefix = if apcu.1.is_some() || apcu.0 || cfg_bool("apcu-autoloader") {
        Some(apcu.1.map(str::to_owned).unwrap_or_else(|| {
            use std::fmt::Write as _;
            let mut s = String::new();
            for b in vivacity_core::random_bytes(10) {
                let _ = write!(s, "{b:02x}");
            }
            s
        }))
    } else {
        None
    };
    let opts = vivacity_autoload::DumpOptions {
        apcu_prefix,
        dev_mode,
        optimize,
        authoritative,
        platform_check,
        ignore_all_platform_reqs: ignore_all || ignored.iter().any(|p| p == "*"),
        ignored_platform_reqs: ignored.to_vec(),
        suffix: None,
        dev_package_names,
        classmap_cache: if std::env::var_os("VIVACITY_NO_CLASSMAP_CACHE").is_some() {
            None
        } else {
            Some(vivacity_autoload::ClassmapCacheConfig {
                store_root: vivacity_core::platform::cache_dir().join("store"),
                cache_root: vivacity_core::platform::cache_dir(),
            })
        },
    };
    let (plan, report) = vivacity_autoload::plan(project, lock, manifest, layout, &opts)?;
    Ok(PlannedDump {
        plan,
        report,
        runtime_stub: plugins_enabled
            && lock
                .wanted_packages(dev_mode)
                .any(|p| p.name() == "symfony/runtime")
            && matches!(
                vivacity_core::layout::plugin_allowed(manifest, "symfony/runtime"),
                vivacity_core::layout::PluginVerdict::Allowed
            ),
    })
}

/// A dump computed but not written: `finish` writes it (byte-compare per
/// file), the `symfony/runtime` stub after it, and prints the warnings.
struct PlannedDump {
    plan: vivacity_autoload::DumpPlan,
    report: vivacity_autoload::DumpReport,
    runtime_stub: bool,
}

impl PlannedDump {
    fn finish(
        self,
        project: &std::path::Path,
        manifest: &serde_json::Value,
        layout: &vivacity_core::layout::Layout,
    ) -> anyhow::Result<vivacity_autoload::DumpReport> {
        self.plan.commit()?;
        // `symfony/runtime`'s plugin writes vendor/autoload_runtime.php on
        // POST_AUTOLOAD_DUMP: at dump time, with plugins on, never with
        // --no-autoloader.
        if self.runtime_stub {
            vivacity_core::runtime_stub::write_stub(&layout.vendor_dir(), project, manifest)?;
        }
        for w in &self.report.warnings {
            eprintln!("{w}");
        }
        Ok(self.report)
    }
}

/// `dump_autoload_plan` then `finish`: the dump as a single step.
#[allow(clippy::too_many_arguments)]
fn dump_autoload(
    project: &std::path::Path,
    lock: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    dev_mode: bool,
    optimize: bool,
    authoritative: bool,
    ignore_all: bool,
    ignored: &[String],
    apcu: (bool, Option<&str>),
    plugins_enabled: bool,
    dev_package_names: Vec<String>,
) -> anyhow::Result<vivacity_autoload::DumpReport> {
    dump_autoload_plan(
        project,
        lock,
        manifest,
        layout,
        dev_mode,
        optimize,
        authoritative,
        ignore_all,
        ignored,
        apcu,
        plugins_enabled,
        dev_package_names,
    )?
    .finish(project, manifest, layout)
}

/// `wikimedia/composer-merge-plugin` on this run: `Some` when the plugin is
/// locked (wanted for the mode), allowed, plugins enabled and a merge
/// configuration declared — the merged root (`extra.branch-alias` kept
/// from the original: `RootPackageLoader` reads it before the plugin
/// runs). `Err`: the reason vivacity cannot emulate this project.
fn merge_plugin_root(
    project: &std::path::Path,
    lock: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> Result<Option<vivacity_resolver::merge_plugin::Merged>, String> {
    use vivacity_resolver::merge_plugin;
    // `PluginManager::loadInstalledPlugins` activates the plugin from
    // installed.json whatever the mode (a dev-only plugin still merges at
    // INIT on an `install --no-dev` over a dev vendor), and an install
    // from the lock activates it as it lands.
    let present = lock
        .wanted_packages(with_dev)
        .any(|p| p.name() == merge_plugin::PLUGIN_NAME)
        || installed_packages(project, manifest)
            .iter()
            .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(merge_plugin::PLUGIN_NAME));
    merge_root_when(project, manifest, with_dev, plugins_enabled && present)
}

/// The merge itself, once the caller knows whether Composer would load
/// the plugin on this run (`present`): `None` unless it is allowed and a
/// merge configuration is declared.
fn merge_root_when(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    with_dev: bool,
    present: bool,
) -> Result<Option<vivacity_resolver::merge_plugin::Merged>, String> {
    use vivacity_resolver::merge_plugin::{self, Settings};
    if !present
        || !matches!(
            vivacity_core::layout::plugin_allowed(manifest, merge_plugin::PLUGIN_NAME),
            vivacity_core::layout::PluginVerdict::Allowed
        )
        || !Settings::declared(manifest)
    {
        return Ok(None);
    }
    let root = vivacity_core::state::RootPackage::detect(manifest, project, with_dev);
    let mut merged = merge_plugin::merge(
        project,
        manifest,
        &root.version,
        &root.pretty_version,
        with_dev,
    )?;
    let original_alias = manifest
        .get("extra")
        .and_then(|e| e.get("branch-alias"))
        .cloned();
    if let Some(extra) = merged
        .manifest
        .get_mut("extra")
        .and_then(|e| e.as_object_mut())
    {
        match original_alias {
            Some(a) => {
                extra.insert("branch-alias".to_owned(), a);
            }
            None => {
                extra.remove("branch-alias");
            }
        }
    }
    Ok(Some(merged))
}

/// composer-merge-plugin required, allowed and configured but not
/// installed yet: Composer does not load it (`PluginManager` reads
/// installed.json), resolves the root unmerged, then — once the plugin
/// lands — runs the plugin's implicit `update` of the merged names, a
/// second resolution. Not emulated: refused with the reason.
fn refuse_merge_plugin_resolution(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_resolver::merge_plugin::{self, Settings};
    let required = ["require", "require-dev"].iter().any(|k| {
        manifest
            .get(k)
            .and_then(|m| m.get(merge_plugin::PLUGIN_NAME))
            .is_some()
    });
    let installed = vivacity_core::scope::active_plugins(project, manifest, plugins_enabled)
        .iter()
        .any(|p| p == merge_plugin::PLUGIN_NAME);
    if plugins_enabled
        && required
        && !installed
        && matches!(
            vivacity_core::layout::plugin_allowed(manifest, merge_plugin::PLUGIN_NAME),
            vivacity_core::layout::PluginVerdict::Allowed
        )
        && Settings::declared(manifest)
    {
        anyhow::bail!(
            "wikimedia/composer-merge-plugin is not installed yet: Composer would resolve without the merge, then run the plugin's implicit update once it is installed (not emulated) — run `composer update` for this project"
        );
    }
    Ok(())
}

/// `<vendor-dir>/composer` of a project before its layout is resolved
/// (reading installed.json): the configured directory, or `vendor/composer`
/// when the configuration is one vivacity refuses (the layout will say so).
fn composer_dir_of(project: &std::path::Path, manifest: &serde_json::Value) -> std::path::PathBuf {
    vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .composer_dir(project)
}

fn run_dump(args: &DumpArgs) -> anyhow::Result<i32> {
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(project.join("composer.json"))
            .context("cannot read composer.json")?,
    )
    .context("invalid composer.json")?;
    let lock = vivacity_core::lock::Lock::read(&project.join("composer.lock"))?;
    // bamarni/composer-bin-plugin's `COMMAND` listener.
    if vivacity_core::scope::active_plugins(&project, &manifest, !args.no_plugins)
        .iter()
        .any(|p| p == vivacity_core::bamarni_bin::PLUGIN_NAME)
    {
        if let Ok(cfg) = vivacity_core::bamarni_bin::config(manifest.get("extra")) {
            vivacity_core::bamarni_bin::print_deprecations(&cfg);
        }
    }
    // Dev mode: that of the installed state (installed.json), like Composer.
    let installed_json =
        vivacity_core::jsonfile::read(&composer_dir_of(&project, &manifest).join("installed.json"));
    let installed_dev = installed_json
        .as_ref()
        .and_then(|v| v.get("dev").and_then(serde_json::Value::as_bool))
        .unwrap_or(true);
    // `$localRepo->getDevPackageNames()`: installed.json's list.
    let installed_dev_package_names: Vec<String> = installed_json
        .as_ref()
        .and_then(|v| {
            v.get("dev-package-names")
                .and_then(serde_json::Value::as_array)
        })
        .map(|a| {
            a.iter()
                .filter_map(|n| n.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let dev_mode = !args.no_dev && installed_dev;
    // The merge plugin's PRE_AUTOLOAD_DUMP merge, with the dump's dev mode.
    let manifest = match merge_plugin_root(&project, &lock, &manifest, dev_mode, !args.no_plugins) {
        Ok(Some(m)) => m.manifest,
        Ok(None) => manifest,
        Err(reason) => {
            eprintln!("vivacity: this lock is outside what vivacity handles natively:");
            eprintln!("  - wikimedia/composer-merge-plugin: {reason}");
            eprintln!("Run `composer dump-autoload` instead.");
            return Ok(3);
        }
    };
    let layout = match vivacity_core::layout::Layout::resolve(
        &project,
        &lock,
        &manifest,
        dev_mode,
        !args.no_plugins,
    ) {
        Ok(l) => l,
        Err(issues) => {
            eprintln!("vivacity: this lock is outside what vivacity handles natively:");
            for i in &issues {
                eprintln!("  - {i}");
            }
            eprintln!("Run `composer dump-autoload` instead.");
            return Ok(3);
        }
    };
    // Composer runs the PRE_AUTOLOAD_DUMP listeners of every installed plugin
    // (drupal/core-composer-scaffold adds classmap entries and writes
    // vendor/drupal/DrupalInstalled.php): a plugin vivacity neither emulates
    // nor knows to be inert makes the dump non-reproducible.
    if !args.no_plugins {
        let issues = vivacity_core::scope::plugin_issues(&lock, installed_dev);
        if !issues.is_empty() {
            eprintln!("vivacity: this lock is outside what vivacity handles natively:");
            for i in &issues {
                eprintln!("  - {i}");
            }
            eprintln!("Run `composer dump-autoload` instead.");
            return Ok(3);
        }
    }
    let installed_order: Vec<String> = installed_packages(&project, &manifest)
        .iter()
        .filter_map(|p| {
            p.get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    // `DumpAutoloadCommand`: the two autoload events around the dump.
    let runner = if args.run_scripts {
        match scripts::Runner::new(&project, &manifest, dev_mode, which_composer()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("vivacity: {e}");
                return Ok(3);
            }
        }
    } else {
        None
    };
    if let Some(r) = &runner {
        let code = r.run(scripts::PRE_AUTOLOAD_DUMP)?;
        if code != 0 {
            return Ok(code);
        }
    }
    let report = dump_autoload(
        &project,
        &lock,
        &manifest,
        &layout,
        dev_mode,
        args.optimize || args.classmap_authoritative,
        args.classmap_authoritative,
        args.ignore_platform_reqs,
        &args.ignore_platform_req,
        (args.apcu_autoloader, args.apcu_autoloader_prefix.as_deref()),
        !args.no_plugins,
        installed_dev_package_names,
    )?;
    {
        let order: Vec<&str> = installed_order.iter().map(String::as_str).collect();
        let previously_present = order.contains(&vivacity_core::pest_plugin::PLUGIN_NAME);
        emulate_pest_plugin(
            &project,
            &lock,
            &manifest,
            dev_mode,
            &order,
            !args.no_plugins,
            previously_present,
        )?;
        // yii2-composer's `activate` alone: no installer operation on a dump.
        emulate_yii2_composer(
            &project,
            &lock,
            &manifest,
            dev_mode,
            &order,
            &[],
            !args.no_plugins,
        )?;
        emulate_bamarni_bin(&lock, &manifest, dev_mode, !args.no_plugins);
    }
    if let Some(r) = &runner {
        let code = r.run(scripts::POST_AUTOLOAD_DUMP)?;
        if code != 0 {
            return Ok(code);
        }
    }
    eprintln!(
        "vivacity: autoloader generated ({} classes) in {:.2}s",
        report.classes,
        t0.elapsed().as_secs_f32()
    );
    Ok(0)
}

fn fallback_or_fail(
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

fn which_composer() -> Option<PathBuf> {
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

/// `composer update`: resolution (exact port of Composer's solver), lock
/// written if its data changes, then `install`.
/// Network transport for remote composer repositories: the vivacity-core
/// Fetcher (Composer auth, retries), made synchronous, with a parallel
/// batch (Composer: curl multi, 12 downloads at a time).
/// What the platform check found, reported after the policy join.
enum PlatformOutcome {
    /// php probed: the requirements the lock cannot satisfy (empty = fine).
    Failures(Vec<vivacity_core::platform::PlatformFailure>),
    /// No php: a warning when the lock has platform requirements to check.
    NoPhp { warn: bool },
}

fn http_transport(
    project: &std::path::Path,
    offline: bool,
) -> anyhow::Result<vivacity_resolver::repository::HttpTransports> {
    let runtime =
        Arc::new(tokio::runtime::Runtime::new().context("cannot start the async runtime")?);
    let fetcher = Arc::new(vivacity_core::fetch::Fetcher::new(
        vivacity_core::fetch::composer_cache_dir(),
        vivacity_core::fetch::Auth::load(project),
    )?);
    fn to_fetched(
        r: vivacity_core::fetch::MetadataResponse,
    ) -> vivacity_resolver::repository::Fetched {
        use vivacity_core::fetch::MetadataResponse as M;
        use vivacity_resolver::repository::Fetched as F;
        match r {
            M::NotModified => F::NotModified,
            M::NotFound => F::NotFound,
            M::Body {
                bytes,
                last_modified,
            } => F::Body {
                bytes,
                last_modified,
            },
        }
    }
    let http: vivacity_resolver::repository::HttpFetch = {
        let runtime = runtime.clone();
        let fetcher = fetcher.clone();
        Arc::new(move |url: &str, ims: Option<&str>| {
            if offline {
                return Err(format!("offline: cannot fetch {url}"));
            }
            runtime
                .block_on(fetcher.metadata_fetch(url, ims))
                .map(to_fetched)
                .map_err(|e| e.to_string())
        })
    };
    // Parallel batch (Composer: curl multi, 12 downloads at a time).
    let http_many: vivacity_resolver::repository::HttpFetchMany = {
        let runtime = runtime.clone();
        let fetcher = fetcher.clone();
        Arc::new(move |requests: &[vivacity_resolver::repository::Request]| {
            if offline {
                return requests
                    .iter()
                    .map(|(u, _)| Err(format!("offline: cannot fetch {u}")))
                    .collect();
            }
            let requests: Vec<(String, Option<String>)> = requests.to_vec();
            runtime.block_on(async {
                let sem = Arc::new(tokio::sync::Semaphore::new(12));
                let tasks: Vec<_> = requests
                    .into_iter()
                    .map(|(u, ims)| {
                        let sem = sem.clone();
                        let fetcher = fetcher.clone();
                        async move {
                            let _permit = sem.acquire().await;
                            fetcher
                                .metadata_fetch(&u, ims.as_deref())
                                .await
                                .map(to_fetched)
                                .map_err(|e| e.to_string())
                        }
                    })
                    .collect();
                futures_join_all(tasks).await
            })
        })
    };
    // Form POST (security advisories API).
    let http_post: vivacity_resolver::repository::HttpPost = {
        let runtime = runtime.clone();
        let fetcher = fetcher.clone();
        Arc::new(move |url: &str, body: &str| {
            if offline {
                return Err(format!("offline: cannot fetch {url}"));
            }
            runtime
                .block_on(fetcher.post_form(url, body))
                .map(to_fetched)
                .map_err(|e| e.to_string())
        })
    };
    Ok((http, Some(http_many), Some(http_post)))
}

fn run_update(args: &UpdateArgs) -> anyhow::Result<i32> {
    // Partial update: `update a/b [-w|-W]` (UpdateCommand).
    let env_flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
    for p in &args.packages {
        if ["lock", "nothing", "mirrors"].contains(&p.as_str()) {
            anyhow::bail!(
                "`vivacity update {p}` (lock file metadata refresh) is not supported yet"
            );
        }
        if p.contains([' ', '=', ':']) {
            anyhow::bail!("temporary constraints (`update {p}`, `--with`) are not supported yet");
        }
    }
    let transitive = if args.with_all_dependencies || env_flag("COMPOSER_WITH_ALL_DEPENDENCIES") {
        vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDeps
    } else if args.with_dependencies || env_flag("COMPOSER_WITH_DEPENDENCIES") {
        vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDepsNoRootRequire
    } else {
        vivacity_resolver::pool::UpdateMode::OnlyListed
    };
    let mut options = if args.packages.is_empty() {
        vivacity_resolver::session::UpdateOptions::default()
    } else {
        vivacity_resolver::session::UpdateOptions::partial(&args.packages, transitive)
    };
    options.no_blocking = args.no_blocking || args.no_security_blocking;
    // BaseCommand: COMPOSER_PREFER_STABLE / COMPOSER_PREFER_LOWEST count as
    // the options.
    let prefer_stable = args.prefer_stable || env_flag("COMPOSER_PREFER_STABLE");
    let prefer_lowest = args.prefer_lowest || env_flag("COMPOSER_PREFER_LOWEST");
    {
        let project = project_dir(args.working_dir.as_deref())?;
        if let Ok(manifest) = std::fs::read_to_string(project.join("composer.json"))
            .map_err(anyhow::Error::from)
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(Into::into))
        {
            if let Some(code) = resolution_fallback(
                &project,
                &manifest,
                vivacity_core::scope::ResolutionCommand::Update,
                args.no_plugins,
                args.no_fallback,
                !args.no_install && !args.dry_run,
            )? {
                return Ok(code);
            }
        }
    }
    run_update_resolved(args, options, prefer_stable, prefer_lowest)
}

/// Resolution, lock write and install, once the list and the update mode
/// are decided (shared by `update` and `remove`).
fn run_update_resolved(
    args: &UpdateArgs,
    options: vivacity_resolver::session::UpdateOptions,
    prefer_stable: bool,
    prefer_lowest: bool,
) -> anyhow::Result<i32> {
    let resolved = resolve_and_lock(args, options, prefer_stable, prefer_lowest)?;
    if resolved.status != 0 {
        return Ok(resolved.status);
    }
    let project = project_dir(args.working_dir.as_deref())?;
    if !args.no_install {
        let (virtual_lock, virtual_manifest) = if args.dry_run {
            (resolved.lock.clone(), Some(resolved.manifest.clone()))
        } else {
            (None, None)
        };
        let status = install_after_update(args, virtual_lock, virtual_manifest)?;
        if status != 0 {
            return Ok(status);
        }
    }
    print_post_update(&resolved, &project);
    Ok(0)
}

/// The install that follows the lock write (`Installer::run` with
/// `setInstall(true)`).
fn install_after_update(
    args: &UpdateArgs,
    virtual_lock: Option<serde_json::Value>,
    virtual_manifest: Option<serde_json::Value>,
) -> anyhow::Result<i32> {
    run_install(&InstallArgs {
        no_dev: args.no_dev,
        no_autoloader: args.no_autoloader,
        optimize_autoloader: args.optimize_autoloader,
        classmap_authoritative: args.classmap_authoritative,
        apcu_autoloader: args.apcu_autoloader,
        apcu_autoloader_prefix: args.apcu_autoloader_prefix.clone(),
        no_scripts: args.no_scripts,
        run_scripts: false,
        no_plugins: args.no_plugins,
        ignore_platform_reqs: args.ignore_platform_reqs,
        ignore_platform_req: args.ignore_platform_req.clone(),
        no_fallback: args.no_fallback,
        check_scope: false,
        offline: args.offline,
        no_blocking: args.no_blocking,
        no_security_blocking: args.no_security_blocking,
        no_install: false,
        dry_run: args.dry_run,
        working_dir: args.working_dir.clone(),
        spawn_fallback: args.spawn_fallback,
        after_update: true,
        virtual_lock,
        virtual_manifest,
    })
}

/// Resolution and lock write: 0, or 2 on an unsolvable set.
/// The outcome of the resolution: the status (0, or 2 on an unsolvable
/// set) and the lock data, written or not (`config.lock: false` resolves
/// without writing; Composer then keeps a "virtual" lock).
pub(crate) struct Resolved {
    pub(crate) status: i32,
    pub(crate) lock: Option<serde_json::Value>,
    /// What `Installer::run` prints after the install phase (suggestions,
    /// abandoned packages), ready to print; the funding count is read
    /// from installed.json at that time (`print_post_update`).
    pub(crate) post: Vec<String>,
    /// The manifest as the (possibly patched) root package sees it.
    pub(crate) manifest: serde_json::Value,
    /// symfony/flex was active for this resolution: its `POST_UPDATE_CMD`
    /// output follows the report (`print_post_update`).
    pub(crate) flex_active: bool,
}

/// Prints the post-update report of `Installer::run` once the install
/// phase (if any) is done: the precomputed lines, then the funding count
/// of the packages installed now — unless `dumpAutoloader` is off (dry
/// run).
pub(crate) fn print_post_update(resolved: &Resolved, project: &std::path::Path) {
    for line in &resolved.post {
        eprintln!("{line}");
    }
    print_funding(project, &resolved.manifest);
    if resolved.flex_active {
        print_flex_post_update(&resolved.manifest);
    }
}

/// `Flex::install` on `POST_UPDATE_CMD` after a resolution that installed
/// nothing (no operations, hence no recipes): an empty line, then
/// `finish()` — `synchronizePackageJson`'s notices (the synchronisation
/// itself, with a package.json or importmap.php, is a fallback guard
/// upstream), `symfony.lock` unchanged — then the recipes hint when the
/// downloader is enabled (symfony/flex required by the root).
fn print_flex_post_update(manifest: &serde_json::Value) {
    eprintln!();
    let sync = manifest
        .get("extra")
        .and_then(|e| e.get("symfony/flex"))
        .and_then(|f| f.get("synchronize_package_json"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let enabled = ["require", "require-dev"].iter().any(|k| {
        manifest
            .get(k)
            .and_then(|m| m.get("symfony/flex"))
            .is_some()
    });
    if !sync {
        eprintln!("Skip synchronizing package.json with PHP packages");
    } else if !enabled {
        eprintln!("Synchronizing package.json is disabled: \"symfony/flex\" not found in the root composer.json");
    }
    if enabled {
        eprintln!("Run composer recipes at any time to see the status of your Symfony recipes.");
        eprintln!();
    }
}

/// The Flex behaviours at `POST_UPDATE_CMD` that write files, decided
/// before resolving: `.env.dist` copied to `.env` (`Flex::install`) and
/// package.json / importmap.php synchronised with the lock
/// (`PackageJsonSynchronizer::shouldSynchronize`). Either → Composer.
fn flex_write_guard(project: &std::path::Path, manifest: &serde_json::Value) -> Option<String> {
    let root_dir = manifest
        .get("extra")
        .and_then(|e| e.get("symfony"))
        .and_then(|s| s.get("root-dir"))
        .and_then(serde_json::Value::as_str)
        .map(|d| project.join(d))
        .unwrap_or_else(|| project.to_path_buf());
    let dotenv = manifest
        .get("extra")
        .and_then(|e| e.get("runtime"))
        .and_then(|r| r.get("dotenv_path"))
        .and_then(serde_json::Value::as_str)
        .map(|p| root_dir.join(p))
        .unwrap_or_else(|| root_dir.join(".env"));
    let dist = dotenv.with_file_name(format!(
        "{}.dist",
        dotenv
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let local = dotenv.with_file_name(format!(
        "{}.local",
        dotenv
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    if !dotenv.exists() && !local.exists() && dist.exists() {
        let mentions_local = std::fs::read_to_string(&dist)
            .map(|t| t.contains(".env.local"))
            .unwrap_or(false);
        if !mentions_local {
            return Some(format!(
                "would copy {} to {} (not emulated)",
                dist.display(),
                dotenv.display()
            ));
        }
    }
    let sync = manifest
        .get("extra")
        .and_then(|e| e.get("symfony/flex"))
        .and_then(|f| f.get("synchronize_package_json"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let enabled = ["require", "require-dev"].iter().any(|k| {
        manifest
            .get(k)
            .and_then(|m| m.get("symfony/flex"))
            .is_some()
    });
    if sync
        && enabled
        && (root_dir.join("package.json").exists() || root_dir.join("importmap.php").exists())
    {
        return Some(
            "would synchronize package.json / importmap.php with the lock (not emulated)"
                .to_owned(),
        );
    }
    // `Flex::update` → `unpack()`: every root requirement is looked up
    // (installed.json, else the repositories) and a `symfony-pack` with
    // requirements is unpacked into composer.json. Decided here from
    // installed.json and the lock: a requirement found in neither is left
    // to Composer (its type is unknown before loading it).
    if manifest.get("flex-require").is_none() && manifest.get("flex-require-dev").is_none() {
        let installed = installed_packages(project, manifest);
        let lock: Option<serde_json::Value> =
            std::fs::read_to_string(project.join("composer.lock"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
        let by_name = |name: &str| -> Option<serde_json::Value> {
            let named = |p: &serde_json::Value| {
                p.get("name").and_then(serde_json::Value::as_str) == Some(name)
            };
            installed.iter().find(|p| named(p)).cloned().or_else(|| {
                lock.as_ref().and_then(|l| {
                    ["packages", "packages-dev"].iter().find_map(|k| {
                        l.get(k)
                            .and_then(serde_json::Value::as_array)
                            .and_then(|a| a.iter().find(|p| named(p)))
                            .cloned()
                    })
                })
            })
        };
        for key in ["require", "require-dev"] {
            let Some(reqs) = manifest.get(key).and_then(serde_json::Value::as_object) else {
                continue;
            };
            for name in reqs.keys() {
                let lname = name.to_ascii_lowercase();
                if vivacity_resolver::platform::is_platform_package(&lname) {
                    continue;
                }
                match by_name(&lname) {
                    Some(p) => {
                        let is_pack = p.get("type").and_then(serde_json::Value::as_str)
                            == Some("symfony-pack");
                        let has_links = ["require", "require-dev"].iter().any(|k| {
                            p.get(k)
                                .and_then(serde_json::Value::as_object)
                                .is_some_and(|m| !m.is_empty())
                        });
                        if is_pack && has_links {
                            return Some(format!(
                                "would unpack the symfony-pack {lname} into composer.json (not emulated)"
                            ));
                        }
                    }
                    None => {
                        return Some(format!(
                            "looks up the root requirement {lname} (not installed nor locked) to decide an unpack (not emulated)"
                        ));
                    }
                }
            }
        }
    }
    None
}

/// The packages of vendor/composer/installed.json (list form of old
/// Composers accepted), minus those whose install path is gone
/// (`Factory::purgePackages` through `LibraryInstaller::isInstalled`; a
/// metapackage is always installed).
/// `LocalRepoTransaction` over installed.json and the lock, when the raw
/// entries prove it empty: same names, and for each the same version,
/// dist and source references, and `abandoned` state — what
/// `Transaction::new` compares. Returns the present names in
/// installed.json order (the local repository's order, which the emulated
/// plugins read). `None` when anything differs, or when either side could
/// carry an alias (a root alias in the lock, a `branch-alias` on a dev
/// version): aliases add operations of their own, and the full
/// computation handles them.
fn unchanged_local_repository(
    installed: &[serde_json::Value],
    lock: &serde_json::Value,
    with_dev: bool,
) -> Option<Vec<String>> {
    use serde_json::Value;
    if lock
        .get("aliases")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return None;
    }
    let wanted: Vec<&Value> = ["packages", "packages-dev"]
        .iter()
        .take(if with_dev { 2 } else { 1 })
        .filter_map(|k| lock.get(*k))
        .filter_map(Value::as_array)
        .flatten()
        .collect();
    if with_dev && lock.get("packages-dev").is_none() {
        return None; // the loader errors on this: let it
    }
    if wanted.len() != installed.len() {
        return None;
    }
    // `ArrayLoader::getBranchAlias` only looks at a dev version (and at
    // `default-branch`): a stable entry never grows an alias, whatever its
    // `extra`. Anything else declines, and the full computation decides.
    let aliased = |p: &Value| {
        let dev = p
            .get("version")
            .and_then(Value::as_str)
            .is_some_and(|v| v.starts_with("dev-") || v.ends_with("-dev"));
        (dev && p
            .get("extra")
            .and_then(|e| e.get("branch-alias"))
            .is_some_and(|b| !b.is_null()))
            || p.get("default-branch") == Some(&Value::Bool(true))
    };
    let key = |p: &Value| -> Option<String> {
        p.get("name")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
    };
    let field = |p: &Value, a: &str, b: &str| p.get(a).and_then(|d| d.get(b)).cloned();
    let identity = |p: &Value| {
        (
            p.get("version").cloned(),
            field(p, "dist", "reference"),
            field(p, "source", "reference"),
            p.get("abandoned").cloned(),
        )
    };
    let mut by_name: std::collections::BTreeMap<String, &Value> = Default::default();
    for p in &wanted {
        if aliased(p) {
            return None;
        }
        by_name.insert(key(p)?, p);
    }
    let mut names = Vec::with_capacity(installed.len());
    for p in installed {
        if aliased(p) {
            return None;
        }
        let name = key(p)?;
        if identity(by_name.get(&name)?) != identity(p) {
            return None;
        }
        names.push(name);
    }
    (names.len() == by_name.len()).then_some(names)
}

fn installed_packages(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let composer_dir = composer_dir_of(project, manifest);
    let Some(installed) = vivacity_core::jsonfile::read(&composer_dir.join("installed.json"))
    else {
        return Vec::new();
    };
    let list = installed
        .get("packages")
        .and_then(|p| p.as_array())
        .cloned()
        .or_else(|| installed.as_array().cloned())
        .unwrap_or_default();
    list.into_iter()
        .filter(|p| {
            if p.get("type").and_then(|t| t.as_str()) == Some("metapackage") {
                return true;
            }
            match p.get("install-path").and_then(|v| v.as_str()) {
                Some(rel) => composer_dir.join(rel).exists(),
                None => true,
            }
        })
        .collect()
}

/// `Installer::run`: the funding count of the local repository's packages
/// (never an alias), unless `COMPOSER_FUND` is a number equal to 0. Printed
/// in every mode, dry run included (`$localRepo` is then the mocked
/// repository of the same packages).
fn print_funding(project: &std::path::Path, manifest: &serde_json::Value) {
    if let Ok(env) = std::env::var("COMPOSER_FUND") {
        let t = env.trim();
        if let Ok(n) = t.parse::<f64>() {
            if n == 0.0 {
                return;
            }
        }
    }
    let funding = installed_packages(project, manifest)
        .iter()
        .filter(|p| {
            p.get("funding")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|f| !f.is_empty())
        })
        .count();
    if funding > 0 {
        eprintln!(
            "{funding} package{} you are using {} looking for funding.\nUse the `composer fund` command to find out more!",
            if funding == 1 { "" } else { "s" },
            if funding == 1 { "is" } else { "are" }
        );
    }
}

fn resolve_and_lock(
    args: &UpdateArgs,
    options: vivacity_resolver::session::UpdateOptions,
    prefer_stable: bool,
    prefer_lowest: bool,
) -> anyhow::Result<Resolved> {
    use vivacity_resolver::platform_filter::PlatformRequirementFilter;
    use vivacity_resolver::session::UpdateSession;
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    let manifest_path = project.join("composer.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("cannot read {}", manifest_path.display()))?;
    let installer_dev_mode =
        !(args.no_dev || std::env::var("COMPOSER_NO_DEV").is_ok_and(|v| !v.is_empty() && v != "0"));
    let mut options = options;
    if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
        refuse_merge_plugin_resolution(&project, &m, !args.no_plugins)?;
        // composer-merge-plugin installed and allowed: the root Composer
        // resolves is the merged one (INIT, then `PRE_UPDATE_CMD` with the
        // installer's dev mode for the `require-dev` sections). A merged
        // `repositories` section was handed to Composer before getting
        // here (`resolution_fallback`).
        let active = vivacity_core::scope::active_plugins(&project, &m, !args.no_plugins)
            .iter()
            .any(|p| p == vivacity_resolver::merge_plugin::PLUGIN_NAME);
        options.merged = merge_root_when(&project, &m, installer_dev_mode, active)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(merged) = &options.merged {
            if !merged.repositories.is_empty() {
                anyhow::bail!(
                    "wikimedia/composer-merge-plugin: {} declares repositories (not emulated at resolution)",
                    merged.repositories.join(", ")
                );
            }
        }
    }
    let home = vivacity_core::fetch::composer_home();
    let http = http_transport(&project, args.offline)?;
    let cache_repo_dir = vivacity_core::fetch::composer_cache_dir().join("repo");
    let mut session = UpdateSession::prepare_update(
        &project,
        home.as_deref(),
        true,
        Some(http),
        Some(&cache_repo_dir),
        &options,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    trace("prepare", t0);
    // symfony/flex active (installed, allowed, `extra.symfony.require` or
    // `SYMFONY_REQUIRE` set): its `PRE_POOL_CREATE` filter runs on the
    // pool, from the index fetched like Flex's Downloader does. The
    // with-install and `require` cases were handed to Composer before
    // getting here (`resolution_fallback`).
    let mut flex_active = false;
    if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
        flex_active = vivacity_core::scope::active_plugins(&project, &m, !args.no_plugins)
            .iter()
            .any(|p| p == "symfony/flex");
        if flex_active {
            if let Some(symfony_require) = vivacity_resolver::flex_filter::symfony_require(&m) {
                let parsed = vivacity_resolver::constraint::parse_constraints(&symfony_require)
                    .map_err(|e| {
                        anyhow::anyhow!("extra.symfony.require {symfony_require:?}: {e}")
                    })?;
                let versions = flex::versions(&project, &m, args.offline)?;
                session.flex = Some(vivacity_resolver::session::FlexSession {
                    symfony_require,
                    symfony: parsed.constraint,
                    versions,
                });
            }
        }
    }
    session.prefer_stable = prefer_stable;
    session.prefer_lowest = prefer_lowest;
    session.installer_dev_mode = installer_dev_mode;
    // BaseCommand: COMPOSER_IGNORE_PLATFORM_REQS counts as the option,
    // COMPOSER_IGNORE_PLATFORM_REQ (comma-separated list) counts as the
    // list when it is empty.
    let env_flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
    let ignore_all = args.ignore_platform_reqs || env_flag("COMPOSER_IGNORE_PLATFORM_REQS");
    let mut ignore_list = args.ignore_platform_req.clone();
    if !ignore_all && ignore_list.is_empty() {
        if let Ok(env) = std::env::var("COMPOSER_IGNORE_PLATFORM_REQ") {
            if !env.is_empty() {
                eprintln!("COMPOSER_IGNORE_PLATFORM_REQ is set to ignore {env}. You may experience unexpected errors.");
                ignore_list = env.split(',').map(str::to_owned).collect();
            }
        }
    }
    if ignore_all && env_flag("COMPOSER_IGNORE_PLATFORM_REQS") && !args.ignore_platform_reqs {
        eprintln!("COMPOSER_IGNORE_PLATFORM_REQS is set. You may experience unexpected errors.");
    }
    let filter = if ignore_all {
        PlatformRequirementFilter::IgnoreAll
    } else if !ignore_list.is_empty() {
        PlatformRequirementFilter::from_list(&ignore_list)
    } else {
        PlatformRequirementFilter::IgnoreNothing
    };
    eprintln!("Loading composer repositories with package information");
    // (`Updating dependencies` is printed by the session once the pool is
    // built — Flex's notice comes before it.)
    // `Installer::run`: an unsolvable set means exit code 2
    // (`SolverProblemsException`), any other error is an exception.
    let (lock, report) = match session.update(&manifest_text, &filter) {
        Ok(r) => r,
        Err(e) if e.kind == vivacity_resolver::session::SessionErrorKind::Unsolvable => {
            eprintln!("{e}");
            return Ok(Resolved {
                status: 2,
                lock: None,
                post: Vec::new(),
                manifest: serde_json::Value::Null,
                flex_active: false,
            });
        }
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };
    trace("resolve", t0);

    let lock_path = project.join("composer.lock");
    let mut text = vivacity_core::phpjson::php_json_encode_with(
        &lock,
        vivacity_core::phpjson::FLAGS_JSONFILE,
    )?;
    // `JsonFile::read` remembers the existing lock's indentation, `write`
    // reuses it.
    let old_text = std::fs::read_to_string(&lock_path).ok();
    if let Some(old) = &old_text {
        let indent = vivacity_resolver::json_manipulator::detect_indenting(old)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if indent != "    " {
            text = vivacity_resolver::config_source::reindent(&text, &indent);
        }
    }
    text.push('\n');
    // `JsonFile::write` goes through `filePutContentsIfModified`: the file
    // is only rewritten if its bytes change. (`Locker::setLockData` also
    // compares the decoded data, but `{}` versus `[]` makes that comparison
    // always false for an ordinary lock; the bytes are the observable
    // criterion.)
    let unchanged = old_text.as_deref() == Some(text.as_str());
    // `config.lock: false`: Composer resolves without writing a lock.
    let write_lock = config_lock_enabled(&manifest_text);
    let ops = &report.transaction.transaction.operations;
    if ops.is_empty() {
        eprintln!("Nothing to modify in lock file");
    } else if write_lock {
        let count = |f: &dyn Fn(&vivacity_resolver::transaction::Operation) -> bool| {
            ops.iter().filter(|o| f(o)).count()
        };
        use vivacity_resolver::transaction::Operation;
        let installs = count(&|o| matches!(o, Operation::Install(_)));
        let updates = count(&|o| matches!(o, Operation::Update(..)));
        let removals = count(&|o| matches!(o, Operation::Uninstall(_)));
        eprintln!(
            "Lock file operations: {installs} install{}, {updates} update{}, {removals} removal{}",
            if installs == 1 { "" } else { "s" },
            if updates == 1 { "" } else { "s" },
            if removals == 1 { "" } else { "s" }
        );
        // `  - Locking …` / `Upgrading` / `Removing`, removals first, by name.
        for line in vivacity_resolver::transaction::lock_operation_lines(&session.arena, ops) {
            eprintln!("{line}");
        }
    }
    // `$updatedLock && $this->writeLock && $this->executeOperations`: a dry
    // run neither writes nor says so.
    if write_lock && !args.dry_run {
        eprintln!("Writing lock file");
        if !unchanged {
            std::fs::write(&lock_path, &text)
                .with_context(|| format!("cannot write {}", lock_path.display()))?;
        }
    }
    trace("write lock", t0);
    // `Installer::run` after `doUpdate` (and after the install phase): the
    // suggestions of the newly installed packages (and of the root on a
    // fresh install), the abandoned packages of the new lock.
    let manifest = patched_manifest(&session, &manifest_text);
    let fresh_install = !composer_dir_of(&project, &manifest)
        .join("installed.json")
        .is_file();
    let post = post_update_report(&session, ops, &lock, &manifest_text, fresh_install);
    Ok(Resolved {
        status: 0,
        lock: Some(lock),
        post,
        manifest,
        flex_active,
    })
}

/// composer.json with the session root's `require`/`require-dev` (the
/// in-memory patch of a dry run applied; identical otherwise).
fn patched_manifest(
    session: &vivacity_resolver::session::UpdateSession,
    manifest_text: &str,
) -> serde_json::Value {
    let mut manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap_or_default();
    let links_to_object = |links: &vivacity_resolver::package::Links| -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for l in links.iter() {
            map.insert(
                l.target.clone(),
                serde_json::Value::String(l.pretty_constraint.clone()),
            );
        }
        serde_json::Value::Object(map)
    };
    if let Some(obj) = manifest.as_object_mut() {
        for (key, links) in [
            ("require", &session.root.package.requires),
            ("require-dev", &session.root.package.dev_requires),
        ] {
            if links.is_empty() {
                obj.remove(key);
            } else {
                obj.insert(key.to_owned(), links_to_object(links));
            }
        }
    }
    manifest
}

/// The lines of `Installer::run` after a successful update:
/// `SuggestedPackagesReporter::outputMinimalistic` (suggestions of the
/// packages installed by this update, plus the root's own on a fresh
/// install, minus the targets some other package provides), one warning
/// per abandoned package of the new lock. The funding count is added by
/// `print_post_update`.
fn post_update_report(
    session: &vivacity_resolver::session::UpdateSession,
    ops: &[vivacity_resolver::transaction::Operation],
    lock: &serde_json::Value,
    manifest_text: &str,
    fresh_install: bool,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    use vivacity_resolver::transaction::Operation;
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap_or_default();
    // (source pretty name, target)
    let mut suggestions: Vec<(String, String)> = Vec::new();
    let add_from =
        |suggestions: &mut Vec<(String, String)>, source: &str, raw: &serde_json::Value| {
            if let Some(map) = raw.get("suggest").and_then(serde_json::Value::as_object) {
                for target in map.keys() {
                    suggestions.push((source.to_owned(), target.clone()));
                }
            }
        };
    for op in ops {
        if let Operation::Install(idx) = op {
            let p = &session.arena[*idx];
            add_from(&mut suggestions, &p.pretty_name, &p.raw);
        }
    }
    if fresh_install {
        let root_name = manifest
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("__root__");
        add_from(&mut suggestions, root_name, &manifest);
    }
    if !suggestions.is_empty() {
        // `InstalledRepository([locked repo (dev mode), platform repo, root])`:
        // `getNames()` of each package -> the packages providing that name.
        let mut installed_names: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut add_names = |name: &str, names: Vec<String>| {
            for n in names {
                installed_names.entry(n).or_default().push(name.to_owned());
            }
        };
        let mut lock_sections = vec!["packages"];
        if session.installer_dev_mode {
            lock_sections.push("packages-dev");
        }
        for section in lock_sections {
            for p in lock
                .get(section)
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = p.get("name").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let name = name.to_lowercase();
                let mut names = vec![name.clone()];
                for key in ["provide", "replace"] {
                    for target in p
                        .get(key)
                        .and_then(serde_json::Value::as_object)
                        .into_iter()
                        .flat_map(|m| m.keys())
                    {
                        if !names.contains(target) {
                            names.push(target.clone());
                        }
                    }
                }
                add_names(&name, names);
            }
        }
        for &idx in &session.platform {
            let p = &session.arena[idx];
            add_names(&p.name, p.names(true));
        }
        let root = &session.root.package;
        add_names(&root.name, root.names(true));
        let count = suggestions
            .iter()
            .filter(|(source, target)| {
                let source_lower = source.to_lowercase();
                !installed_names
                    .get(target)
                    .is_some_and(|providers| providers.iter().any(|p| *p != source_lower))
            })
            .count();
        if count > 0 {
            out.push(format!(
                "{count} package suggestions were added by new dependencies, use `composer suggest` to see details."
            ));
        }
    }
    out.extend(abandoned_warnings(lock));
    out
}

/// `Installer::run`: one warning per abandoned package of the lock (dev
/// included), in lock order.
fn abandoned_warnings(lock: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for section in ["packages", "packages-dev"] {
        for p in lock
            .get(section)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            // `CompletePackage::isAbandoned` = `(bool) $abandoned`,
            // `getReplacementPackage` = the string, else null.
            let abandoned = match p.get("abandoned") {
                None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false)) => {
                    continue
                }
                Some(serde_json::Value::String(s)) if s.is_empty() || s == "0" => continue,
                Some(serde_json::Value::Number(n)) if n.as_f64() == Some(0.0) => continue,
                Some(serde_json::Value::Array(a)) if a.is_empty() => continue,
                Some(serde_json::Value::Object(o)) if o.is_empty() => continue,
                Some(serde_json::Value::String(replacement)) => Some(replacement.clone()),
                Some(_) => None,
            };
            let replacement = match abandoned {
                Some(r) => format!("Use {r} instead"),
                None => "No replacement was suggested".to_owned(),
            };
            out.push(format!(
                "Package {} is abandoned, you should avoid using it. {replacement}.",
                p.get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
            ));
        }
    }
    out
}

/// `Config::get('lock')`: project then global config; `"false"` and PHP's
/// falsy values disable it.
fn config_lock_enabled(manifest_text: &str) -> bool {
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap_or_default();
    match require::config_value(&manifest, "lock") {
        None => true,
        Some(serde_json::Value::String(s)) => s != "false" && !(s.is_empty() || s == "0"),
        Some(serde_json::Value::Bool(b)) => b,
        Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Array(a)) => !a.is_empty(),
        Some(serde_json::Value::Object(m)) => !m.is_empty(),
    }
}

/// Minimal `join_all` (no futures dependency): the tasks run concurrently
/// on the runtime, results in order.
async fn futures_join_all<F: std::future::Future + Send + 'static>(tasks: Vec<F>) -> Vec<F::Output>
where
    F::Output: Send + 'static,
{
    let handles: Vec<_> = tasks.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.expect("metadata task panicked"));
    }
    out
}

/// `RemoveCommand::execute`: composer.json edited through
/// `JsonConfigSource`, `allow-plugins` cleaned up, then a partial update
/// (list = removed packages, "with dependencies except root requirements"
/// mode by default), composer.json restored if it fails.
fn run_remove(args: &RemoveArgs) -> anyhow::Result<i32> {
    use vivacity_resolver::config_source::{composer_file, JsonConfigSource};
    let env_flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
    if args.minimal_changes || env_flag("COMPOSER_MINIMAL_CHANGES") {
        anyhow::bail!("`--minimal-changes` (update-with-minimal-changes) is not supported yet");
    }
    // `--dry-run`: the links are collected instead of removed from the
    // file, and the root package is patched in memory (`$toRemove`).
    let dry_run = args.dry_run;
    let mut to_remove: Vec<(bool, String)> = Vec::new();
    if args.packages.is_empty() && !args.unused {
        eprintln!("Not enough arguments (missing: \"packages\").");
        return Ok(1);
    }
    let project = project_dir(args.working_dir.as_deref())?;
    let file = composer_file(&project);
    // The name Composer displays is the path as `Factory` gives it. An
    // alternate manifest (`COMPOSER=alt.json`, lock `alt.lock`) is not
    // followed by the resolution: rejected rather than resolved against
    // composer.json.
    let file_label = match std::env::var("COMPOSER").ok().map(|f| f.trim().to_owned()) {
        Some(f) if !f.is_empty() && f != "composer.json" && f != "./composer.json" => {
            anyhow::bail!("COMPOSER={f}: an alternate manifest is not supported yet");
        }
        Some(f) if !f.is_empty() => f,
        _ => "./composer.json".to_owned(),
    };
    let manifest_text = std::fs::read_to_string(&file)
        .with_context(|| format!("cannot read {}", file.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
        .with_context(|| format!("{} does not contain valid JSON", file.display()))?;
    refuse_merge_plugin_resolution(&project, &manifest, !args.no_plugins)?;
    if serde_json::from_str::<serde_json::Value>(&manifest_text)
        .ok()
        .and_then(|m| m.get("config")?.get("update-with-minimal-changes").cloned())
        .is_some_and(|v| v == serde_json::Value::Bool(true))
    {
        anyhow::bail!("config.update-with-minimal-changes is not supported yet");
    }
    let mut packages: Vec<String> = args.packages.iter().map(|p| p.to_lowercase()).collect();

    // `Locker::isLocked`: a readable lock that has a `packages` key.
    let locked_data: Option<serde_json::Value> =
        std::fs::read_to_string(project.join("composer.lock"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(|v: &serde_json::Value| v.get("packages").is_some());
    if args.unused {
        let lock = locked_data.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "A valid composer.lock file is required to run this command with --unused"
            )
        })?;
        packages.extend(unused_locked_packages(&manifest, &lock));
        if packages.is_empty() {
            eprintln!("No unused packages to remove");
            return Ok(0);
        }
    }

    if let Some(code) = resolution_fallback(
        &project,
        &manifest,
        vivacity_core::scope::ResolutionCommand::Remove,
        args.no_plugins,
        args.no_fallback,
        !args.no_install && !args.dry_run,
    )? {
        return Ok(code);
    }
    let backup = manifest_text.clone();
    let json = JsonConfigSource::new(&file);
    let (link_type, alt_type) = if args.dev {
        ("require-dev", "require")
    } else {
        ("require", "require-dev")
    };
    if args.update_with_dependencies {
        eprintln!("You are using the deprecated option \"update-with-dependencies\". This is now default behaviour. The --no-update-with-dependencies option can be used to remove a package without its dependencies.");
    }
    // `$composer[$linkType][strtolower($name)] = $name`: the lowercase keys
    // are added to the decoded array (the originals stay, and `array_keys`
    // sees them all).
    let keyed = |section: &str| -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> = Vec::new();
        if let Some(m) = manifest.get(section).and_then(|v| v.as_object()) {
            for k in m.keys() {
                keys.push((k.clone(), k.clone()));
            }
            for k in m.keys() {
                let lower = k.to_lowercase();
                match keys.iter_mut().find(|(key, _)| *key == lower) {
                    Some(entry) => entry.1 = k.clone(),
                    None => keys.push((lower, k.clone())),
                }
            }
        }
        keys
    };
    let sections = [(link_type, keyed(link_type)), (alt_type, keyed(alt_type))];
    let lookup = |section: &str, key: &str| -> Option<String> {
        sections
            .iter()
            .find(|(s, _)| *s == section)?
            .1
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, name)| name.clone())
    };
    let grep = |section: &str, pattern: &str| -> Vec<String> {
        let re = vivacity_resolver::pool::package_name_regexp(pattern);
        sections
            .iter()
            .find(|(s, _)| *s == section)
            .map(|(_, keys)| {
                keys.iter()
                    .filter(|(k, _)| re.is_match(k.as_bytes()).unwrap_or(false))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    let has_section = |section: &str| manifest.get(section).is_some_and(|v| !v.is_null());
    let dev_key = link_type == "require-dev";
    for package in &packages {
        if let Some(name) = lookup(link_type, package) {
            if dry_run {
                to_remove.push((dev_key, name));
            } else {
                json.remove_link(link_type, &name)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
        } else if let Some(name) = lookup(alt_type, package) {
            eprintln!("{name} could not be found in {link_type} but it is present in {alt_type}");
        } else if has_section(link_type) && !grep(link_type, package).is_empty() {
            for matched in grep(link_type, package) {
                if dry_run {
                    to_remove.push((dev_key, matched));
                } else {
                    json.remove_link(link_type, &matched)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
            }
        } else if has_section(alt_type) && !grep(alt_type, package).is_empty() {
            for matched in grep(alt_type, package) {
                eprintln!(
                    "{matched} could not be found in {link_type} but it is present in {alt_type}"
                );
            }
        } else {
            eprintln!("{package} is not required in your composer.json and has not been removed");
        }
    }
    eprintln!("{file_label} has been updated");
    if args.no_update {
        return Ok(0);
    }

    // `allow-plugins`: the merged config (project then global); entries
    // whose key is a removed package are dropped from composer.json.
    let updated: serde_json::Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    if let (false, Some(serde_json::Value::Object(allow))) = (
        dry_run,
        vivacity_core::layout::merged_allow_plugins(
            updated.get("config").and_then(|c| c.get("allow-plugins")),
            vivacity_core::layout::global_allow_plugins().as_ref(),
        ),
    ) {
        let removed: Vec<&String> = allow.keys().filter(|k| packages.contains(k)).collect();
        if !removed.is_empty() {
            if removed.len() == allow.len() {
                json.remove_config_setting("allow-plugins")
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            } else {
                for plugin in removed {
                    json.remove_config_setting(&format!("allow-plugins.{plugin}"))
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
            }
        }
    }

    // `Request::UPDATE_LISTED_WITH_TRANSITIVE_DEPS_NO_ROOT_REQUIRE` by
    // default; `COMPOSER_WITH_ALL_DEPENDENCIES` counts as the option
    // (BaseCommand), `COMPOSER_WITH_DEPENDENCIES` has no option here.
    let transitive = if args.update_with_all_dependencies
        || args.with_all_dependencies
        || env_flag("COMPOSER_WITH_ALL_DEPENDENCIES")
    {
        vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDeps
    } else if args.no_update_with_dependencies {
        vivacity_resolver::pool::UpdateMode::OnlyListed
    } else {
        vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDepsNoRootRequire
    };
    let mut flags = String::new();
    if transitive == vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDeps {
        flags.push_str(" --with-all-dependencies");
    } else if transitive == vivacity_resolver::pool::UpdateMode::OnlyListed {
        flags.push_str(" --with-dependencies");
    }
    eprintln!("Running composer update {}{flags}", packages.join(" "));
    // `setUpdateAllowList` only if a lock exists.
    let mut options = if locked_data.is_some() {
        vivacity_resolver::session::UpdateOptions::partial(&packages, transitive)
    } else {
        vivacity_resolver::session::UpdateOptions::default()
    };
    options.no_blocking = args.no_blocking || args.no_security_blocking;
    if dry_run {
        options.root_patch = Some(vivacity_resolver::root::RootPatch {
            removals: to_remove,
            ..Default::default()
        });
    }
    let update_args = UpdateArgs {
        packages: Vec::new(),
        with_dependencies: false,
        with_all_dependencies: false,
        no_install: args.no_install,
        dry_run: args.dry_run,
        no_dev: args.update_no_dev || env_flag("COMPOSER_NO_DEV"),
        no_autoloader: args.no_autoloader,
        optimize_autoloader: args.optimize_autoloader,
        classmap_authoritative: args.classmap_authoritative,
        apcu_autoloader: args.apcu_autoloader,
        apcu_autoloader_prefix: args.apcu_autoloader_prefix.clone(),
        no_scripts: args.no_scripts,
        no_plugins: args.no_plugins,
        no_audit: args.no_audit,
        no_blocking: args.no_blocking,
        no_security_blocking: args.no_security_blocking,
        prefer_stable: false,
        prefer_lowest: false,
        ignore_platform_reqs: args.ignore_platform_reqs,
        ignore_platform_req: args.ignore_platform_req.clone(),
        no_fallback: args.no_fallback,
        offline: args.offline,
        working_dir: args.working_dir.clone(),
        // The rest of `remove` (restore, local repository check) must run
        // even if the install is handed over to Composer.
        spawn_fallback: true,
    };
    // `remove` has no prefer-stable/lowest option: only the manifest
    // counts.
    // An exception (transport, invalid manifest) propagates without a
    // restore; only a non-zero status from `Installer::run` restores.
    let status = run_update_resolved(&update_args, options, false, false)?;
    if status != 0 {
        eprintln!("\nRemoval failed, reverting {file_label} to its original content.");
        std::fs::write(&file, &backup)
            .with_context(|| format!("cannot restore {}", file.display()))?;
    }
    // Is the package still in the local repository? That repository is
    // vendor/composer/installed.json minus the packages whose install path
    // no longer exists (`Factory::purgePackages` via
    // `LibraryInstaller::isInstalled`); a metapackage always counts. Not
    // checked on a dry run.
    if dry_run {
        return Ok(status);
    }
    for package in &packages {
        if locally_installed(&project, &manifest, package) {
            eprintln!("Removal failed, {package} is still present, it may be required by another package. See `composer why {package}`.");
            return Ok(2);
        }
    }
    Ok(status)
}

/// `$composer->getRepositoryManager()->getLocalRepository()->findPackages($name)`
/// is non-empty.
fn locally_installed(project: &std::path::Path, manifest: &serde_json::Value, name: &str) -> bool {
    installed_packages(project, manifest)
        .iter()
        .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
}

/// `Locker::getMissingRequirementInfo`: the root requirements the lock
/// does not satisfy — `require` against the non-dev lock, `require-dev`
/// against the whole lock — with replacers and providers counting, and
/// the three fixed lines when anything is missing.
fn missing_requirement_info(
    manifest: &serde_json::Value,
    lock: &serde_json::Value,
    include_dev: bool,
    root_version: &str,
    root_pretty_version: &str,
    merged_links: Option<(
        &[vivacity_resolver::merge_plugin::MergedLink],
        &[vivacity_resolver::merge_plugin::MergedLink],
    )>,
) -> anyhow::Result<Vec<String>> {
    use vivacity_resolver::constraint::{parse_constraints, Constraint, Op};
    use vivacity_resolver::package::{Origin, Package};
    let mut out = Vec::new();
    let mut missing = false;
    let root_name = manifest
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("__root__")
        .to_lowercase();
    // `RootPackageRepository`: the root itself, with its `replace` and
    // `provide` links (`self.version` = its version), is a candidate too.
    // (target, constraint, pretty constraint, "replaced"/"provided")
    let root_links: Vec<(String, Constraint, String, &str)> =
        [("replace", "replaced"), ("provide", "provided")]
            .iter()
            .flat_map(|(key, word)| {
                manifest
                    .get(*key)
                    .and_then(serde_json::Value::as_object)
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .filter_map(move |(t, c)| {
                        let text = c.as_str()?;
                        let source = if text == "self.version" {
                            root_version
                        } else {
                            text
                        };
                        Some((
                            t.to_lowercase(),
                            parse_constraints(source).ok()?.constraint,
                            text.to_owned(),
                            *word,
                        ))
                    })
            })
            .collect();
    let root_eq = Constraint::new(Op::Eq, root_version);
    let mut sets: Vec<(bool, &str, &str)> = vec![(false, "require", "Required")];
    if include_dev {
        sets.push((true, "require-dev", "Required (in require-dev)"));
    }
    for (with_dev, key, description) in sets {
        let mut arena: Vec<Package> = Vec::new();
        let repo =
            vivacity_resolver::repository::locked_repository_with(lock, &mut arena, with_dev)
                .map_err(|e| anyhow::anyhow!("composer.lock: {}", e.0))?;
        // The root's links: the merged, structured ones when the merge
        // plugin is active (a duplicate key is a conjunction the text of
        // the manifest cannot carry faithfully), else the manifest's.
        let links: Vec<(String, String, Constraint)> = match merged_links {
            Some((req, dev)) => (if with_dev { dev } else { req })
                .iter()
                .map(|l| (l.target.clone(), l.pretty.clone(), l.constraint.clone()))
                .collect(),
            None => {
                let Some(map) = manifest.get(key).and_then(serde_json::Value::as_object) else {
                    continue;
                };
                map.iter()
                    .filter_map(|(target, pretty)| {
                        let pretty = pretty.as_str()?;
                        let parsed = parse_constraints(pretty).ok()?;
                        Some((target.to_lowercase(), pretty.to_owned(), parsed.constraint))
                    })
                    .collect()
            }
        };
        for (target, pretty, constraint) in links {
            let pretty = pretty.as_str();
            if vivacity_resolver::platform::is_platform_package(&target) || pretty == "self.version"
            {
                continue;
            }
            let parsed = vivacity_resolver::constraint::ParsedConstraint {
                constraint,
                pretty: pretty.to_owned(),
            };
            // `findPackagesWithReplacersAndProviders($target, $constraint)`
            // over [lock repo, root repo]: a package of that name, or a
            // replacer/provider whose link matches the constraint.
            let matches = |idx: usize, constraint: Option<&Constraint>| -> bool {
                let p = &arena[idx];
                if p.name == target {
                    return constraint
                        .is_none_or(|c| c.matches(&Constraint::new(Op::Eq, p.version.clone())));
                }
                p.replaces.iter().chain(p.provides.iter()).any(|l| {
                    l.target == target && constraint.is_none_or(|c| c.matches(&l.constraint))
                })
            };
            let root_matches = |constraint: Option<&Constraint>| -> bool {
                if root_name == target {
                    return constraint.is_none_or(|c| c.matches(&root_eq));
                }
                root_links
                    .iter()
                    .any(|(t, lc, _, _)| *t == target && constraint.is_none_or(|c| c.matches(lc)))
            };
            if repo.iter().any(|&i| matches(i, Some(&parsed.constraint)))
                || root_matches(Some(&parsed.constraint))
            {
                continue;
            }
            let any: Option<usize> = repo.iter().copied().find(|&i| matches(i, None));
            let line = match any {
                Some(idx) => {
                    let p = &arena[idx];
                    let mut description_text = p.pretty_version.clone();
                    if p.name != target {
                        for (links, text) in [
                            (&p.replaces, "replaced as {} by {}"),
                            (&p.provides, "provided as {} by {}"),
                        ] {
                            if let Some(l) = links.iter().find(|l| l.target == target) {
                                description_text =
                                    text.replacen("{}", &l.pretty_constraint, 1).replacen(
                                        "{}",
                                        &format!("{} {}", p.pretty_name, p.pretty_version),
                                        1,
                                    );
                                break;
                            }
                        }
                    }
                    format!("- {description} package \"{target}\" is in the lock file as \"{description_text}\" but that does not satisfy your constraint \"{pretty}\".")
                }
                // The root repository comes after the lock's: the root by
                // name, or through one of its replace/provide links.
                None if root_matches(None) => {
                    let root_name_pretty = manifest
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("__root__");
                    let description_text = if root_name == target {
                        root_pretty_version.to_owned()
                    } else {
                        match root_links.iter().find(|(t, _, _, _)| *t == target) {
                            Some((_, _, pc, word)) => {
                                format!(
                                    "{word} as {pc} by {root_name_pretty} {root_pretty_version}"
                                )
                            }
                            None => root_pretty_version.to_owned(),
                        }
                    };
                    format!("- {description} package \"{target}\" is in the lock file as \"{description_text}\" but that does not satisfy your constraint \"{pretty}\".")
                }
                None => {
                    format!("- {description} package \"{target}\" is not present in the lock file.")
                }
            };
            out.push(line);
            missing = true;
        }
        let _ = Origin::Locked;
    }
    if missing {
        out.push("This usually happens when composer files are incorrectly merged or the composer.json file is manually edited.".to_owned());
        out.push("Read more about correctly resolving merge conflicts https://getcomposer.org/doc/articles/resolving-merge-conflicts.md".to_owned());
        out.push("and prefer using the \"require\" command over editing the composer.json file directly https://getcomposer.org/doc/03-cli.md#require-r".to_owned());
    }
    Ok(out)
}

/// `remove --unused`: the lock's packages (non-dev) required neither by
/// the root nor by any package reachable from it, in lock order.
fn unused_locked_packages(manifest: &serde_json::Value, lock: &serde_json::Value) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut required: BTreeSet<String> = BTreeSet::new();
    for section in ["require", "require-dev"] {
        if let Some(m) = manifest.get(section).and_then(|v| v.as_object()) {
            required.extend(m.keys().map(|k| k.to_lowercase()));
        }
    }
    let mut locked: Vec<&serde_json::Value> = lock
        .get("packages")
        .and_then(|p| p.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let names = |p: &serde_json::Value| -> Vec<String> {
        let mut out = vec![p
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_lowercase()];
        for key in ["replace", "provide"] {
            if let Some(m) = p.get(key).and_then(|v| v.as_object()) {
                out.extend(m.keys().map(|k| k.to_lowercase()));
            }
        }
        out
    };
    loop {
        let mut found = false;
        let mut i = 0;
        while i < locked.len() {
            let p = locked[i];
            if names(p).iter().any(|n| required.contains(n)) {
                if let Some(m) = p.get("require").and_then(|v| v.as_object()) {
                    required.extend(m.keys().map(|k| k.to_lowercase()));
                }
                found = true;
                locked.remove(i);
            } else {
                i += 1;
            }
        }
        if !found {
            break;
        }
    }
    locked
        .iter()
        .map(|p| {
            p.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase()
        })
        .collect()
}

#[cfg(test)]
mod local_repository_tests {
    use super::unchanged_local_repository;
    use serde_json::{json, Value};

    fn pkg(name: &str, version: &str) -> Value {
        json!({"name": name, "version": version, "dist": {"reference": "abc"}})
    }

    fn lock(packages: Vec<Value>, dev: Vec<Value>) -> Value {
        json!({"packages": packages, "packages-dev": dev, "aliases": []})
    }

    #[test]
    fn accepts_only_a_truly_unchanged_repository() {
        let installed = vec![pkg("a/b", "1.0.0"), pkg("c/d", "2.0.0")];
        let l = lock(vec![pkg("a/b", "1.0.0")], vec![pkg("c/d", "2.0.0")]);
        assert_eq!(
            unchanged_local_repository(&installed, &l, true),
            Some(vec!["a/b".to_owned(), "c/d".to_owned()]),
            "same packages, same versions and references"
        );
        // --no-dev over a dev vendor: the dev package must be uninstalled.
        assert!(unchanged_local_repository(&installed, &l, false).is_none());
    }

    #[test]
    fn declines_every_difference_the_transaction_would_see() {
        let installed = vec![pkg("a/b", "1.0.0")];
        let same = || vec![pkg("a/b", "1.0.0")];
        let decline = |packages: Vec<Value>, why: &str| {
            assert!(
                unchanged_local_repository(&installed, &lock(packages, vec![]), true).is_none(),
                "{why}"
            );
        };
        decline(vec![pkg("a/b", "1.1.0")], "a new version is an update");
        decline(
            vec![json!({"name": "a/b", "version": "1.0.0", "dist": {"reference": "zzz"}})],
            "a new dist reference is an update",
        );
        decline(
            vec![
                json!({"name": "a/b", "version": "1.0.0", "dist": {"reference": "abc"}, "source": {"reference": "s"}}),
            ],
            "a new source reference is an update",
        );
        decline(
            vec![
                json!({"name": "a/b", "version": "1.0.0", "dist": {"reference": "abc"}, "abandoned": true}),
            ],
            "becoming abandoned is an update",
        );
        decline(
            vec![pkg("e/f", "1.0.0")],
            "another package is install + uninstall",
        );
        decline(vec![], "an empty lock uninstalls");
        decline(
            vec![pkg("a/b", "1.0.0"), pkg("e/f", "1.0.0")],
            "one more package is an install",
        );
        // Aliases add operations of their own: never the fast path.
        let mut aliased = lock(same(), vec![]);
        aliased["aliases"] =
            json!([{"package": "a/b", "alias": "1.0", "alias_normalized": "1.0.0.0"}]);
        assert!(unchanged_local_repository(&installed, &aliased, true).is_none());
        let dev_alias = json!({"name": "a/b", "version": "dev-main", "dist": {"reference": "abc"},
                               "extra": {"branch-alias": {"dev-main": "1.x-dev"}}});
        assert!(
            unchanged_local_repository(
                std::slice::from_ref(&dev_alias),
                &lock(vec![dev_alias.clone()], vec![]),
                true
            )
            .is_none(),
            "a dev version with a branch-alias grows an alias package"
        );
        // A stable version keeps its `extra` and grows nothing.
        let stable_extra = json!({"name": "a/b", "version": "1.0.0", "dist": {"reference": "abc"},
                                  "extra": {"branch-alias": {"dev-main": "1.x-dev"}}});
        assert!(unchanged_local_repository(
            std::slice::from_ref(&stable_extra),
            &lock(vec![stable_extra.clone()], vec![]),
            true
        )
        .is_some());
        // A lock without `packages-dev` is the loader's error, not ours.
        assert!(
            unchanged_local_repository(&installed, &json!({"packages": same()}), true).is_none()
        );
    }
}
