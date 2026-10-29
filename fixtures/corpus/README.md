# The corpus

Real PHP projects, reduced to what a `composer install` reads, to measure
how much of the real world `vivacity install` serves natively
(`harness/corpus.sh`, report in `docs/corpus/`). One directory per entry:

- `composer.json` and `composer.lock` as committed by the project (`git`
  entries, pinned by commit) or as `create-project` leaves them with a lock
  resolved **once** by Composer at the date noted (`template` entries —
  application templates never commit a lock);
- the files the root autoload rules name (`files` copied verbatim,
  `psr-4`/`psr-0`/`classmap` directories present but empty: Composer refuses
  to scan a missing directory), nothing else of the project;
- `PROVENANCE`: one line — kind, origin, commit or version, date, Composer;
- `UPSTREAM` (optional, `git` entries): project files the reference needs
  on disk to run as it does in the real project, **not committed** —
  a `# <owner/repo>@<sha>` header, then `<sha256>  <path>` per file.
  `harness/corpus.sh` downloads them at that commit when it measures
  (`raw.githubusercontent.com`, cached under `$WORK/cache/upstream` once
  the sum matches, checked again at every use; a missing or altered file
  makes the entry `unavailable`, named); the offline `--scan` does without
  them. `tools/corpus-upstream.sh <entry> <path>…` writes the list, and
  `tools/corpus-add.sh` rewrites it at the new commit when it refreshes an
  entry. Used by chamilo-lms (2026-10-29): its root `files` script
  `public/legacy.php` requires thirteen legacy files and the `Database`
  class from a `classmap` directory, which phpcs loads through
  `vendor/autoload.php` when dealerdirect registers the standards — without
  them the reference fails ("Failed to set PHP CodeSniffer installed_paths
  Config"). Fifteen files, 903 KB (903,027 bytes), GPL-3.0-or-later: data the harness
  fetches, never code in this repository.

`tools/corpus-add.sh git <owner/repo> [ref]` and
`tools/corpus-add.sh template <vendor/name> [version]` add or refresh an
entry. `excluded.txt` lists what cannot enter (authentication required,
a baseline install that fails deterministically) and why.

The inputs are pinned on purpose: between two runs only the network
changes, and a report is reproducible from the entries it names. Re-pin
when the report is refreshed (the date is in PROVENANCE and in the
report's name). Nothing here is part of vivacity: manifests and locks are
public data published by their projects, and the handful of copied
autoload files keep their projects' licences (chamilo-lms's are fetched
instead, see `UPSTREAM`).
