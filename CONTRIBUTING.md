# Contributing to vivacity

The most valuable contribution is a `composer.lock` that vivacity gets wrong.
The second most valuable is a `composer.lock` that vivacity refuses (falls back
to Composer) where you think it shouldn't have to.

## Report a lock that breaks (5 minutes)

1. On a project of yours, in two copies of the same checkout:
   ```bash
   composer install --no-plugins --no-scripts   # copy A
   vivacity install                               # copy B
   diff -r A/vendor B/vendor
   ```
2. If there is any difference, or if vivacity printed
   `this lock is outside what vivacity handles natively`, open an issue with
   the **lock parity** template: `composer.json`, `composer.lock`, vivacity's
   output, OS. Private packages: redact URLs/tokens, keep the structure.

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

- **git-only packages** — a lock entry with a `source` and no `dist`
  (`GitDownloader`: mirror clone under `cache/vcs`, checkout into vendor
  with its `.git/`). Three packages in the corpus (ampache, friendica,
  kovah-linkace). The parity contract needs a decision first: a `.git/`
  directory is not byte-deterministic (packs, dated reflogs). `path`
  repositories (0.9) hold the pieces a port reuses: the git-like version
  guesser (`root_version::guess_version`), the reference dump. `tar`
  dists (asset-packagist) are done (plan v0.17).
- **Plugins at resolution time** — `update` / `require` / `remove` hand
  the command to Composer when an installed, allowed plugin changes the
  resolution (`scope::resolution_issues`, plan v0.16 A); `symfony/flex`'s
  pool filter is emulated for `update`/`remove --no-install`
  (`vivacity-resolver::flex_filter`, `vivacity::flex`, plan v0.16 B).
  `wikimedia/composer-merge-plugin`'s merged root (links, stability
  flags, aliases, references) enters the pool (plan v0.16 C;
  `UpdateOptions.merged`). Next: Flex with install (recipes,
  `symfony.lock`) as its own plan; a merged `repositories` section. The emulated plugins (`pest_plugin.rs`, `phpcs_installer.rs`,
  `extension_installers.rs`, `merge_plugin.rs`) are the pattern: vendor
  the writer under `docs/reference/plugins/`, prove it on a fixture and
  the corpus entries.
- **`path` repositories on Windows** — junctions (`Filesystem::junction`,
  `PathDownloader`'s Windows branch); today such a lock goes through the
  Composer fallback there.
- **`--prefer-source` / `--prefer-dist` / `--prefer-install` on `update`,
  `require` and `remove`** — `install` honours them since v0.19 (the ported
  `getPreferredInstallOptions`); the other three reject them, because their
  path re-reads composer.json once the in-memory manifest is gone and
  carrying the effective preference there is plumbing, not a patch.
- **The refusals that come from Composer's JSON schema** — vivacity refuses
  the root manifest's own rules at every command since v0.19 (invalid root
  name, self-require, invalid link name) but does not validate the schema,
  so five of eight realistic invalid manifests are still accepted. What is
  accepted is stated in the README, measured.
- **Modification times of zip dists** — `unzip` restores the archive's DOS
  timestamp, vivacity leaves the install's time (measured, README says so).
  Doing it exactly means converting a DOS timestamp as `unzip` does, local
  wall-clock with the DST rules of that date, hence a timezone database; the
  cheap approximations (current offset, UTC) are wrong for half the year.
- **The root-version warning** — `Composer could not detect the root package
  (<name>) version, defaulting to '1.0.0'`, printed once as the first line
  when the manifest has no `version`, has a name, and its `type` is not
  `project`. Measured, not yet printed.
- **Plugins to emulate natively** — every install-time plugin proven harmless
  (or reproduced exactly, like `symfony/runtime` and `composer/installers`)
  moves a whole ecosystem off the fallback path. See `scope.rs` for the
  lists and `runtime_stub.rs` for the pattern; parity is proven with the
  harness, never assumed. Ports of GPL-licensed plugins are not accepted
  (NOTICE.md).
- **Auth** — `gitlab-token`/`gitlab-oauth`; custom CAs (`SSL_CERT_FILE`) with rustls.
- **Remaining `update` options** — `--minimal-changes`,
  `bump-after-update`, `--patch-only`, `-i`, the audit, `--verbose`
  explanations. (`--with` and the `a/b:^1` shorthand are done, plan
  v0.18.)

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
  reported as blind, not green), `harness/drift-reference.sh` (every
  vendored file under `docs/reference/` identical to the phar).
  `harness/linux.sh` runs the whole chain in a Linux container; the
  Windows parity job runs on `windows-latest` in CI.
- Every ported function has its reference vendored under `docs/reference/`
  with its licence, and a row in NOTICE.md when it comes from a new
  origin. Documentation (README, HANDOVER, DECISIONS, CHANGELOG, the crate
  READMEs, `docs/`) is updated in the same change as the code.

`DECISIONS.md` records why things are the way they are; `HANDOVER.md` lists
what is not covered yet. Read both before a large change — and add to them
with it.
