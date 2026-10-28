//! The second round of emulated plugins at the points Composer calls them
//! (docs/plans/plugins-round2.md; the decisions and the effects are in
//! `vivacity_core::effect_plugins` and its neighbours):
//!
//! - `PRE_AUTOLOAD_DUMP`: liborm85/composer-vendor-cleaner cleans the
//!   packages (two lines), cakephp/plugin-installer adds the application
//!   plugins to the root's autoload;
//! - `POST_AUTOLOAD_DUMP`: cakephp/plugin-installer writes
//!   `cakephp-plugins.php`;
//! - `POST_INSTALL_CMD` / `POST_UPDATE_CMD`: laminas-skeleton-installer at
//!   priority 1000 (before every listener at 0), then dg/composer-cleaner
//!   and liborm85's bin directory and summary in their plugins' activation
//!   order.

use std::io::IsTerminal as _;
use vivacity_core::effect_plugins as fx;

/// Composer's IO is interactive: stdin is a terminal and neither `-n` nor
/// `COMPOSER_NO_INTERACTION` says otherwise (`Application::doRun`).
pub(crate) fn interactive(no_interaction: bool) -> bool {
    !no_interaction
        && std::env::var_os("COMPOSER_NO_INTERACTION").is_none_or(|v| v.is_empty())
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
    let order =
        crate::plugin_order::listeners_in_order(listeners, &[fx::VENDOR_CLEANER, fx::CAKEPHP]);
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
        }
    }
    Ok((root, counts))
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
