//! `install`: lay out the lock — `InstallCommand`, the install half of
//! `Installer::run`, and the plugins vivacity emulates while doing it.

use super::*;

/// `Installer::doInstall` solves the lock again (`Solver.php:231`): its
/// duration at VERBOSE, after the stale-lock warning and the missing
/// requirements, before the problems or the operation count. Never for the
/// install that follows an update (`$alreadySolved`).
fn say_lock_resolution(since: std::time::Instant) {
    vivacity_core::say_verbose!(
        "Dependency resolution completed in {:.3} seconds",
        since.elapsed().as_secs_f64()
    );
}

/// What a phase of `run_install` that can end the command returns:
/// `Break(code)` is the exit code of the command, `Continue` carries what
/// the phases after it need.
type Flow<T = ()> = std::ops::ControlFlow<i32, T>;

/// `?` for a [`Flow`] inside `run_install`: the value it carries, or
/// `return Ok(code)`.
macro_rules! proceed {
    ($flow:expr) => {
        match $flow {
            std::ops::ControlFlow::Continue(value) => value,
            std::ops::ControlFlow::Break(code) => return Ok(code),
        }
    };
}

/// `InstallCommand` then `Installer::run` / `doInstall`, phase by phase.
/// The lock verification (the list filter's network wait, the platform,
/// the solve) runs on its own thread while the local-repository
/// transaction, the scope and (for a no-op) the autoloader are computed in
/// silence; it is joined before anything is printed or written past that
/// point, after the missing requirements, in `doInstall`'s order.
pub(crate) fn run_install(args: &InstallArgs) -> anyhow::Result<i32> {
    // `BaseCommand::initialize` loads the root package first: its version
    // warning precedes everything below. The install that follows an update
    // runs in the update's `Composer`, which already wrote it.
    if !args.after_update() {
        if let Ok(project) = project_dir(args.working_dir.as_deref()) {
            hold_root_version_warning(&project);
            hold_ignored_lock_warning(&project);
        }
    }
    // `InstallCommand` and `Factory::create`.
    let run = proceed!(read_manifests(args)?);
    let run = proceed!(merge_plugin(run)?);
    // `InstallCommand::execute` dispatches `PluginEvents::COMMAND` before
    // `Installer::run` (InstallCommand.php:111); `update`, `require` and
    // `remove` dispatch theirs before their resolution.
    if !args.after_update() {
        bamarni_command_event(&run.project, &run.manifest, !args.no_plugins);
    }
    // `Installer::run`.
    let run = proceed!(pre_install_cmd(run)?);
    // `Installer::doInstall`.
    let verify_start = proceed!(headline(&run)?);
    let verification = spawn_verification(&run)?;
    let root = verification.as_ref().map(|v| root_package(&run, v));
    let tx = local_repository_transaction(&run, root.as_ref())?;
    let scope = analyze_scope(&run);
    let early_dump = plan_autoload_early(&run, &tx, scope.as_ref());
    // `if (!$alreadySolved)`: the missing requirements (exit 4) before the
    // pool exists, then the solve on the locked pool (exit 2, or 4). None
    // of it after an update, which installs the lock it just solved.
    if let Some(verification) = verification {
        let mut verification = verification;
        proceed!(report_missing(&run, &tx.missing, &mut verification)?);
        proceed!(join_verification(&run, verification, verify_start)?);
    }
    report_operations(&tx);
    if args.dry_run {
        return dry_run(&run, &tx);
    }
    let layout = proceed!(native_layout(&run, scope)?);
    let installed = proceed!(execute_operations(&run, &tx, &layout)?);
    // `Installer::run`, after `doInstall`.
    let autoload_note = if args.no_autoloader {
        String::new()
    } else {
        proceed!(dump_autoloader(
            &run,
            &tx,
            &layout,
            &installed.report,
            early_dump
        )?)
    };
    proceed!(post_install(&run, &layout, &installed.report)?);
    say_summary(&run, &installed.report, &autoload_note);
    Ok(0)
}

/// What the phases of one install share, fixed before the first line is
/// printed: built by `read_manifests`, the root package replaced by
/// `merge_plugin`, the scripts runner set by `pre_install_cmd`.
struct Run<'a> {
    args: &'a InstallArgs,
    /// The start of the command: the `trace` origin, and the duration the
    /// closing line reports.
    t0: std::time::Instant,
    project: PathBuf,
    /// composer.json as written: the content-hash and `config.lock` read the
    /// file, not the root package below.
    manifest_text: String,
    /// The root package Composer works with: the effective
    /// `preferred-install` written in, then merged by composer-merge-plugin.
    manifest: serde_json::Value,
    merged: Option<vivacity_resolver::merge_plugin::Merged>,
    lock_value: serde_json::Value,
    lock: vivacity_core::lock::Lock,
    with_dev: bool,
    /// `--ignore-platform-reqs` / `--ignore-platform-req` as
    /// `BaseCommand::initialize` leaves them: the environment fills what the
    /// command line left empty.
    ignore_platform_reqs: bool,
    ignore_platform_req: Vec<String>,
    runner: Option<scripts::Runner>,
}

/// `InstallCommand`'s refusals, then the manifest (with the effective
/// `preferred-install`) and the lock as `Factory::create` reads them.
fn read_manifests(args: &InstallArgs) -> anyhow::Result<Flow<Run<'_>>> {
    let (ignore_platform_reqs, ignore_platform_req) = platform_requirement_options(args);
    if args.no_install {
        vivacity_core::say!("Invalid option \"--no-install\". Use \"composer update --no-install\" instead if you are trying to update the composer.lock file.");
        return Ok(Flow::Break(1));
    }
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    let manifest_path = project.join("composer.json");
    let lock_path = project.join("composer.lock");
    refuse_alternate_manifest()?;
    if let Some(code) = refuse_missing_manifest(&project) {
        return Ok(Flow::Break(code));
    }

    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("cannot read {}", manifest_path.display()))?;
    let manifest: serde_json::Value = match &args.virtual_manifest {
        Some(v) => v.clone(),
        None => serde_json::from_str(&manifest_text).context("invalid composer.json")?,
    };
    // Before the lock is even looked at: Composer refuses the manifest before
    // it reads the lock, so a stale lock is not the error to report.
    if let Some(code) = refuse_invalid_root(&manifest) {
        return Ok(Flow::Break(code));
    }
    // `--prefer-source` / `--prefer-dist` / `--prefer-install` are what
    // `getPreferredInstallOptions` makes of them: the effective
    // `preferred-install` this run uses. Written into the manifest vivacity
    // analyses, so the scope detector judges it per package exactly as it
    // judges the config — a source install then hands the command over.
    let mut manifest = manifest;
    match vivacity_core::scope::effective_preferred_install(
        &manifest,
        args.prefer_source,
        args.prefer_dist,
        args.prefer_install.as_deref(),
    ) {
        Err(message) => {
            vivacity_core::say_always!("{message}");
            return Ok(Flow::Break(1));
        }
        Ok(Some(value)) => {
            if let Some(config) = manifest.get_mut("config").and_then(|c| c.as_object_mut()) {
                config.insert("preferred-install".to_owned(), value);
            } else if let Some(root) = manifest.as_object_mut() {
                root.insert(
                    "config".to_owned(),
                    serde_json::json!({"preferred-install": value}),
                );
            }
        }
        Ok(None) => {}
    }
    let manifest = manifest;
    // Parsed once: the `Value` serves the transaction, the abandoned
    // warnings and the missing-requirement check; the `Lock` everything else.
    let lock_value: serde_json::Value = match &args.virtual_lock {
        // A dry-run update (or one that writes no lock) installs from its
        // unwritten lock.
        Some(v) => serde_json::from_str(&vivacity_core::phpjson::php_json_encode_with(
            v,
            vivacity_core::phpjson::FLAGS_JSONFILE,
        )?)
        .context("invalid composer.lock")?,
        None => {
            // `Installer::run`: without a lock to install (`isLocked()`
            // false — `config.lock: false`, whose `Locker` reads
            // `/dev/null`, no file, no `packages`), the install updates. An
            // unreadable lock is Composer's ParsingException.
            let enabled = config_lock_enabled(&manifest_text);
            let text = match std::fs::read_to_string(&lock_path) {
                Ok(text) if enabled => text,
                Err(e) if enabled && e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(anyhow::Error::new(e)
                        .context(format!("cannot read {}", lock_path.display())));
                }
                _ => return Ok(Flow::Break(install_without_lock(args, enabled)?)),
            };
            let value: serde_json::Value =
                serde_json::from_str(&text).context("invalid composer.lock")?;
            if !value.get("packages").is_some_and(|p| !p.is_null()) {
                return Ok(Flow::Break(install_without_lock(args, enabled)?));
            }
            value
        }
    };
    let lock = vivacity_core::lock::Lock::from_value(&lock_value);
    trace("read manifests", t0);

    let with_dev = !args.no_dev && !env_no_dev();
    Ok(Flow::Continue(Run {
        args,
        t0,
        project,
        manifest_text,
        manifest,
        merged: None,
        lock_value,
        lock,
        with_dev,
        ignore_platform_reqs,
        ignore_platform_req,
        runner: None,
    }))
}

/// `Installer::run` (Installer.php:277-285) when there is no lock to
/// install: it says so, then runs as `update` with install — the lock
/// written unless `config.lock` is false, in which case the install phase
/// reads it from memory. Entered with `install`'s own options only (none of
/// the `COMPOSER_PREFER_*`/`COMPOSER_WITH_*` variables `update` reads).
/// `--check-scope` (no network, no write) and `--run-scripts` (update's
/// scripts, which vivacity's `update` does not run) stop or hand over.
fn install_without_lock(args: &InstallArgs, lock_enabled: bool) -> anyhow::Result<i32> {
    let line = format!(
        "{} Updating dependencies to latest instead of installing from lock file. See https://getcomposer.org/install for more information.",
        if lock_enabled {
            "No composer.lock file present."
        } else {
            "Lockfile creation is disabled in config."
        }
    );
    if args.check_scope || args.run_scripts {
        let report = vivacity_core::scope::ScopeReport {
            issues: vec![vivacity_core::scope::ScopeIssue::Layout(format!(
                "no lock to install: Composer resolves instead ({})",
                if lock_enabled {
                    "no composer.lock"
                } else {
                    "config.lock is false"
                }
            ))],
            ..Default::default()
        };
        return fallback_or_fail(args, &report);
    }
    let project = project_dir(args.working_dir.as_deref())?;
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(project.join("composer.json"))?)
            .context("invalid composer.json")?;
    if let Some(code) = resolution_fallback(
        &project,
        &manifest,
        vivacity_core::scope::ResolutionCommand::Update,
        args.no_plugins,
        args.no_fallback,
        !args.dry_run,
    )? {
        return Ok(code);
    }
    let update_args = UpdateArgs {
        compat: GlobalCompatArgs {
            no_interaction: args.compat.no_interaction,
            no_ansi: args.compat.no_ansi,
            ..GlobalCompatArgs::default()
        },
        no_progress: args.no_progress,
        packages: Vec::new(),
        with: Vec::new(),
        with_dependencies: false,
        with_all_dependencies: false,
        no_install: false,
        dry_run: args.dry_run,
        no_dev: args.no_dev,
        no_autoloader: args.no_autoloader,
        optimize_autoloader: args.optimize_autoloader,
        classmap_authoritative: args.classmap_authoritative,
        apcu_autoloader: args.apcu_autoloader,
        apcu_autoloader_prefix: args.apcu_autoloader_prefix.clone(),
        no_scripts: args.no_scripts,
        no_plugins: args.no_plugins,
        // `install` audits only on `--audit`.
        no_audit: true,
        no_blocking: args.no_blocking,
        no_security_blocking: args.no_security_blocking,
        prefer_stable: false,
        prefer_lowest: false,
        ignore_platform_reqs: args.ignore_platform_reqs,
        ignore_platform_req: args.ignore_platform_req.clone(),
        no_fallback: args.no_fallback,
        offline: args.offline,
        working_dir: args.working_dir.clone(),
        prefer_source: args.prefer_source,
        prefer_dist: args.prefer_dist,
        prefer_install: args.prefer_install.clone(),
        command_dispatched: false,
        lockless_install: Some(line),
    };
    let options = vivacity_resolver::session::UpdateOptions {
        no_blocking: args.no_blocking || args.no_security_blocking,
        ..Default::default()
    };
    run_update_resolved(&update_args, options, false, false, &|| Ok(())).map(|e| e.status)
}

/// `BaseCommand::initialize`'s reading of the environment
/// ([`platform_requirement_env`]). The install that follows an update reads
/// it without warning: the update already did.
fn platform_requirement_options(args: &InstallArgs) -> (bool, Vec<String>) {
    platform_requirement_env(
        args.ignore_platform_reqs,
        &args.ignore_platform_req,
        !args.after_update(),
    )
}

/// `wikimedia/composer-merge-plugin`: the root package Composer works
/// with is the merged one (INIT, before any output — a merge error
/// fails Composer inside Factory::create). Everything after reads the
/// merged manifest; the content-hash keeps reading the file.
fn merge_plugin(run: Run<'_>) -> anyhow::Result<Flow<Run<'_>>> {
    let merged = match merge_plugin_root(
        &run.project,
        &run.lock,
        &run.manifest,
        run.with_dev,
        !run.args.no_plugins,
    ) {
        Ok(m) => m,
        Err(reason) => {
            let report = vivacity_core::scope::ScopeReport {
                issues: vec![vivacity_core::scope::ScopeIssue::MergePlugin(reason)],
                ..Default::default()
            };
            return Ok(Flow::Break(fallback_or_fail(run.args, &report)?));
        }
    };
    let manifest = match &merged {
        Some(m) => m.manifest.clone(),
        None => run.manifest,
    };
    Ok(Flow::Continue(Run {
        manifest,
        merged,
        ..run
    }))
}

/// `--run-scripts`: the declared events go to `composer run-script` at
/// Composer's own points; `pre-install-cmd` fires first in
/// `Installer::run` (after the command's `COMMAND` event, before the
/// headline and the lock validation).
fn pre_install_cmd(run: Run<'_>) -> anyhow::Result<Flow<Run<'_>>> {
    let args = run.args;
    let runner = if args.run_scripts && !args.no_scripts {
        match scripts::Runner::new(&run.project, &run.manifest, run.with_dev, which_composer()) {
            Ok(r) => r,
            Err(e) => {
                vivacity_core::say_always!("vivacity: {e}");
                return Ok(Flow::Break(3));
            }
        }
    } else {
        None
    };
    if let Some(r) = &runner {
        let code = r.run(scripts::PRE_INSTALL_CMD)?;
        if code != 0 {
            return Ok(Flow::Break(code));
        }
    }
    Ok(Flow::Continue(Run { runner, ..run }))
}

/// bamarni/composer-bin-plugin's `COMMAND` listener: its deprecation
/// lines, when the plugin is already installed and allowed (loaded from
/// installed.json before the command). Every command that dispatches the
/// event calls this at the point where Composer's command does.
pub(crate) fn bamarni_command_event(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) {
    if vivacity_core::scope::active_plugins(project, manifest, plugins_enabled)
        .iter()
        .any(|p| p == vivacity_core::bamarni_bin::PLUGIN_NAME)
    {
        if let Ok(cfg) = vivacity_core::bamarni_bin::config(manifest.get("extra")) {
            vivacity_core::bamarni_bin::print_deprecations(&cfg);
        }
    }
}

/// `Installer::doInstall`: the headline, the locked repository (which a lock
/// without `packages-dev` refuses in dev mode), then the platform
/// verification notice (the lock is solved against the platform when not
/// coming straight from an update), then the freshness warning. Returns
/// when the verification started, for the resolution line.
fn headline(run: &Run<'_>) -> anyhow::Result<Flow<std::time::Instant>> {
    let config_lock = config_lock_enabled(&run.manifest_text);
    if config_lock {
        vivacity_core::say!(
            "Installing dependencies from lock file{}",
            if run.with_dev {
                " (including require-dev)"
            } else {
                ""
            }
        );
    }
    // `$this->locker->getLockedRepository($this->devMode)` (Locker.php:157-165):
    // `isset($lockData['packages-dev'])`, else a RuntimeException — exit 1,
    // its message shown even under `-q`, before "Verifying…". Printed
    // without Symfony Console's box, like `RootPackageLoader`'s refusals.
    // An update's lock always has the key.
    if run.with_dev && !run.args.after_update() {
        refuse_lock_without_dev(run)?;
    }
    // The resolution line `-v` shows measures from here to the join: the
    // solve runs on the verification thread, alongside the local
    // repository's transaction.
    let verify_start = std::time::Instant::now();
    if !run.args.after_update() {
        vivacity_core::say!("Verifying lock file contents can be installed on current platform.");
        // Lock freshness: same behaviour as Composer, a warning.
        if let (Ok(actual), Some(expected)) = (
            vivacity_core::content_hash::content_hash(&run.manifest_text),
            run.lock.content_hash.as_deref(),
        ) {
            if actual != expected {
                vivacity_core::say_always!(
                    "Warning: The lock file is not up to date with the latest changes in composer.json. \
                     You may be getting outdated dependencies. It is recommended that you run `composer update` or `composer update <package name>`."
                );
            }
        }
    }
    Ok(Flow::Continue(verify_start))
}

/// `Locker::getLockedRepository(true)` on a lock without `packages-dev`
/// (`isset` false: the key absent or null): Composer's exception.
fn refuse_lock_without_dev(run: &Run<'_>) -> anyhow::Result<()> {
    if run
        .lock_value
        .get("packages-dev")
        .is_none_or(serde_json::Value::is_null)
    {
        return Err(anyhow::Error::new(ComposerException(
            vivacity_resolver::repository::LOCK_WITHOUT_DEV.to_owned(),
        )));
    }
    Ok(())
}

/// The lock verification running on its own thread, and the channel the
/// root version reaches it by. Dropped without being joined (the command
/// stopped before the join), it tells the thread to stop after its network
/// wait.
struct Verification {
    handle: Option<
        std::thread::JoinHandle<
            Result<
                vivacity_resolver::session::InstallVerification,
                vivacity_resolver::session::SessionError,
            >,
        >,
    >,
    root_version: Option<std::sync::mpsc::Sender<vivacity_core::root_version::RootVersion>>,
    abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// `--ignore-platform-reqs` (or `--ignore-platform-req=*`): no warning
    /// without php.
    ignores_all: bool,
    /// The lock has platform requirements to check (the warning without
    /// php).
    lock_platform: bool,
}

impl Verification {
    /// The command stops (or hands over) before the join: the thread skips
    /// what is left after its network wait.
    fn abandon(&self) {
        self.abandoned
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Abandons the thread and waits for it, its result dropped: before a
    /// Composer process is started, so that the two never write the
    /// repository cache at the same time (the thread may be in its network
    /// wait, about to write `packages.json` or the filter summary).
    fn abandon_and_wait(&mut self) {
        self.abandon();
        drop(self.root_version.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Verification {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.abandon();
        }
    }
}

/// `Installer::doInstall` solves the lock again before laying it out
/// (`session::verify_install`): the locked packages through the list
/// filter in install scope (one conditional request per install,
/// Composer's `loadFilterSummary`, `packages.json` cached 600 s — the only
/// network wait of an install from lock), then the platform probed and the
/// solve. On its own thread while the transaction and the scope are
/// computed; joined before anything is printed or written past this point.
/// None after an update (`$alreadySolved`).
fn spawn_verification(run: &Run<'_>) -> anyhow::Result<Option<Verification>> {
    let args = run.args;
    if args.after_update() {
        return Ok(None);
    }
    let abandoned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let input = vivacity_resolver::session::InstallVerifyInput {
        project_dir: run.project.clone(),
        composer_home: vivacity_core::fetch::composer_home(),
        http: Some(http_transport(&run.project, args.offline)?),
        cache_repo_dir: Some(vivacity_core::fetch::composer_cache_dir().join("repo")),
        manifest_text: run.manifest_text.clone(),
        root_manifest: run.manifest.clone(),
        dev_mode: run.with_dev,
        no_blocking: args.no_blocking || args.no_security_blocking,
        ignore_platform_reqs: run.ignore_platform_reqs,
        ignore_platform_req: run.ignore_platform_req.clone(),
        // `prependRepositories` runs when the plugin is activated: on a bare
        // vendor it is not yet, and `doInstall`'s filter never sees them.
        prepended_repositories: run
            .merged
            .as_ref()
            .filter(|_| {
                installed_packages(&run.project, &run.manifest)
                    .iter()
                    .any(|p| {
                        p.get("name").and_then(serde_json::Value::as_str)
                            == Some(vivacity_resolver::merge_plugin::PLUGIN_NAME)
                    })
            })
            .map(|m| m.repository_definitions.clone())
            .unwrap_or_default(),
        abandoned: abandoned.clone(),
    };
    let (root_version, received) = std::sync::mpsc::channel();
    let handle =
        std::thread::spawn(move || vivacity_resolver::session::verify_install(input, received));
    Ok(Some(Verification {
        handle: Some(handle),
        root_version: Some(root_version),
        abandoned,
        ignores_all: run.ignore_platform_reqs || run.ignore_platform_req.iter().any(|p| p == "*"),
        lock_platform: !run.lock.platform.is_empty()
            || (run.with_dev && !run.lock.platform_dev.is_empty()),
    }))
}

/// The root package (`RootPackageLoader`) the missing requirements are
/// checked against: its version guessed once (git), and handed to the
/// verification thread, which waits for it only after its network wait.
fn root_package(run: &Run<'_>, verification: &Verification) -> vivacity_core::state::RootPackage {
    let rv = vivacity_core::root_version::detect(&run.manifest, &run.project);
    // A thread that already ended (an error) has dropped its receiver: its
    // error is reported at the join.
    if let Some(sender) = &verification.root_version {
        let _ = sender.send(rv.clone());
    }
    vivacity_core::state::RootPackage::with_version(&run.manifest, rv, run.with_dev)
}

/// The local repository against the lock, and the root requirements the
/// lock does not satisfy.
struct LocalTransaction {
    /// The packages of both repositories (left empty by the no-op proof).
    arena: Vec<vivacity_resolver::package::Package>,
    /// installed.json's packages, as arena indices.
    present: Vec<usize>,
    /// The no-op proof: the present names, in installed.json order.
    unchanged_names: Option<Vec<String>>,
    transaction: vivacity_resolver::transaction::Transaction,
    /// `Locker::getMissingRequirementInfo`'s lines (empty: nothing missing).
    missing: Vec<String>,
}

/// `LocalRepoTransaction` (installed.json against the lock) and
/// `Locker::getMissingRequirementInfo`, computed in silence: reported by
/// `report_missing`, `report_operations` and `dry_run`.
fn local_repository_transaction(
    run: &Run<'_>,
    root: Option<&vivacity_core::state::RootPackage>,
) -> anyhow::Result<LocalTransaction> {
    let mut arena: Vec<vivacity_resolver::package::Package> = Vec::new();
    // The local repository: installed.json purged of the packages whose
    // install path is gone (`Factory::purgePackages`).
    let mut installed_now = installed_packages(&run.project, &run.manifest);
    // A `symfony-pack` laid out by Flex's pack installer has no install
    // path in installed.json (`MetapackageInstaller`), which the purge above
    // reads as "always installed". `Factory::createComposer` loads the
    // installed plugins (`loadInstalledPlugins`) before `purgePackages`:
    // without Flex among them (`--no-plugins`, Flex not installed or not
    // allowed), the pack's installer is `LibraryInstaller`, whose
    // `isInstalled` asks for `<vendor-dir>/<name>` (`Filesystem::isReadable`:
    // a file would do, a dangling link would not) — absent, the pack is not
    // installed and the run installs it.
    if !vivacity_core::scope::active_plugins(&run.project, &run.manifest, !run.args.no_plugins)
        .iter()
        .any(|p| p == "symfony/flex")
    {
        let vendor = vivacity_core::dirs::Dirs::resolve(&run.manifest)
            .unwrap_or_default()
            .vendor_dir(&run.project);
        installed_now.retain(|p| {
            let pack = p.get("type").and_then(serde_json::Value::as_str) == Some("symfony-pack");
            let no_path = p.get("install-path").is_none_or(serde_json::Value::is_null);
            let name = p
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            !(pack && no_path && !vendor.join(name).exists())
        });
    }
    trace("installed.json", run.t0);
    // Nothing to do at all (the commonest install): proved on the raw
    // entries, so the two repositories are not loaded into the arena —
    // ~8 ms of constraint parsing on a 100-package lock. `None` falls back
    // to the full computation.
    let unchanged_names = unchanged_local_repository(&installed_now, &run.lock_value, run.with_dev);
    let (present, result) = if unchanged_names.is_some() {
        (Vec::new(), Vec::new())
    } else {
        let as_lock = serde_json::json!({"packages": installed_now, "packages-dev": []});
        let present =
            vivacity_resolver::repository::locked_repository_with(&as_lock, &mut arena, true)
                .map_err(|e| anyhow::anyhow!("installed.json: {}", e.0))?;
        let result = vivacity_resolver::repository::locked_repository_with(
            &run.lock_value,
            &mut arena,
            run.with_dev,
        )
        .map_err(|e| anyhow::anyhow!("composer.lock: {}", e.0))?;
        (present, result)
    };
    // `Locker::getMissingRequirementInfo`: a root requirement the lock does
    // not satisfy (a hand-edited composer.json) stops with code 4
    // (`report_missing`). Not after an update (`$alreadySolved`): no root
    // was detected for it then.
    let merged_links = run
        .merged
        .as_ref()
        .map(|m| (m.requires.as_slice(), m.requires_dev.as_slice()));
    let missing = match root {
        None => Vec::new(),
        Some(root) => missing_requirement_info(
            &run.manifest,
            &run.lock_value,
            run.with_dev,
            &root.version,
            &root.pretty_version,
            merged_links,
        )?,
    };
    trace("local repositories", run.t0);
    let transaction = vivacity_resolver::transaction::Transaction::new(&arena, &present, &result);
    Ok(LocalTransaction {
        arena,
        present,
        unchanged_names,
        transaction,
        missing,
    })
}

/// The scope analysis is pure (lock, manifest, installed.json, global
/// config): computed here, reported after the join. `None` for a dry run.
fn analyze_scope(run: &Run<'_>) -> Option<vivacity_core::scope::ScopeReport> {
    let args = run.args;
    let scope = if args.dry_run {
        None
    } else {
        Some(vivacity_core::scope::analyze(
            &run.project,
            &run.lock,
            &run.manifest,
            run.with_dev,
            !args.no_plugins,
        ))
    };

    trace("scope analysis", run.t0);
    scope
}

/// The autoloader, planned during the network wait when the install has
/// nothing to place (every wanted package present, same identity): its
/// inputs are then all known — the local repository the install would
/// produce, the manifest, the layout — and `plan` writes nothing under
/// vendor/. Kept as a `Result` and only looked at where the dump runs
/// today, so an error surfaces at the same point with the same text.
/// Not with `--run-scripts` (pre-autoload-dump may edit sources).
fn plan_autoload_early(
    run: &Run<'_>,
    tx: &LocalTransaction,
    scope: Option<&vivacity_core::scope::ScopeReport>,
) -> Option<anyhow::Result<PlannedDump>> {
    let args = run.args;
    let early_dump: Option<anyhow::Result<PlannedDump>> = if args.dry_run
        || args.no_autoloader
        || run.runner.is_some()
        || !tx.transaction.operations.is_empty()
    {
        None
    } else {
        scope
            .filter(|s| s.is_native_ok())
            .and_then(|s| s.layout.as_ref())
            .and_then(|layout| {
                vivacity_core::installer::local_repository_if_unchanged(
                    &run.lock,
                    layout,
                    run.with_dev,
                )
                .map(|local| {
                    dump_autoload_plan(
                        &run.project,
                        &local,
                        &run.manifest,
                        layout,
                        run.with_dev,
                        args.optimize_autoloader || args.classmap_authoritative,
                        args.classmap_authoritative,
                        run.ignore_platform_reqs,
                        &run.ignore_platform_req,
                        (args.apcu_autoloader, args.apcu_autoloader_prefix.as_deref()),
                        !args.no_plugins,
                        run.lock
                            .packages_dev
                            .iter()
                            .map(|p| p.name().to_owned())
                            .collect(),
                    )
                })
            })
    };
    trace("autoload planned", run.t0);
    early_dump
}

/// The verification joined, reported as `doInstall` does: the list
/// filter's warnings (written while the pool was built), the resolution
/// line (`-v`), then the problems (`SolverProblemsException`, exit
/// `max(1, 2)`) or the operations a solved lock should not have (exit 4).
fn join_verification(
    run: &Run<'_>,
    verification: Verification,
    verify_start: std::time::Instant,
) -> anyhow::Result<Flow> {
    use vivacity_resolver::session::InstallVerdict;
    let mut verification = verification;
    let Some(handle) = verification.handle.take() else {
        return Ok(Flow::Continue(()));
    };
    // Sent before the join already; dropped so that a thread still waiting
    // could never wait forever.
    drop(verification.root_version.take());
    let verified = match handle
        .join()
        .map_err(|_| anyhow::anyhow!("the lock verification thread panicked"))?
    {
        Ok(v) => v,
        Err(e) => {
            // Written by Composer before it threw (a degraded repository).
            for line in &e.preface {
                vivacity_core::say!("{line}");
            }
            return Err(session_error(e));
        }
    };
    trace("lock verification", run.t0);
    for w in &verified.warnings {
        vivacity_core::say!("{w}");
    }
    // No reference here: Composer does not run without php.
    if verified.php_missing && !verification.ignores_all && verification.lock_platform {
        vivacity_core::say!(
            "Warning: php not found, platform requirements were not checked \
             (--ignore-platform-reqs silences this warning)"
        );
    }
    say_lock_resolution(verify_start);
    match verified.verdict {
        InstallVerdict::Installable => Ok(Flow::Continue(())),
        InstallVerdict::Problems(pretty) => {
            // The headline at QUIET, the problems at the default level.
            let head = "Your lock file does not contain a compatible set of packages. Please run composer update.";
            vivacity_core::say_always!("{head}");
            vivacity_core::say!("{pretty}");
            // `$ghe->emit($err."\n".$prettyProblem)` (Installer.php:829).
            vivacity_core::io::github_action_error(&format!("{head}\n{pretty}"));
            Ok(Flow::Break(2))
        }
        InstallVerdict::NeedsChanges => {
            vivacity_core::say_always!("Your lock file cannot be installed on this system without changes. Please run composer update.");
            Ok(Flow::Break(4))
        }
        // The exception leaves `doInstall` after the resolution line.
        InstallVerdict::Invalid(message) => Err(anyhow::anyhow!("{message}")),
    }
}

/// The root requirements the lock does not satisfy: printed, then exit 4
/// unless `allow-missing-requirements`.
fn report_missing(
    run: &Run<'_>,
    missing: &[String],
    verification: &mut Verification,
) -> anyhow::Result<Flow> {
    if !missing.is_empty() {
        // On a bare vendor the plugin is not active yet in Composer's run:
        // it would install itself, then run a partial `composer update` of
        // the merged requirements and rewrite the lock. vivacity does not
        // do that on a user's behalf: the lock goes to Composer, with the
        // reason. With the plugin installed, Composer refuses like this.
        if let Some(m) = &run.merged {
            let plugin_installed =
                installed_packages(&run.project, &run.manifest)
                    .iter()
                    .any(|p| {
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
                verification.abandon_and_wait();
                return Ok(Flow::Break(fallback_or_fail(run.args, &report)?));
            }
        }
        for line in missing {
            vivacity_core::say!("{line}");
        }
        if !require::truthy(require::config_value(
            &run.manifest,
            "allow-missing-requirements",
        )) {
            verification.abandon();
            return Ok(Flow::Break(4));
        }
    }
    Ok(Flow::Continue(()))
}

/// `Package operations: …` (the operation list itself at `-v`), or
/// `Nothing to install, update or remove`.
fn report_operations(tx: &LocalTransaction) {
    use vivacity_resolver::transaction::Operation;
    let ops = &tx.transaction.operations;
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
        vivacity_core::say!("Nothing to install, update or remove");
    } else {
        vivacity_core::say!(
            "Package operations: {installs} install{}, {updates} update{}, {removals} removal{}",
            if installs == 1 { "" } else { "s" },
            if updates == 1 { "" } else { "s" },
            if removals == 1 { "" } else { "s" }
        );
        for line in vivacity_resolver::transaction::verbose_operation_summary(ops, &tx.arena) {
            vivacity_core::say_verbose!("{line}");
        }
    }
}

/// A dry run stops after the operations: their lines in transaction order,
/// then (an update's suggestions and) the abandoned packages, and the
/// funding count — no dump, and no `POST_*_CMD` (`runScripts` is off).
/// 0, unless the lock has no `packages-dev` ([`report_before_dump`]).
fn dry_run(run: &Run<'_>, tx: &LocalTransaction) -> anyhow::Result<i32> {
    for op in &tx.transaction.operations {
        if let Some(line) = op.show(&tx.arena, false) {
            vivacity_core::say!("  - {line}");
        }
    }
    // `Installer::run` after `doInstall`.
    report_before_dump(run)?;
    print_funding(&run.project, &run.manifest);
    Ok(0)
}

/// `Installer::run` after `doInstall`/`doUpdate`, before the dump
/// (Installer.php:343-373): an update's suggestions and the abandoned
/// packages of its new lock, precomputed by the update, which knows the
/// operations; for an install, the abandoned packages of the lock (dev
/// included).
fn report_before_dump(run: &Run<'_>) -> anyhow::Result<()> {
    let say_all = |lines: &[String]| {
        for line in lines {
            vivacity_core::say!("{line}");
        }
    };
    match &run.args.post_update {
        Some(report) => say_all(&report.lines),
        None => {
            // `$this->locker->getLockedRepository(true)` (Installer.php:356),
            // dev mode or not: a `--no-dev` install of a lock without
            // `packages-dev` fails here, its packages laid out, before the
            // autoloader.
            refuse_lock_without_dev(run)?;
            say_all(&abandoned_warnings(&run.lock_value));
        }
    }
    Ok(())
}

/// Out of scope: Composer as fallback (default) or fail explicitly. In
/// scope: the notes on what is emulated or skipped, then the layout the
/// install places packages by (`--check-scope` stops here).
fn native_layout(
    run: &Run<'_>,
    scope: Option<vivacity_core::scope::ScopeReport>,
) -> anyhow::Result<Flow<vivacity_core::layout::Layout>> {
    let Some(scope) = scope else {
        anyhow::bail!("internal: scope was not analysed");
    };
    if !scope.is_native_ok() {
        return Ok(Flow::Break(fallback_or_fail(run.args, &scope)?));
    }
    let Some(layout) = scope.layout else {
        anyhow::bail!("internal: scope is native but no layout was resolved");
    };
    if let Some(tag) = &layout.installers_tag {
        vivacity_core::say!(
            "Note: composer/installers {tag} emulated natively (custom install paths)"
        );
    }
    for plugin in &scope.skipped_plugins {
        vivacity_core::say!(
            "Note: plugin {plugin} installed as a plain library (vivacity never runs plugins)"
        );
    }
    trace("scope", run.t0);
    if run.args.check_scope {
        vivacity_core::say_always!("vivacity: this lock is installed natively");
        return Ok(Flow::Break(0));
    }
    Ok(Flow::Continue(layout))
}

/// The installer's report, and the runtime it ran on: kept until the end
/// of `run_install`, where it always was dropped (dropping a runtime waits
/// for its blocking tasks).
struct Installed {
    report: vivacity_core::installer::InstallReport,
    _runtime: tokio::runtime::Runtime,
}

/// The operations: their lines decided on the state before the
/// transaction, the transaction itself, then the lines (codeception/c3's
/// `uninstall` before its own); then, as `Installer::run` prints them
/// before the dump, an update's suggestions and the abandoned packages.
fn execute_operations(
    run: &Run<'_>,
    tx: &LocalTransaction,
    layout: &vivacity_core::layout::Layout,
) -> anyhow::Result<Flow<Installed>> {
    let args = run.args;
    // The operation lines of a real install, in transaction order, with the
    // downloader's appendix (`getInstallOperationAppendix`: `: Extracting
    // archive`, `: Symlinking from …`, `: Mirroring from …`, `: Source
    // already present`) computed on the state before the transaction, like
    // Composer does right before each operation. They are printed once the
    // transaction has succeeded: the placements run in parallel, and a
    // failure is reported alone.
    let operation_lines = operation_lines(
        &run.project,
        &tx.arena,
        &tx.transaction.operations,
        layout,
        run.lock
            .flex_packs(&run.manifest, run.with_dev, !args.no_plugins),
        !args.no_plugins,
    )?;
    // Composer decides these before it fetches anything; computed here for the
    // same reason `operation_lines` is — our own fetch fills that cache.
    let downloading = download_lines(
        &run.lock,
        &tx.arena,
        &tx.transaction.operations,
        run.with_dev,
    );

    // Transaction.
    let store = Arc::new(vivacity_core::store::Store::default_location());
    let auth = vivacity_core::fetch::Auth::load(&run.project);
    let fetcher = Arc::new(vivacity_core::fetch::Fetcher::new(
        vivacity_core::fetch::composer_cache_dir(),
        auth,
    )?);
    let opts = vivacity_core::installer::InstallOptions {
        with_dev: run.with_dev,
        offline: args.offline,
        plugins_enabled: !args.no_plugins,
        ..Default::default()
    };
    let runtime = tokio::runtime::Runtime::new().context("cannot start the async runtime")?;
    let report = match runtime.block_on(vivacity_core::installer::install(
        &run.project,
        &run.lock,
        &run.manifest,
        layout,
        store,
        fetcher,
        &opts,
    )) {
        Ok(r) => r,
        // Emulation refusal detected before any write: vendor/ is intact.
        Err(vivacity_core::Error::Unsupported(msg)) => {
            let scope = vivacity_core::scope::ScopeReport {
                issues: vec![vivacity_core::scope::ScopeIssue::Layout(msg)],
                skipped_plugins: vec![],
                layout: None,
            };
            return Ok(Flow::Break(fallback_or_fail(args, &scope)?));
        }
        Err(vivacity_core::Error::Refused(msg)) => {
            vivacity_core::say_always!("{msg}");
            return Ok(Flow::Break(1));
        }
        Err(e) => return Err(e.into()),
    };
    trace("install transaction", run.t0);
    for line in &downloading {
        vivacity_core::say!("{line}");
    }
    // codeception/c3's `uninstall` when this transaction removes the plugin:
    // `PluginInstaller::uninstall` calls it before `LibraryInstaller`
    // removes the code and prints the operation's line.
    let c3_leaves = tx.transaction.operations.iter().any(|op| {
        matches!(*op, vivacity_resolver::transaction::Operation::Uninstall(p)
            if tx.arena[p].name == vivacity_core::c3_plugin::PLUGIN_NAME)
    });
    let c3_line = format!("  - Removing {} (", vivacity_core::c3_plugin::PLUGIN_NAME);
    let mut c3_uninstalled = false;
    for line in &operation_lines {
        if c3_leaves && !c3_uninstalled && line.starts_with(&c3_line) {
            emulate_c3_uninstall(&run.project, &run.manifest, !args.no_plugins)?;
            c3_uninstalled = true;
        }
        vivacity_core::say!("{line}");
    }
    if c3_leaves && !c3_uninstalled {
        emulate_c3_uninstall(&run.project, &run.manifest, !args.no_plugins)?;
    }
    // `BinaryInstaller` notices belong to the operations, before the dump.
    for m in &report.messages {
        vivacity_core::say!("{m}");
    }
    report_before_dump(run)?;
    Ok(Flow::Continue(Installed {
        report,
        _runtime: runtime,
    }))
}

/// `Installer::run`'s dump, after `doInstall`: the "Generating autoload
/// files" line, pre-autoload-dump, the autoloader (the plan made during the
/// network wait, or made now), the emulated post-autoload-dump plugins,
/// post-autoload-dump. Returns the closing line's autoloader note.
fn dump_autoloader(
    run: &Run<'_>,
    tx: &LocalTransaction,
    layout: &vivacity_core::layout::Layout,
    report: &vivacity_core::installer::InstallReport,
    early_dump: Option<anyhow::Result<PlannedDump>>,
) -> anyhow::Result<Flow<String>> {
    let args = run.args;
    // `Installer::run` (Installer.php:377-381), before
    // `AutoloadGenerator::dump` dispatches PRE_AUTOLOAD_DUMP: "optimized"
    // as soon as the effective optimize flag is on — the option,
    // `config.optimize-autoloader`, or class-map authoritative (which
    // implies it), the same computation `dump_autoload_plan` makes.
    vivacity_core::say!(
        "Generating{} autoload files",
        if effective_optimize(
            &run.manifest,
            args.optimize_autoloader,
            args.classmap_authoritative
        ) {
            " optimized"
        } else {
            ""
        }
    );
    if let Some(r) = &run.runner {
        let code = r.run(scripts::PRE_AUTOLOAD_DUMP)?;
        if code != 0 {
            return Ok(Flow::Break(code));
        }
    }
    // `AutoloadGenerator::dump($localRepo)`: the local repository, not
    // the lock (they differ for an unchanged package whose lock entry
    // moved).
    let local = report.local_repository.as_ref().unwrap_or(&run.lock);
    let planned = match early_dump {
        // Planned during the network wait: the install placed nothing,
        // the inputs are what they were.
        Some(planned) => planned?,
        None => dump_autoload_plan(
            &run.project,
            local,
            &run.manifest,
            layout,
            run.with_dev,
            args.optimize_autoloader || args.classmap_authoritative,
            args.classmap_authoritative,
            run.ignore_platform_reqs,
            &run.ignore_platform_req,
            (args.apcu_autoloader, args.apcu_autoloader_prefix.as_deref()),
            !args.no_plugins,
            // `$localRepo->setDevPackageNames($this->locker->getDevPackageNames())`
            run.lock
                .packages_dev
                .iter()
                .map(|p| p.name().to_owned())
                .collect(),
        )?,
    };
    let dumped = planned.finish(&run.project, &run.manifest, layout)?;
    let autoload_note = format!(", autoloader with {} classes", dumped.classes);
    post_autoload_dump_plugins(run, tx, local)?;
    // `AutoloadGenerator::dump`: post-autoload-dump once the files are
    // written (the emulated post-autoload-dump plugins included).
    if let Some(r) = &run.runner {
        let code = r.run(scripts::POST_AUTOLOAD_DUMP)?;
        if code != 0 {
            return Ok(Flow::Break(code));
        }
    }
    trace("autoload dump", run.t0);
    Ok(Flow::Continue(autoload_note))
}

/// Plugins listening to post-autoload-dump, emulated: the local
/// repository's order is the previous installed.json order minus
/// the removed and updated packages, then the operations' order.
fn post_autoload_dump_plugins(
    run: &Run<'_>,
    tx: &LocalTransaction,
    local: &vivacity_core::lock::Lock,
) -> anyhow::Result<()> {
    use vivacity_resolver::transaction::Operation;
    let args = run.args;
    let arena = &tx.arena;
    let previous: Vec<&str> = match &tx.unchanged_names {
        Some(names) => names.iter().map(String::as_str).collect(),
        None => tx.present.iter().map(|&i| arena[i].name.as_str()).collect(),
    };
    let mut gone: Vec<&str> = Vec::new();
    let mut fresh: Vec<&str> = Vec::new();
    for op in &tx.transaction.operations {
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
    let order = vivacity_core::pest_plugin::local_repository_order(&previous, &gone, &fresh);
    let previously_present = previous.contains(&vivacity_core::pest_plugin::PLUGIN_NAME);
    emulate_pest_plugin(
        &run.project,
        local,
        &run.manifest,
        run.with_dev,
        &order,
        !args.no_plugins,
        previously_present,
    )?;
    // yiisoft/yii2-composer: `activate` then its installer's
    // per-operation rewrites of extensions.php — one write here,
    // when an extension was installed, updated or removed.
    let touched: Vec<&str> = gone.iter().chain(fresh.iter()).copied().collect();
    emulate_yii2_composer(
        &run.project,
        local,
        &run.manifest,
        run.with_dev,
        &order,
        &touched,
        !args.no_plugins,
    )?;
    emulate_bamarni_bin(local, &run.manifest, run.with_dev, !args.no_plugins);
    emulate_package_versions(
        &run.project,
        local,
        &run.manifest,
        run.with_dev,
        !args.no_plugins,
    )?;
    Ok(())
}

/// The end of `Installer::run` (Installer.php:412-442): the funding count,
/// then `POST_INSTALL_CMD` / `POST_UPDATE_CMD` — the plugins' listeners
/// first (all at priority 0), then the scripts of composer.json, which
/// `EventDispatcher::getListeners` appends after the plugins' priority-0
/// listeners. Composer orders the plugins by activation
/// (`PackageSorter::sortPackages`, a plugin installed by the command last);
/// vivacity runs its emulations in a fixed order — dealerdirect, the
/// extension-installers, codeception/c3, symfony/flex — of which only c3
/// and Flex print (docs/plans/install-order.md). Runs without the dump too
/// (`--no-autoloader`), as in Composer.
fn post_install(
    run: &Run<'_>,
    layout: &vivacity_core::layout::Layout,
    report: &vivacity_core::installer::InstallReport,
) -> anyhow::Result<Flow> {
    let args = run.args;
    print_funding(&run.project, &run.manifest);
    let local = report.local_repository.as_ref().unwrap_or(&run.lock);
    if !args.no_plugins {
        // `dealerdirect/phpcodesniffer-composer-installer`.
        let installed: Vec<&vivacity_core::lock::LockPackage> =
            local.wanted_packages(run.with_dev).collect();
        let allowed = matches!(
            vivacity_core::layout::plugin_allowed(
                &run.manifest,
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
                &run.project,
                layout,
                &installed,
                &run.manifest,
            )?;
        }
        extension_installers::phpstan(
            layout,
            local,
            &run.manifest,
            run.with_dev,
            !args.no_plugins,
        )?;
        extension_installers::rector(layout, local, &run.manifest, run.with_dev, !args.no_plugins)?;
    }
    emulate_c3(
        &run.project,
        local,
        &run.manifest,
        run.with_dev,
        args.after_update(),
        !args.no_plugins,
    )?;
    // symfony/flex: `install` on `POST_INSTALL_CMD`, `update` on
    // `POST_UPDATE_CMD` (decided by the update, which saw the operations).
    match &args.post_update {
        None => emulate_flex_install(&run.project, &run.manifest, !args.no_plugins)?,
        Some(update) if update.flex_active => {
            print_flex_post_update(
                &update.manifest,
                update.thanks_reminder,
                update.flex_plan.as_deref(),
            )?;
        }
        Some(_) => {}
    }
    if let Some(r) = &run.runner {
        let code = r.run(scripts::POST_INSTALL_CMD)?;
        if code != 0 {
            return Ok(Flow::Break(code));
        }
    }
    Ok(Flow::Continue(()))
}

/// vivacity's closing line: what the install did and where it found it.
fn say_summary(
    run: &Run<'_>,
    report: &vivacity_core::installer::InstallReport,
    autoload_note: &str,
) {
    let warmed = if report.store_warmed > 0 {
        format!(", store warmed for {} packages", report.store_warmed)
    } else {
        String::new()
    };
    vivacity_core::say!(
        "vivacity: {} installed, {} unchanged, {} removed ({} from store, {} from cache, {} from network){warmed}{autoload_note} in {:.2}s",
        report.installed,
        report.unchanged,
        report.removed,
        report.store_hits,
        report.from_cache,
        report.from_network,
        run.t0.elapsed().as_secs_f32()
    );
}

/// `pestphp/pest-plugin`'s post-autoload-dump listener, emulated: with
/// plugins on and the plugin wanted, `vendor/pest-plugins.json` from the
/// installed packages' `extra.pest.plugins` in local-repository order
/// (`order`, package names); the file goes when the plugin leaves.
pub(crate) fn emulate_pest_plugin(
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
pub(crate) fn emulate_yii2_composer(
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

/// `codeception/c3`'s `uninstall`, emulated (the transaction removes the
/// plugin): with plugins on and the plugin allowed, `c3.php` deleted from
/// the project root. Message on stdout, as `$io->write` prints.
pub(crate) fn emulate_c3_uninstall(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_core::c3_plugin as c3;
    let allowed = matches!(
        vivacity_core::layout::plugin_allowed(manifest, c3::PLUGIN_NAME),
        vivacity_core::layout::PluginVerdict::Allowed
    );
    if plugins_enabled && allowed && c3::delete_c3(project)? {
        vivacity_core::out!("[codeception/c3] Deleting c3.php from the root of your project...");
    }
    Ok(())
}

/// `codeception/c3`'s `POST_INSTALL_CMD` / `POST_UPDATE_CMD` listener,
/// emulated: with plugins on and the plugin installed and allowed, `c3.php`
/// copied to the project root (`copyC3V2`; `askForUpdateV2` after an
/// update: no "up-to-date" line). Messages on stdout, as `$io->write`
/// prints.
pub(crate) fn emulate_c3(
    project: &std::path::Path,
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
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
    if !wanted || !allowed {
        return Ok(());
    }
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .vendor_dir(project);
    match c3::copy_c3(&vendor, project)? {
        c3::C3Action::UpToDate if !after_update => {
            vivacity_core::out!("[codeception/c3] c3.php is already up-to-date");
        }
        c3::C3Action::Copied => {
            vivacity_core::out!("[codeception/c3] Copying c3.php to the root of your project...");
            vivacity_core::out!("[codeception/c3] Include c3.php into index.php in order to collect codecoverage from server scripts");
        }
        _ => {}
    }
    Ok(())
}

/// bamarni/composer-bin-plugin's `POST_AUTOLOAD_DUMP` listener: the
/// deprecation lines again (the plugin now installed and allowed).
pub(crate) fn emulate_bamarni_bin(
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

/// `composer/package-versions-deprecated`'s `POST_AUTOLOAD_DUMP`
/// listener, emulated: with plugins on and the plugin installed and
/// allowed, its `Versions.php` is rewritten from the lock and the root
/// package (`vivacity_core::package_versions`).
pub(crate) fn emulate_package_versions(
    project: &std::path::Path,
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_core::package_versions as pv;
    let allowed = matches!(
        vivacity_core::layout::plugin_allowed(manifest, pv::PLUGIN_NAME),
        vivacity_core::layout::PluginVerdict::Allowed
    );
    if !plugins_enabled
        || !allowed
        || !local
            .wanted_packages(with_dev)
            .any(|p| p.name() == pv::PLUGIN_NAME)
    {
        return Ok(());
    }
    let root = vivacity_core::state::RootPackage::detect(manifest, project, with_dev);
    let replaces: Vec<(String, String)> = manifest
        .get("replace")
        .and_then(serde_json::Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|c| (k.to_lowercase(), c.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let versions = pv::versions(
        local,
        with_dev,
        &root.name,
        &root.pretty_version,
        root.reference.as_deref().unwrap_or(""),
        &replaces,
    );
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .vendor_dir(project);
    pv::write(&vendor, &pv::render(&root.name, &versions))?;
    Ok(())
}

/// `  - <operation><appendix>` for every operation of a real install.
/// `FileDownloader::download`'s line, for the packages this install will fetch.
///
/// Composer writes it synchronously in `downloadAndExecuteBatch`'s registration
/// loop — `return $download()` runs its `writeError` before `addCopy`, and
/// `addJob` does not tick the curl loop — so the order is the transaction's,
/// not the completion order, and the lines form one block before the operation
/// lines. A package with no dist (a metapackage) or a `path` dist gets no
/// `FileDownloader` and no line. A dry run downloads nothing and prints none
/// (measured).
///
/// The decision is Composer's: the line appears when its files cache does not
/// serve the archive. Checked by presence alone — a cached archive whose sha1 no
/// longer matches makes Composer print where we do not, which is the one corner
/// left (our own fetch throws that entry away and re-downloads either way).
pub(crate) fn download_lines(
    lock: &vivacity_core::lock::Lock,
    arena: &[vivacity_resolver::package::Package],
    operations: &[vivacity_resolver::transaction::Operation],
    with_dev: bool,
) -> Vec<String> {
    use vivacity_resolver::transaction::{DisplayRef, Operation};
    let by_name: std::collections::BTreeMap<&str, &vivacity_core::lock::LockPackage> = lock
        .wanted_packages(with_dev)
        .map(|p| (p.name(), p))
        .collect();
    let cache_root = vivacity_core::fetch::composer_cache_dir();
    let mut lines = Vec::new();
    for op in operations {
        let target = match *op {
            Operation::Install(p) | Operation::Update(_, p) => p,
            _ => continue,
        };
        let package = &arena[target];
        let Some(locked) = by_name.get(package.name.as_str()) else {
            continue;
        };
        let kind = match locked
            .raw
            .get("dist")
            .and_then(|d| d.get("type"))
            .and_then(serde_json::Value::as_str)
        {
            Some(kind) if kind != "path" => kind.to_owned(),
            _ => continue,
        };
        let Some(url) = locked.dist_url_expanded() else {
            continue;
        };
        let cached =
            vivacity_core::fetch::dist_cache_path_of(&cache_root, locked.name(), &url, &kind);
        if cached.exists() {
            continue;
        }
        lines.push(format!(
            "  - Downloading {} ({})",
            package.name,
            vivacity_resolver::transaction::full_pretty_version(
                package,
                true,
                DisplayRef::SourceRefIfDev
            )
        ));
    }
    lines
}

pub(crate) fn operation_lines(
    project: &std::path::Path,
    arena: &[vivacity_resolver::package::Package],
    operations: &[vivacity_resolver::transaction::Operation],
    layout: &vivacity_core::layout::Layout,
    flex_packs: bool,
    plugins_enabled: bool,
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
                    // `SymfonyPackInstaller extends MetapackageInstaller`:
                    // with Flex active a `symfony-pack` is laid out like a
                    // metapackage — nothing downloaded, nothing appended.
                    Some(_)
                        if pkg.package_type != "metapackage"
                            && !(flex_packs && pkg.package_type == "symfony-pack") =>
                    {
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
        // `PluginInstaller::install|update` registers the package right
        // after laying it out, and `PluginManager::registerPackage` says so
        // when plugins are off.
        if !plugins_enabled {
            if let Operation::Install(p) | Operation::Update(_, p) = *op {
                let pkg = &arena[p];
                if matches!(
                    pkg.package_type.as_str(),
                    "composer-plugin" | "composer-installer"
                ) {
                    lines.push(format!(
                        "The \"{}\" plugin was not loaded as plugins are disabled.",
                        pkg.name
                    ));
                }
            }
        }
    }
    Ok(lines)
}

/// Does vendor/ already hold this package exactly as the new lock names it?
/// Then the install lays nothing out for it, and what Flex reads at
/// `POST_UPDATE_CMD` is what is there now.
pub(crate) fn installed_as_locked(
    installed: &[serde_json::Value],
    p: &vivacity_core::lock::LockPackage,
) -> bool {
    let wanted = serde_json::Value::Object(p.raw.clone());
    installed.iter().any(|i| {
        i.get("name").and_then(serde_json::Value::as_str) == Some(p.name())
            && package_identity(i) == package_identity(&wanted)
    })
}

/// What `Transaction::calculateOperations` compares to decide that a present
/// package and a resolved one are the same thing, and so that there is no
/// operation: the version, both references, and the abandoned mark (an
/// `abandoned` that appears or changes is an update on its own).
/// `unchanged_local_repository` asks the same question of the whole set, and
/// the two must not drift apart — a package this calls unchanged while the
/// installer updates it loses Flex's `POST_PACKAGE_UPDATE`.
pub(crate) fn package_identity(
    p: &serde_json::Value,
) -> (
    Option<serde_json::Value>,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
) {
    let field = |a: &str, b: &str| p.get(a).and_then(|v| v.get(b)).cloned();
    (
        p.get("version").cloned(),
        field("dist", "reference"),
        field("source", "reference"),
        p.get("abandoned").cloned(),
    )
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
pub(crate) fn unchanged_local_repository(
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
    let identity = package_identity;
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

pub(crate) fn installed_packages(
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
/// (never an alias), unless `COMPOSER_FUND` is a number whose `intval` is 0
/// ([`funding_shown`]). Printed
/// in every mode, dry run included (`$localRepo` is then the mocked
/// repository of the same packages).
pub(crate) fn print_funding(project: &std::path::Path, manifest: &serde_json::Value) {
    if !funding_shown(std::env::var("COMPOSER_FUND").ok().as_deref()) {
        return;
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
        vivacity_core::say!(
            "{funding} package{} you are using {} looking for funding.\nUse the `composer fund` command to find out more!",
            if funding == 1 { "" } else { "s" },
            if funding == 1 { "is" } else { "are" }
        );
    }
}

/// `is_numeric($fundEnv)` then `intval($fundEnv) !== 0`: a numeric
/// `COMPOSER_FUND` that truncates to 0 (`0`, `0.5`, `-0.9`) hides the line,
/// and so does one that overflows to infinity (`intval(INF)` is 0). `inf`
/// and `nan` are not numeric to PHP, whatever Rust parses.
fn funding_shown(env: Option<&str>) -> bool {
    let Some(t) = env.map(str::trim) else {
        return true;
    };
    let lower = t.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return true;
    }
    match t.parse::<f64>() {
        Ok(n) => n.is_finite() && n.trunc() != 0.0,
        Err(_) => true,
    }
}

/// `Installer::run`: one warning per abandoned package of the lock (dev
/// included), in lock order.
pub(crate) fn abandoned_warnings(lock: &serde_json::Value) -> Vec<String> {
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

#[cfg(test)]
mod funding_tests {
    use super::funding_shown;

    #[test]
    fn funding_hidden_by_a_numeric_fund_that_truncates_to_zero() {
        assert!(funding_shown(None));
        assert!(funding_shown(Some("")));
        assert!(funding_shown(Some("no")));
        assert!(funding_shown(Some("1")));
        assert!(funding_shown(Some(" 2.5 ")));
        assert!(!funding_shown(Some("0")));
        assert!(!funding_shown(Some("0.5")));
        assert!(!funding_shown(Some("-0.9")));
        assert!(!funding_shown(Some("1e400")));
        assert!(funding_shown(Some("inf")));
        assert!(funding_shown(Some("NaN")));
    }
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
