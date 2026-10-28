# Paquets sans dist, servis par un dépôt git — plan

> Le corpus du 2026-10-21 (106 projets réels, 76 natifs en `--no-dev`, 0
> diff) rend à Composer six entrées pour « package has no usable dist (no
> zip or tar dist, no path source) ». Composer installe un paquet qui n'a
> qu'une `source` git par `GitDownloader` : un clone, laissé dans vendor/
> **avec son répertoire `.git`**. Niveau 2 : contrat de sortie (vendor/,
> stdout, stderr, code) ; ce qui n'est pas reproduit part à Composer, nommé,
> avant toute écriture. Méta-analyse et revue par sous-agents frais.

## Environnement de mesure

macOS 26 (Darwin 25.6.0, Apple Silicon), PHP 8.5.12, Composer 2.10.3 (le
phar de Homebrew, extrait pour la lecture), git 2.54.0. Variables du banc
(`harness_git_env`) : `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`,
`LC_ALL=C`. Réseau : github.com joignable.

## Références (Composer 2.10.3)

- `Downloader/DownloadManager.php:437-476` `getAvailableSources` : `source`
  avant `dist` ; un paquet sans dist n'a que `source`, quelle que soit la
  préférence. `:417-431` la préférence ne départage que les deux.
- `Downloader/VcsDownloader.php` : `download` → `doDownload` ; `prepare`
  (`install` : `emptyDirectory`) ; `install` écrit
  `  - Installing <nom> (<version>): ` **sans** fin de ligne puis
  `doInstall` ; `prepareUrls` : une URL locale (`Filesystem::isLocalPath`)
  passe par `realpath` (et garde `file://`) ; `getInstallationSource()` =
  `source`.
- `Downloader/GitDownloader.php` :
  - `doDownload` : URL locale → rien ; sinon `cache-vcs-dir/<url assainie>/`
    et, git ≥ 2.3.0 et `Cache::isUsable`, la ligne
    `  - Syncing <nom> (<getFullPrettyVersion>) into cache` puis
    `Git::fetchRefOrSyncMirror` ; succès → `cachedPackages[id][ref]`.
  - `doInstall` : depuis le cache `git clone --no-checkout <cache> <path>
    --dissociate --reference <cache>` (sauf `transport-options.git.
    single_use_clone`), `git remote set-url origin -- <url sans
    identifiants>`, `git remote add composer -- <idem>` ; sans cache
    `git clone --no-checkout -- <url> <path>`, `git remote add composer --
    <url>`, `git fetch composer`, puis les deux `set-url`. Ligne
    `Cloning <getShortHash>[ from cache]`. Puis `updateOriginUrl` (URL
    utilisée ≠ URL du paquet) ou `setPushUrl` (GitHub : `pushurl`
    `git@github.com:x/y.git` quand `github-protocols` contient `ssh`, le
    défaut), puis `updateToCommit`.
  - `updateToCommit:435-510` : `git branch -r` ; référence non hexadécimale
    40 et présente comme `composer/<ref>` → `checkout -B <branche>
    composer/<ref>` + `reset --hard composer/<ref>` ; hexadécimale 40 →
    `checkout <branche>` ou `checkout -B <branche> composer/<branche>`, puis
    `reset --hard <ref>` (branche = version jolie sans `dev-` / `.x-dev`,
    `v` préfixé si seule `composer/v<branche>` existe) ; sinon
    `checkout <ref>` + `reset --hard <ref>`. Ne renvoie jamais de nouvelle
    référence en 2.10.3.
  - `getShortHash` : 10 caractères d'une référence hexadécimale 40, la
    référence entière sous `-v`.
  - `cleanChanges`/`getUnpushedChanges`/`getLocalChanges` : une mise à jour
    ou une suppression d'un paquet installé depuis la source interroge le
    clone (modifications locales, commits non poussés, question
    interactive, `discard-changes`).
- `Util/Git.php` : `runCommand` (boucle `github-protocols` pour une URL
  GitHub, authentification sur échec, `prohibitUrlByConfig`) ;
  `syncMirror` (`clone --mirror`, ou `remote set-url origin` + `remote
  update --prune origin` + `gc --auto`, puis l'URL assainie) ;
  `fetchRefOrSyncMirror` (référence absente → synchronisation ; présente,
  hexadécimale 40, et ni branche ni tag au nom de la version → re-
  synchronisation) ; `cleanEnv` (`GIT_TERMINAL_PROMPT=0`, `LANGUAGE=C`,
  `GIT_DIR`, `GIT_WORK_TREE` et `DYLD_LIBRARY_PATH` retirés).
- `Repository/FilesystemRepository.php:369-374` : la `reference`
  d'installed.php est celle de la source quand `installation-source` vaut
  `source`, de la dist sinon, l'autre à défaut.
- `Installer/InstallationManager.php` `downloadAndExecuteBatch` : tous les
  `download` (donc les lignes `Downloading` et `Syncing`) dans l'ordre des
  opérations, avant toute opération.

## Mesures (Composer 2.10.3)

Projet à dépôt `package` (source seule) contre un dépôt nu local, en
chemin absolu (pas de cache) et en `https://` réécrit vers ce dépôt par
`url.<base>.insteadOf` dans la configuration git du banc — Composer le
traite en URL distante (cache, `Syncing`), git le lit sur le disque : le
chemin du cache est mesurable **sans réseau**. Plus deux paquets réels
(github.com, ceux d'ampache et de kovah-linkace).

1. **Lignes.** Distante : `  - Syncing acme/lib (1.0.0) into cache` dans
   la phase de téléchargement, intercalée dans l'ordre des opérations avec
   les `  - Downloading …` des dists (mesuré sur un projet mixte zip +
   git), puis `  - Installing acme/lib (1.0.0): Cloning 7aefa0a840 from
   cache`. Locale : pas de `Syncing`, `…: Cloning 7aefa0a840`. Version dev :
   `(dev-master 514f828)`, référence nommée : `(dev-master master)` et
   `Cloning master`. `-v` : la référence entière ; `-q` : rien. Cache
   froid ou chaud : mêmes lignes. Install sans opération : « Nothing to
   install, update or remove ».
2. **installed.json** : l'entrée du lock plus `"installation-source":
   "source"` à sa place d'`ArrayDumper` ; installed.php : `reference` = la
   référence source (`master` pour une référence nommée).
3. **`.git`.** Deux installs Composer du même lock, à une seconde
   d'écart : `diff -r` des `.git` ne montre que `index` (cache stat :
   mtime, ctime, inode, device) et `logs/` (reflogs : l'horodatage
   `<epoch> <tz>` de chaque ligne). `config`, `HEAD`, `ORIG_HEAD`,
   `FETCH_HEAD`, `packed-refs`, `refs/`, `objects/` (packs compris),
   `hooks/` (gabarits de git), `info/`, `description` : identiques à
   l'octet. Mêmes constats entre cache froid et cache chaud, et entre deux
   caches froids indépendants sur les deux dépôts GitHub réels (packs
   identiques). Le message de reflog du clone porte l'URL clonée — le
   chemin du cache pour l'install depuis le cache. L'arbre de travail : les
   modes suivent l'umask (git), comme pour Composer.
4. **État de `.git` selon le cas** : tag (`1.0.0`) → `HEAD` détachée ;
   `dev-master` + sha → `refs/heads/master` réinitialisée au sha ;
   référence nommée → `master` ; depuis le cache, `git branch -r` ne liste
   que `origin/*` (aucun `composer/*` : pas de fetch), donc une référence
   nommée passe par `checkout master` + `reset --hard master` ; en local,
   `checkout -B master composer/master`. GitHub : `pushurl =
   git@github.com:acme/lib.git` sur `origin`.

## Décision de parité pour `.git`

« Identique à l'octet » vaut pour tout `.git` **sauf trois familles dont
le contenu dépend de l'instant, du système de fichiers ou de
l'ordonnancement des threads de git**, que Composer lui-même ne reproduit
pas d'un run à l'autre :

- `index` : comparé par ce qu'il signifie — `git ls-files --stage` (mode,
  objet, étage, chemin) et `git status --porcelain` des deux côtés ;
- `logs/**` : comparés ligne à ligne après masquage du seul champ
  `<epoch> <fuseau>` ; anciens et nouveaux objets, identité et message
  restent comparés ;
- `objects/` hors `objects/info/` (packs, index de pack, objets libres) :
  comparé par l'**ensemble des objets** (`git cat-file
  --batch-all-objects --batch-check`, trié : nom, type, taille) — ajouté
  après la méta-analyse, qui a montré trois packs différents pour trois
  clones d'un même cache à objets libres (le `repack -a -d` de
  `--dissociate` est multi-thread). `objects/info/` reste comparé à
  l'octet : un `alternates` vers le cache serait une vraie différence.

Tout le reste — fichiers, liste, modes (inventaire `stat` du `.git` hors
`objects/`) — est comparé tel quel. Conséquence : les deux côtés doivent
utiliser un cache VCS **au même chemin** (le message du reflog du clone le
porte) ; le banc donne à chacun le sien à ce chemin, `corpus.sh` partage le
même. `harness/lib/compare.sh` cesse d'ignorer un `.git` situé **sous
vendor/** (les autres, dont ceux des dépôts `path` du banc, restent
exclus) ; `harness/compare-selftest.sh` prouve qu'une différence de `HEAD`,
de `config`, d'objet ou de reflog hors horodatage le fait échouer.

Ce n'est atteignable que par construction : vivacity lance **le même
binaire git avec la même suite de commandes, les mêmes arguments et le
même environnement** que `GitDownloader`. Une bibliothèque git en Rust
écrirait un autre `.git` (packs, config, reflogs) : écartée.

## Classement des six entrées (scan du 2026-10-28, 0.24.0)

| entrée | paquet | nature | autres blocages |
|---|---|---|---|
| ampache | krixon/xbmc-php-rpc | **git seul** (dépôt `package`, github, référence nommée `master`) | aucun |
| kovah-linkace | shaarli/netscape-bookmark-parser | **git seul** (dépôt `git`, github, sha) | aucun |
| friendica | friendica/json-ld | **git seul** (dépôt `vcs`, git.friendi.ca, tag 1.1.5) | oomphinc/composer-installers-extender (plugin de disposition) |
| mautic | mautic/core-lib | **`path`** (`app`) absent de la fixture | composer-patches, package-versions 1.11.99.1, installeur `mautic` |
| drupal | drupal/core-composer-scaffold (+3) | **`path`** (`composer/Plugin/*`) absent de la fixture | plugins drupal non connus, tbachert/spi, core-project-message |
| omeka-s | omeka/composer-addon-installer | **`path`** (`application/data/…`) absent de la fixture | plugin non connu |

Les trois `path` ne sont pas des sources git : la fixture du corpus ne
garde que l'arbre minimal de l'autoload racine, le répertoire du dépôt
`path` n'y est pas. Le message les confondait ; il les nommera désormais
(« path package … whose directory … does not exist »). Seules ampache et
kovah-linkace peuvent devenir natives.

## Le modèle porté

1. **Décision par paquet** (`vivacity_core::git_source`, appelée par
   `scope::classify_package` quand le paquet n'a **aucune** dist) : natif
   seulement si toutes ces conditions tiennent, sinon un motif nommé
   (« package X: git source not installed natively (<raison>) ») :
   - Linux/macOS (`cfg!(unix)`, comme `path`) ; `source.type` = `git` ;
     une référence, et la branche que `updateToCommit` dérive de la
     version, faites de `[A-Za-z0-9._/-]`, sans `-` initial ; pas de
     `source.mirrors` ni de `transport-options.git.single_use_clone` ;
   - binaire `git` présent, version ≥ 2.3.0 (`getVersion`, seuil du cache
     et de `--dissociate`) ;
   - URL `https://` sans identifiants, ou URL locale (`isLocalPath`) dont
     `realpath` existe ; tout autre schéma (`http`, `git`, `ssh`, `git@…`)
     part à Composer (`prohibitUrlByConfig`, avertissements, protocoles) ;
   - URL distante : cache VCS utilisable (ni `COMPOSER_CACHE_VCS_DIR`, ni
     `cache-dir`/`cache-vcs-dir` dans la configuration — vivacity ne les
     lit pas —, un chemin absolu sans `~` ni `$`, pas `/dev/null`), réseau permis (ni `--offline` ni
     `COMPOSER_DISABLE_NETWORK`), et pour un domaine GitHub ni
     `github-protocols` ni `github-domains` configurés (la boucle de
     protocoles n'essaie alors que l'URL elle-même en premier).
2. **Garde d'état** (`scope::analyze`, donc vue par `--check-scope`) : un
   paquet qu'installed.json dit `installation-source: source` et que la
   transaction met à jour ou retire, ou un paquet git-seul que la
   transaction met à jour depuis une installation existante → refus
   `Unsupported` nommé, donc Composer (sa logique `cleanChanges` interroge
   le clone et peut poser une question). Seule l'installation **neuve**
   d'un paquet source est native ; un paquet source inchangé reste intact.
3. **Phase réseau, avant toute écriture** : pour chaque paquet source à
   installer, dans l'ordre du lock, `fetchRefOrSyncMirror` porté
   (`rev-parse --git-dir`, `rev-parse --quiet --verify <ref>^{commit}`,
   `branch`, `tag`, `clone --mirror` ou `remote update --prune` + `gc
   --auto`, `remote set-url` encadrant). Échec, ou référence absente après
   synchronisation → `Unsupported` → Composer rejoue et produit sa propre
   erreur. Source locale : la référence est vérifiée dans le dépôt source
   (`rev-parse --verify`) avant toute écriture, pour la même raison.
4. **Préparation puis placement** : la suite exacte de `doInstall` +
   `updateToCommit` tourne avant la première suppression, dans un
   répertoire de travail sous vendor/ ; un échec l'efface et rend la main
   à Composer ; le placement n'est qu'un `rename` ( aucun chemin de destination n'est écrit
   dans `.git`, vérifié : ni la config ni les reflogs ne le portent).
   Arguments en tableau (jamais de shell), `--` devant chaque URL comme
   Composer, environnement de `cleanEnv`. Aucune récursion de sous-module,
   aucun crochet, aucune config lue du dépôt cloné : ce que fait Composer,
   puisque ce sont les mêmes commandes.
5. **Lignes** : `download_lines` intercale `  - Syncing …` (URL distante)
   avec les `Downloading` dans l'ordre des opérations ; `operation_lines`
   ajoute `: Cloning <court>[ from cache]` (`-v` : référence entière).
6. **État** : `installation-source: source` pour ces paquets ;
   installed.php prend la référence selon `installation-source`
   (`FilesystemRepository:369-374`) — la règle actuelle (dist sinon
   source) n'en diffère que pour un paquet installé depuis la source qui a
   aussi une dist.
7. **`preferred-install`** : un paquet sans dist s'installe depuis la
   source quelle que soit la préférence, un paquet sans source depuis sa
   dist : `config_issues` ne compte plus que les paquets qui ont les deux.
   **`preferred-install: source` / `--prefer-source` restent rendus à
   Composer** : ce ne serait pas le même travail — tous les paquets de
   Packagist ont une source GitHub et une dist, les passer en source c'est
   cloner l'écosystème (boucle d'authentification GitHub, `mirrors`, repli
   source→dist sur échec avec ses avertissements) ; à mesurer à part.

## Méta-analyse (sous-agent frais) et ce qui change

Le réfutateur a reçu le cadrage, ce plan et l'accès au code ; dix failles,
toutes examinées :

1. **Les packs ne sont pas reproductibles** (critique, reproduite par lui) :
   un cache aux objets **non empaquetés** — ce que laisse un `remote update`
   sous `transfer.unpackLimit` — donne à chaque `clone --dissociate` un pack
   différent (`repack -a -d` multi-thread : trois noms de pack pour trois
   clones du même cache). La mesure n'avait vu que des caches fraîchement
   clonés, tout en packs. **Corrigé** : `objects/` sort de la comparaison à
   l'octet ; on compare l'ensemble des objets (`git cat-file
   --batch-all-objects --batch-check`, trié) et `objects/info/` à l'octet
   (un `alternates` resté vers le cache serait une vraie différence). Cas
   d'auto-test : un cache à objets libres.
2. **Le clone au placement** (majeur) : après les suppressions et en
   parallèle, un échec de checkout laisserait vendor/ à moitié réécrit sans
   repli possible. **Corrigé** : clone, remotes, `pushurl` et
   `updateToCommit` tournent dans une **phase de préparation**, avant la
   première suppression, dans des répertoires de travail sous vendor/ ; un
   échec les efface et rend `Unsupported` (Composer) ; le placement ne fait
   plus qu'un `rename`.
3. **Déclencheur trop large** (majeur) : « dist inutilisable » couvre une
   dist `file`/`xz`/`phar` ou `path` absente, que Composer essaie d'abord
   (avec ses avertissements de repli vers la source). **Corrigé** : natif
   seulement quand il n'y a **aucune** dist (`DistKind::Missing`).
4. **Chemin du cache** (majeur) : `COMPOSER_CACHE_VCS_DIR`, `expandPath`
   (`~`, `$VAR`), chemin relatif — le chemin finit dans les reflogs.
   **Corrigé** : repli quand `COMPOSER_CACHE_VCS_DIR` est posée, quand la
   configuration (projet ou globale) a `cache-dir`/`cache-vcs-dir`, ou quand
   le chemin obtenu n'est pas absolu ou contient `~`/`$`.
5. **Le cache froid de vivacity jamais exercé** (majeur) : avec un cache
   partagé, Composer passe le premier et vivacity trouve un miroir chaud.
   **Corrigé** : chaque côté a son cache **au même chemin** (celui de
   Composer mis de côté avant le passage de vivacity), et les deux miroirs
   sont comparés (refs, config, ensemble d'objets) ; le cas chaud part de
   deux copies identiques d'un même cache.
6. **Garde d'état seulement à l'installation** (mineur) : `--check-scope`
   et le scan du corpus l'ignoreraient. **Corrigé** : calculée dans
   `scope::analyze` (installed.json, purge de `purgePackages` comprise) ;
   seul le réseau reste un refus de l'installeur.
7. **La branche dérivée de la version n'est pas validée** (mineur) :
   `dev---detach` deviendrait une option de `git checkout`. **Corrigé** :
   même jeu de caractères, pas de `-` initial, pour la branche dérivée.
8. **`single_use_clone`** (mineur) : **repli nommé**.
9. **Portée de `compare.sh`** (mineur) : les `.git` des dépôts `path` hors
   vendor/ (path-repos) diffèrent par nature. **Corrigé** : seuls les
   `.git` sous `<vendor>/` sont comparés ; les autres restent exclus.
10. **Cache de classmap sur un clone** (mineur, préexistant) : un paquet
    installé depuis la source par Composer, avec aussi une dist, serait
    « réchauffé » dans le store et sa classmap lue depuis la dist.
    **Corrigé** : pas de réchauffage ni d'association au store pour une
    entrée `installation-source: source`.

Notés aussi et retenus : `process-timeout` (300 s par défaut,
`COMPOSER_PROCESS_TIMEOUT`, config) appliqué à chaque commande git, un
dépassement rendant la main à Composer ; synchronisation dans l'ordre des
opérations plutôt que du lock (sans effet visible, mais c'est l'ordre de
Composer).

## Fichiers

- `crates/vivacity-core/src/git_source.rs` (nouveau) : décision, cache
  VCS, synchronisation, clone, lignes.
- `crates/vivacity-core/src/scope.rs` : `classify_package`, nouveaux
  motifs (`GitSource`, `MissingPathSource`), `config_issues`.
- `crates/vivacity-core/src/installer.rs` : garde d'état, phase réseau,
  placement, `installation-source`.
- `crates/vivacity-core/src/state.rs` : référence d'installed.php.
- `crates/vivacity/src/install.rs` : `download_lines`, `operation_lines`.
- `harness/lib/compare.sh`, `harness/compare-selftest.sh`,
  `harness/git-sources.sh` (nouveau), `.github/workflows/ci.yml`.
- Docs : CHANGELOG, DECISIONS (2026-10-28), HANDOVER, README, ce plan.

## Cas de harnais (`harness/git-sources.sh`, Composer 2.10.3 contre `vivacity --no-fallback`)

Dépôt nu fabriqué par le banc (deux commits, tag `1.0.0`, un fichier
exécutable, un `bin`) ; configuration git du banc avec `insteadOf` pour
`https://git.example.invalid/acme/lib.git` et `https://github.com/acme/
lib.git` ; cache VCS partagé, vidé avant chaque cas ; projet entier
comparé (`compare_vendor`, `.git` compris), stdout et stderr entiers
(ligne `vivacity:` exceptée), code de sortie.

- `local-tag`, `local-dev`, `local-branch` : URL locale, tag / sha / réf.
  nommée.
- `cache-tag`, `cache-dev`, `cache-branch` : URL distante réécrite (cache
  froid), puis `cache-warm` (cache chaud).
- `github` : domaine GitHub (`pushurl`).
- `verbose` (`-v`), `quiet` (`-q`), `no-op` (second install), `mixed`
  (dist zip locale + deux sources : ordre `Downloading`/`Syncing`),
  `no-dev` (source en require-dev), `dump` (`dump-autoload -o` après).
- Replis nommés, Composer rejoue (vendor/ identique après) : `remove`
  (paquet source retiré du lock), `unreachable` (référence absente).
- `real` (réseau) : krixon/xbmc-php-rpc et shaarli/netscape-bookmark-
  parser depuis github.com.

Rouges sur main (8a48734) : tous les cas natifs (code 3 sous
`--no-fallback`). Plus `compare-selftest.sh` : nouveaux cas `.git`.

## Critère « vert »

`cargo fmt --check`, clippy (hôte et `x86_64-pc-windows-gnu`), `cargo
test --workspace` ; `git-sources.sh` à 0 ; tous les bancs de ci.yml et
`root-manifest.sh` à 0 (sauf le défaut connu d'`allow-plugins.sh` sous
macOS) ; `corpus.sh --only ampache,kovah-linkace,friendica,mautic,drupal,
omeka-s` : ampache et kovah-linkace natives en 0 diff dans les deux modes,
les quatre autres en repli avec leurs vrais motifs.

## Non-objectifs

`--prefer-source` / `preferred-install: source` ; mise à jour et
suppression d'un paquet installé depuis la source ; autres VCS (hg, svn,
fossil, perforce) ; Windows ; `http://`, `git://`, ssh ; les URL à
identifiants et l'authentification ; `cache-dir`/`cache-vcs-dir`
configurés.
