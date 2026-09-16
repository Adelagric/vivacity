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
- `PROVENANCE`: one line — kind, origin, commit or version, date, Composer.

`tools/corpus-add.sh git <owner/repo> [ref]` and
`tools/corpus-add.sh template <vendor/name> [version]` add or refresh an
entry. `excluded.txt` lists what cannot enter (authentication required,
a baseline install that fails deterministically) and why.

The inputs are pinned on purpose: between two runs only the network
changes, and a report is reproducible from the entries it names. Re-pin
when the report is refreshed (the date is in PROVENANCE and in the
report's name). Nothing here is part of vivacity: manifests and locks are
public data published by their projects, and the handful of copied
autoload files keep their projects' licences.
