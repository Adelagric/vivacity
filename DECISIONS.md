# DECISIONS

Choix de conception non triviaux, datés, avec alternatives. Compléter en continu.

## 2026-09-09 — Architecture store + clone (plan r3)

Le spike M0 (bench/M0-profil.md) montre que réécrire l'extraction en Rust ne gagne
presque rien (1,01× sur sylius) : le goulot warm est l'I/O metadata (40k fichiers).
Décision : extraire une fois par (paquet, version, ref) dans un store local, puis
cloner vers vendor/ — `clonefile(2)` APFS (0,73 s vs 5,8 s mesuré), reflink
btrfs/xfs, fallback hardlink puis copie. Alternative écartée : extraction directe
optimisée (plafond démontré trop bas).

## 2026-09-09 — Émulation native d'une allowlist de plugins (plan r2)

`--no-plugins` casse le boot de tout Symfony moderne : `vendor/autoload_runtime.php`
est généré par le plugin `symfony/runtime` (stub ~30 lignes, déterministe, options
`extra.runtime`). v1 émule nativement ce stub (test de drift contre le plugin réel) ;
tout autre plugin → fallback `exec composer`. Alternative écartée : exécuter les
plugins PHP (réintroduit le runtime Composer, annule le gain).

## 2026-09-09 — Oracle différentiel comme gate de parité

L'oracle de compatibilité est le Composer installé (phar 2.10.3 copié en .phar,
classes invoquées via `php -r`). Golden bloquant : content-hash des 3 fixtures.
Sources canoniques épinglées depuis le phar dans docs/reference/ (Locker.php,
JsonFile.php) — jamais de port de mémoire.

## 2026-09-09 — serde_json avec `preserve_order` + `float_roundtrip`

- `preserve_order` : le content-hash de Composer ne ksorte que le premier niveau ;
  l'ordre d'insertion des clés imbriquées doit survivre au parsing.
- `float_roundtrip` : trouvé par property test (cas figé en régression dans
  oracle_content_hash.rs) — sans elle, serde_json parse les doubles à 1 ULP près
  et le hash diverge. Coût parsing accepté (les manifestes sont petits).

## 2026-09-09 — Formatage des doubles : port empirique de PHP

`json_encode` (serialize_precision=-1) : chiffres shortest-round-trip, notation
fixe ssi point décimal ∈ (-4, 17], sinon exponentielle `d[.ddd|.0]e±X`, pas de
`.0` final en fixe, `-0` pour le zéro négatif, int > PHP_INT_MAX décodé float.
Bornes relevées empiriquement sur PHP 8.5.10 (table dans l'historique de session,
vérifiée en continu par le proptest différentiel). On réutilise les chiffres de
`format!("{:e}")` (shortest std) plutôt qu'une dépendance ryu directe.

## 2026-09-09 — Dépendances (workspace, épinglées `=X.Y.Z`)

anyhow (main uniquement), serde/serde_json (manifestes), tokio+reqwest (fan-out
HTTP/2, pattern uv/pnpm), zip (extraction, pas d'async nécessaire : spawn_blocking),
sha1/sha2 (cache/shasum Composer), md-5 (content-hash), thiserror (erreurs lib),
clap (CLI), proptest (dev, oracle fuzzé). `hex` retiré au profit de format hex
std à venir dans vivace-core (le spike jetable l'utilise encore).

## 2026-09-09 — M2 : fidélité vendor/ prouvée par diff -r, pas par tests unitaires

Le juge de paix de l'installeur est `harness/diff-vendor.sh` : `diff -r` entre
le vendor de Composer et le nôtre sur les 3 fixtures. Trois comportements de
Composer découverts en chemin et reproduits (aucun n'était dans le plan) :
`target-dir` legacy (chemin d'install `vendor/<name>/<target-dir>`), chemins
d'état raccourcis `./x` pour le namespace `composer/*` (findShortestPath depuis
vendor/composer), `replace`/`provide` du composer.json racine dans installed.php.
Alternative écartée : un harness Rust structuré dès M2 — le script shell suffit
et M4 le formalisera (normalisations, boot, CI).

## 2026-09-09 — Store : clé (nom, version, ref12), pas de hash de contenu

L'entrée de store est adressée par (paquet, version, 12 hex de la référence de
dist) — pas par sha256 du zip : Packagist ne fournit pas de shasum, et la
référence git est déjà l'ancrage d'intégrité de Composer. Coût : un zip
re-publié sous la même référence (cas pathologique) réutiliserait l'ancienne
extraction. Accepté et documenté ; `vivace store prune` viendra plus tard.

## 2026-09-09 — Clone : clonefile(2) macOS, hardlinks ailleurs, copie en repli

Sur Linux, le mode hardlink signifie qu'éditer EN PLACE un fichier de vendor/
modifie le store (même inode) — pnpm vit avec la même contrainte. vivace ne
modifie jamais un fichier cloné (il remplace des répertoires entiers). À
documenter pour les utilisateurs qui patchent vendor/ à la main ; reflink
FICLONE par fichier sur btrfs/xfs est une amélioration possible.

## 2026-09-10 — M3 : classmap par port du cleaner + pcre2, pas de tokenizer PHP

Composer détecte les classes par `php_strip_whitespace()` (tokenizer) puis un
cleaner à état + UN pattern PCRE (possessifs, lookbehind, octets `\x7f-\xff`).
vivace porte le cleaner (en ajoutant les commentaires `#` que strip_whitespace
retirait) et exécute le pattern original via `pcre2` (décision F4). Preuve :
`PhpFileParser::findClasses` du phar sur ~50 000 fichiers des fixtures, zéro
divergence (tests/oracle_classmap.rs, noms comparés en base64).
Alternative écartée : mago-syntax (vrai parser) — plus « juste » mais pas
identique à Composer sur les cas tordus, et la fidélité prime.

## 2026-09-10 — Noms de classes en octets bruts

`symfony/cache` déclare une classe nommée d'un octet non-UTF-8 et Composer
l'écrit tel quel. Les noms circulent donc en `Vec<u8>` de bout en bout
(scanner, var_export, fichiers assemblés en octets) — le prix d'une parité
octale, trouvé par le harness sur la fixture symfony.

## 2026-09-10 — Chemin classmap absent = erreur, comme Composer

Une règle `classmap` pointant sur un chemin inexistant fait échouer Composer
(« Could not scan for classes inside … »). vivace faisait de même en silence :
aligné sur l'erreur explicite. Corollaire : le harness copie désormais le projet
complet (sans vendor/node_modules/var), pas seulement composer.json/lock.

## 2026-09-10 — `optimize-autoloader` / `classmap-authoritative` : flags OU config

Comme InstallCommand : `-o`/`-a` ou `config.optimize-autoloader` /
`config.classmap-authoritative` du composer.json (Laravel active le premier
par défaut). Le suffixe des classes d'init est le content-hash du lock
(Composer ≥ 2.2), donc déterministe et identique entre les deux outils.

## 2026-09-10 — M5 : mesurer avant d'optimiser a évité deux fausses pistes

La décomposition (bench/M5-perf.md) a montré que le « scan plus lent que
Composer » était un cache de pages froid, et que paralléliser la LECTURE est
contre-productif sur APFS (3-4× plus lent). Optimisations retenues : détection
parallèle sur le CPU (threads scoped std, pas de dépendance rayon), `realpath`
seulement sous symlink, et surtout un cache de classmap par entrée de store.

## 2026-09-10 — Cache de classmap : classes brutes par fichier, sémantique rejouée

Le cache ne stocke que la sortie de `find_classes` par fichier (ordre de
parcours) ; exclusions, dédoublonnage, filtre PSR et ambiguïtés sont rejoués à
chaque dump. Ainsi le cache ne dépend ni du projet ni des règles d'autoload,
seulement de l'entrée de store (immuable) et du sous-répertoire. Un arbre avec
symlink n'est jamais caché. Contrat assumé : vendor/ immuable entre installs
(documenté ; `VIVACE_NO_CLASSMAP_CACHE=1` pour l'ignorer). Alternative écartée :
mettre en cache la classmap finale par projet — dépendante des exclusions et
des chemins, plus fragile pour un gain identique.

## 2026-09-10 — M4 : harness en shell, Linux d'abord en conteneur, CI ensuite

Le harness reste un script (`harness/diff-vendor.sh` + `harness/boot.sh`) :
`diff -r` sur des copies complètes du projet est le test le plus fidèle qui
existe, et le suffixe d'autoloader étant déterministe (content-hash), aucune
normalisation n'est nécessaire — un harness Rust n'apporterait rien. Linux est
exercé localement dans un conteneur (`harness/linux.sh`, image php:8.4 +
rustup, code monté en lecture seule, volumes pour cargo/target/fixtures/caches)
AVANT toute CI distante, pour ne pas déboguer à l'aveugle sur des runners. Le
workflow GitHub Actions (`.github/workflows/ci.yml`, matrice ubuntu + macos)
rejoue la même chaîne.

## 2026-09-10 — Fixtures créées sans exécuter de PHP

`fixtures/make.sh` fait `create-project --no-scripts --no-plugins --no-install`
puis `install --no-plugins --no-scripts` : aucun code PHP tiers n'est exécuté
sur la machine de dev ni en CI (les scripts post-create de Laravel/Sylius sont
inutiles pour la qualification au boot). Effet de bord observé : une résolution
fraîche de sylius-standard ne boote pas sur PHP 8.5 (Doctrine ORM exige
symfony/var-exporter ou les lazy objects natifs 8.4 — incompatibilité amont) ;
la CI épingle PHP 8.4. `VIVACE_FIXTURES_DIR` permet un répertoire alternatif.

## 2026-09-10 — TLS : rustls, pas OpenSSL

Le premier build Linux a échoué sur `openssl-sys` (reqwest en `native-tls`).
Bascule sur `rustls-tls` avec `default-features = false` : aucune bibliothèque
système requise, même comportement HTTP/2, et un binaire distribuable sans
dépendre de la libssl de l'hôte (utile pour `cargo dist` en M6). Coût : la
racine de confiance est celle embarquée par rustls (webpki-roots via reqwest),
pas le magasin système — acceptable pour Packagist/GitHub ; à noter pour les
dépôts privés à CA interne (`SSL_CERT_FILE` non lu : limitation documentée).

## 2026-09-10 — Version racine devinée depuis git, comme Composer

`installed.php` divergeait sur l'entrée `root` dans tout checkout git (Composer
devine `dev-<branche>` + SHA via VersionGuesser). Port de RootPackageLoader +
guessGitVersion (branche courante, HEAD détaché → tag exact, branche de feature
→ parente la plus proche par `git rev-list`, branch-alias, COMPOSER_ROOT_VERSION),
sortie git forcée en anglais (`LANGUAGE=C`, comme GitUtil::cleanEnv — un git en
français a fait échouer le premier test). Le harness commite désormais un dépôt
git identique (même arbre, auteur et date figés → même SHA) des deux côtés.
Non porté : hg/fossil/svn.

## 2026-09-10 — Alias de branche des paquets verrouillés (fixture rector)

Premier lock extérieur passé au harness (rector-src, généré par `composer
update --no-install`) : `installed.php` divergeait sur les quatre paquets
`rector/rector-*` verrouillés en `dev-main` avec `"default-branch": true` —
Composer leur attache un AliasPackage `9999999-dev` (ArrayLoader::getBranchAlias)
et installed.php liste sa version jolie dans `aliases`. vivace ne calculait
d'alias que pour la racine, et avec une règle partielle (raw `extra.branch-alias`).
Décision : port complet de `getBranchAlias` dans `root_version::branch_alias_of`
(cible `-dev` normalisée par normalizeBranch, source comparée sans casse,
préfixes numériques compatibles, sinon `9999999-dev` si default-branch et
version non numérique ; version jolie = `(\.9{7})+` → `.x`), utilisé pour la
racine ET pour chaque paquet du lock. Oracle `tests/oracle_branch_alias.rs`
(31 configurations contre le phar, 0 divergence). rector devient la 4e fixture
figée (squelette minimal : manifestes, lock, points d'entrée d'autoload) ; il
couvre aussi les deux plugins extension-installer et `platform-check: false`.

## 2026-09-10 — composer/installers émulé nativement (v0.2)

Premier plugin de layout émulé. Le chemin d'un paquet n'est jamais deviné :
`layout::resolve` ne s'active que si le plugin est verrouillé, **autorisé**
par `config.allow-plugins` (port des trois formes de PluginManager : booléen,
motifs `packageNameToRegexp`, fusion avec `$COMPOSER_HOME/config.json` ;
non listé → fallback, car Composer non interactif refuse), et **porté** : une
table par tag 2.0.0…2.3.0 (`assets/installers/<tag>.json`, générées par
`tools/gen-installers-table.php` par réflexion sur la source, jamais
recopiées ; la logique Installer/BaseInstaller est identique sur tout 2.x,
seules les tables bougent). 58 frameworks « table seule » sont émulés
(WordPress, Drupal, Laravel, Magento, Moodle…) ; les 38 qui surchargent
`inflectPackageVars`/`getLocations`/`getInstallPath` (CakePHP, Grav,
October/Winter, Shopware, Mautic, Matomo, MediaWiki, ProcessWire…) restent en
fallback avec un message nominatif. Refus délibérés (→ fallback) : cible
absolue, hors projet, racine du projet, sous vendor/, deux paquets sur la même
cible, cible contenant une autre cible, `installer-paths`/`installer-name`
hors schéma, variable de template inconnue.

Vérité : un oracle de bout en bout (`tests/oracle_installers.rs`) construit
un Composer avec le vrai plugin activé et interroge
`InstallationManager::getInstallPath` (dispatch réel : `supports` faux →
LibraryInstaller → vendor/) puis `findShortestPath` — 665 chemins comparés,
0 divergence ; la fixture `wordpress` (wpackagist + roots/soil + un dépôt
`package` avec `bin`) est comparée **projet entier** contre Composer avec le
plugin actif, et `harness/removal.sh` vérifie qu'un paquet retiré du lock
disparaît au bon endroit, dans et hors vendor/.

État du plugin : Composer le charge depuis installed.json
(`PluginManager::loadInstalledPlugins`) et l'installe en premier dans la
transaction. vivace n'émule que les états où installed.json et le lock
concordent ; un plugin présent d'un seul côté (ajouté à un vendor existant,
retiré, ou en require-dev avec `--no-dev`) alors qu'un paquet a un type que
le plugin prendrait → fallback nominatif (« let Composer handle this
transition »). `--no-plugins` désactive l'émulation comme chez Composer.
Trouvé par la revue indépendante, pas par le harness (qui installe toujours
à partir de zéro).

Conséquences dans le code : `Filesystem::findShortestPath(Code)` et
`normalizePath` sont désormais des ports exacts dans vivace-core (l'ancienne
approximation de vivace-autoload est remplacée) ; les proxies bin calculent
leurs chemins relatifs au lieu de `../` codé en dur ; la suppression d'un
paquet retiré suit la règle de LibraryInstaller (chemin recalculé avec la
config courante, égal à l'ancien install-path, sinon fallback ; répertoire
effacé = `getPackageBasePath`, donc `vendor/<name>` sans le target-dir ;
parent vide supprimé, jamais la racine du projet). La racine du projet est
absolutisée dès la CLI : un `--working-dir` relatif produisait des proxies
bin à chemin absolu (régression trouvée par la revue).

## 2026-09-10 — Extraction : strip du dossier racine seulement s'il est unique

`extract.rs` retirait le premier composant de chaque entrée sans condition —
un zip à racine multiple (fichier + dossier, ou deux dossiers) perdait ses
fichiers de premier niveau en silence. Règle d'`ArchiveDownloader::install`
portée : strip ssi exactement une entrée de premier niveau et que c'est un
répertoire (`.DS_Store` ignoré). Trouvé par la méta-analyse du plan v0.2,
avant qu'un dépôt `package` ne le déclenche.

## 2026-09-10 — Drift en deux étages, CI épinglée

`ci.yml` teste désormais `composer:2.10.3` (la référence vendorée) pour être
déterministe. `drift.yml` (hebdomadaire + manuel) interroge le dernier stable
et le snapshot : étage 1, `harness/drift-reference.sh` diffe chaque fichier
de docs/reference/ avec son jumeau dans le phar (signal nominatif en
secondes) ; étage 2, tests + harness complet. Un échec ouvre ou commente une
issue `drift`. Le même script tourne en fin de CI contre le Composer épinglé
(garde-fou contre une référence modifiée à la main).

## 2026-09-10 — GitHub Action à la racine du dépôt

`action.yml` (composite) plutôt qu'un dépôt `setup-vivace` séparé : versionné
par les tags de release, `uses: Adelagric/vivace@v0.2.0` installe exactement
cette version (défaut `github.action_ref`, jamais « latest » depuis une
action ; un ref qui n'est pas un tag est refusé). `install.sh` authentifie la
requête API avec `GITHUB_TOKEN` quand il existe (limite anonyme partagée sur
les runners). `action-test.yml` teste l'action contre le binaire du commit
courant (`binary:`), puis un vrai `vivace install` réseau de la fixture
Laravel. L'attestation de provenance des binaires est notée pour plus tard :
le README dit « sha256-verified download », pas « verified binary ».

## 2026-09-11 — Doubles : égalités exactes arrondies au chiffre pair (dtoa)

Le property test a tiré `-2124202659384827.2` (double exact `…27.25`, à
mi-chemin entre « .2 » et « .3 ») : PHP (`zend_gcvt` mode 0 = dtoa) arrondit
au chiffre pair, `{:e}` de Rust vers le haut. Correctif dans `phpjson` :
recalcul des mêmes n chiffres par le formatage à précision fixe de Rust
(exact, demi-pair), gardé s'il round-trippe encore. Régression déterministe
figée dans `prop_phpjson_oracle.rs`, 400 cas rejoués sans divergence. Second
piège de formatage trouvé par le même test après `float_roundtrip` : la
parité à l'octet sur les flottants ne se devine pas, elle se fuzze.

## 2026-09-11 — drupal/core-composer-scaffold émulé nativement (v0.3)

Fait vérifié : `--no-scripts` ne coupe que les scripts du composer.json
racine ; les écouteurs des plugins s'exécutent, et le scaffold écrit
web/index.php, .htaccess, sites/default/default.settings.php,
web/autoload.php, autoload_runtime.php — ignorés par git dans un projet
Drupal type. Sans émulation, `vivace install` ne donne pas un Drupal qui
démarre. Port complet dans `scaffold.rs` : paquets autorisés (implicites +
récursifs, racine en dernier), opérations replace/append/skip avec surcharge
entre paquets, `checkUnchanged`, fichiers autoload de référence (sauf si
trackés par git), `.gitignore` (règle exacte : option, sinon dépôt git ET
`vendor` ignoré), et `preAutoloadDump` (entrées de classmap Symfony/PSR +
`vendor/drupal/DrupalInstalled.php`, hash xxh3 des noms uniques triés —
d'où la seule dépendance nouvelle, `xxhash-rust`, égalité vérifiée avec
`hash('xxh3')`).

Version du plugin : **empreinte de la source** (sha256 des PHP hors tests)
plutôt qu'un tag — le cœur est identique de 10.3.0 à 12.0.0-alpha1, seules
trois fonctionnalités s'ajoutent (preAutoloadDump en 11.3.0, hash trié en
11.3.4, autoload_runtime.php en 11.4.0) : 11 empreintes, 120 versions, un
port paramétré par profil. Refusé (fallback) : 11.3.0–11.3.3 (hash dans
l'ordre de la transaction Composer, non reproductible), `symlink: true`,
destination existante qui n'est pas un fichier régulier (Composer
l'effacerait récursivement), source hors du paquet, destination hors du
projet, et une copie installée du plugin dont la source diffère de celle du
lock (Composer exécute alors l'ancien Handler avec le nouveau Plugin — cas
réel de toute montée de version du cœur, testé par harness/transitions.sh).
Le plan est calculé après le fetch et **avant** toute suppression : un
refus laisse vendor/ intact. `core-project-message` et `core-recipe-unpack`
sont BENIGN (événements écoutés cités dans scope.rs).

Vérité : oracle bout en bout `tests/oracle_scaffold.rs` (15 mini-projets,
Composer avec le plugin contre Composer `--no-plugins` + le port, arbres
entiers comparés, trois versions du plugin) ; fixture `drupal` comparée
projet entier ; `harness/removal.sh` et `harness/transitions.sh`.
Non-objectif assumé : `cweagans/composer-patches` (modifie les dists →
clé de store à définir), chantier suivant.

## 2026-09-11 — Deux écarts découverts par la fixture Drupal, hors scaffold

- `installed.json` : `installation-source` n'existe pas pour un metapackage
  (ArrayDumper n'écrit la clé que si une source a été choisie) — première
  fixture avec un metapackage (drupal/core-recommended).
- `include_paths.php` (paquets PEAR à `include-path`) : Composer l'écrit dans
  l'ordre d'achèvement de ses extractions asynchrones — trois
  `composer install` successifs, trois ordres — et `composer dump-autoload`
  le réécrit dans l'ordre d'installed.json. vivace produit ce dernier ; le
  harness compare ce seul fichier trié, avec la raison dans le script.

## 2026-09-11 — Résolveur : port du solveur de Composer, et le juge d'abord (R0)

Décision utilisateur (option A du cadrage `docs/plans/v0.4-resolver.md`) :
porter `Composer\DependencyResolver\*`, `ComposerRepository`/`PoolBuilder`
et `Installer::doUpdate` plutôt qu'adopter PubGrub. Raison : quand plusieurs
solutions existent, le lock dépend de la politique **et** de l'ordre de
décision du solveur ; un autre algorithme donne un lock valide mais
différent, invérifiable — contraire à la promesse et à tout ce qui a été
prouvé jusqu'ici. Coût assumé : 6-7 000 lignes de PHP, 3 à 5 semaines,
livrées en jalons (R0 juge, R1 métadonnées, R2 solveur, R3 `update`, puis
`require`/`remove`/messages).

R0 : Packagist bouge, deux `update` à une heure d'écart ne sont pas
comparables. `tools/snapshot-packagist.sh` capture le cache Composer d'un
`update --no-install` vierge (toutes les réponses p2 chargées par
PoolBuilder), le rejoue en `file://` jusqu'à ce que Composer n'échoue plus
(paquets virtuels 404 → stub « aucune version ») et archive le lock de
référence. `harness/update.sh` injecte le dépôt local par la config globale
(composer.json intact → content-hash comparé) et vérifie d'abord que
Composer lui-même reproduit le lock de référence sur l'instantané (vrai sur
les cinq fixtures), puis compare le lock de vivace. Découverte : Composer
tolère un 404 de métadonnées en HTTP mais pas un fichier absent en
`file://`. wpackagist (protocole v1) hors périmètre.

## 2026-09-14 — Retrait de l'émulation drupal/core-composer-scaffold (licence)

Fait : `scaffold.rs` était un port déclaré, fonction par fonction, de
`drupal/core-composer-scaffold`, et `assets/scaffold/*.tpl` des copies de ses
gabarits. Le plugin est GPL-2.0-or-later ; un port est une œuvre dérivée, que
l'on ne peut pas distribuer sous MIT/Apache-2.0 avec le reste des crates et
des binaires. Options considérées : (a) isoler le port dans un crate GPL
optionnel hors des binaires publiés — mais les binaires sont l'usage
principal et un crate GPL dans le workspace reste un piège pour qui dépend
de `vivacity-core` ; (b) passer tout le projet en GPL — écarte l'intégration
dans des outils MIT (ePHPm) ; (c) réécriture en salle blanche à partir de la
seule spécification observable — coûteuse à prouver, pour 4 fixtures ;
(d) retirer l'émulation. Décision : (d). Le plugin redevient un plugin
inconnu : `install` bascule sur `composer install` avant toute écriture,
`dump-autoload` refuse (Composer exécuterait son `pre-autoload-dump`). La
fixture drupal reste dans le harness et passe par le fallback ; la source
vendorée `docs/reference/drupal-scaffold/` est supprimée avec le port. Les
versions 0.3.0 à 0.5.0 ne sont plus sur crates.io (crates `vivace*`
supprimés le 2026-09-15 — un yank ne libère pas le nom, signalé par
svandragt dans l'issue #3).

## 2026-09-14 — Renommage en vivacity (v0.6)

Fait : `svandragt/vivace` (créé le 2026-09-06, quatre jours avant ce dépôt)
est une réimplémentation de Composer en Rust avec la même promesse d'un
`vendor/` identique à l'octet, binaire `viv`, v0.11.0 et ~330 commits au
2026-09-14 ; deux `composer-rs` existent aussi. Même nom, même pitch, même
semaine : la coexistence n'exposait à rien juridiquement (pas de marque,
noms libres) mais brouillait toute recherche, toute mention et l'antériorité
est à lui. Décision : renommer en **vivacity** (libre sur crates.io pour les
quatre crates, aucun projet Rust de ce nom sur GitHub), tout d'un bloc —
binaire, crates, entrée `vivacity::run`, variables `VIVACITY_*`, dossiers de
cache, action, dépôt (redirection GitHub conservée). Les plans historiques
sous `docs/plans/` et les entrées datées de ce fichier gardent l'ancien
nom. Alternative écartée : garder le nom et compter sur la différence
d'approche (oracles différentiels, port du solveur) — invisible depuis un
nom de crate.

## 2026-09-15 — Explications d'un ensemble insoluble : l'oracle est stderr (v0.7)

Fait : `steps.sh` faisait tourner Composer avec `--quiet`, or `Installer`
écrit l'en-tête au niveau QUIET et les explications au niveau normal — un
oracle « comparer stderr » bâti sur la commande existante n'aurait vu que
l'en-tête (méta-analyse, faille 1). Décision : pour les cas `@stderr`, la
référence tourne sans `--quiet`, stderr capturé à part (sous GitHub Actions
`GithubActionError` écrit `::error ::…` sur stdout, neutralisé par
`COMPOSER_TESTS_ARE_RUNNING`), et le harness refuse un cas dont la sortie de
référence ne contient aucune ligne `    - …` (oracle aveugle). La
comparaison va de la ligne d'ancrage à la fin, à l'octet, rendu `--no-ansi`
(styles `<error>`/`<warning>`/`<info>`/`<href>` retirés, `<https://…>`
conservé). Alternatives écartées : un format « maison » (deux vocabulaires
pour un solveur, rien de vérifiable) ; déléguer les cas insolubles à
`composer update` (le port existe pour ne pas dépendre de PHP, et `require`
restaure les fichiers — un sous-processus ne le coordonne pas).
Conséquence structurelle : `SolveError::Problems` porte désormais les règles
matérialisées (le `RuleSet` ne survit pas au solveur), le `Pool` enregistre
les versions retirées par l'optimiseur (`recordRemovedVersionsForPackage`,
sauté jusqu'ici « parce que seuls les messages s'en servent ») et par les
politiques, et la sonde PHP renvoie les fichiers `.ini`.

## 2026-09-17 — Les scripts : à Composer, opt-in, jamais de PHP dans vivacity (v0.14)

Mesure sur le corpus : 62/105 projets ont des scripts au moment de
l'install — 74 lignes `php` (45 × `artisan …`), **46 callables PHP
statiques** qui reçoivent `Composer\Script\Event` (accès au `Composer`,
à l'IO, au dépôt local), 24 alias, 9 commandes shell. Les callables ne
peuvent être exécutés que par le Composer PHP : embarquer PHP (et donc
Composer) dans vivacity en ferait « Composer avec un installeur Rust
devant », le produit d'ePHPm, qui embarque vivacity précisément parce
qu'elle n'a pas de PHP. Décision (utilisateur) : frontière technique
ferme — le déterministe (structure, réseau, extraction, résolution,
autoload) à vivacity, le PHP arbitraire à un runtime PHP — et une
couture : `--run-scripts`, opt-in, délègue chaque événement déclaré à
`composer run-script` dans l'ordre exact d'`Installer::run` (lu dans la
source : `pre-install-cmd` avant tout, autoload autour du dump,
`post-install-cmd` après le funding), `COMPOSER_DEV_MODE` posé par
`run-script` depuis `--no-dev`, code de sortie propagé. Le défaut reste
« aucun script » (contrat, CI reproductible) ; l'hôte qui embarque PHP
(ePHPm) gère ses scripts lui-même via la bibliothèque. Ce que
`run-script` ne sait pas porter, dit tel quel : les événements par
paquet (l'opération manque, 5 projets du corpus) et le drapeau
`optimize` des événements d'autoload (vérifié : `optimize=0` chez
Composer, absent via `run-script`) ; les écouteurs de plugins de ces
événements rejouent (idempotents pour les émulés). Preuve :
`harness/scripts.sh` — journal de 15 lignes identique à Composer.

## 2026-09-17 — composer-merge-plugin : la fusion, jamais sa mise à jour implicite (v0.13)

Constat mesuré (open-web-analytics avec ses vrais `modules/*/composer.json`) :
sur un vendor vierge, `composer install` — même `--no-scripts` — installe le
plugin, l'active, et à `post-install-cmd` lance un `composer update` partiel
des exigences fusionnées contre les dépôts en ligne, qui **réécrit
composer.lock** (maxmind-db/reader 1.13.1 → 1.14.0, monolog 2.11.0 →
2.11.1, symfony/filesystem 7.4.15 → 7.4.18). Le résultat d'un `install`
dépend donc de l'heure : Composer lui-même n'est pas reproductible là.
Avec le plugin déjà installé, rien de tel : fusion dès INIT, lock validé
contre les exigences fusionnées (`getMissingRequirementInfo`, code 4).
Décision, avec l'utilisateur : vivacity émule la fusion (`ExtraPackage` /
`PluginState` portés dans `vivacity-resolver::merge_plugin`, contraintes
des doublons **structurées** — une conjonction textuelle `"^2.11 || ^1.0,
^2.8"` se lit `^2.11 || (^1.0, ^2.8)` et accepterait un lock que Composer
refuse, vérifié) et valide le lock comme Composer le fait avec le plugin
installé ; elle **ne lance jamais** la mise à jour implicite : sur un
vendor vierge, un lock qui ne satisfait pas les exigences fusionnées est
rendu à Composer avec la raison, et `update` / `require` / `remove` sont
refusés sur un projet qui configure le plugin (leur pool n'a pas encore
les exigences et dépôts fusionnés). Parité prouvable : à dépôts figés, la
mise à jour implicite ne trouve rien et le résultat de Composer est celui
de vivacity ; le harness `merge-plugin.sh` prend pour référence l'état
stable (plugin pré-amorcé seul — un premier `install --no-plugins` laisse
un résidu de disposition avec composer/installers, vérifié —, `autoload.php`
de l'amorce retiré car Composer reprend le suffixe d'un autoload existant
avant celui du lock, lock inchangé vérifié). Précédent : l'ordre de
`pest-plugins.json` et de `files` sur un cycle — là où Composer n'est pas
reproductible, vivacity écrit la forme déterministe et le dit. Trouvé au
passage : le rapport post-install (abandoned + funding) absent d'un
`install` réel ; le filtre `--no-dev` de l'autoloader par nom vs par
atteignabilité selon ce que le dépôt local connaît ; la racine candidate
avec ses replace/provide dans `getMissingRequirementInfo`.

## 2026-09-17 — Aucune bibliothèque C hors PCRE2 : bzip2/LZMA en Rust pur, pas de xz/zstd

Constat : PCRE2 préfixée, le run E2E suivant d'ePHPm (0.11.0 épinglée)
tombe sur `duplicate symbol: BZ2_bzDecompressInit…` — `zip` en features par
défaut tire `bzip2-sys`, `lzma-sys` (xz) et `zstd-sys`, et `libphp.a`
embarque libbz2 (`ext/bz2`), libzip et ses compresseurs. Décision : `zip`
sans features par défaut — `deflate` (miniz_oxide), `deflate64`, `bzip2`
sur le backend Rust `libbz2-rs-sys` (exports déjà préfixés
`LIBBZ2_RS_SYS_v0.1.x_`), `lzma` (lzma-rs), `time` ; ni `xz` ni `zstd`
(Info-ZIP `unzip`, l'extracteur de Composer, ne les lit pas non plus).
Preuve : `nm` du binaire = 0 symbole `BZ2_`/`lzma_`/`ZSTD_`/`pcre2_`
non préfixé ; le test de lien définit aussi les `BZ2_*` étrangers et fait
un aller-retour bzip2 (vérifié : avec `bzip2-sys`, `BZ_CONFIG_ERROR` des
dummies → échec immédiat, `mem.rs:123`) ; diff-vendor, vendor-dir,
path-repos inchangés. Règle retenue : vivacity ne livre qu'une seule
bibliothèque C, la sienne, préfixée — tout ajout de `*-sys` C passe par
le test de lien.

## 2026-09-17 — PCRE2 compilé avec des symboles préfixés, jamais la lib système (v0.12)

Constat : ePHPm (ephpm/ephpm#523, `ephpm composer` = `vivacity::run` en
process) échoue à l'édition de liens — `duplicate symbol:
_pcre2_check_escape_8…` — parce que `libphp.a` embarque la PCRE2 de PHP et
que `pcre2-sys` compile la sienne quand pkg-config ne trouve pas
`libpcre2-8`. Constat 2 (méta-analyse) : nos binaires de release 0.10.0
macOS arm64 et Linux x86_64 étaient liés **dynamiquement** à une PCRE2
système (Homebrew `libpcre2-8.0.dylib`, `libpcre2-8.so.0`) — un Mac sans
`brew install pcre2` ne les démarrait pas. Options écartées : remplacer
PCRE (le `JsonManipulator` de Composer vit de sous-motifs récursifs
`(?&json)`, le scanner de classes de lookbehind + possessifs en mode octets
— aucun moteur Rust pur ne les a, et la parité est prouvée contre PCRE) ;
`objcopy --prefix-symbols` (préfixe aussi les indéfinis, inexistant sous
MSVC) ; demander à l'hôte `--with-external-pcre` (déplace le problème).
Décision : deux crates à nous, `vivacity-pcre2-sys` (pcre2-sys 0.2.10 +
PCRE2 10.46, toujours compilée depuis la source vendorée, `PCRE2_SYMBOL_PREFIX=vivacity_`
par un patch de 16 lignes de `pcre2.h` — les DEUX définitions de
`PCRE2_SUFFIX` — et de `pcre2_internal.h` — six symboles indépendants de la
largeur — ; bindings en `#[link_name]`) et `vivacity-pcre2` (pcre2 0.2.11
intact). Un `[patch]` ne se propage pas aux consommateurs crates.io, d'où
des crates publiées, versionnées avec le workspace (base amont dans la
description). Preuves : `tools/check-pcre2-symbols.sh` (137/137 globaux
préfixés, en CI Linux/macOS), `crates/pcre2-link-test` (un objet C
définissant `pcre2_compile_8`, `_pcre2_check_escape_8`, `pcre2_match_8`
lié à plat par `rustc-link-arg-tests` — vérifié : sans préfixe, doublons
sous GNU ld/lld, masquage silencieux sous ld d'Apple attrapé par
l'assertion de match ; avec, vert), l'assertion « pas de PCRE2 dynamique »
dans release.yml, et la suite de harness rejouée (le moteur exercé sur
macOS/Linux passe de la lib système à la 10.46 vendorée). Le patch est
proposable à rust-pcre2 tel quel (sans la macro, les symboles sont
identiques à l'octet à la base).

## 2026-09-16 — L'ordre de `files` chez Composer dépend de l'ordre de son dépôt local (cycles)

Constat, sur wallabag (`--no-dev`) devenu natif avec `bin-dir` : le
`autoload_files.php` d'un `composer install` frais et celui de
`composer dump-autoload` lancé juste après **sur le même arbre** diffèrent
(`symfony/polyfill-ctype` avant/après `hoa/consistency`). Cause :
`PackageSorter::sortPackages` calcule le poids d'un paquet récursivement
sur ses utilisateurs et **tronque un cycle là où il le rencontre**
(`$computing[$name]` → 0) ; les paquets `hoa/*` se requièrent en cycle, donc
le résultat dépend de l'ordre d'itération du package map — l'ordre du dépôt
local en mémoire à l'installation (ordre d'exécution des opérations, lié à
l'achèvement des extractions) contre l'ordre d'installed.json (trié par
nom) au `dump-autoload`. vivacity écrit la forme de `dump-autoload`, la
seule reproductible. Décision : le corpus (`harness/corpus.sh`) retente la
comparaison après un `composer dump-autoload` de la référence (mêmes
`--ignore-platform-req`, même `--no-dev`) et compte l'entrée native
**annotée** quand seule cette différence subsistait ; la promesse reste
« identique à Composer », précisée en « à sa forme déterministe » sur ce
point. Candidat à un rapport upstream (install ≠ dump-autoload sur un
arbre à cycles).

## 2026-09-16 — `vendor-dir` / `bin-dir` : la forme relative normalisée, les autres formes refusées (v0.11)

Composer résout `vendor-dir` et `bin-dir` (`Config::get`) en absolu sans
canonicaliser (`baseDir . '/' . valeur`), puis chaque consommateur fait
`realpath` avant d'en dériver un chemin écrit (`LibraryInstaller`,
`BinaryInstaller`, `AutoloadGenerator`, `FilesystemRepository`). Décision :
vivacity garde la forme **relative au projet, normalisée** (`lib/vendor`,
`vendor/bin`) dans `dirs::Dirs`, portée par `Layout` ; tout ce qui est
dérivé (`install-path`, `$vendorDir`/`$baseDir`, proxies) passe par les
mêmes `findShortestPath*` que Composer sur des chemins canoniques, donc les
octets sont les mêmes (harness `vendor-dir.sh`, 6 variantes dont
`./vendor/composer/vendor` et `src/vendor` sous un PSR-4 scanné en `-o`).
Précédence identique : `COMPOSER_VENDOR_DIR` / `COMPOSER_BIN_DIR` (vide =
absent), `config` du projet, config globale, défaut `{$vendor-dir}/bin` ;
seul le placeholder `{$vendor-dir}` est substitué. Les formes que le
harness ne peut pas couvrir — absolu, `..`, la racine elle-même, `~/`,
`$VAR` / `%VAR%`, autres placeholders — sont **refusées** (issue de layout,
repli Composer) plutôt que supportées sans preuve : Composer y produit des
`install-path` et un `$baseDir` absolus (`Filesystem::findShortestPath`
quand le préfixe commun est `/`), qu'aucune fixture ne compare ; aucun des
105 projets du corpus ne les emploie. Retenu de la méta-analyse : un
`bin-dir` peut être le répertoire du projet (`bin/console`, `bin/phpunit`
de la fixture symfony) — le proxy d'un fichier existant est *sauté* avec le
message de Composer (`Skipped installation of bin … name conflicts with an
existing file`, silencieux sur la passe de présence), la purge ne touche
plus que les proxies des paquets retirés ou mis à jour (`removeBinaries`,
`rmdir` si le répertoire se vide), et le harness tourne `--no-fallback`
pour qu'un site oublié échoue au lieu d'être rendu à Composer.

## 2026-09-16 — Le corpus comme étalon de « prêt au quotidien » (v0.10)

Fait : six fixtures prouvent la parité là où elle s'applique, pas la
fréquence à laquelle elle s'applique ; « quand vivacity peut-il servir
quotidiennement avec Composer en fallback ? » n'avait pas de chiffre.
Décision : un corpus de projets réels épinglés (`fixtures/corpus/`, json +
lock + arbre minimal de l'autoload, résolus une fois par Composer pour les
gabarits `create-project` qui ne commitent pas de lock), un scan hors ligne
(`install --check-scope`) qui décide le seau fallback sans réseau, et le
double install seulement sur les prévus natifs, dans les deux modes dev et
`--no-dev`, avec un inventaire des modes et des liens. La référence est
nommée : Composer 2.10.3 `--no-scripts`, **plugins actifs** — la méta
avait affirmé que `--no-scripts` coupe aussi les écouteurs de plugins ;
le corpus l'a réfuté (Flex, dealerdirect, pest-plugin ont écrit), et la
lecture confirme : `Factory` ne coupe que `EventDispatcher::runScripts`
(les scripts de composer.json). Conséquence assumée : la liste
`BENIGN_PLUGINS`, qualifiée contre `--no-plugins`, n'est pas « bénigne »
sous cette référence (trois plugins écrivent des fichiers) ; le rapport
les compte en `diff` plutôt que de les cacher, et leur émulation est le
prochain port classé par fréquence. Décision (même jour) : ces plugins
sortent de la liste tant qu'ils ne sont pas émulés — un projet qui les a
est rendu à Composer avec la raison, jamais déclaré natif avec un fichier
manquant — et la référence des fixtures passe à plugins actifs partout où
le manifeste en autorise (c'est la seule qui puisse qualifier une
émulation). Prix accepté : sylius et rector quittent le natif jusqu'à
l'émulation des extension-installers ; le corpus en dev tombe de 36 à 35
natifs et à zéro diff. Les runs sont locaux et avant chaque
tag ; pas de hebdo, pas d'issue automatique (entrées épinglées : seul le
réseau bouge). Le premier run a rendu six corrections avant tout rapport
(plateforme `lib-*`, zéros non significatifs et `dev-master` dans
`version_normalized`, `%prettyVersion%` des urls de dist, le gabarit de
`symfony/runtime`, les `symfony-pack` de Flex, `apcu-autoloader`) — le
corpus trouve ce que six fixtures ne pouvaient pas.

## 2026-09-16 — Dépôts `path` : la capture d'abord, le miroir jusqu'aux modes (v0.9)

Fait : la méta-analyse du plan (docs/plans/v0.9-path-repositories.md, §3)
a trouvé quatre failles majeures que la lecture du PHP a confirmées — un
plafond git posé sur le répertoire du projet cache le dépôt du projet aux
paquets (`GIT_CEILING_DIRECTORIES` n'inspecte jamais le plafond lui-même),
la mise à jour d'un paquet symlinké imprime `: Source already present`
(l'appendice est calculé sur le chemin courant de vendor/, avant le
retrait), `diff -r` ne voit ni les modes que Symfony `copy` pose
(`0666 & ~umask | bits x`) ni un `.git` copié à tort (que `--exclude=.git`
masquait), un lien vers un dossier non vide disparaît du miroir
(`accept()` suit `isDir()`). Décision : la fixture et les captures de
Composer (lock, inventaire `stat`, stderr de chaque étape) précèdent tout
Rust ; le harness dédié `harness/path-repos.sh` compare vendor/ par
`diff -r --no-dereference` ET par un inventaire des modes et cibles de
liens, sans exclure `.git` ; les harnais fixent le plafond git au parent du
projet et le projet de la fixture vit sur `develop` pour que la remontée
vers le dépôt du projet soit distinguable du repli `dev-main`. `glob()` et
`serialize()` sont écrits à la main (≈ 250 et 90 lignes, oracles PHP)
plutôt qu'importés : la sémantique à reproduire est celle de la libc et de
PHP (ordre des alternatives d'accolades, `GLOB_PERIOD` absent, clés
entières), pas celle d'une crate. Les lignes d'opérations d'une
installation réelle sont imprimées après la transaction (les placements
sont parallèles depuis la PR #4 ; Composer les imprime une à une avant
chaque opération) : même texte, même ordre, un échec n'imprime que
l'erreur ; la ligne `vivacity: …` reste et les harnais la tolèrent.
Windows : jonctions non portées, un lock avec un paquet `path` part en
fallback (le lock, lui, est identique). Revue indépendante (21 constats) :
les quatre majeurs corrigés — une mise à jour d'un paquet `path` retire
d'abord l'ancienne disposition (`FileDownloader::update`), le parsing de
`(HEAD detached from X)`, `vendor/bin` seulement hors purge, et le
**dépôt local** : installed.json et l'autoloader sont désormais produits
depuis les entrées d'installed.json pour les paquets inchangés et depuis
le lock pour les autres (Composer garde les objets chargés ; avec une
référence HEAD ou `none`, une édition non commitée change le lock sans
changer l'identité) — l'ancien modèle « état = lock » était faux dès que
les deux divergent, ce que seuls les dépôts `path` rendent routinier. Le guesser git ne fixe plus
`GIT_DIR` : la remontée est celle de Composer (un projet dans un dépôt
parent prend sa branche), et c'est le plafond du harnais qui protège
l'oracle du checkout de vivacity.

## 2026-09-15 — Parité de stderr par défaut, et `--dry-run` par un patch du paquet racine (v0.8)

Fait : en portant `--dry-run`, la lecture de `Installer::doUpdate` a montré
que Composer imprime toujours une ligne par opération de lock (`  - Locking
…`), puis un compte de suggestions, un avertissement par paquet abandonné
et la ligne funding — rien de tout ça n'était imprimé par vivacity, et
aucun harness ne le voyait (seul le lock était comparé). Décision : le
harness `steps.sh` compare stderr **par défaut** (de la première ligne
d'opération ou d'en-tête à la fin ; `@nostderr` pour les cas rendus par
Symfony, `--no-update`, `config.lock: false`, un texte d'erreur réseau),
et la parité de sortie devient une propriété mesurée, pas une intention.
Pour `require`/`remove --dry-run`, Composer patche le paquet racine en
mémoire (`array_merge` des liens, non trié) au lieu d'écrire le fichier ;
réécrire puis restaurer composer.json aurait donné le même fichier mais un
ordre de règles différent (`--sort-packages`), donc potentiellement
d'autres décisions du solveur : `RootPatch` est appliqué au chargement de
la session (méta, faille 7). La phase d'installation d'un dry run lit le
lock *non écrit* de la résolution (`Locker::setLockData` le garde en
mémoire) — sans quoi la liste des opérations montre l'ancien lock (trouvé
par l'oracle `@dry-install`). Reporté en v0.9 : les dépôts `path`
(six sous-ports préalables identifiés par la méta : `serialize()`, un
guesser « comme git », `ArchivableFilesFinder`, glob à accolades,
`findShortestPath` avec `preferRelative`, dépôts git imbriqués dans les
fixtures).

## 2026-09-15 — Parallélisme d'E/S par plateforme (PR #4 de Luther Monson)

Fait : la PR parallélise le scan de classmap (répertoires sur rayon) et la
matérialisation store→vendor, avec des gains mesurés sur ext4/WSL2 (sylius :
`dump-autoload -o` à froid 1,03 s → 0,48 s, `install` vendor effacé 3,75 s →
1,68 s ; ×2). Mesuré ici sur APFS (M4 Max) : le même scan passe de 749 ms à
901 ms (temps système 0,47 s → 8,4 s) — c'est la contention de lecture que
M5 avait mesurée (3-4×) et qui avait fait choisir des lectures séquentielles
et une détection CPU sur threads std, sans rayon. Décision : garder le
parallélisme de la PR **là où il paie**. `vivacity_core::platform::parallel_io()`
vaut vrai sur Linux, faux ailleurs (`VIVACITY_PARALLEL_IO=0|1` pour forcer) ;
il gouverne les répertoires en parallèle du scan et le fan-out de la
matérialisation ; la détection de classes reste parallèle sur le CPU partout.
Résultat : Linux (conteneur, 4 vCPU, overlay) scan à froid 550 → 340 ms,
install vendor effacé 806 → 374 ms ; macOS scan 768 → 697 ms (la détection
sur rayon), install 604 → 623 ms (bruit). rayon est adopté (5 crates, épinglé
`=1.12.0`) : un pool global au lieu d'un jeu de threads scoped par
répertoire, et le même outil pour les deux fan-outs — l'entrée M5 « pas de
dépendance rayon » est amendée par celle-ci. Le reste de la PR est fidèle à
Composer : écriture des fichiers générés seulement si les octets changent
(`filePutContentsIfModified`), reflink FICLONE sur Linux (prévu depuis le
plan r3, jamais implémenté — désactivé pour le run dès le premier
ENOTTY/EOPNOTSUPP/EXDEV/EINVAL), cache de classmap en binaire v2
(auto-invalidation par `CACHE_FORMAT`, fichier tronqué ou octets en trop
→ rescan, compteurs lus jamais pré-alloués). Corrigé en revue : les proxies
de `vendor/bin` — `Installer::run` appelle `ensureBinariesPresence` sur
chaque paquet installé à chaque run, donc un proxy manquant d'un paquet
inchangé est recréé (un `vendor/bin` effacé revient), un proxy existant est
laissé ; `libc::FICLONE` (la constante codée en dur ne compile pas sous musl).

## 2026-09-18 — Gate de perf en ratio, pas en secondes ; sonde de plateforme en cache

Fait : un runner GitHub varie de 30 à 45 % d'un run à l'autre sur un code
identique (mesuré chez svandragt/vivace, bench/compare.py — leur idée, reprise
telle quelle) ; une baseline en secondes déclenche sur le runner, pas sur le
code. Décision : `bench/gate.py` compare `médiane vivacity / médiane Composer`
par scénario (no-op, warm, dump -o), mesurées dans le même job — la vitesse
du runner se simplifie dans le ratio — contre `bench/results/baseline-ratio.json`,
tolérance 15 % et marge absolue de 5 ms (un no-op à 10 ms franchit toute
tolérance relative sur un aléa d'horloge). Baseline = médiane de plusieurs
runs CI (`--merge`), jamais un run seul. Écarté : un seuil absolu (chasse le
runner), un benchmark sur machine dédiée (pas de machine).

Constat en chemin : depuis `3364470` (v0.8), `install` sondait php à chaque
run (`platform::probe`, 30–60 ms) — le cache de `vivacity_core::platform::detect`
n'avait plus d'appelant. Décision : cache de la sonde clé sur ce dont le résultat
dépend (binaire php, fichiers ini chargés/scannés + répertoire de scan, variables
PHPRC / PHP_INI_SCAN_DIR / XDEBUG_MODE / XDEBUG_CONFIG), pas sur le seul binaire
comme `detect` le faisait (une extension activée dans php.ini doit être vue).
Non copié de viv : son no-op ne régénère pas l'autoloader (état = content-hash +
sha256 de composer.json) — un fichier ajouté dans un dossier `classmap` racine
n'est pas vu là où `composer install` le voit (vérifié sur viv 0.14.0) ; vivacity
garde le contrat de Composer et paie le dump (~50 ms avec le cache de classmap).

## 2026-09-18 — P1 : la requête de listes recouvre le travail local (plan v0.15-perf-install)

Fait : un `install` depuis le lock attend une requête conditionnelle (résumé
des listes de blocage, `If-Modified-Since` → 304 ; `packages.json` en cache
600 s — `loadFilterSummary` / `loadRootServerFile(600)` de Composer) avant
de faire quoi que ce soit d'autre : 73–92 ms sur le conteneur Linux arm64,
100–200 ms sur le Mac selon l'heure, pour 30–47 ms de travail réel hors ligne.
Décision : la vérification tourne sur un thread ; plateforme, transaction
(`LocalRepoTransaction`) et analyse de scope sont calculées en silence
pendant l'attente ; jointure avant toute impression ou écriture, puis le
rapport dans l'ordre de `Installer::doInstall` (avertissements de politique,
problèmes → code 2, plateforme → code 4, exigences manquantes, opérations,
notes de scope). Mesuré (laravel, `--no-plugins --no-scripts`) : trace Linux
scope = jointure (75,6 ms → 75,6 ms : ~10 ms recouverts), Mac 13–18 ms ;
hyperfine no-op en ligne Linux 128 → 117 ms (σ 37–43 : dans le bruit
réseau), warm 128 vs 131 (idem). stderr identique (diff), 286 cas de harnais
0 échec, 214 tests. Écarté : lancer la requête avant `pre-install-cmd` (un
script peut changer auth.json/config) ; un TTL sur le résumé (contrat :
Composer revalide à chaque install — décision séparée).

## 2026-09-18 — P2 : le dump de l'autoloader, profilé plutôt que supposé (plan v0.15-perf-install)

Fait : le plan supposait que les ~50 ms de dump sur laravel (Mac, `-o`)
venaient du scan psr-4 racine non caché. Un échantillonnage (`sample` sur
une boucle in-process de 40 dumps) a montré autre chose : 28 % du dump dans
`std::path::compare_components` — le set des chemins réels déjà pris
(`$this->scannedFiles`) était un `BTreeSet<PathBuf>`, comparé composant par
composant à chaque `contains`/`insert` sur 6 849 fichiers ; puis
`replace_bytes` (substitution `__DIR__` sur le var_export de ~1 Mo, un
`starts_with` par octet et par motif), `normalize_path` réalloué pour chaque
fichier déjà normalisé, `format!("{vendor}/")` par classe dans `getPathCode`,
la regex d'exclusion recompilée et `literal_prefix` recalculé pour chacun des
~250 jobs d'un `-o`. Décisions : `HashSet<Vec<u8>>` sur les octets du chemin ;
`memchr::memmem` pour la substitution ; `normalize_path_cow` /
`is_normalized_absolute` (identité sans allocation quand le chemin est déjà
normalisé — les fichiers scannés le sont tous) ; regex et préfixes littéraux
mémorisés par texte de motif. Le cache par fichier hors store
(`FileCacheSlot`, clé chemin canonique, entrée (mtime ns, taille, classes),
répertoire relu à chaque scan) est livré aussi : il ne pèse pas sur laravel
(les sources racine sont petites) mais couvre un paquet `path` ou un vendor/
que le store ne connaît pas ; `harness/root-scan.sh` (9 étapes : ajout,
classe en plus, réécriture même taille à la seconde suivante, sous-répertoire,
suppression, psr-4 sous `-o`) le compare à Composer à chaque pas.
Mesuré (laravel, `optimize-autoloader: true`) : `dump-autoload` Mac 61,5 →
28,1 ms ; Linux (conteneur arm64) `dump -o` 39,7 → 19,4 ms, no-op `--offline`
53,2 → 34,7 ms, warm `--offline` 72,5 → 53,1 ms. Sortie identique à l'octet
(diff-vendor `--with-autoloader` cache froid puis chaud, 298 cas de harnais).
Reste dans le dump : chargement des ~250 caches de store (`scan_only`),
var_export + réindentation de la statique, `installed.json` parsé en `Value`.

## 2026-09-18 — P3 refusé : pas de raccourci du dump sur empreinte (plan v0.15-perf-install)

Fait : le raccourci proposé (sauter le dump de l'autoloader quand une
empreinte des entrées du dump précédent correspond) a été soumis à une
méta-analyse par un relecteur frais, lecture du code à l'appui. Entrées du
dump absentes de l'empreinte, chacune avec une divergence concrète :
les avertissements du dump (« Ambiguous class resolution », violations PSR,
doublons `files`) que Composer réimprime à chaque no-op — un hit les
tairait, stderr diverge (`lib.rs`, impression de `report.warnings`) ; un
dossier psr-4 absent au dump précédent puis créé (`generator.rs`, `is_dir`
→ `continue`) ; le `config.json` global (`allow-plugins` gouverne
`autoload_runtime.php`) ; le gabarit `extra.runtime.autoload_template` lu au
dump ; `vendor/composer/LICENSE` et `include_paths.php` parmi les sorties ;
les chemins canoniques de `$baseDir`/`$vendorDir` ; un préfixe apcu tiré au
sort par dump. Et un fait de coût : après P2 un scan chaud hors store est
déjà stat-only, le hit coûterait à peu près le scan ; ce qui reste dans le
dump (~250 caches de store chargés, var_export + réindentation de la
statique, `installed.json` en `Value`) se réduit sans toucher au contrat.
Décision : **refusé**. Raison de fond : une empreinte n'est jamais complète
par construction — toute lecture ajoutée à `dump` devra y être reportée à la
main, et le harnais ne teste que les entrées qu'on a imaginées ; c'est le
défaut du raccourci « content-hash » déjà écarté, à grain plus fin. Gain en
jeu : ~15–20 ms hors ligne seulement. À la place : (b) un cache de store
consolidé par lock et une statique émise directement dans sa forme finale ;
(c) le dump calculé en mémoire pendant l'attente réseau et écrit après la
jointure (`write` compare déjà les octets). Puis la seule décision restante
est celle de la requête de listes elle-même (revalidation par install, comme
Composer, ou TTL) — décision de contrat, à prendre avec les chiffres.

## 2026-09-18 — P3b/P3c : mesures négatives et positives (plan v0.15-perf-install)

P3b, statique en une passe : `static_property` réécrit pour émettre
`autoload_static.php` directement dans sa forme finale (une passe au lieu de
var_export → substitution → réindentation). Sortie identique, tests verts —
et **aucun gain mesurable** (hyperfine dump laravel Mac 28,1 → 28,0 ms) : la
substitution par `memmem` (P2) avait déjà retiré ce que ces passes coûtaient ;
ce qui reste dans la phase « static » est `php_str` et `absolute_value` par
classe. Décision : **revenu en arrière** (deux implémentations pour zéro
gain, c'est du code en plus). Le cache de store consolidé n'a pas été fait
non plus : la phase « scan » chaude est à 4 ms sur Mac pour ~250 fichiers,
le coût est dans les allocations par fichier, pas dans les ouvertures. Le
profil du dump est plat désormais (scan 4 · merge 5 · statique 4 · classmap
3 · JSON 5 · jobs 2 ms sur Mac) : chaque poste vaut 2–5 ms.

P3c, le dump planifié pendant l'attente réseau : `generator::dump` scindé en
`plan` (tout ce qui lit — manifestes, scans, `autoload.php` existant pour le
suffixe, caches de classmap qu'il peut écrire — rien d'écrit sous vendor/) et
`DumpPlan::commit` (les écritures dans l'ordre historique, `write` comparant
les octets, suppressions si présents). Dans `run_install`, quand la
transaction est vide, sans `--dry-run`/`--no-autoloader`/`--run-scripts`, le
dépôt local que l'install produira est calculable avant lui
(`installer::local_repository_if_unchanged` : chaque paquet voulu présent
dans installed.json à la même identité, répertoire en place) : le plan est
calculé avant la jointure de la requête de listes, gardé en `Result` et
consommé là où le dump tourne aujourd'hui (une erreur sort au même point,
même texte), puis `commit` après l'install. Mesuré (laravel, no-op en ligne) :
Mac 141,7 → 116,4 ms, Linux 106,5 → 94,7 ms — trace : plan terminé à 30–44 ms,
requête revenue à 82–100 ms, le dump est sorti du chemin critique. Hors
ligne +1,6 ms sur Linux (34,9 → 36,5 : installed.json lu une fois de plus —
P4 le retire). stderr identique, 311 cas de harnais, 214 tests.

## 2026-09-18 — P4 : un parse par fichier JSON et par processus (plan v0.15-perf-install)

Fait : `installed.json` (~1 Mo sur laravel) était lu et parsé en `Value` par
le scope, le layout, la transaction, l'installer (deux fois), l'émulation
pest et le dump, dans le même run ; `~/.composer/config.json` par chaque
`global_config_value` (sept appelants). Décision : `vivacity_core::jsonfile::read`
— un cache par processus, clé chemin, validé à chaque lecture sur
(mtime ns, taille) du fichier : un fichier réécrit en cours de run
(installed.json par l'installer) est reparsé, un fichier disparu répond
absent. Écarté : passer la valeur de main en main (sept signatures à
changer pour le même effet). Mesuré (laravel, Mac) : no-op `--offline`
51,0 → 50,1 ms (la lecture ajoutée par P3c annulée et au-delà),
`dump-autoload` 28,1 → 25,8 ms. 215 tests, 299 cas de harnais.

Bilan de la journée sur le no-op laravel (Mac, M4 Max) : 163 ms (matin) →
50 ms hors ligne, 116 ms en ligne dont ~100 d'attente réseau ; Linux
(conteneur arm64) 103–125 → 95 ms en ligne, 35 hors ligne. La seule
décision qui reste pour le no-op en ligne est celle de la requête de listes
(revalidation à chaque install, comme Composer, ou TTL).

## 2026-09-19 — La requête de listes reste une revalidation par install (pas de TTL)

Fait : depuis Composer 2.10, `install` depuis un lock passe le pool par le
filtre des listes de blocage (`createFilterListPoolFilter(BLOCK_SCOPE_INSTALL)`,
liste `malware` de Packagist) : `packages.json` pris en cache sous 600 s
(`loadRootServerFile(600)`), le résumé des listes (`filter-summary.json`,
quelques Ko) revalidé à **chaque** install par une requête conditionnelle
(`If-Modified-Since`, 304). C'est la seule barrière post-lock : une version
verrouillée ajoutée à la liste après l'écriture du lock fait refuser
`composer install` (code 2) sans que le lock ait changé. Coût mesuré
(2026-09-18) : 70–100 ms sur le conteneur Linux, 100–200 ms sur le Mac —
un aller-retour TLS complet, rien de réutilisé d'un processus à l'autre ;
après P1–P4 c'est ~90 % du no-op en ligne (Linux 95 ms dont 35 de travail,
Mac 116 dont ~50).

Options examinées : (A) revalider à chaque install ; (B) TTL sur le résumé
— pendant la fenêtre, `vivacity install` installe une version que
`composer install` refuserait, première exception *volontaire* au contrat et
précisément sur le cas malware, invisible aux harnais (qui comparent contre
Composer maintenant, pas contre une liste qui change entre deux runs), sans
gain en CI (cache vide à chaque runner) ; l'argument « Composer cache déjà
600 s » ne tient pas : Composer cache le gros fichier de découverte et
revalide toujours le petit fichier qui décide ; (C) stale-while-revalidate
— même divergence bornée à un install, plus une requête en vol à la fin du
processus ; (D) opt-in `VIVACITY_LIST_TTL` — un bouton de plus alors que
`--no-blocking` / `--no-security-blocking`, `config.policy.malware.block` /
`block-scope` et `--offline` donnent déjà la vitesse **avec la sémantique
de Composer** (no-op laravel Mac ≈ 50 ms avec `--no-blocking`, mesuré) ;
(E) raccourcir la requête elle-même sans toucher au contrat : reprise de
session TLS 1.3 entre processus (tickets rustls persistés), une seule
connexion, pas de résolution DNS superflue — 20–40 ms par install, et le
seul levier qui vaut aussi pour le premier install et la CI.

Décision : **A, et E comme piste de code.** Le contrat n'a pas d'exception ;
celle-ci porterait sur la sécurité. Ce qu'on documente : les drapeaux et la
config qui coupent la vérification existent chez Composer et ont les mêmes
conséquences ici, avec les chiffres. Si un jour un TTL est voulu, ce sera D
(opt-in nommé), jamais un défaut. E entre dans le plan v0.15 comme P5, à
mesurer avant de croire au chiffre.

## 2026-09-19 — Le gate de perf : ce qu'un ratio simplifie, et ce qu'il ne simplifie pas

Fait : quatre runs CI sur un code identique ont donné pour le no-op sylius
446–926 ms chez Composer et 104–140 ms chez vivacity. Le ratio ne se
simplifiait pas — la requête de listes (latence réseau fixe) dominait notre
no-op, le CPU dominait celui de Composer ; deux runs sur quatre auraient
échoué à 15 %. Le ratio de compare.py (svandragt/vivace) marche pour un
no-op de 4 ms de CPU, pas pour le nôtre. Décision : `bench/ci-bench.sh`
passe `--no-blocking` aux deux outils (le même drapeau, la même sémantique
— la requête est un sujet à part, DECISIONS 2026-09-19 ci-dessus) : les deux
côtés deviennent CPU-bound et déterministes (Mac : Composer 1,08 s → 0,68 s
σ 21 ms, vivacity 134 → 49 ms σ 0,4). Rejoué : dispersion des ratios sur
quatre runs identiques 3–12 % pour no-op et `dump -o`, mais 18–32 % pour
warm (40 000 fichiers écrits : le disque du runner, pas son CPU). Décision :
**no-op et dump -o gardés, warm rapporté sans garde**. Baseline =
médiane des quatre runs `8533a09`, committée.
Trouvé en chemin : les quatre premiers runs « verts » n'avaient produit que
laravel no-op/warm — `composer dump-autoload --no-blocking` n'existe pas, le
script mourait derrière un `| tee` qui avalait le code de sortie. Corrigé
(`shell: bash` = pipefail, hyperfine bruyant) et le gate **échoue si un des
neuf fixture × scénario manque** : un bench mort à mi-chemin ne vaut pas
« rien n'a régressé ».

## 2026-09-19 — Le classmap suit l'ordre `readdir`, comme le Finder de Composer

Fait : `ClassMapGenerator::scanPaths` itère `Finder::create()->files()->followLinks()->in($path)`
**sans tri** — `RecursiveIteratorIterator::SELF_FIRST` sur
`RecursiveDirectoryIterator`, l'ordre brut de `readdir()`, profondeur d'abord,
un sous-répertoire descendu là où readdir le liste. M3 avait choisi un tri par
nom (déterminisme) en notant que « seul le gagnant d'une ambiguïté en dépend ».
Or ce gagnant est observable (`autoload_classmap.php`), les avertissements
« Ambiguous class resolution » et les violations PSR le sont aussi (stderr),
et l'ordre réel diffère du tri par octets partout (APFS et ext4 hachent, NTFS
trie insensible à la casse). Décision : `walkdir` sans tri — la même séquence
que PHP sur le même répertoire — `CACHE_FORMAT` v3. Pour les sources du projet
(là où les doublons existent : du legacy avec des copies), c'est le répertoire
même que Composer scanne : exact sur tout FS. Pour un paquet scanné dans le
store, l'ordre est celui du store, pas de vendor/ — un doublon *interne* à un
paquet avertirait tous ses utilisateurs, cas non rencontré dans le corpus ;
l'ordre *entre* paquets ne dépend pas du FS. Écarté : reproduire l'ordre de
création des petits répertoires ext4 (l'ordre d'extraction du zip) — Composer
sur deux machines ne s'accorde pas non plus, le contrat est Composer sur la
même machine.
Trouvé en chemin, corrigé ensemble : le filtre par défaut de
`getAmbiguousClasses` (`{/(test|fixture|example|stub)s?/}i` : un doublon sous
tests/ n'est pas signalé) n'était pas porté ; l'ordre des avertissements est
l'ordre de découverte (tableau PHP), pas l'ordre des noms ; la formulation
« was found 3x: in » au-delà de deux fichiers ; la ligne « To resolve
ambiguity … exclude-from-classmap » ; les violations PSR sans préfixe
« Warning: » et avec le cwd (= le projet, Composer fait `chdir`) remplacé par
`.`. `harness/root-scan.sh` compare désormais ces lignes (trois fichiers
ambigus, une violation PSR-4) : 12/12, steps.sh 0 diff de stderr.

## 2026-09-19 — Tolérance du gate à 25 %

Fait : premier run gardé après la baseline (`c0b6313`, l'ordre readdir) :
sylius/`dump -o` à 0,056 contre 0,048 — +16,7 %, +19 ms — alors que le même
changement mesuré ici donne 69,8 → 70,1 ms (bruit). Le runner était rapide :
Composer 2 476 ms au lieu de 3 165–3 236 sur trois des quatre runs de
baseline, vivacity 138 au lieu de 150–157 ; nos ~140 ms sont pour moitié
du disque (lecture des caches, écriture de 800 Ko), qui ne suit pas le CPU du
runner. Le ratio garde donc une dépendance résiduelle au runner même sur les
scénarios « CPU ». Décision : tolérance 25 % (deux fois la dispersion mesurée
sur quatre runs identiques, 3–12 %), marge absolue 5 ms inchangée. Un gate
qui sonne à faux est pire qu'un gate large : il finit ignoré.

