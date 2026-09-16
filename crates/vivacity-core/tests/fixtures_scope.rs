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
fn fixtures_native_or_handed_over_as_documented() {
    // Native: no plugin, or only plugins proven to write nothing under a
    // Composer with plugins active (the corpus baseline).
    for name in ["laravel", "symfony", "wordpress"] {
        let report = analyze(name);
        assert!(
            report.is_native_ok(),
            "fixture {name} out of scope: {:?}",
            report.issues
        );
    }
    // Handed to Composer: the extension installers write GeneratedConfig.php
    // (docs/corpus/2026-09-16.md); rector locks both, sylius allows rector's.
    for (name, plugin) in [
        ("rector", "phpstan/extension-installer"),
        ("sylius", "rector/extension-installer"),
    ] {
        let report = analyze(name);
        assert!(
            report.issues.iter().any(
                |i| matches!(i, vivacity_core::scope::ScopeIssue::UnknownPlugin(p) if p == plugin)
            ),
            "fixture {name}: expected {plugin} to be an unknown plugin, got {:?}",
            report.issues
        );
    }
}

#[test]
fn expected_benign_plugins_are_reported() {
    assert!(analyze("laravel").skipped_plugins.is_empty());
    assert_eq!(analyze("symfony").skipped_plugins, vec!["symfony/flex"]);
    let sylius = analyze("sylius").skipped_plugins;
    assert!(sylius.contains(&"symfony/flex".to_owned()));
    assert!(sylius.contains(&"php-http/discovery".to_owned()));
    assert_eq!(sylius.len(), 3, "unexpected benign list: {sylius:?}");
}
