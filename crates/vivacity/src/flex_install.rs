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

/// `Flex::install` on `POST_UPDATE_CMD` after a resolution that installed
/// nothing (no operations, hence no recipes): an empty line, then
/// `finish()` — `synchronizePackageJson`'s notices, the synchronisation
/// itself decided by `flex_sync_reason` on the solved lock, `symfony.lock`
/// unchanged — then the recipes hint when the downloader is enabled
/// (symfony/flex required by the root).
pub(crate) fn print_flex_post_update(manifest: &serde_json::Value, thanks_reminder: bool) {
    let sync = flex_synchronize_enabled(manifest);
    let enabled = flex_downloader_enabled(manifest);
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

/// Why `symfony/flex` has to handle this `update` itself, or `None` when
/// it would write nothing but what vivacity already reproduces.
///
/// Flex's operations are not the update transaction's: `recordOperations`
/// (`PRE_OPERATIONS_EXEC`) builds a synthetic `Transaction` between the
/// packages `symfony.lock` names and the whole resolved set, then
/// `shouldRecordOperation` keeps only the `InstallOperation`s whose name
/// `symfony.lock` does not hold — an `UpdateOperation` falls through and
/// returns false, an `UninstallOperation` is dropped by the caller. So the
/// recorded set depends on `symfony.lock` against the new lock, and not on
/// what the install lays out.
///
/// For each recorded package Flex looks for a recipe in the endpoints'
/// index and, failing that, for a bundle class — either one means files
/// written into the project. The bundle class is read from
/// `<vendor-dir>/<name>/…` as it stands at `POST_UPDATE_CMD`, i.e. at the
/// version the install has just laid out: from the dist for a package the
/// install changes, from vendor/ for one it leaves alone, and nowhere at
/// all for one an installer puts outside `<vendor-dir>/<name>`.
pub(crate) fn flex_install_reason(
    project: &std::path::Path,
    manifest: &serde_json::Value,
    lock_value: &serde_json::Value,
    with_dev: bool,
    offline: bool,
    plugins_enabled: bool,
) -> anyhow::Result<Option<String>> {
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
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("name").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    for entry in &installed {
        let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if wanted.contains(name) || !locked.contains(name) {
            continue;
        }
        if !with_dev && dev_names.contains(name) {
            continue;
        }
        return Ok(Some(format!(
            "would unconfigure {name} and drop it from symfony.lock (not emulated)"
        )));
    }
    let recorded: Vec<&vivacity_core::lock::LockPackage> = lock
        .wanted_packages(with_dev)
        .filter(|p| !locked.contains(p.name()))
        .collect();
    if recorded.is_empty() {
        return Ok(None);
    }
    let index = flex::recipe_index(project, manifest, offline)?;
    for p in &recorded {
        if index.applies_to(p.name(), p.version(), p.raw.get("extra")) {
            return Ok(Some(format!(
                "would apply the recipe of {} (not emulated)",
                p.name()
            )));
        }
    }
    let dirs = vivacity_core::dirs::Dirs::resolve(manifest).unwrap_or_default();
    let vendor = dirs.vendor_dir(project);
    let vendor_rel = dirs.vendor_rel().to_owned();
    // `Layout::resolve` places every package: an installer plugin can put
    // one outside `<vendor-dir>/<name>`, where Flex's read finds nothing.
    let layout =
        vivacity_core::layout::Layout::resolve(project, &lock, manifest, with_dev, plugins_enabled)
            .ok();
    enum Where {
        Elsewhere,
        LaidOut,
        Dist,
    }
    let placed: Vec<(&vivacity_core::lock::LockPackage, Where)> = recorded
        .iter()
        .map(|p| {
            let at_vendor = layout.as_ref().is_none_or(|l| {
                l.rel(p.name())
                    .is_none_or(|rel| rel == format!("{vendor_rel}/{}", p.name()))
            });
            // No candidate class file at all — the commonest case, and the
            // one `getClassNames` answers without opening anything. Asked
            // first, so a package with no dist to fetch (a metapackage) is
            // not handed over for a read it would never do.
            let has_candidate =
                !flex::bundle_candidate_paths(&serde_json::Value::Object(p.raw.clone())).is_empty();
            let w = if !at_vendor || !has_candidate {
                Where::Elsewhere
            } else if installed_as_locked(&installed, p) {
                Where::LaidOut
            } else {
                Where::Dist
            };
            (*p, w)
        })
        .collect();
    let reader = if placed.iter().any(|(_, w)| matches!(w, Where::Dist)) {
        Some(flex::DistReader::new(project)?)
    } else {
        None
    };
    for (p, w) in &placed {
        let raw = serde_json::Value::Object(p.raw.clone());
        let answer = match w {
            // Flex reads `<vendor-dir>/<name>`, where nothing will stand.
            Where::Elsewhere => flex::has_bundle_class_from(&raw, &flex::ClassSource::Elsewhere),
            Where::LaidOut => {
                let root = vendor.join(p.name());
                flex::has_bundle_class_from(&raw, &flex::ClassSource::LaidOut(&root))
            }
            Where::Dist => {
                let Some(reader) = reader.as_ref() else {
                    return Ok(Some(format!(
                        "cannot read {}'s dist before writing (no reader)",
                        p.name()
                    )));
                };
                match reader.dist_bytes(p, offline) {
                    Ok((bytes, kind)) => flex::has_bundle_class_from(
                        &raw,
                        &flex::ClassSource::Dist {
                            bytes: &bytes,
                            kind,
                        },
                    ),
                    // Unanswerable before writing — hand over rather than
                    // guess: guessing wrong loses a bundle registration.
                    Err(e) => {
                        return Ok(Some(format!(
                            "cannot tell whether {} registers a bundle before writing ({e})",
                            p.name()
                        )))
                    }
                }
            }
        };
        match answer {
            Ok(true) => {
                return Ok(Some(format!(
                    "would register {}'s bundle in config/bundles.php (not emulated)",
                    p.name()
                )))
            }
            Ok(false) => {}
            Err(e) => {
                return Ok(Some(format!(
                    "cannot tell whether {} registers a bundle before writing ({e})",
                    p.name()
                )))
            }
        }
    }
    Ok(None)
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
    // Both cases print a notice instead of synchronising, and
    // `print_flex_post_update` prints it.
    if !flex_synchronize_enabled(manifest) || !flex_downloader_enabled(manifest) {
        return None;
    }
    let root_dir = flex_root_dir(project, manifest);
    let importmap = root_dir.join("importmap.php");
    let package_json = root_dir.join("package.json");
    if !importmap.exists() && !package_json.exists() {
        return None;
    }
    let ux = flex_ux_package(lock_value);
    let controllers = root_dir.join("assets").join("controllers.json").exists();
    if importmap.exists() {
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
    let text = match std::fs::read_to_string(&package_json) {
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
