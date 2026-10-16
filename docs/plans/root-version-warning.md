# L'avertissement « could not detect the root package version » — plan

> Niveau 2 : le contrat est la parité à l'octet de stderr avec Composer
> 2.10.3, sur toutes les commandes (`install`, `update`, `require`,
> `remove`, `dump-autoload`) ; la ligne touche aussi les chemins de
> délégation (fallback). Méta-analyse et revue par sous-agents frais.

## Le problème

Quand composer.json n'a pas de `version`, que `COMPOSER_ROOT_VERSION` est
vide ou absente et que `VersionGuesser::guessVersion` ne trouve rien (pas de
dépôt git, dépôt sans commit, HEAD détaché « from » sans tag…), Composer écrit
sur stderr, sauf pour une racine sans `name` (`__root__`) ou de type
`project` :

```
Composer could not detect the root package (acme/lib) version, defaulting to '1.0.0'. See https://getcomposer.org/root-version
```

vivacity n'écrit jamais cette ligne. Les harnais la neutralisent en posant
`COMPOSER_ROOT_VERSION` (36 occurrences dans `harness/*.sh`) ;
`docs/plans/v0.21-verbosity.md` la range parmi les écarts connus. Une
bibliothèque développée hors git (archive, CI qui copie les sources, dépôt
fraîchement initialisé) voit donc une sortie différente de celle de Composer.

## Références Composer 2.10.3

Lues dans le phar du harnais (`/opt/homebrew/Cellar/composer/2.10.3`,
extrait hors dépôt ; `docs/reference/RootPackageLoader.php` est identique à
blancs près — `stripWhitespace` garde les numéros de ligne).

- `Package/Loader/RootPackageLoader.php:79` `load` ; `:85-89` nom absent →
  `__root__`, nom invalide → exception **avant** l'avertissement ;
  `:91-117` la version : `isset($config['version'])` (une `version: null`
  compterait comme absente, mais le schéma la refuse avant, voir la matrice ;
  vivacity, qui ne porte pas le schéma, la refuse dans `load_with_version` :
  pas de ligne non plus — revue),
  `:95` `Platform::getEnv('COMPOSER_ROOT_VERSION')` (vrai au sens PHP : `""`
  et `"0"` ne comptent pas), `:98` `guessVersion`, `:106-114`
  l'avertissement si `$this->io !== null && name !== '__root__' &&
  'project' !== ($config['type'] ?? '')` (comparaison exacte, sensible à la
  casse), `:115-116` `1.0.0` et `$autoVersioned` ; `:146`
  `1.0.0+no-version-set` ; `:169` auto-exigence et `:179` nom de lien
  invalide : exceptions **après** l'avertissement.
- `IO/BaseIO.php:250-253` `warning` → `log(WARNING)` ; `:296-297`
  `writeError('<warning>…</warning>')`, niveau par défaut `NORMAL`
  (`IO/ConsoleIO.php:142`, `:194` sur la sortie d'erreur) : stderr, muet
  sous `-q`, présent sous `-v`.
- `Factory.php:281` `createComposer` ; `:303` composer.json absent et
  `:313` schéma `LAX_SCHEMA` : exceptions **avant** le chargement (pas de
  ligne) ; `:393-394` `loadRootPackage(…, $io)` puis `load` ;
  `Console/Application.php:629` (`getComposer` : `Factory::create` avec
  l'IO de la console), mis en cache dans `$this->composer`.
- `Command/BaseCommand.php:240` `initialize` appelle `tryComposer` **avant**
  les avertissements `COMPOSER_IGNORE_PLATFORM_REQS` (`:280`) et
  `COMPOSER_IGNORE_PLATFORM_REQ` (`:289`) : la ligne de version vient avant
  eux, et avant tout ce que `execute` imprime (`InstallCommand.php:99-100`,
  le refus de `--no-install` ; Symfony valide les arguments après
  `initialize`, donc aussi avant « Not enough arguments »).
- `Command/RequireCommand.php:328` « has been updated », `:330` retour
  `--no-update`, `:409-410` `doUpdate` : `resetComposer` +
  `requireComposer` → **seconde** création, donc seconde ligne, avant
  `:459` « Running composer update ». Même chose dans
  `Command/RemoveCommand.php:210` « has been updated », `:216-222`
  (`tryComposer`, `resetComposer`, `requireComposer`), `:239` `COMMAND`,
  `:275` « Running composer update ». `--dry-run` passe par le même
  chemin (deux lignes).
- `InstallCommand.php:105`, `UpdateCommand.php:136`,
  `DumpAutoloadCommand.php:57`, `RunScriptCommand.php:120` :
  `requireComposer`, déjà créé par `initialize` (cache) — une ligne.
- `Console/Application.php:267`, `:396` : avec un alias (`i`, `u`,
  `dumpautoload`…) ou `run-script`, et un script non-événement dans
  composer.json, le Composer est créé dans `doRun`, avant `initialize` ; la
  ligne sort une fois (cache), toujours en tête.
- `Package/Version/VersionGuesser.php:72-105` `guessVersion` (git, hg,
  fossil, svn) ; `:151-155` la regex de la branche courante (`(no branch)`,
  `(detached from X)`, `(HEAD detached at X)` — pas `(HEAD detached from
  X)`) ; `Util/Git.php:598-623` `cleanEnv` (`LANGUAGE=C`, seulement).
- `wikimedia/composer-merge-plugin` (`docs/reference/plugins/…`) ne fusionne
  jamais `version` dans la racine (`ExtraPackage.php:136-137` ne touche que
  les fichiers inclus) et fusionne après `load` : sans effet sur la ligne.
  Sa mise à jour implicite (`MergePlugin.php:362`, `Factory::create`) en
  imprimerait une seconde ; vivacity ne la fait jamais (fallback).

## Matrice mesurée sur Composer 2.10.3

Projet sans réseau (`packagist.org: false`, deux métapaquets d'un dépôt
`package`, `name: acme/lib`, un script `hello`), hors de tout dépôt git
(`GIT_CEILING_DIRECTORIES`), `COMPOSER_ROOT_VERSION` non posée sauf
mention. « n » = nombre de lignes sur stderr ; jamais sur stdout.

| commande / condition | n | position |
|---|---|---|
| `install` | 1 | 1re ligne |
| `update` | 1 | 1re ligne |
| `dump-autoload` | 1 | 1re ligne (avant « Generating autoload files » sur stdout) |
| `run-script hello` | 1 | 1re ligne |
| `require acme/meta2:1.0.0` | **2** | 1re ligne ; puis entre « has been updated » et « Running composer update » |
| `remove acme/meta2` | **2** | idem |
| `require --dry-run`, `remove --dry-run` | 2 | idem |
| `require --no-update`, `remove --no-update` | 1 | 1re ligne |
| `install -q`, `--quiet`, `SHELL_VERBOSITY=-1` | 0 | — |
| `install -v` | 1 | 1re ligne |
| `type: "project"` | 0 | — |
| `type: "library"`, `"composer-plugin"`, absent | 1 | — |
| `type: "Project"` | — | refusé par le schéma (exit 1, pas de ligne) |
| `name` absent | 0 | — |
| `version: "2.0.0"` | 0 | — |
| `version: ""` / `null` | — | refusés par le schéma (exit 1, pas de ligne) |
| `COMPOSER_ROOT_VERSION=1.2.3` | 0 | — |
| `COMPOSER_ROOT_VERSION=not-a-version` | 0 | exit 1 (version illisible) |
| `COMPOSER_ROOT_VERSION=""` ou `0` | 1 | — |
| dépôt git sans commit | 1 | — |
| git sur `main` | 0 | — |
| HEAD détaché « at » (sans tag, avec tag) | 0 | `dev-<sha>` / le tag |
| HEAD détaché « from » sans tag | 1 | — |
| HEAD détaché « from » avec tag exact | 0 | le tag |
| `--working-dir=p`, `-d p` | 1 | 1re ligne |
| `COMPOSER_IGNORE_PLATFORM_REQS=1` (`install`, `require`) | 1 (2) | **avant** l'avertissement de la variable |
| `COMPOSER_IGNORE_PLATFORM_REQ=ext-foo` (`update`) | 1 | avant l'avertissement |
| `install --no-install` | 1 | avant « Invalid option » (exit 1) |
| `remove` sans argument | 1 | avant « Not enough arguments » |
| `update --with=acme/meta` (sans contrainte) | 1 | avant l'exception |
| `update acme/meta` sans lock (exit 3) | 1 | avant le refus |
| `install` sans lock | 1 | avant « No composer.lock file present » |
| auto-exigence / nom de lien invalide (`install`, `dump-autoload`) | **3** | puis l'encadré (exit 1) — créations ratées d'`initialize`, d'`execute`, et de `hintCommonErrors` (`Application.php:538`) |
| alias `dumpautoload`, `i` (avec et sans script) | 1 | 1re ligne |

La ligne vient avant tout ce que la commande imprime. Ce qui peut la
précéder chez Composer, et que vivacity n'imprime pas (non-objectifs) :
l'avertissement root/sudo et celui de Xdebug (`Application::doRun`), « A
script named install would override… » avec un alias et un script de ce nom
(mesuré par la méta), `disable-tls` (`Factory.php:651`, avant `load`), et
« dubious ownership » écrit par `guessGitVersion` lui-même (`Git.php:49-57`,
réaliste en conteneur). `git` parle anglais des deux côtés (`cleanEnv` pose
`LANGUAGE=C`, `root_version::git` `LANGUAGE=C` et `LC_ALL=C`) — le
« (HEAD détachée depuis …) » d'un git francophone ne change rien.

## Ce qui imprime, et quand vivacity rend la main

vivacity a deux sortes de délégation à Composer :

1. **précoces**, décidées avant les lignes ordinaires de la commande :
   `-vv`/`-vvv` et `-v` avec un plugin actif (`run`, avant `dispatch`) ; le
   plugin de fusion d'`install` (`merge_plugin`, avant l'en-tête, mais
   après l'avertissement `COMPOSER_IGNORE_PLATFORM_REQ(S)` s'il y en a un) ;
   `resolution_fallback` d'`update`/`require`/`remove` (avant toute ligne
   d'`update` ; avant l'édition pour `require`/`remove`, mais après
   l'avertissement `--no-suggest` de `require`) ;
2. **tardives**, décidées après des lignes déjà imprimées : la portée
   d'`install` (`native_layout`, après l'en-tête, « Verifying… » et
   « Package operations »), la résolution rendue après coup (Flex,
   `hand_resolution_over`, après « Loading composer repositories… »), la
   phase d'installation d'un `update`/`require`/`remove` rendue à
   `composer install` (`after_update_line`).

Les tardives impriment déjà en double (`docs/plans/fallback-argv.md`,
« Trouvés en lisant ») ; la ligne de version n'y échappera pas : vivacity
l'a imprimée en tête, le Composer lancé l'imprime à son tour. Les précoces
ne doivent **pas** l'imprimer : Composer le fera, une fois — sauf quand une
ligne de vivacity est déjà sortie avant elles (variable d'environnement,
`--no-suggest`) : la ligne de version est alors sortie avec elle, et
Composer répète les deux.

Règle retenue : **la ligne est la première d'une commande — imprimée par
vivacity dès que vivacity écrit quoi que ce soit, abandonnée si la commande
est rendue à Composer avant cela.** Mécanisme : un tampon de lignes
« retenues » dans `vivacity_core::io`, que toutes les macros d'écriture
vident avant d'écrire (une seule source de vérité, l'ordre est garanti
quelle que soit la première ligne de la commande — refus, avertissement de
variable d'environnement, erreur), et que la délégation jette avant de lancer
Composer. Sous `--no-fallback` (exit 3, Composer jamais lancé), la ligne
sort avant les raisons de vivacity : c'est vivacity qui parle — sauf pour
`-vv` et `-v` avec un plugin, refusés dans `run` avant que la commande ne
lise le projet (rien n'est encore retenu). `--check-scope` imprime déjà le
récit d'`install` (en-tête, « Verifying… », opérations) : la ligne vient
avec lui.

Écarté : imprimer la ligne directement au début de chaque commande. Plus
simple, mais chaque délégation précoce l'imprimerait deux fois, et les
raisons d'un fallback (« vivacity: … outside what vivacity handles… »)
viendraient après elle — la règle « pas avant de rendre la main » ne serait
tenue nulle part.

Écarté : décider la portée d'`install` avant l'en-tête pour supprimer le
double des délégations tardives. Changement de l'ordre des décisions
(vérification du lock avant portée), hors périmètre ; noté en suite.

## Coût

La décision demande git quand les conditions bon marché sont réunies :
niveau normal ou plus (rien sous `-q`), `name` présent et valide, pas de
`version`, pas de `COMPOSER_ROOT_VERSION`, type autre que `project`. Pas
`guess_version` en entier (la méta l'a montré : une branche de
fonctionnalité y coûte un `git rev-list` par branche parente candidate,
≈ 60 ms sur sept branches) mais un prédicat, `version_guessable` : `git
branch` (≈ 8 ms seul), et `describe` seulement quand aucune ligne courante
ne donne de version. Le chemin de détection de la version
(`root_version::detect`) relance git plus loin : un appel de plus dans ce
seul cas. Aucun cas du banc (`bench/ci-bench.sh` : laravel, symfony,
sylius, tous `project`) n'est touché. Partager le résultat avec `detect`
demanderait de faire passer la version devinée dans `install`, la session
de résolution et `dump`, ou un cache de processus qu'un embarqueur garderait
d'une run à l'autre : écarté, mesure à l'appui (Résultat).

## Fichiers, dans l'ordre

1. `harness/verbosity.sh` : préparation `@unset:NOM` (variable retirée des
   deux côtés pour le cas, dans le sous-shell de la commande — l'export
   global de `COMPOSER_ROOT_VERSION=1.0.0` reste pour les autres cas) ;
   les cas rouges (ci-dessous), joués sur le binaire de `main` (aa5f018).
2. `harness/fallback-args.sh` : section `g` (le faux `composer` n'imprime
   rien : on compte les lignes de vivacity seule).
3. `crates/vivacity-core/src/io.rs` :
   - `hold(level, line)` : retient une ligne de stderr (une seule,
     `Option<String>`) si le niveau de la run la montre ;
   - `release_held()` : l'écrit (sous verrou, une fois) ; appelé par
     `say!`, `say_always!`, `say_verbose!`, `out!` avant d'écrire, et par un
     nouveau `say_at!(level, …)` pour les trois `eprintln!` à niveau
     calculé (`update.rs` : `print_session_error`, les événements retenus
     de `SolveReport` ; `session.rs` : `say_event`) ;
   - `drop_held()` : l'abandonne ;
   - `Scope::drop` l'abandonne aussi (une run suivante d'un embarqueur
     repart vide). Garde-fou `AtomicBool` : le cas courant (rien de
     retenu) ne prend pas de verrou.
4. `crates/vivacity-core/src/root_version.rs` : `version_guessable(project)`
   (le prédicat du coût) ; l'analyse de la ligne courante de `git branch`
   sortie de `guess_version` en `current_branch`, partagée par les deux.
5. `crates/vivacity-resolver/src/root.rs` : `undetected_version_warning(
   manifest, project) -> Option<String>` — le port des conditions de
   `RootPackageLoader::load:85-114` : objet, `name` chaîne sans erreur de
   nommage (`package_naming_error`, comme `manifest_error`), `version`
   absente (clé absente : une `null` est refusée avant), pas de variable, `type` ≠ `"project"`, puis
   `version_guessable` faux. Tests unitaires (sans git, dépôt sans commit,
   sur `main`, HEAD détaché « at » puis « from », avec tag, chaque
   condition ; git sans configuration globale).
6. `crates/vivacity/src/manifest.rs` : `hold_root_version_warning(project)
   -> Option<String>` — rien sous `-q` (git n'est pas lancé), rien quand
   `COMPOSER=` désigne un autre fichier (refusé par vivacity ; Composer ne
   lirait pas composer.json), rien quand composer.json est absent, illisible
   ou invalide (Composer échoue avant `load`) ; sinon retient la ligne et la
   rend (pour la seconde de `require`/`remove`).
7. Appels, chacun en **première instruction qui peut écrire** :
   `install.rs` (`run_install`, hors `after_update()`, avant
   `read_manifests` qui écrit les avertissements de variables) ;
   `update.rs` (`run_update`, après `refuse_missing_manifest`, avant
   l'analyse des paquets) ; `require.rs` (`run_require`, avant
   `--no-suggest`) ; `remove.rs` (`run_remove`, avant « Not enough
   arguments ») ; `dump.rs` (`run_dump`). Seconde ligne :
   `require.rs` après le retour `--no-update`, avant « Running composer
   update » ; `remove.rs` après le retour `--no-update`, avant
   `bamarni_command_event`.
8. `crates/vivacity/src/fallback.rs` : `drop_held()` en **première
   instruction** de `hand_over` (avant sa ligne « delegating to ») et,
   quand Composer sera lancé (`!stops`), avant les lignes de raison de
   `delegate_resolution` et `fallback_or_fail` (`delegate_for_verbosity`
   passe par `hand_over`, avant que rien ne soit retenu).
9. `crates/vivacity/src/scripts.rs` : `release_held()` avant de lancer
   `composer run-script` (la ligne de vivacity avant la sortie du script).
10. `crates/vivacity/src/lib.rs` : `release_held()` après `dispatch` (une
    commande qui n'a rien écrit d'autre imprime quand même la ligne).
11. Docs dans le même commit : `CHANGELOG.md`, `DECISIONS.md`,
    `HANDOVER.md`, `docs/plans/v0.21-verbosity.md`, commentaires de
    `verbosity.sh` (la neutralisation globale reste, justifiée autrement),
    ce plan.

## Cas de harnais (rouges sur `main`, verts après)

`verbosity.sh` (stdout et stderr entiers, niveaux normal, `-q`, `-v` sauf
mention ; « L » = `@jq:.type="library" @unset:COMPOSER_ROOT_VERSION`,
laravel rendu bibliothèque, copie sans dépôt git sous le plafond) :

| cas | Composer | `main` |
|---|---|---|
| `install --dry-run` L | 1 ligne (0 sous -q) | **rouge** (normal, -v) |
| `install` L `@vendor` (no-op réel, fil de vérification) | 1 | **rouge** |
| `install --dry-run` L `@env:COMPOSER_IGNORE_PLATFORM_REQS=1` | ligne puis avertissement | **rouge** |
| `install --no-install` L | ligne puis refus, exit 1 | **rouge** |
| `update --dry-run --no-install` L | 1 | **rouge** |
| `require symfony/uid --dry-run --no-install` L | 2 | **rouge** |
| `require symfony/uid --no-update` L | 1 | **rouge** |
| `remove laravel/tinker --dry-run --no-install` L | 2 | **rouge** |
| `dump-autoload` L `@vendor` | 1 (stderr), stdout inchangé | **rouge** |
| `install --dry-run` L `@levels:-vv` (délégation) | 1, imprimée par Composer seul | vert (garde-fou) |
| `install --dry-run @unset:COMPOSER_ROOT_VERSION` (`project`) | 0 | vert (témoin) |
| `install --dry-run @jq:del(.name)\|.type="library" @unset:…` | 0 | vert (témoin) |
| `install --dry-run @jq:.version="1.0.0"\|.type="library" @unset:…` | 0 | vert (témoin) |
| `solver-conflict` `update --no-install @nolock @unset:…` (pas de type) | 1, puis le conflit, exit 2 | **rouge** |
| `install --dry-run` L sauf `@env:COMPOSER_ROOT_VERSION=0` (méta) | 1 | **rouge** |
| `install --dry-run` L sauf `@env:COMPOSER_ROOT_VERSION=` (méta) | 1 | **rouge** |
| `path-repos` `update --no-install @unset:…` (dépôt git sur `develop`, méta) | 0 | vert (témoin) |

`@env` rétablit désormais la valeur d'avant le cas au lieu de faire `unset`
(méta : un `@env:COMPOSER_ROOT_VERSION=…` aurait retiré l'export global à
tous les cas suivants).

`fallback-args.sh`, section `g` (faux `composer`, sans
`COMPOSER_ROOT_VERSION`, projets bibliothèques nommés) — vivacity ne doit
écrire **aucune** ligne avant une délégation précoce, et en écrit une
avant une tardive :

- `g-update`, `g-require`, `g-remove` (plugin inconnu installé,
  `resolution_fallback`) : 0 ligne de vivacity (sur `main` : 0 aussi —
  garde-fou de la règle) ;
- `g-merge-dry-run` (plugin de fusion, fichier inclus absent) : 0 ;
- `g-install-vv` (`-vv`) : 0 ;
- `g-install-late` (`preferred-install: source`, portée) : 1 — écart connu,
  Composer en imprime une seconde ; **rouge** sur `main` (0) ;
- `g-update-no-fallback`, `g-install-no-fallback` : 1, avant les raisons
  (exit 3) ; **rouges** ;
- `g-install-native` (même projet, `install --dry-run` natif) : 1 —
  **rouge** ;
- (méta) `g-merge-env` (plugin de fusion avec
  `COMPOSER_IGNORE_PLATFORM_REQS=1`) et `g-require-no-suggest` : 1 — la
  ligne de vivacity sortie avant la délégation précoce l'entraîne (écart
  connu) ; **rouges** ;
- (méta) `g-run-scripts` : `install --run-scripts`, `pre-install-cmd`
  confié au faux `composer`, qui écrit un marqueur sur stderr et sort en 9 :
  la ligne de version **avant** le marqueur ; **rouge**.

Vu rouge aussi le garde-fou : sans `drop_held` dans `fallback.rs` et sans
`release_held` dans `scripts.rs`, `g-update`, `g-require`, `g-remove`,
`g-merge-dry-run` et l'ordre de `g-run-scripts` échouent.

Assertions de cohérence côté Composer : chaque cas « L » vérifie que la
sortie de Composer contient la ligne autant de fois qu'annoncé (un cas qui
ne montre plus ce qu'il teste échoue).

## Critère « vert »

- Les rouges ci-dessus verts, les témoins toujours verts.
- `cargo fmt --all -- --check` ; `cargo clippy --workspace --all-targets
  -- -D warnings` (macOS et `--target x86_64-pc-windows-gnu`) ;
  `cargo test --workspace` (le compte de `main` plus les nouveaux tests).
- Les 29 passages de harnais de `.github/workflows/ci.yml`, chacun à 0, et
  `harness/root-manifest.sh`.

## Non-objectifs (vus, laissés)

- Les doublons des délégations tardives (ci-dessus) et de la phase
  d'installation rendue à `composer install`.
- `--run-scripts` : chaque `composer run-script` lancé imprime la ligne à
  son tour (processus Composer à part entière) ; poser
  `COMPOSER_ROOT_VERSION` pour lui changerait l'environnement des scripts.
- Le manifeste invalide **après** l'avertissement (auto-exigence, nom de
  lien) : Composer imprime la ligne trois fois (créations ratées
  d'`initialize`, d'`execute` et de `hintCommonErrors`), vivacity une fois ; l'encadré
  diffère déjà (`root-manifest.sh` compare les mots du message).
- Les refus du schéma `LAX_SCHEMA` (non porté) : vivacity imprimerait la
  ligne là où Composer s'arrête avant (`type: "Project"`).
- hg/fossil/svn (non portés, comme pour la version elle-même).
- Le composer.json **global** (méta, mesuré) : sans composer.json de
  projet, `initialize` crée le Composer global (`Factory::createGlobal`,
  `fullLoad`), qui écrit la ligne pour `COMPOSER_HOME/composer.json` s'il a
  un `name` ; vivacity n'écrit que « could not find a composer.json ».
- Ce qui précède la ligne chez Composer sans être porté (root/sudo, Xdebug,
  script qui masquerait une commande, `disable-tls`, « dubious
  ownership »).
- `guess_version` s'arrête quand `git branch` échoue, là où Composer tente
  encore `describe` (méta, nit) : préexistant, sans effet hors d'un dépôt
  (les deux échouent) ; le prédicat suit `guess_version`, pour que la ligne
  dise exactement quand vivacity retient `1.0.0`.
- Les traces `VIVACITY_TRACE` contournent le mécanisme (diagnostic).
- Les avertissements `COMPOSER_IGNORE_PLATFORM_REQ(S)` d'`update`,
  `require` et `remove`, imprimés tard par vivacity (suite déjà notée) :
  la ligne de version reste avant eux.

## Méta-analyse (sous-agent frais, sur le code, 2026-10-16) — et ce qui change

Le relecteur a lu le plan, le code touché et les sources de Composer, et
mesuré sur le vrai `composer` (trois expériences hors dépôt). Pas de
bloquant ; un majeur, des mineurs et des nits. Ce qui en est fait :

- **Le coût était sous-estimé** (majeur) : `guess_version` en entier lance,
  sur une branche de fonctionnalité, un `git rev-list` par branche parente
  candidate (mesuré par lui : ≈ 60 ms sur sept branches) ; et il tournait
  sous `-q`. **Adopté** : le prédicat `version_guessable` (un appel quand
  HEAD est sur une branche ou détaché « at », `describe` sinon), rien sous
  `-q`, mesures avant/après sur `main` et sur une branche de
  fonctionnalité (Résultat).
- **Délégations « précoces » précédées d'une ligne** (mineur) : la variable
  `COMPOSER_IGNORE_PLATFORM_REQS` avant le plugin de fusion d'`install`,
  `--no-suggest` avant `resolution_fallback` de `require`. **Adopté** :
  taxonomie corrigée, cas `g-merge-env` et `g-require-no-suggest` (1 ligne,
  écart connu).
- **Place de `drop_held` dans `hand_over`** (mineur) : **adopté**, première
  instruction (avant « delegating to ») ; vu rouge par mutation.
- **`COMPOSER=autre.json` dans `remove`** (mineur) : la retenue passe avant
  le refus ; **adopté**, `hold_root_version_warning` ne fait rien quand la
  variable désigne un autre fichier.
- **`--no-fallback` avec `-vv`** (mineur) : exception **documentée**
  (refusé avant la lecture du projet, aucune ligne).
- **composer.json global** (mineur, mesuré) : **non-objectif**.
- **Explication des trois lignes** (nit) : **corrigée**
  (`hintCommonErrors`).
- **« Rien ne précède la ligne »** faux (nit) : **corrigé**, la liste de ce
  qui peut la précéder chez Composer passe en non-objectifs.
- **`cleanEnv` ne pose que `LANGUAGE`** (nit) : **corrigé**.
- **Aucun cas de bout en bout avec git** (mineur) : **adopté**, cas
  `path-repos` (témoin, 0 ligne).
- **`@env` fait `unset` après le cas** (mineur) : **adopté**, la valeur
  d'avant est rétablie ; cas `COMPOSER_ROOT_VERSION=0` et vide ajoutés.
- **`release_held` avant `run-script` sans test** (mineur) : **adopté**,
  faux `composer` qui écrit un marqueur, cas `g-run-scripts`, vu rouge par
  mutation.
- **Traces `VIVACITY_TRACE`** (nit) : **documenté** (module `io`).
- **Test unitaire et configuration git globale, « detached at »** (nit) :
  **adopté** (`GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM`,
  assertion ajoutée).
- **État global, `Vec` de lignes** (nit) : **adopté**, `Option<String>`,
  limite de l'embarqueur concurrent écrite à côté de celle du niveau.
- **Colonne « rouge sur main » de la section g** (nit) : **corrigée** ;
  `--check-scope` imprime déjà le récit d'`install`, la ligne vient avec
  lui (décidé, écrit plus haut).

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-16) — et ce qui a changé

Une première relecture s'est arrêtée sur une commande qui n'est jamais
revenue, sans rendre de constats ; une seconde, bornée, a relu le diff et le
code autour, et comparé la branche au vrai `composer` et au binaire de
`main` (ordre des lignes sur `install`, `install --no-install`, `remove` sans
argument, `require`, `remove --dry-run`, `--no-update`, `dump-autoload` ;
niveaux `-q`, `-v`, `SHELL_VERBOSITY=±1` ; sept états git — dépôt sans
commit, `main`, branche de fonctionnalité, détaché « from » avec et sans
tags, tag non normalisable, branche orpheline — comptes identiques des deux
côtés). Pas de bloquant ni de majeur. Ce qui en est fait :

- **`version: null`** (mineur) : la ligne sortait, puis vivacity
  s'arrêtait sur « has no version defined » ; Composer refuse par le schéma,
  sans ligne. **Corrigé** : la clé présente coupe la ligne (le test
  l'exige), comme `load_with_version` qui la refuse.
- **Section « Résultat » annoncée mais absente** (mineur) : **ajoutée**
  ci-dessous.
- **Lancement de Composer qui échoue après `drop_held`** (mineur) : la ligne
  est perdue avant « cannot run composer ». **Documenté** (marginal : un
  `composer` trouvé mais non exécutable).
- **CHANGELOG : `git describe` oublié dans le coût** (nit) : **corrigé**.
- **Plan : les trois lignes, `delegate_for_verbosity`, `VersionGuesser`
  72-105** (nits) : **corrigés**.
- **`remove` retenait la ligne après le refus `--minimal-changes`**, à
  l'inverse de `require` (nit) : **aligné**, première instruction.
- **Commentaire « A held line » de `session.rs`**, ambigu avec `io::hold`
  (nit) : **corrigé**.
- **Pas de cas `SHELL_VERBOSITY`** (nit) : vérifié à la main par le
  relecteur, laissé (le calcul du niveau a ses propres cas dans
  `verbosity.sh`).

Vu en passant, hors périmètre : `dump-autoload` sans `vendor/` compte 1472
classes chez vivacity (branche comme `main`) contre 1 chez Composer.

La première relecture est finalement revenue après la fusion dans `main`
(4b68941) : ni bloquant ni majeur, stderr identique à Composer sur ses
essais (`install`, `update`, `require`, `remove`, `dump-autoload`, niveaux
`-q`/`-v`, `--dry-run`, `--no-install`, `--no-update`). Ses constats, notés
ici après coup :

- **hg, svn, fossil** (mineur) : `VersionGuesser.php:84-104` les essaie
  après git avant d'avertir ; vivacity ne regarde que git. Dans un tel
  dépôt, Composer devine une version et se tait, vivacity avertit — et
  prenait déjà `1.0.0+no-version-set` (HANDOVER, « Version du root
  package »). L'écart devient visible sur stderr. Suite possible : rendre la
  main quand un `.hg`, `.svn` ou `_FOSSIL_` est trouvé en remontant.
- **Schéma LAX** (mineur) : `Factory` valide le manifeste avant
  `RootPackageLoader::load` ; un manifeste refusé (`type: "Project"`) n'a
  pas de ligne chez Composer, vivacity l'écrit puis refuse. Déjà dans les
  non-objectifs (le schéma n'est pas porté).
- **Erreurs d'usage** (nit) : `Application::doRun` crée le `Composer` avant
  de valider la ligne de commande, la ligne précède l'encadré « option does
  not exist » ; le rendu clap de vivacity divergeait déjà entièrement.
- **git tourne deux fois** (nit) : `version_guessable` puis
  `guess_version` — les ≈ 5 ms du Coût, déjà listées comme suite.
- **Test `undetected_version_warning_conditions`** (nit) : panique sans
  git au lieu de sauter, et `git init -b` exige git ≥ 2.28 — comme le test
  existant de `root_version.rs`.
- **Commentaire de `current_branch`** (nit) : il acceptait un peu plus que
  la regex de `VersionGuesser.php:151` (hexadécimal majuscule, sha en fin
  de ligne, formes que git n'émet pas) ; **adouci**.
- **Hors diff, non comparé à `main`** : `require acme/b` sans contrainte sur
  un dépôt `path` réussit chez Composer (« Using version ^1.0 ») et sort en
  1 chez vivacity (« Could not find a matching version of package acme/b »).
  À comparer avec `main`, lot suivant.

## Résultat (2026-10-16)

**Rouges sur `main`** (binaire d'aa5f018) : `verbosity.sh laravel
solver-conflict` 78 OK, 20 FAIL — les dix cas rouges du tableau, au niveau
normal et sous `-v`, chacun pour la seule ligne absente (`1d0 < Composer
could not detect…`, et `3d1` en plus pour `require`/`remove`) ; sous `-q`,
les témoins et `-vv` : verts. Les cas ajoutés après la méta
(`COMPOSER_ROOT_VERSION=0` et vide) sont rouges de la même façon.
`fallback-args.sh` : les 41 appels conformes, 8 assertions g en échec
(`g-update-no-fallback`, `g-install-late`, `g-install-no-fallback`,
`g-install-native`, `g-merge-env`, `g-require-no-suggest`, `g-run-scripts`
deux fois — le compte, puis l'ordre).

**Vu rouge, le mécanisme lui-même** (mutation) : sans `drop_held` dans
`fallback.rs` et sans `release_held` dans `scripts.rs`, `g-update`,
`g-require`, `g-remove`, `g-merge-dry-run` (1 ligne au lieu de 0) et l'ordre
de `g-run-scripts` échouent.

**Verts** sur le binaire final : `verbosity.sh` 125/125 (76 sur `main` +
49 : seize cas × trois niveaux, `-vv`), `fallback-args.sh` 41/41 et toutes
ses assertions g.

**Portes** : `cargo fmt --all -- --check` ; clippy `-D warnings` macOS et
`x86_64-pc-windows-gnu` ; `cargo test --workspace` 310 réussis, 0 échec
(36 binaires ; 309 sur `main`, plus `undetected_version_warning_conditions`).
**Harnais** (binaire final, chacun dans son répertoire, supprimé après) : les
29 passages de la CI et `root-manifest.sh` sortent à 0 — diff-vendor (12),
`--with-autoloader` deux fois (12, 12), update (19), steps (270), removal
(3), transitions (13), path-repos (14), vendor-dir (6), merge-plugin (16),
scripts (6), tar-dist (7), yii2-composer (11), bin-plugin (12), wp-core
(5), custom-dirs (4), package-versions (7), flex-install (7), flex-update
(26), download-lines (2), artifact-repo (9), compare-selftest (11),
two-stage (12), transport-exit (5), verbosity (125), fallback-args (41),
root-scan (12), boot (6), drift-reference (129 fichiers identiques),
root-manifest (35) — 720 lignes `OK`, 0 `FAIL`. Le job Windows n'a pas
tourné ici.

**Coût** (hyperfine, 40 runs, no-op `install --no-blocking` d'une copie de
laravel rendue bibliothèque, sans `COMPOSER_ROOT_VERSION`, M4 Max) :

| cas | `main` | branche |
|---|---|---|
| hors git (la ligne sort) | 44,4 ms | 47,0 ms |
| dépôt sur `main` | 45,8 ms | 50,9 ms |
| branche de fonctionnalité (quatre branches candidates) | 80,2 ms | 84,9 ms |
| type `project` (témoin) | 44,4 ms | 44,0 ms |

Avec `guess_version` en entier au lieu du prédicat, le même cas sur `main`
coûtait 49,6 → 53,9 ms (première version, mesurée avant la méta) ; sur une
branche de fonctionnalité, la méta a mesuré ≈ 60 ms de plus.

**Suites proposées** : décider la portée d'`install` avant l'en-tête (plus de
doublon sur la délégation tardive, ni de cette ligne ni des autres) ;
partager le résultat de `git branch` avec la détection de la version (le
dernier appel de trop, ≈ 5 ms) ; imprimer les avertissements de plateforme
d'`update`/`require`/`remove` en tête, juste après cette ligne ; lire le
composer.json global quand le projet n'en a pas.
