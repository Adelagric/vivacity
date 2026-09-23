//! The out-of-scope detector on the fixtures: native where no plugin
//! writes anything under a Composer with plugins active, handed over where
//! one does (the corpus baseline), the benign plugins reported, no more, no
//! less.

use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/work")
        .join(name);
    assert!(
        dir.is_dir(),
        "fixture {name} missing — run fixtures/make.sh"
    );
    dir
}

fn analyze(name: &str) -> vivacity_core::scope::ScopeReport {
    let dir = fixture(name);
    let lock = vivacity_core::lock::Lock::read(&dir.join("composer.lock")).expect("lock");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("composer.json")).expect("composer.json"),
    )
    .expect("manifest");
    vivacity_core::scope::analyze(&dir, &lock, &manifest, true, true)
}

#[test]
fn all_fixtures_are_native() {
    // No plugin, or plugins emulated or proven to write nothing under a
    // Composer with plugins active (the corpus baseline).
    for name in ["laravel", "symfony", "sylius", "rector", "wordpress"] {
        let report = analyze(name);
        assert!(
            report.is_native_ok(),
            "fixture {name} out of scope: {:?}",
            report.issues
        );
    }
}

#[test]
fn expected_benign_plugins_are_reported() {
    assert!(analyze("laravel").skipped_plugins.is_empty());
    // `symfony/flex` and `composer/package-versions-deprecated` were
    // counted here until they moved to the emulated list (both write at
    // install time); `php-http/discovery` is the one left on sylius.
    assert!(analyze("symfony").skipped_plugins.is_empty());
    assert_eq!(
        analyze("sylius").skipped_plugins,
        vec!["php-http/discovery"]
    );
}
