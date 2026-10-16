# Les défauts laissés par la vérification du lock d'`install`, et les retours des relecteurs — plan

> Suite de `docs/plans/install-verify.md` (section « Défauts vus, laissés
> tels quels », numéros repris ici) et des constats tardifs de la revue de
> `docs/plans/root-version-warning.md`. Niveau 2 : sorties publiques (stderr,
> stdout, codes, fichiers écrits) de toutes les commandes. Méta-analyse et
> revue par sous-agents frais.

Références : `docs/reference/*.php` quand le fichier y est, sinon le phar
2.10.3 extrait (`src/Composer/…`, mêmes octets que `/opt/homebrew/bin/composer`,
sha256 `7a2d379d…`). Les numéros de ligne sont ceux de ces fichiers (phar
sans commentaires). Mesures : Composer 2.10.3 contre le binaire de `main`
(db349c1), sur les instantanés figés des harnais (`harness/steps.sh`,
`harness/verbosity.sh`), sauf mention.

## 1. `config.lock: false` (défaut 5)

**Référence.** `Factory::createComposer` (docs/reference/Factory.php:423-429) :
un composer.lock présent alors que `config.lock` est faux écrit
`<warning>./composer.lock is present but ignored as the "lock" config option
is disabled.</warning>` — juste après le chargement de la racine (donc après
l'avertissement de version racine), pour **toute** commande qui crée un
`Composer` complet — et le `Locker` lit `/dev/null` : `isLocked()` est faux
(Locker.php:124-133). `Installer::run` (Installer.php:277-285) : une install
sans lock écrit `Lockfile creation is disabled in config. Updating
dependencies to latest instead of installing from lock file. See
https://getcomposer.org/install for more information.` (ou, `config.lock`
vrai mais pas de lock : `No composer.lock file present. …`), puis passe en
`update` (`doUpdate($localRepo, true)`) : `PRE_UPDATE_CMD`, « Loading
composer repositories… », résolution **sans dépôt verrouillé**, pas de
« Lock file operations » ni de lignes `Locking` (l. 609, 655), pas d'écriture
(`writeLock` faux, l. 678-682), puis `doInstall($localRepo, true)` sur les
données gardées en mémoire (`setLockData(..., false)` : lock « virtuel »),
sans en-tête « Installing dependencies from lock file » (l. 753).
`RequireCommand`/`RemoveCommand` recréent le `Composer` après l'édition du
manifeste : la ligne de `Factory` sort deux fois.

**Mesure** (laravel, `config.lock: false`, lock présent).

| commande | Composer | vivacity (`main`) |
|---|---|---|
| `install --dry-run` | ligne Factory, « Lockfile creation is disabled… », résolution : `laravel/pint` **v1.32.1**, 49 suggestions | « Verifying… », installe le lock (**v1.32.0**) |
| `install` (sans lock, `config.lock: false`) | les deux lignes moins celle de Factory, installe | exit 1, « no composer.lock » |
| `update` (avec install) | ligne Factory, résout, installe **v1.32.1** | résout (lock lu comme dépôt verrouillé), installe le lock **du disque** (v1.32.0) |
| `update --dry-run`, `--no-install` | ligne Factory, « 49 package suggestions were added… » | ni l'une ni l'autre (le lock lu rend la transaction vide : pas de suggestions) |
| `require x --no-install`, `remove x --no-install` | la ligne Factory deux fois (avant et après « has been updated ») | aucune |
| `dump-autoload` | ligne Factory | aucune |

**Correctif.**

1. *La ligne de Factory*, retenue comme l'avertissement de version racine
   (même mécanisme, point 3 ci-dessous) pour les cinq commandes, et réécrite
   par `require`/`remove` à la seconde création (comme la ligne de version).
   `./composer.lock` littéral : `Factory::getLockFile('./composer.json')`
   (`COMPOSER=` est refusé par vivacity).
2. *La session ignore le lock* quand `config.lock` est faux
   (`UpdateSession::prepare_update` lisait composer.lock sans condition) :
   plus de dépôt verrouillé, la transaction de lock est celle de Composer
   (suggestions comprises). Une mise à jour partielle sans lock sort alors
   comme Composer (« Cannot update only a partial set of packages without a
   lock file present », code 3) — à mesurer pour `require`/`remove`.
3. *L'install qui suit un `update` sans écriture* installe le lock résolu
   (le lock « virtuel », comme le `--dry-run` le faisait déjà), pas celui du
   disque.
4. *`install` sans lock utilisable* (`config.lock` faux, ou fichier absent, ou
   sans clé `packages` : `isLocked()` faux) : la ligne d'`Installer::run`,
   puis le chemin d'`update` de vivacity (mêmes options : `--no-dev`,
   `--dry-run`, `--ignore-platform-req(s)`, `--no-blocking`, `--prefer-*`,
   autoloader), la ligne imprimée après l'événement `COMMAND` de bamarni et
   avant « Loading composer repositories… » (champ interne d'`UpdateArgs`).
   `--run-scripts` (des scripts d'`update`, que l'`update` de vivacity ne
   joue pas) : rendu à Composer. Le lock reste écrit quand `config.lock` est
   vrai (cas « pas de lock »), comme Composer.
   Remplace l'erreur « `install` needs one » de vivacity.

**Cas** (rouges sur `main`) : `verbosity.sh` (stdout+stderr entiers, trois
niveaux) — `laravel|install --dry-run @jq:.config.lock=false`,
`laravel|install --dry-run @nolock`, `laravel|update --dry-run --no-install
@jq:.config.lock=false`, `laravel|dump-autoload @vendor
@jq:.config.lock=false`, `laravel|require symfony/uid --dry-run --no-install
@jq:.config.lock=false`, `laravel|remove laravel/tinker --dry-run
--no-install @jq:.config.lock=false` ; `steps.sh` (lock, manifeste, code) :
`laravel|install @lockfalse @nostderr`, `laravel|install @nolock @nostderr` ;
une vraie installation `update` + `config.lock: false` comparée sur
`vendor/composer/installed.json` (nouveau prep `@files:` de verbosity.sh,
voir point 3).

**Laissé** : `dump-autoload` lit composer.lock pour sa disposition (Composer :
le dépôt local) — avec `config.lock: false` il lit un lock que Composer
ignore. Préexistant, noté.

## 2. `ignoredTypes` du `PoolBuilder` (défaut 8)

**Référence.** `Installer::$ignoredTypes = ['php-ext', 'php-ext-zend']`
(Installer.php:196), passé à `createPool` par `doUpdate` (l. 534) et
`doInstall` (l. 808) ; `PoolBuilder::loadPackagesMarkedForLoading` saute un
paquet chargé de ces types (resolver/PoolBuilder.php:461) — pas les paquets
fixés ou verrouillés, qui n'y passent pas. `extractDevPackages` :
`createPoolWithAllPackages`, sans filtre.

**Mesure** : aucune fixture n'a de paquet de ces types. Fixture synthétique
ajoutée à `solver-problems` (`fixtures/make-solver-problems.py`) :
`acme/phpext` en `1.0.0` (`library`) et `2.0.0` (`php-ext`).

**Correctif** : `PoolBuilder` reçoit la liste d'`Installer` (constante
`INSTALLER_IGNORED_TYPES`) pour la résolution d'`update` et la vérification
d'`install` ; vide ailleurs (comme `RepositorySet::createPool` par défaut).

**Cas** : `solver-problems|update @nolock @stderr @require:acme/phpext=*`
(Composer verrouille 1.0.0 ; vivacity, attendu, 2.0.0) et
`…@require:acme/phpext=^2` (problème : la 2.0.0 n'est pas chargée).

## 3. `COMPOSER_IGNORE_PLATFORM_REQ(S)` sur `dump-autoload`, et les lignes retenues (défaut 2)

**Référence.** `BaseCommand::initialize` (src/Composer/Command/BaseCommand.php:276-291) :
pour toute commande qui a l'option, la variable remplit l'option laissée
vide, avec son avertissement, après la création du `Composer` (ligne de
version racine, ligne de Factory) et avant `execute`.
`DumpAutoloadCommand` a les deux options (l. 40-41) et passe le filtre à
`AutoloadGenerator::setPlatformRequirementFilter` (l. 114) : `platform_check.php`
en dépend.

**Mesure** : `dump-autoload` avec `COMPOSER_IGNORE_PLATFORM_REQS=1` : Composer
écrit l'avertissement et un `platform_check.php` sans exigence (ou aucun) ;
vivacity n'écrit pas l'avertissement et garde les exigences.

**Correctif.**

- `dump-autoload` lit les deux variables comme `install` (même fonction,
  sortie dans `vivacity_core` ou `manifest.rs`), avertissement compris.
- **Les lignes retenues** : `io::hold` ne gardait qu'une ligne. Il en garde
  une file : ce que `Factory` et `BaseCommand::initialize` écrivent avant la
  commande (ligne de version racine, ligne de Factory du point 1, les deux
  avertissements des variables), et l'avertissement de dépréciation
  `--no-suggest` de `require` (écrit par `RequireCommand::execute` avant
  toute autre ligne). Toutes sortent, dans l'ordre, à la première ligne
  propre de vivacity ; toutes sont jetées par une délégation précoce, dont le
  Composer les écrit lui-même — plus de doublon sur les délégations
  **précoces** (`g-merge-env`, `g-require-no-suggest` de `fallback-args.sh`,
  qui affirment aujourd'hui 1 ligne de vivacity : écart connu, à passer à 0).
  Les délégations **tardives** gardent leurs doublons (hors périmètre, déjà
  noté).
- `update`, `require`, `remove` impriment leurs avertissements de variables
  tard (après `resolution_fallback`) : ramenés en tête, retenus, au point de
  `BaseCommand::initialize` — si la mesure montre l'écart (cas ci-dessous).

**Cas** : `verbosity.sh` — `laravel|dump-autoload @vendor
@env:COMPOSER_IGNORE_PLATFORM_REQS=1 @files:vendor/composer/platform_check.php`,
idem `COMPOSER_IGNORE_PLATFORM_REQ=php` ; nouveau prep `@files:chemin[,chemin]`
(le fichier comparé à l'octet entre les deux projets, présence comprise).
`fallback-args.sh` : `g-merge-env`, `g-require-no-suggest` à 0 ligne de
vivacity, et un compte des lignes d'avertissement de variable (0 avant une
délégation précoce).

## 4. `COMPOSER_NO_DEV` (défaut 3)

**Référence.** `BaseCommand::initialize` l. 255-274 : `(bool)
Platform::getEnv('COMPOSER_NO_DEV')` — vrai pour toute chaîne non vide
différente de `"0"` — pose `--no-dev` (`install`, `dump-autoload`) ou
`--update-no-dev` (`require`, `remove`), et `--no-dev` d'`update`.

**Mesure** : `install --dry-run` avec `COMPOSER_NO_DEV=yes` : Composer sans
dev (76 paquets), vivacity avec (109). `dump-autoload` : vivacity ignore la
variable.

**Correctif** : `install` et `dump-autoload` lisent la variable comme
`update` (`!v.is_empty() && v != "0"`).

**Cas** : `verbosity.sh` — `laravel|install --dry-run
@env:COMPOSER_NO_DEV=yes`, `laravel|dump-autoload @vendor
@env:COMPOSER_NO_DEV=true @files:vendor/composer/autoload_static.php`, et le
témoin `@env:COMPOSER_NO_DEV=0`.

## 5. Lock sans `packages-dev` en mode dev (défaut 7)

**Référence.** `Installer::doInstall` l. 753-757 : l'en-tête, puis
`Locker::getLockedRepository($this->devMode)` (Locker.php:157-165) lève
`RuntimeException('The lock file does not contain require-dev information,
run install with the --no-dev option or delete it and run composer update to
generate a new lock file.')` — avant « Verifying… ». Code 1 (exception),
message dans l'encadré de Symfony Console, sous `-q` aussi.

**Mesure** : Composer : en-tête, encadré, code 1. vivacity : en-tête,
« Verifying… », `Error: composer.lock: The lock file …`, code 1.

**Correctif** : le test dans `headline`, juste après l'en-tête (pas après une
mise à jour, dont le lock a toujours `packages-dev`) : le message seul, au
niveau QUIET, code 1 — la forme déjà prise pour les refus de
`RootPackageLoader` (sans encadré).

**Cas** : `root-manifest.sh` gagne un cas « lock » (même extraction de
l'encadré, plus les lignes qui le précèdent chez Composer : l'en-tête) :
`install` avec `del(.["packages-dev"])`, et le témoin `install --no-dev`
(code 0, rien à comparer : dans `steps.sh`).

## 6. `GithubActionError::emit` (défaut 4)

**Référence.** `src/Composer/Console/GithubActionError.php` :
`GITHUB_ACTIONS` vrai et `COMPOSER_TESTS_ARE_RUNNING` faux (`Platform::getEnv`,
truthiness PHP) → `$io->write("::error ::" . escapeData($message))` sur
**stdout**, niveau normal (`%`→`%25`, `\r`→`%0D`, `\n`→`%0A`). Appelants :
`Installer::doUpdate` (l. 554, `err."\n".prettyProblem`),
`extractDevPackages` (l. 735, idem, sans les deux lignes d'aide),
`doInstall` (l. 829), `Application::doRun` (l. 482 : le message de toute
exception ; l. 307 : `ParsingException` du manifeste avec `file=`/`line=`).

**Mesure** (solver-problems, `update` insoluble, `GITHUB_ACTIONS=true`, sans
`COMPOSER_TESTS_ARE_RUNNING`) : une ligne `::error ::Your requirements could
not be resolved…%0A%0A  Problem 1%0A…` sur stdout ; vivacity : rien.

**Correctif** : `vivacity_core::io::github_action_error(message)` ; appelée
aux trois points du solveur (le texte imprimé, déjà sans balises) et pour les
exceptions dont vivacity reproduit le message de Composer (refus de
`RootPackageLoader::load`, lock sans `packages-dev`). Les autres `Error: …` de
vivacity ont leur propre texte : pas d'annotation (laissé, noté).

**Cas** : `verbosity.sh`, `@unset:COMPOSER_TESTS_ARE_RUNNING
@env:GITHUB_ACTIONS=true` sur `solver-problems|update --no-install @nolock
@require:acme/missing-dep=^1.0`, `…--no-dev … @require:acme/nope=^1.0`
(extraction dev : à trouver), `laravel|install --dry-run
@require:ext-nope=*` ; témoin `GITHUB_ACTIONS=0`.

## 7. `--offline` (défaut 9)

**Référence.** `HttpDownloader` (src/Composer/Util/HttpDownloader.php:73,
328-338) : `COMPOSER_DISABLE_NETWORK` vrai → une requête avec
`If-Modified-Since` reçoit un **304** (le cache sert, aucun avertissement),
une requête sans lui échoue en `TransportException('Network disabled,
request canceled: '.Url::sanitize($url), 499)`. Les `file://` passent par
`RemoteFilesystem`, sans ce contrôle.

**Mesure** (laravel sur Packagist réel, `install --dry-run`) : cache chaud —
Composer muet, vivacity « Warning: https://repo.packagist.org could not be
fully loaded (offline: cannot fetch …/summary.json), package information was
loaded from the local cache… » ; cache vide — les deux écrivent « Filter list
data could not be fetched… », suivi de `  - Network disabled, request
canceled: https://repo.packagist.org/packages.json` chez Composer et `  -
offline: cannot fetch …` chez vivacity.

**Correctif** : le transport hors ligne répond 304 à une requête
conditionnelle et l'erreur de Composer, URL assainie (`sanitize_url` de
`metacache.rs`), aux autres ; `COMPOSER_DISABLE_NETWORK` (vrai au sens PHP)
vaut `--offline` pour vivacity aussi, comme pour Composer (à mesurer : la
variable est déjà transmise au fallback).

**Cas** : `steps.sh` — `laravel|install @env:COMPOSER_DISABLE_NETWORK=1
@jq:.repositories={"pk":{"type":"composer","url":"https://repo.packagist.org"}}`
(cache vide : la liste de filtrage, comparée) et le même avec un
`packages.json` en cache (nouveau prep `@repocache:` : un `packages.json`
avec `last-modified`, sans réseau) : Composer muet. Test unitaire du
transport hors ligne.

## 8. `require` sans contrainte d'un paquet d'un dépôt `path` (retour du relecteur)

**Référence.** `PackageDiscoveryTrait::getRepos` (l. 44-55) : plateforme
puis `RepositoryFactory::defaultReposWithDefaultManager` (les dépôts du
manifeste, `path` compris) ; `VersionSelector::findBestCandidate` interroge
tout le jeu.

**Mesure** (binaire de `main`, fixture path-repos) : `require acme/beta` —
Composer « Using version ^2.0 for acme/beta », code 0 ; vivacity code 1,
« Could not find a matching version of package acme/beta ». Défaut réel,
pas une régression.

**Cause** : `UpdateSession::find_packages_for_require` ne lit que les dépôts
`Composer` ; les `Path` sont sautés.

**Correctif** : les dépôts `Path` lus comme un `ArrayRepository`
(`array_repository_load_packages`, déjà utilisé par le `PoolBuilder`).

**Cas** : `steps.sh` — `path-repos|require acme/beta`, `path-repos|require
acme/beta --dev`.

## 9. Dépôt hg, svn ou fossil (retour du relecteur)

**Référence.** `VersionGuesser::guessVersion` (docs/reference/VersionGuesser.php:84-104) :
après git, `hg branch` (l. 239), `fossil branch list` / `fossil tag list`
(l. 377, 384), `svn info --xml` (l. 405), exécutés dans le répertoire du
projet ; un code 0 donne une version (hg : toujours ; fossil, svn : presque
toujours). Binaire absent : `ProcessExecutor` rend un code non nul, la
version reste nulle — avertissement et `1.0.0`, comme vivacity.

**Mesure** (laravel rendu bibliothèque, hors git, un `.hg`, un faux `hg` qui
répond `default`) : Composer exécute `hg branch`, `hg branches`, `hg
bookmarks` et se tait ; vivacity écrit l'avertissement (et prend
`1.0.0+no-version-set`). Sans `hg` dans le `PATH` : les deux avertissent.

**Correctif** : quand vivacity ne devine pas la version (pas de `version`,
pas de `COMPOSER_ROOT_VERSION`, git sans résultat) **et** qu'un marqueur
(`.hg`, `.svn`, `_FOSSIL_` ou `.fslckout`) est trouvé en remontant depuis le
projet, la commande de Composer correspondante est lancée dans le projet ;
code 0 → la commande entière est rendue à Composer avant toute lecture
(délégation précoce, comme `-vv`), `--no-fallback`/`--check-scope` → 3 avec
la raison. Sans marqueur (le cas courant) : quelques `stat`, pas de
processus. Binaire absent ou code non nul : natif (Composer avertit aussi).

**Cas** : `fallback-args.sh` (faux composer, faux `hg`) : délégation avec la
raison, 0 ligne d'avertissement de vivacity ; `.hg` + faux `hg` qui échoue :
natif, 1 ligne. `verbosity.sh` : le même projet avec un faux `hg` sur le
`PATH` des deux côtés (Composer muet, vivacity rendu) — à voir selon le
coût du prep.

## 10. Le second appel git (retour du relecteur)

`version_guessable` (avertissement) puis `guess_version` (la version)
relancent `git branch -a -v` (et `describe`). **Correctif** : un cache de la
sortie de ces deux commandes par répertoire, vidé à la fin de `run`. Mesure
avant/après (hyperfine, no-op d'une bibliothèque sur une branche).

## 11. Défauts 6 et 10 — cadrage

- **6a. Dépôts apportés par composer-merge-plugin et filtre de liste.**
  `ExtraPackage::prependRepositories` (plugins/composer-merge-plugin/ExtraPackage.php:217-245)
  ajoute en tête du `RepositoryManager` les dépôts des fichiers fusionnés ;
  le filtre de liste d'`install` les interroge. vivacity ne lit que ceux du
  manifeste. Correctif possible : `Merged` garde les définitions (et pas
  seulement les fichiers), `verify_install` les met en tête dans l'ordre de
  `prependRepository`. Coût : moyen (une fixture de merge-plugin avec un
  dépôt composer servi hors ligne et une liste). Livré si le temps le
  permet, sinon documenté.
- **6b. Le fallback du plugin de fusion pendant l'attente réseau du fil.**
  `report_missing` rend la main pendant que le fil peut encore écrire le
  cache de dépôt : le fil est **joint** (résultat jeté) avant de lancer
  Composer. Coût : faible.
- **10. Avertissements d'un dépôt dégradé perdus si un autre échoue.**
  `verify_install` : sur l'erreur du filtre, les lignes gardées
  (`take_held`) des autres dépôts sont jointes à l'erreur et imprimées avant
  elle, l'ordre de Composer (il les écrit en construisant le pool). Test
  unitaire (transport factice : un dépôt dégradé, un dépôt en erreur).

## Ordre de livraison (un commit par point)

1. Lignes retenues en file + variables d'environnement de `dump-autoload`
   (point 3) — le mécanisme dont le point 1 a besoin.
2. `config.lock: false` (point 1).
3. `COMPOSER_NO_DEV` (point 4).
4. Lock sans `packages-dev` (point 5).
5. `ignoredTypes` (point 2).
6. `GithubActionError` (point 6).
7. `--offline` (point 7).
8. `require` sur dépôt `path` (point 8).
9. hg/svn/fossil (point 9).
10. git une fois (point 10).
11. 6b, 10, puis 6a.

Chaque point : le cas rouge sur `main` (binaire `vivacity-main` copié avant
tout changement), le correctif, le cas vert, CHANGELOG/DECISIONS/HANDOVER et
la marque dans `install-verify.md` ou `root-version-warning.md` dans le même
commit.

## Écartés

- **Schéma `LAX_SCHEMA`** (relecteur de root-version) : non porté, déjà noté.
- **Doublons des délégations tardives** : décider la portée d'`install` avant
  l'en-tête, hors de ce lot.
- **Annotations GitHub des autres erreurs de vivacity** : leurs textes ne sont
  pas ceux de Composer ; annoter un texte faux n'avance pas la parité.

## Vérification

`cargo fmt --all -- --check` ; clippy `-D warnings` macOS et
`x86_64-pc-windows-gnu` ; `cargo test --workspace` ; les 30 passages de
`.github/workflows/ci.yml` et `harness/root-manifest.sh`, chacun à 0,
répertoires de travail supprimés après chaque passe.
