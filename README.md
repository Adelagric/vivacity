# vivacity

[![ci](https://github.com/Adelagric/vivacity/actions/workflows/ci.yml/badge.svg)](https://github.com/Adelagric/vivacity/actions/workflows/ci.yml)

`composer install` and `composer update`, reimplemented in Rust. Given the
same `composer.json` and `composer.lock`, it writes the same `vendor/` and
the same lock file as Composer 2.10.3, byte for byte. No PHP is run.
(Named `vivace` up to 0.5.0; see the changelog for why it moved.)

```
composer install       →  vivacity install
composer update        →  vivacity update        (also `update vendor/name [-w|-W]`)
composer require       →  vivacity require
composer remove        →  vivacity remove
composer dump-autoload →  vivacity dump-autoload
```

Flags: `--no-dev`, `-o`, `-a`, `--no-autoloader`, `--no-install`,
`--prefer-stable`, `--prefer-lowest`, `--with` (and the
`update vendor/name:^1` shorthand), `--ignore-platform-reqs`,
`--ignore-platform-req`, `--working-dir`. `config.optimize-autoloader`,
`config.classmap-authoritative`, `config.platform`, `config.allow-plugins`
and `config.lock` are read from `composer.json` the way Composer reads them.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Adelagric/vivacity/main/install.sh | sh
```

Linux x86_64/arm64, macOS arm64/x86_64, Windows x86_64 (Git Bash); the
script checks the sha256. The Linux binaries run on **glibc 2.17 or later**
(CentOS/RHEL 7 and up, Amazon Linux 2 and 2023, Debian 10+, Ubuntu 20.04+,
GitHub's `ubuntu-22.04` and `ubuntu-24.04` runners); on **musl** (Alpine,
the official `composer` image, `php:*-alpine`) the script picks a static
build — it needs `curl` there (`apk add curl`). Both are checked on every
release by running the binary in `centos:7` and `alpine`
(`tools/linux-release.sh`). On Windows from PowerShell:

```powershell
irm https://raw.githubusercontent.com/Adelagric/vivacity/main/install.ps1 | iex
```

Also `cargo install vivacity` (crates.io), `cargo binstall vivacity` (the
same release assets), or `cargo install --path crates/vivacity`; a
`cargo install` build links against the glibc of the machine that builds
it.

GitHub Actions:

```yaml
- uses: Adelagric/vivacity@v0.25.0
- run: vivacity install
```

The action caches Composer's cache directory only (dist zips, VCS mirrors,
repository metadata), keyed on the hash of the `composer.lock` files.
vivacity reads that cache and feeds it, and rebuilds its own store from the
zips without the network (dists only: a package installed from git may
still fetch), so its store is not cached: restored, it did not make the install faster (DECISIONS
2026-10-29), and a store written by an older layout is never read. Its
class map records are not cached either. They would save the class scan
of an `-o` install (1.4 s on Laravel, 2.8 s on Sylius, on an M4 Max:
[bench/M8-classmap-cache.md](bench/M8-classmap-cache.md)), but a record is
keyed on a package's version and reference, not on the archive the store
was just rebuilt from, so a restored one could list other files. The
directory is the one `vivacity config --global cache-dir` prints — the
same value as `composer config --global cache-dir` — exposed as the
`composer-cache-dir` output and resolved once, when the action runs. Set
`COMPOSER_CACHE_DIR`, `COMPOSER_HOME` or `XDG_CACHE_HOME` at job level
(the job's `env:`), so that the action sees them: one set in a later step,
or only on the `run:` step that installs, is not followed. A relative value
is not cached (a notice says so), nor is a `cache-dir` set in a project's
`composer.json`. Do not cache that same
directory yourself as well (`actions/cache` or setup-php's recipe): two
caches would save the same files under two keys. The action uses
`actions/cache` v6.1.0, which needs runner 2.327.1 or later (GitHub-hosted
runners are newer; check GHES and old self-hosted runners). `cache:
'false'` turns caching off. In a job `container:`, the action runs its steps
with `bash` and `curl`: an Alpine image needs both installed first.

In a Docker image build: [docs/docker.md](docs/docker.md). Deploying with
Capistrano: [docs/capistrano.md](docs/capistrano.md); with Deployer:
[docs/deployer.md](docs/deployer.md).

## How it is checked

`harness/diff-vendor.sh` runs `composer install` and `vivacity install` on six
projects (Laravel, the Symfony demo, Sylius, rector-src, a WordPress site
using `composer/installers`, a Drupal `recommended-project`) and `diff -r`s
the results — the whole project tree for the last two, since their files
land outside `vendor/`; each fixture that allows plugins is then installed
again under `--no-plugins` on both sides, where every plugin is a plain
library and vivacity must stay native. `harness/update.sh` does the same for
`composer update --no-install` and `vivacity update --no-install` against
frozen Packagist snapshots, comparing the lock files; `harness/path-repos.sh`
plays install, update, remove and require on a project served by `path`
repositories and compares stderr, the lock and `vendor/` down to file modes
and link targets; `harness/tar-dist.sh` installs `tar` dists (asset-packagist's
npm tarballs) on both sides and compares the tree, modes included — the
modes come from the tar headers, as `PharData` writes them, and the
tarballs are read from Composer's own cache. `harness/artifact-repo.sh`
needs no network at all: it builds an `artifact` repository of zips carrying
modes real zipballs never have (0600, 0640, 0666, 0700) and installs it on
both sides with a cold files cache and a cold store, comparing exit codes,
stderr, `vendor/` and every mode. All must report no
difference; CI runs them on Linux and macOS on every push.

The comparator those harnesses share has its own test:
`harness/compare-selftest.sh` builds eleven pairs of trees and demands a verdict
for each, because a tolerance that is not anchored erases the difference it
touches — three of them were substring matches until 2026-10-01, and four of the
eleven cases are red against that version.

Underneath, each generated file and each step of the resolver is a port of
the corresponding Composer function; the source files ported are vendored
in `docs/reference/` and a CI step re-diffs them against the phar, so an
upstream change fails the build instead of drifting silently. Ports with
tricky semantics (class detection, JSON encoding, version constraints,
the solver's decision sequence) also have tests that call the real Composer
phar on the same inputs. `tests/`, `harness/`, `tools/oracle-*.php` and
`fixtures/` are all in the repo; the fixtures are downloaded by
`fixtures/make.sh`.

How much of the real world that covers is measured, not assumed:
[docs/corpus/](docs/corpus/) holds the latest run of `harness/corpus.sh` on
106 real PHP projects (application templates and applications with a
committed lock, pinned — `fixtures/corpus/`). On 2026-10-29 (after
0.25.0), against Composer 2.10.3 `--no-scripts` with its
plugins active, `vivacity install` laid out **95 of 106 projects (90 %)
natively with `--no-dev`** and 95 of 106 (90 %) with the dev packages,
byte-identical down to file modes and link targets (`.git` directories
of git-only packages included), and **no diff in either mode**: every
other project was handed to Composer before any write, for reasons the
report ranks — the Drupal plugins (the scaffold is GPL, not ported),
mautic's and silverstripe's own logic, then plugins one by one. The dev
count rose from 94 when `dealerdirect/phpcodesniffer-composer-installer`
v0.7.2 was emulated and chamilo-lms's fixture started fetching the legacy
files its autoload loads (the thirteen entries that use dealerdirect
replayed, the rest carried from the run before). Earlier that day it was 93 and 91, before `infection/extension-installer`
(roave-backwardcompatibilitycheck), the TYPO3 plugins (typo3) and
`mlocati/composer-patcher` (concretecms); before that 92 and 90, before
`drupol/composer-packages` (bolt-project);
earlier that day (0.25.0), 85 and 83, the seven plugins that act at script
events making the difference; on 2026-10-21, 76 and 74, before eight
custom installer plugins and git-only packages. The
reasons that headed that list three days earlier (`symfony/thanks`,
`yiisoft/yii2-composer`, asset-packagist's tarballs,
`bamarni/composer-bin-plugin`, `roots/wordpress-core-installer`,
`ergebnis/composer-normalize`) are handled now. The corpus also found
thirteen parity bugs that six fixtures never could; all are fixed.

What is not covered is listed in [HANDOVER.md](HANDOVER.md).

## Speed

Warm `install` is bound by writing tens of thousands of small files and
rescanning class maps, not by PHP. vivacity extracts each package once into a
content-addressed store, clones it into `vendor/` (`clonefile` on APFS,
hardlinks elsewhere), and caches the class map per store entry. Measured
numbers and the scripts that produce them are in [bench/](bench/); the
short version is that a no-op install takes tens of milliseconds, a warm
reinstall of Sylius under a second, and `update --no-install` on Sylius
about a third of Composer's time. Cold network installs are not faster.
Online, a no-op install is mostly one request: like Composer 2.10,
`install` revalidates Packagist's blocking lists on every run (a
conditional request, 70–200 ms depending on the network), so a locked
version flagged as malware after the lock was written is refused. That
check is not cached behind a TTL, by decision (DECISIONS 2026-09-19); the
ways to skip it are Composer's own, with Composer's consequences:
`--no-blocking` / `--no-security-blocking`, `config.policy.malware.block`
/ `block-scope`, or `--offline` (cache, with a warning). With
`--no-blocking` a Laravel no-op is ~50 ms on an M4 Max, ~35 ms on Linux.
CI computes the vivacity/Composer ratio per scenario (`bench/gate.py`,
tolerance 15 % past `bench/results/baseline-ratio.json`) and writes the
verdict into the job summary. It **reports**, it does not block: the ratio
moves 15–50 % between identical runs on shared runners (six false alarms on
2026-09-19/20), so the `bench` job stays green and a real regression is
caught by `hyperfine` before the commit.

## The resolver

`vivacity update` uses Composer's own algorithm, ported function by function:
`PoolBuilder`, `PoolOptimizer`, `RuleSetGenerator`, the CDCL `Solver`,
`DefaultPolicy`, `LockTransaction`, `Locker`. Any other algorithm would pick
different versions on ambiguous inputs and produce a lock nobody can compare
with Composer's. The port is checked on frozen snapshots by comparing the
candidate pool, then the solver's complete decision sequence, with what
Composer computes on the same data (`tools/oracle-pool.php`).

When no set satisfies the requirements, vivacity prints Composer's
explanation — the numbered problems with their reasons, the hints
(`Potential causes`, the php.ini list for a missing extension, the `-W`
advice) — word for word: `Problem::getPrettyString`, `Rule::getPrettyString`
and `SolverProblemsException` are ported, and `harness/steps.sh` compares
that text byte for byte with Composer's on 78 cases that exit 2 (a
synthetic registry built by `fixtures/make-solver-problems.py` reaches every
reason branch the CLI can produce, plus real-registry cases).

What the commands print is Composer's text too: the `Lock file
operations` summary and its `  - Locking …` / `Upgrading` / `Removing`
lines, the suggestions count, the abandoned-package warnings, the funding
line, `install`'s `Package operations`, and `--dry-run` on `update`,
`require` and `remove` (resolution and listing, nothing written — the
install phase lists its operations from the unwritten lock). The stderr
of every `steps.sh` case is compared byte for byte from the first of
those lines to the end (234 cases). A real `install` prints the same
`  - Installing … : Extracting archive` / `Symlinking from …` lines and
`Generating autoload files`, plus one `vivacity:` summary line.

Repositories: `composer` (Packagist v2 protocol and plain `packages.json`
files, local or over HTTPS) and `path` (globs and braces, a package's own
git repository as reference, the version guessed from its branch or the
project's, symlinked or mirrored with Composer's exclusion rules — on
Linux and macOS; on Windows a lock with a `path` package goes through the
Composer fallback). Not yet: `vcs` repositories, `--minimal-changes`, the
audit (`--no-audit` is the compared behaviour).

A locked package with no dist at all and a `git` source (a `package` or
`vcs` repository without archives) is installed as Composer installs it:
the `git` binary runs `GitDownloader`'s commands — the mirror in the VCS
cache (`  - Syncing … into cache`), then a clone of it with its `.git`
directory (`: Cloning <ref> from cache`), `installation-source: source` —
for an `https://` URL without credentials or a local path, on Linux and
macOS. Its `.git` is compared with Composer's (`harness/git-sources.sh`):
byte for byte, except the index, the reflog timestamps and the packing of
objects, which two Composer runs do not reproduce either. Other schemes
(ssh, `http`, `git://`), other VCS, a configured cache or GitHub protocol
list, network disabled, an install path outside the vendor directory, the
update or removal of a package installed from source, and
`preferred-install: source` / `--prefer-source` for packages that have a
dist go through the fallback with the reason.

Composer's global options work on every command: `-q` leaves only what
Composer prints at its quiet level (errors, the headline of an unsolvable
set, the stale-lock warning) and the exit code; `-v` adds Composer's
verbose lines in their places — resolution durations, the pool and rule
counts, the `Installs:` / `Updates:` / `Removals:` summaries, the detailed
explanations, and the `> <event>: <class>-><method>` line Composer writes
for each plugin listener it calls, in the plugins' activation order:
symfony/runtime, pestphp/pest-plugin, php-http/discovery,
composer/package-versions-deprecated, phpstan/extension-installer,
rector/extension-installer, codeception/c3, drupal/core-project-message,
drupal/core-recipe-unpack and bamarni/composer-bin-plugin (with its two
`Calling …` lines), each in the versions whose source was read;
composer/installers, roots/wordpress-core-installer,
mnsami/composer-custom-directory-installer, ibexa/post-install and
ergebnis/composer-normalize have no listener and add nothing.
`-vv` and `-vvv` hand the command to Composer, whose debug output vivacity
does not reproduce; so does `-v` with symfony/flex, dealerdirect,
composer-merge-plugin, yii2-composer, symfony/thanks or the licence-check
plugin, with any other plugin or version, with a plugin installed
globally, and under `--run-scripts`; a plugin that
a resolution would bring in is judged on the solved lock, before it is
written, and the delegation line names the reason. The level is
read as Symfony Console reads it (`SHELL_VERBOSITY`, then `-q`, then
`-v…`), and passed on to every Composer process vivacity starts.
`harness/verbosity.sh` compares stdout and stderr whole, at each level.

Composer 2.10's dependency policies are applied the same way: versions
covered by a security advisory or flagged on Packagist's malware list are
removed from the pool before resolution (`config.policy`, the legacy
`config.audit` keys, `COMPOSER_POLICY*`, `--no-blocking`), and `install`
refuses a lock that pins a flagged version. Advisories and list entries
come from the same metadata files Composer reads, so the frozen snapshots
carry them and the parity checks run with the policies on. Custom policy
lists with sources and a repository `filter.api-url` are not supported
(refused when reached).

Before laying out a lock it did not just resolve, `install` solves it
again, as `Installer::doInstall` does: every locked package fixed, the
root's platform requirements and then the lock's, the root's `conflict`
links, the lock's stability and `platform-overrides`, the blocking lists,
`--ignore-platform-req(s)` (and `COMPOSER_IGNORE_PLATFORM_REQ(S)`). A lock
that fails gets Composer's explanation and exit code — 4 for a root
requirement the lock does not contain (checked first), 2 for an unsolvable
lock, 4 for one that would need changes.

`vivacity require` and `vivacity remove` edit `composer.json` the way Composer
does — a port of `JsonManipulator`, which rewrites only the affected keys
and keeps the file's layout — pick the same `^x.y` constraint (a port of
`VersionSelector`), then run the same partial update, and `require`
rewrites the constraint and the lock's `content-hash` from the resolved
version the way `RequireCommand` does. `harness/steps.sh` plays the same
commands through both tools on the frozen snapshots and compares
`composer.json`, `composer.lock` and the exit code. Not ported: the
interactive prompts (vivacity behaves like `--no-interaction`) and the
Packagist search behind "Did you mean …".

## Plugins and scripts

Scripts are never run by vivacity itself (see `--run-scripts` below).
Thirty-three plugins are emulated and checked against the
real ones: `symfony/runtime`, `composer/installers` (versions
1.12.0 and 2.0.0–2.3.0, frameworks that only use the plugin's path table —
WordPress and Drupal included; 1.12.0's PHP 8.4 deprecation lines
included), `pestphp/pest-plugin` (`vendor/pest-plugins.json`),
`dealerdirect/phpcodesniffer-composer-installer` (PHP_CodeSniffer's
`installed_paths`, versions 0.7.2 and 1.0.0–1.2.1 but 1.1.0; 0.7.2 searches
the whole project for rulesets, vendor/ included), `phpstan/extension-installer`
(1.4.x) and `rector/extension-installer` (0.11.x, 1.0.0) (their
`GeneratedConfig.php`), and
`wikimedia/composer-merge-plugin` (the included manifests merged into the
root — autoload, requirements, links, `extra` — for `install` and
`dump-autoload`; with one deliberate difference: on a bare vendor the
plugin would run an implicit `composer update` of the merged requirements
and rewrite `composer.lock`, vivacity never does — a lock that misses a
merged requirement is handed to Composer instead, and `update` on such a
project is refused), and `yiisoft/yii2-composer` (`vendor/yiisoft/extensions.php`
from the `yii2-extension` packages — aliases from their `psr-0` / `psr-4`,
`bootstrap` from their `extra` — rewritten as extensions come and go;
`yiisoft/yii2-dev`'s `Yii.php` shims are not emulated), and
`codeception/c3` (`c3.php` copied to the project root, never over a
modified one, deleted when the plugin leaves), and
`bamarni/composer-bin-plugin` (its deprecation lines; a project with
`forward-command: true` — nested installs under `vendor-bin/` — goes to
Composer), `roots/wordpress-core-installer` (the `wordpress-core`
package at `extra.wordpress-install-dir`, Bedrock's layout), and
`mnsami/composer-custom-directory-installer` (a library named in
`extra.installer-paths` laid out there), the custom installers
`getkirby/composer-installer` 1.2.1 (Kirby and its plugins, the
package's own `vendor/` removed as the plugin does),
`neos/composer-plugin` 2.1.1–2.1.3 (Flow packages under `Packages/`),
`davidbarratt/custom-installer` 1.1.0 (`extra.custom-installer`),
`johnpbloch/wordpress-core-installer` 2.0.0, `thelia/installer` 1.6
(Thelia 2 and 3 layouts), `contao-components/installer` 1.4.0–1.4.1
(`extra.contao-component-dir`) and `oomphinc/composer-installers-extender`
2.0.0–2.0.1 (`extra.installer-types`), and `craftcms/plugin-installer`
1.6.0 when no `craft-plugin` package is involved — when two of these
installers take one package type and what they do besides the path
differs, the command goes to Composer — and
`composer/package-versions-deprecated` (1.11.99.4/5: its own `Versions.php`, rewritten
from the lock at autoload-dump time — the shipped file is a stub), and
`symfony/flex` at install time (`.env` copied from `.env.dist`, its own
lines, `symfony-pack` packages laid out as metapackages; what it does
when resolving is described below), and seven that act at script events
(`docs/plans/plugins-round2.md`): `dg/composer-cleaner` v2.2.2 and
`liborm85/composer-vendor-cleaner` 1.7.1 (the files they delete from
vendor/, their lines), `cakephp/plugin-installer` 2.0.2
(`vendor/cakephp-plugins.php` and the application plugins' autoload),
`laminas/laminas-skeleton-installer` 1.2.0–1.3.0 (the non-interactive
minimal install: composer.json rewritten), and, where they do nothing,
`spiral/composer-publish-plugin` v1.1.1–v1.1.2,
`sculpin/sculpin-theme-composer-plugin` 1.0.3 and
`laminas/laminas-component-installer` 3.5.0–3.8.0 — a publish command, a
theme, a configuration file to inject into, the skeleton installer's
removal (Composer loops on it), and the other cases the plan names go to
Composer — and `drupol/composer-packages` 2.0.0
(`docs/plans/duration-drupol-collector.md`: its five classes generated from
the lock under `vendor/drupol/composer-packages/build/`; its removal, an
installer plugin or a cleaner beside it, Twig 2 and the other cases the
plan names go to Composer), and, after `docs/plans/remaining-fallbacks.md`,
`infection/extension-installer` 0.1.2 (its `GeneratedExtensionsConfig.php`),
`typo3/class-alias-loader` v2.0.1 (its two files and the include file in
the root's autoload; packages with class alias maps — PHP code — go to
Composer) with `typo3/cms-composer-installers` v5.0.2 where it was
observed to do nothing (a root named `typo3/cms`; its source is GPL and
was not read), and `mlocati/composer-patcher` 1.2.6 (patches applied by
the `patch` binary with the plugin's own command lines, `patches_applied`
in installed.json; `update`, remote patches and the other cases the plan
names go to Composer). The lines these plugins print are
printed too, in the order Composer calls their listeners (the order it
activated the plugins: installed.json sorted by `PackageSorter`, then those
the command installs) — on `update`, `require` and `remove` with
`--no-install` too, where Composer still runs `post-update-cmd` for the
plugins already installed, and they read the installed packages rather
than the new lock; another version of a plugin whose listener changed
across versions, or an extension these installers list under an alias,
goes to Composer with the reason (without an install, so does
`php-http/discovery` when it looks after a PSR implementation: its
listener may edit `composer.json` and update again). A plugin listed
as `false` in `allow-plugins` is
skipped, like Composer does. A short list of plugins that do nothing at install
time (`php-http/discovery`, `symfony/thanks`,
`ergebnis/composer-normalize`, `ibexa/post-install`, …) is installed as
plain libraries. `drupal/core-composer-scaffold` is not
emulated (its source is GPL-2.0-or-later, see NOTICE.md): a project that
uses it goes through the Composer fallback below.

`config.vendor-dir` and `config.bin-dir` are honoured (project config,
global config, `COMPOSER_VENDOR_DIR` / `COMPOSER_BIN_DIR`, the
`{$vendor-dir}` placeholder): the packages, the proxies, the state files
and the autoloader follow the directories, and a project-owned file in
the bin directory is kept with Composer's `Skipped installation of bin`
notice. Forms the harness does not cover (absolute or `..` paths, `~`,
`$VAR`) go through the fallback.

Anything else — other plugins, `composer/installers` cases with custom
naming, source-only packages beyond git over https or a local path, a
`path` package on Windows, a plugin
upgrade in progress — is detected
before `vendor/` is touched, and vivacity hands the command to the real
`composer install` instead (`--no-fallback` to make it fail): the command
line it was given, minus vivacity's own options (`--offline` becomes
`COMPOSER_DISABLE_NETWORK=1`; `--no-scripts` is added unless
`--run-scripts`), run as a child process in the same directory — a
relative `--working-dir` means the same thing — whose exit code is
vivacity's. A program that embeds `vivacity::run` keeps its process.

The resolution commands have the same policy. Composer loads the
installed plugins before resolving, and some change what gets resolved
or written — `symfony/flex` filters the pool against
`extra.symfony.require` and applies recipes, `wikimedia/composer-merge-plugin`
merges other manifests into the root, `drupal/core-recipe-unpack` edits
composer.json on `require`. When one of those does something vivacity
does not emulate (or a plugin outside vivacity's lists is installed and
allowed), `update`, `require` and `remove` hand the whole command to
Composer with the same arguments, before any write (an edit of
composer.json by `require`/`remove` is undone first), and say why;
`--no-fallback` stops with exit 3 instead. When only their install phase
is out of scope, the lock is written as Composer writes it and Composer
gets a `composer install` built from the same options. Plugins with no
resolution-time listener (`composer/installers`, `symfony/runtime`, the
extension-installers, `php-http/discovery`, `ergebnis/composer-normalize`,
`cweagans/composer-patches`…) do not trigger it; `symfony/thanks` only on
`update` with install (its reminder queries GitHub after a package
update).

`wikimedia/composer-merge-plugin` is emulated at resolution as it is at
install: the root `update`, `require` and `remove` resolve is the merged
one — the included files' links (with the plugin's duplicate rules,
`replace`, `ignore-duplicates`, `merge-dev`, `recurse`), their
`conflict` / `replace` / `provide`, their stability flags as the plugin
extracts them from each file's own constraints, their aliases and
references — and the lock is Composer's with the plugin active
(`harness/merge-plugin.sh`: update, `--no-dev`, `replace`, an `@dev`
flag in an included file, require, remove). Two cases stay Composer's: an
included file that declares `repositories` (fallback), and the plugin
required but not installed yet, where Composer resolves the unmerged
root and then runs the plugin's implicit update (refused with the
reason).

`symfony/flex`'s pool filter is emulated for `update` and `remove` with
`--no-install`, and `update` **with** install is emulated too — including
when Flex applies recipes. What Flex records is not the install's transaction:
`recordOperations` builds a synthetic one between the packages
`symfony.lock` names and the whole resolved set, and keeps only the
installs of names that file does not hold. Each of those is answered on its
own — a recipe in the endpoints' index, or a bundle class in the content
the install would lay out (read out of the dist for a package it changes,
out of vendor/ for one it leaves alone, and nowhere for one an installer
puts outside `<vendor-dir>/<name>`, which is the only place Flex looks).
The recipes are then fetched from the endpoints like Flex's `Downloader`
does (the `recipe-conflicts` of the index and a recipe's own `conflict`
evaluated against the lock) and applied in Flex's order at the point
Composer dispatches `POST_UPDATE_CMD`: `Symfony operations: N recipes`,
`  - Configuring …`, the files, `symfony.lock` with its new entries. The
configurators reproduced are `bundles` (and the auto-generated recipe of a
bundle without one: `config/bundles.php`, recognised only in the exact form
Flex writes it), `copy-from-recipe` (executable bit included, existing files
left alone), `env` and `dotenv` (`.env*` and the PHPUnit files),
`gitignore`, `add-lines`, `composer-scripts` (composer.json, then the
lock Flex rewrites through `updateComposerLock`), and the two Docker
configurators when `extra.symfony.docker` is false. Everything is decided
on a virtual copy of the project before the lock is written; what is not
reproduced hands the command over with its reason: the `container`,
`makefile`, `composer-commands` and `copy-from-package` configurators,
Docker configuration that is enabled or not decided (Flex asks), a contrib
recipe without `allow-contrib`, a `%generate(secret)%` value that would be
written, a recipe conflict that matches, a non-default `root-dir`, a
recipe that would look at a path the install itself writes (`bin-dir`,
packages laid out outside vendor/), an endpoint on the GitHub API, and
`--offline` with a recipe to download (`docs/plans/flex-configurators.md`).
Anything unanswerable before writing, a removal of a package
`symfony.lock` holds (Flex unconfigures it), or `symfony/flex` being
installed by the run itself — Flex then runs a whole second installer —
hands over before the lock is written: the index of its endpoints (`extra.symfony.endpoint`,
`SYMFONY_ENDPOINT`, the two recipe indexes by default; cached in Flex's
own format under Composer's cache) prunes the pool against
`extra.symfony.require` / `SYMFONY_REQUIRE` exactly as
`PackageFilter::removeLegacyPackages` does (`Restricting packages listed
in "symfony/symfony" to …`), with `COMPOSER_PREFER_DEV_OVER_PRERELEASE`
as Flex sets it; the lock is Composer's with Flex active (checked by
`harness/update.sh` on the Symfony demo and Sylius). What Flex writes
besides the lock and the recipes stays Composer's: on `require` (aliases,
version guessing) and `remove` with install (unconfiguring), a `.env.dist`
to copy, a `symfony-pack` to unpack.

The `package.json` / `importmap.php` synchronisation is emulated by
answering what it would write rather than by its trigger. Flex runs it on
`update` (never on `install`: `finish()` is only reached at
`POST_UPDATE_CMD`) as soon as either file exists, but with `importmap.php`
it writes nothing when no locked package carries the `symfony-ux` keyword
and `assets/controllers.json` is absent, and with `package.json` nothing
when, on top of that, no `@…: file:<vendor-dir>/…/assets` link has lost
its package and the file already has the shape Composer's
`JsonManipulator` gives it back (`trim` plus one newline — that rewrite is
unconditional). Those are the cases vivacity runs natively; anything else
hands the command over before the lock is written.

Scripts stay Composer's business: `vivacity install --run-scripts` hands
each event the project declares to `composer run-script`, at the points
where Composer dispatches them — `pre-install-cmd` before anything,
`pre-autoload-dump` and `post-autoload-dump` around the dump,
`post-install-cmd` at the end — with `COMPOSER_DEV_MODE` set from
`--no-dev`; a failing script stops the run with its exit code, like
Composer. Static callables (`Illuminate\Foundation\ComposerScripts::…`)
work, since Composer runs them. Not carried: the per-package events
(`pre/post-package-*`) and the `optimize` flag of the autoload events.
Without the flag, no script ever runs (`composer run-script post-install-cmd`
is one command away).

Root manifest refusals are Composer's. `Factory::createComposer` loads the
root package for every command, so Composer refuses an invalid manifest
whatever you asked it to do; vivacity only built a root package for a
resolution, and so used to accept manifests the reference rejects — writing a
lock, or dumping an autoloader, where Composer writes nothing and exits 1.
Now refused at all five commands, before anything is written, with Composer's
own message and its exit code: an invalid **root package name** (the rules the
schema lets through — a `.json` suffix, a reserved Windows device name), a
package that **requires itself**, and an invalid **link name** in any of
`require`, `conflict`, `provide`, `replace`, `require-dev`
(`ValidatingArrayLoader::hasPackageNamingError`, ported with PCRE2 so the
case-insensitive class stays ASCII as PHP's does). The message is printed
without Symfony Console's box and without the command synopsis — the same
deviation already taken for path repositories, and measured: the box is
wrapped to the terminal's width and is sometimes chained.

Three of these rules are ported; the rest of what Composer refuses in a root
manifest is not, and the shape of each gap is different.

*Refused, with different words.* Composer validates its JSON schema before the
loader, and the schema catches an uppercase or malformed `name` (its pattern is
case-sensitive) and a constraint that is not a string; `parent::load` catches an
unparsable constraint before the link-name loop. On such a manifest vivacity
refuses too — same exit code, nothing written — but names the rule it can see
rather than the one Composer names. Words are the cheaper divergence, and the
project already takes one for the box; staying silent would let `dump-autoload`
write an autoloader where Composer writes nothing.

*Accepted on `install` and `dump-autoload`.* An unknown `minimum-stability`, a
`version` that is not a version, a repository with no `type` or an unknown one:
Composer refuses all three through the schema, and `update` refuses the first
two through its own ports. Reaching them on the other commands means loading
the root package, i.e. guessing its version, i.e. running git on the install
path — so they are left. Porting the schema itself means validating
`res/composer-schema.json`, which vivacity does not do.

*Not refused at all.* `COMPOSER=<file>` is refused as unsupported rather than
honoured, so a manifest named there is never validated — nor installed from.

A hostile archive is the one family where vivacity **fails instead of handing
over**. Everything else it cannot reproduce is detected before the first write,
so the command goes to Composer; an archive, by definition, is only known once
it is being read, halfway through an install. So a dist that would write
outside its own directory — a symlink whose target leaves it, an entry written
through a symlink the archive just created, a path with `..` or an absolute one,
more decompressed bytes than the 512 MB budget — stops the command with a
non-zero exit. Measured on the reference, the outcomes coincide: `unzip` also
refuses the traversal, Composer then retries with `ZipArchive` on top of the
partial tree, that fails too, and the install fails with nothing installed for
that package.

What differs is what a *failed* install leaves behind. Composer keeps the
packages it had already installed and records them in `installed.json`;
vivacity writes `vendor/` only once every archive is in its store, so a failure
anywhere leaves `vendor/` untouched (measured on a two-package project whose
second dist is hostile: Composer leaves the first one installed, vivacity
leaves nothing). Re-running either command converges to the same tree, and
vivacity's store makes its retry cheap — but the intermediate state is not the
same, so it is written here rather than implied.

Modification times are not reproduced for zip dists, and that is a deliberate
gap. `unzip` restores the archive's DOS timestamp on files and directories, so
Composer's `vendor/` carries the dates the packager stored (measured: a dist
stamped 2020-01-02 03:04:06 installs with that date on both sides of Composer's
own path); vivacity leaves the time of the install. Reproducing it means
interpreting a DOS timestamp the way `unzip` does — local wall-clock, converted
with the DST rules in force at that date — which needs a timezone database, for
a property no file content, mode or link target depends on and which the
contract does not claim. `PharData` does not restore them either, so `tar` dists
match the reference as they are.

Not supported: `gitlab-token` auth, root version detection from
hg/svn/fossil.

## Embedding

The `vivacity` crate is a library with a thin binary on top: another program
can run any vivacity command in-process and get the binary's exit code.

```rust
// Cargo.toml: vivacity = "0.5"
let code = vivacity::run(["vivacity", "install", "--working-dir", "/srv/app"]);
```

The lower layers are separate crates (`vivacity-core`: manifests, lock,
store, installers; `vivacity-resolver`: the resolver port; `vivacity-autoload`:
the autoloader generator).

Embedded by [ePHPm](https://github.com/ephpm/ephpm) since
[v0.11.0](https://github.com/ephpm/ephpm/releases/tag/v0.11.0)
(vivacity 0.15 as of its
[v0.11.1](https://github.com/ephpm/ephpm/releases/tag/v0.11.1)): its
[`ephpm composer`](https://ephpm.dev/reference/cli/composer/) subcommand
forwards every argument to vivacity compiled into the same binary, so
`ephpm composer install` needs neither a PHP runtime nor `composer.phar`.

Embedding brings in vivacity's HTTP stack (reqwest over rustls). The default
feature `rustls-tls-ring` bundles the ring crypto provider — right for the
standalone binary, wrong for a host that already links another rustls
provider: Cargo unions features across the whole tree, so both providers end
up compiled in and `rustls::CryptoProvider::from_crate_features()` returns
`None`, which panics every bare `ClientConfig::builder()` in the process at
runtime. Such a host selects the provider-agnostic backend instead and
installs its own process-wide default provider before the first request:

```toml
vivacity = { version = "0.25", default-features = false, features = ["rustls-tls-no-provider"] }
```

vivacity's regular expressions run on PCRE2 (Composer's patterns need it:
recursive subpatterns, possessive quantifiers, byte-mode class scanning),
built in from source with every symbol prefixed `vivacity_`
(`vivacity-pcre2-sys`, `vivacity-pcre2`). A host that links its own PCRE2
— PHP's `libphp.a` bundles one, with the usual `pcre2_*_8` names — gets no
duplicate symbols from vivacity; another crate pulling the stock
`pcre2-sys` into the same binary would still collide with the host's, that
is not vivacity's to fix. The prefix is checked in CI
(`tools/check-pcre2-symbols.sh`) and by a link test against a foreign
PCRE2 and bzip2 (`crates/pcre2-link-test`).

The decompressors behind dist extraction follow the same rule: everything
decompresses in pure Rust — deflate on flate2's Rust backend, bzip2 on
`libbz2-rs-sys` (exports carry the `LIBBZ2_RS_SYS_v…` semver prefix), LZMA
on `lzma-rs` — so no unprefixed `BZ2_*` / `inflate*` / `lzma_*` / `ZSTD_*`
symbol reaches the host's link. PHP's static builds bundle bz2 and zlib
everywhere, lzma and zstd depending on the platform. Zip entries using the
`xz` or `zstd` methods are refused rather than supported by a colliding C
decoder — no Composer registry produces them; store and deflate are what
dists contain. `tools/check-clib-deps.sh` pins all of this in CI.

## Development

```bash
fixtures/make.sh             # once; needs php and composer
cargo test                   # unit tests and oracles against the Composer phar
harness/diff-vendor.sh --with-autoloader
harness/update.sh
harness/steps.sh
harness/path-repos.sh
harness/removal.sh
harness/transitions.sh
harness/boot.sh
harness/drift-reference.sh   # docs/reference/ vs the installed Composer
harness/linux.sh             # everything in a Linux container
```

Design notes: [DECISIONS.md](DECISIONS.md). Plans: `docs/plans/`.

## License

MIT or Apache-2.0, at your option. The ports are derived from Composer
and its libraries (MIT, © Nils Adermann, Jordi Boggiano, and the Composer
project) and from `composer/installers` (MIT, © Kyle Robinson Young);
the reference sources are vendored in `docs/reference/` with their
license texts — see [NOTICE.md](NOTICE.md).
