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
| [composer/xdebug-handler](https://github.com/composer/xdebug-handler) | MIT | Composer | `docs/reference/resolver/xdebug-handler-*.php` (`LICENSE.composer-xdebug-handler`) | `vivacity-resolver` (`platform::ini_files`) |
| [symfony/filesystem](https://github.com/symfony/filesystem), [symfony/finder](https://github.com/symfony/finder) | MIT | Fabien Potencier | `docs/reference/symfony-Filesystem.php`, `docs/reference/symfony-finder-Glob.php` (`LICENSE.symfony`) | `vivacity-core` (`path_install`: the mirror of a `path` package, `Glob::toRegex` of the `.gitattributes` patterns) |
| [pestphp/pest-plugin](https://github.com/pestphp/pest-plugin) v5.0.0 | MIT | Nuno Maduro | `docs/reference/plugins/pest-plugin/` (`LICENSE.md`; `Manager.php`, `DumpCommand.php` — not in the phar, no drift twin) | `vivacity-core` (`pest_plugin`: `vendor/pest-plugins.json` at autoload-dump time) |
| [PHPCSStandards/composer-installer](https://github.com/PHPCSStandards/composer-installer) v1.2.1 (`dealerdirect/phpcodesniffer-composer-installer`) | MIT | Franck Nijhof, Juliette Reinders Folmer | `docs/reference/plugins/phpcodesniffer-composer-installer/` (`LICENSE`; `Plugin.php` — not in the phar, no drift twin) | `vivacity-core` (`phpcs_installer`: `installed_paths` of PHP_CodeSniffer, `CodeSniffer.conf` in its var_export format) |
| [phpstan/extension-installer](https://github.com/phpstan/extension-installer) 1.4.3 | MIT | Ondřej Mirtes | `docs/reference/plugins/phpstan-extension-installer/` (`LICENSE`; `Plugin.php` — no drift twin) | `vivacity` (`extension_installers`: `src/GeneratedConfig.php`) |
| [rector/extension-installer](https://github.com/rectorphp/extension-installer) 0.11.2 | MIT | Tomas Votruba | `docs/reference/plugins/rector-extension-installer/` (`LICENSE`; `Plugin.php`, `PluginInstaller.php` — no drift twin) | `vivacity` (`extension_installers`: `src/GeneratedConfig.php`) |
| [yiisoft/yii2-composer](https://github.com/yiisoft/yii2-composer) 2.0.11 | BSD-3-Clause | Yii Software LLC | `docs/reference/plugins/yii2-composer/` (`LICENSE.md`; `Plugin.php`, `Installer.php` — no drift twin) | `vivacity-core` (`yii2_composer`: `vendor/yiisoft/extensions.php` at install and dump time) |
| [codeception/c3](https://github.com/Codeception/c3) 2.9.0 | MIT (composer.json; no LICENSE file at that commit — `LICENSE-NOTE.md`) | Codeception | `docs/reference/plugins/codeception-c3/` (`Installer.php` — no drift twin) | `vivacity-core` (`c3_plugin`: `c3.php` at the project root after install) |
| [bamarni/composer-bin-plugin](https://github.com/bamarni/composer-bin-plugin) 1.9.1 | MIT | Bilal Amarni | `docs/reference/plugins/composer-bin-plugin/` (`LICENSE`; `BamarniBinPlugin.php`, `Config.php`, `Logger.php` — no drift twin) | `vivacity-core` (`bamarni_bin`: the deprecation lines, the forwarding refused) |
| [roots/wordpress-core-installer](https://github.com/roots/wordpress-core-installer) v4.0.0 | MIT | Roots | `docs/reference/plugins/wordpress-core-installer/` (`LICENSE.md`; `WordPressCoreInstaller.php`, `WordPressCorePlugin.php` — no drift twin) | `vivacity-core` (`layout::wp_core_dir`) |
| [mnsami/composer-custom-directory-installer](https://github.com/mnsami/composer-custom-directory-installer) 2.0.0 | MIT | Mina Nabil Sami | `docs/reference/plugins/composer-custom-directory-installer/` (`LICENSE`; `PackageUtils.php`, `LibraryInstaller.php`, `LibraryPlugin.php`, `PluginInstaller.php`, `PluginPlugin.php` — no drift twin) | `vivacity-core` (`layout::custom_dir`) |
| [wikimedia/composer-merge-plugin](https://github.com/wikimedia/composer-merge-plugin) v2.1.0 | MIT | Bryan Davis, Wikimedia Foundation, and contributors | `docs/reference/plugins/composer-merge-plugin/` (`LICENSE`; `MergePlugin.php`, `ExtraPackage.php`, `PluginState.php`, `NestedArray.php`, `StabilityFlags.php`, `MultiConstraint.php`, `MissingFileException.php` — no drift twin) | `vivacity-resolver` (`merge_plugin`), `vivacity-core` (`glob::glob_plain`, `phparray`) |
| [PCRE2](https://github.com/PCRE2Project/pcre2) 10.46 (with sljit) | BSD-3-Clause WITH PCRE2-exception; sljit BSD-2-Clause | University of Cambridge (Philip Hazel, Zoltán Herczeg); Zoltán Herczeg | `crates/vivacity-pcre2-sys/upstream/` (`LICENCE-pcre2`) — compiled into every vivacity binary, symbols prefixed `vivacity_` (`PCRE2_SYMBOL_PREFIX`, a 16-line patch of `pcre2.h` / `pcre2_internal.h`) | `vivacity-pcre2-sys` |
| [rust-pcre2](https://github.com/BurntSushi/rust-pcre2) (`pcre2-sys` 0.2.10, `pcre2` 0.2.11) | Unlicense OR MIT | Andrew Gallant | `crates/vivacity-pcre2-sys/` (build script: always the vendored source, prefixed; bindings: `#[link_name]`), `crates/vivacity-pcre2/` (unchanged) (`COPYING`, `LICENSE-MIT`, `UNLICENSE`) | `vivacity-pcre2-sys`, `vivacity-pcre2` |

The PHP sources under `docs/reference/` are not part of the compiled
crates; they are kept so that `harness/drift-reference.sh` can re-diff
them against the phar and name the port that must be re-read when
upstream moves.

Nothing is ported from GPL-licensed software. Releases 0.3.0 to 0.5.0
shipped an emulation of `drupal/core-composer-scaffold` (GPL-2.0-or-later)
ported from its source; it was removed in 0.6.0 for that reason, and the
`vivace*` crates were deleted from crates.io (only 0.5.0 had been published).
