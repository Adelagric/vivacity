//! `update`, `require`, `remove` with no PHP to probe (docs/plans/update-without-php.md):
//! under `--ignore-platform-reqs` they resolve on the platform `install`
//! already uses without PHP; without it they stop with exit 1 and say
//! how to go on; what depends on the real platform stops with exit 3 and
//! is named — never handed to a Composer that would need a PHP too.
//!
//! The binary runs in a child process: `VIVACITY_PHP`, `PATH` and
//! `COMPOSER_HOME` are its own. Packages come from a `path` repository,
//! Packagist is off: no network.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Project {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    bin: PathBuf,
}

/// A project requiring `acme/base`, whose requirements no PHP satisfies,
/// next to the other packages a case may require.
fn project(require: &str) -> Project {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().join("app");
    let pkgs = root.join("pkgs");
    let write = |dir: &Path, json: &str| {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join("composer.json"), json).expect("write");
    };
    write(
        &pkgs.join("base"),
        r#"{"name": "acme/base", "version": "1.0.0", "require": {"php": ">=99", "ext-nope": "*", "lib-nope": "^1"}, "conflict": {"ext-json": "*"}}"#,
    );
    write(
        &pkgs.join("replacer"),
        r#"{"name": "acme/replacer", "version": "1.0.0", "replace": {"ext-foo": "*"}}"#,
    );
    write(
        &root,
        &format!(
            r#"{{"name": "acme/app", "repositories": [{{"type": "path", "url": "pkgs/*"}}], "require": {require}}}"#
        ),
    );
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(
        home.join("config.json"),
        r#"{"repositories": {"packagist.org": false}}"#,
    )
    .expect("config");
    // A `composer` that must never run: a hand-over without PHP cannot work.
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let fake = bin.join("composer");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ntouch '{}'\nexit 0\n",
            tmp.path().join("ran").display()
        ),
    )
    .expect("fake composer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    Project {
        _tmp: tmp,
        root,
        home,
        bin,
    }
}

impl Project {
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_vivacity"))
            .args(args)
            .current_dir(&self.root)
            .env("VIVACITY_PHP", self.root.join("no-such-php"))
            .env("VIVACITY_NO_PLATFORM_CACHE", "1")
            .env("COMPOSER_HOME", &self.home)
            .env("COMPOSER_CACHE_DIR", self.home.join("cache"))
            .env("COMPOSER_ROOT_VERSION", "1.0.0")
            .env(
                "GIT_CEILING_DIRECTORIES",
                self.root.parent().expect("parent"),
            )
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env_remove("COMPOSER_IGNORE_PLATFORM_REQS")
            .env_remove("COMPOSER_IGNORE_PLATFORM_REQ")
            .output()
            .expect("run vivacity");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn manifest(&self) -> String {
        std::fs::read_to_string(self.root.join("composer.json")).expect("composer.json")
    }

    fn composer_ran(&self) -> bool {
        self._tmp.path().join("ran").exists()
    }
}

#[test]
#[cfg(unix)]
fn update_resolves_without_php_under_ignore_platform_reqs() {
    let p = project(r#"{"acme/base": "*"}"#);
    let (code, err) = p.run(&["update", "--no-install", "--ignore-platform-reqs"]);
    assert_eq!(code, 0, "{err}");
    let lock = std::fs::read_to_string(p.root.join("composer.lock")).expect("lock");
    assert!(lock.contains("\"name\": \"acme/base\""), "{lock}");
    assert!(lock.contains("\"ext-nope\": \"*\""), "{lock}");
}

#[test]
#[cfg(unix)]
fn update_without_php_and_without_the_flag_says_how_to_go_on() {
    for extra in [
        &[][..],
        &["--ignore-platform-req=ext-nope"],
        &["--ignore-platform-req=*"],
    ] {
        let p = project(r#"{"acme/base": "*"}"#);
        let mut args = vec!["update", "--no-install"];
        args.extend(extra);
        let (code, err) = p.run(&args);
        assert_eq!(code, 1, "{args:?}: {err}");
        assert!(err.contains("--ignore-platform-reqs"), "{err}");
        assert!(err.contains("VIVACITY_PHP"), "{err}");
        assert!(!p.root.join("composer.lock").exists());
    }
}

#[test]
#[cfg(unix)]
fn remove_without_php_and_without_the_flag_edits_nothing() {
    let p = project(r#"{"acme/base": "*"}"#);
    let before = p.manifest();
    let (code, err) = p.run(&["remove", "acme/base", "--no-install"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("VIVACITY_PHP"), "{err}");
    assert_eq!(p.manifest(), before);
}

#[test]
#[cfg(unix)]
fn require_of_a_platform_package_without_constraint_stops_named() {
    for name in ["ext-foo", "php", "lib-curl", "ext-vendor/package"] {
        let p = project("{}");
        let before = p.manifest();
        let (code, err) = p.run(&["require", name, "--no-install", "--ignore-platform-reqs"]);
        assert_eq!(code, 3, "{name}: {err}");
        assert!(err.contains(name), "{name}: {err}");
        assert!(err.contains("VIVACITY_PHP"), "{name}: {err}");
        assert_eq!(p.manifest(), before, "{name}");
        assert!(!p.composer_ran(), "{name}: composer was run");
    }
    // The composer* packages are the same with or without PHP.
    let p = project("{}");
    let (code, err) = p.run(&[
        "require",
        "composer-plugin-api",
        "--no-install",
        "--ignore-platform-reqs",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(p.manifest().contains("\"composer-plugin-api\": \"*\""));
}

#[test]
#[cfg(unix)]
fn a_package_replacing_a_platform_name_stops_named() {
    let p = project(r#"{"acme/replacer": "*"}"#);
    let (code, err) = p.run(&["update", "--no-install", "--ignore-platform-reqs"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("acme/replacer"), "{err}");
    assert!(err.contains("ext-foo"), "{err}");
    assert!(!p.root.join("composer.lock").exists());
    assert!(!p.composer_ran(), "composer was run");
}

#[test]
#[cfg(unix)]
fn a_temporary_constraint_on_a_platform_name_stops_named() {
    let p = project(r#"{"acme/base": "*", "ext-json": "*"}"#);
    let (code, err) = p.run(&[
        "update",
        "--no-install",
        "--ignore-platform-reqs",
        "--with",
        "ext-json:^99",
    ]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("ext-json"), "{err}");
    assert!(!p.root.join("composer.lock").exists());
}

#[test]
#[cfg(unix)]
fn verbose_output_without_php_stops_named() {
    let p = project(r#"{"acme/base": "*"}"#);
    let (code, err) = p.run(&["update", "--no-install", "--ignore-platform-reqs", "-v"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("VIVACITY_PHP"), "{err}");
    assert!(!p.root.join("composer.lock").exists());
    assert!(!p.composer_ran(), "composer was run");
}
