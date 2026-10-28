# vivacity with Deployer

Deployer's `deploy:vendors` task runs
`{{bin/composer}} {{composer_action}} {{composer_options}}` in every new
release directory. Each deploy therefore installs into an empty `vendor/` on
a server that already holds the previous releases: once vivacity's store is
warm, a release's `vendor/` is laid out by hardlinking (or reflinking) files
from the store instead of extracting archives.

| warm install into an empty `vendor/`, Laravel 13 skeleton, 77 packages | Composer | vivacity |
|---|---|---|
| Apple M4 Max, APFS, PHP 8.5.12, 6 runs | 1 790–1 850 ms | 230–260 ms |

Measured on 2026-10-28 with Deployer's own options (`--no-scripts` added
to time the install alone). A deploy adds SSH, `artisan` and your other
tasks around it: measure yours.

## Setup

Install vivacity on each target host (the install script checks the
sha256 of the release asset; pin the version as you pin Composer):

```bash
curl -fsSL https://raw.githubusercontent.com/Adelagric/vivacity/main/install.sh \
  | VIVACITY_VERSION=v0.25.0 sh
```

It lands in `~/.local/bin` (`VIVACITY_INSTALL_DIR` to change it); give
Deployer the absolute path, as below for a `deploy` user. Keep `composer`
installed too: vivacity hands it what it does not reproduce, and runs
`composer.json`'s scripts through it.

Then in `deploy.php`, after the recipe:

```php
set('bin/composer', '/home/deploy/.local/bin/vivacity');
set('composer_options', '--prefer-dist --no-progress --no-interaction --no-dev --optimize-autoloader --run-scripts');
```

- **`--run-scripts`.** vivacity never runs `composer.json` scripts on its
  own; with this flag it runs them through `composer run-script` at
  Composer's points (Laravel's `package:discover`, Symfony's
  `auto-scripts`). Without it, a deploy that relies on them breaks.
- **`--verbose` left out.** Deployer's default options pass it. Under `-v`,
  vivacity stays native with the plugins whose verbose lines it knows (a
  standard Laravel project's among them) and hands the install to Composer
  with any other (symfony/flex, dealerdirect's PHP_CodeSniffer
  installer…), which then gains nothing.
- **`dep -v`, `-vv`, `-vvv`.** Deployer exports its verbosity to the
  commands it runs (`SHELL_VERBOSITY`), as Composer reads it: `dep deploy
  -v` makes the install verbose as `--verbose` would, and under `-vv` or
  `-vvv` vivacity hands the install to Composer (it does not reproduce
  Composer's debug output). The deploy still succeeds, at Composer's speed.
- **`composer_version`** only steers Deployer's own lookup of Composer; it
  has no effect once `bin/composer` is set.
- **The store** is `~/.cache/vivacity` (`VIVACITY_CACHE_DIR` to change it).
  Hardlinks need the store and the releases on the same filesystem; across
  filesystems vivacity copies (same result, smaller gain), so keep the
  deploy user's home and `deploy_path` on one volume, or point
  `VIVACITY_CACHE_DIR` next to `deploy_path`.

## What `bin/composer` covers

Setting `bin/composer` replaces the binary for every task that uses it:

- `deploy:vendors` (`install`) and the Magento 2 recipe's
  `dump-autoload -o` are vivacity commands;
- the Symfony recipe's optional `deploy:dump-env` runs `composer dump-env`,
  a symfony/flex command vivacity rejects (exit 2). If you use it, leave
  `bin/composer` alone and override only the install task, for example:

```php
task('deploy:vendors', function () {
    run('cd {{release_or_current_path}} && /home/deploy/.local/bin/vivacity install {{composer_options}} 2>&1');
});
```

A lock vivacity does not lay out natively (an unported plugin, a `git`-only
package, the Drupal scaffold…) goes to the `composer` on the host, with the
reason on stderr, before anything is written: the deploy still succeeds, at
Composer's speed. `vivacity install --check-scope` on a checkout tells you
in advance (no network, nothing written; exit 0 means native).

## Checked

Run with Deployer 7.5.12 and its `recipe/laravel.php` against `localhost()`
(`writable_mode` set to `chmod`, a shared `.env` on SQLite), deploying a
Laravel 13 skeleton from a local git repository twice with Composer and
twice with vivacity and the settings above: all four deploys succeed
(`artisan` migrate and caches included), the `vendor/` of every release is
identical to Composer's, and the install's output differs only by the
duration on the `Dependency resolution completed` line and vivacity's
summary line. Not run over SSH against a remote host.
