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
`--prefer-stable`, `--prefer-lowest`, `--ignore-platform-reqs`,
`--ignore-platform-req`, `--working-dir`. `config.optimize-autoloader`,
`config.classmap-authoritative`, `config.platform`, `config.allow-plugins`
and `config.lock` are read from `composer.json` the way Composer reads them.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Adelagric/vivacity/main/install.sh | sh
```

Linux x86_64/arm64, macOS arm64/x86_64, Windows x86_64 (Git Bash); the
script checks the sha256. On Windows from PowerShell:

```powershell
irm https://raw.githubusercontent.com/Adelagric/vivacity/main/install.ps1 | iex
```

Also `cargo install vivacity` (crates.io), `cargo binstall vivacity`, or
`cargo install --path crates/vivacity`.

GitHub Actions:

```yaml
- uses: Adelagric/vivacity@v0.16.0
- run: vivacity install
```

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
and link targets. All must report no difference; CI runs them on Linux and
macOS on every push.

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
105 real PHP projects (application templates and applications with a
committed lock, pinned — `fixtures/corpus/`). On 2026-09-19 (after the
0.14 `--no-plugins` fix and the install/dump speed-ups, which the corpus
re-checks byte for byte), against Composer 2.10.3 `--no-scripts` with its
plugins active, `vivacity install` laid out **66 of 105 projects (63 %)
natively with `--no-dev`** and 58 of 105 (55 %) with the dev packages,
byte-identical down to file modes and link targets, and **no diff in
either mode**: every other project was handed to Composer
before any write, for reasons the report ranks — packages without a zip
dist, then plugins one by one. The reasons that headed that list a day
earlier (`pestphp/pest-plugin`,
`dealerdirect/phpcodesniffer-composer-installer`,
`phpstan/extension-installer`, `config.vendor-dir`, `bin-dir`,
`wikimedia/composer-merge-plugin`) are handled now. The corpus also found
twelve parity bugs that six fixtures never could; all are fixed.

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
CI gates on the vivacity/Composer ratio per scenario (`bench/gate.py`,
tolerance 15 % past `bench/results/baseline-ratio.json`), so a regression
shows up as a red `bench` job rather than a slower table.

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
that text byte for byte with Composer's on 50 unsolvable cases (a
synthetic registry built by `fixtures/make-solver-problems.py` reaches every
reason branch the CLI can produce, plus real-registry cases).

What the commands print is Composer's text too: the `Lock file
operations` summary and its `  - Locking …` / `Upgrading` / `Removing`
lines, the suggestions count, the abandoned-package warnings, the funding
line, `install`'s `Package operations`, and `--dry-run` on `update`,
`require` and `remove` (resolution and listing, nothing written — the
install phase lists its operations from the unwritten lock). The stderr
of every `steps.sh` case is compared byte for byte from the first of
those lines to the end (206 cases). A real `install` prints the same
`  - Installing … : Extracting archive` / `Symlinking from …` lines and
`Generating autoload files`, plus one `vivacity:` summary line.

Repositories: `composer` (Packagist v2 protocol and plain `packages.json`
files, local or over HTTPS) and `path` (globs and braces, a package's own
git repository as reference, the version guessed from its branch or the
project's, symlinked or mirrored with Composer's exclusion rules — on
Linux and macOS; on Windows a lock with a `path` package goes through the
Composer fallback). Not yet: `--with`, `vcs` repositories,
`--minimal-changes`, the audit (`--no-audit` is the compared behaviour),
the `--verbose` form of the explanations.

Composer 2.10's dependency policies are applied the same way: versions
covered by a security advisory or flagged on Packagist's malware list are
removed from the pool before resolution (`config.policy`, the legacy
`config.audit` keys, `COMPOSER_POLICY*`, `--no-blocking`), and `install`
refuses a lock that pins a flagged version. Advisories and list entries
come from the same metadata files Composer reads, so the frozen snapshots
carry them and the parity checks run with the policies on. Custom policy
lists with sources and a repository `filter.api-url` are not supported
(refused when reached).

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
Seven plugins are emulated and checked against the
real ones: `symfony/runtime`, `composer/installers` (versions
2.0.0–2.3.0, frameworks that only use the plugin's path table — WordPress
and Drupal included), `pestphp/pest-plugin` (`vendor/pest-plugins.json`),
`dealerdirect/phpcodesniffer-composer-installer` (PHP_CodeSniffer's
`installed_paths`), `phpstan/extension-installer` and
`rector/extension-installer` (their `GeneratedConfig.php`), and
`wikimedia/composer-merge-plugin` (the included manifests merged into the
root — autoload, requirements, links, `extra` — for `install` and
`dump-autoload`; with one deliberate difference: on a bare vendor the
plugin would run an implicit `composer update` of the merged requirements
and rewrite `composer.lock`, vivacity never does — a lock that misses a
merged requirement is handed to Composer instead, and `update` on such a
project is refused). A plugin listed as `false` in `allow-plugins` is
skipped, like Composer does. A short list of plugins that do nothing at install
time (`symfony/flex`, `php-http/discovery`, `phpstan/extension-installer`,
…) is installed as plain libraries. `drupal/core-composer-scaffold` is not
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
naming, source-only packages, a `path` package on Windows, a plugin
upgrade in progress — is detected
before `vendor/` is touched, and vivacity execs the real `composer install`
instead (`--no-fallback` to make it fail).

The resolution commands have the same policy. Composer loads the
installed plugins before resolving, and some change what gets resolved
or written — `symfony/flex` filters the pool against
`extra.symfony.require` and applies recipes, `wikimedia/composer-merge-plugin`
merges other manifests into the root, `drupal/core-recipe-unpack` edits
composer.json on `require`. When one of those (or a plugin outside
vivacity's lists) is installed and allowed, `update`, `require` and
`remove` hand the whole command to Composer with the same arguments,
before any write, and say why; `--no-fallback` stops with exit 3
instead. Plugins with no resolution-time listener (`composer/installers`,
`symfony/runtime`, the extension-installers, `php-http/discovery`,
`symfony/thanks`, `cweagans/composer-patches`…) do not trigger it.

`symfony/flex`'s pool filter is emulated for `update` and `remove` with
`--no-install`: the index of its endpoints (`extra.symfony.endpoint`,
`SYMFONY_ENDPOINT`, the two recipe indexes by default; cached in Flex's
own format under Composer's cache) prunes the pool against
`extra.symfony.require` / `SYMFONY_REQUIRE` exactly as
`PackageFilter::removeLegacyPackages` does (`Restricting packages listed
in "symfony/symfony" to …`), with `COMPOSER_PREFER_DEV_OVER_PRERELEASE`
as Flex sets it; the lock is Composer's with Flex active (checked by
`harness/update.sh` on the Symfony demo and Sylius). What Flex writes
besides the lock stays Composer's: with install (recipes, `symfony.lock`),
on `require` (aliases), a `.env.dist` to copy, a `package.json` or
`importmap.php` to synchronise, a `symfony-pack` to unpack.

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
vivacity = { version = "0.16", default-features = false, features = ["rustls-tls-no-provider"] }
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
