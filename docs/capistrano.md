# vivacity with Capistrano

[capistrano/composer](https://github.com/capistrano/composer) runs
`composer install` in every new release directory, before
`deploy:updated`. Each deploy therefore installs into an empty `vendor/` on
a server that already holds the previous releases: once vivacity's store is
warm, a release's `vendor/` is laid out by hardlinking (or reflinking) files
from the store instead of extracting archives.

| warm install into an empty `vendor/`, Linux | Composer | vivacity |
|---|---|---|
| Laravel | 1 700 ms | 138 ms |
| Symfony demo | 1 525 ms | 115 ms |
| Sylius | 3 522 ms | 659 ms |

From [bench/M6-linux-ci.md](../bench/M6-linux-ci.md), 2026-09-10 (method,
machine and the other scenarios there). A deploy adds SSH and your other tasks around
it: measure yours.

## Setup

Install vivacity on each target host (the install script checks the
sha256 of the release asset; pin the version as you pin Composer):

```bash
curl -fsSL https://raw.githubusercontent.com/Adelagric/vivacity/main/install.sh \
  | VIVACITY_VERSION=v0.26.0 sh
```

It lands in `~/.local/bin` (`VIVACITY_INSTALL_DIR` to change it); give
Capistrano the absolute path, as below for a `deploy` user. Keep
`composer` installed too: vivacity hands it what it does not reproduce, and
runs `composer.json`'s scripts through it.

Then in `deploy.rb`:

```ruby
SSHKit.config.command_map[:composer] = "/home/deploy/.local/bin/vivacity"
set :composer_install_flags, '--no-dev --prefer-dist --no-interaction --quiet --optimize-autoloader --run-scripts'
```

- **`--run-scripts`.** vivacity never runs `composer.json` scripts on its
  own; with this flag it runs them through `composer run-script` at
  Composer's points (Laravel's `package:discover`, Symfony's
  `auto-scripts`). Without it, a deploy that relies on them breaks.
- **No `--verbose` needed.** Under `-v`, vivacity stays native with the
  plugins whose verbose lines it knows (a standard Laravel project's among
  them) and hands the deploy to Composer with any other (symfony/flex,
  dealerdirect's PHP_CodeSniffer installer…), which then gains nothing. The
  default flags above do not pass it.
- **The store** is `~/.cache/vivacity` (`VIVACITY_CACHE_DIR` to change it).
  Hardlinks need the store and the releases on the same filesystem; across
  filesystems vivacity copies (same result, smaller gain), so keep the
  deploy user's home and `deploy_to` on one volume, or point
  `VIVACITY_CACHE_DIR` next to `deploy_to`.

## What the mapping covers

`command_map[:composer]` replaces the binary for every task of
capistrano/composer:

- `composer:install` (the default hook) and `composer:dump_autoload`
  (`dumpautoload`) are vivacity commands;
- `composer:self_update` and `composer:run` with any other command
  (`config`, `show`, a plugin's own command…) are not: vivacity rejects them.
  If you use those, leave the mapping out and override only the install
  task, for example:

```ruby
Rake::Task['composer:install'].clear_actions
namespace :composer do
  task :install do
    on release_roles(fetch(:composer_roles)) do
      within fetch(:composer_working_dir) do
        execute '/home/deploy/.local/bin/vivacity', :install, fetch(:composer_install_flags)
      end
    end
  end
end
```

A lock vivacity does not lay out natively (an unported plugin, a `git`-only
package, the Drupal scaffold…) goes to the `composer` on the host, with the
reason on stderr, before anything is written: the deploy still succeeds, at
Composer's speed. `vivacity install --check-scope` on a checkout tells you
in advance (no network, nothing written; exit 0 means native).

## Checked

Read against capistrano/composer's tasks and README (`lib/capistrano/tasks/composer.rake`:
`composer:install` runs `composer install` with `composer_install_flags`
before `deploy:updated`; `composer:dump_autoload` runs `dumpautoload`, an
alias vivacity accepts). Not yet run against a real Capistrano deploy.
