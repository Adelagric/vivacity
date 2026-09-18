# Changelog

All notable changes to vivacity (named vivace up to 0.5.0). The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [SemVer](https://semver.org/) — the CLI surface and the
byte-identical-output promise are the public API.

## [Unreleased]

### Changed
- **`install` overlaps its one network request with the local work**: the
  filter-summary revalidation (Composer's `loadFilterSummary`) runs on a
  thread while the platform check, the transaction and the scope are
  computed; the join comes before any output or write, and the report
  keeps `Installer::doInstall`'s order (policy, platform, missing
  requirements, operations). ~10 ms hidden on Linux, 13–18 on macOS.
- **The autoloader dump is twice as fast** on a Laravel-sized classmap
  (macOS 61 → 28 ms, Linux `-o` 40 → 19 ms) with byte-identical output:
  the scanned-files set compares path bytes instead of `PathBuf`
  components, `__DIR__` substitution uses a real substring search,
  already-normalised paths are not re-normalised, exclusion regexes and
  literal prefixes are compiled once per pattern.
- **Scans outside the store are cached per file** (project sources, `path`
  packages, a vendor/ the store does not know): key (mtime ns, size) per
  file, directory listed on every scan, so an added, changed or removed
  file is always seen. `harness/root-scan.sh` replays nine such edits
  against Composer. `VIVACITY_NO_CLASSMAP_CACHE=1` disables it with the
  store cache.

## [0.14.0] — 2026-09-18

### Changed
- **The platform probe is cached.** `install` and the resolver ran
  `php assets/platform-probe.php` on every invocation since 0.8 (the
  lock check against the full platform repository), 30–60 ms of a no-op
  install. The result is now cached (`platform-probe.json` in vivacity's
  cache directory) and reused while the php binary (path, mtime, size),
  every ini file PHP loaded or scanned, the ini scan directory, and
  `PHPRC` / `PHP_INI_SCAN_DIR` / `XDEBUG_MODE` / `XDEBUG_CONFIG` are
  unchanged; `VIVACITY_NO_PLATFORM_CACHE=1` bypasses it. A Laravel no-op
  install goes from 163 ms to 84 ms here (M4 Max, APFS).

### Added
- **Performance gate on the vivacity/Composer ratio** (`bench/gate.py`,
  run by `bench.yml` after `bench/ci-bench.sh`): each scenario's
  `vivacity_median / composer_median`, measured in the same job on the
  same runner, is compared to `bench/results/baseline-ratio.json` and
  fails past baseline × 1.15 when the regression is worth at least 5 ms
  in that run's seconds. The ratio cancels the runner's speed, which
  varies tens of percent between identical runs; the baseline is the
  median across several CI runs (`bench/gate.py --merge`). The gate
  skips with a notice until the baseline exists.

### Fixed
- **`install --no-plugins` on a lock naming a plugin outside vivacity's
  lists** (yiisoft/yii2-composer, roots/wordpress-core-installer,
  craftcms/plugin-installer, drupal/core-composer-scaffold…) is now
  installed natively instead of refused or handed to Composer: with the
  flag Composer loads no plugin at all (`PluginManager::registerPackage`
  returns at once) and every plugin is a plain library in `vendor/`.
  `scope::analyze` already passed the plugin regime to the layout but
  classified plugins without it. `harness/diff-vendor.sh` now replays each
  plugin-allowing fixture under `--no-plugins` on both sides and requires
  a native, identical `vendor/` (found through viv's bench corpus, where
  four of ten projects were `n/a` for this reason).

## [0.13.0] — 2026-09-17

### Added
- **`--run-scripts`** on `install` and `dump-autoload`
  (`docs/plans/v0.14-run-scripts.md`): the events the manifest declares
  go to `composer run-script [--no-dev] <event>` at Composer's own
  points — `pre-install-cmd` before the headline and the lock
  validation, `pre-autoload-dump` / `post-autoload-dump` around the dump
  (never with `--no-autoloader`), `post-install-cmd` after the funding
  lines; a non-zero exit stops the run with that code. vivacity embeds
  no PHP: the static callables receive Composer's `Script\Event` because
  Composer runs them. The fallback keeps the scripts too (`composer
  install` without `--no-scripts`). Not carried: the per-package events
  (`pre/post-package-*`, five corpus projects) and the `optimize` flag of
  the autoload events (`run-script` passes none); the plugins' own
  listeners of those events run again through `run-script`. Fixture
  `fixtures/projects/scripts` and `harness/scripts.sh` (log of the
  events with `COMPOSER_DEV_MODE`, callables reading the local
  repository, dev / `--no-dev` / `dump -o`, a failing script, `--no-autoloader`,
  nothing without the flag), in CI. Off by default: the contract
  "vivacity never runs scripts" is unchanged.

## [0.12.0] — 2026-09-17

### Added
- **`wikimedia/composer-merge-plugin` emulated** (v2.0.1–v2.1.0;
  `docs/plans/v0.13-merge-plugin.md`): the manifests named by
  `extra.merge-plugin.include` / `require` (PHP `glob()`, no flags,
  recursive) are merged into the root for `install` and `dump-autoload`
  — `require` / `require-dev` (duplicate rules `ignore-duplicates`,
  `replace`, else Composer's conjunctive constraint with the
  `Intervals::isSubsetOf` short-cuts, structured, not textual),
  `autoload` / `autoload-dev` (`array_merge_recursive`, paths re-based on
  the included file's directory), `conflict` / `replace` / `provide`,
  `extra` with `merge-extra` (shallow or deep, root or include wins),
  `self.version` on every link section, `merge-dev`. The lock is
  validated against the merged requirements exactly as Composer does
  with the plugin installed (exit 4, same lines). **Deliberately not
  reproduced**: on a bare vendor the plugin installs itself, then runs a
  partial `composer update` of the merged requirements against the live
  repositories and rewrites `composer.lock` (measured: three packages
  bumped on a fresh checkout of open-web-analytics); vivacity hands such
  a lock to Composer with the reason instead, and refuses `update` /
  `require` / `remove` on a project that configures the plugin (the
  merged requirements and repositories are not in vivacity's pool yet).
  Fixture `fixtures/projects/merge-plugin`, `harness/merge-plugin.sh`
  (five variants × install, `dump -o`, `dump -a`, `--no-dev`, the two
  missing-requirement cases, the refusal), in CI. `tools/corpus-add.sh`
  keeps the included manifests (they were absent from the corpus
  fixtures, where the plugin was a silent no-op), `harness/corpus.sh`
  builds the steady-state reference for such entries. Corpus: 58/105
  native in dev (was 55), 66/105 in `--no-dev` (was 63), 0 diff
  (`docs/corpus/2026-09-17-v0.12.md`).

- **Fixture `solver-held-branch`**: `minimum-stability: dev`, six `dev-*`
  branches held at their lock entry with their `extra.branch-alias`
  (`composer/installers` `dev-main` ⇐ `^1.0 || ^2.0`, `psr/http-client`
  `dev-master` ⇐ `^1.0` one hop further), three partial updates
  (`psr/log`, `php-http/curl-client -w`, `psr/log -W`) in the pool oracle
  and `harness/update.sh`. Pool, decisions and lock byte-identical to
  Composer without any code change: the `ArrayLoader` port
  (`loader::load_packages`) creates the alias package for every origin,
  the locked repository included, so a held branch never loses its alias
  and its dependants never read "could not be found".

### Fixed
- A real `install` now prints Composer's post-install report — the
  abandoned packages of the lock and the funding count — like a dry run
  and an update already did; `Skipped installation of bin` notices come
  with the operations, before `Generating autoload files`.
- `--no-dev` autoloader: Composer drops the dev packages **by name** when
  the local repository knows them (`Installer::doInstall` sets the lock's
  dev names before the dump) and only falls back to the reachability
  filter (`filterPackageMap`) when it knows none (`dump-autoload` after a
  `--no-dev` install); vivacity always filtered by reachability, which
  differs for a locked package no root requirement reaches.
- `Locker::getMissingRequirementInfo`: the root package is a candidate
  with its `replace` / `provide` links (`RootPackageRepository`), so a
  root requirement satisfied by the root's own replace no longer reads
  "is not present in the lock file" — and when only the root matches by
  name with an unsatisfied constraint, the line reads `is in the lock
  file as "replaced as <c> by <root> <v>"` like Composer's.
- `normalize_pretty`: a `.` before the stability word (`v1.2.3.stable`,
  gymadarasz/ace in the SuiteCRM corpus entry — `installed.json` carried
  the raw string), exactly one separator like `VersionParser`'s
  `[._-]?`, and a bare trailing `.` (`1.2.3.` is `1.2.3.0`); the
  normalisation oracle covers the forms.

## [0.11.1] — 2026-09-17

### Fixed
- **No C compression library in the graph**: `zip` now runs bzip2 on its
  pure-Rust backend (`libbz2-rs-sys`, exports carry their own prefix) and
  LZMA on `lzma-rs`; the `xz` and `zstd` features (C `lzma-sys`,
  `zstd-sys`) are off — Info-ZIP's `unzip`, Composer's extractor, has
  neither. With PCRE2 prefixed, bzip2 was the next duplicate-symbol
  failure for a host linking `libphp.a` (ephpm/ephpm#523, PHP's `ext/bz2`).
  The link test (`crates/pcre2-link-test`) now also defines the foreign
  `BZ2_*` names and round-trips a bzip2 stream. vivacity's binary holds
  exactly one C library: its own prefixed PCRE2 (plus ring's versioned
  symbols with the default TLS feature). Luther Monson's PR #6, opened
  47 minutes after the same change landed on main, adds what it lacked:
  `tools/check-clib-deps.sh` pinning the graph in CI (no `lzma-sys`,
  `zstd-sys`, `libz-sys`; `bzip2-sys` with `__disabled` only;
  `libbz2-rs-sys` with an explicit `semver-prefix`), a bzip2 entry
  extraction test, and the measurement that settles zstd: PHP's Windows
  SDK (`php8embed.lib`) bundles 294 `ZSTD_*` and 33 `BZ2_*`, the Linux SDK
  33 `BZ2_*` and 219 `lzma_*` (`docs/plans/v0.12-pure-rust-extraction.md`).

## [0.11.0] — 2026-09-17

### Added
- **PCRE2 built in with prefixed symbols** (`docs/plans/v0.12-pcre2-prefix.md`):
  two crates of our own, `vivacity-pcre2-sys` (pcre2-sys 0.2.10 with PCRE2
  10.46, always built from the vendored source, every symbol prefixed
  `vivacity_` through a 16-line `PCRE2_SYMBOL_PREFIX` patch, bindings on
  `#[link_name]`) and `vivacity-pcre2` (pcre2 0.2.11, unchanged). A
  program that embeds vivacity and links its own PCRE2 — PHP's `libphp.a`
  bundles one — no longer gets `duplicate symbol: _pcre2_check_escape_8…`
  (ePHPm's `ephpm composer`, ephpm/ephpm#523). Checked by
  `tools/check-pcre2-symbols.sh` (137/137 globals) and a link test against
  a foreign PCRE2 (`crates/pcre2-link-test`), on the three CI platforms.
- **`config.vendor-dir` and `config.bin-dir`** (`docs/plans/v0.11-vendor-dir.md`):
  resolved like `Config::get` — `COMPOSER_VENDOR_DIR` / `COMPOSER_BIN_DIR`,
  then the project's `config`, then the global config; `{$vendor-dir}`
  substituted; trailing slashes and `./` dropped — and followed by every
  derived path: install paths, `installed.json` / `installed.php`
  (the root's `install_path` included), the bin proxies, the autoloader's
  `$vendorDir` / `$baseDir` and its scan exclusion, the emulated plugins'
  files. The bin directory may be the project's own: an existing regular
  file keeps its place with Composer's `Skipped installation of bin …`
  notice, only the proxies of removed or updated packages are unlinked,
  and the directory is removed when a removal empties it
  (`BinaryInstaller::removeBinaries`). Absolute, `..`, `.`, `~/` and
  `$VAR` forms are refused as scope issues. `harness/vendor-dir.sh`: six
  variants on the symfony and wordpress fixtures (install, `dump -o`,
  no-op; whole project compared), in CI on Linux, macOS and Windows.
  Corpus: 55/105 native in dev (was 50), 63/105 in `--no-dev` (was 56),
  0 diff (`docs/corpus/2026-09-16-v0.11.md`).
- `bench/ci-cache.sh` + `ci-cache` workflow (on demand): what a CI cache
  artifact costs, Composer's zip cache against the extracted store, and the
  install each one enables; results and reading in `bench/M7-ci-cache.md`.

### Fixed
- The macOS (arm64) and Linux release binaries no longer depend on a
  system `libpcre2-8` (0.10.0's did: `pcre2-sys` picked Homebrew's /
  Ubuntu's library through pkg-config on the runners, so the macOS binary
  failed to start on a Mac without `brew install pcre2`); `release.yml`
  now asserts it.
- `installed.php` lists the inline aliases of the lock (`"x/y": "dev-branch
  as 1.2.3"`, the lock's `aliases` list) in a package's `aliases`, like
  Composer's `MarkAliasInstalled` does. Found by the corpus once
  joomla-cms (`vendor-dir: libraries/vendor`) became native.
- `harness/update.sh` runs the Drupal case with `--no-security-blocking` on
  both sides: `packages.drupal.org` (the project's own repository, not
  frozen by the snapshot) serves live advisories for `drupal/core`, and
  SA-CORE-2026-013 (2026-09-16) made the locked 11.4.6 blocked — Composer
  then crashes on the snapshot's partial advisories instead of explaining
  the problem, where vivacity prints the explanation. Blocking parity is
  covered by the solver-policies fixtures.

## [0.10.0] — 2026-09-16

### Added
- `harness/lib/compare.sh`: the vendor comparison shared by
  `diff-vendor.sh` and the corpus harness, with a `stat` inventory of
  file modes and link targets that `diff -r` never saw. It found two
  divergences on the six fixtures, both fixed: a package's own binaries
  are now made executable like `BinaryInstaller::installBinaries` does
  (`chmod 0777 & ~umask`), and a symbolic link extracted from a zip gets
  `unzip`'s 0777 mode on macOS.
- `config.vendor-dir`, `bin-dir` and `preferred-install: source` are scope
  issues: `install` hands such a project to Composer instead of laying it
  out differently.
- **`pestphp/pest-plugin` emulated** (v1.0.0 to v5.0.0, one generator):
  `vendor/pest-plugins.json` from the installed packages' `extra.pest.plugins`
  at autoload-dump time, gone with the plugin; seven corpus projects
  became native with their dev packages. Composer's own order in that
  file is the completion order of its parallel extractions — the
  harnesses compare it sorted.
- **`dealerdirect/phpcodesniffer-composer-installer` emulated** (0.7.2 to
  1.2.1): the `ruleset.xml` search in the `phpcodesniffer-standard`
  packages (and in the project when it is one), the paths made relative
  to `squizlabs/php_codesniffer` with `findShortestPath`, sorted and
  written as `installed_paths` into `<phpcs>/CodeSniffer.conf` in phpcs's
  own var_export format; existing entries kept while their directory
  exists. Four corpus projects became native with their dev packages.
  An emulated plugin listed as `false` in `allow-plugins` is skipped,
  like Composer skips the real one.
- **`phpstan/extension-installer` (1.4.3) and `rector/extension-installer`
  (0.11.2) emulated**: `src/GeneratedConfig.php` — the extensions keyed
  by name with their absolute and shortest-relative install paths,
  `extra`, `getFullPrettyVersion()`, phpstan's `NOT_INSTALLED` list and
  the compacted `phpstan/phpstan` constraint (`Intervals::compactConstraint`)
  — written after the transaction like the plugins' `post-install-cmd`
  listeners. Four corpus projects, and the sylius and rector fixtures,
  native again with their dev packages.
- **`BENIGN_PLUGINS` pruned on the corpus's evidence**: `phpstan/extension-installer`,
  `rector/extension-installer` and `dealerdirect/phpcodesniffer-composer-installer`
  write files under a Composer with plugins active; a project locking one
  is handed to Composer until each is emulated. The fixture harnesses now
  run Composer with plugins wherever the manifest allows one, compare the
  whole project and say native or fallback (sylius and rector are handed
  over until the extension installers are emulated); the Composer
  fallback always passes `--no-scripts`; `autoload_runtime.php` is written
  at autoload-dump time, never with `--no-autoloader`.
- **The corpus** (`fixtures/corpus/`, 106 real projects; `harness/corpus.sh`;
  `tools/corpus-add.sh`; `tools/corpus-report.py`; `vivacity install
  --check-scope`): the measured share of real locks `install` lays out
  natively, against Composer 2.10.3 `--no-scripts` with its plugins active,
  in both dev modes, with the fallback reasons ranked. Report in
  `docs/corpus/`.
- Found by the corpus and fixed: `install`'s platform check now runs on
  the full platform repository (`lib-*` libraries, `composer-runtime-api`,
  `config.platform`, provide/replace links); `version_normalized` keeps
  digit runs as written (`2026.04.1.0`, `RC01`) and `dev-master` as
  `dev-master` (composer/semver 3); dist urls with `%prettyVersion%` and
  the other `ComposerMirror` placeholders are expanded; `autoload_runtime.php`
  comes from the installed `symfony/runtime`'s own template with
  `extra.runtime` substituted (older runtimes, custom options — no
  fallback any more); a `symfony-pack` is installed nowhere when Flex is
  active; `apcu-autoloader` writes `setApcuPrefix` (random prefix like
  Composer's, ignored by the comparison).

## [0.9.0] — 2026-09-16

### Added
- **Selectable TLS backend** for embedders: `rustls-tls-ring` (default —
  the standalone binary is unchanged, ring bundled) and
  `rustls-tls-no-provider` (the rustls stack with no crypto provider, for
  a host that installs its own process-wide default, e.g. one that links
  aws-lc-rs). Exposed on `vivacity`, `vivacity-core`, `vivacity-resolver`
  and `vivacity-autoload`; internal crate edges take `vivacity-core` with
  `default-features = false` so an embedder's opt-out is not re-added by
  feature union.
- **`path` repositories** (`{"type": "path", "url": "packages/*"}`):
  `update`, `require` and `remove` read them — glob and brace patterns in
  libc order, `~`/`$VAR` expansion, the dist reference `sha1(json .
  serialize(options))` or the HEAD commit of the package's own git
  repository (`reference: auto`), `reference: none`, the version taken
  from `options.versions`, the package's `version`, `COMPOSER_ROOT_VERSION`
  (when the package and the project share a HEAD), the git branch (a
  feature branch and its parent become two packages; a package without a
  repository takes the project's branch) or `dev-main`; `transport-options`
  with `relative` and `symlink` — and `install` lays them out: a symbolic
  link (relative through `findShortestPath`, or absolute with `relative:
  false`) or a mirror (`symlink: false`, `COMPOSER_MIRROR_PATH_REPOS`)
  filtered like `ArchivableFilesFinder` (VCS directories at any depth, the
  root `.gitattributes` `export-ignore` rules, links to non-empty
  directories, dangling or outward links dropped, empty directories kept,
  modes and mtimes of Symfony's `copy`). Windows keeps the Composer
  fallback for such locks.
- A real `install` prints Composer's operation lines with the downloader's
  appendix (`: Extracting archive`, `: Symlinking from …`, `: Mirroring
  from …`, `: Source already present`) after the transaction, then
  `Generating autoload files`; the `vivacity:` summary line stays.
- An update or a removal creates `vendor/bin` even without binaries
  (`BinaryInstaller::removeBinaries`), as Composer does.
- Fixture `path-repos` (`fixtures/path-repos.sh` materialises nested git
  repositories with pinned hashes, links, modes, an empty directory, the
  project on `develop`), `harness/path-repos.sh` (seven steps: symlink and
  mirror installs, no-op reinstall, update after editing a package,
  remove, require of the parent branch, links turned into mirrors then
  into absolute links by changing the repository options, a reinstall
  after a link was deleted by hand — stderr, lock and `vendor/` with a
  `stat` inventory of modes and link targets and the mirror mtime
  invariant), 20 `steps.sh` cases (229 cases, 206 with stderr
  byte-identical), the fixture in `update.sh`.
- The git version guesser no longer pins `GIT_DIR`: git walks up to the
  enclosing repository exactly as Composer's `git branch` does;
  `non-feature-branches` entries are regex alternatives like Composer's;
  `(HEAD detached from X)` yields no version, like Composer's regex.
- installed.json, installed.php and the autoloader are produced from the
  local repository (the previous installed.json entry for an unchanged
  package, the lock's for an installed or updated one), as
  `InstalledFilesystemRepository::write` and `AutoloadGenerator::dump`
  do; a package listed in installed.json whose directory is gone is
  purged first (`Factory::purgePackages`).

### Changed
- `harness/lib/fixture.sh` stages fixtures and sets the git environment
  of every harness (`GIT_CEILING_DIRECTORIES` = parent of the project,
  `GIT_CONFIG_GLOBAL=/dev/null`, `LC_ALL=C`); the global config no longer
  injects an empty snapshot repository; `@pkgedit:` prep;
  `@env:COMPOSER_ROOT_VERSION` reaches both tools.
- References vendored: `PathRepository`, `Platform`, `PathDownloader`,
  `FileDownloader`, the Archiver filters, Symfony `Filesystem` and Finder
  `Glob` (MIT, NOTICE row).
- **Faster warm paths** (Luther Monson, [#4](https://github.com/Adelagric/vivacity/pull/4)):
  the classmap scan and the store→vendor materialization fan out where
  parallel I/O pays (Linux: sylius cold scan 550 → 340 ms, `install` with
  `vendor/` wiped 806 → 374 ms in the parity container; `VIVACITY_PARALLEL_IO`
  overrides the per-platform default — APFS stays sequential, where the
  same fan-out was measured 20 % slower), the class detection runs on a
  rayon pool everywhere (macOS scan 768 → 697 ms), generated and state
  files are written only when their bytes change (Composer's
  `filePutContentsIfModified`), Linux uses a `FICLONE` reflink before the
  hardlink/copy fallback, zip extraction memoizes created directories, and
  the per-store-entry classmap cache switches to a length-prefixed binary
  encoding (v2; older caches are ignored). `vendor/bin` proxies follow
  `Installer::run`'s `ensureBinariesPresence`: a missing proxy of an
  unchanged package is recreated, an existing one is left alone.

## [0.8.0] — 2026-09-15

### Added
- **`--dry-run` on `update`, `require` and `remove`**: the resolution runs
  and everything Composer prints is printed, nothing is written — the
  root package is patched in memory for `require`/`remove` (Composer's
  `array_merge` order, its mixed-case `remove` quirk included), a
  `composer.json` created for the run is deleted afterwards, and the
  install phase lists its operations from the unwritten lock.
- **Composer's update output**: `  - Locking x (v)` / `Upgrading x (a =>
  b)` / `Downgrading` / `Removing` lines after the `Lock file operations`
  summary (removals first, then by name; dev packages show their
  reference), the `N package suggestions were added by new dependencies`
  line, `Package x is abandoned …` warnings, the funding line; `install`
  prints `Installing dependencies from lock file (including
  require-dev)`, `Verifying lock file contents can be installed on
  current platform.`, `Package operations: …` / `Nothing to install,
  update or remove` and, in a dry run, the `  - Installing …` lines in
  transaction order. Policy warnings lose their `Warning:` prefix.
- `harness/steps.sh` compares stderr on every case by default (from the
  first operation or headline to the end; `@nostderr` opts out), and
  `@dry-install` exercises the install phase of a dry run: 209 cases,
  187 with stderr byte-identical.
- `install` checks the lock against the root requirements like
  `Locker::getMissingRequirementInfo` (a hand-edited `composer.json`
  exits 4 with Composer's lines), and installed.json entries whose
  install path is gone count as absent (`Factory::purgePackages`).

### Fixed
- `require` did not restore `composer.json`/`composer.lock` when the
  install phase (not the resolution) failed; Composer reverts on any
  non-zero status.

## [0.7.0] — 2026-09-15

### Added
- **Composer's explanation of an unsolvable set.** `vivacity update`,
  `require` and `remove` now print what Composer prints when no set of
  packages satisfies the requirements: the numbered problems with their
  reasons (`Root composer.json requires … -> satisfiable by …`, `… conflicts
  with …`, `… could not be found in any version`, `… does not match your
  minimum-stability`, locked-package and platform-package variants, the
  providers of a missing extension or library), the hints (`Potential
  causes`, the php.ini list and `--ignore-platform-req` advice for a
  missing extension, the `-W` advice when the lock is the cause), the
  `--no-dev` warning and the dev-extraction failure. `Problem`,
  `Rule::getPrettyString` and `SolverProblemsException` are ported, with
  `RepositorySet::findPackages`/`getProviders`, `IniHelper::getAll`, and
  the pool's bookkeeping of the versions removed by the optimizer and the
  policies. `harness/steps.sh` gains `@stderr`: Composer's stderr from
  the headline to the end is compared byte for byte, on 50 cases
  (`fixtures/make-solver-problems.py` builds a registry that reaches every
  reason branch the CLI can produce). `install` on a lock blocked by a
  policy prints the same structure.
- **Windows binaries and installers**: the release ships
  `vivacity-<tag>-x86_64-pc-windows-msvc.tar.gz` (with `vivacity.exe`),
  `install.sh` handles Git Bash/MSYS2, a new `install.ps1` handles
  PowerShell (5.1 and 7), and the GitHub Action installs on
  `windows-latest`. Both installers are exercised in CI against a
  package built from the commit (`tools/package-release.sh`, the same
  script the release uses); a `workflow_dispatch` dry run of the release
  builds every target without publishing. `bin-compat` now also reads
  the global `COMPOSER_HOME/config.json` layer, and `COMPOSER_BIN_COMPAT`
  set to `""` or `"0"` falls through like Composer's `?:`.

### Fixed
- `install.sh` failed on Linux with "vivacity: Not found in archive": the
  release archives up to 0.6.0 stored their members with a `./` prefix,
  which GNU tar does not match against a bare name (bsdtar on macOS
  does). New archives have no prefix and the script accepts both.
- **Windows support** (Luther Monson, [#2](https://github.com/Adelagric/vivacity/pull/2)):
  `vendor/bin` `.bat` proxies written exactly when Composer writes them
  (`bin-compat` resolved from `COMPOSER_BIN_COMPAT`, then
  `config.bin-compat`, default `auto` = `full` on Windows/WSL only), with
  `determineBinaryCaller` and `installFullBinaries` ported to the byte;
  canonical paths without the `\\?\` verbatim prefix; Composer's
  `%APPDATA%`/`%LOCALAPPDATA%` cache directories; zip symlink entries
  extracted as plain files like `ZipArchive`; `.gitattributes` forcing LF.
  A `windows-latest` job now runs the six-fixture parity harness against
  Composer 2.10.3 — 0 diff, with and without the autoloader.

## [0.6.0] — 2026-09-14

### Changed
- **Renamed to vivacity.** Another Rust reimplementation of Composer,
  `svandragt/vivace`, predates this project by four days under the same
  name with the same byte-identical promise; sharing the name helps no one.
  Everything follows: the binary is `vivacity`, the crates are `vivacity`,
  `vivacity-core`, `vivacity-resolver` and `vivacity-autoload`, the library
  entry point is `vivacity::run`, the environment variables are
  `VIVACITY_*` (`VIVACE_*` is no longer read), the cache lives under
  `~/.cache/vivacity` / `~/Library/Caches/vivacity` (a `vivace` cache is
  simply left behind), the action is `Adelagric/vivacity`, the repository
  redirects from its old name. The `vivace*` crates are deleted from
  crates.io, which frees the name for the older project; nothing else
  changes in behaviour.
- All module docs, comments, error messages and `--help` texts are in
  English.

### Removed
- **`drupal/core-composer-scaffold` emulation.** It was a port of the
  plugin's source, which is GPL-2.0-or-later — a derivative work that
  cannot be distributed under MIT/Apache-2.0 with the rest of the crates
  and binaries (see NOTICE.md). The plugin is now handled like any other
  unknown plugin: `install` delegates to `composer install` before
  touching `vendor/`, and `dump-autoload` exits with code 3 while the
  plugin is locked and allowed, because Composer would run its
  `pre-autoload-dump` listener. The vendored source under
  `docs/reference/drupal-scaffold/` is removed with it. Releases 0.3.0 to
  0.5.0 contained it; the only one published on crates.io (0.5.0) is
  deleted.

## [0.5.0] — 2026-09-14

### Added
- **`vivace require`**: a port of `RequireCommand` — `vendor/name`,
  `vendor/name:^1.0`, `vendor/name ^1.0`, `--dev`, `--fixed`,
  `--no-update`, `--no-install`, `-w`/`-W`, `--sort-packages`,
  `--prefer-stable`/`--prefer-lowest`, `--update-no-dev`, the platform
  filters; a package given without constraint gets the version
  `VersionSelector` picks, the same partial update runs, then the
  constraint is rewritten from the locked version and the lock's
  `content-hash` and `stability-flags` are updated in place
  (`Locker::updateHash`); `composer.json` and `composer.lock` are restored
  (or deleted when just created) when the resolution fails. Not
  supported: `--dry-run`, `--minimal-changes`, the interactive prompts,
  the "Did you mean" search, installing from a virtual lock
  (`config.lock: false` without `--no-install`).
- **`vivace remove`**: a port of `RemoveCommand` — `composer.json` is
  edited in place (names matched case-insensitively and by `vendor/*`
  patterns, `--dev`, a package found in the other section is reported and
  left alone as in non-interactive Composer), `allow-plugins` entries of
  removed plugins are dropped, then the same partial update as Composer
  runs (`-W`, `--no-update-with-dependencies`, `--no-update`,
  `--no-install`, `--update-no-dev`, `--unused`), `composer.json` is
  restored when it fails, and exit code 2 is returned when the package is
  still installed. Not supported: `--dry-run`, `--minimal-changes`,
  `COMPOSER=other.json`.
- **Partial updates**: `vivace update vendor/name [-w|-W]` (patterns,
  `COMPOSER_WITH_DEPENDENCIES`/`COMPOSER_WITH_ALL_DEPENDENCIES`), with
  Composer's warnings; `update lock|nothing|mirrors` and temporary
  constraints are refused.
- **Dependency policies** (Composer 2.10): the pool filters that remove
  versions covered by a security advisory (`policy.advisories`, default
  on), versions on a filter list — Packagist's malware list
  (`policy.malware`, default on, `block-scope`) — and abandoned packages
  (`policy.abandoned`, default off), with the legacy `config.audit`
  keys, the global/project merge, `COMPOSER_POLICY`,
  `COMPOSER_POLICY_*_BLOCK`, `COMPOSER_NO_BLOCKING` and `--no-blocking`;
  `ignore`/`ignore-id`/`ignore-severity`/`ignore-source` rules, the
  `ignore-unreachable` behaviour, and the security-advisories API call
  when a rule needs complete advisories. A locked version flagged by a
  list is a resolution problem (exit 2) as in Composer, and `install`
  refuses a lock that pins one, with Composer's message. Until now vivace
  behaved as if `--no-blocking` were always set. `install --dry-run` runs
  the checks without writing anything; `install --no-install` is refused
  as in Composer.
- `update` keeps a metadata cache in Composer's own `cache-repo-dir`, in
  Composer's layout and byte format, revalidated with `If-Modified-Since`;
  a cache written by Composer is read by vivace and vice versa. When the
  network fails and the cache has a dated copy, the copy is used with a
  warning, like Composer's degraded mode. Metadata files of a pool batch
  are fetched in parallel (12 at a time), as `loadAsyncPackages` does.
- Ports checked against the phar on their own: `JsonManipulator` (12 102
  editing scenarios over 794 real manifests plus 725 synthetic layouts)
  and `VersionSelector` (902 names over the five snapshots, platform and
  stability variants).
- The `vivace` crate is a library with a thin binary: `vivace::run(args)`
  runs any command in-process and returns the exit code, so another
  program can embed the commands instead of shelling out. Crate metadata
  is ready for crates.io.
- `update`, `require` and `remove` honour `COMPOSER_IGNORE_PLATFORM_REQS`
  and `COMPOSER_IGNORE_PLATFORM_REQ`, as `BaseCommand` does.
- Verification: `harness/steps.sh` plays `require`, `remove`, `update`
  and `install` cases through Composer and vivace on the frozen snapshots
  and compares `composer.json`, `composer.lock` and the exit code (128
  cases, in CI); the resolver oracle and the harnesses now run with the
  policies on (5 fixtures, pools reduced by 2-23 %, locks unchanged) plus
  the synthetic `solver-policies` fixture.

### Changed
- `update` exits with code 2 when the requirements cannot be resolved,
  Composer's `ERROR_DEPENDENCY_RESOLUTION_FAILED`, instead of 1.
- `update` and `require` keep the indentation of an existing
  `composer.lock` when rewriting it, as `JsonFile::write` does.
- `install` now reads the repositories' `packages.json` (and, for the
  malware list, Packagist's summary) before installing, as Composer 2.10
  does; unreachable repositories are ignored with a warning by default.

### Fixed
- The JSON encoder now escapes U+2028 and U+2029 as `json_encode` does
  without `JSON_UNESCAPED_LINE_TERMINATORS`; a package description
  containing either would have produced a `composer.lock` differing from
  Composer's by those two characters.
- `config.allow-plugins` rules from the project now take precedence over
  the global ones in the order Composer merges them; a global `*` rule
  could win over a project rule for the same package.

## [0.4.0] — 2026-09-12

### Added
- **`vivace update`**: dependency resolution by a line-by-line port of
  Composer 2.10.3's resolver — semver (`composer/semver`), `ComposerRepository`
  (Packagist v2 protocol, `~dev` files, inline packages, `available-packages`),
  `PoolBuilder`, `PoolOptimizer`, `RuleSetGenerator`, the CDCL `Solver` with
  its learning and backtracking, `DefaultPolicy`, `Transaction` /
  `LockTransaction`, `extractDevPackages` (the second solve that splits
  `packages-dev`) and `Locker::setLockData` (`ArrayDumper`, content-hash,
  root aliases, platform requirements). The lock it writes is byte-identical
  to Composer's on the five application fixtures and four solver cases,
  from frozen Packagist snapshots; remote `composer` repositories over HTTPS
  are supported. `--no-install`, `--no-dev`, `--prefer-stable`,
  `--prefer-lowest`, `--ignore-platform-reqs`, `--ignore-platform-req`.
  Plain (Satis-style) `composer` repositories without `metadata-url`,
  repository `mirrors` and `options` (written as `dist.mirrors`,
  `source.mirrors`, `transport-options`) are handled like Composer does;
  solved packages go through the same security validation as
  Composer's `ValidatingArrayLoader::validatePackage`.
  Not yet: `require`/`remove`, partial updates, `--with`, `vcs`/`path`
  repositories, Composer's problem messages on an unsolvable set.
- Oracles for the port: `tools/oracle-pool.php` (pool, and with `--solve`
  the solver's decisions read by reflection), frozen snapshots in
  `fixtures/registry/`, `harness/update.sh`.

## [0.3.0] — 2026-09-11

### Added
- **`drupal/core-composer-scaffold` emulated natively**: a stock
  `drupal/recommended-project` installs with vivace alone — scaffolded web
  root files, `web/autoload.php` / `autoload_runtime.php`, `.gitignore`
  management, and the plugin's autoload-time additions (classmap entries,
  `vendor/drupal/DrupalInstalled.php` with its xxh3 hash). Plugin versions
  are recognised by the fingerprint of their source (116 of the 120
  releases from 10.3.0 to 12.0.0-alpha1). Checked against the real plugin on
  15 synthetic projects (whole trees compared) and on the whole Drupal fixture.
- `drupal/core-project-message` and `drupal/core-recipe-unpack` classified
  as inert at install time (installed as plain libraries).
- Frozen fixture `drupal`; `harness/transitions.sh` (a plugin upgrade in
  progress is handed to Composer before anything is written).

### Fixed
- `installed.json` no longer carries `installation-source` for
  metapackages (Composer omits it).
- `include_paths.php` (PEAR `include-path`) is written in `installed.json`
  order, the order `composer dump-autoload` produces; `composer install`'s
  own order depends on asynchronous extraction and varies between runs.

## [0.2.0] — 2026-09-10

### Added
- **`composer/installers` emulated natively**: packages placed outside
  `vendor/` (WordPress plugins/themes, Drupal modules, …) when the plugin is
  locked at 2.0.0–2.3.0, allowed by `config.allow-plugins` and its framework
  only uses the location table (58 of 96 frameworks). `installer-paths` (by
  package, `type:`, `vendor:`), `installer-name` and `installer-disable` are
  supported. Everything else falls back to Composer with a message naming
  the reason. Checked against the real plugin on 665 cases and on a whole
  WordPress project in CI.
- **GitHub Action** at the repository root: `uses: Adelagric/vivace@v0.2.0`
  installs the matching release and adds it to `PATH` (with store/dist
  caching).
- **Drift job** (`drift.yml`): weekly run against the latest Composer and the
  snapshot, diffing the vendored reference files first; opens a `drift`
  issue on failure. CI itself is now pinned to Composer 2.10.3.
- `harness/removal.sh`: packages dropped from the lock disappear like with
  Composer, inside and outside `vendor/`.
- Frozen fixtures `rector` (dev-main packages with `default-branch`,
  extension-installer plugins, `platform-check: false`) and `wordpress`.

### Fixed
- `installed.php` listed no `aliases` for locked packages on a default
  branch (`9999999-dev`) or with `extra.branch-alias`; the root package's
  alias pretty version now matches Composer too.
- Archive extraction stripped the first path component unconditionally: a
  zip whose top level is not exactly one directory lost its root files. It
  now follows `ArchiveDownloader`'s single-directory rule.
- `vendor/bin` proxies and `install-path` values are computed with an exact
  port of `Filesystem::findShortestPath` instead of an approximation.
- `install.sh` authenticates the release lookup with `GITHUB_TOKEN` when
  present (anonymous API rate limit on CI runners).

## [0.1.1] — 2026-09-10

### Changed
- CLI and installer messages in English.

## [0.1.0] — 2026-09-10

First public release: `vivace install` from `composer.lock`, byte-identical
`vendor/` (packages, bin proxies, `installed.json`/`installed.php`, full
autoloader) on Laravel, the Symfony demo and Sylius; content-addressed store
with clone/hardlink; per-store-entry classmap cache; fallback to Composer for
anything outside scope.
