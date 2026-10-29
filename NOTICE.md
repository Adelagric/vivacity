# Notices

vivacity is distributed under the MIT or Apache-2.0 license, at your option
(see LICENSE-MIT and LICENSE-APACHE). It reimplements the behaviour of
Composer, and parts of it are ports — function by function — of the
following software. Their source files are vendored in `docs/reference/`
as the reference each port is checked against (the copies come from the
Composer 2.10.3 phar, which strips comment headers; the license texts are
kept next to them). The copyright notices below apply to those files and
to the Rust code ported from them.

| Origin | License | Copyright | Vendored copy | Ported in |
|---|---|---|---|---|
| [Composer](https://github.com/composer/composer) 2.10.3 (`src/Composer/`) | MIT | Nils Adermann, Jordi Boggiano | `docs/reference/*.php`, `docs/reference/resolver/`, `docs/reference/policy/` (`LICENSE.composer`) | `vivacity-core`, `vivacity-resolver`, `vivacity-autoload`, `vivacity` |
| [composer/semver](https://github.com/composer/semver) | MIT | Composer | `docs/reference/resolver/semver-*.php` (`LICENSE.composer-semver`) | `vivacity-resolver` (`version`, `constraint`, `intervals`, `phpver`) |
| [composer/class-map-generator](https://github.com/composer/class-map-generator) | MIT | Composer | `docs/reference/cmg-*.php` (`LICENSE.composer-class-map-generator`) | `vivacity-autoload` |
| [composer/metadata-minifier](https://github.com/composer/metadata-minifier) | MIT | Composer | `docs/reference/resolver/MetadataMinifier.php` (`LICENSE.composer-metadata-minifier`) | `vivacity-resolver` (`loader`) |
| [composer/installers](https://github.com/composer/installers) 2.0.0–2.3.0 | MIT | Kyle Robinson Young | `docs/reference/installers/` (`LICENSE`) | `vivacity-core` (`installers`, tables in `assets/installers/`) |
| [composer/installers](https://github.com/composer/installers) v1.12.0 | MIT | Kyle Robinson Young | `docs/reference/installers-v1.12.0/` (`LICENSE`; `Installer.php`, `BaseInstaller.php` — no drift twin) | `vivacity-core` (`installers`, `assets/installers/v1.12.0.json`), `vivacity` (`plugin_allow::installers_deprecation`) |
| [getkirby/composer-installer](https://github.com/getkirby/composer-installer) 1.2.1 | MIT (composer.json; no LICENSE file at that tag — `LICENSE-NOTE.md`) | Kirby Team | `docs/reference/plugins/kirby-composer-installer/` (`Plugin.php`, `Installer.php`, `CmsInstaller.php`, `PluginInstaller.php` — no drift twin) | `vivacity-core` (`custom_installers`: the kirby placer, `postInstall`) |
| [neos/composer-plugin](https://github.com/neos/composer-plugin) 2.1.3 | MIT | Neos project contributors | `docs/reference/plugins/neos-composer-plugin/` (`LICENSE`; `Installer.php` — no drift twin) | `vivacity-core` (`custom_installers`: the neos placer) |
| [davidbarratt/custom-installer](https://github.com/davidbarratt/custom-installer) 1.1.0 | Apache-2.0 | David Barratt | `docs/reference/plugins/davidbarratt-custom-installer/` (`LICENSE`; `CustomInstaller.php`, `Configuration.php`, `ConfigurationAlpha1.php` — no drift twin) | `vivacity-core` (`custom_installers`: the davidbarratt placer) |
| [oomphinc/composer-installers-extender](https://github.com/oomphinc/composer-installers-extender) 2.0.1 | MIT | Oomph, Inc. | `docs/reference/plugins/composer-installers-extender/` (`LICENSE`; `Plugin.php`, `Installers/Installer.php`, `Installers/CustomInstaller.php` — no drift twin) | `vivacity-core` (`custom_installers`: the oomphinc placer) |
| [composer/xdebug-handler](https://github.com/composer/xdebug-handler) | MIT | Composer | `docs/reference/resolver/xdebug-handler-*.php` (`LICENSE.composer-xdebug-handler`) | `vivacity-resolver` (`platform::ini_files`) |
| [symfony/filesystem](https://github.com/symfony/filesystem), [symfony/finder](https://github.com/symfony/finder) | MIT | Fabien Potencier | `docs/reference/symfony-Filesystem.php`, `docs/reference/symfony-finder-Glob.php` (`LICENSE.symfony`) | `vivacity-core` (`path_install`: the mirror of a `path` package, `Glob::toRegex` of the `.gitattributes` patterns) |
| [pestphp/pest-plugin](https://github.com/pestphp/pest-plugin) v5.0.0 | MIT | Nuno Maduro | `docs/reference/plugins/pest-plugin/` (`LICENSE.md`; `Manager.php`, `DumpCommand.php` — not in the phar, no drift twin) | `vivacity-core` (`pest_plugin`: `vendor/pest-plugins.json` at autoload-dump time) |
| [PHPCSStandards/composer-installer](https://github.com/PHPCSStandards/composer-installer) v1.2.1 (`dealerdirect/phpcodesniffer-composer-installer`) | MIT | Franck Nijhof, Juliette Reinders Folmer | `docs/reference/plugins/phpcodesniffer-composer-installer/` (`LICENSE`; `Plugin.php` — not in the phar, no drift twin) | `vivacity-core` (`phpcs_installer`: `installed_paths` of PHP_CodeSniffer, `CodeSniffer.conf` in its var_export format) |
| [PHPCSStandards/composer-installer](https://github.com/PHPCSStandards/composer-installer) v0.7.2 (`1c968e5`, `dealerdirect/phpcodesniffer-composer-installer`) | MIT | Dealerdirect B.V. | `docs/reference/plugins/phpcodesniffer-composer-installer-v0.7.2/` (`LICENSE.md`; `src/Plugin.php` as `Plugin.php` — not in the phar, no drift twin) | `vivacity-core` (`phpcs_installer`: the search of the whole project, `phpcs_installer::variant`); the walk follows symfony/finder v5.4.45 as the Composer 2.10.3 phar bundles it (`Finder::searchInDirectory`, `RecursiveDirectoryIterator`, read, not copied) |
| [phpstan/extension-installer](https://github.com/phpstan/extension-installer) 1.4.3 | MIT | Ondřej Mirtes | `docs/reference/plugins/phpstan-extension-installer/` (`LICENSE`; `Plugin.php` — no drift twin) | `vivacity` (`extension_installers`: `src/GeneratedConfig.php`) |
| [rector/extension-installer](https://github.com/rectorphp/extension-installer) 0.11.2 | MIT | Tomas Votruba | `docs/reference/plugins/rector-extension-installer/` (`LICENSE`; `Plugin.php`, `PluginInstaller.php` — no drift twin) | `vivacity` (`extension_installers`: `src/GeneratedConfig.php`) |
| [yiisoft/yii2-composer](https://github.com/yiisoft/yii2-composer) 2.0.11 | BSD-3-Clause | Yii Software LLC | `docs/reference/plugins/yii2-composer/` (`LICENSE.md`; `Plugin.php`, `Installer.php` — no drift twin) | `vivacity-core` (`yii2_composer`: `vendor/yiisoft/extensions.php` at install and dump time) |
| [codeception/c3](https://github.com/Codeception/c3) 2.9.0 | MIT (composer.json; no LICENSE file at that commit — `LICENSE-NOTE.md`) | Codeception | `docs/reference/plugins/codeception-c3/` (`Installer.php` — no drift twin) | `vivacity-core` (`c3_plugin`: `c3.php` at the project root after install) |
| [bamarni/composer-bin-plugin](https://github.com/bamarni/composer-bin-plugin) 1.9.1 | MIT | Bilal Amarni | `docs/reference/plugins/composer-bin-plugin/` (`LICENSE`; `BamarniBinPlugin.php`, `Config.php`, `Logger.php` — no drift twin) | `vivacity-core` (`bamarni_bin`: the deprecation lines, the forwarding refused) |
| [mnsami/composer-custom-directory-installer](https://github.com/mnsami/composer-custom-directory-installer) 2.0.0 | MIT | Mina Nabil Sami | `docs/reference/plugins/composer-custom-directory-installer/` (`LICENSE`; `PackageUtils.php`, `LibraryInstaller.php`, `LibraryPlugin.php`, `PluginInstaller.php`, `PluginPlugin.php` — no drift twin) | `vivacity-core` (`layout::custom_dir`) |
| [composer/package-versions-deprecated](https://github.com/composer/package-versions-deprecated) 1.11.99.5 | MIT | Marco Pivetta, Jordi Boggiano | `docs/reference/plugins/package-versions-deprecated/` (`LICENSE`; `Installer.php` — no drift twin) and its generated-class template, verbatim, in `crates/vivacity-core/assets/package-versions-template.php` | `vivacity-core` (`package_versions`) |
| [wikimedia/composer-merge-plugin](https://github.com/wikimedia/composer-merge-plugin) v2.1.0 | MIT | Bryan Davis, Wikimedia Foundation, and contributors | `docs/reference/plugins/composer-merge-plugin/` (`LICENSE`; `MergePlugin.php`, `ExtraPackage.php`, `PluginState.php`, `NestedArray.php`, `StabilityFlags.php`, `MultiConstraint.php`, `MissingFileException.php` — no drift twin) | `vivacity-resolver` (`merge_plugin`), `vivacity-core` (`glob::glob_plain`, `phparray`) |
| [dg/composer-cleaner](https://github.com/dg/composer-cleaner) v2.2.2 | BSD-3-Clause (composer.json; no licence file at that tag — `LICENSE-NOTE.md`) | David Grudl | `docs/reference/plugins/composer-cleaner/` (`Plugin.php`, `Cleaner.php` — no drift twin) | `vivacity-core` (`composer_cleaner`: `Cleaner::clean`, PHP's `fnmatch`) |
| [liborm85/composer-vendor-cleaner](https://github.com/liborm85/composer-vendor-cleaner) 1.7.1, with its copy of Symfony's `Finder\Glob` | MIT | Libor M.; Fabien Potencier (`Finder/Glob.php`) | `docs/reference/plugins/composer-vendor-cleaner/` (`LICENSE`, `Finder/LICENSE`; `Plugin.php`, `Cleaner.php`, `DevFilesFinder.php`, `GlobFilter.php`, `Directory.php`, `Filesystem.php`, `Package.php`, `Finder/Glob.php` — no drift twin) | `vivacity-core` (`vendor_cleaner`; `path_install::glob_to_regex_with`) |
| [cakephp/plugin-installer](https://github.com/cakephp/plugin-installer) 2.0.2 | MIT | Cake Software Foundation, Inc. | `docs/reference/plugins/cakephp-plugin-installer/` (`LICENSE.txt`; `Plugin.php` — no drift twin) | `vivacity-core` (`cakephp_plugins`: the root autoload, `cakephp-plugins.php`) |
| [drupol/composer-packages](https://github.com/drupol/composer-packages) 2.0.0 | MIT | Pol Dellaiera | `docs/reference/plugins/composer-packages/` (`LICENSE`; `composer.json`, `src/` with its five Twig templates — no drift twin) and the templates, verbatim, with the licence, in `crates/vivacity-core/assets/composer-packages/` | `vivacity-core` (`composer_packages`: the exporters, `Name::camelize`, the templates rendered by a port of the Twig subset they use), `vivacity` (`effect_plugins::composer_packages`) |
| [infection/extension-installer](https://github.com/infection/extension-installer) 0.1.2 | BSD-3-Clause | Maks Rafalko | `docs/reference/plugins/infection-extension-installer/` (`LICENSE`; `Plugin.php` — no drift twin) | `vivacity` (`extension_installers::infection`: `src/GeneratedExtensionsConfig.php` and its lines) |
| [typo3/class-alias-loader](https://github.com/TYPO3/class-alias-loader) v2.0.1 | MIT | Helmut Hummel | `docs/reference/plugins/class-alias-loader/` (`LICENSE`; `Plugin.php`, `ClassAliasMapGenerator.php`, `Config.php`, `IncludeFile.php`, `IncludeFile/*.php`, `alias-loader-include.tmpl.php` — no drift twin) | `vivacity-core` (`class_alias_loader`: the include file from its template, the empty alias map, the suffix), `vivacity` (`effect_plugins`) |
| [mlocati/composer-patcher](https://github.com/mlocati/composer-patcher) 1.2.6 | MIT | Michele Locati | `docs/reference/plugins/composer-patcher/` (`LICENSE`; `src/` — no drift twin) | `vivacity-core` (`composer_patcher`: the collection, the hash, the `patch` command lines and the reading of their output), `vivacity` (`effect_plugins::patcher_pre`, `patcher_post`) |
| [laminas/laminas-skeleton-installer](https://github.com/laminas/laminas-skeleton-installer) 1.3.0 | BSD-3-Clause | Laminas Project a Series of LF Projects, LLC. | `docs/reference/plugins/laminas-skeleton-installer/` (`LICENSE.md`, `COPYRIGHT.md`; `Plugin.php`, `OptionalPackagesInstaller.php`, `OptionalPackage.php`, `ComposerJsonRetrievalTrait.php`, `Collection.php`, `Uninstaller.php` — no drift twin) | `vivacity-core` (`effect_plugins`: the minimal install, composer.json rewritten) |
| [laminas/laminas-component-installer](https://github.com/laminas/laminas-component-installer) 3.8.0 | BSD-3-Clause | Laminas Project a Series of LF Projects, LLC. | `docs/reference/plugins/laminas-component-installer/` (`LICENSE.md`, `COPYRIGHT.md`; `ComponentInstaller.php`, `ConfigDiscovery.php`, `ConfigDiscovery/*.php` — no drift twin) | `vivacity-core` (`effect_plugins`: when it is inert) |
| [spiral/composer-publish-plugin](https://github.com/spiral/composer-publish-plugin) v1.1.2 | MIT | Spiral Scout | `docs/reference/plugins/spiral-composer-publish-plugin/` (`LICENSE`; `PublishPlugin.php` — no drift twin) | `vivacity-core` (`effect_plugins`: when it is inert) |
| [sculpin/sculpin-theme-composer-plugin](https://github.com/sculpin/sculpin-theme-composer-plugin) 1.0.3 | MIT | Dragonfly Development Inc. | `docs/reference/plugins/sculpin-theme-composer-plugin/` (`LICENSE`; `SculpinThemePlugin.php` — no drift twin) | `vivacity-core` (`effect_plugins`: when it is inert) |
| [symfony/flex](https://github.com/symfony/flex) 2.11.0 | MIT | Fabien Potencier | not vendored: the reference is the copy `fixtures/make.sh` installs (`fixtures/work/symfony/vendor/symfony/flex/`, the version the fixtures lock) | `vivacity` (`flex`, `flex_install`, `flex_recipes`: `Downloader::getRecipes`, `Flex::install`/`fetchRecipes`/`recordOperations`, `SymfonyBundle`, `Lock`, `Options`, `Path`, the `bundles`, `copy-from-recipe`, `env`, `dotenv`, `gitignore`, `add-lines` and `composer-scripts` configurators), `vivacity-resolver` (`flex_filter`: `PackageFilter`) |
| [PCRE2](https://github.com/PCRE2Project/pcre2) 10.46 (with sljit) | BSD-3-Clause WITH PCRE2-exception; sljit BSD-2-Clause | University of Cambridge (Philip Hazel, Zoltán Herczeg); Zoltán Herczeg | `crates/vivacity-pcre2-sys/upstream/` (`LICENCE-pcre2`) — compiled into every vivacity binary, symbols prefixed `vivacity_` (`PCRE2_SYMBOL_PREFIX`, a 16-line patch of `pcre2.h` / `pcre2_internal.h`) | `vivacity-pcre2-sys` |
| [rust-pcre2](https://github.com/BurntSushi/rust-pcre2) (`pcre2-sys` 0.2.10, `pcre2` 0.2.11) | Unlicense OR MIT | Andrew Gallant | `crates/vivacity-pcre2-sys/` (build script: always the vendored source, prefixed; bindings: `#[link_name]`), `crates/vivacity-pcre2/` (unchanged) (`COPYING`, `LICENSE-MIT`, `UNLICENSE`) | `vivacity-pcre2-sys`, `vivacity-pcre2` |

The PHP sources under `docs/reference/` are not part of the compiled
crates; they are kept so that `harness/drift-reference.sh` can re-diff
them against the phar and name the port that must be re-read when
upstream moves.

Nothing is ported from GPL-licensed software. The behaviour of
roots/wordpress-core-installer and johnpbloch/wordpress-core-installer
(GPL-2.0-or-later), thelia/installer
and contao-components/installer (LGPL-3.0-or-later) is reimplemented from
what they do, measured against Composer; none of their source is copied or
vendored. typo3/cms-composer-installers (GPL-2.0-or-later) and
contao/manager-plugin (LGPL-3.0-or-later) were read to classify them
(docs/plans/plugins-round2.md); nothing of them is copied either.
contao/manager-plugin is not emulated. typo3/cms-composer-installers is
reproduced only where it does nothing — under a root package named
`typo3/cms` —, a rule taken from Composer runs with the plugin allowed and
refused (docs/plans/remaining-fallbacks.md), its source left unopened in
that work. Until 2026-10-28 this file listed roots/wordpress-core-installer
as MIT, with a copy of its source under `docs/reference/`; it is
GPL-2.0-or-later, so that copy was removed and `layout::wp_core_dir`
rewritten from the plugin's measured behaviour (DECISIONS 2026-10-28).
Releases 0.3.0 to 0.5.0
shipped an emulation of `drupal/core-composer-scaffold` (GPL-2.0-or-later)
ported from its source; it was removed in 0.6.0 for that reason, and the
`vivace*` crates were deleted from crates.io (only 0.5.0 had been published).

The corpus (`fixtures/corpus/`) holds the manifests and locks of real
projects, some of them GPL. Since 2026-10-29 the PHP files of chamilo-lms
(GPL-3.0-or-later) that its root autoload loads are not committed: their
paths and SHA-256 sums are (`fixtures/corpus/chamilo-lms/UPSTREAM`), and
`harness/corpus.sh` downloads them at the pinned commit when it measures
(`public/legacy.php`, previously committed, was removed from the tree that
day; it remains in the history). Other entries still carry a few `files`
autoload scripts copied verbatim, under their projects' licences
(`fixtures/corpus/README.md`).
