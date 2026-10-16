//! `update`: resolve, write the lock, install — `UpdateCommand` and the
//! resolution half of `Installer::run`; `require` and `remove` go through
//! it too.

use super::*;

/// Network transport for remote composer repositories: the vivacity-core
/// Fetcher (Composer auth, retries), made synchronous, with a parallel
/// batch (Composer: curl multi, 12 downloads at a time).
pub(crate) fn http_transport(
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

/// `{\S+[ =:]\S+}`: a `packages` entry that carries a constraint. The
/// name alone is what the allow list takes (`{^([^ =:]+)[ =:].*$}`).
pub(crate) fn package_of_requirement(entry: &str) -> &str {
    match entry.find([' ', '=', ':']) {
        Some(i)
            if entry[i + 1..]
                .trim_start_matches([' ', '=', ':'])
                .is_empty() =>
        {
            entry
        }
        Some(i) => &entry[..i],
        None => entry,
    }
}

/// `UpdateCommand`: `formatRequirements(--with)` plus the `packages`
/// entries that carry a constraint — `a/b:^1`, `a/b=^1`, `a/b ^1`
/// (`VersionParser::parseNameVersionPairs`). A pair without a version is
/// the command's own error.
pub(crate) fn temporary_requirements(
    with: &[String],
    packages: &[String],
) -> anyhow::Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    let carrying: Vec<String> = packages
        .iter()
        .filter(|p| package_of_requirement(p) != p.as_str())
        .cloned()
        .collect();
    for source in [with, &carrying] {
        for (name, version) in vivacity_resolver::version::parse_name_version_pairs(source) {
            let Some(version) = version else {
                anyhow::bail!(
                    "Option {name} is missing a version constraint, use e.g. {name}:^1.0"
                );
            };
            // `$requires[$name] = $version`: the last one wins.
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = version,
                None => out.push((name, version)),
            }
        }
    }
    Ok(out)
}

/// `composer update`: resolution (exact port of Composer's solver), lock
/// written if its data changes, then `install`.
pub(crate) fn run_update(args: &UpdateArgs) -> anyhow::Result<i32> {
    refuse_alternate_manifest()?;
    if let Some(code) = refuse_missing_manifest(&project_dir(args.working_dir.as_deref())?) {
        return Ok(code);
    }
    // `BaseCommand::initialize`: the root-version warning before anything
    // `UpdateCommand` prints, then the platform options' environment.
    hold_root_version_warning(&project_dir(args.working_dir.as_deref())?);
    platform_requirement_env(args.ignore_platform_reqs, &args.ignore_platform_req, true);
    // Partial update: `update a/b [-w|-W]` (UpdateCommand).
    let env_flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
    for p in &args.packages {
        if ["lock", "nothing", "mirrors"].contains(&p.as_str()) {
            anyhow::bail!(
                "`vivacity update {p}` (lock file metadata refresh) is not supported yet"
            );
        }
    }
    let transitive = if args.with_all_dependencies || env_flag("COMPOSER_WITH_ALL_DEPENDENCIES") {
        vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDeps
    } else if args.with_dependencies || env_flag("COMPOSER_WITH_DEPENDENCIES") {
        vivacity_resolver::pool::UpdateMode::ListedWithTransitiveDepsNoRootRequire
    } else {
        vivacity_resolver::pool::UpdateMode::OnlyListed
    };
    // `UpdateCommand`: `--with a/b:^1` and the `update a/b:^1` shorthand
    // are temporary constraints; the shorthand's package keeps its name in
    // the allow list.
    let reqs = temporary_requirements(&args.with, &args.packages)?;
    let allow_list: Vec<String> = args
        .packages
        .iter()
        .map(|p| package_of_requirement(p).to_owned())
        .collect();
    let mut options = if allow_list.is_empty() {
        vivacity_resolver::session::UpdateOptions::default()
    } else {
        vivacity_resolver::session::UpdateOptions::partial(&allow_list, transitive)
    };
    options.no_blocking = args.no_blocking || args.no_security_blocking;
    options.temporary_requirements = reqs;
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
    // `update` edits nothing before resolving: nothing to undo.
    run_update_resolved(args, options, prefer_stable, prefer_lowest, &|| Ok(()))
        .map(|ended| ended.status)
}

/// A session error the command prints itself: each part at its Composer
/// level when the session split it, else the whole message at the default
/// level (`UpdateCommand`'s temporary-constraint refusal).
fn print_session_error(e: &vivacity_resolver::session::SessionError) {
    if e.parts.is_empty() {
        vivacity_core::say!("{}", e.message);
    }
    for (level, text) in &e.parts {
        vivacity_core::say_at!(*level, "{text}");
    }
    if let Some(annotation) = &e.annotation {
        vivacity_core::io::github_action_error(annotation);
    }
}

/// How [`run_update_resolved`] ended: the exit code, and whether the whole
/// command went to Composer after the resolution — the caller (`remove`)
/// then adds nothing of its own: no revert, no local-repository check.
pub(crate) struct Ended {
    pub(crate) status: i32,
    pub(crate) by_composer: bool,
}

/// Resolution, lock write and install, once the list and the update mode
/// are decided (shared by `update` and `remove`). `undo_edit` restores the
/// caller's composer.json before a hand-over ([`hand_resolution_over`]).
pub(crate) fn run_update_resolved(
    args: &UpdateArgs,
    options: vivacity_resolver::session::UpdateOptions,
    prefer_stable: bool,
    prefer_lowest: bool,
    undo_edit: &dyn Fn() -> anyhow::Result<()>,
) -> anyhow::Result<Ended> {
    let resolved = resolve_and_lock(args, options, prefer_stable, prefer_lowest)?;
    if let Some(issue) = &resolved.hand_over {
        return Ok(Ended {
            status: hand_resolution_over(issue, args.no_fallback, undo_edit)?,
            by_composer: true,
        });
    }
    let ended = |status| Ended {
        status,
        by_composer: false,
    };
    if resolved.status != 0 {
        return Ok(ended(resolved.status));
    }
    if !args.no_install {
        let (virtual_lock, virtual_manifest) = if args.dry_run {
            (resolved.lock.clone(), Some(resolved.manifest.clone()))
        } else {
            (None, None)
        };
        // The install phase prints the end of `Installer::run` itself, at
        // Composer's points — or Composer does, when it was handed over.
        let status = install_after_update(args, resolved.report(), virtual_lock, virtual_manifest)?;
        return Ok(ended(status));
    }
    print_post_update(&resolved, &project_dir(args.working_dir.as_deref())?)?;
    Ok(ended(0))
}

/// The install that follows the lock write (`Installer::run` with
/// `setInstall(true)`): its status. It prints the end of `Installer::run`
/// (`report`) at Composer's points — unless it goes to Composer, whose
/// `install` then prints its own end (the abandoned packages, the funding
/// count; the suggestions, which only an update prints, are what is
/// missing).
pub(crate) fn install_after_update(
    args: &UpdateArgs,
    report: UpdateReport,
    virtual_lock: Option<serde_json::Value>,
    virtual_manifest: Option<serde_json::Value>,
) -> anyhow::Result<i32> {
    let install = InstallArgs {
        compat: GlobalCompatArgs::default(),
        prefer_source: args.prefer_source,
        prefer_dist: args.prefer_dist,
        prefer_install: args.prefer_install.clone(),
        no_progress: args.no_progress,
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
        fallback_line: Some(after_update_line(args)),
        virtual_lock,
        virtual_manifest,
        post_update: Some(Box::new(report)),
    };
    run_install(&install)
}

/// Resolution and lock write: 0, or 2 on an unsolvable set.
/// The outcome of the resolution: the status (0, or 2 on an unsolvable
/// set) and the lock data, written or not (`config.lock: false` resolves
/// without writing; Composer then keeps a "virtual" lock).
pub(crate) struct Resolved {
    pub(crate) status: i32,
    pub(crate) lock: Option<serde_json::Value>,
    /// What `Installer::run` prints after `doUpdate` and before the dump
    /// (suggestions, abandoned packages), ready to print; the funding count
    /// is read from installed.json after the dump, by the install phase or
    /// by [`print_post_update`].
    pub(crate) post: Vec<String>,
    /// The manifest as the (possibly patched) root package sees it.
    pub(crate) manifest: serde_json::Value,
    /// symfony/flex was active for this resolution: its `POST_UPDATE_CMD`
    /// output follows the funding count ([`UpdateReport`]).
    pub(crate) flex_active: bool,
    /// The install would perform at least one update operation, which arms
    /// Flex's `symfony/thanks` reminder (`enableThanksReminder` on
    /// `POST_PACKAGE_UPDATE`); computed before the install, since it needs
    /// the state the install replaces.
    pub(crate) thanks_reminder: bool,
    /// The recipes Flex applies at `POST_UPDATE_CMD`, reproduced
    /// (`flex_recipes`): printed and written by the install phase.
    pub(crate) flex_plan: Option<std::sync::Arc<crate::flex_recipes::FlexPlan>>,
    /// The solved lock shows a Flex behaviour vivacity does not emulate:
    /// nothing is written, and the whole command goes to Composer — run by
    /// the caller ([`hand_resolution_over`]), which first undoes its own
    /// edit of composer.json (`require`/`remove`): Composer must start from
    /// the manifest the user had, or `remove` finds its package gone.
    pub(crate) hand_over: Option<vivacity_core::scope::ScopeIssue>,
}

/// What `Installer::run` prints around an update's install phase, handed
/// to that phase ([`install_after_update`]): the suggestions and the
/// abandoned packages before the dump (`Installer.php:343-373`), Flex's
/// `POST_UPDATE_CMD` output after the funding (l. 412-442).
#[derive(Debug)]
pub(crate) struct UpdateReport {
    pub(crate) lines: Vec<String>,
    /// The manifest Flex reads (the patched root package).
    pub(crate) manifest: serde_json::Value,
    pub(crate) flex_active: bool,
    pub(crate) thanks_reminder: bool,
    pub(crate) flex_plan: Option<std::sync::Arc<crate::flex_recipes::FlexPlan>>,
}

impl Resolved {
    pub(crate) fn report(&self) -> UpdateReport {
        UpdateReport {
            lines: self.post.clone(),
            manifest: self.manifest.clone(),
            flex_active: self.flex_active,
            thanks_reminder: self.thanks_reminder,
            flex_plan: self.flex_plan.clone(),
        }
    }

    /// The command stops with `status` before anything is written.
    fn stopped(status: i32) -> Resolved {
        Resolved {
            status,
            lock: None,
            post: Vec::new(),
            manifest: serde_json::Value::Null,
            flex_active: false,
            thanks_reminder: false,
            flex_plan: None,
            hand_over: None,
        }
    }
}

/// The resolution handed to Composer once solved ([`Resolved::hand_over`]):
/// `undo_edit` restores composer.json as it was, silently — Composer prints
/// its own `has been updated` and reverts on its own failure — then
/// [`delegate_resolution`] (exit 3 under `--no-fallback`, the edit undone
/// all the same). The command adds nothing after it: no revert, no report.
pub(crate) fn hand_resolution_over(
    issue: &vivacity_core::scope::ScopeIssue,
    no_fallback: bool,
    undo_edit: &dyn Fn() -> anyhow::Result<()>,
) -> anyhow::Result<i32> {
    undo_edit()?;
    delegate_resolution(std::slice::from_ref(issue), no_fallback)
}

/// Prints the post-update report of `Installer::run` when the update has
/// no install phase (`--no-install`): the precomputed lines, the funding
/// count, then Flex's `POST_UPDATE_CMD` output. With an install phase,
/// that phase prints it (`run_install`).
pub(crate) fn print_post_update(
    resolved: &Resolved,
    project: &std::path::Path,
) -> anyhow::Result<()> {
    for line in &resolved.post {
        vivacity_core::say!("{line}");
    }
    print_funding(project, &resolved.manifest);
    if resolved.flex_active {
        print_flex_post_update(&resolved.manifest, resolved.thanks_reminder, None)?;
    }
    Ok(())
}

pub(crate) fn resolve_and_lock(
    args: &UpdateArgs,
    options: vivacity_resolver::session::UpdateOptions,
    prefer_stable: bool,
    prefer_lowest: bool,
) -> anyhow::Result<Resolved> {
    use vivacity_resolver::platform_filter::PlatformRequirementFilter;
    use vivacity_resolver::session::UpdateSession;
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    refuse_alternate_manifest()?;
    let manifest_path = project.join("composer.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("cannot read {}", manifest_path.display()))?;
    let installer_dev_mode = !(args.no_dev || env_no_dev());
    let mut options = options;
    if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
        // `RootPackage::load` refuses these too, but through the error path,
        // which prefixes the message; here it is worded as Composer words it.
        if let Some(code) = refuse_invalid_root(&m) {
            return Ok(Resolved::stopped(code));
        }
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
    let mut session = match UpdateSession::prepare_update(
        &project,
        home.as_deref(),
        true,
        Some(http),
        Some(&cache_repo_dir),
        &options,
    ) {
        Ok(s) => s,
        // The command prints and returns the code itself: the message
        // alone on stderr, nothing written.
        Err(e)
            if matches!(
                e.kind,
                vivacity_resolver::session::SessionErrorKind::Printed(_)
            ) =>
        {
            // Exit 3 (`ERROR_NO_LOCK_FILE_FOR_PARTIAL_UPDATE`) is the partial
            // update without a lock, which `Installer::doUpdate` refuses
            // after `COMMAND` (Installer.php:508-510); the temporary
            // constraints (exit 1) are refused before it
            // (UpdateCommand.php:183-193).
            if e.kind == vivacity_resolver::session::SessionErrorKind::Printed(3)
                && !args.command_dispatched
            {
                if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
                    bamarni_command_event(&project, &m, !args.no_plugins);
                }
            }
            print_session_error(&e);
            if let Some(line) = &e.stdout {
                vivacity_core::out!("{line}");
            }
            let vivacity_resolver::session::SessionErrorKind::Printed(code) = e.kind else {
                unreachable!()
            };
            return Ok(Resolved::stopped(code));
        }
        Err(e) => return Err(session_error(e)),
    };
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
    // `BaseCommand::initialize`: the environment fills the options left
    // empty. Its warnings were held by the command (`update`, `require`,
    // `remove`, or the `install` that resolves without a lock), at the
    // point Composer writes them.
    let (ignore_all, ignore_list) =
        platform_requirement_env(args.ignore_platform_reqs, &args.ignore_platform_req, false);
    let filter = if ignore_all {
        PlatformRequirementFilter::IgnoreAll
    } else if !ignore_list.is_empty() {
        PlatformRequirementFilter::from_list(&ignore_list)
    } else {
        PlatformRequirementFilter::IgnoreNothing
    };
    // `UpdateCommand::execute` dispatches `PluginEvents::COMMAND` after its
    // own refusals (the temporary constraints, here in `prepare_update`) and
    // before `Installer::run` (UpdateCommand.php:250); `require` and
    // `remove` have dispatched theirs already.
    if !args.command_dispatched {
        if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
            bamarni_command_event(&project, &m, !args.no_plugins);
        }
    }
    vivacity_core::say!("Loading composer repositories with package information");
    // (`Updating dependencies` is printed by the session once the pool is
    // built — Flex's notice comes before it.)
    // `Installer::run`: an unsolvable set means exit code 2
    // (`SolverProblemsException`), any other error is an exception.
    let (lock, report) = match session.update(&manifest_text, &filter) {
        Ok(r) => r,
        Err(e) if e.kind == vivacity_resolver::session::SessionErrorKind::Unsolvable => {
            print_session_error(&e);
            return Ok(Resolved::stopped(2));
        }
        Err(e) => return Err(session_error(e)),
    };
    trace("resolve", t0);

    // symfony/flex with install: its `POST_UPDATE_CMD` applies recipes to
    // what the install lays out. vivacity emulates that only where Flex
    // would apply none — and the question is answerable exactly here, on
    // the solved lock, before anything is written.
    // The package.json / importmap.php synchronisation comes with it, and
    // it does not need the install: measured on Composer 2.10.3, `update
    // --no-install` still dispatches `POST_UPDATE_CMD` — so Flex still
    // synchronises — while `--dry-run` does not dispatch it at all.
    if !args.dry_run && !args.no_install {
        if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
            if let Some(reason) =
                flex_reinstall_reason(&project, &m, &lock, installer_dev_mode, !args.no_plugins)
            {
                return Ok(Resolved {
                    hand_over: Some(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
                        "symfony/flex".to_owned(),
                        reason,
                    )),
                    ..Resolved::stopped(0)
                });
            }
        }
    }
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
    // With recipes, Flex's plan: computed now, applied at
    // `POST_UPDATE_CMD` by the install phase.
    let mut flex_plan: Option<std::sync::Arc<crate::flex_recipes::FlexPlan>> = None;
    if flex_active && !args.dry_run {
        if let Ok(m) = serde_json::from_str::<serde_json::Value>(&manifest_text) {
            let reason = if args.no_install {
                None
            } else {
                match flex_install_decision(
                    &project,
                    &m,
                    &manifest_text,
                    &lock,
                    write_lock.then_some(text.as_str()),
                    installer_dev_mode,
                    args.offline,
                    !args.no_plugins,
                )? {
                    FlexDecision::HandOver(reason) => Some(reason),
                    FlexDecision::Native(plan) => {
                        flex_plan = plan.map(std::sync::Arc::new);
                        None
                    }
                }
            };
            // With a plan, the synchronisation was decided on the files the
            // recipes leave.
            let reason = reason.or_else(|| {
                if flex_plan.is_some() {
                    None
                } else {
                    flex_sync_reason(&project, &m, &lock)
                }
            });
            if let Some(reason) = reason {
                return Ok(Resolved {
                    hand_over: Some(vivacity_core::scope::ScopeIssue::ResolutionPlugin(
                        "symfony/flex".to_owned(),
                        reason,
                    )),
                    ..Resolved::stopped(0)
                });
            }
        }
    }

    // `Flex::enableThanksReminder` on `POST_PACKAGE_UPDATE`: the reminder
    // needs one update operation, so it is decided here, on the state the
    // install is about to replace. `class_exists(Thanks::class, false)` is
    // true exactly when Composer activated the symfony/thanks plugin — from
    // the project or from COMPOSER_HOME — since nothing autoloads it.
    let thanks_reminder = flex_active && !args.dry_run && !args.no_install && {
        let m: serde_json::Value =
            serde_json::from_str(&manifest_text).unwrap_or(serde_json::Value::Null);
        let loaded = vivacity_core::scope::active_plugins(&project, &m, !args.no_plugins)
            .iter()
            .any(|p| p == "symfony/thanks");
        let installed = installed_packages(&project, &m);
        !loaded
            && vivacity_core::lock::Lock::from_value(&lock)
                .wanted_packages(installer_dev_mode)
                .any(|p| {
                    installed.iter().any(|i| {
                        i.get("name").and_then(serde_json::Value::as_str) == Some(p.name())
                    }) && !installed_as_locked(&installed, p)
                })
    };

    let ops = &report.transaction.transaction.operations;
    // What `Installer::doUpdate` writes between the resolution and the lock
    // operations (the resolution lines, `Nothing to modify in lock file`),
    // held by the session until the checks above have decided the run is
    // vivacity's: a delegation lets Composer write its own.
    for (level, line) in &report.events {
        vivacity_core::say_at!(*level, "{line}");
    }
    if !ops.is_empty() && write_lock {
        let count = |f: &dyn Fn(&vivacity_resolver::transaction::Operation) -> bool| {
            ops.iter().filter(|o| f(o)).count()
        };
        use vivacity_resolver::transaction::Operation;
        let installs = count(&|o| matches!(o, Operation::Install(_)));
        let updates = count(&|o| matches!(o, Operation::Update(..)));
        let removals = count(&|o| matches!(o, Operation::Uninstall(_)));
        vivacity_core::say!(
            "Lock file operations: {installs} install{}, {updates} update{}, {removals} removal{}",
            if installs == 1 { "" } else { "s" },
            if updates == 1 { "" } else { "s" },
            if removals == 1 { "" } else { "s" }
        );
        // At VERBOSE, the names in the transaction's own order.
        for line in vivacity_resolver::transaction::verbose_operation_summary(ops, &session.arena) {
            vivacity_core::say_verbose!("{line}");
        }
        // `  - Locking …` / `Upgrading` / `Removing`, removals first, by name.
        for line in vivacity_resolver::transaction::lock_operation_lines(&session.arena, ops) {
            vivacity_core::say!("{line}");
        }
    }
    // `$updatedLock && $this->writeLock && $this->executeOperations`: a dry
    // run neither writes nor says so.
    if write_lock && !args.dry_run {
        vivacity_core::say!("Writing lock file");
        if !unchanged {
            std::fs::write(&lock_path, &text)
                .with_context(|| format!("cannot write {}", lock_path.display()))?;
        }
    }
    trace("write lock", t0);
    // `Installer::run` after `doUpdate` (and its install), before the dump:
    // the suggestions of the newly installed packages (and of the root on a
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
        // Measured on Composer 2.10.3: `--dry-run` does not dispatch
        // `POST_UPDATE_CMD` at all, so Flex prints nothing — not even the
        // recipes hint. `--no-install` does dispatch it.
        flex_active: flex_active && !args.dry_run,
        thanks_reminder,
        flex_plan,
        hand_over: None,
    })
}

/// composer.json with the session root's `require`/`require-dev` (the
/// in-memory patch of a dry run applied; identical otherwise).
pub(crate) fn patched_manifest(
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
/// per abandoned package of the new lock. The install phase prints them
/// before its dump; [`print_post_update`] without one.
pub(crate) fn post_update_report(
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

/// Minimal `join_all` (no futures dependency): the tasks run concurrently
/// on the runtime, results in order.
pub(crate) async fn futures_join_all<F: std::future::Future + Send + 'static>(
    tasks: Vec<F>,
) -> Vec<F::Output>
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

/// `Locker::getMissingRequirementInfo`: the root requirements the lock
/// does not satisfy — `require` against the non-dev lock, `require-dev`
/// against the whole lock — with replacers and providers counting, and
/// the three fixed lines when anything is missing.
pub(crate) fn missing_requirement_info(
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
pub(crate) fn unused_locked_packages(
    manifest: &serde_json::Value,
    lock: &serde_json::Value,
) -> Vec<String> {
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
mod temporary_requirement_tests {
    use super::{package_of_requirement, temporary_requirements};

    fn v(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_the_three_forms_and_the_shorthand() {
        // `--with` takes `a/b:^1`, `a/b=^1` and `a/b ^1`.
        let reqs =
            temporary_requirements(&v(&["a/b:^1", "c/d=^2", "e/f ^3"]), &[]).expect("parsed");
        assert_eq!(
            reqs,
            vec![
                ("a/b".to_owned(), "^1".to_owned()),
                ("c/d".to_owned(), "^2".to_owned()),
                ("e/f".to_owned(), "^3".to_owned())
            ]
        );
        // A `packages` entry carrying a constraint joins them; a bare name
        // does not.
        let reqs = temporary_requirements(&[], &v(&["a/b:^1", "c/d"])).expect("parsed");
        assert_eq!(reqs, vec![("a/b".to_owned(), "^1".to_owned())]);
        // `$reqs[$package] = $constraint`: the argument wins over `--with`.
        let reqs = temporary_requirements(&v(&["a/b:^1"]), &v(&["a/b:^2"])).expect("parsed");
        assert_eq!(reqs, vec![("a/b".to_owned(), "^2".to_owned())]);
        // The allow list takes the name alone.
        assert_eq!(package_of_requirement("a/b:^1"), "a/b");
        assert_eq!(package_of_requirement("a/b=^1"), "a/b");
        assert_eq!(package_of_requirement("a/b ^1"), "a/b");
        assert_eq!(package_of_requirement("a/b"), "a/b");
        assert_eq!(package_of_requirement("a/*"), "a/*");
    }

    #[test]
    fn a_pair_without_a_version_is_an_error() {
        let e = temporary_requirements(&v(&["a/b"]), &[])
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "Option a/b is missing a version constraint, use e.g. a/b:^1.0"
        );
    }
}
