//! symfony/flex at install time and after an update: what its
//! `POST_INSTALL_CMD` / `POST_UPDATE_CMD` listeners would do, decided
//! before anything is written — `flex.rs` is its resolution-time filter.

use super::*;

/// `extra.symfony/flex.synchronize_package_json`, default true.
pub(crate) fn flex_synchronize_enabled(manifest: &serde_json::Value) -> bool {
    // `!($extra[...] ?? true)`: PHP's falsiness, so `0`, `""`, `"0"` and `[]`
    // all skip the synchronisation and print the notice.
    match manifest
        .get("extra")
        .and_then(|e| e.get("symfony/flex"))
        .and_then(|f| f.get("synchronize_package_json"))
    {
        None => true,
        Some(v) => php_truthy(v),
    }
}

/// `Flex::install` on `POST_UPDATE_CMD`. Without recipes (`plan` `None`):
/// an empty line, then `finish()` — `synchronizePackageJson`'s notices, the
/// synchronisation itself decided by `flex_sync_reason` on the solved lock,
/// `symfony.lock` unchanged — then the recipes hint when the downloader is
/// enabled (symfony/flex required by the root). With recipes: the empty
/// line, the plan's lines (`Symfony operations…`, `  - Configuring …`) and
/// its files, then `finish()`'s notice — and no hint.
pub(crate) fn print_flex_post_update(
    manifest: &serde_json::Value,
    thanks_reminder: bool,
    plan: Option<&crate::flex_recipes::FlexPlan>,
) -> anyhow::Result<()> {
    let sync = flex_synchronize_enabled(manifest);
    let enabled = flex_downloader_enabled(manifest);
    // `fetchRecipes` → `Downloader::getRecipes` → `initialize`: the
    // endpoints' requests, in order, unless `PRE_POOL_CREATE` made them
    // already — `secure-http` false warns about each insecure host once
    // (the recipes' own requests go to those hosts, `fetch_recipe`).
    if enabled {
        if let Ok(hosts) = crate::flex::endpoint_hosts(manifest) {
            for (scheme, host) in hosts {
                if let Some(line) = vivacity_core::secure_http::warn_once(&scheme, &host) {
                    vivacity_core::say!("{line}");
                }
            }
        }
    }
    // `fetchRecipes` writes this one before `install`'s blank line.
    if !enabled {
        vivacity_core::say!(
            "Symfony recipes are disabled: \"symfony/flex\" not found in the root composer.json"
        );
    }
    // `2 === $this->displayThanksReminder`, between the recipes and the
    // blank line. The glyphs carry their own trailing space, and the
    // template adds another one.
    if thanks_reminder {
        let (love, star) = if cfg!(windows) {
            ("love", "star")
        } else {
            ("\u{1f496} ", "\u{2605} ")
        };
        vivacity_core::say!();
        vivacity_core::say!(
            "What about running composer global require symfony/thanks && composer thanks now?"
        );
        vivacity_core::say!(
            "This will spread some {love} by sending a {star} to the GitHub repositories of your fellow package maintainers."
        );
    }
    vivacity_core::say!();
    if let Some(plan) = plan {
        plan.print_and_apply()?;
        // `finish` → `synchronizePackageJson`; the downloader is enabled
        // (there are recipes), so only the opt-out notice can show.
        if !sync {
            vivacity_core::say!("Skip synchronizing package.json with PHP packages");
        }
        return Ok(());
    }
    if !sync {
        vivacity_core::say!("Skip synchronizing package.json with PHP packages");
    } else if !enabled {
        vivacity_core::say!("Synchronizing package.json is disabled: \"symfony/flex\" not found in the root composer.json");
    }
    if enabled {
        vivacity_core::say!(
            "Run composer recipes at any time to see the status of your Symfony recipes."
        );
        vivacity_core::say!();
    }
    Ok(())
}

/// `Flex::initOptions`: `array_merge([… 'root-dir' => extra.symfony.root-dir
/// ?? '.' …], $extra)` — a TOP-LEVEL `extra.root-dir` overrides the one
/// under `extra.symfony`, because `$extra` is merged second.
pub(crate) fn flex_root_dir(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> std::path::PathBuf {
    let extra = manifest.get("extra");
    extra
        .and_then(|e| e.get("root-dir"))
        .or_else(|| {
            extra
                .and_then(|e| e.get("symfony"))
                .and_then(|s| s.get("root-dir"))
        })
        .and_then(serde_json::Value::as_str)
        .map(|d| project.join(d))
        .unwrap_or_else(|| project.to_path_buf())
}

/// `Flex::install`'s `.env` copy: `<root-dir>/<runtime.dotenv_path ?? .env>`
/// is created from its `.dist` when neither it nor its `.local` exists and
/// the `.dist` does not mention `.env.local`.
pub(crate) fn flex_dotenv_paths(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let root_dir = flex_root_dir(project, manifest);
    let dotenv = manifest
        .get("extra")
        .and_then(|e| e.get("runtime"))
        .and_then(|r| r.get("dotenv_path"))
        .and_then(serde_json::Value::as_str)
        .map(|p| root_dir.join(p))
        .unwrap_or_else(|| root_dir.join(".env"));
    let dist = std::path::PathBuf::from(format!("{}.dist", dotenv.to_string_lossy()));
    (dotenv, dist)
}

/// `Downloader::isEnabled`: recipes apply only when `symfony/flex` is a
/// root requirement.
pub(crate) fn flex_downloader_enabled(manifest: &serde_json::Value) -> bool {
    // An exact-case lookup of an exact-case key. `ArrayLoader::parseLinks`
    // does lowercase its targets and skip a non-string constraint, but
    // neither can be reached from a root manifest: `RootPackageLoader`
    // refuses `require.Symfony/Flex` outright ("it should not contain
    // uppercase characters") and the JSON schema refuses anything but a
    // string as a constraint. Both measured against Composer 2.10.3.
    ["require", "require-dev"].iter().any(|k| {
        manifest
            .get(k)
            .and_then(|m| m.get("symfony/flex"))
            .is_some()
    })
}

/// `Flex::recordFlexInstall` → `$reinstall`: when the install installs
/// symfony/flex itself, `Flex::install` stops the event's propagation and
/// runs a whole second `Installer` over a freshly created Composer — this
/// time with Flex active, so with its pool filter. Composer's first
/// resolution ran without the filter, exactly as vivacity's does, and the
/// second run can settle somewhere else entirely.
///
/// This is decided outside the "is Flex an active plugin" question, since
/// the answer is precisely that it is not active yet: it is not in
/// installed.json, which is where Composer loads its plugins from.
pub(crate) fn flex_reinstall_reason(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    lock_value: &serde_json::Value,
    with_dev: bool,
    plugins_enabled: bool,
) -> Option<String> {
    if !plugins_enabled
        || vivacity_core::layout::plugin_allowed(manifest, "symfony/flex")
            != vivacity_core::layout::PluginVerdict::Allowed
    {
        return None;
    }
    let installed = installed_packages(project, manifest);
    if installed
        .iter()
        .any(|i| i.get("name").and_then(serde_json::Value::as_str) == Some("symfony/flex"))
    {
        return None;
    }
    vivacity_core::lock::Lock::from_value(lock_value)
        .wanted_packages(with_dev)
        .any(|p| p.name() == "symfony/flex")
        .then(|| {
            "would install itself, then run a second install of its own with its pool filter active (not emulated)"
                .to_owned()
        })
}

/// What symfony/flex's `POST_UPDATE_CMD` does to this `update`, decided on
/// the solved lock before anything is written.
pub(crate) enum FlexDecision {
    /// Something vivacity does not reproduce: the reason, for the hand-over.
    HandOver(String),
    /// Native: no recipe to apply (`None`), or the recipes' plan.
    Native(Option<crate::flex_recipes::FlexPlan>),
}

/// Why `symfony/flex` has to handle this `update` itself, or the plan of
/// what it writes when vivacity reproduces it.
///
/// Flex's operations are not the update transaction's: `recordOperations`
/// (`PRE_OPERATIONS_EXEC`) builds a synthetic `Transaction` between the
/// packages `symfony.lock` names and the whole resolved set, then
/// `shouldRecordOperation` keeps only the `InstallOperation`s whose name
/// `symfony.lock` does not hold — an `UpdateOperation` falls through and
/// returns false, an `UninstallOperation` is dropped by the caller. So the
/// recorded set depends on `symfony.lock` against the new lock, and not on
/// what the install lays out; its ORDER is that transaction's, and it is
/// the order of the recipes (`fetchRecipes`).
///
/// For each recorded package Flex looks for a recipe in the endpoints'
/// index (`Downloader::getRecipes`) and, failing that, for a bundle class —
/// read from `<vendor-dir>/<name>/…` as it stands at `POST_UPDATE_CMD`,
/// i.e. at the version the install has just laid out: from the dist for a
/// package the install changes, from vendor/ for one it leaves alone, and
/// nowhere at all for one an installer puts outside `<vendor-dir>/<name>`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flex_install_decision(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    manifest_text: &str,
    lock_value: &serde_json::Value,
    lock_text: Option<&str>,
    with_dev: bool,
    offline: bool,
    plugins_enabled: bool,
) -> anyhow::Result<FlexDecision> {
    use serde_json::Value;
    let hand_over = |r: String| Ok(FlexDecision::HandOver(r));
    // `fetchRecipes` with the downloader disabled (symfony/flex not a root
    // requirement): the warning, and `[]` — no recipe, no auto-generated
    // one, no unconfiguring. The warning is `print_flex_post_update`'s.
    if !flex_downloader_enabled(manifest) {
        return Ok(FlexDecision::Native(None));
    }
    let lock = vivacity_core::lock::Lock::from_value(lock_value);
    let installed = installed_packages(project, manifest);
    let locked = flex::lock_names(project);
    // `Flex::record` on `POST_PACKAGE_UNINSTALL` records every uninstall
    // (`shouldRecordOperation` returns true for them outright), and
    // `fetchRecipes` then drops the name from `symfony.lock` and
    // unconfigures it: with `$uninstall` true, `getClassNames()` returns
    // every candidate class without reading a single file, so a removal
    // almost always yields an auto-generated recipe. A removal of a name
    // `symfony.lock` does not hold is skipped before any write, and with
    // `--no-dev` a package the new lock has under `packages-dev` is not
    // recorded at all.
    let wanted: std::collections::BTreeSet<&str> =
        lock.wanted_packages(with_dev).map(|p| p.name()).collect();
    let dev_names: std::collections::BTreeSet<&str> = lock_value
        .get("packages-dev")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("name").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    // The uninstalls Flex records but `fetchRecipes` skips (a name
    // `symfony.lock` does not hold): `getRecipes` still looks them up.
    let mut uninstalled: Vec<String> = Vec::new();
    for entry in &installed {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        if wanted.contains(name) {
            continue;
        }
        if !with_dev && dev_names.contains(name) {
            continue;
        }
        if !locked.contains(name) {
            uninstalled.push(name.to_owned());
            continue;
        }
        return hand_over(format!(
            "would unconfigure {name} and drop it from symfony.lock (not emulated)"
        ));
    }
    if let Some(reason) = flex::unreadable_lock_entry(project) {
        return hand_over(reason);
    }
    let recorded = match recorded_operations(lock_value, &locked, with_dev) {
        Ok(r) => r,
        Err(e) => return hand_over(e),
    };
    if recorded.is_empty() && uninstalled.is_empty() {
        return Ok(FlexDecision::Native(None));
    }
    let by_name = |name: &str| lock.wanted_packages(with_dev).find(|p| p.name() == name);
    let mut index = flex::recipe_index(project, manifest, offline)?;
    // An uninstalled package the index lists: Flex downloads its recipe
    // (or prints `Skipping recipe…`), and a failure or a warning shows.
    for name in &uninstalled {
        if index.lists(name) {
            return hand_over(format!(
                "would look up the recipe of the removed {name} (not emulated)"
            ));
        }
    }
    if recorded.is_empty() {
        return Ok(FlexDecision::Native(None));
    }
    // `getLockedRepository(true)`: what `recipe-conflicts` and a manifest's
    // `conflict` are checked against.
    let locked_repo = match repository_versions(lock_value, true) {
        Ok(r) => r,
        Err(e) => return hand_over(e),
    };
    let find = |name: &str, constraint: &str| -> Result<bool, String> {
        let parsed = vivacity_resolver::constraint::parse_constraints(constraint)
            .map_err(|e| format!("cannot parse the constraint {constraint:?}: {e}"))?;
        let name = name.to_lowercase();
        Ok(locked_repo
            .iter()
            .any(|(n, v)| *n == name && parsed.constraint.matches_version(v)))
    };
    if let Err(e) = index.apply_conflicts(&find) {
        return hand_over(e);
    }
    let dirs = vivacity_core::dirs::Dirs::resolve(manifest).unwrap_or_default();
    let vendor = dirs.vendor_dir(project);
    let vendor_rel = dirs.vendor_rel().to_owned();
    // `Layout::resolve` places every package: an installer plugin can put
    // one outside `<vendor-dir>/<name>`, where Flex's read finds nothing.
    let layout = match vivacity_core::layout::Layout::resolve(
        project,
        &lock,
        manifest,
        with_dev,
        plugins_enabled,
    ) {
        Ok(l) => Some(l),
        Err(e) => {
            return hand_over(format!(
                "cannot place the packages to answer Flex ({})",
                e.join("; ")
            ))
        }
    };
    let mut reader: Option<flex::DistReader> = None;
    // `fetchRecipes`: a recipe from the index, or the auto-generated one.
    let mut recipes: Vec<(crate::flex_recipes::Recipe, Value)> = Vec::new();
    for name in &recorded {
        let Some(p) = by_name(name) else {
            return hand_over(format!("{name} is recorded but not in the lock"));
        };
        let raw = Value::Object(p.raw.clone());
        match index.select(p.name(), p.version(), p.raw.get("extra")) {
            flex::Selection::Unsure(r) => return hand_over(r),
            flex::Selection::AllConflicted => {
                return hand_over(format!(
                "would skip the recipe of {name}, every version of which conflicts (not emulated)"
            ))
            }
            flex::Selection::Recipe {
                version,
                endpoint,
                lock_version,
            } => {
                let Some((url, config)) = index.endpoints.get(endpoint) else {
                    return hand_over(format!("{name}'s recipe endpoint is unknown"));
                };
                if offline {
                    // Without the network, Flex falls back to its cache in
                    // "degraded mode", with two warnings, or fails.
                    return hand_over(format!("would download the recipe of {name} (--offline)"));
                }
                match fetch_recipe(project, url, config, p, &version, &lock_version, offline) {
                    Ok(r) => recipes.push(r),
                    Err(e) => return hand_over(e),
                }
            }
            flex::Selection::None => {
                let classes = match bundle_classes(
                    project,
                    p,
                    &raw,
                    &installed,
                    &vendor,
                    &vendor_rel,
                    layout.as_ref(),
                    &mut reader,
                    offline,
                ) {
                    Ok(c) => c,
                    Err(e) => return hand_over(e),
                };
                if classes.is_empty() {
                    continue;
                }
                let envs = if dev_names.contains(p.name()) {
                    vec![Value::String("dev".into()), Value::String("test".into())]
                } else {
                    vec![Value::String("all".into())]
                };
                let mut bundles = serde_json::Map::new();
                for class in classes {
                    bundles
                        .entry(class)
                        .or_insert_with(|| Value::Array(envs.clone()));
                }
                let mut manifest_map = serde_json::Map::new();
                manifest_map.insert("bundles".into(), Value::Object(bundles));
                recipes.push((
                    crate::flex_recipes::Recipe {
                        name: p.name().to_owned(),
                        origin: format!("{}:{}@auto-generated recipe", p.name(), p.version()),
                        manifest: manifest_map,
                        files: Vec::new(),
                        is_contrib: false,
                        package_type: p.package_type().to_owned(),
                    },
                    serde_json::json!({ "version": p.version() }),
                ));
            }
        }
    }
    if recipes.is_empty() {
        return Ok(FlexDecision::Native(None));
    }
    // `doesRecipeConflict`: a manifest whose `conflict` matches the lock
    // sends Flex looking for an older recipe — not reproduced.
    for (recipe, _) in &recipes {
        let Some(conflict) = recipe.manifest.get("conflict") else {
            continue;
        };
        if !crate::manifest::php_truthy(conflict) {
            continue;
        }
        let Value::Object(conflict) = conflict else {
            return hand_over(format!(
                "{}'s recipe conflict is not an object",
                recipe.name
            ));
        };
        for (other, constraint) in conflict {
            let Some(constraint) = constraint.as_str() else {
                return hand_over(format!(
                    "{}'s recipe conflict on {other} is not a string",
                    recipe.name
                ));
            };
            match find(other, constraint) {
                Ok(false) => {}
                Ok(true) => {
                    return hand_over(format!(
                        "would look for an older recipe of {}, whose recipe conflicts with {other} {constraint} (not emulated)",
                        recipe.name
                    ))
                }
                Err(e) => return hand_over(e),
            }
        }
    }
    // composer-merge-plugin can merge `extra` into the root Flex reads its
    // options from (`merge-extra`); the recipes are decided on the manifest.
    if vivacity_core::scope::active_plugins(project, manifest, plugins_enabled)
        .iter()
        .any(|p| p == vivacity_resolver::merge_plugin::PLUGIN_NAME)
    {
        return hand_over(
            "composer-merge-plugin is active, and Flex reads its options from the merged root (not combined with recipes)"
                .to_owned(),
        );
    }
    if std::env::var_os("SYMFONY_LOCKFILE").is_some_and(|v| !v.is_empty()) {
        return hand_over("SYMFONY_LOCKFILE is set (not emulated with recipes)".to_owned());
    }
    // `fetchRecipes`' return: symfony/flex, the symfony-packs, the
    // metapackages, then symfony/framework-bundle ahead of the rest.
    let rank = |r: &crate::flex_recipes::Recipe| -> u8 {
        if r.package_type == "symfony-pack" {
            1
        } else if r.package_type == "metapackage" {
            2
        } else if r.name == "symfony/flex" {
            0
        } else if r.name == "symfony/framework-bundle" {
            3
        } else {
            4
        }
    };
    recipes.sort_by_key(|(r, _)| rank(r));
    // Local repository after the install (`isPackageInstalled`).
    let local = match repository_versions(lock_value, with_dev) {
        Ok(r) => r,
        Err(e) => return hand_over(e),
    };
    let mut ctx =
        match crate::flex_recipes::Ctx::new(project, manifest, "symfony.lock", &local, lock_text) {
            Ok(c) => c,
            Err(e) => return hand_over(e),
        };
    for (recipe, entry) in &recipes {
        ctx.lock.set(&recipe.name, entry.clone());
    }
    let list: Vec<crate::flex_recipes::Recipe> = recipes.into_iter().map(|(r, _)| r).collect();
    if let Err(e) = crate::flex_recipes::run_recipes(&mut ctx, &list, manifest, manifest_text) {
        return hand_over(e);
    }
    // `finish` synchronises package.json / importmap.php after the recipes,
    // on the files they leave.
    if let Some(reason) = flex_sync_reason_in(project, manifest, lock_value, Some(&ctx.files)) {
        return hand_over(reason);
    }
    // The decision is taken on the project as it stands now; the install
    // writes some of it before the event (bin proxies, packages laid out —
    // or removed — outside vendor/, c3.php). A path Flex would look at
    // there is not decided here.
    let areas = install_areas(&dirs, layout.as_ref(), &lock, with_dev);
    for seen in ctx.files.observed() {
        if let Some(area) = areas.iter().find(|a| {
            seen == **a || seen.starts_with(&format!("{a}/")) || a.starts_with(&format!("{seen}/"))
        }) {
            return hand_over(format!(
                "a recipe reads or writes {seen}, which the install itself writes ({area})"
            ));
        }
    }
    Ok(FlexDecision::Native(Some(crate::flex_recipes::into_plan(
        ctx,
    ))))
}

/// The project paths the install writes before `POST_UPDATE_CMD`: the
/// vendor and bin directories, every package laid out outside the vendor
/// directory, codeception/c3's `c3.php`.
fn install_areas(
    dirs: &vivacity_core::dirs::Dirs,
    layout: Option<&vivacity_core::layout::Layout>,
    lock: &vivacity_core::lock::Lock,
    with_dev: bool,
) -> Vec<String> {
    let mut areas = vec![
        dirs.vendor_rel().trim_end_matches('/').to_owned(),
        dirs.bin_rel().trim_end_matches('/').to_owned(),
        "c3.php".to_owned(),
    ];
    if let Some(layout) = layout {
        for p in lock.wanted_packages(with_dev) {
            if let Some(rel) = layout.rel(p.name()) {
                if !rel.starts_with(&format!("{}/", dirs.vendor_rel())) {
                    areas.push(rel.trim_end_matches('/').to_owned());
                }
            }
        }
        // What the install removes: installed.json's packages it drops,
        // where they were laid out.
        let root = layout.root().to_path_buf();
        for (_, dir) in layout.removals() {
            if let Ok(rel) = dir.strip_prefix(&root) {
                areas.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    areas.retain(|a| !a.is_empty() && !a.starts_with("..") && !a.starts_with('/'));
    areas
}

/// `recordOperations`: the names of the install operations of a
/// `Transaction` from `symfony.lock`'s packages to the lock's (at the
/// install's dev mode), in that transaction's order, minus the names
/// `symfony.lock` holds.
fn recorded_operations(
    lock_value: &serde_json::Value,
    locked: &std::collections::BTreeSet<String>,
    with_dev: bool,
) -> Result<Vec<String>, String> {
    use vivacity_resolver::package::{Origin, Package};
    use vivacity_resolver::transaction::{Operation, Transaction};
    let mut arena: Vec<Package> = Vec::new();
    // Only the names matter: a present package never yields an install.
    let mut present = Vec::new();
    for name in locked {
        arena.push(Package::new(name, "1.0.0.0", "1.0.0", Origin::Locked));
        present.push(arena.len() - 1);
    }
    let result =
        vivacity_resolver::repository::locked_repository_with(lock_value, &mut arena, with_dev)
            .map_err(|e| format!("composer.lock: {}", e.0))?;
    let tx = Transaction::new(&arena, &present, &result);
    Ok(tx
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Install(i) => Some(arena[*i].name.clone()),
            _ => None,
        })
        .filter(|n| !locked.contains(n))
        .collect())
}

/// The packages of the lock as a repository (`getLockedRepository`, aliases
/// included): name and normalized version, for `findPackage`.
fn repository_versions(
    lock_value: &serde_json::Value,
    with_dev: bool,
) -> Result<Vec<(String, String)>, String> {
    let mut arena = Vec::new();
    let ids =
        vivacity_resolver::repository::locked_repository_with(lock_value, &mut arena, with_dev)
            .map_err(|e| format!("composer.lock: {}", e.0))?;
    Ok(ids
        .into_iter()
        .map(|i| (arena[i].name.clone(), arena[i].version.clone()))
        .collect())
}

/// One recipe from the index, fetched and decoded as `getRecipes` does:
/// the `Recipe` and the `symfony.lock` entry `fetchRecipes` sets for it.
#[allow(clippy::too_many_arguments)]
fn fetch_recipe(
    project: &std::path::Path,
    endpoint_url: &str,
    endpoint: &serde_json::Value,
    p: &vivacity_core::lock::LockPackage,
    version: &str,
    lock_version: &str,
    offline: bool,
) -> Result<(crate::flex_recipes::Recipe, serde_json::Value), String> {
    use serde_json::Value;
    let name = p.name();
    if endpoint_url.starts_with("file://") {
        // HttpDownloader gives no status line for a local file, so Flex
        // never reads such an endpoint at all.
        return Err(format!(
            "{name}'s recipe comes from {endpoint_url}, an endpoint Flex does not read"
        ));
    }
    let links = endpoint.get("_links").cloned().unwrap_or(Value::Null);
    let url = flex::recipe_url(endpoint_url, &links, name, version)?;
    // `Downloader::get` sends its own `Accept` header to the GitHub API
    // (`preg_match('{^https?://api\.github\.com/}', $url)`).
    if url.starts_with("https://api.github.com/") || url.starts_with("http://api.github.com/") {
        return Err(format!(
            "{name}'s recipe comes from the GitHub API ({url}), which Flex reads with its own Accept header (not emulated)"
        ));
    }
    // `secure-http`: a recipe served from its endpoint's insecure host was
    // warned about with the index; another insecure host would be warned
    // about among the recipes' requests — not reproduced.
    {
        use vivacity_core::secure_http::{classify, insecure, Verdict};
        match classify(&url) {
            Verdict::Allowed => {}
            Verdict::Warn { host, .. } => {
                if insecure(endpoint_url).and_then(|(_, h)| h).as_deref() != Some(host.as_str()) {
                    return Err(format!(
                        "the recipe of {name} is read from {host} over an insecure protocol, a host its endpoint does not share"
                    ));
                }
            }
            Verdict::Refuse => {
                return Err(format!(
                "the recipe of {name} is read over an insecure protocol, which secure-http refuses"
            ))
            }
            Verdict::HandOver(reason) => {
                return Err(format!(
                    "the recipe of {name} is read over an insecure protocol, with {reason}"
                ))
            }
        }
    }
    let body = flex::fetch_recipe(project, &url, offline)
        .map_err(|e| format!("cannot download the recipe of {name} ({e:#})"))?;
    for key in ["warning", "info"] {
        if body.get(key).is_some_and(crate::manifest::php_truthy) {
            return Err(format!(
                "the recipe of {name} carries a {key} message (not emulated)"
            ));
        }
    }
    let Some(Value::Object(data)) = body.get("manifests").and_then(|m| m.get(name)) else {
        return Err(format!("{url} has no manifest for {name}"));
    };
    let field = |v: Option<&Value>, what: &str| -> Result<Value, String> {
        v.cloned()
            .ok_or_else(|| format!("{name}'s recipe endpoint lacks {what}"))
    };
    let repository = field(links.get("repository"), "_links.repository")?;
    let branch = field(endpoint.get("branch"), "branch")?;
    let reference = field(data.get("ref"), "a ref")?;
    let origin_template = links
        .get("origin_template")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name}'s recipe endpoint lacks _links.origin_template"))?;
    let mut files: Vec<(String, crate::flex_recipes::RecipeFile)> = Vec::new();
    match data.get("files") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (path, file) in map {
                let contents = match file.get("contents") {
                    Some(Value::Array(lines)) => {
                        let lines: Vec<&str> = lines
                            .iter()
                            .map(|l| {
                                l.as_str().ok_or_else(|| {
                                    format!("{name}'s recipe file {path} has a non-string line")
                                })
                            })
                            .collect::<Result<_, _>>()?;
                        lines.join("\n").into_bytes()
                    }
                    Some(Value::String(b64)) => {
                        use base64::Engine as _;
                        base64::engine::general_purpose::STANDARD
                            .decode(b64)
                            .map_err(|e| format!("{name}'s recipe file {path}: {e}"))?
                    }
                    _ => return Err(format!("{name}'s recipe file {path} has no contents")),
                };
                let executable = file.get("executable").and_then(Value::as_bool);
                files.push((
                    path.clone(),
                    crate::flex_recipes::RecipeFile {
                        contents,
                        executable,
                    },
                ));
            }
        }
        Some(_) => return Err(format!("{name}'s recipe files are not an object")),
    }
    // `$manifest + [...]`: the recipe's own keys win.
    // An `origin` key the recipe sets itself wins, even null
    // (`getOrigin()`: `?? ''`).
    let origin = match data.get("origin") {
        Some(Value::String(o)) => o.clone(),
        Some(Value::Null) => String::new(),
        Some(other) => return Err(format!("{name}'s recipe origin is {other}")),
        None => origin_template
            .replace("{package}", name)
            .replace("{version}", version),
    };
    let is_contrib = match data.get("is_contrib") {
        Some(v) => crate::manifest::php_truthy(v),
        None => endpoint
            .get("is_contrib")
            .is_some_and(crate::manifest::php_truthy),
    };
    let Some(Value::Object(manifest)) = data.get("manifest") else {
        return Err(format!("{name}'s recipe has no manifest"));
    };
    let entry = serde_json::json!({
        "version": lock_version,
        "recipe": {
            "repo": repository,
            "branch": branch,
            "version": version,
            "ref": reference,
        },
    });
    Ok((
        crate::flex_recipes::Recipe {
            name: name.to_owned(),
            origin,
            manifest: manifest.clone(),
            files,
            is_contrib,
            package_type: p.package_type().to_owned(),
        },
        entry,
    ))
}

/// `SymfonyBundle::getClassNames` for a recorded package, from where the
/// class file stands at `POST_UPDATE_CMD`.
#[allow(clippy::too_many_arguments)]
fn bundle_classes(
    project: &std::path::Path,
    p: &vivacity_core::lock::LockPackage,
    raw: &serde_json::Value,
    installed: &[serde_json::Value],
    vendor: &std::path::Path,
    vendor_rel: &str,
    layout: Option<&vivacity_core::layout::Layout>,
    reader: &mut Option<flex::DistReader>,
    offline: bool,
) -> Result<Vec<String>, String> {
    // No candidate class file at all — the commonest case, and the one
    // `getClassNames` answers without opening anything. Asked first, so a
    // package with no dist to fetch (a metapackage) is not handed over for
    // a read it would never do.
    if flex::bundle_candidate_paths(raw).is_empty() {
        return Ok(Vec::new());
    }
    let at_vendor = layout.is_none_or(|l| {
        l.rel(p.name())
            .is_none_or(|rel| rel == format!("{vendor_rel}/{}", p.name()))
    });
    let unanswerable = |e: String| {
        format!(
            "cannot tell whether {} registers a bundle before writing ({e})",
            p.name()
        )
    };
    if !at_vendor {
        // Flex reads `<vendor-dir>/<name>`, where nothing will stand.
        return flex::bundle_classes_from(raw, &flex::ClassSource::Elsewhere)
            .map_err(|e| unanswerable(format!("{e:#}")));
    }
    if installed_as_locked(installed, p) {
        let root = vendor.join(p.name());
        return flex::bundle_classes_from(raw, &flex::ClassSource::LaidOut(&root))
            .map_err(|e| unanswerable(format!("{e:#}")));
    }
    if reader.is_none() {
        *reader = Some(flex::DistReader::new(project).map_err(|e| unanswerable(format!("{e:#}")))?);
    }
    let Some(r) = reader.as_ref() else {
        return Err(unanswerable("no reader".to_owned()));
    };
    // Unanswerable before writing — hand over rather than guess: guessing
    // wrong loses a bundle registration.
    let (bytes, kind) = r
        .dist_bytes(p, offline)
        .map_err(|e| unanswerable(format!("{e:#}")))?;
    flex::bundle_classes_from(
        raw,
        &flex::ClassSource::Dist {
            bytes: &bytes,
            kind,
        },
    )
    .map_err(|e| unanswerable(format!("{e:#}")))
}

/// `Flex::finish` → `synchronizePackageJson`: why it has to run, or
/// `None` when it would write nothing and print nothing.
///
/// `shouldSynchronize()` is `package.json || importmap.php`, which is far
/// wider than what actually gets written. With `importmap.php` — which
/// wins when both exist — `synchronizeForAssetMapper` calls
/// `updateImportMap`, which returns before reading the file when no entry
/// was resolved, and `updateControllersJsonFile`, which returns when
/// `assets/controllers.json` is absent; it returns false, so the three
/// stderr lines are not printed either. Entries only come from packages
/// carrying the `symfony-ux` keyword, and keywords travel in the lock.
///
/// With `package.json`: an unparsable file returns false before any write;
/// otherwise `removeObsoletePackageJsonLinks` writes **unconditionally**,
/// and since `JsonManipulator` holds `trim($contents)` and its
/// `getContents()` appends the newline, that write is only byte-preserving
/// on a file that already has that shape.
pub(crate) fn flex_sync_reason(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    lock_value: &serde_json::Value,
) -> Option<String> {
    flex_sync_reason_in(project, manifest, lock_value, None)
}

/// [`flex_sync_reason`] on the project as the recipes leave it: `finish`
/// synchronises AFTER them, so a recipe that creates `importmap.php`,
/// `package.json` or `assets/controllers.json` changes the answer. With
/// recipes, `root-dir` is `.` (anything else was refused).
pub(crate) fn flex_sync_reason_in(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    lock_value: &serde_json::Value,
    files: Option<&crate::flex_recipes::Vfs>,
) -> Option<String> {
    // Both cases print a notice instead of synchronising, and
    // `print_flex_post_update` prints it.
    if !flex_synchronize_enabled(manifest) || !flex_downloader_enabled(manifest) {
        return None;
    }
    let root_dir = flex_root_dir(project, manifest);
    let exists = |rel: &str| match files {
        // An unreadable state counts as present: more checks, never fewer.
        Some(f) => f.exists(rel).unwrap_or(true),
        None => root_dir.join(rel).exists(),
    };
    let importmap = exists("importmap.php");
    let package_json = root_dir.join("package.json");
    if !importmap && !exists("package.json") {
        return None;
    }
    let ux = flex_ux_package(lock_value);
    let controllers = exists("assets/controllers.json");
    if importmap {
        if let Some(name) = ux {
            return Some(format!(
                "would add the symfony-ux assets of {name} to importmap.php (not emulated)"
            ));
        }
        if controllers {
            return Some("would update assets/controllers.json (not emulated)".to_owned());
        }
        return None;
    }
    let read = match files {
        Some(f) => f
            .read("package.json")
            .map_err(|e| e.to_string())
            .and_then(|b| b.ok_or_else(|| "no file".to_owned()))
            .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string())),
        None => std::fs::read_to_string(&package_json).map_err(|e| e.to_string()),
    };
    let text = match read {
        Ok(t) => t,
        Err(e) => {
            return Some(format!(
                "would rewrite {}, which is unreadable here ({e})",
                package_json.display()
            ))
        }
    };
    // `JsonFile::parseJson` failing is Flex's own way out, before any write.
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return None,
    };
    // `JsonManipulator` demands an object and throws on anything else:
    // Composer would abort, which is not something to reproduce.
    let Some(object) = json.as_object() else {
        return Some("would fail on a package.json that is not an object".to_owned());
    };
    if let Some(name) = ux {
        return Some(format!(
            "would add the JavaScript dependencies of {name} to package.json (not emulated)"
        ));
    }
    if controllers {
        return Some("would update assets/controllers.json (not emulated)".to_owned());
    }
    // `removeObsoletePackageJsonLinks`: an `@…` link to
    // `file:<vendor-dir>/…/assets` whose package.json is gone is dropped.
    let vendor = vivacity_core::dirs::Dirs::resolve(manifest)
        .unwrap_or_default()
        .vendor_dir(project);
    // `trim(makePathRelative($vendorDir, $rootDir), '/')`: a vendor
    // directory outside the root is `../vendor`, not a reason to give up.
    let vendor_rel = match vendor.strip_prefix(&root_dir) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => vivacity_core::pathutil::find_shortest_path(
            &root_dir.to_string_lossy(),
            &vendor.to_string_lossy(),
            true,
        ),
    };
    let prefix = format!("file:{}/", vendor_rel.trim_end_matches('/'));
    for key in ["dependencies", "devDependencies"] {
        for (name, value) in object
            .get(key)
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(target) = value.as_str() else {
                continue;
            };
            if !name.starts_with('@') || !target.starts_with(&prefix) || !target.contains("/assets")
            {
                continue;
            }
            if !root_dir.join(&target[5..]).join("package.json").exists() {
                return Some(format!(
                    "would remove the obsolete package.json link {name} (not emulated)"
                ));
            }
        }
    }
    // That unconditional write, byte for byte. The port of the manipulator
    // answers it: its constructor holds the whole subtlety (`trim`, an empty
    // object spread over two lines, the newline taken from the TRIMMED
    // text), and `oracle_json_manipulator` already holds that port to the
    // real class — including with no operation at all, which is this case.
    let Ok(manipulator) = vivacity_resolver::json_manipulator::JsonManipulator::new(&text) else {
        return Some("would fail on a package.json that is not an object".to_owned());
    };
    if manipulator.contents() != text {
        return Some(
            "would rewrite package.json, which Composer's JsonManipulator does not hand back byte for byte"
                .to_owned(),
        );
    }
    None
}

/// The first locked package carrying the `symfony-ux` keyword, the one
/// thing `resolvePackageJson` looks at (`in_array('symfony-ux',
/// $phpPackage['keywords'] ?? [], true)`). Flex merges `packages` and
/// `packages-dev` whatever the install's dev mode.
pub(crate) fn flex_ux_package(lock_value: &serde_json::Value) -> Option<&str> {
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| lock_value.get(k))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .find(|p| {
            p.get("keywords")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|k| k.iter().any(|w| w.as_str() == Some("symfony-ux")))
        })
        .and_then(|p| p.get("name"))
        .and_then(serde_json::Value::as_str)
}

/// `Flex::install` on `POST_INSTALL_CMD` (an `install`, not an update):
/// `fetchRecipes` is not called — the event is not POST_UPDATE_CMD — so
/// the listener only copies `.env` and prints. Returns the `.env` it
/// created, for the caller's note.
pub(crate) fn emulate_flex_install(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    plugins_enabled: bool,
) -> anyhow::Result<()> {
    if !plugins_enabled
        || !vivacity_core::scope::active_plugins(project, manifest, plugins_enabled)
            .iter()
            .any(|p| p == "symfony/flex")
    {
        return Ok(());
    }
    let (dotenv, dist) = flex_dotenv_paths(project, manifest);
    let local = std::path::PathBuf::from(format!("{}.local", dotenv.to_string_lossy()));
    if !dotenv.exists() && !local.exists() && dist.is_file() {
        let content = std::fs::read_to_string(&dist).unwrap_or_default();
        if !content.contains(".env.local") {
            std::fs::copy(&dist, &dotenv).with_context(|| {
                format!("cannot copy {} to {}", dist.display(), dotenv.display())
            })?;
        }
    }
    vivacity_core::say!();
    if flex_downloader_enabled(manifest) {
        vivacity_core::say!(
            "Run composer recipes at any time to see the status of your Symfony recipes."
        );
        vivacity_core::say!();
    }
    Ok(())
}

/// The Flex behaviours at `POST_UPDATE_CMD` that write files, decided
/// before resolving: `.env.dist` copied to `.env` (`Flex::install`) and
/// package.json / importmap.php synchronised with the lock
/// (`PackageJsonSynchronizer::shouldSynchronize`). Either → Composer.
pub(crate) fn flex_write_guard(
    project: &std::path::Path,
    manifest: &serde_json::Value,
) -> Option<String> {
    let root_dir = flex_root_dir(project, manifest);
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
    // The package.json / importmap.php synchronisation, asked of the lock
    // that is there now. It is asked again on the new lock just before it
    // is written, and that is the authoritative answer — the keywords
    // belong to the packages the update settles on. This early pass can
    // therefore only hand over: the current lock may hold a `symfony-ux`
    // package the update is about to drop, and a needless hand-over is the
    // safe direction. Without it, a project that will hand over anyway
    // would pay a full resolution first, and a resolution that cannot run
    // (an uncached recipe index offline) would mask the reason.
    let current_lock = vivacity_core::jsonfile::read(&project.join("composer.lock"))
        .map(|v| (*v).clone())
        .unwrap_or(serde_json::Value::Null);
    if let Some(reason) = flex_sync_reason(project, manifest, &current_lock) {
        return Some(reason);
    }
    // `Flex::update` → `unpack()`: every root requirement is looked up
    // (installed.json, else the repositories) and a `symfony-pack` with
    // requirements is unpacked into composer.json. Decided here from
    // installed.json and the lock: a requirement found in neither is left
    // to Composer (its type is unknown before loading it).
    // `Flex::update` with `flex-require`/`flex-require-dev` takes the other
    // branch of `unpack()`: composer.json goes back through
    // `JsonManipulator` and `file_put_contents` unconditionally, the
    // `flex-require` keys are merged into `require` and removed, and then
    // `reinstall()` runs a whole second `Installer`.
    if manifest.get("flex-require").is_some() || manifest.get("flex-require-dev").is_some() {
        return Some(
            "would merge flex-require into composer.json and run a second install of its own (not emulated)"
                .to_owned(),
        );
    }
    {
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

#[cfg(test)]
mod flex_sync_tests {
    use super::flex_ux_package;

    #[test]
    fn ux_package_found_in_either_lock_section() {
        let lock = serde_json::json!({
            "packages": [
                {"name": "psr/log", "keywords": ["log"]},
                {"name": "symfony/ux-icons", "keywords": ["icons", "symfony-ux"]},
            ],
            "packages-dev": [{"name": "symfony/stimulus-bundle", "keywords": ["symfony-ux"]}],
        });
        assert_eq!(flex_ux_package(&lock), Some("symfony/ux-icons"));
        // `packages-dev` is read whatever the install's dev mode.
        let dev_only = serde_json::json!({
            "packages": [{"name": "psr/log"}],
            "packages-dev": [{"name": "symfony/stimulus-bundle", "keywords": ["symfony-ux"]}],
        });
        assert_eq!(flex_ux_package(&dev_only), Some("symfony/stimulus-bundle"));
        assert_eq!(flex_ux_package(&serde_json::json!({})), None);
    }
}
