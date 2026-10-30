//! `remove`: drop packages from composer.json, then resolve and install —
//! `RemoveCommand`.

use super::*;

/// `RemoveCommand::execute`: composer.json edited through
/// `JsonConfigSource`, `allow-plugins` cleaned up, then a partial update
/// (list = removed packages, "with dependencies except root requirements"
/// mode by default), composer.json restored if it fails.
pub(crate) fn run_remove(args: &RemoveArgs) -> anyhow::Result<i32> {
    use vivacity_resolver::config_source::{composer_file, JsonConfigSource};
    // `BaseCommand::initialize`, before Symfony validates the arguments: the
    // root-version warning comes first; written again below.
    let root_version_warning =
        hold_root_version_warning(&project_dir(args.working_dir.as_deref())?);
    let ignored_lock_warning =
        hold_ignored_lock_warning(&project_dir(args.working_dir.as_deref())?);
    platform_requirement_env(args.ignore_platform_reqs, &args.ignore_platform_req, true);
    let env_flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
    if args.minimal_changes || env_flag("COMPOSER_MINIMAL_CHANGES") {
        anyhow::bail!("`--minimal-changes` (update-with-minimal-changes) is not supported yet");
    }
    // `--dry-run`: the links are collected instead of removed from the
    // file, and the root package is patched in memory (`$toRemove`).
    let dry_run = args.dry_run;
    let mut to_remove: Vec<(bool, String)> = Vec::new();
    if args.packages.is_empty() && !args.unused {
        vivacity_core::say_always!("Not enough arguments (missing: \"packages\").");
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
    // Before the manifest is edited: Composer refuses it while loading the
    // root package, so nothing is written and no revert notice is printed.
    if let Some(code) = refuse_invalid_root(&manifest) {
        return Ok(code);
    }
    // `BaseCommand::initialize` loads the installed plugins on the manifest
    // as it is; a refusal there is swallowed (`tryComposer`): the edit goes
    // on and the command's own load, on the edited manifest, decides —
    // removing the refused plugin itself works.
    let first_load = (!args.no_plugins).then(|| crate::plugin_allow::load_for(&project, &manifest));
    if let Some(load) = &first_load {
        if let Some(name) = &load.hand_over {
            return crate::delegate_resolution(
                &[crate::plugin_allow::hand_over_issue(name)],
                args.no_fallback,
            );
        }
        if let Some(issue) = crate::plugin_allow::deprecation_issue(load) {
            return crate::delegate_resolution(&[issue], args.no_fallback);
        }
        // `--unused` asks for the `Composer` before the edit.
        if load.refusal.is_some() && args.unused {
            if let Some(issue) = crate::plugin_allow::root_issue() {
                return crate::delegate_resolution(&[issue], args.no_fallback);
            }
            return crate::plugin_allow::refuse(&project, &manifest, load, 2);
        }
        crate::plugin_allow::hold_warnings(load);
    }
    let first_refused = first_load.as_ref().is_some_and(|l| l.refusal.is_some());
    refuse_merge_plugin_resolution(&project, &manifest, !args.no_plugins)?;
    if serde_json::from_str::<serde_json::Value>(&manifest_text)
        .ok()
        .and_then(|m| m.get("config")?.get("update-with-minimal-changes").cloned())
        .is_some_and(|v| v == serde_json::Value::Bool(true))
    {
        anyhow::bail!("config.update-with-minimal-changes is not supported yet");
    }
    let mut packages: Vec<String> = args.packages.iter().map(|p| p.to_lowercase()).collect();

    // `Locker::isLocked`: `config.lock` on (the `Locker` reads `/dev/null`
    // otherwise), and a readable lock that has a `packages` key.
    let locked_data: Option<serde_json::Value> = if config_lock_enabled(&manifest_text) {
        std::fs::read_to_string(project.join("composer.lock"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(|v: &serde_json::Value| v.get("packages").is_some())
    } else {
        None
    };
    if args.unused {
        let lock = locked_data.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "A valid composer.lock file is required to run this command with --unused"
            )
        })?;
        packages.extend(unused_locked_packages(&manifest, &lock));
        if packages.is_empty() {
            vivacity_core::say!("No unused packages to remove");
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
        !args.dry_run,
        !(args.update_no_dev || env_flag("COMPOSER_NO_DEV")),
    )? {
        return Ok(code);
    }
    // The first `Composer`'s Flex read its index on these arguments, as it
    // activated: after `Factory`'s lines, before `BaseCommand::initialize`'s
    // (the platform variables); refused, the run is Composer's, before the
    // edit.
    match crate::flex::activation_lines(&project, &manifest, !args.no_plugins, &args.packages) {
        Ok(lines) => {
            vivacity_core::io::hold_after_factory(vivacity_core::io::Verbosity::Normal, &lines)
        }
        Err(reason) => {
            return crate::delegate_resolution(
                &[vivacity_core::scope::ScopeIssue::ResolutionPlugin(
                    "symfony/flex".to_owned(),
                    reason,
                )],
                args.no_fallback,
            );
        }
    }
    let backup = manifest_text.clone();
    let json = JsonConfigSource::new(&file);
    let (link_type, alt_type) = if args.dev {
        ("require-dev", "require")
    } else {
        ("require", "require-dev")
    };
    if args.update_with_dependencies {
        vivacity_core::say!("You are using the deprecated option \"update-with-dependencies\". This is now default behaviour. The --no-update-with-dependencies option can be used to remove a package without its dependencies.");
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
            vivacity_core::say!(
                "{name} could not be found in {link_type} but it is present in {alt_type}"
            );
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
                vivacity_core::say!(
                    "{matched} could not be found in {link_type} but it is present in {alt_type}"
                );
            }
        } else {
            vivacity_core::say!(
                "{package} is not required in your composer.json and has not been removed"
            );
        }
    }
    vivacity_core::say!("{file_label} has been updated");
    if args.no_update {
        return Ok(0);
    }
    // `allow-plugins`: the merged config (project then global); entries
    // whose key is a removed package are dropped from composer.json.
    let updated: serde_json::Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    // `resetComposer` + `requireComposer` (RemoveCommand.php:216-222) load
    // the root package again, warning included, then the installed plugins
    // on the manifest as written (unedited under `--dry-run`), before
    // `COMMAND`. When the first load was refused, `tryComposer` (:216) runs
    // one more load first; a refusal now stops the command, composer.json
    // left edited.
    let held: Vec<&String> = [&root_version_warning, &ignored_lock_warning]
        .into_iter()
        .flatten()
        .collect();
    let second_load = (!args.no_plugins).then(|| crate::plugin_allow::load_for(&project, &updated));
    if let Some(load) = &second_load {
        let issue = match &load.hand_over {
            Some(name) => Some(crate::plugin_allow::hand_over_issue(name)),
            None if load.refusal.is_some() => crate::plugin_allow::root_issue(),
            None => None,
        };
        if let Some(issue) = issue {
            std::fs::write(&file, &backup)
                .with_context(|| format!("cannot restore {}", file.display()))?;
            return crate::delegate_resolution(&[issue], args.no_fallback);
        }
    }
    for line in &held {
        vivacity_core::say!("{line}");
    }
    if let Some(load) = &second_load {
        for attempt in 0..if first_refused { 2 } else { 1 } {
            if attempt > 0 {
                for line in &held {
                    vivacity_core::say!("{line}");
                }
            }
            for w in &load.warnings {
                vivacity_core::say!("{w}");
            }
        }
        if let Some(refusal) = &load.refusal {
            crate::plugin_allow::activate_before_refusal(&project, &updated, load)?;
            vivacity_core::io::github_action_error(refusal);
            for line in &held {
                vivacity_core::say!("{line}");
            }
            vivacity_core::say_always!("{refusal}");
            return Ok(1);
        }
    }
    // That `Composer` loads every installed plugin's classes again: renamed,
    // as the -v lines print them.
    crate::verbose_listeners::reload_plugins(&project, &updated, !args.no_plugins);
    // `PluginEvents::COMMAND` (RemoveCommand.php:239), from the Composer
    // re-created on the edited manifest: before the `allow-plugins` entries
    // go and before "Running composer update".
    crate::bamarni_command_event(&project, &updated, !args.no_plugins);
    let mut allow_plugins_at_load: Option<serde_json::Value> = None;
    if let (false, Some(serde_json::Value::Object(allow))) = (
        dry_run,
        vivacity_core::layout::merged_allow_plugins(
            updated.get("config").and_then(|c| c.get("allow-plugins")),
            vivacity_core::layout::global_allow_plugins().as_ref(),
        ),
    ) {
        let removed: Vec<&String> = allow.keys().filter(|k| packages.contains(k)).collect();
        if !removed.is_empty() {
            allow_plugins_at_load = updated
                .get("config")
                .and_then(|c| c.get("allow-plugins"))
                .cloned();
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
    // The second `Composer` (`resetComposer` + `requireComposer`): a new
    // `Config`, whose `secure-http` warnings start afresh.
    vivacity_core::secure_http::new_composer();
    // Its Flex reads the index again (refused, the first one stopped).
    for line in crate::flex::activation_lines(&project, &manifest, !args.no_plugins, &args.packages)
        .unwrap_or_default()
    {
        vivacity_core::say!("{line}");
    }
    vivacity_core::say!("Running composer update {}{flags}", packages.join(" "));
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
        // Read by the install phase's fallback line (`after_update_line`).
        compat: GlobalCompatArgs {
            no_interaction: args.no_interaction,
            no_ansi: args.no_ansi,
            ..GlobalCompatArgs::default()
        },
        no_progress: args.no_progress,
        with: Vec::new(),
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
        prefer_source: false,
        prefer_dist: false,
        prefer_install: None,
        command_dispatched: true,
        resolution_command: Some(vivacity_core::scope::ResolutionCommand::Remove),
        lockless_install: None,
        allow_plugins_at_load,
    };
    // `remove` has no prefer-stable/lowest option: only the manifest
    // counts.
    // An exception (transport, invalid manifest) propagates without a
    // restore; only a non-zero status from `Installer::run` restores.
    let ended = run_update_resolved(&update_args, options, false, false, &|| {
        std::fs::write(&file, &backup).with_context(|| format!("cannot restore {}", file.display()))
    })?;
    // Composer ran `remove` itself, from the manifest as it was.
    if ended.by_composer {
        return Ok(ended.status);
    }
    let status = ended.status;
    if status != 0 {
        vivacity_core::say!("\nRemoval failed, reverting {file_label} to its original content.");
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
            vivacity_core::say!("Removal failed, {package} is still present, it may be required by another package. See `composer why {package}`.");
            return Ok(2);
        }
    }
    Ok(status)
}

/// `$composer->getRepositoryManager()->getLocalRepository()->findPackages($name)`
/// is non-empty.
pub(crate) fn locally_installed(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    name: &str,
) -> bool {
    installed_packages(project, manifest)
        .iter()
        .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
}
