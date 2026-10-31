# Contributing to vivacity

The most valuable contribution is a `composer.lock` that vivacity gets wrong.
The second most valuable is a `composer.lock` that vivacity refuses (falls back
to Composer) where you think it shouldn't have to.

## Report a lock that breaks

1. Install Composer 2.10.3 and vivacity (a prebuilt binary, see the README:
   no Rust toolchain needed).
2. Work on disposable copies, never on your project itself, and give both
   tools the same starting state: the same files and modes (`cp -Rp`), the
   same path, the same shell (no `COMPOSER_*` variable or Composer config
   changed in between). Plugins write outside `vendor/` too (symfony/flex
   copies `.env` from `.env.dist`, codeception/c3 puts `c3.php` at the
   root), so the first install changes what the second would start from;
   and some write absolute paths (phpstan's and rector's extension
   installers, in `GeneratedConfig.php`), so two copies side by side differ
   by their directory name alone. Composer runs with its plugins active, as
   vivacity emulates them, and without scripts, which vivacity never runs.
   `--no-fallback` makes vivacity stop instead of handing the install to
   Composer: exit code 0 means it installed natively.
   ```bash
   cp -Rp your-project /tmp/try-src && rm -rf /tmp/try-src/vendor
   cp -Rp /tmp/try-src /tmp/try
   (cd /tmp/try && composer install --no-scripts) > composer.log 2>&1
   mv /tmp/try /tmp/try-composer
   cp -Rp /tmp/try-src /tmp/try
   (cd /tmp/try && vivacity install --no-fallback) > vivacity.log 2>&1
   echo "exit code $?" >> vivacity.log
   # contents, files created or deleted on either side
   diff -r /tmp/try-composer /tmp/try > diff.log 2>&1
   # what diff -r does not compare: link targets and executable bits
   for d in /tmp/try-composer /tmp/try; do
     (cd "$d" && { find . -type l -exec sh -c 'printf "%s -> %s\n" "$1" "$(readlink "$1")"' _ {} \;
                   find . -type f -perm -u+x; } | sort) > "$(basename "$d").modes"
   done
   diff try-composer.modes try.modes >> diff.log
   ```
3. Read the end of `vivacity.log` first. `exit code 3` means vivacity
   does not handle this lock natively: it printed
   `vivacity: this lock is outside what vivacity handles natively:` and
   one line per reason, wrote nothing, and there is nothing to compare.
   Without `--no-fallback` it would have handed the install to Composer
   (``vivacity: delegating to `composer install …`…``), and an empty diff
   would then prove nothing.
4. A dangling symlink present in both trees makes `diff -r` print "No such
   file or directory": that is not a difference. If a difference looks
   random, run Composer once more on a third copy made the same way: when
   Composer's two runs differ from each other, the difference is not
   vivacity's (some plugins write the time, the environment or a cache
   path), and the issue should say so.
5. If `diff.log` shows a difference, or if vivacity refused a lock you
   think it should handle, open an issue with the **lock parity** template:
   `composer.json`, `composer.lock`, the three logs, OS. Private packages:
   redact URLs/tokens, keep the structure.

That is enough: the differential harness (`harness/diff-vendor.sh`) turns a
lock into a permanent regression test. Your bug becomes everyone's test.

The same goes for `update`, `require` and `remove`: a `composer.json` (with
or without its lock) on which `composer update --no-install` and
`vivacity update --no-install` write a different `composer.lock`, print a
different explanation of an unsolvable set, or list different operations,
is a case for `harness/steps.sh` — it replays a command through both tools
on a frozen registry snapshot and compares the files, the exit code and
stderr byte for byte.

A real project that behaves differently is a corpus entry:
`tools/corpus-add.sh git <owner/repo>` (a committed lock) or
`tools/corpus-add.sh template <vendor/name>` (a `create-project` template,
lock resolved once) adds it under `fixtures/corpus/`, and
`harness/corpus.sh --only <name>` runs both tools on it. The report in
`docs/corpus/` ranks what keeps projects on the fallback; that ranking is
the roadmap below.

## Bigger pieces, roughly in order of impact

- **Plugins at resolution time** — `update` / `require` / `remove` hand
  the command to Composer when an installed, allowed plugin changes the
  resolution (`scope::resolution_issues`, plan v0.16 A); `symfony/flex`'s
  pool filter is emulated for `update`/`remove --no-install`
  (`vivacity-resolver::flex_filter`, `vivacity::flex`, plan v0.16 B).
  `wikimedia/composer-merge-plugin`'s merged root (links, stability
  flags, aliases, references) enters the pool (plan v0.16 C;
  `UpdateOptions.merged`). Flex's recipes at `update` are
  reproduced for the configurators `flex_recipes.rs` names, any other
  hands the command over. Next: a merged `repositories` section. The emulated plugins (`pest_plugin.rs`, `phpcs_installer.rs`,
  `extension_installers.rs`, `merge_plugin.rs`) are the pattern: vendor
  the writer under `docs/reference/plugins/`, prove it on a fixture and
  the corpus entries.
- **`path` repositories on Windows** — junctions (`Filesystem::junction`,
  `PathDownloader`'s Windows branch); today such a lock goes through the
  Composer fallback there.
- **`--prefer-source` / `--prefer-dist` / `--prefer-install` on `update`,
  `require` and `remove`** — `install` honours them since v0.19 (the ported
  `getPreferredInstallOptions`), and `require` carries them to its install
  phase since the fallback rework (a source install goes to Composer with
  the option); `update` still rejects them — the plumbing now exists
  (`UpdateArgs::prefer_*`, read by the install phase), what is left is to
  expose them as options and validate them before resolving, as `require`
  does. (`remove` has none, as in Composer.)
- **The refusals that come from Composer's JSON schema** — vivacity refuses
  the root manifest's own rules at every command since v0.19 (invalid root
  name, self-require, invalid link name) but does not validate the schema,
  so five of eight realistic invalid manifests are still accepted. What is
  accepted is stated in the README, measured.
- **Windows names, with `unzip` instead of 7-Zip** — the renaming rule in
  `on_disk` is 7-Zip's, measured on the runner (`CON` → `_CON`, a trailing dot or
  space → `_`). Composer prefers 7-Zip on Windows, so that is the common case;
  what Info-ZIP's `unzip` does with the same names on a box without 7-Zip is
  unmeasured, and the test will say so there (it compares against whichever tool
  it finds). If the runner's answer differs from the Windows box you care about
  (long-path opt-in, no 7-Zip), say so in the issue with its output.
- **An entry name that is not UTF-8, on Windows** — on unix the raw bytes are
  used, as `unzip` and `PharData` use them (measured, oracles). A Windows
  filename has to be convertible to UTF-16, so the crate's decoded name is kept
  there, and what 7-Zip does with such a name is unmeasured.
- **Does `7z x -y` recreate a symlink entry as a link?** On Windows Composer
  prefers `7z` (found in `C:\Program Files\7-Zip` as well as on the `PATH`)
  over `unzip`, and vivacity mirrors that discovery — but whether 7-Zip makes a
  link without `-snl` is unmeasured, and the answer decides what a symlink entry
  becomes on a box that has 7-Zip and no `unzip`. It needs a measurement on
  Windows, not a guess; `extract.rs`'s `windows_unzip_tool_present` says so at
  the point where it matters.
- **Modification times of zip dists** — `unzip` restores the archive's DOS
  timestamp, vivacity leaves the install's time (measured, README says so).
  Doing it exactly means converting a DOS timestamp as `unzip` does, local
  wall-clock with the DST rules of that date, hence a timezone database; the
  cheap approximations (current offset, UTC) are wrong for half the year.
- **Plugins to emulate natively** — every install-time plugin proven harmless
  (or reproduced exactly, like `symfony/runtime` and `composer/installers`)
  moves a whole ecosystem off the fallback path. See `scope.rs` for the
  lists and `runtime_stub.rs` for the pattern; parity is proven with the
  harness, never assumed. Ports of GPL-licensed plugins are not accepted
  (NOTICE.md).
- **Auth** — `gitlab-token`/`gitlab-oauth`; custom CAs (`SSL_CERT_FILE`) with rustls.
- **Remaining `update` options** — `--minimal-changes`,
  `bump-after-update`, `--patch-only`, `-i`, the audit. (`--with` and the `a/b:^1` shorthand are done, plan
  v0.18.)

## Where a command lives in `crates/vivacity`

`lib.rs` is the command line only: the clap structures, `run`, the dispatch
and the exit codes. Each command has its module — `install.rs` (and the
plugins emulated while laying out the lock), `update.rs` (resolution and lock,
which `require` and `remove` go through), `remove.rs`, `require.rs`,
`dump.rs` (the autoloader every command ends with). Around them:
`manifest.rs` (the root manifest's refusals and config keys, the merged root
of composer-merge-plugin), `fallback.rs` (handing the command to Composer),
`flex.rs` and `flex_install.rs` (symfony/flex at resolution and at install),
`scripts.rs` (`--run-scripts`).

## Ground rules that keep the project honest

- **Composer is the oracle.** Any behaviour is ported from the pinned Composer
  source (`docs/reference/`, 2.10.3) and checked against the real phar
  (`tests/oracle_*.rs`) or against a real `vendor/` (`harness/`). No porting
  from memory, no "close enough".
- **Never run PHP inside vivacity.** Plugins are emulated from their
  source, never executed; scripts are Composer's — `--run-scripts` only
  hands the declared events to `composer run-script`, off by default.
- **Measure before optimising** (`bench/`), and publish the losing numbers too.
- **Six crates are published**, in dependency order by `cargo publish
  --workspace`: `vivacity-pcre2-sys`, `vivacity-pcre2` (forks of
  BurntSushi's, see their READMEs — nothing else changes there), then
  `vivacity-core`, `vivacity-autoload`, `vivacity-resolver`, `vivacity`.
- Gates before a PR: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` (needs php + composer 2.10.3 on PATH),
  `harness/diff-vendor.sh --with-autoloader`, `harness/update.sh`,
  `harness/steps.sh` (stderr is compared on every case unless `@nostderr`;
  a case whose reference output has no operation or reason line is
  reported as blind, not green), `harness/verbosity.sh` (stdout and stderr
  whole under normal, `-q` and `-v`), `harness/fallback-args.sh` (what the
  fallback hands Composer: argv, directory, environment, exit code — a
  fake `composer` records it), `harness/drift-reference.sh` (every
  vendored file under `docs/reference/` identical to the phar).
  `harness/linux.sh` runs the whole chain in a Linux container; the
  Windows parity job runs on `windows-latest` in CI. CI wraps every harness
  in `harness/net-retry.sh`: a harness that fails with a network error in
  its output (a runner losing its DNS while resolving against Packagist)
  runs once more a minute later; a real difference fails twice. Harness
  scripts stay compatible with bash 3.2, the one macOS ships (no
  associative arrays, no `mapfile`).
- Every ported function has its reference vendored under `docs/reference/`
  with its licence, and a row in NOTICE.md when it comes from a new
  origin. Documentation (README, HANDOVER, DECISIONS, CHANGELOG, the crate
  READMEs, `docs/`) is updated in the same change as the code.

`DECISIONS.md` records why things are the way they are; `HANDOVER.md` lists
what is not covered yet. Read both before a large change — and add to them
with it.
