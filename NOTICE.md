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

The PHP sources under `docs/reference/` are not part of the compiled
crates; they are kept so that `harness/drift-reference.sh` can re-diff
them against the phar and name the port that must be re-read when
upstream moves.

Nothing is ported from GPL-licensed software. Releases 0.3.0 to 0.5.0
shipped an emulation of `drupal/core-composer-scaffold` (GPL-2.0-or-later)
ported from its source; it was removed in 0.6.0 for that reason, and the
`vivace*` crates were deleted from crates.io (only 0.5.0 had been published).
