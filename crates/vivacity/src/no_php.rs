//! The commands that resolve (`update`, `require`, `remove`, an `install`
//! without a lock) when no PHP can be started (docs/plans/update-without-php.md).
//!
//! Under `--ignore-platform-reqs` (or `COMPOSER_IGNORE_PLATFORM_REQS`),
//! Composer skips every rule on a platform package: the platform's content
//! no longer weighs on the resolution, so Composer run with a PHP is the
//! reference for vivacity run with none. Without that filter the platform
//! decides, and only a PHP can report it: the command stops (exit 1) and
//! says how to go on.
//!
//! The decision is taken once, at the start of the command, before anything
//! is printed or edited. In this mode a hand-over to Composer cannot work
//! (Composer needs a PHP too): every delegation stops with exit 3, its
//! reasons named ([`absent`], read by the fallback paths).

use std::sync::atomic::{AtomicBool, Ordering};

static ABSENT: AtomicBool = AtomicBool::new(false);

/// This run resolves without PHP.
pub(crate) fn absent() -> bool {
    ABSENT.load(Ordering::Acquire)
}

/// Forgets the previous run's decision (`vivacity::run` may be called
/// several times in one process).
pub(crate) fn reset() {
    ABSENT.store(false, Ordering::Release);
}

/// What the command does about PHP.
pub(crate) enum Decision {
    /// A PHP can be started: the probe runs as always.
    Php,
    /// None, under `--ignore-platform-reqs`: the run resolves without it.
    Without,
    /// None, and the run cannot go on: its exit code.
    Stop(i32),
}

/// The decision of a resolving command (`command` names it in the
/// message), `ignore_all` being `--ignore-platform-reqs` or
/// `COMPOSER_IGNORE_PLATFORM_REQS`; `solves`: the command runs the solver
/// (not `require --no-update`), whose -v lines count the platform. A list
/// of `--ignore-platform-req` (even `*`) or `config.platform` leaves the
/// platform to the probe: the extensions are not overridden, and Composer's
/// `IgnoreAll` checks (`instanceof`) do not see a list.
pub(crate) fn decide(
    command: &str,
    ignore_all: bool,
    solves: bool,
    no_fallback: bool,
) -> anyhow::Result<Decision> {
    // Already decided by `run_once` (`decide_early`), or decided now.
    if !absent() {
        if vivacity_resolver::platform::php_launchable() {
            return Ok(Decision::Php);
        }
        let php = vivacity_resolver::platform::php_command();
        if !ignore_all {
            anyhow::bail!(
                "cannot run {php}: no such executable\n\
                 `{command}` reads the platform (the PHP version and its extensions) from PHP. \
                 Point VIVACITY_PHP at a PHP executable, or pass --ignore-platform-reqs to resolve \
                 without the platform (--ignore-platform-req and config.platform leave the \
                 extensions to PHP)."
            );
        }
        ABSENT.store(true, Ordering::Release);
    }
    // `Installer.php:560-561`: under -v, the pool and rule counts include
    // the platform packages a PHP would have listed.
    if solves && vivacity_core::io::level() >= vivacity_core::io::Verbosity::Verbose {
        return crate::delegate_resolution(
            &[vivacity_core::scope::ScopeIssue::Layout(
                "-v without PHP: Composer's `Analyzed N packages` and `Analyzed N rules` count the platform packages, which only a PHP can list".to_owned(),
            )],
            no_fallback,
        )
        .map(Decision::Stop);
    }
    Ok(Decision::Without)
}

/// `run_once`, before the hand-overs it makes ahead of every command
/// (`-vv`, `-v` with plugins, the cache directory, a root version outside
/// git): a command that will resolve without PHP is known now, so that
/// they stop instead of launching a Composer with another PHP.
pub(crate) fn decide_early(resolves: bool, ignore_all: bool) {
    if resolves && ignore_all && !vivacity_resolver::platform::php_launchable() {
        ABSENT.store(true, Ordering::Release);
    }
}

/// The line that ends a delegation in this mode, in place of the hand-over.
pub(crate) fn stop_line() -> String {
    format!(
        "No PHP to run Composer with ({} cannot be started, see VIVACITY_PHP): stopping here.",
        vivacity_resolver::platform::php_command()
    )
}

/// What a `require` without a version constraint cannot write without PHP:
/// the constraint and the name Composer read from the platform repository
/// (`PackageDiscoveryTrait::findBestVersionAndNameForPackage`), and the
/// `ext-*` rule of `VersionSelector::findRecommendedRequireVersion`, which
/// compares with `PHP_VERSION` any name starting with `ext-`. The three
/// `composer*` packages are the same with or without PHP.
pub(crate) fn unguessable_requirement(name: &str) -> Option<vivacity_core::scope::ScopeIssue> {
    let lower = name.to_lowercase();
    let platform = vivacity_resolver::platform::is_platform_package(name)
        && !vivacity_resolver::platform::known_without_php(name, &serde_json::Map::new());
    (platform || lower.starts_with("ext-")).then(|| {
        vivacity_core::scope::ScopeIssue::Layout(format!(
            "{name} without a version constraint: Composer reads its name and constraint from the platform repository and the PHP version, which only a PHP can report"
        ))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn unguessable_requirements_are_the_platform_and_ext_prefix() {
        for name in [
            "ext-foo",
            "EXT-Intl",
            "php",
            "php-64bit",
            "lib-icu",
            "hhvm",
            "ext-vendor/pkg",
        ] {
            assert!(super::unguessable_requirement(name).is_some(), "{name}");
        }
        for name in [
            "composer",
            "composer-plugin-api",
            "composer-runtime-api",
            "psr/log",
            "php-http/discovery",
        ] {
            assert!(super::unguessable_requirement(name).is_none(), "{name}");
        }
    }
}
