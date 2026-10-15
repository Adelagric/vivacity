//! `dump-autoload`, and the autoloader dump every other command ends with —
//! `DumpAutoloadCommand`, `AutoloadGenerator::dump`.

use super::*;

/// The computing half of the dump (`vivacity_autoload::plan` with the
/// options InstallCommand derives): nothing written under vendor/.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dump_autoload_plan(
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
pub(crate) struct PlannedDump {
    plan: vivacity_autoload::DumpPlan,
    report: vivacity_autoload::DumpReport,
    runtime_stub: bool,
}

impl PlannedDump {
    pub(crate) fn finish(
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
            vivacity_core::say!("{w}");
        }
        Ok(self.report)
    }
}

/// `dump_autoload_plan` then `finish`: the dump as a single step.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dump_autoload(
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

pub(crate) fn run_dump(args: &DumpArgs) -> anyhow::Result<i32> {
    let t0 = std::time::Instant::now();
    let project = project_dir(args.working_dir.as_deref())?;
    refuse_alternate_manifest()?;
    if let Some(code) = refuse_missing_manifest(&project) {
        return Ok(code);
    }
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(project.join("composer.json"))
            .context("cannot read composer.json")?,
    )
    .context("invalid composer.json")?;
    if let Some(code) = refuse_invalid_root(&manifest) {
        return Ok(code);
    }
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
            vivacity_core::say_always!(
                "vivacity: this lock is outside what vivacity handles natively:"
            );
            vivacity_core::say_always!("  - wikimedia/composer-merge-plugin: {reason}");
            vivacity_core::say_always!("Run `composer dump-autoload` instead.");
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
            vivacity_core::say_always!(
                "vivacity: this lock is outside what vivacity handles natively:"
            );
            for i in &issues {
                vivacity_core::say_always!("  - {i}");
            }
            vivacity_core::say_always!("Run `composer dump-autoload` instead.");
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
            vivacity_core::say_always!(
                "vivacity: this lock is outside what vivacity handles natively:"
            );
            for i in &issues {
                vivacity_core::say_always!("  - {i}");
            }
            vivacity_core::say_always!("Run `composer dump-autoload` instead.");
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
                vivacity_core::say_always!("vivacity: {e}");
                return Ok(3);
            }
        }
    } else {
        None
    };
    // `DumpAutoloadCommand`: its two lines go to STDOUT (`$io->write`), the
    // first before `AutoloadGenerator::dump` (whose PRE_AUTOLOAD_DUMP comes
    // after it), the second once the dump and its POST_AUTOLOAD_DUMP are
    // done.
    let authoritative =
        args.classmap_authoritative || config_bool(&manifest, "classmap-authoritative");
    let optimize = authoritative || args.optimize || config_bool(&manifest, "optimize-autoloader");
    let kind = if authoritative {
        "optimized autoload files (authoritative)"
    } else if optimize {
        "optimized autoload files"
    } else {
        "autoload files"
    };
    vivacity_core::out!("Generating {kind}");
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
        emulate_package_versions(&project, &lock, &manifest, dev_mode, !args.no_plugins)?;
    }
    if let Some(r) = &runner {
        let code = r.run(scripts::POST_AUTOLOAD_DUMP)?;
        if code != 0 {
            return Ok(code);
        }
    }
    if optimize {
        vivacity_core::out!("Generated {kind} containing {} classes", report.classes);
    } else {
        vivacity_core::out!("Generated {kind}");
    }
    vivacity_core::say!(
        "vivacity: autoloader generated ({} classes) in {:.2}s",
        report.classes,
        t0.elapsed().as_secs_f32()
    );
    Ok(0)
}
