//! The second round of emulated plugins at the points Composer calls them
//! (docs/plans/plugins-round2.md; the decisions and the effects are in
//! `vivacity_core::effect_plugins` and its neighbours):
//!
//! - `PRE_AUTOLOAD_DUMP`: liborm85/composer-vendor-cleaner cleans the
//!   packages (two lines), cakephp/plugin-installer adds the application
//!   plugins to the root's autoload;
//! - `POST_AUTOLOAD_DUMP`: cakephp/plugin-installer writes
//!   `cakephp-plugins.php`; drupol/composer-packages regenerates its five
//!   classes (with the printing listeners, in activation order:
//!   `install::post_autoload_dump_printers`);
//! - `POST_INSTALL_CMD` / `POST_UPDATE_CMD`: laminas-skeleton-installer at
//!   priority 1000 (before every listener at 0), then dg/composer-cleaner
//!   and liborm85's bin directory and summary in their plugins' activation
//!   order.

use std::io::IsTerminal as _;
use vivacity_core::effect_plugins as fx;

/// Composer's IO is interactive: stdin is a terminal (`Application::doRun`),
/// neither `-n` nor a truthy `COMPOSER_NO_INTERACTION` (PHP's `"0"` is
/// false), and the output not quiet — Symfony's `configureIO` makes the
/// input non-interactive at `-q` or `SHELL_VERBOSITY=-1`.
pub(crate) fn interactive(no_interaction: bool) -> bool {
    let env_true = |k: &str| {
        std::env::var(k)
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0")
    };
    !no_interaction
        && !env_true("COMPOSER_NO_INTERACTION")
        && vivacity_core::io::level() != vivacity_core::io::Verbosity::Quiet
        && std::env::var("SHELL_VERBOSITY").ok().as_deref() != Some("-1")
        && std::io::stdin().is_terminal()
}

/// The packages installed in COMPOSER_HOME.
pub(crate) fn global_packages() -> Vec<serde_json::Value> {
    crate::plugin_order::global_composer_dir()
        .map(|d| crate::plugin_order::installed_json_packages(&d))
        .unwrap_or_default()
}

fn allowed(manifest: &serde_json::Value, name: &str) -> bool {
    matches!(
        vivacity_core::layout::plugin_allowed(manifest, name),
        vivacity_core::layout::PluginVerdict::Allowed
    )
}

/// The plugin is in the local repository the run ends with, allowed, with
/// plugins on.
pub(crate) fn active(
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
    plugin: &str,
) -> bool {
    plugins_enabled
        && allowed(manifest, plugin)
        && local.wanted_packages(with_dev).any(|p| p.name() == plugin)
}

/// liborm85's configuration when it cleans in this run (active,
/// configured, and not held back by `no-dev-only` in dev mode).
pub(crate) fn vendor_cleaner(
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    dev_mode: bool,
    plugins_enabled: bool,
) -> Option<vivacity_core::vendor_cleaner::Config> {
    if !active(
        local,
        manifest,
        dev_mode,
        plugins_enabled,
        fx::VENDOR_CLEANER,
    ) {
        return None;
    }
    vivacity_core::vendor_cleaner::Config::read(manifest)
        .ok()
        .flatten()
        .filter(|c| !(c.no_dev_only && dev_mode))
}

/// The two listeners that change what the dump sees, before it is planned:
/// whether the dump can be planned ahead (`plan_autoload_early`).
pub(crate) fn dump_depends_on_listeners(
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    dev_mode: bool,
    plugins_enabled: bool,
) -> bool {
    vendor_cleaner(local, manifest, dev_mode, plugins_enabled).is_some()
        || active(local, manifest, dev_mode, plugins_enabled, fx::CAKEPHP)
        || active(
            local,
            manifest,
            dev_mode,
            plugins_enabled,
            fx::CLASS_ALIAS_LOADER,
        )
}

/// The local repository as liborm85 walks it: every package (dev ones
/// included when `packages` holds them), with its install path.
fn packages_with_paths(
    packages: &[&vivacity_core::lock::LockPackage],
    layout: &vivacity_core::layout::Layout,
) -> Vec<(String, Option<std::path::PathBuf>)> {
    packages
        .iter()
        .map(|p| (p.name().to_owned(), layout.abs(p.name())))
        .collect()
}

/// `PRE_AUTOLOAD_DUMP`: liborm85's package cleaning (its lines, then, on
/// `dump-autoload`, the bin directory and the summary at once) and
/// cakephp's root autoload. Returns the root package the dump reads, and
/// the counts liborm85 reports at `POST_INSTALL_CMD`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pre_autoload_dump(
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    local: &vivacity_core::lock::Lock,
    packages: &[&vivacity_core::lock::LockPackage],
    dev_mode: bool,
    plugins_enabled: bool,
    listeners: &[String],
    whole_run: bool,
) -> anyhow::Result<(
    serde_json::Value,
    Option<vivacity_core::vendor_cleaner::Counts>,
)> {
    let mut root = manifest.clone();
    let mut counts = None;
    let cleaner = vendor_cleaner(local, manifest, dev_mode, plugins_enabled);
    let order = crate::plugin_order::listeners_in_order(
        listeners,
        &[fx::VENDOR_CLEANER, fx::CAKEPHP, fx::CLASS_ALIAS_LOADER],
    );
    for plugin in order {
        if plugin == fx::VENDOR_CLEANER {
            let Some(config) = cleaner.as_ref() else {
                continue;
            };
            vivacity_core::out!();
            vivacity_core::out!("Composer vendor cleaner: Cleaning packages in vendor directory");
            let mut c = vivacity_core::vendor_cleaner::Cleaner::new(config);
            c.cleanup_packages(&packages_with_paths(packages, layout))?;
            if whole_run {
                finish_vendor_cleaner(&mut c, layout)?;
            }
            counts = Some(c.counts);
        } else if plugin == fx::CAKEPHP
            && active(local, manifest, dev_mode, plugins_enabled, fx::CAKEPHP)
        {
            let paths = vivacity_core::cakephp_plugins::plugin_paths(manifest)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let real = vivacity_core::cakephp_plugins::real_vendor(&layout.vendor_dir())?;
            root = vivacity_core::cakephp_plugins::root_with_app_plugins(&root, &real, &paths)?;
        } else if plugin == fx::CLASS_ALIAS_LOADER
            && active(
                local,
                manifest,
                dev_mode,
                plugins_enabled,
                fx::CLASS_ALIAS_LOADER,
            )
        {
            root = class_alias_loader(manifest, &root, layout)?;
        }
    }
    Ok((root, counts))
}

/// typo3/class-alias-loader's `generateAliasMapFiles` (a run with class
/// alias maps, a `vendor-dir`, or a suffix that is not a string was handed
/// over by `effect_plugins::issues`): the suffix read from
/// `vendor/autoload.php` before the dump rewrites it, the two files, the
/// line, and the root's autoload with the include file. Without
/// `always-add-alias-loader`, nothing.
fn class_alias_loader(
    manifest: &serde_json::Value,
    root: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
) -> anyhow::Result<serde_json::Value> {
    use vivacity_core::class_alias_loader as cal;
    if !cal::always_add(manifest) {
        return Ok(root.clone());
    }
    let vendor = layout.vendor_dir();
    let configured = cal::configured_suffix(manifest).map_err(|e| anyhow::anyhow!("{e}"))?;
    let existing = std::fs::read(vendor.join("autoload.php"))
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    let suffix = cal::pick_suffix(configured, existing.as_deref(), cal::random_suffix);
    cal::write_files(&vendor, &suffix, manifest)?;
    vivacity_core::out!("Generating empty class alias map file");
    cal::root_with_include(root, "vendor").map_err(|e| anyhow::anyhow!("{e}"))
}

/// liborm85's `cleanupBinary` and `finishCleanup` (its last three lines).
fn finish_vendor_cleaner(
    c: &mut vivacity_core::vendor_cleaner::Cleaner<'_>,
    layout: &vivacity_core::layout::Layout,
) -> anyhow::Result<()> {
    if c.cleanup_binary(&layout.bin_dir())? {
        vivacity_core::out!("Composer vendor cleaner: Cleaning vendor binary directory");
    }
    vivacity_core::out!("{}", c.summary());
    vivacity_core::out!();
    Ok(())
}

/// `POST_AUTOLOAD_DUMP`: cakephp's `cakephp-plugins.php`, from the local
/// repository's packages.
pub(crate) fn post_autoload_dump(
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    local: &vivacity_core::lock::Lock,
    packages: &[&vivacity_core::lock::LockPackage],
    dev_mode: bool,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    if !active(local, manifest, dev_mode, plugins_enabled, fx::CAKEPHP) {
        return Ok(());
    }
    let paths = vivacity_core::cakephp_plugins::plugin_paths(manifest)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let vendor = layout.vendor_dir();
    let real = vivacity_core::cakephp_plugins::real_vendor(&vendor)?;
    let raws: Vec<&serde_json::Map<String, serde_json::Value>> =
        packages.iter().map(|p| &p.raw).collect();
    let contents = vivacity_core::cakephp_plugins::config_file(&raws, &real, &paths)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    vivacity_core::cakephp_plugins::write_config_file(&vendor, &contents)?;
    Ok(())
}

/// laminas-skeleton-installer's `installOptionalDependencies`, at priority
/// 1000 of `POST_INSTALL_CMD` / `POST_UPDATE_CMD`: configured,
/// non-interactive (checked before anything was written), the minimal
/// install — its two lines and composer.json rewritten.
pub(crate) fn skeleton(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    local: &vivacity_core::lock::Lock,
    with_dev: bool,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    if !active(
        local,
        manifest,
        with_dev,
        plugins_enabled,
        fx::LAMINAS_SKELETON,
    ) || fx::laminas_skeleton_configured(manifest) != Ok(true)
    {
        return Ok(());
    }
    for line in fx::SKELETON_LINES {
        vivacity_core::out!("{line}");
    }
    let file = vivacity_resolver::config_source::composer_file(project);
    fx::laminas_skeleton_rewrite(&file)?;
    Ok(())
}

/// dg/composer-cleaner's `clean` at `POST_INSTALL_CMD` / `POST_UPDATE_CMD`.
pub(crate) fn composer_cleaner(
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    local: &vivacity_core::lock::Lock,
    with_dev: bool,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    if !active(local, manifest, with_dev, plugins_enabled, fx::DG_CLEANER) {
        return Ok(());
    }
    let ignore = vivacity_core::composer_cleaner::ignore_paths(manifest)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let cleaned = vivacity_core::composer_cleaner::clean(&layout.vendor_dir(), &ignore)?;
    for e in &cleaned.errors {
        vivacity_core::say!("{e}");
    }
    vivacity_core::out!(
        "Composer cleaner: Removed {} files or directories.",
        cleaned.removed
    );
    Ok(())
}

/// liborm85's second call, at `POST_INSTALL_CMD` / `POST_UPDATE_CMD`: the
/// bin directory and the summary, with the counts of the first.
pub(crate) fn vendor_cleaner_finish(
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    local: &vivacity_core::lock::Lock,
    with_dev: bool,
    plugins_enabled: bool,
    counts: Option<&vivacity_core::vendor_cleaner::Counts>,
) -> anyhow::Result<()> {
    let (Some(config), Some(counts)) = (
        vendor_cleaner(local, manifest, with_dev, plugins_enabled),
        counts,
    ) else {
        return Ok(());
    };
    let mut c = vivacity_core::vendor_cleaner::Cleaner::new(&config);
    c.counts = counts.clone();
    finish_vendor_cleaner(&mut c, layout)
}

/// The emulated plugins whose listener prints or writes every time it runs,
/// and the script events `composer run-script` would dispatch to it again
/// under `--run-scripts` (`scripts.rs`: the plugins' own listeners of an
/// event run again through `run-script`) — where Composer's own run has
/// them once. Decided before the first script.
const REPLAYED: &[(&str, &[&str])] = &[
    (fx::DRUPOL, &[crate::scripts::POST_AUTOLOAD_DUMP]),
    (
        crate::extension_installers::INFECTION,
        &[crate::scripts::POST_INSTALL_CMD],
    ),
    (fx::CLASS_ALIAS_LOADER, &[crate::scripts::PRE_AUTOLOAD_DUMP]),
    (
        fx::PATCHER,
        &[
            crate::scripts::PRE_INSTALL_CMD,
            crate::scripts::POST_INSTALL_CMD,
        ],
    ),
];

/// A plugin of [`REPLAYED`] with `--run-scripts` and one of its events
/// declared in composer.json's `scripts` and run by this command
/// (`commands_events`: install runs the four, dump-autoload the two
/// autoload events).
pub(crate) fn replay_issue(
    manifest: &serde_json::Value,
    local: &vivacity_core::lock::Lock,
    with_dev: bool,
    plugins_enabled: bool,
    run_scripts: bool,
    commands_events: &[&str],
) -> Option<String> {
    if !run_scripts {
        return None;
    }
    let declared = crate::scripts::declared_events(manifest);
    REPLAYED.iter().find_map(|(plugin, events)| {
        let event = events
            .iter()
            .find(|e| declared.contains(**e) && commands_events.contains(e))?;
        active(local, manifest, with_dev, plugins_enabled, plugin).then(|| {
            format!("plugin {plugin} would run again through `composer run-script {event}` (not emulated)")
        })
    })
}

/// drupol/composer-packages in this run: active (plugins on, allowed, in
/// the local repository the dump reads), with the lock data its exporters
/// read and whether Flex lays `symfony-pack` packages out as metapackages.
pub(crate) struct DrupolRun<'a> {
    lock: &'a serde_json::Value,
    flex_packs: bool,
}

pub(crate) fn drupol_run<'a>(
    lock_data: Option<&'a serde_json::Value>,
    local: &vivacity_core::lock::Lock,
    manifest: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> Option<DrupolRun<'a>> {
    if !active(local, manifest, with_dev, plugins_enabled, fx::DRUPOL) {
        return None;
    }
    Some(DrupolRun {
        lock: lock_data?,
        flex_packs: local.flex_packs(manifest, with_dev, plugins_enabled),
    })
}

/// drupol/composer-packages' `Plugin::regeneration`: its two lines around
/// the five classes (`vivacity_core::composer_packages`), written under
/// `<vendor>/drupol/composer-packages/build/`. The run was checked before
/// anything was written (`effect_plugins::issues`).
pub(crate) fn composer_packages(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    run: &DrupolRun<'_>,
) -> anyhow::Result<()> {
    use vivacity_core::composer_packages as cp;
    vivacity_core::out!("drupol/composer-packages: Regenerating classes...");
    // `RootPackage::getName()`: lower case, `__root__` without a name.
    let root_name = manifest
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| "__root__".to_owned(), str::to_ascii_lowercase);
    // `LibraryInstaller`: `realpath()` of the configured vendor directory,
    // itself `Config::get('vendor-dir')` joined to the working directory
    // (`getcwd()`, physical).
    let base = vivacity_core::pathutil::canonicalize(project)?;
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .vendor_dir(&base);
    let vendor_real = cp::php_realpath(&vendor)?;
    let vendor_real = vendor_real
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("{}: not UTF-8", vendor_real.display()))?;
    let generated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let classes = cp::render(
        run.lock,
        &cp::Input {
            root_name: &root_name,
            vendor_real,
            flex_packs: run.flex_packs,
            generated_at,
        },
    )?;
    cp::write(
        &vendor
            .join("drupol")
            .join("composer-packages")
            .join("build"),
        &classes,
    )?;
    vivacity_core::out!("drupol/composer-packages: Done.");
    Ok(())
}

/// `PATH` as `EventDispatcher::ensureBinDirIsInPath` leaves it for a
/// plugin's listener: the bin directory (`realpath`) first when it exists
/// and is not in `PATH` yet. `None`: unchanged.
fn path_with_bin_dir(bin: &std::path::Path) -> Option<std::ffi::OsString> {
    if !bin.is_dir() {
        return None;
    }
    let real = std::fs::canonicalize(bin).ok()?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    if std::env::split_paths(&path).any(|p| p == real) {
        return None;
    }
    std::env::join_paths(std::iter::once(real).chain(std::env::split_paths(&path))).ok()
}

fn patcher_loaded_at_start(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> Option<Vec<serde_json::Value>> {
    if !plugins_enabled || !allowed(manifest, fx::PATCHER) {
        return None;
    }
    let installed = crate::plugin_order::installed_json_packages(
        &crate::manifest::composer_dir_of(project, manifest),
    );
    installed
        .iter()
        .any(|p| p.get("name").and_then(serde_json::Value::as_str) == Some(fx::PATCHER))
        .then_some(installed)
}

/// mlocati/composer-patcher's `preOperation` (`PRE_INSTALL_CMD`), the
/// plugin loaded from installed.json: its collection lines (a package it
/// would uninstall was handed over by `effect_plugins::issues`).
pub(crate) fn patcher_pre(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    use vivacity_core::composer_patcher as cp;
    let Some(installed) = patcher_loaded_at_start(project, manifest, plugins_enabled) else {
        return Ok(());
    };
    let Some(root) = manifest.as_object() else {
        return Ok(());
    };
    let mut entries: Vec<&serde_json::Map<String, serde_json::Value>> = installed
        .iter()
        .filter_map(serde_json::Value::as_object)
        .collect();
    entries.sort_by(|a, b| package_name(a).cmp(package_name(b)));
    let vendor = layout.vendor_dir();
    let sources: Vec<cp::Source<'_>> = entries
        .iter()
        .map(|p| cp::Source {
            package: p,
            dir: vendor.join(package_name(p)),
        })
        .collect();
    let root_source = cp::Source {
        package: root,
        dir: project.to_path_buf(),
    };
    let collection =
        cp::collect(manifest, &root_source, &sources).map_err(|e| anyhow::anyhow!("{e}"))?;
    for line in &collection.lines {
        vivacity_core::out!("{line}");
    }
    Ok(())
}

fn package_name(p: &serde_json::Map<String, serde_json::Value>) -> &str {
    p.get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

/// mlocati/composer-patcher's `postOperation` (`POST_INSTALL_CMD`): the
/// collection from the packages the install leaves, its lines, each
/// patch applied by `patch` (vivacity_core::composer_patcher), and the
/// local repository with `extra.patches_applied` set on the packages whose
/// patches all passed — `Some` when installed.json must be written again.
/// The packages in installed.json's order (by name): Composer's own order
/// after a fresh install is that of its parallel extractions.
pub(crate) fn patcher_post(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    layout: &vivacity_core::layout::Layout,
    local: &vivacity_core::lock::Lock,
    with_dev: bool,
    plugins_enabled: bool,
) -> anyhow::Result<Option<vivacity_core::lock::Lock>> {
    use vivacity_core::composer_patcher as cp;
    if !active(local, manifest, with_dev, plugins_enabled, fx::PATCHER) {
        return Ok(None);
    }
    let Some(root) = manifest.as_object() else {
        return Ok(None);
    };
    let mut packages: Vec<&vivacity_core::lock::LockPackage> =
        local.wanted_packages(with_dev).collect();
    packages.sort_by(|a, b| a.name().cmp(b.name()));
    let sources: Vec<cp::Source<'_>> = packages
        .iter()
        .map(|p| cp::Source {
            package: &p.raw,
            dir: layout
                .abs(p.name())
                .unwrap_or_else(|| layout.vendor_dir().join(p.name())),
        })
        .collect();
    let root_source = cp::Source {
        package: root,
        dir: project.to_path_buf(),
    };
    let collection =
        cp::collect(manifest, &root_source, &sources).map_err(|e| anyhow::anyhow!("{e}"))?;
    for line in &collection.lines {
        vivacity_core::out!("{line}");
    }
    if collection.patches.is_empty() {
        vivacity_core::out!("No patches supplied.");
        return Ok(None);
    }
    let mut patcher = cp::Patcher::new(path_with_bin_dir(&layout.bin_dir()));
    let mut records: Vec<(String, serde_json::Value)> = Vec::new();
    for p in &packages {
        let patches = collection.for_package(p.name(), p.version());
        if patches.is_empty() {
            continue;
        }
        let Some(dir) = layout.abs(p.name()) else {
            continue;
        };
        // A store-backed copy gets inodes of its own before `patch` writes.
        cp::detach(&dir)?;
        let mut all_passed = true;
        for patch in &patches {
            let outcome = patcher.apply(patch, &dir)?;
            vivacity_core::out!(
                "Applying patch {}/{} to {}... {}",
                patch.from_name,
                patch.description,
                p.name(),
                outcome.text()
            );
            if matches!(outcome, cp::Outcome::Failed(_)) {
                all_passed = false;
            }
        }
        if all_passed {
            if let Some(record) = cp::applied_record(&patches) {
                records.push((p.name().to_owned(), record));
            }
        }
    }
    if records.is_empty() {
        return Ok(None);
    }
    let mut patched = local.clone();
    for p in patched
        .packages
        .iter_mut()
        .chain(patched.packages_dev.iter_mut())
    {
        if let Some((_, record)) = records.iter().find(|(n, _)| n == p.name()) {
            cp::set_applied(&mut p.raw, record.clone());
        }
    }
    Ok(Some(patched))
}
