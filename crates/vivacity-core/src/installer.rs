//! The install transaction: diff (lock vs installed state), parallel fetch
//! into the store, store-to-vendor clone, bin proxies, state files, runtime
//! stub. Idempotent (rerun after an interruption, it converges): the reference
//! state is `installed.json` + the presence of the directories, and each
//! package is laid out by cloning into a previously removed vendor/<name>.

use crate::error::{Error, Result};
use crate::fetch::{Fetcher, Provenance};
use crate::layout::Layout;
use crate::lock::{DistKind, Lock, LockPackage};
use crate::state::RootPackage;
use crate::store::Store;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

pub struct InstallOptions {
    pub with_dev: bool,
    pub offline: bool,
    /// Download/extraction parallelism.
    pub jobs: usize,
    /// No `--no-plugins`: Flex, when installed and allowed, lays out
    /// `symfony-pack` packages as metapackages.
    pub plugins_enabled: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        InstallOptions {
            with_dev: true,
            offline: false,
            jobs: 16,
            plugins_enabled: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct InstallReport {
    pub installed: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub from_cache: usize,
    pub from_network: usize,
    pub store_hits: usize,
    /// Unchanged packages extracted into the store (pre-existing vendor).
    pub store_warmed: usize,
    /// The local repository after the transaction: the lock's entries for
    /// the packages installed or updated, the previous installed.json
    /// entries for the unchanged ones (Composer keeps the loaded objects and
    /// dumps them back; the autoloader is generated from them too).
    pub local_repository: Option<Lock>,
    /// Lines Composer prints on stderr during the operations and vivacity
    /// reproduces (`Skipped installation of bin …`).
    pub messages: Vec<String>,
}

/// Installed identity of a package: version + dist reference.
fn identity(p: &LockPackage) -> (String, String) {
    (
        p.version().to_owned(),
        p.dist_reference().unwrap_or("").to_owned(),
    )
}

/// What installed.json says of a package: its identity and, for a `path`
/// package, the source it was laid out from.
struct Installed {
    identity: (String, String),
    path_source: Option<String>,
    /// The entry as written, for a package that stays.
    raw: serde_json::Map<String, Value>,
}

fn installed_packages(composer_dir: &Path) -> BTreeMap<String, Installed> {
    let mut out = BTreeMap::new();
    let path = composer_dir.join("installed.json");
    let Some(v) = crate::jsonfile::read(&path) else {
        return out;
    };
    for p in v["packages"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or_default();
        let version = p["version"].as_str().unwrap_or_default();
        let reference = p["dist"]["reference"].as_str().unwrap_or_default();
        let path_source = (p["dist"]["type"].as_str() == Some("path"))
            .then(|| p["dist"]["url"].as_str().map(str::to_owned))
            .flatten();
        out.insert(
            name.to_owned(),
            Installed {
                identity: (version.to_owned(), reference.to_owned()),
                path_source,
                raw: p.as_object().cloned().unwrap_or_default(),
            },
        );
    }
    out
}

/// The local repository the state files and the autoloader are produced
/// from: the lock, where an unchanged package keeps the entry installed.json
/// already had (minus what is recomputed).
fn local_repository(
    lock: &Lock,
    previous: &BTreeMap<String, Installed>,
    unchanged_names: &std::collections::BTreeSet<&str>,
    flex_ready: bool,
) -> Lock {
    let mut local = lock.clone();
    for p in local
        .packages
        .iter_mut()
        .chain(local.packages_dev.iter_mut())
    {
        if unchanged_names.contains(p.name()) {
            // Untouched by this run: Composer dumps the package object it
            // loaded from installed.json, so the entry keeps what it had —
            // `installation-source` included, whether or not it is there.
            if let Some(prev) = previous.get(p.name()) {
                let mut raw = prev.raw.clone();
                for key in ["version_normalized", "install-path"] {
                    raw.remove(key);
                }
                p.raw = raw;
            }
            continue;
        }
        // Laid out by this run: the DOWNLOAD phase sets the source, and it
        // runs for the whole batch before any operation executes. A
        // metapackage is never downloaded; a `symfony-pack` escapes it only
        // when Flex was already registered by then (`flex_ready`) —
        // installed in the same run, the pack is downloaded first and laid
        // out afterwards by Flex's metapackage installer.
        p.raw.remove("installation-source");
        if !p.is_virtual(flex_ready) {
            p.raw.insert(
                "installation-source".to_owned(),
                serde_json::Value::String("dist".to_owned()),
            );
        }
    }
    local
}

/// The local repository `install` will produce when its transaction is
/// empty (every wanted package present in installed.json with the same
/// identity, its directory in place): computable before the install runs,
/// so the autoloader can be planned while the install waits on the
/// network. `None` when a wanted package is not that — the caller then
/// plans after the install, as before.
pub fn local_repository_if_unchanged(lock: &Lock, layout: &Layout, with_dev: bool) -> Option<Lock> {
    let previous = installed_packages(&layout.composer_dir());
    let mut unchanged: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for p in lock.wanted_packages(with_dev) {
        if previous.get(p.name()).map(|i| &i.identity) != Some(&identity(p)) {
            return None;
        }
        if layout.install_path(p.name()).is_some()
            && !layout.abs(p.name()).is_some_and(|d| d.is_dir())
        {
            return None;
        }
        unchanged.insert(p.name());
    }
    // Every package is unchanged here, so the flag is not consulted.
    Some(local_repository(lock, &previous, &unchanged, false))
}

pub async fn install(
    _project_dir: &Path,
    lock: &Lock,
    root_manifest: &Value,
    layout: &Layout,
    store: Arc<Store>,
    fetcher: Arc<Fetcher>,
    opts: &InstallOptions,
) -> Result<InstallReport> {
    // Absolute root (the layout's): the relative paths of the proxies and of
    // the state files must not depend on a relative --working-dir.
    let project_dir = layout.root();
    let vendor = layout.vendor_dir();
    let bin_dir = layout.bin_dir();
    std::fs::create_dir_all(&vendor).map_err(Error::io(&vendor))?;

    let mut report = InstallReport::default();
    let wanted: Vec<&LockPackage> = lock.wanted_packages(opts.with_dev).collect();
    let wanted_names: std::collections::BTreeSet<&str> = wanted.iter().map(|p| p.name()).collect();
    // `Factory::purgePackages`: a package of installed.json whose install
    // path is gone is not installed at all (a fresh install, not an
    // update — no `removeBinaries`, no removal of the old path).
    let mut previous = installed_packages(&layout.composer_dir());
    previous.retain(|name, installed| {
        // `purgePackages` asks the installer: `MetapackageInstaller::
        // isInstalled` answers from the repository alone, so a package
        // nothing is laid out for (a metapackage, a Flex pack) is never
        // purged — there is no path to look at.
        if installed
            .raw
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some("metapackage")
            || layout.install_path(name).is_none()
        {
            return true;
        }
        layout
            .abs(name)
            .or_else(|| layout.removals().find(|(n, _)| n == name).map(|(_, d)| d))
            .is_some_and(|d| d.exists())
    });

    // To lay out: changed identity, or missing directory. Unchanged packages
    // whose store entry is missing (vendor/ laid out by Composer before vivacity)
    // are extracted into the store without being re-cloned: the classmap
    // cache applies from the next run on.
    let mut to_install: Vec<&LockPackage> = Vec::new();
    let mut to_warm: Vec<&LockPackage> = Vec::new();
    let mut unchanged_names: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for p in &wanted {
        // Installed nowhere (a metapackage, a Flex pack): the layout has no
        // path for it.
        if layout.install_path(p.name()).is_none() {
            if previous.get(p.name()).map(|i| &i.identity) == Some(&identity(p)) {
                unchanged_names.insert(p.name());
            }
            continue;
        }
        // `LibraryInstaller::isInstalled`: a dangling link is not installed
        // (`is_dir` follows links).
        let unchanged = previous.get(p.name()).map(|i| &i.identity) == Some(&identity(p))
            && layout.abs(p.name()).is_some_and(|d| d.is_dir());
        if unchanged {
            unchanged_names.insert(p.name());
            report.unchanged += 1;
            if matches!(p.dist_kind(), DistKind::Zip | DistKind::Tar)
                && !store.contains(p.name(), p.version(), p.store_reference().as_deref())
            {
                to_warm.push(p);
            }
        } else {
            to_install.push(p);
        }
    }
    // `PathDownloader::download`: a package cannot be laid out inside its
    // own source; checked before anything is written.
    for p in &to_install {
        if p.dist_kind() == DistKind::Path {
            if let (Some(dest), Some(url)) = (layout.abs(p.name()), p.dist_url()) {
                crate::path_install::check_not_inside_source(project_dir, &dest, url, p.name())?;
            }
        }
    }

    // Fetch + extraction into the store, with bounded parallelism. Packages to
    // "warm" only use the local cache (never the network) and their failure
    // is silent: it is an optimisation, not an obligation.
    let sem = Arc::new(tokio::sync::Semaphore::new(opts.jobs.max(1)));
    let mut tasks = tokio::task::JoinSet::new();
    let warm_names: std::collections::BTreeSet<&str> = to_warm.iter().map(|p| p.name()).collect();
    for p in to_install.iter().chain(to_warm.iter()) {
        if p.dist_kind() == DistKind::Path {
            continue;
        }
        if store.contains(p.name(), p.version(), p.store_reference().as_deref()) {
            report.store_hits += 1;
            continue;
        }
        let warm_only =
            warm_names.contains(p.name()) && !to_install.iter().any(|q| q.name() == p.name());
        let (name, version) = (p.name().to_owned(), p.version().to_owned());
        let dist_ref = p.store_reference();
        let kind = if p.dist_kind() == DistKind::Tar {
            crate::store::DistKind::Tar
        } else {
            crate::store::DistKind::Zip
        };
        let url = p.dist_url_expanded().ok_or_else(|| Error::Http {
            url: name.clone(),
            message:
                "package without a dist url (the scope detector should have routed to the fallback)"
                    .to_owned(),
        })?;
        let shasum = p.dist_shasum().map(str::to_owned);
        let (store, fetcher, sem) = (store.clone(), fetcher.clone(), sem.clone());
        let offline = opts.offline || warm_only;
        tasks.spawn(async move {
            let _permit = sem.acquire().await.map_err(|_| Error::Http {
                url: url.clone(),
                message: "semaphore closed".to_owned(),
            })?;
            let dist_type = match kind {
                crate::store::DistKind::Zip => "zip",
                crate::store::DistKind::Tar => "tar",
            };
            let fetched = fetcher
                .dist_bytes_of(&name, &url, dist_type, shasum.as_deref(), offline)
                .await;
            let (bytes, provenance) = match fetched {
                Ok(v) => v,
                // Warming: zip missing from the cache, do not insist.
                Err(_) if warm_only => return Ok::<Option<Provenance>, Error>(None),
                Err(e) => return Err(e),
            };
            let store_name = name.clone();
            let version2 = version.clone();
            let dist_ref2 = dist_ref.clone();
            tokio::task::spawn_blocking(move || {
                store.ensure(&store_name, &version2, dist_ref2.as_deref(), &bytes, kind)
            })
            .await
            .map_err(|e| Error::Http {
                url: name.clone(),
                message: format!("extraction task interrupted: {e}"),
            })??;
            Ok::<Option<Provenance>, Error>(Some(provenance))
        });
    }
    while let Some(joined) = tasks.join_next().await {
        let provenance = joined.map_err(|e| Error::Http {
            url: "join".to_owned(),
            message: e.to_string(),
        })??;
        match provenance {
            Some(Provenance::Cache) => report.from_cache += 1,
            Some(Provenance::Network) => report.from_network += 1,
            None => {}
        }
    }
    report.store_warmed = to_warm.len();

    // Removals: present before, no longer wanted, at the path validated by the
    // layout (old install-path = recomputed path, like LibraryInstaller).
    for name in previous.keys() {
        if !wanted_names.contains(name.as_str()) {
            report.removed += 1;
        }
    }
    for (name, dir) in layout.removals() {
        // `PathDownloader::remove`: the install path that *is* the source
        // stays (", source is still present").
        let own_source = previous
            .get(name)
            .and_then(|i| i.path_source.as_deref())
            .is_some_and(|url| {
                crate::path_install::is_own_source(project_dir, &dir.to_string_lossy(), url)
            });
        if own_source {
            continue;
        }
        if std::fs::symlink_metadata(&dir).is_ok() {
            crate::path_install::remove_path(&dir)?;
            prune_empty_parent(project_dir, &dir);
        }
    }

    // Layout: remove the old version, then clone from the store.
    // Packages land in disjoint directories → fan-out where parallel I/O
    // pays (Linux: sylius vendor/ wiped 3.75 s -> 1.68 s on ext4/WSL2);
    // sequential where it does not (APFS clonefile is metadata-bound and
    // contends: 607 ms -> 635 ms on an M4 Max). Each package stays atomic
    // (remove-before-clone); only the inter-package order changes, which
    // affects nothing but mtimes. `VIVACITY_PARALLEL_IO=0|1` overrides.
    let place = |p: &&LockPackage| -> Result<bool> {
        let (Some(pkg_root), Some(dest)) = (layout.package_root(p.name()), layout.abs(p.name()))
        else {
            return Ok(false);
        };
        if p.dist_kind() == DistKind::Path {
            let url = p.dist_url().unwrap_or_default();
            // `FileDownloader::update` removes the previous layout before
            // `install` (a link becomes a mirror when the options changed);
            // a fresh install keeps a path that already resolves to the
            // source.
            if previous.contains_key(p.name()) {
                crate::path_install::remove_path(&pkg_root)?;
            }
            crate::path_install::install(project_dir, &dest, url, p.raw.get("transport-options"))?;
            return Ok(true);
        }
        // Always start again from an empty package root (target-dir included).
        if std::fs::symlink_metadata(&pkg_root).is_ok() {
            crate::path_install::remove_path(&pkg_root)?;
        }
        let src = store.entry_path(p.name(), p.version(), p.store_reference().as_deref());
        crate::clone::clone_tree(&src, &dest)?;
        Ok(true)
    };
    let placed: Vec<bool> = if crate::platform::parallel_io() {
        use rayon::prelude::*;
        to_install.par_iter().map(place).collect::<Result<_>>()?
    } else {
        to_install.iter().map(place).collect::<Result<_>>()?
    };
    let installed: usize = placed.into_iter().filter(|placed| *placed).count();
    report.installed += installed;

    // `BinaryInstaller::removeBinaries` runs `initializeBinDir` before
    // looking at the package's binaries: an update, a removal, or the
    // reinstall of a package still listed in installed.json creates
    // vendor/bin even when nothing has a `bin`.
    let touches_installed = to_install.iter().any(|p| previous.contains_key(p.name()))
        || previous.keys().any(|n| !wanted_names.contains(n.as_str()));
    if touches_installed {
        std::fs::create_dir_all(&bin_dir).map_err(Error::io(&bin_dir))?;
    }

    // Bin proxies: rebuilt for the packages actually (re)placed
    // (`BinaryInstaller::installBinaries` on install/update), and — like
    // `Installer::run`'s `ensureBinariesPresence` over every installed
    // package — written for an unchanged package only where the proxy is
    // MISSING (`installBinaries(..., warnOnOverwrite: false)` skips an
    // existing one): a wiped `vendor/bin` comes back on a no-op install,
    // and a no-op install otherwise touches nothing. The purge of orphaned
    // proxies (removed packages) runs on `wanted`. The `.bat` follows the
    // resolved bin-compat (`full`, or `auto` on Windows/WSL), like
    // Composer's BinaryInstaller — a plain Linux/macOS install writes no
    // `.bat`.
    let bin_compat = crate::binproxy::resolve_bin_compat(root_manifest)?;
    let placed: std::collections::HashSet<&str> = to_install.iter().map(|p| p.name()).collect();
    // `LibraryInstaller::update` removes the old version's binaries before
    // installing the new ones (`removeBinaries` then `installBinaries`); a
    // fresh install only adds. The bin directory is removed when a removal
    // leaves it empty — and recreated by the next `initializeBinDir`.
    let previous_bins = |name: &str| -> Vec<String> {
        previous
            .get(name)
            .and_then(|i| i.raw.get("bin"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    for p in &wanted {
        let bins = p.bins();
        let Some(dir) = layout.abs(p.name()) else {
            continue;
        };
        if placed.contains(p.name()) && previous.contains_key(p.name()) {
            let old = previous_bins(p.name());
            let old: Vec<&str> = old.iter().map(String::as_str).collect();
            crate::binproxy::remove_binaries(&bin_dir, &old)?;
        }
        if bins.is_empty() {
            continue;
        }
        let missing = || {
            bins.iter().any(|b| {
                let b = b.trim_start_matches("./");
                let link_name = b.rsplit_once('/').map(|(_, f)| f).unwrap_or(b);
                dir.join(b).exists() && !bin_dir.join(link_name).exists()
            })
        };
        if placed.contains(p.name()) {
            report.messages.extend(crate::binproxy::install_binaries(
                &vendor,
                &bin_dir,
                p.name(),
                &dir,
                &bins,
                bin_compat,
                true,
            )?);
        } else if missing() {
            crate::binproxy::install_binaries(
                &vendor,
                &bin_dir,
                p.name(),
                &dir,
                &bins,
                bin_compat,
                false,
            )?;
        }
    }
    // Removed packages: `removeBinaries` on each (their proxies and `.bat`).
    for name in previous.keys() {
        if !wanted_names.contains(name.as_str()) {
            let old = previous_bins(name);
            let old: Vec<&str> = old.iter().map(String::as_str).collect();
            crate::binproxy::remove_binaries(&bin_dir, &old)?;
        }
    }
    prune_stale_bat_proxies(&bin_dir, &wanted, bin_compat)?;

    // State files + runtime stub, from the local repository: an unchanged
    // package keeps the entry installed.json already had (its own
    // `version_normalized`/`installation-source`/`install-path` are
    // recomputed), so a lock that changed a package's metadata without
    // changing its identity — routine with `path` packages whose reference
    // is a git HEAD or none — leaves installed.json and the autoloader as
    // Composer leaves them.
    let flex_ready = opts.plugins_enabled
        && previous.contains_key("symfony/flex")
        && matches!(
            crate::layout::plugin_allowed(root_manifest, "symfony/flex"),
            crate::layout::PluginVerdict::Allowed
        );
    let local = local_repository(lock, &previous, &unchanged_names, flex_ready);
    let root = RootPackage::detect(root_manifest, project_dir, opts.with_dev);
    crate::state::write_state_files(
        &layout.composer_dir(),
        &local,
        &root,
        root_manifest,
        opts.with_dev,
        layout,
    )?;
    report.local_repository = Some(local);

    Ok(report)
}

/// `LibraryInstaller::uninstall`: the parent directory of the removed package
/// (vendor/<ns>, web/app/plugins...) is removed if empty, never the project
/// root.
fn prune_empty_parent(project_dir: &Path, removed: &Path) {
    let Some(parent) = removed.parent() else {
        return;
    };
    if parent == project_dir {
        return;
    }
    if std::fs::read_dir(parent)
        .map(|mut d| d.next().is_none())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_dir(parent);
    }
}

/// A `<bin>.bat` next to an expected proxy is the Windows proxy of that
/// bin — kept only when the resolved bin-compat writes `.bat` proxies at
/// all; otherwise a leftover from a previous full-mode install, purged so
/// the tree converges on what Composer produces on a bare checkout.
/// Nothing else is touched: the bin directory may be the project's own.
fn prune_stale_bat_proxies(
    bin_dir: &Path,
    wanted: &[&LockPackage],
    bin_compat: crate::binproxy::BinCompat,
) -> Result<()> {
    if bin_compat == crate::binproxy::BinCompat::Full {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(bin_dir) else {
        return Ok(());
    };
    let mut expected: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in wanted {
        for bin in p.bins() {
            let bin = bin.trim_start_matches("./");
            let link = bin.rsplit_once('/').map(|(_, f)| f).unwrap_or(bin);
            expected.insert(link.to_owned());
        }
    }
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        // A declared bin named `<x>.bat` (paratest ships one) is a proxy
        // in its own right, written like any other on unix: never stale.
        if !expected.contains(&file_name)
            && file_name
                .strip_suffix(".bat")
                .is_some_and(|stem| expected.contains(stem))
        {
            let p = entry.path();
            std::fs::remove_file(&p).map_err(Error::io(&p))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod bat_prune_tests {
    use super::*;

    #[test]
    fn a_declared_bat_bin_is_not_pruned() {
        let d = tempfile::tempdir().expect("tmp");
        let bin_dir = d.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        for f in ["paratest", "paratest.bat", "phpunit", "phpunit.bat"] {
            std::fs::write(bin_dir.join(f), "x").expect("write");
        }
        // paratest declares `bin/paratest.bat` (a proxy of its own on unix);
        // phpunit does not: its `.bat` is a leftover of a full-mode install.
        let lock = Lock::parse(
            &serde_json::json!({"packages": [
                {"name": "brianium/paratest", "version": "7.3.2", "bin": ["bin/paratest", "bin/paratest.bat"]},
                {"name": "phpunit/phpunit", "version": "11.0.0", "bin": ["phpunit"]}
            ], "packages-dev": []})
            .to_string(),
        )
        .expect("lock");
        let wanted: Vec<&LockPackage> = lock.packages.iter().collect();
        prune_stale_bat_proxies(&bin_dir, &wanted, crate::binproxy::BinCompat::Proxy)
            .expect("prune");
        assert!(
            bin_dir.join("paratest.bat").exists(),
            "a declared .bat bin was pruned"
        );
        assert!(
            !bin_dir.join("phpunit.bat").exists(),
            "the stale .bat was kept"
        );
    }
}
