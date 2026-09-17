# wikimedia/composer-merge-plugin v2.1.0 — reference copy

`src/` of the v2.1.0 tag (MIT, `LICENSE`), ported in
`crates/vivacity-resolver/src/merge_plugin.rs` (with
`vivacity-core::glob::glob_plain` and `vivacity-core::phparray`). The
v2.0.1 the corpus also holds differs only by `array()` syntax, `static`
closures and the lock backup of the implicit update. No drift twin: the
plugin is not part of the Composer phar.

Not ported, on purpose: `onPostPackageInstall` / `onPostInstallOrUpdate`
(the implicit partial `composer update` on the run that installs the
plugin — DECISIONS, 2026-09-17), `mergeScripts`, `mergeSuggests`,
`mergeAliases` / `mergeReferences`, `prependRepositories`,
`StabilityFlags` (resolution only), the Composer 1 branches.
