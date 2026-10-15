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

pub(crate) fn run_install(args: &InstallArgs) -> anyhow::Result<i32> {
    if args.no_install {
        vivacity_core::say!("Invalid option \"--no-install\". Use \"composer update --no-install\" instead if you are trying to update the composer.lock file.");
        return Ok(1);
    }
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    let manifest_path = project.join("composer.json");
    let lock_path = project.join("composer.lock");
    refuse_alternate_manifest()?;
    if let Some(code) = refuse_missing_manifest(&project) {
        return Ok(code);
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
        return Ok(code);
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
            return Ok(1);
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
            return fallback_or_fail(args, &report);
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
                vivacity_core::say_always!("vivacity: {e}");
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
        vivacity_core::say!(
            "Installing dependencies from lock file{}",
            if with_dev {
                " (including require-dev)"
            } else {
                ""
            }
        );
    }
    // vivacity reads the transaction off the lock where Composer solves it
    // again: the resolution line `-v` shows measures that stretch instead.
    let verify_start = std::time::Instant::now();
    if !args.after_update {
        vivacity_core::say!("Verifying lock file contents can be installed on current platform.");
        // Lock freshness: same behaviour as Composer, a warning.
        if let (Ok(actual), Some(expected)) = (
            vivacity_core::content_hash::content_hash(&manifest_text),
            lock.content_hash.as_deref(),
        ) {
            if actual != expected {
                vivacity_core::say_always!(
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
            .map_err(session_error)
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
            vivacity_core::say!("{w}");
        }
        if !problems.is_empty() {
            // `Installer::doInstall`: the headline, then
            // `SolverProblemsException::getPrettyString` (problems
            // deduplicated and numbered, each ending with a newline).
            say_lock_resolution(verify_start);
            vivacity_core::say_always!("Your lock file does not contain a compatible set of packages. Please run composer update.");
            let mut text = String::from("\n");
            let mut seen: Vec<&String> = Vec::new();
            for p in &problems {
                if seen.contains(&p) {
                    continue;
                }
                seen.push(p);
                text.push_str(&format!("  Problem {}\n    {p}\n", seen.len()));
            }
            vivacity_core::say!("{text}");
            return Ok(2);
        }
        trace("policy", t0);
    }
    match platform_outcome {
        PlatformOutcome::Failures(failures) if !failures.is_empty() => {
            vivacity_core::say_always!("Your lock file cannot be installed on this platform:");
            for f in &failures {
                let by = f
                    .required_by
                    .as_deref()
                    .map(|p| format!(" (required by {p})"))
                    .unwrap_or_default();
                vivacity_core::say_always!(
                    "  - {} {}{}: {:?}",
                    f.requirement,
                    f.constraint,
                    by,
                    f.reason
                );
            }
            vivacity_core::say_always!(
                "Use --ignore-platform-req=<req> or --ignore-platform-reqs to bypass."
            );
            return Ok(4);
        }
        PlatformOutcome::NoPhp { warn: true } => {
            vivacity_core::say!(
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
                return fallback_or_fail(args, &report);
            }
        }
        for line in &missing {
            vivacity_core::say!("{line}");
        }
        if !require::truthy(require::config_value(
            &manifest,
            "allow-missing-requirements",
        )) {
            return Ok(4);
        }
    }
    if !args.after_update {
        say_lock_resolution(verify_start);
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
            vivacity_core::say!("Nothing to install, update or remove");
        } else {
            vivacity_core::say!(
                "Package operations: {installs} install{}, {updates} update{}, {removals} removal{}",
                if installs == 1 { "" } else { "s" },
                if updates == 1 { "" } else { "s" },
                if removals == 1 { "" } else { "s" }
            );
            for line in vivacity_resolver::transaction::verbose_operation_summary(ops, &arena) {
                vivacity_core::say_verbose!("{line}");
            }
        }
    }
    if args.dry_run {
        for op in &transaction.operations {
            if let Some(line) = op.show(&arena, false) {
                vivacity_core::say!("  - {line}");
            }
        }
        // `Installer::run` after `doInstall` (an update prints its own
        // report after this phase).
        if !args.after_update {
            for line in abandoned_warnings(&lock_value) {
                vivacity_core::say!("{line}");
            }
            print_funding(&project, &manifest);
        }
        return Ok(0);
    }

    // Out of scope: Composer as fallback (default) or fail explicitly.
    let Some(scope) = scope else {
        anyhow::bail!("internal: scope was not analysed");
    };
    if !scope.is_native_ok() {
        return fallback_or_fail(args, &scope);
    }
    let Some(layout) = scope.layout.as_ref() else {
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
    trace("scope", t0);
    if args.check_scope {
        vivacity_core::say_always!("vivacity: this lock is installed natively");
        return Ok(0);
    }

    // The operation lines of a real install, in transaction order, with the
    // downloader's appendix (`getInstallOperationAppendix`: `: Extracting
    // archive`, `: Symlinking from …`, `: Mirroring from …`, `: Source
    // already present`) computed on the state before the transaction, like
    // Composer does right before each operation. They are printed once the
    // transaction has succeeded: the placements run in parallel, and a
    // failure is reported alone.
    let operation_lines = operation_lines(
        &project,
        &arena,
        &transaction.operations,
        layout,
        lock.flex_packs(&manifest, with_dev, !args.no_plugins),
        !args.no_plugins,
    )?;
    // Composer decides these before it fetches anything; computed here for the
    // same reason `operation_lines` is — our own fetch fills that cache.
    let downloading = download_lines(&lock, &arena, &transaction.operations, with_dev);

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
        plugins_enabled: !args.no_plugins,
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
            return fallback_or_fail(args, &scope);
        }
        Err(vivacity_core::Error::Refused(msg)) => {
            vivacity_core::say_always!("{msg}");
            return Ok(1);
        }
        Err(e) => return Err(e.into()),
    };
    trace("install transaction", t0);
    for line in &downloading {
        vivacity_core::say!("{line}");
    }
    for line in &operation_lines {
        vivacity_core::say!("{line}");
    }
    // `BinaryInstaller` notices belong to the operations, before the dump.
    for m in &report.messages {
        vivacity_core::say!("{m}");
    }
    // `Installer::run`: the abandoned packages of the lock (dev included)
    // before the dump, the funding count after it; an install driven by
    // `update` reports through it.
    if !args.after_update {
        for line in abandoned_warnings(&lock_value) {
            vivacity_core::say!("{line}");
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
        vivacity_core::say!(
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
            emulate_package_versions(&project, local, &manifest, with_dev, !args.no_plugins)?;
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
        // symfony/flex's `POST_INSTALL_CMD` listener (after an update it is
        // `POST_UPDATE_CMD` that fires, handled by the update path).
        emulate_flex_install(&project, &manifest, !args.no_plugins)?;
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
    vivacity_core::say!(
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

/// `codeception/c3`, emulated: with plugins on, `c3.php` copied to the
/// project root after the install (`copyC3V2`; `askForUpdateV2` after an
/// update: no "up-to-date" line), or deleted when the plugin was
/// uninstalled by this run. Messages on stdout, as `$io->write` prints.
pub(crate) fn emulate_c3(
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
            vivacity_core::out!(
                "[codeception/c3] Deleting c3.php from the root of your project..."
            );
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
/// (never an alias), unless `COMPOSER_FUND` is a number equal to 0. Printed
/// in every mode, dry run included (`$localRepo` is then the mocked
/// repository of the same packages).
pub(crate) fn print_funding(project: &std::path::Path, manifest: &serde_json::Value) {
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
        vivacity_core::say!(
            "{funding} package{} you are using {} looking for funding.\nUse the `composer fund` command to find out more!",
            if funding == 1 { "" } else { "s" },
            if funding == 1 { "is" } else { "are" }
        );
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
