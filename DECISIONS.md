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

## 2026-09-20 — Le coût de l'ordre readdir, mesuré, et la baseline refaite

Fait : après `c0b6313` (ordre readdir), trois runs CI consécutifs mettent le
no-op sylius à 0,274–0,304 contre 0,237–0,266 sur les quatre runs de baseline
(+12 à +27 %), et `dump -o` sylius à +16–21 %. Mesuré hors runner : Mac
62,9 → 63,4 ms (bruit) ; conteneur Linux (ext4-like, ordre de hachage) no-op
56,5 → 60,0 ms (+6 %, σ 2–3 ms), `dump -o` 60,5 → 61,9 (+2 %). Le code
explique au plus 3 ms ; le reste est le runner du jour sur le scénario le
plus sensible au disque (~450 caches de store lus). Décision : le coût est
accepté — c'est un correctif de contrat (le gagnant d'une classe ambiguë et
les avertissements sont ceux de Composer), pas une optimisation à défaire —
et la baseline est refaite sur le code courant (médiane de quatre runs sur
`b128c23`). Trouvé en chemin : la version racine des fixtures venait du
dépôt vivacity lui-même (git remonte) — `0.16.0` sur un tag, d'où un conflit
`rector/rector <2.0` sur le bench lancé sur `v0.16.0` ; `fixtures/make.sh`
donne maintenant un dépôt git à chaque fixture, comme les harnais.

## 2026-09-20 — Le gate de perf ne fait plus échouer le job

Fait : six faux positifs en deux jours, à 15 % puis 25 % ; sur trois runs
identiques lancés ensemble, Composer va de 698 à 995 ms sur le no-op laravel
(runners différents) quand vivacity va de 55 à 85 — nos 50–150 ms sont pour
moitié du disque et des sous-processus (git pour la version racine, depuis
que les fixtures ont leur dépôt), qui ne suivent pas le CPU du runner. Le
ratio n'est pas un instrument à 15 % ici, ni même à 50 % (sylius no-op à
+52 % sur un run). Décision : l'étape est en `continue-on-error` — tableau et
avertissements ⚠ dans le résumé du job, jamais de job rouge. Ce qui garde
réellement contre une régression : hyperfine sur la même machine avant de
commiter (P2, P3b, readdir l'ont tous eu), et le corpus pour la parité.
Baseline = médiane des trois runs `b128c23` (fixtures avec `.git`, comme un
projet réel).

## 2026-09-20 — Les commandes de résolution rendent la main à Composer devant un plugin actif (v0.16 A)

Fait : `composer update --no-install` sur la fixture symfony imprime
« Restricting packages listed in "symfony/symfony" to "8.1.*" » — le
filtre `PRE_POOL_CREATE` de Flex ; vivacity résolvait le pool entier et
écrivait un lock que Composer n'écrirait pas, en silence. Le harnais ne
le voyait pas : l'instantané est résolu `--no-plugins`. Décision : la
même politique que pour `install` — un plugin installé (installed.json du
projet et de `COMPOSER_HOME`), autorisé, et qui touche la résolution
(`scope::resolution_effect` : Flex, merge-plugin, core-recipe-unpack sur
`require`, tout plugin hors liste) fait rendre la commande entière à
Composer avec les mêmes arguments, avant toute écriture, avec la raison ;
`--no-fallback` → code 3. Les plugins sans écouteur de résolution sont
inertes (liste `RESOLUTION_INERT`, lue dans leurs sources). Écarté :
émuler Flex d'abord (c'est l'étape B ; sans le repli, chaque projet Flex
reste faux entre-temps) ; décider sur le lock plutôt que sur installed.json
(Composer charge ce qui est installé, pas ce qui est verrouillé).
Trouvé en chemin : Flex pose `COMPOSER_PREFER_DEV_OVER_PRERELEASE`, et
Composer charge alors les métadonnées `~dev` — absentes d'un instantané
enregistré sans plugins ; 47 fichiers ajoutés au snapshot symfony (additif).

## 2026-09-20 — Le filtre de pool de Flex, émulé ; le reste de Flex, à Composer (v0.16 B)

Fait : `Flex::truncatePackages` (`PRE_POOL_CREATE`) retire du pool, avant
sa création, les versions des paquets listés dans `symfony/symfony` qui
ne satisfont pas `extra.symfony.require` — les `splits` de l'index des
recettes, élagués par `getVersions` (un paquet dont toutes les versions
listées matchent, ou aucune, n'est pas filtré : porté tel quel). Décision :
port ligne à ligne (`flex_filter.rs`), appliqué au même point que Composer
(après le chargement, avant `Pool::new`, avant les filtres de politique),
`COMPOSER_PREFER_DEV_OVER_PRERELEASE` posé sur la politique comme Flex le
fait ; l'index lu comme `Downloader` (endpoints, cache au format de Flex
sous `cache-repo-dir/flex`, partagé avec Composer). Émulé pour `update` et
`remove` sans install seulement ; tout ce que Flex *écrit* reste à
Composer, décidé avant de résoudre : `require` (alias, recettes), l'install
(recettes, `symfony.lock`), `.env.dist`, package.json / importmap.php
(`PackageJsonSynchronizer`), un `symfony-pack` à déballer (`Unpacker`,
POST_UPDATE_CMD — qui charge chaque exigence racine depuis les dépôts :
c'est de là que venaient les fichiers `~dev`, pas du pool). Écarté :
`file://` comme endpoint dans les harnais (Flex exige un statut HTTP 200 —
`php -S` + `secure-http: false` sur les copies) ; émuler la
synchronisation package.json (elle écrit). Vérifié : lock identique à
Composer avec Flex actif sur la démo Symfony et Sylius (harnais hermétique
sur `fixtures/flex/index.json`), et en direct contre Packagist.

## 2026-09-20 — composer-merge-plugin à la résolution : la racine fusionnée dans le pool (v0.16 C)

Fait : le plugin fusionne les manifestes inclus dans la racine à INIT (sans
dev) puis à `PRE_UPDATE_CMD` (les `require-dev`, avec le mode dev de
l'installateur) ; `update`, `require` et `remove` résolvent cette racine-là.
Décision : la fusion déjà portée pour `install` (`merge_plugin::merge`)
alimente la session (`UpdateOptions.merged`) — la racine est chargée depuis
le manifeste fusionné (liens, conflict/replace/provide, alias et références
recalculés par le plugin depuis les liens fusionnés, `mergeAliases` /
`mergeReferences`), **sauf les flags de stabilité** : `RootPackageLoader`
les a extraits du fichier original, et le plugin ajoute ceux de chaque
fichier inclus depuis ses *propres* contraintes (`StabilityFlags::extractAll`
— explicite `@flag` le plus instable des parties, sinon stabilité analysée
quand elle est dev et pas plus stable que le minimum, jamais abaissée, sans
le test « jeton unique » du loader), pas depuis le texte fusionné. Le
content-hash reste celui du fichier. Écarté : résoudre la racine non
fusionnée quand le plugin est requis mais pas installé — Composer le fait
puis relance sa mise à jour implicite à l'install, deux résolutions ;
refusé avec la raison (comme avant). Un fichier inclus qui déclare
`repositories` (`prependRepositories`) : rendu à Composer avant toute
écriture. `--no-plugins` : racine non fusionnée des deux côtés. Vérifié
(`harness/merge-plugin.sh`, plugin amorcé des deux côtés, Packagist en
direct) : lock identique à l'octet sur update, `--no-dev`, `replace`, un
`@dev` dans un fichier inclus (flag 20 dans le lock, `1.x-dev` choisi),
`require`, `remove`, `update` avec install (projet identique) ; le refus
et le repli laissent le lock intact.

## 2026-09-21 — Un correctif de Composer 2.11 porté avant la référence : la requête d'avis par lots de 500

Fait : le drift hebdomadaire (snapshot 2.11-dev `85ae025`) signale
`ComposerRepository::getSecurityAdvisories` : la requête POST à l'API
security-advisories est désormais découpée par lots de 500 noms
(`ADVISORY_API_BATCH_SIZE`). Raison upstream : chaque nom est une variable
de formulaire, PHP tronque `$_POST` au-delà de `max_input_vars` (1000 par
défaut) **sans erreur** — un lock de plus de ~1000 paquets perdait ses
avis en silence, chez Composer 2.10.3 comme chez nous (même requête
unique, [repository.rs](crates/vivacity-resolver/src/repository.rs)). Nos
fixtures plafonnent vers 450 paquets : invisible au harnais, réel sur un
gros Drupal / Magento, et c'est la politique de blocage qui laisserait
passer. Décision : porter le lot de 500 tout de suite, référence maintenue
à 2.10.3 — la règle « même sortie que 2.10.3 » cède devant un correctif
upstream d'une perte silencieuse, parce que la sortie est identique
jusqu'à 500 noms et que l'autre choix est de reproduire un bug connu. Le
message « names which were not requested » prend le texte 2.11 (liste
tronquée à 20 noms) : jamais exercé par le harnais, on suit le code porté.
Requêtes séquentielles (Composer les lance en parallèle via sa boucle
curl) : 3 requêtes pour 1201 noms, pas de raison de paralléliser avant
mesure. Écarté : rafraîchir le jumeau `docs/reference/resolver/
ComposerRepository.php` (les références sont un instantané cohérent de
2.10.3 ; le drift continuera de le signaler, noté dans HANDOVER).
`PlatformRepository.php` (oniguruma absent dès PHP 8.6) : à porter avec
le passage en 2.11, sans effet avant. Vérifié : test unitaire, 1201 noms
→ 3 corps de 500/500/201 dans l'ordre, l'avis du dernier lot trouvé.

## 2026-09-22 — Deux plugins déclarés bénins à l'install sur lecture de leur source ; un inerte de trop corrigé

Fait : le corpus du 19/09 rendait cinq entrées à Composer pour un seul
plugin chacune — `symfony/thanks` (heimdall, nette-web-project,
sulu-skeleton) et `ergebnis/composer-normalize` (prestashop, wallabag).
Lecture des sources (thanks v1.4.1, normalize 2.45.0) : normalize est un
`CommandProvider` sans écouteur ; thanks n'arme son rappel que si la
commande est `update` (`activate` inspecte l'`ArgvInput`), et ne l'affiche
qu'après un `POST_PACKAGE_UPDATE`, après un POST GraphQL GitHub. Décision :
les deux en `BENIGN_PLUGINS` (install natif) ; normalize inerte à la
résolution ; **thanks retiré de `RESOLUTION_INERT`** où la 0.16 A l'avait
mis à tort — `update` avec install part chez Composer (sans token, c'est
l'invite d'authentification ou une exception en `--no-interaction` ;
impossible de savoir avant de résoudre s'il y aura une mise à jour),
`--no-install`, `require`, `remove` restent natifs. Écarté : émuler le
rappel (il dépend de l'état « starred » du compte GitHub de l'utilisateur).
Vérifié : les cinq entrées natives 0 diff dans les deux modes contre
Composer plugins actifs ; corpus complet relancé pour les compteurs.

## 2026-09-22 — Dists `tar` : reproduire PharData, y compris ses bizarreries (v0.17)

Fait : « paquets sans dist » (8 entrées du corpus) recouvre trois choses
distinctes — 263 paquets à dist `tar` (asset-packagist → tarballs npm :
elgg 186, humhub 54, friendica 23), 3 paquets git-only, des sources `path`
absentes de la fixture. La feuille de route disait « vcs » ; c'était faux
sur les nombres. Composer installe un `tar` par `TarDownloader` =
`new PharData($file); extractTo($path, null, true)` puis la règle de racine
unique d'`ArchiveDownloader`. Mesuré sur PHP 8.5.10 (umask 022) : fichiers
au mode exact de l'en-tête (0666, 0664, 0755 gardés, pas d'umask), mode
des entrées répertoire ignoré (0777 & ~umask), **symlinks et hard links
écrits comme des fichiers vides** au mode de l'en-tête (PharData ne crée
aucun lien), `a/../b` normalisé lexicalement. Décision : port de ce
comportement tel quel — la parité vaut aussi pour la bizarrerie des liens
(si PHP la corrige un jour, le job drift le verra) ; `..` et chemins
absolus refusés (plus strict, aucune archive réelle n'en a) ; crate `tar`
0.4.46 épinglée (lecture d'en-têtes seulement, sans C ; gzip par flate2
déjà présent via zip, backend Rust) ; clé de store d'un dist sans
référence = sha1 de l'URL (les tarballs npm n'ont pas de `reference`),
comme la clé de cache de Composer ; fichier de cache `.tar` sous
`files/<nom>/<sha1>.tar` pour que Composer et vivacity lisent le même.
Écarté : un mode « exécutable → 0755 sinon défaut » comme pour le zip
(faux pour PharData, prouvé par les 0666/0664 de blob et after). Vérifié :
oracle PharData sur archives fabriquées (modes, répertoires, liens, noms
longs pax, racine `v1.1/`), harnais fixture 4 tar + 1 zip (projet identique
modes compris, no-op sans réseau, tarballs lus dans le cache de Composer),
corpus elgg natif 0 diff (289 paquets) dans les deux modes. Gain corpus
honnête : +1 (humhub attend yii2-composer, friendica reste sur git +
installers-extender) ; le vrai gain est l'écosystème Yii, bâti sur
asset-packagist.

## 2026-09-22 — yiisoft/yii2-composer émulé ; l'ordre d'extensions.php, comme pest (v0.17)

Fait : le plugin (2.0.11 dans les trois entrées du corpus qui l'ont) fait
deux choses à l'install — `activate` crée `vendor/yiisoft/extensions.php`
(`return [];`) s'il manque, et son installateur du type `yii2-extension`
réécrit la carte (nom, version normalisée, alias déduits des psr-0/psr-4,
`bootstrap`) après chaque opération, via `var_export` avec
`$vendorDir . '…'` pour les chemins sous vendor/. Il fait aussi deux
choses hors install : les statiques `postInstall` / `postCreateProject`
(chmod, clé de cookie) sont des **scripts** de composer.json, pas des
écouteurs — `--run-scripts` les confie à Composer ; et
`POST_UPDATE_CMD` imprime les notes d'UPGRADE.md quand yiisoft/yii2 a été
mis à jour. Décision : émulation à l'install et au dump (`yii2_composer`),
carte réécrite d'un bloc quand une extension entre, sort ou change ;
ordre = celui du dépôt local reconstruit comme pour pest (installed.json
précédent moins les partis, puis les opérations) — l'ordre de Composer
suit l'achèvement des extractions parallèles et n'est pas reproductible,
`compare.sh` compare la carte clés triées via `php -r` (`$vendorDir`
évalué puis neutralisé). Une entrée écrite à la main dans la carte est
perdue (Composer la garderait : il recharge le fichier) — même limite que
pest, documentée. `yiisoft/yii2-dev` (trois shims `Yii.php` dans
`yiisoft/yii2`) refusé avec la raison ; `update` avec install rendu à
Composer (les notes de mise à niveau ne sont connues qu'après résolution
— un contrôle post-résolution le rendrait natif, différé). Vérifié :
harnais 7 étapes identiques du premier coup (dont `--no-dev` qui retire
deux extensions, le retour en dev, le vendor vierge, `--no-plugins` sans
fichier des deux côtés).

## 2026-09-22 — codeception/c3 émulé : la copie, jamais l'écrasement (v0.17)

Fait : dernier bloqueur de yiisoft-yii2-app-basic en dev. Sous Composer 2
le plugin écoute `POST_INSTALL_CMD` / `POST_UPDATE_CMD` : copie
`vendor/codeception/c3/c3.php` à la racine (`getcwd()`) si absent ; s'il
existe et diffère, `askConfirmation` (défaut non) — sous
`--no-interaction`, jamais remplacé, rien imprimé ; s'il est identique
(md5), « already up-to-date » à l'install seulement. `uninstall` du plugin
supprime le fichier. Décision : port tel quel dans `c3_plugin` (copie,
garde, suppression quand le plugin quitte le vendor — installed.json
précédent l'avait, le lock voulu ne l'a plus), messages sur stdout comme
`$io->write` ; c3 inerte à la résolution (ses écouteurs sont ceux de
l'install, émulés là). Écarté : reproduire l'invite (interactive chez
Composer, hors contrat). Vérifié : fixture yii2-composer avec c3 en dev —
c3.php identique après install, absent des deux côtés en `--no-dev`,
recopié au retour en dev.

## 2026-09-22 — bamarni/composer-bin-plugin : émulé quand il ne transfère pas (v0.17)

Fait : à 1.9.1 le plugin écoute `COMMAND` et `POST_AUTOLOAD_DUMP` ; aux
deux, il lit `extra.bamarni-bin`, imprime sur stderr une ligne
`[bamarni-bin] …` par réglage laissé au défaut que la 2.x inverse
(`bin-links`, `forward-command`), et — seulement si `forward-command` est
vrai et la commande `install` ou `update` — relance la commande dans chaque
espace `vendor-bin/*` (installs imbriqués, avec leurs propres locks).
nextcloud le règle explicitement à faux, owncloud le laisse au défaut
(faux). Décision : émulation des lignes aux mêmes points (à `COMMAND` si
installed.json a déjà le plugin, à `POST_AUTOLOAD_DUMP` si le lock voulu
l'a — donc une ligne sur install à froid et sur `--no-dev` qui le retire,
deux sur no-op et dump) ; `forward-command: true` → repli avec la raison
avant toute écriture ; un type de réglage invalide → refus avec le message
du plugin (Composer s'arrête dessus). Écarté : émuler le transfert (ce
sont des installs complets dans d'autres répertoires ; possible un jour en
appelant vivacity lui-même, hors de ce sprint). Vérifié : harnais
bin-plugin, stderr complète identique à Composer sur les quatre étapes.

## 2026-09-22 — roots/wordpress-core-installer : un placeur de plus dans Layout (v0.17)

Fait : bedrock et wordplate ne tenaient qu'à ce plugin — un installateur
pur du type `wordpress-core` : `getInstallPath` = `extra.wordpress-install-dir`
de la racine (chaîne, ou carte par nom, avec la sémantique `empty()` de
PHP), sinon celui du paquet, sinon `wordpress` ; `.` et le vendor-dir
lèvent une exception. Décision : le placeur entre dans `Layout::place`
à côté de la table composer/installers, avec les mêmes contrôles de cible
(relatif, hors racine, hors vendor/), la même détection de conflits et la
même règle de transition (plugin ajouté à ou retiré d'un install existant
→ Composer) ; les refus du plugin deviennent des raisons de repli (Composer
s'arrête sur l'exception, nous avant d'écrire). Écarté :
`johnpbloch/wordpress-core-installer` (themosis), presque identique mais
l'entrée demande aussi composer/installers 1.12, non porté. Vérifié :
harnais wp-core — chaîne, carte, défaut, `--no-plugins` (sous vendor/ des
deux côtés), `.` refusé — projet identique.

## 2026-09-22 — mnsami/composer-custom-directory-installer : de « plugin de layout refusé » à placeur (v0.17)

Fait : listé depuis la 0.2 parmi les plugins qui changent le layout,
toujours refusés. Lu à 2.0.0 (akaunting) : il remplace l'installateur des
types `library` et `composer-plugin` par un placeur « nom exact →
chemin » depuis `extra.installer-paths` de la racine (templates `{$name}`
/ `{$vendor}`, `installer-name` du paquet), défaut sinon — rien d'autre.
Décision : troisième placeur de `Layout` (après composer/installers et
wordpress-core), mêmes contrôles de cible, conflits et règle de
transition ; retiré de `LAYOUT_PLUGINS`. Refusé : la coexistence avec
composer/installers — les deux font `array_unshift` de leur installateur,
l'ordre de préséance suit l'ordre d'activation des plugins (l'ordre
d'installed.json chez Composer), un détail que rien ne fixe. Constaté en
harnais : quand la carte est vidée, Composer réinstalle sous vendor/ et
laisse l'ancien répertoire en place (le paquet n'est plus « installé » à
son ancien chemin) — vivacity fait pareil, le harnais le vérifie plutôt
que l'inverse que j'avais d'abord écrit.

## 2026-09-22 — Le no-op d'`install` ne charge plus les deux dépôts (mesuré sur Doppar)

Fait : test de vivacity sur le squelette du framework Doppar (74 paquets) —
lock identique à l'octet, projet identique, une ligne de stderr en écart
(corrigée séparément). Profil du no-op (37 ms) : ~8 ms entre le contrôle de
plateforme et la transaction, pour construire deux arènes de paquets
résolveur (installed.json et le lock) dont la seule sortie utile est
« aucune opération ». Le chemin chaud le plus fréquent payait l'analyse de
toutes les contraintes de tous les paquets pour ne rien faire.
Décision : comparer d'abord les entrées brutes sur exactement ce que
`Transaction::new` regarde (nom, version, références dist et source,
`abandoned`) ; si c'est identique des deux côtés, ne pas charger les dépôts.
Tout le reste — un alias racine dans le lock, une version dev avec
`branch-alias`, un `default-branch`, un écart quelconque, un `packages-dev`
absent — retombe sur le chemin complet inchangé : le raccourci ne décide
jamais d'une opération, il ne fait que prouver qu'il n'y en a aucune.
Écarté : rendre `Package::raw` partagé (`Arc`) — le clone du tableau de
paquets mesuré à 1,1 ms seulement, le reste est l'analyse des contraintes,
que ce raccourci évite entièrement ; et le parallélisme de la pose sur macOS,
revérifié sur Doppar (128 ms contre 130 ms séquentiel, σ 4 : rien, la
décision de 2026-09-11 tient). Mesuré (M4 Max, caches chauds, hyperfine
25 runs, `--offline --no-blocking`) : Doppar 38 → 35 ms, Laravel 57 → 53 ms,
Sylius 76 → 63 ms (−17 %) — le gain suit la taille du lock. Le warm
(vendor supprimé) est inchangé : ses 99 ms de pose sont un plancher noyau
(`clonefile` par paquet ; `cp -Rc` du même arbre prend 780 ms).
Vérifié : 243 tests (dont 2 sur le garde-fou : chaque différence que la
transaction verrait doit faire décliner), 14 harnais + steps 232/232 +
update 19/19 — tous exercent des no-op.

## 2026-09-22 — `update --with` : le résolveur l'avait déjà, la CLI le refusait (v0.18)

Fait : en regardant si vivacity pouvait entrer dans la CI de Doppar, les six
jobs de `doppar/framework` installent tous par
`composer update --prefer-lowest|--prefer-stable --with="phpunit/phpunit:~13.3.x"`.
C'est la forme de toute CI à matrice (Laravel, Symfony, Sylius font pareil) :
épingler une dépendance pour le run sans toucher composer.json. vivacity la
refusait d'emblée — donc aucune CI de ce format ne pouvait l'utiliser, quelle
que soit la parité par ailleurs. Constat en ouvrant le code : le résolveur
portait déjà tout depuis la v0.4 (`RepositorySet.temporary_constraints`, le
filtre du pool après chargement et avant `PRE_POOL_CREATE`, et jusqu'à la
phrase d'explication d'insoluble) ; seuls la CLI et le câblage manquaient.
Décision : porter le bloc de `UpdateCommand` tel quel — `formatRequirements`
sur `--with` et sur les arguments portant une contrainte (le nom seul va dans
la liste d'autorisation), `extractReferences` / `extractStabilityFlags` sur la
racine, expansion des jokers sur les exigences racine **dans l'ordre de
composer.json**, refus si l'intersection avec composer.json est vide. Un refus
de commande n'est pas une exception : `SessionErrorKind::Printed(code)` imprime
le message seul et rend le code de Composer (1 ici), avec `SessionError.stdout`
pour la ligne que Composer écrit sur stdout à côté. Écarté : `--patch-only` et
`-i`, qui alimentent la même carte mais ne sont pas des formes de CI.
Trois bugs de parité trouvés par le harnais, tous corrigés : l'explication
imprimait l'intervalle compilé au lieu du texte écrit (`psr/log:1.1.*`) — d'où
`pool::TemporaryConstraint` qui porte les deux ; un update partiel sans lock
sortait en 1 avec un préfixe `Error:` au lieu du message nu et du code 3 de
Composer (préexistant) ; le joker signalait la première exigence par ordre
alphabétique au lieu de l'ordre du fichier. Vérifié : 6 cas `steps.sh` rouges
sans le changement, les formes exactes de la CI de Doppar donnent un lock
identique à l'octet dans les deux modes de stabilité.

## 2026-09-23 — `composer/package-versions-deprecated` n'était pas bénin : il réécrit son propre fichier

Fait : en classant deux plugins de la traîne du corpus (`ibexa/post-install`,
inerte ; `metasyntactical/…-license-check`, inerte tant qu'il n'est pas
configuré), l'entrée ibexa est passée de repli à comparaison réelle — et a
montré un diff sur
`vendor/composer/package-versions-deprecated/src/PackageVersions/Versions.php`.
Lecture du plugin (1.11.99.5) : à `POST_AUTOLOAD_DUMP` il régénère ce
fichier avec le nom du paquet racine et la carte `nom => version@référence`
de tout le lock ; le fichier livré dans le dist est un stub qui l'annonce
(« in place only for scenarios where PackageVersions is installed with a
`--no-scripts` flag »). Il était dans `BENIGN_PLUGINS` depuis la 0.10 en
violation de notre propre règle (« un plugin qui écrit un fichier est soit
émulé, soit inconnu »). Pourquoi c'était resté invisible : le plugin n'agit
que si `allow-plugins` l'autorise, et les seules entrées natives qui
l'installent (forkcms, suitecrm) ne l'autorisent pas ; les quatre qui
l'autorisent étaient toutes en repli pour un autre motif. Décision : émuler
(le gabarit du plugin repris tel quel, la carte dans l'ordre du générateur —
paquets du lock, puis dev sauf `--no-dev`, puis les `replace` de la racine
avec `self.version` résolu, puis la racine ; 0664 par fichier temporaire et
rename, rien si le répertoire du paquet a disparu) plutôt que de le déclasser
en inconnu : le contenu est déterministe et un projet qui lit
`PackageVersions\Versions` obtenait sinon le repli du stub. Écarté :
`drupol/composer-packages` (génère aussi une classe, mais depuis un gabarit
plus large — sa propre entrée) ; `liborm85/composer-vendor-cleaner` et
`mlocati/composer-patcher` (suppriment ou patchent). Vérifié : fixture
`package-versions` + harnais 7 étapes (dont `allow-plugins: false` et
`--no-plugins`, où le stub doit rester), rouge sans le correctif ; corpus
ibexa, forkcms et suitecrm natifs 0 diff ; 248 tests, steps 240/240.

## 2026-09-23 — Flex avec install : la tranche prévue était fausse, la revue l'a dit (v0.19)

Fait : dernier grand manque fonctionnel — `update` avec install sur un
projet Symfony part chez Composer. Plan écrit avec une première tranche
« Flex n'écrit que symfony.lock », puis méta-analyse adversariale par un
relecteur frais, comme le veut la méthodologie au niveau 2. Verdict :
abandonner cette tranche. Trois raisons que j'ai reproduites avant de les
accepter : les opérations que Flex voit ne sont pas celles de la
transaction de mise à jour mais celles de `recordOperations`
(`PRE_OPERATIONS_EXEC`), reconstruites depuis `symfony.lock` — 123
opérations d'install sur la fixture symfony là où la transaction est
vide ; le point de repli prévu se situe **après** l'écriture de
composer.lock, ce qui casse le contrat « rien d'écrit avant de rendre la
main » et ferait re-résoudre Composer contre notre lock ; et le chemin
heureux diverge déjà sur la stderr (rappel `symfony/thanks`). En prime
`SymfonyBundle::getClassNames` lit vendor/, donc la question n'est pas
calculable avant l'install pour les paquets que la transaction touche —
et se tromper dans le sens permissif perd silencieusement l'enregistrement
d'un bundle. Décision : faire d'abord la tranche A — fermer les
divergences de Flex sur les chemins que vivacity prend **déjà** en natif —
qui n'ouvre aucune surface d'émulation nouvelle et supprime une casse
silencieuse vivante : `vivacity install` sur un projet Symfony ne copiait
pas `.env` depuis `.env.dist` et n'en disait rien. Les tranches B (install
dont la transaction locale est vide) et C (lire la classe candidate dans
l'archive du store) gardent leur plan. Trouvés en chemin, tous corrigés :
le suffixe `: Extracting archive` sur un `symfony-pack` (posé en
métapaquet par Flex), la ligne `Symfony recipes are disabled…` absente, la
précédence réelle de `root-dir` dans `initOptions`, et — trouvé par le
nouveau harnais — `The "<nom>" plugin was not loaded as plugins are
disabled.` que Composer imprime sous `--no-plugins` après chaque plugin
installé et que nous n'imprimions jamais. Vérifié : fixture `flex-install`,
7 cas, stderr complète comparée ; install Symfony complet (152 paquets)
stderr identique et `.env` identique ; 248 tests, steps 240/240, 16 harnais.

## 2026-09-23 — Flex, tranche B : `update` avec install quand aucune recette ne s'applique

Fait : suite de la tranche A, dans l'ordre que le relecteur avait fixé. Le
point dur est que les opérations de Flex ne sont pas celles de la
transaction : `recordOperations` les reconstruit depuis `symfony.lock` et
enregistre un install par paquet du lock résolu que ce fichier ne contient
pas. Décision : émuler le seul cas où la question est décidable avant
d'écrire — l'install ne pose rien de neuf, donc tout est déjà extrait à la
bonne version, donc la recherche de recette (index) et la détection de
bundle (lecture de vendor/) sont possibles ; la décision est prise entre la
résolution et l'écriture du lock, pas après (le constat 5 de la revue).
Portés pour ça : le domaine de `recordOperations`, la sélection de version
de `Downloader::getRecipes`, et `SymfonyBundle::getClassNames` +
`isBundleClass`. `recipe-conflicts` volontairement non appliqué : il ne
fait que retirer des entrées de l'index, donc l'ignorer ne peut causer
qu'un repli inutile, jamais une recette manquée — le seul sens d'erreur
acceptable ici, puisque se tromper dans l'autre sens perdrait
silencieusement l'enregistrement d'un bundle. Trois bugs de parité trouvés
par le harnais, tous corrigés : `installation-source` vient de la phase de
téléchargement (donc un `symfony-pack` l'a quand Flex s'installe dans le
même run, et pas quand Flex était déjà là — `MetapackageInstaller::download`
ne télécharge rien), un paquet inchangé garde ce que son entrée avait (la
décision passe du sérialiseur à l'installateur), et `purgePackages` ne doit
pas retirer l'entrée d'un paquet qu'on ne pose nulle part, ce qui faisait
réécrire l'entrée d'un pack Flex à chaque no-op. Limite connue laissée
telle quelle : nous écrivons installed.json avec notre indentation là où
Composer reprend celle du fichier — visible seulement sur un fichier édité
à la main. Vérifié : harnais flex-update (5 cas, rouge sans le changement),
251 tests, steps 240/240, 18 harnais.


## 2026-09-24 — Flex, tranche C item 1 : le garde de synchronisation ramené à ce qui s'écrit

Fait : la revue adversariale de la tranche C a désigné notre propre garde
comme le vrai blocage — `package.json` ou `importmap.php` présent faisait
rendre la main sur *toute* application Symfony réelle, quelle que soit la
transaction, alors que c'était recopier le déclencheur de Flex
(`shouldSynchronize()`) et non ce qu'il écrit. Décision : répondre à la
question « qu'est-ce qui serait écrit », lue dans la source de Flex 2.9 et
de Composer 2.10.3, pas dans un souvenir.

Branche `importmap.php` (elle gagne quand les deux fichiers existent) :
`updateImportMap` rend la main avant de lire le fichier sur une liste
d'entrées vide, `updateControllersJsonFile` sur un `assets/controllers.json`
absent, et `synchronizeForAssetMapper` renvoie toujours false — donc même
les trois lignes de stderr ne sortent pas. Les entrées ne viennent que de
paquets portant le mot-clé `symfony-ux`, et **les mots-clés voyagent dans
le lock** : 4 tels paquets dans la fixture symfony, 5 dans sylius.

Branche `package.json` : un fichier illisible par le parseur fait sortir
Flex avant toute écriture ; sinon `removeObsoletePackageJsonLinks` écrit
**sans condition**, et comme `JsonManipulator::__construct` garde
`trim($contents)` et que `getContents()` rajoute `$this->newline` (un
`\r\n` dès que le fichier en contient un), cette écriture ne préserve les
octets que sur un fichier déjà de cette forme. D'où le prédicat retenu :
pas de mot-clé `symfony-ux`, pas de `assets/controllers.json`, aucun lien
`@… : file:<vendor-dir>/…/assets` dont le paquet a disparu, et
`trim(octets) + newline == octets`.

Corollaire de placement : ces questions portent sur le **nouveau** lock,
donc le garde ne pouvait pas rester avant la résolution. Il passe à côté de
`flex_install_reason`, entre la résolution et l'écriture du lock — le point
que la tranche B a déjà prouvé sans écriture. Et il ne dépend pas de
l'install : mesuré sur Composer 2.10.3, `update --no-install` dispatche
quand même `POST_UPDATE_CMD` (Flex synchronise, `--no-scripts` ne coupe que
les scripts de composer.json, pas les abonnés du plugin), tandis que
`--dry-run` ne le dispatche pas du tout.

Sens d'erreur assumé : un paquet `symfony-ux` dont le propre
`assets/package.json` manque ne produit aucune entrée, mais le vérifier
demanderait une lecture de vendor/ — laissé en repli inutile, jamais en
écriture manquée. Vérifié : harnais flex-update passé de 5 à 11 cas (les 6
nouveaux rouges avant le changement, dont 2 natifs), flex-install 7/7,
update 12/12, 253 tests.

## 2026-09-24 — Flex, tranche C : l'ordre de la revue corrigé par deux faits

Fait : l'ordre recoupé par la revue mettait le rappel `symfony/thanks` en
deuxième et la lecture d'archive en dernier (« le tiers le moins utile »).
Deux faits l'inversent partiellement.

Le rappel `symfony/thanks` n'est pas observable avant les réponses par
paquet : il exige un `POST_PACKAGE_UPDATE`, donc une opération d'update
exécutée, alors que la précondition de la tranche B
(`unchanged_local_repository`) exclut toute opération. L'écrire maintenant
serait du code qu'aucun cas différentiel ne peut exercer — il part avec
l'item 3, et il doit en faire partie, puisque le premier update natif qui
met un paquet à jour perdrait sinon trois lignes de stderr.

La lecture d'archive est un prérequis des réponses par paquet, pas l'étape
optionnelle finale. `recordOperations` enregistre un paquet dès que
`symfony.lock` n'a pas son nom, que l'opération soit un install ou un
update ; pour chacun, la question du bundle porte sur le **nouveau**
contenu (`getClassNames` dérive les candidats de l'autoload du nouveau
paquet, `isBundleClass` lit le fichier que l'install vient d'extraire).
Avant l'install, vendor/ contient l'ancienne version : le lire est faux
précisément pour les paquets qu'un update touche. La seule source saine
avant écriture est le dist. Le seul négatif sain sans archive — une entrée
de lock sans espace de noms PSR-4/PSR-0 n'a aucun nom de classe candidat —
ne couvre que peu de paquets.

Décision d'ordre de téléchargement, que la revue demandait d'écrire : les
dists des paquets enregistrés sont récupérés avant l'écriture du lock, en
silence. Le coût en stderr est nul — vivacity n'imprime jamais les lignes
`  - Downloading …` de Composer (divergence connue sur cache froid, filtrée
par trois harnais), donc déplacer le transfert plus tôt ne change aucune
sortie ; et en cas de repli le transfert n'est pas perdu, Composer lit le
même cache.

Livré ici : `read_zip_entry` / `read_tar_entry`, adressés comme l'arbre
extrait (même règle de racine unique), avec un test d'invariant qui extrait
chaque archive puis relit chaque fichier par le lecteur — les extracteurs
étant déjà comparés à `ZipArchive` et `PharData`, l'oracle est transitif.
Un lien symbolique répond `None` et non le fichier vide que PharData écrit :
c'est le sens d'erreur sûr pour « ce fichier déclare-t-il un bundle ».

## 2026-09-24 — Flex, tranche C : réponses par paquet, rappel thanks, et un trou trouvé

Fait : la tranche B n'était native que si l'install ne posait rien. En
relisant `recordOperations` et `shouldRecordOperation` dans la source, le
domaine s'avère indépendant de la transaction d'install : une `Transaction`
synthétique est construite entre les paquets de `symfony.lock` et le jeu
résolu **entier**, et seuls les `InstallOperation` dont `symfony.lock` n'a
pas le nom sont retenus (un `UpdateOperation` retombe sur `return false`,
un `UninstallOperation` est écarté par l'appelant). Donc la précondition
globale n'avait qu'un rôle : garantir que vendor/ contenait déjà la bonne
version pour lire la classe de bundle. Elle est remplacée par la lecture de
la bonne source, paquet par paquet : le dist pour un paquet que l'install
pose ou change, vendor/ pour un paquet qu'elle laisse tel quel, et rien du
tout pour un paquet qu'un installateur place ailleurs que
`<vendor-dir>/<nom>` — seul endroit où `isBundleClass` regarde, ce qui rend
la réponse négative par construction. Tout ce qui n'est pas répondable
avant écriture rend la main.

Le rappel `symfony/thanks` part avec, comme prévu : dès qu'un update natif
peut mettre un paquet à jour, `enableThanksReminder` s'arme sur
`POST_PACKAGE_UPDATE` et trois lignes apparaissent. Prédicat mesuré :
armé à 1 uniquement si la commande résolue est `update`, promu à 2 au
premier POST_PACKAGE_UPDATE si `class_exists(Thanks::class, false)` est
faux — donc si Composer n'a pas activé le plugin symfony/thanks, du projet
ou de COMPOSER_HOME, ce que `scope::active_plugins` couvre déjà. Décidé
avant l'install, sur l'état qu'elle va remplacer. Vérifié à l'octet contre
Composer (mêmes lignes 14 et 15, emoji et doubles espaces compris).

Trou trouvé par le nouveau cas `flex-installed`, préexistant : quand
symfony/flex est dans le lock mais pas dans installed.json, il n'est pas un
plugin actif, donc ni le filtre de pool ni rien d'autre ne s'appliquait —
et Composer, lui, exécute alors `Flex::install` → `$reinstall` → un second
`Installer` complet, cette fois avec le filtre. Le repli est donc décidé
hors de la question « le plugin est-il actif », puisque la réponse est
précisément qu'il ne l'est pas encore.

Régression introduite puis corrigée, trouvée par la CI et non par moi :
déplacer la décision de synchronisation après la résolution faisait payer
une résolution à un projet qui allait rendre la main de toute façon, et
hors ligne l'index de recettes non caché masquait le motif (`transitions.sh`
attendait 3, recevait 1). Rétabli une passe précoce du même prédicat sur le
lock **actuel**, qui ne peut que rendre la main : l'ancien lock peut porter
un paquet `symfony-ux` que l'update va retirer, et un repli inutile est le
sens d'erreur sûr. La passe tardive sur le nouveau lock reste
l'autoritaire. Leçon : le hook pre-push ne fait tourner que fmt/clippy/tests
— une retouche de l'émulation d'un plugin doit faire tourner tous les
harnais qui le nomment (`grep -l` sur harness/), ici transitions.sh et
vendor-dir.sh.

Trou refermé dans le même mouvement, ouvert par la suppression de la
précondition globale : les **suppressions**. `Flex::record` (POST_PACKAGE_
UNINSTALL) enregistre tout uninstall, et `fetchRecipes` retire alors le nom
de `symfony.lock` puis désinstalle le bundle — avec `$uninstall` vrai,
`getClassNames()` rend tous les noms candidats sans lire un seul fichier.
Sans le garde, le cas `remove-package` ne rendait pas la main et **écrivait**
le lock. Une suppression d'un nom absent de `symfony.lock` est sautée avant
toute écriture, et en `--no-dev` un paquet que le nouveau lock porte sous
`packages-dev` n'est pas enregistré du tout.

Mesure de ce que la tranche achète sur un projet réel : avec le **vrai**
`symfony.lock` de la fixture symfony (30 entrées, donc 123 des 153 paquets
enregistrés), `update` avec install est natif — projet, composer.lock,
symfony.lock et stderr identiques. C'est cohérent : les paquets qui portent
un bundle sont exactement ceux dont la recette a été appliquée, donc ceux
que `symfony.lock` détient. Le cas jumeau `real-lock-gap` (le même fichier
moins `symfony/twig-bundle`) rend la main sur ce bundle : sans lui, la paire
serait vide de sens, un fichier ignoré ou complet donnant aussi « natif ».

Bug de parité préexistant trouvé en passant, en se demandant ce que
`--dry-run` fait du bloc de Flex : Composer ne dispatche pas du tout
`POST_UPDATE_CMD` sur un dry run (mesuré), donc Flex n'affiche rien, pas
même la ligne des recettes — nous en imprimions trois. Corrigé et couvert
par un cas (`dry-run`), le harnais acceptant désormais des arguments en
plus pour les deux côtés.

Vérifié : flex-update 19/19 (les 14 nouveaux cas rouges avant), transitions,
vendor-dir, flex-install 7/7, update 19/19, steps 240/240, 256 tests.

## 2026-09-27 — Drift : les deux jumeaux qui bougent, et un acquittement daté

Fait : l'issue #1 (canal `snapshot`) répète depuis le 2026-09-20 « reference
twins: failure · tests + harness: success ». Le log nomme les deux fichiers :
`ComposerRepository.php` (44 lignes) et `PlatformRepository.php` (5 lignes).
Lus en amont par l'API GitHub, sans télécharger de phar.

`ComposerRepository.php` : commits `53e6ddca8` (batching des requêtes à l'API
des avis de sécurité, `ADVISORY_API_BATCH_SIZE = 500`, parce que PHP tronque
`$_POST` au-delà de `max_input_vars` sans erreur) et `24e396b52` (options de
transport passées à `FilterListApiClient`). Le premier est ce que vivacity
avait déjà porté par avance (`crates/vivacity-resolver/src/repository.rs`) :
le jumeau diffère parce qu'amont a rattrapé, pas parce que nous sommes en
retard. Nuance relevée et non portée : amont lance désormais les lots **en
parallèle** (`httpDownloader->add` + `loop->wait`), trie les réponses par
index de lot, et n'affiche les avertissements du dépôt qu'une fois pour
l'ensemble — notre boucle est séquentielle. Sans effet contre la référence
épinglée 2.10.3, qui n'a pas le batching du tout.

`PlatformRepository.php` : `MB_ONIGURUMA_VERSION` est déprécié en PHP 8.6, le
garde passe de `PHP_VERSION_ID < 90000` à `< 80600` et `Silencer::call`
disparaît. Rien à porter : sur PHP 8.6 amont retombe sur la branche qui lit la
version dans la sortie de `phpinfo` — celle que notre sonde a déjà en
`elseif` — donc `lib-mbstring-oniguruma` existe des deux côtés, seule la
source de la chaîne de version change, et notre `@constant(...)` supprime déjà
la notice.

Décision : deux dérives lues, aucune à porter, donc acquittables. Mécanisme
ajouté dans `harness/drift-reference.sh` : `docs/reference/DRIFT-ACK` liste
`<fichier> <sha256 du diff> <raison>`. La clé est l'empreinte du **diff**, si
bien qu'un nouveau mouvement amont périme l'acquittement et réalerte — un
acquittement tait un écart dont la décision est écrite, il n'endort jamais la
surveillance. Le script imprime sous chaque dérive la ligne exacte à coller.
Vérifié sur les trois branches dans un arbre jetable : non acquittée (code 1 +
ligne à coller), acquittée (code 0, `ACK` avec la raison), amont qui rebouge
(code 1 à nouveau). Piège rencontré en route : sortir le `diff` de l'intérieur
d'un `echo` le soumet à `set -e -o pipefail`, qui tuait le script sur le statut
1 de `diff` — le diff est maintenant écrit une fois dans un fichier, puis lu.

Titre de l'issue de drift rendu stable par canal (`Drift against Composer
($CHANNEL)`), la version passant dans le corps : la chaîne de version du canal
`snapshot` porte un ref de build, donc la mettre dans le titre ouvrait une
issue neuve dès qu'amont recompilait. La recherche d'issue existante passe de
`--search "… in:title"` (substring, et le titre contient des parenthèses) à une
comparaison exacte en `jq`. Reste à faire, hors de portée d'ici : renommer
l'issue #1 au nouveau titre pour que le prochain run la commente au lieu d'en
ouvrir une seconde, et coller les deux lignes d'acquittement dès qu'un run
contre le snapshot aura imprimé leurs empreintes.

## 2026-09-28 — Revue adversariale de la tranche C : deux identités trop faibles

Fait : revue indépendante de `98a04a8..76098f8` par un relecteur frais,
mandaté pour réfuter. Elle a rapporté neuf défauts de correction, dont deux
mesurés de bout en bout, plus des trous de couverture. Traités ici, les deux
qui produisaient un verdict « natif » faux. Les deux ont la même forme : un
modèle de « inchangé » plus faible que celui de Composer.

`JsonManipulator::__construct` n'est pas `trim` seul. Ses deux dernières
lignes, que j'avais coupées en lisant le phar :

    $this->newline = false !== strpos($contents, "\r\n") ? "\r\n" : "\n";
    $this->contents = $contents === '{}' ? '{' . $this->newline . '}' : $contents;

Donc le saut de ligne est cherché dans la chaîne **trimée** — un fichier
d'une seule ligne dont le seul CRLF est ses deux derniers octets revient en
`\n` — et un objet vide est étalé sur deux lignes. Deux `package.json`
réécrits par Composer là où nous répondions « rien à faire ». Le test à
table qui figeait la mauvaise valeur est remplacé par un test différentiel
contre la classe elle-même, sur quinze entrées : une table écrite à la main
est précisément ce qui s'est trompé.

`installed_as_locked` comparait (nom, version, référence de dist), et
l'identité de l'installateur (`installer.rs`) le couple (version, référence
de dist). `Transaction::calculateOperations` compare la version, les **deux**
références et la marque `abandoned`. Conséquences mesurées sur un paquet dont
seule `source.reference` bouge : Composer fait une opération d'update, réécrit
l'entrée d'installed.json depuis le lock et Flex sort son rappel
`symfony/thanks` ; nous ne reposions rien, gardions l'ancienne entrée (donc
une référence de source périmée sur disque) et perdions trois lignes de
stderr. L'identité à quatre champs est désormais partagée par tous les
appelants au lieu d'être redérivée plus étroite à deux endroits.

Le cas `abandoned` mérite sa note : `ArrayDumper` omet le champ quand il vaut
false, et il peut valoir true ou un nom de remplacement — les trois états
sont ramenés à une chaîne comparable.

Vérifié : flex-update 22/22 (les 3 nouveaux cas rouges avant, dont
`thanks-src-ref` qui a d'abord échoué sur installed.json et a révélé le
second bug), steps 240/240, update 19/19, diff-vendor, removal, path-repos,
transitions, boot, vendor-dir, flex-install, 257 tests.

## 2026-09-28 — Le lecteur de dist contre l'arbre extrait : trois désaccords

Fait : la même revue a mesuré trois cas où `read_zip_entry` /
`read_tar_entry` répondent autrement que la lecture du fichier posé. Tous
trois vont dans le sens dangereux — « ce paquet ne porte pas de bundle »
alors qu'il en porte, donc l'enregistrement du bundle et l'entrée de
`symfony.lock` sautent en silence.

1. `extract_zip` pose un vrai lien symbolique pour une entrée de mode
   0o120777 (vérifié : l'entrée sort en `120777`, le fichier sur disque est
   un lien, sa lecture rend le contenu de la cible) ; le lecteur sautait les
   liens. Il les suit maintenant, résolus lexicalement dans l'archive, avec
   un plafond de huit sauts pour l'équivalent d'`ELOOP`. En tar la réponse
   `None` reste juste : `PharData` écrit un lien comme un fichier vide.
2. L'extraction écrit toutes les entrées dans l'ordre, donc un nom répété
   finit avec les octets de la **dernière** ; le lecteur rendait la première.
3. Sur un système de fichiers insensible à la casse — celui de macOS et de
   Windows par défaut — lire `src/AcmeBundle.php` trouve une entrée nommée
   `src/acmebundle.php` ; le lecteur comparait les octets. Une différence de
   seule casse est désormais acceptée **après** l'échec d'une correspondance
   exacte : sur un système sensible à la casse cela ne peut que sur-répondre,
   et une sur-réponse du bundle est un repli inutile, jamais un bundle
   manqué. Casse ASCII seulement, ce qui couvre tout nom de fichier de classe
   PHP.

Le test d'invariant excluait explicitement les liens symboliques de son
inventaire : c'est ce qui laissait passer le premier. Il les parcourt
maintenant, et deux tests s'ajoutent pour le nom répété et la casse. Les
trois vérifiés rouges avant le correctif.

Leçon de méthode, la troisième de la journée : un `str.replace` sans
vérification est un correctif qui peut ne pas s'appliquer. Deux de mes
éditions ont été silencieusement perdues — l'une parce que `cargo fmt` avait
replié un `vec!` sur une ligne entre-temps — et un cas de test n'a pas
existé pendant deux exécutions qui le déclaraient vert. Toute substitution
porte désormais son assertion.

## 2026-09-28 — Revue, suite : un repli inutile qui allait mordre, et une panique

Fait : trois autres points de la même revue, dans `flex.rs`.

Le dist était récupéré avant qu'on demande s'il y avait quoi que ce soit à
lire. `getClassNames` répond « aucun candidat » depuis la seule entrée de
lock quand le paquet ne déclare pas de `psr-4`/`psr-0` — ce qui est le cas
d'un métapaquet et d'un `symfony-pack`, qui n'ont pas de dist du tout. Un
`update` qui en installait un rendait donc la main sur « impossible de lire
le dist », pour une lecture que Flex n'aurait jamais faite. La liste des
fichiers candidats est maintenant calculée d'abord (`bundle_candidate_paths`),
et seul un paquet qui en a un est lu. Aucune fixture ne porte de paquet sans
dist, donc le garde est couvert par des tests unitaires plutôt que par le
banc — déclaré ici faute de mieux.

Un chemin d'autoload non-chaîne faisait sauter tout l'espace de noms, là où
PHP enveloppe le scalaire (`if (!is_array($paths))`) et l'utilise comme
chemin : `{"psr-4": {"Acme\\": 5}}` cherche son candidat sous `5/`.

`extract_class_names` pouvait **paniquer** au lieu de répondre :
`substr($suffix, -6)` compte des octets et la comparaison est binaire, alors
que nous découpions la &str — un segment d'espace de noms finissant par un
caractère multi-octets tombait sur une frontière invalide. Comparé sur les
octets désormais. C'est le chemin que prennent les 123 paquets enregistrés du
cas `real-symfony-lock`, donc une panique à portée d'un paquet réel.

## 2026-09-28 — Revue, fin : trois écarts de fidélité, et un constat réfuté

Fait : le reste des points de correction de la revue.

`synchronize_package_json` : Flex nie la valeur, donc la fausseté de PHP
s'applique — `0`, `""`, `"0"`, `[]` sautent la synchronisation ET impriment
« Skip synchronizing package.json with PHP packages ». Nous lisions un
booléen et traitions tout le reste comme vrai. Couvert par `sync-disabled-0`,
avec un `package.json` qui aurait été réécrit sans ça.

`vendor-dir` hors de la racine : Composer calcule
`trim(makePathRelative($vendorDir, $rootDir), '/')`, soit `../vendor`, et
continue ; nous rendions la main. Passé par `find_shortest_path`.

`flex-require` / `flex-require-dev` : seule la vérification d'`unpack` était
sautée pour ces projets, rien ne couvrait la réécriture. Vérifié dans
`Flex::update` (lignes 365-398) : la branche alternative repasse composer.json
par `JsonManipulator` avec un `file_put_contents` inconditionnel, fusionne les
clés dans `require`, les retire, puis appelle `reinstall()`. Repli.

**Constat réfuté.** La revue signalait que `flex_downloader_enabled` fait une
recherche sensible à la casse là où `ArrayLoader::parseLinks` met les cibles
en minuscules : avec `"Symfony/Flex": "^2"` dans `require`, Flex serait actif
et nous le croirions désactivé. Mesuré : Composer refuse le projet **avant**,
`RootPackageLoader` (ligne 179) répondant « require.Symfony/Flex is invalid,
it should not contain uppercase characters », exit 1 ; et une contrainte
non-chaîne est refusée par le schéma JSON (`Factory` ligne 317). Les deux
moitiés du constat sont donc inatteignables depuis un manifeste racine. Le
changement a été retiré : du code inatteignable qui suggère un scénario
impossible vaut moins que pas de code.

Ce que la tentative a trouvé en revanche, et qui reste ouvert : sur ces deux
manifestes, **vivacity réussit là où Composer refuse** (exit 0 contre exit 1,
lock écrit). La validation du manifeste racine — nom d'exigence en majuscules,
schéma JSON — n'est pas portée. C'est un écart réel, hors de la tranche Flex,
et de sens inverse à tous les autres : nous acceptons un projet que Composer
rejette.

Trous de couverture refermés au passage, signalés par la revue : l'appel
**tardif** de `flex_sync_reason` — celui que ces notes appellent
l'autoritaire — n'était exercé par aucun cas, tous les cas `sync-*` partant
en repli dès la passe précoce ; `sync-ux-late` retire les mots-clés
`symfony-ux` du lock **actuel** seulement, si bien que la passe précoce ne
voit rien, que la résolution les rétablit et que c'est la passe tardive qui
rend la main. Et `--no-install` n'était couvert par aucun cas alors que la
décision d'y appliquer le garde repose sur une mesure : `no-install` l'exerce
maintenant.

Reste ouvert de la revue, consigné sans être traité : `Where::Elsewhere`
n'est exercé par aucun cas (aucune fixture n'a de paquet placé par
composer/installers) ; le lecteur tar est plus permissif que l'extracteur sur
les types d'entrée inconnus, ce qui reporte l'échec au lieu de rendre la main
proprement ; et le prédicat `symfony-ux` reste un sur-ensemble assumé de ce
que `resolvePackageJson` demande vraiment (un `assets/package.json` dans le
paquet).

## 2026-09-28 — Le port existait déjà : ne pas remodéliser ce qui est porté

Fait : Windows a refusé le lot précédent, et la cause remonte à une faute
plus profonde que le symptôme.

Le symptôme : le test différentiel que j'avais écrit pour `JsonManipulator`
vivait dans les tests unitaires de bibliothèque, seuls exécutés par le
workflow `windows` (`cargo test --workspace --lib`). Il cherche le phar par
`which composer`, ce qui sur Windows ne rend pas un phar. Tous les autres
oracles à phar du dépôt sont des tests d'**intégration**, jamais lancés là :
mon test était au mauvais endroit.

La faute : `crates/vivacity-resolver/src/json_manipulator.rs` contient depuis
longtemps un port complet de la classe, dont le constructeur fait exactement
ce que j'ai passé la soirée à redécouvrir — `trim`, vide → `{}`, la
vérification `^\{(.*)\}$`, le saut de ligne cherché dans la chaîne **trimée**,
et `{}` étalé sur deux lignes. Il est tenu à la classe réelle par
`oracle_json_manipulator`. J'ai écrit un second modèle du même objet, à la
main, faux, avec son propre test qui figeait l'erreur — alors que la réponse
juste était à un appel de fonction.

Correctif : `flex_sync_reason` appelle `JsonManipulator::new(&text)` et
compare `contents()` aux octets du fichier ; le `Err` du constructeur est
exactement le cas « pas un objet », qui fait avorter Composer et doit donc
rendre la main. Mon helper et ses deux tests sont supprimés. L'oracle du
manipulateur gagne un scénario **sans aucune opération** : le constructeur et
`getContents()` seuls, rejoués sur tout le corpus de manifestes réels — c'est
précisément la propriété dont dépend le garde de synchronisation, et elle
n'était couverte par personne.

Règle retenue : avant de modéliser un comportement de Composer, chercher s'il
est déjà porté. `grep` sur le nom de la classe aurait suffi.

## 2026-09-29 — Validation du manifeste racine : refus natif aux cinq commandes

Fait : vivacity réussissait là où Composer refuse — le premier écart de ce sens
du projet. `Factory::createComposer` charge le paquet racine pour **toute**
commande, donc Composer refuse un manifeste invalide quoi qu'on lui demande ;
nous ne construisions un paquet racine que pour une résolution, si bien que
`install`, `dump-autoload`, `require` et `remove` acceptaient ces manifestes.
`dump-autoload` allait jusqu'à **écrire** l'autoloader là où Composer sort en 1
sans rien écrire.

Décision : refus **natif**, texte du message seul, code de sortie identique,
rien d'écrit. Ni reproduction à l'octet, ni délégation. La maison avait déjà
tranché ce cas et l'avait écrit (HANDOVER, dépôts `path` : « la forme encadrée
des erreurs … vivacity imprime le texte seul, code identique »). Mesuré contre
la reproduction : l'encadré de Symfony Console est replié à la largeur du
terminal (COLUMNS=40 → 39 colonnes, COLUMNS=200 → 113, coupant les mots), suivi
du synopsis de la commande, et parfois **chaîné** (deux encadrés pour une
contrainte invalide). Mesuré contre la délégation : `run_dump` n'a aucun chemin
de délégation (il rendrait 3, Composer rend 1), `fallback_or_fail` exigerait une
variante de `ScopeIssue` dont l'en-tête décrirait mal un manifeste que la
référence refuse aussi, et sans Composer sur le `PATH` la délégation rend 3 —
le cas de l'utilisateur du binaire autonome.

Ce que la méta adversariale a réfuté du premier plan est consigné dans
`docs/plans/v0.19-root-manifest-validation.md` §6. Le plus coûteux : la
fonction que le plan voulait écrire **existait déjà**
(`lockfile.rs::package_naming_error`), et le plan aurait produit une seconde
copie de la même fonction de Composer dans le même crate. Elle portait un bug,
trouvé en pointant un oracle dessus : la suggestion de nom de la branche
`$isLink = false` éclate le camelCase avant de minusculer (`Foo/BarBaz` →
`foo/bar-baz`), nous rendions `foo/barbaz`. Ce message est celui de « Invalid
package found during dependency resolution », donc le bug était livré.

Découverte en cours d'exécution, par le banc que j'écrivais : le motif `name`
du schéma JSON est **sensible à la casse** (`^[a-z0-9]([_.-]?[a-z0-9]+)*/…$`,
lu dans `res/composer-schema.json`), et le schéma passe avant le chargeur. Un
nom racine en majuscules ou de forme invalide est donc refusé par le schéma,
avec un message que nous ne portons pas — nous en imprimions un autre. Le
contrôle du nom racine est désormais **conditionné** au franchissement du motif
du schéma : seuls le suffixe `.json` et les noms réservés l'atteignent, et ce
sont les seuls cas où nous parlons. Sans cette découverte, ce changement aurait
remplacé « accepter à tort » par « refuser avec le mauvais message ».

Périmètre assumé, mesuré sur huit manifestes invalides réalistes : trois refus
sont portés, cinq restent ouverts — tous ceux qui viennent du schéma JSON, dont
le portage demanderait de valider `res/composer-schema.json` (90 886 octets,
draft-04). La contrainte invalide (`ArrayLoader` + `VersionParser`, deux
encadrés chaînés) est reportée parce qu'elle exige en plus que le texte d'erreur
de notre parseur de versions corresponde. Et sur `install`/`dump-autoload`, les
refus qui exigent un manifeste analysé (alias malformé, contrainte illisible)
ne sont pas appliqués : y arriver demande de devin(er) la version de la racine,
donc de lancer git, sur le chemin chaud de l'install.

Trou de surveillance refermé au passage : `docs/reference/` n'avait pas
`ValidatingArrayLoader.php`, donc les règles de nommage déjà livrées n'étaient
surveillées par aucun job de drift. Le jumeau est ajouté (127 fichiers suivis
après le dédoublonnage du 2026-09-30).

Vérifié : les 21 cas de `harness/root-manifest.sh` rouges avant (21/21), le bon
rouge par commande étant différent à chaque fois — `install` sortait 4 (lock
périmé), `dump-autoload` 0 **en écrivant**, `update` 2, `remove` 2, `require` 1
avec un autre message ; 9 lignes de plus dans steps.sh (249/249) ; update 19/19,
flex-update 26/26, diff-vendor, removal, path-repos, transitions, boot,
vendor-dir, flex-install ; 263 tests dont un oracle de nommage contre le phar
sur un corpus fabriqué **et** tous les noms de liens des fixtures.

## 2026-09-29 — Revue indépendante : mon garde était à l'envers

Fait : la revue indépendante (niveau 2) a rapporté cinq défauts réels dont une
régression, et surtout elle m'a fait voir que j'avais optimisé le mauvais
critère.

**Le garde du schéma était à l'envers.** J'avais conditionné la règle du nom
racine au franchissement du motif `name` du schéma, pour ne pas imprimer un
message que Composer n'imprime pas. La revue montre le prix de ce choix :
sur un manifeste que le schéma refuse **aussi**, se taire nous fait retomber
sur « accepter », donc `dump-authoload` écrit un autoloader là où Composer sort
en 1 sans rien écrire. C'est exactement le péché que ce changement corrige. Le
mot juste est moins important que l'issue juste, et le projet assume déjà une
déviation de mots (l'encadré). Le garde est donc **retiré** : la règle parle
toujours. Trois effets : la régression signalée disparaît (elle ne portait que
sur les mots d'un manifeste à deux fautes), le cas du saut de ligne final
devient **exact** sans rien coder (le motif du chargeur le refuse, comme
Composer), et deux fonctions auxiliaires disparaissent.

**`COMPOSER=<fichier>` contournait le garde** sur `install`, `dump-autoload` et
`update`, qui lisaient `composer.json` sans regarder la variable — donc
validaient un fichier que Composer ne regarde pas, et écrivaient. Trou
préexistant (`remove` et `require` refusaient déjà la variable) : mesuré,
`update` écrivait même `composer.lock` au lieu d'`alt.lock`. Les trois refusent
maintenant comme les deux autres.

**`RootPackage::load` n'appelle plus `manifest_error`.** Le plugin merge charge
un manifeste **fusionné** par `RootPackage::load`, alors que Composer applique
`setRequires` à un paquet racine déjà chargé — il ne revalide jamais les noms
injectés. Sans ce retrait, un fichier inclus déclarant `core` ou
`fluid_styled_content` (25 noms de ce type dans le cache packagist local, que
Composer accepte via le plugin) aurait fait refuser un projet que Composer
résout. Les cinq gardes de commande suffisent.

Écarts résiduels, écrits plutôt que corrigés : un nom de paquet invalide passé
en **argument** de `require` (`require Psr/Log:^3`) est attrapé après la
réécriture de composer.json, donc une ligne de plus et l'ordre de la note de
retour inversé — mais l'issue est juste (avant ce changement, cette commande
sortait 0 en laissant la mauvaise exigence dans le manifeste) ; et
`{"require": {"0": "^1"}}` sort 1 chez nous contre 255 chez Composer, dont le
`declare(strict_types=1)` lève une TypeError sur la clé devenue entière.

Vérifié après révision : root-manifest 35/35 (cinq commandes × sept règles),
steps 251/251, update 19/19, flex-update 26/26, diff-vendor, removal,
path-repos, transitions, boot, vendor-dir, flex-install, 263 tests. La revue a
par ailleurs poussé la preuve d'absence de faux positif bien au-delà de mes
fixtures : 1 122 manifestes du dépôt et 211 497 versions de paquets réelles du
cache packagist local, zéro refus indu, et un fuzz de 187 384 comparaisons de
`suggest_name` contre le phar sans divergence.

## 2026-09-30 — Drift : ce que l'acquittement a fait remonter

Fait : le cron du 2026-09-28 a tourné avec le mécanisme d'acquittement en
place, et il a imprimé ce pour quoi il a été fait — une ligne prête à coller
par dérive. Mais le snapshot avait bougé (`2.11-dev+cc854808`, contre
`85ae0251` la fois d'avant), donc ce ne sont plus les deux mêmes fichiers : dix
jumeaux ont dérivé, et surtout **l'étage 2 a échoué aussi**, ce qui n'était
jamais arrivé — un écart de *comportement*, pas un déplacement de code.

L'écart est dans `oracle_semver` : 3 paires divergentes sur 518 dans
`intervals_match_composer`, 4 contraintes sur 7 494 dans le corpus. Toutes de
la même forme, une borne avec suffixe de pré-version :

    >=2.5.0-RC1   2.10.3 → ">= 2.5.0.0-RC1-dev"   2.11-dev → ">= 2.5.0.0-RC1"
    <3.3-rc2      2.10.3 → "< 3.3.0.0-RC2-dev"    2.11-dev → "< 3.3.0.0-RC2"

Cause amont nommée : composer/semver `9ee1a95ec`, « Fix RC stability suffix
getting an extra `-dev` in `<` and `>=` constraints » (#187, 2026-09-24). Amont
qualifie cela de **correctif** : notre port reproduit donc fidèlement un bug de
la version épinglée. Conséquence de résolution non nulle — `>= 2.5.0.0-RC1-dev`
admet une build de dev de la RC, `>= 2.5.0.0-RC1` non — et ces contraintes
existent dans de vrais manifestes (4 occurrences dans notre corpus de 7 494).

Deux autres dérives s'expliquent par le même lot amont : `346ebc8ca` (clés de
cache concaténées dans `CompilingMatcher`, sans effet observable) et
`b60bbabdb` (#189, ne plus compacter une contrainte sans dev en `!=` nus qui
matchent dev) — ce dernier est probablement la cause des divergences
d'`intervals` au-delà du suffixe.

Décision : rien à porter tant que la référence est 2.10.3 — notre sortie est
juste **par définition du contrat**. À porter le jour où l'on re-épingle, et
c'est désormais écrit noir sur blanc avec le commit amont, ce qui était tout
l'objet du job. Les dix jumeaux ne sont pas acquittés : l'acquittement exige
d'avoir lu chaque diff, et deux sont gros (`ClassMap.php` 167 lignes,
`Platform.php` 72). L'alarme hebdomadaire reste donc rouge, ce qui est le bon
état : elle signale un écart réel et daté, pas du bruit.

## 2026-09-30 — Les dix dérives instruites et acquittées

Fait : chaque jumeau dérivé a été rattaché à son commit amont, par l'API et sans
télécharger de phar, et `docs/reference/DRIFT-ACK` porte maintenant dix lignes
avec une raison chacune. Trois dépôts amont sont en cause et, surprise utile,
trois des dix fichiers ne forment qu'**une seule** nouveauté : composer #13085
ajoute un avertissement pour les chemins qui ne diffèrent que par la casse dans
un dump optimisé, et il consomme `ClassMap::getAmbiguousFolders` ajouté par
class-map-generator #49/#50 ; #48 du même dépôt corrige au passage le slash
initial d'un chemin absolu derrière un wrapper de flux.

Répartition des décisions : quatre « rien à porter » (batching des avis déjà
porté par avance, User-Agent d'agent IA, dépréciation de MB_ONIGURUMA dont amont
prend la branche `phpinfo` que notre sonde a déjà, clés de cache de
`CompilingMatcher`) et cinq « à porter au ré-épinglage », dont les trois de la
nouveauté d'autoload et les deux comportementales de semver (#187 et #189).

La plus intéressante reste #187, dont le message amont explique le mécanisme :
le contrôle du suffixe de stabilité minusculait la version puis la comparait à
une regex sensible à la casse contenant « RC », donc il ne matchait jamais —
`>=1.0-RC1` devenait `>=1.0.0.0-RC1-dev`. Un bug silencieux de la référence, que
notre port reproduit exactement, et qu'il faudra cesser de reproduire le jour du
ré-épinglage.

Redondance corrigée : `SemverVersionParser.php` et `semver-VersionParser.php`
étaient deux jumeaux du **même** fichier du phar (même empreinte de diff, fichiers
identiques). L'entrée de la table `twin()` et la copie de référence sont
supprimées ; 127 fichiers suivis au lieu de 128.

Limite écrite : les empreintes sont celles du build `2.11-dev+cc854808` du
2026-09-28. Le prochain cron tourne contre un build plus récent, et tout fichier
qui a rebougé depuis réalertera — c'est le comportement voulu. La valeur durable
de ces lignes est la raison, pas le hachage.

## 2026-09-30 — `preferred-install` : un vendor/ faux, trouvé en tirant un fil

Fait : en instruisant les drapeaux que vivacity refuse (`--prefer-source` sort
en 2 là où Composer sort en 0), j'ai lu `resolvePackageInstallPreference` et
trouvé autre chose — un résultat faux en silence, pas une ligne de stderr.

La fonction prend le **premier** motif qui matche ; sans aucun motif elle
répond `$package->isDev() ? 'source' : 'dist'`. Donc `auto`, et **toute carte de
motifs qui laisse un paquet en version dev sans correspondance**, font cloner ce
paquet **depuis la source** : un checkout git là où vivacity extrait un dist.
Vérifié contre Composer 2.10.3 sur un projet à paquets dev : il imprime
`  - Syncing composer/installers (dev-main 634ba02) into cache` pour les dev et
`  - Downloading …` pour les stables. Notre `config_issues` ne refusait que la
chaîne littérale `source`.

Le défaut de Composer est `preferred-install: dist` (`Config`), et `Factory`
transforme la chaîne en `setPreferDist(true)` / `setPreferSource(true)` ; seule
une valeur `auto` (ou une carte) laisse la résolution par paquet décider. C'est
pourquoi le cas par défaut n'avait jamais rien révélé.

Décision : porter la fonction au lieu de deviner une règle. `prefers_source`
reproduit la cascade exacte (chaîne → commutateur de `Factory` ; carte →
premier motif, `dist` = dist, `auto` = dist seulement si non-dev, tout le reste
= source ; aucun motif = `isDev ? source : dist`), avec le motif construit comme
`preg_quote` + `\*` → `.*`, ancré, insensible à la casse. Le refus devient exact
dans les deux sens : une carte qui envoie tous les paquets installés vers `dist`
reste native, et `auto` sur des paquets stables aussi — là où une règle
approximative aurait rendu la main pour rien.

La table du motif est prise **de PHP**, pas devinée : 14 paires passées dans la
regex de Composer. Une de mes attentes était fausse (`a*b` matche `ab`, `.*`
acceptant le vide), et c'est la table qui m'a corrigé.

Cas de banc dans `transitions.sh`, sur un projet **fabriqué** et non une
fixture : toutes nos fixtures à paquets dev sont déjà hors périmètre pour une
autre raison (plugin de disposition, plugin non autorisé), ce qui ne prouverait
que la moitié. Trois cas : `auto` + dev et `source` + dev rendent la main sans
toucher au disque ; `auto` + stable ne rend pas la main — c'est l'affirmation de
précision. Vus rouges avant : sans le correctif, `auto` + dev partait poser le
dist (code 1 sur le dist injoignable, option non nommée).

Reste ouvert, mesuré et écrit dans `docs/plans/v0.20-downloading-lines.md` §7 :
les drapeaux eux-mêmes. `vivacity install --prefer-source`, `-vv`,
`--no-progress` sortent en 2 (erreur d'usage de clap) là où Composer sort en 0.
`--prefer-dist` et `--no-progress` sont des non-opérations pour nous et
devraient être acceptés ; `--prefer-source` et `--prefer-install=source|auto`
devraient router vers le refus qu'on vient de rendre exact ; `-v`/`-vv`
demandent leur propre décision, puisque Composer y imprime davantage.

## 2026-09-30 — Les drapeaux de stratégie d'installation

Fait : `vivacity install --prefer-source` sortait en 2 (erreur d'usage de clap)
là où Composer sort en 0. Même famille que la validation du manifeste racine,
en sens inverse : un mur là où la référence travaille.

Décision : porter `BaseCommand::getPreferredInstallOptions` et **traduire les
drapeaux en la préférence effective** écrite dans la vue du manifeste que
vivacity analyse. Le détecteur juge alors cette préférence par paquet — ce que
je venais de rendre exact — sans aucun câblage nouveau. Conséquences utiles et
non devinées, chacune couverte par un cas :
- `--prefer-dist` **remplace** un `auto` venu de la config : un paquet dev reste
  natif, là où une règle qui ignorerait le drapeau rendrait la main pour rien ;
- `--prefer-install auto` **efface** un `dist` venu de la config, donc un paquet
  dev bascule vers la source et la commande est rendue ;
- `--prefer-install` ne se combine ni avec `--prefer-source` ni avec
  `--prefer-dist` (`InvalidArgumentException`), et une valeur inconnue a son
  propre message.

Les trois messages d'usage sont comparés à Composer dans le banc, code de sortie
compris. Le troisième était **faux dans ma première version** : j'avais
reconstitué son préfixe depuis une lecture tronquée du phar (`Invalid
--prefer-install option, expected …` au lieu de `--prefer-install accepts one of
…`). Quatrième fois de la journée qu'une sortie coupée me coûte un aller-retour ;
c'est le banc qui l'a rattrapé, pas moi.

`--no-progress` est accepté partout où Composer l'a, et c'est une vraie
non-opération : nous n'imprimons pas de barre. À noter — et c'est la méta des
lignes `- Downloading` qui l'a trouvé — `--no-ansi` implique silencieusement
`--no-progress` chez Composer, ce qui est la seule raison pour laquelle aucun
harnais n'a jamais vu cette barre. `--prefer-*` reste refusé sur `update`,
`require` et `remove` : leur chemin relit composer.json après que la vue en
mémoire a disparu, donc porter la préférence effective jusque-là demande du
câblage — et accepter un drapeau qu'on ignorerait serait pire que le refuser.

Vérifié : transitions 13/13 (huit nouveaux cas, dont les deux de précision et
les trois d'usage), steps 251/251, update 19/19, root-manifest 35/35,
flex-update 26/26, le reste du balayage vert, 265 tests.

## 2026-09-30 — La barre de progression : infermable, et le drapeau du banc justifié

Fait : la méta des lignes `- Downloading` signalait la barre de progression
comme un écart « inconditionnel et plus gros », invisible au banc parce que
tous les harnais passent `--no-ansi`, qui l'éteint silencieusement
(`BaseCommand.php:251`).

Mesuré, la conclusion s'inverse. Deux `composer install` du même projet (109
paquets, même machine, cache chaud, sans `--no-ansi`) donnent des cadres
**différents** : `25/109 [======>…] 22 %` contre `26/109 [======>…] 23 %`. Les
cadres dépendent du moment où chaque opération parallèle finit, donc la sortie
n'est pas reproductible — ni par nous, ni par Composer lui-même. Il n'y a pas
de cible à atteindre à l'octet.

Et le contrepoint, mesuré aussi : avec `--no-ansi`, deux runs sont **identiques
à l'octet**. Le drapeau que les harnais passent n'est donc pas un filtre qui
cache un écart, c'est ce qui rend la sortie de la référence comparable. Choix
du banc justifié rétrospectivement.

Décision : rien à porter, et l'item est refermé comme infermable plutôt que
laissé ouvert dans la feuille de route. Ce qui reste vrai et qui relève du
produit, non de la parité : un utilisateur en terminal voit une barre chez
Composer et rien chez nous ; la fournir voudrait dire inventer la nôtre,
qu'aucun banc ne pourrait comparer à la référence.

Structure observée au passage, sans `--no-ansi` et à cache chaud : une barre
indéterminée à la phase de téléchargement, les 109 lignes `- Installing`, puis
huit cadres numérotés après elles.

## 2026-09-30 — Les lignes `- Downloading` : livrées, et un critère faux deux fois

Fait : Composer imprime une ligne par paquet dont son cache de fichiers ne sert
pas l'archive, en bloc avant les lignes d'opérations, dans l'ordre de la
transaction. vivacity n'en imprimait aucune : sur un projet de 109 paquets, une
install à cache froid perdait 109 lignes de stderr.

Design retenu après la méta (qui avait tué le premier) : le même parcours de la
transaction que celui qui produit les lignes d'opérations, en sautant ce qui ne
reçoit pas de `FileDownloader` (pas de dist, dist `path`), et en décidant sur la
présence de l'archive dans le cache de Composer. La divergence restante est une
archive en cache dont le sha1 ne correspond plus : Composer imprime, nous non —
et notre récupération jette l'entrée et retélécharge de toute façon.

Piège trouvé en mesurant, pas en relisant : calculées au moment de l'impression,
les lignes étaient **zéro**, parce que notre propre récupération avait déjà
rempli ce cache entre-temps. Composer décide avant de récupérer ; le calcul est
donc fait au même endroit que `operation_lines`, avant la transaction. C'est
exactement la raison pour laquelle ce dernier était déjà calculé là.

Le critère de succès du plan a été **faux deux fois**, et la seconde erreur n'est
apparue qu'en implémentant. Révision 1 : « `diff-vendor.sh` à cache vide » — or
ce banc lance Composer en `--quiet` (zéro octet de stderr) et ne compare jamais
la stderr. Révision 2 : « retirer les trois filtres » — or les harnais qui
comparent la stderr **partagent** un cache et Composer passe en premier : il
imprime ses lignes *et réchauffe le cache*, si bien que vivacity n'a plus rien à
imprimer. Ces filtres ne compensaient donc pas un manque de notre part mais
l'asymétrie du cache partagé ; les retirer casserait ces cas quoi que fasse
vivacity. Ils restent, documentés pour ce qu'ils sont.

Le critère juste est un banc à part, `harness/download-lines.sh` : un cache de
fichiers **vierge par côté**, stderr comparée **sans aucun filtre**, plus un
second cas à cache chaud des deux côtés où personne n'imprime. Vu rouge avant
(2 lignes chez Composer, 0 chez nous). Mesuré aussi sur laravel : 109 lignes de
part et d'autre, stderr identique à l'octet.

Écart voisin repéré en route et laissé ouvert : sur un projet sans git ni
`version`, Composer avertit `Composer could not detect the root package (X)
version, defaulting to '1.0.0'` et nous n'imprimons rien. Déterministe, donc
portable — contrairement à la barre de progression.

## 2026-09-30 — Évasion de l'extraction zip par chaîne de liens (signalée, reproduite, corrigée)

Fait : une revue externe de la v0.18.0 rapporte qu'une archive zip fabriquée
peut écrire hors du répertoire d'extraction par une chaîne de liens
symboliques. **Reproduit avant de toucher au code** :

    pkg/a       -> lien vers "."      (reste dedans, accepté)
    pkg/a/b     -> lien vers ".."     (parent LEXICAL = `a`, donc `a/..` = racine, accepté)
    pkg/a/b/pwned.txt                 (fichier écrit à travers `b`)

`extract_zip` acceptait l'archive et `pwned.txt` atterrissait **à côté** du
répertoire d'extraction. La cause : `check_symlink_target` résout la cible
*lexicalement*, ce qui suppose que chaque composant du chemin est un vrai
répertoire — supposition que deux entrées suffisent à briser, puisque `a` est en
réalité la racine.

Ce que fait la référence, mesuré sur la même archive :
- `ZipDownloader::extract` appelle `extractWithSystemUnzip`, donc `unzip` ;
- `unzip` **crée** le lien `pkg/a -> .` puis **refuse** chaque entrée dont le
  chemin le traverse : « checkdir error: … exists but is not directory ». Aucune
  évasion ;
- `ZipArchive::extractTo` de PHP, lui, ne crée **aucun** lien : il écrit
  l'entrée comme un fichier ordinaire contenant la cible (1 octet, `.`), puis
  échoue sur l'entrée suivante.

Donc créer le lien est le bon comportement pour la parité (c'est `unzip` qui
fait foi), et le correctif est exactement le refus d'`unzip`, pas un changement
de sémantique. L'extracteur mémorise désormais les liens que l'archive crée et
refuse deux choses : écrire **à travers** l'un d'eux, et un lien ultérieur dont
la cible **traverse** l'un d'eux (`z -> a/../evil`, où la résolution lexicale
paraît interne mais sort une fois `a` suivi). Ce second cas ne provoquait pas
d'écriture hors périmètre à l'extraction, mais laissait sur disque un lien qui
sort dès qu'on le suit.

`extract_tar` n'est pas concerné : une entrée de lien y devient un fichier vide
(sémantique `PharData`), donc aucun lien n'y est jamais créé et la chaîne est
impossible.

Trois tests de non-régression, le premier étant l'archive signalée elle-même,
plus un test qui vérifie que le garde ne coûte pas le cas ordinaire (un lien à
côté de sa cible, lu à travers). Vérifié rouge avant : l'archive passait et le
fichier sortait.

Écart assumé, écrit plutôt que tu : nous refusons l'archive entière là où `unzip`
saute l'entrée fautive et continue — plus strict, et cohérent avec ce que nous
faisons déjà des chemins absolus et des `..`, qu'`unzip` saute aussi.

## 2026-09-30 — Les modes d'un zip viennent de l'archive (et le zip n'avait pas d'oracle)

Fait, mesuré sur des archives fabriquées, `unzip 6.00` d'Apple, umask 022 puis
077 et 002 :

| mode stocké | `unzip` | vivacity (022) | vivacity (077) |
|---|---|---|---|
| 0600 | 0600 | 0644 | 0600 |
| 0640 | 0640 | 0644 | 0600 |
| 0664 | 0664 | 0644 | 0600 |
| 0666 | 0666 | 0644 | 0600 |
| 0700 | 0700 | 0755 | 0755 |
| 04755 | 0755 | 0755 | 0755 |

`unzip` pose le mode stocké **sans appliquer l'umask** et retire les bits
spéciaux. La règle de vivacité — « un bit d'exécution quelque part → 0755,
sinon le défaut du système » (décision du 2026-09-17) — n'était juste que pour
0644 et 0755, les deux seuls modes que portent les zipballs réelles : d'où le
silence de 3 768 paquets de corpus. Et sous un umask autre que 022, **chaque
fichier de chaque paquet** divergeait, y compris pour un dist parfaitement
ordinaire, puisque `fs::write` pose `0666 & ~umask` là où Composer ne bouge pas.

Les deux autres branches, mesurées aussi :
- hôte DOS/FAT sans mode, bit lecture seule : `unzip` pose `0444` **réduit par
  le défaut** (0400 sous umask 077, 0444 sous 002) ;
- hôte DOS/FAT sans mode ni bit : le défaut seul (`0666`/`0777` & ~umask), ce
  que `fs::write` et `create_dir_all` font déjà ;
- hôte unix avec des attributs nuls : `unzip` pose **0000**, fichier illisible.
  C'est la référence, donc c'est ce que nous posons.

Le `unix_mode()` du crate `zip` ne permet pas de suivre ça : il **synthétise**
`0664`/`0775` pour un hôte DOS (`types.rs:562-573`) et rend `None` dès que les
attributs sont nuls, sans exposer l'octet d'hôte. L'octet d'hôte et les
attributs externes sont donc lus dans l'annuaire central (`central_attrs`, 30
lignes), qui devient la seule source de vérité pour les modes. Les modes des
entrées répertoire sont posés **à la fin** : une entrée peut donner à son
répertoire un mode qui interdit d'y écrire (0500) et les fichiers qui suivent
doivent quand même atterrir — `unzip` les restaure à la fin pour la même raison.

Cause structurelle, trouvée en méta : `oracle_tar.rs` existait, `oracle_zip.rs`
non. Le chemin tenu par un oracle était exact, l'autre appliquait une règle
écrite à la main. `crates/vivacity-core/tests/oracle_zip.rs` comble ça : six
cas, `unzip -qq` d'un côté, `extract_zip` de l'autre, comparaison des chemins,
types, contenus, modes et cibles de liens. Vu rouge sur trois des six avant le
correctif.

Conséquence opérationnelle écrite ici parce qu'elle survivra à la version : le
store ne porte **aucune marque de version** (`<store>/<vendor>/<pkg>/<version>-<ref12>`),
donc une entrée extraite par une version antérieure garde les anciens modes et
continue d'être servie. Observé sur la sonde. À traiter avec l'hygiène du store
(plan v0.20, R7).

## 2026-09-30 — PharData retire les bits spéciaux, nous les gardions

`extract_tar` posait `mode & 0o7777`, donc écrivait un fichier **setuid** là où
`PharData::extractTo` n'en écrit pas : mesuré, 04755, 02755 et 01755 sortent
tous en 0755. `oracle_tar.rs` couvrait sept modes ordinaires et aucun bit
spécial — d'où le silence. Correctif d'une ligne (`& 0o777`), trois specs
ajoutés à l'oracle, vus rouges d'abord (2541, 1517 et 1005 contre 493).

## 2026-09-30 — Le plafond de décompression compte les octets réellement lus

Fait : le plafond de 512 Mo additionnait des tailles **déclarées**.

- Zip : les deux champs de taille non compressée sont des `u32` qu'on réécrit
  (en-tête local +22, annuaire central +24). Sonde : 305 991 octets contenant
  300 Mo de zéros, déclarés 1 octet — le plafond ne voit rien, l'entrée est
  mise en mémoire d'un bloc, **pic 607 110 032 o** (`/usr/bin/time -l`), et
  `jobs: 16` multiplie ça par le nombre d'extractions simultanées.
- Tar : pire, le plafond était déjà un no-op. `extract_tar` comptait
  `entry.header().size()` alors que le crate borne ses lectures avec
  `entry.size()`, **écrasé par l'enregistrement pax `size`**. Un `.tgz` dont
  l'en-tête ustar dit 0 et dont le pax dit 4096 faisait lire 4096 octets avec
  un total resté à 0 — le test de non-régression affirme les deux nombres avant
  d'affirmer le refus. `read_tar_entry` utilisait déjà le bon.

La comptabilité porte désormais sur les octets **réellement lus**, contre un
budget unique pour toute l'archive, aux quatre sites de lecture : le fichier, le
lien (`S_IFLNK` plus une taille déclarée à 1 faisait lire une entrée entière
comme chaîne UTF-8), et les deux lecteurs qui répondent aux questions de
périmètre **avant** tout install. `take(allowance + 1)` : le `+ 1` a sa raison,
une lecture qui s'arrête pile sur la limite d'un `Take` rend une lecture courte
à `Crc32Reader`, qui ne vérifie la somme de contrôle qu'en voyant le flux
épuisé — le contrôle d'intégrité serait sauté en silence.

Ce qui reste, écrit : le budget borne le total décompressé, pas le pic par
entrée. La même sonde passe de 607 Mo à **338 Mo** de pic, parce qu'une entrée
est encore mise en mémoire entière (l'indice de capacité est maintenant plafonné
par le budget restant, il ne sert plus à réserver 512 Mio sur ordre de
l'attaquant). L'extraction en flux est un autre chantier, à juger sur mesure de
perf.

Borner n'est pas un durcissement : PharData s'arrête sur le `memory_limit` de
PHP (`Allowed memory size of 134217728 bytes exhausted`, sortie 255) et `unzip`
écrit en flux. C'est donc de la parité.

## 2026-09-30 — Un dist sur le disque est lu, pas téléchargé (dépôts `artifact`)

Fait : `FileDownloader::download` appelle `httpDownloader->addCopy($url)`, et
`HttpDownloader::addJob` confie tout ce qui n'est pas `http(s)://` à
`RemoteFilesystem`, qui l'ouvre avec les flux de PHP. Composer installe donc
sans broncher un `dist.url` qui est un `file://…` **ou un chemin nu** — et un
chemin nu est exactement ce qu'un dépôt `artifact` écrit dans le lock.

vivacity donnait l'URL à reqwest et échouait, store et cache vides :

    Error: HTTP failure for /…/artifacts/acme-widget-1.0.0.zip:
      failed after 3 attempts: builder error

Toute la famille `artifact` partait donc en erreur : installs hors réseau,
paquets privés, miroirs locaux. Et pas en délégation — en erreur sèche. Le
défaut était invisible parce que les deux sondes qui l'ont approché tournaient
sur un cache de Composer déjà chaud (l'archive y était, vivacity la lisait) ou
sur un store déjà rempli.

Décision : lire le fichier. `local_dist_path` reconnaît `file://…` et l'absence
de schéma, rien n'est pourcent-décodé (le wrapper `file` de PHP ne décode rien
non plus), la lecture est **unique** (Composer réessaie trois fois, nous une
seule : un `ENOENT` local ne guérit pas en 250 ms — écart assumé, visible
seulement sur le nombre de lignes `- Downloading` d'un échec, jamais sur un
succès), puis le shasum et l'écriture dans le cache de fichiers de Composer
suivent le même chemin que pour un téléchargement.

Écart qui reste, mesuré : sur un dist local **absent**, Composer imprime
`The "<chemin>" file could not be downloaded: Failed to open stream: No such
file or directory` dans une boîte Symfony ; vivacity imprime `Error: …` à plat.
C'est la famille d'écart déjà assumée pour les erreurs rendues en boîte
(décision `COMPOSER=<fichier>`). **Le code de sortie, lui, n'était pas un écart
assumé mais un défaut** : traité le 2026-10-01, voir l'entrée du jour.

Vérifié : `harness/artifact-repo.sh`, sans réseau — deux paquets fabriqués avec
`ZipArchive`, caches de fichiers et store vierges par côté, codes de sortie,
stderr, `vendor/` entier et le mode de chaque fichier. Les zips portent exprès
0600, 0640, 0666 et 0700, que les zipballs réelles n'ont jamais : le banc tient
donc aussi la règle de modes de l'extraction, et Composer y pose bien ces modes
(mesuré dans le banc lui-même, pas supposé). Lancé contre le binaire 0.19.0
publié, il échoue sur le premier dist.

## 2026-09-30 — Une archive hostile échoue, elle ne rend pas la main (et ce qu'un échec laisse)

Question posée par la méta : `Error::HostileArchive` n'est référencé nulle part
ailleurs dans `crates/` — donc aucune délégation, une erreur sèche, après le
début des écritures — alors que le contrat dit « ce qui n'est pas reproductible
est détecté avant toute écriture et la main est rendue ». Les deux ne peuvent
pas être vrais.

Tranché : l'archive hostile est **la** famille où vivacity échoue au lieu de
rendre la main, et c'est écrit dans le README à côté des autres écarts. La
raison est structurelle et non un choix de confort : tout le reste se décide sur
des métadonnées (lock, manifeste, plugins installés) disponibles avant le
premier octet écrit ; le contenu d'une archive ne se connaît qu'en la lisant, au
milieu d'un install. Rendre la main à ce moment supposerait de défaire ce qui
est déjà posé.

Mesuré, et ça tombe bien : sur l'archive de l'évasion, les deux côtés
échouent. `unzip` refuse la traversée (sortie 2), Composer réessaie avec
`ZipArchive` **par-dessus l'arbre partiel**, ça échoue aussi
(`extractTo(…/pkg/a): Failed to open stream: Is a directory`), et l'install
s'arrête. Codes de sortie 1 des deux côtés.

Ce qui diverge, mesuré sur un projet à deux paquets dont le second est hostile :

| | Composer | vivacity |
|---|---|---|
| code de sortie | 1 | 1 |
| paquet sain | installé dans `vendor/`, inscrit dans `installed.json` | **rien d'écrit** |
| évasion | aucune | aucune |

vivacity n'écrit `vendor/` qu'une fois toutes les archives dans son store, donc
un échec n'importe où laisse `vendor/` intact. Re-lancer converge des deux
côtés, et notre store rend le second essai gratuit — mais l'état intermédiaire
n'est pas le même, donc il est écrit plutôt que sous-entendu.

Deux corrections au passage :
- le message nommait `.tmp-EX4VNV` sous le store, ce qui ne dit rien au lecteur.
  Nouvelle variante `HostileDist { name, version, reason }` : « hostile archive
  refused for acme/evil (1.0.0): entry a/b would be written through the symlink
  a » ;
- le store gagne un **segment de disposition** (`v2`). Une entrée de store est
  un *arbre*, pas une archive : elle porte la règle d'extraction de la version
  qui l'a écrite, donc une entrée écrite avant le changement de modes
  continuait de servir l'ancien arbre, indéfiniment et en silence (observé sur
  une sonde : 0644 servi alors que la référence pose 0600). Le segment est ce
  qui fait prendre effet un changement de sémantique. Les arbres précédents
  restent sur disque, inutilisés.

## 2026-09-30 — La règle de racine unique se décide sur le nom de la racine, pas sur un compteur

Lu dans `ArchiveDownloader.php:118-176` (vendu dans `docs/reference/`) :

    $getFolderContent = Finder::create()->ignoreVCS(false)->ignoreDotFiles(false)
        ->notName('.DS_Store')->depth(0)->in($dir);
    $singleDirAtTopLevel = 1 === count($contentDir) && is_dir((string) reset($contentDir));
    …
    $filesystem->rename($extractedDir, $path);

Trois faits que vivacity ignorait :

1. `.DS_Store` est exclu **par nom**, fichier ou répertoire. Notre règle ne le
   sautait que s'il n'était pas un répertoire — donc un zip fabriqué sur macOS
   avec un `.DS_Store/` comptait comme seconde entrée de premier niveau et
   annulait le retrait de racine, pour tout le paquet.
2. Composer **renomme le répertoire racine sur la destination** : ce qui est à
   côté de lui reste dans le répertoire temporaire et disparaît avec. Nous
   retirions un composant à *chaque* entrée, donc `.DS_Store/junk` atterrissait
   en `junk` à la racine du paquet. La règle rend maintenant le **nom** de la
   racine, ce qui rend « à côté de la racine » exprimable ; `strip_root` jette
   ce qui n'en vient pas.
3. `is_dir()` **suit les liens**. Une archive dont tout le premier niveau est un
   lien symbolique garde donc le retrait, et le `rename` déplace **le lien** sur
   `vendor/<nom>` : mesuré, le paquet installé *est* un lien
   (`vendor/hostile/rootlink -> ..`). Une entrée de store est un arbre de
   répertoires : rien là-dedans ne peut devenir un lien. Nous posions un
   répertoire contenant le lien — un `vendor/` silencieusement différent, la
   pire catégorie. Refusé désormais, avec la raison, et insensible à un
   `.DS_Store` posé à côté.

Le cas `tar` n'est pas concerné : une entrée de lien y devient un fichier vide
(PharData), donc `is_dir()` répond faux des deux côtés et le retrait ne
s'applique ni ici ni là.

Vérifié : `oracle_zip.rs` gagne le cas `.DS_Store/` (vu rouge : l'oracle rendait
`composer.json` + `src/A.php`, nous `pkg/…` intact), le test unitaire du lien
racine couvre les trois formes (lien seul, lien plus `.DS_Store`, deux entrées
où le lien reste un lien dans le paquet), 282 tests, et les harnais les plus
exposés au retrait de racine restent verts : diff-vendor 12, tar-dist 7,
path-repos 14, artifact-repo 9, steps 251, flex-install 7, custom-dirs 4,
wp-core 5, package-versions 7, yii2-composer 8.

Au passage, l'oracle zip modélise maintenant le `rename` de Composer plutôt
qu'un « remonter le contenu » : sans ça il ne pouvait pas voir le point 2.

## 2026-09-30 — Les horodatages d'un dist zip : écart assumé, avec ce qu'il coûterait

Mesuré, archive datée du 2020-01-02 03:04:06 :

| | fichier | répertoire |
|---|---|---|
| `unzip` (référence zip) | 2020-01-02 03:04:06 | 2020-01-02 03:04:06 |
| install Composer réel | 2020-01-02 03:04:06 | — |
| vivacity | l'heure de l'install | l'heure de l'install |
| `PharData` (référence tar) | l'heure de l'extraction | — |

Donc l'écart est propre au zip : côté tar, ne rien poser **est** la parité.

Écart assumé, et la raison est écrite plutôt que sous-entendue : un horodatage
DOS est une heure locale *murale*, qu'`unzip` convertit avec les règles de
changement d'heure en vigueur **à cette date**. Reproduire ça exactement demande
une base de données de fuseaux (les approximations bon marché — décalage courant,
UTC — se trompent une moitié de l'année), pour une propriété dont aucun contenu
de fichier, aucun mode et aucune cible de lien ne dépend, et que le contrat ne
revendique pas (`compare_vendor` ne compare pas les mtimes, le README promet
« modes et cibles de liens »). Les dépendances `filetime` et `time` sont déjà
dans le graphe (via `tar` et `zip`) mais aucune ne donne le décalage historique.

Consigné dans le README à côté des autres écarts et dans CONTRIBUTING comme
chantier ouvert, avec ce qu'il exige.

## 2026-09-30 — Windows : Composer cherche `7z` là où nous ne cherchions pas

`ZipDownloader::__construct` construit sa liste de commandes avec
l'`ExecutableFinder` de Symfony, et sous Windows il cherche **`7z` d'abord**,
avec `C:\Program Files\7-Zip` ajouté au chemin de recherche — donc hors `PATH` —
puis `unzip` (`ZipDownloader.php:47-51`, vendu dans `docs/reference/`). Notre
sonde ne regardait que le `PATH` et ne connaissait pas cette priorité : sur une
machine qui a 7-Zip installé normalement mais pas dans le `PATH`, nous
répondions « aucun outil » là où Composer en trouve un, et une entrée de lien
devenait un fichier ordinaire au lieu d'un lien.

La découverte est alignée. Ce qui **n'est pas** tranché, et qui est écrit au
point où ça compte plutôt que deviné : est-ce que `7z x -y` recrée une entrée de
lien *en tant que lien* (il lui faut peut-être `-snl`) ? La réponse décide de
cette branche sur une machine qui a 7-Zip et pas `unzip`, et elle demande une
mesure **sous Windows**, pas une supposition. Consigné dans CONTRIBUTING.

Au passage, `ZipDownloader.php` et `TarDownloader.php` entrent dans
`docs/reference/` avec leur jumeau dans `drift-reference.sh` : tout le travail de
ce sprint repose sur leur comportement, et le script ne surveille que ce qui est
versé (129 fichiers désormais, contre 127).

## 2026-10-01 — Un échec de transport sort en 100, et les réessais sont ceux de Composer

Fait, lu dans le phar : `Application::doRun` attrape une `TransportException`,
**réécrit son code** en `Installer::ERROR_TRANSPORT_EXCEPTION` par réflexion
(`Application.php:502-506`, `Installer.php:92` = 100) et la relance pour que
Symfony Console en fasse le code de sortie. Donc tout ce que Composer n'a pas
réussi à **récupérer** sort en 100 — mesuré sur trois formes du même échec :

| cas | Composer | vivacity avant | maintenant |
|---|---|---|---|
| dist qui répond 404 | 100 | 1 | 100 |
| dépôt `composer` injoignable | 100 | 1 | 100 |
| dist local absent | 100 | 1 | 100 |

Un script qui branche sur le code voyait donc une erreur générique au lieu d'un
problème de réseau, pour **tous** les échecs réseau, pas seulement ceux de ce
sprint.

Ce que ça a demandé : la nature de l'erreur était perdue en route. Le résolveur
distinguait déjà `RepoErrorKind::Transport` de `Data`, mais chaque couche
traversée l'aplatissait en texte — `From<RepoError> for PoolError` jetait le
drapeau, `From<PoolError> for SessionError` aussi, et le CLI faisait
`anyhow!("{e}")`. Le drapeau voyage maintenant de bout en bout : `PoolError` et
`FilterError` portent le `RepoErrorKind` (une seule énumération pour les trois
couches, pas une par couche), `SessionErrorKind` gagne `Transport`, et le CLI
convertit ça en un `TransportFailure` typé dont le `Display` est le message seul
— la ligne `Error:` ne bouge pas. `exit_code_of` cherche dans la chaîne soit ce
type, soit `Error::Http` (le chemin de l'install, qui garde son type).

**La politique de réessai aussi est celle de la référence**, portée depuis
`CurlDownloader` (2.10.3, lignes 384-398 et 465-478) :

- une erreur de transport n'est réessayée que pour les errno curl 6 (hôte non
  résolu), 7 (connexion impossible), 28 (délai), 16/92 (http2), 56/35 avec
  « Connection reset by peer » ;
- un **statut** n'est réessayé que s'il vaut 423, 425, 500, 502, 503, 504, 507,
  510, ou 400 depuis `codeload.github.com`. Un 404, un 401, un 403 sont donc
  définitifs, là où vivacity réessayait trois fois — plus lent, et il annonçait
  « failed after 3 attempts » à propos d'un verdict qui ne bougeait pas ;
- trois réessais au plus (donc quatre tentatives), délais de
  `restartJobWithDelay` : rien, puis 100 ms, puis 500 ms. Les nôtres étaient
  500 ms puis 1 s, et pour trois tentatives seulement.

Mesuré : le 404 passe de ~2 s à **158 ms**, et son message ne parle plus de
tentatives.

Vérifié : `harness/transport-exit.sh`, sans réseau (un `php -S` qui répond 404,
un port fermé, un fichier absent), compare les codes de sortie des deux côtés,
vérifie l'absence de réessai sur le 404 et que le message ne mentionne pas de
tentatives. Lancé contre le binaire 0.19.1 publié : quatre des cinq contrôles
rouges. Plus deux tests unitaires qui épinglent la liste des statuts
réessayables, l'exception `codeload.github.com` et les trois délais.

## 2026-10-01 — Le comparateur se compare lui-même (tolérances ancrées)

`harness/lib/compare.sh` est l'oracle de seize harnais et du corpus de 106
projets : chaque « 0 diff » du projet passe par lui. Trois de ses tolérances
étaient des `grep -v` en **sous-chaîne**, donc elles effaçaient aussi les
différences qu'elles n'étaient pas censées couvrir :

| tolérance prévue | ce qu'elle effaçait aussi |
|---|---|
| `vendor/autoload_runtime.php` (stub d'émulation) | un paquet livrant `src/autoload_runtime.php`, présent d'un seul côté **ou différent** |
| les lignes d'erreur de `diff` sur un lien pendant | un fichier nommé `No such file or directory` |
| `setApcuPrefix` dans `autoload_real.php` (préfixe aléatoire) | la même ligne dans **n'importe quel** fichier d'un paquet |

Chaque tolérance nomme désormais le chemin exact auquel elle s'applique : le
stub est reconnu à `Only in <vendor>: autoload_runtime.php` et nulle part
ailleurs, la plainte de `diff` à son préfixe `diff: ` et son suffixe exact, et le
`-I setApcuPrefix` n'est passé que pour `*/composer/autoload_real.php`.
L'inventaire `stat` applique la même règle par suffixe exact, de sorte qu'un
`autoload_runtime.php.bak` n'est plus couvert.

Ajouté aussi : l'inventaire **refuse bruyamment** quand le nombre de fichiers
trouvés ne correspond pas au nombre de lignes produites — un nom contenant un
saut de ligne casserait l'hypothèse « une ligne par fichier » des deux côtés à la
fois, donc sans faire échouer la comparaison.

Vérifié par `harness/compare-selftest.sh` : onze paires d'arbres fabriquées, un
verdict exigé pour chacune (dont une différence de mode et une différence de
cible de lien, que `diff -r` ne voit pas). Contre le comparateur précédent,
**quatre des onze sont rouges** — les quatre trous du tableau. Et le self-test a
immédiatement attrapé un défaut de ma propre correction : `"${tableau[@]}"` vide
sous `set -u` fait échouer le bash 3.2 de macOS, ce qui faisait répondre
« identiques » à trois cas. C'est exactement ce qu'un oracle sans test laisse
passer.

Les seize harnais qui l'utilisent restent verts : diff-vendor 12, tar-dist 7,
path-repos 14, artifact-repo 9, flex-install 7, vendor-dir 6, custom-dirs 4,
wp-core 5, yii2-composer 8, bin-plugin 5, package-versions 7, merge-plugin 16,
root-manifest 35, root-scan 12, scripts 4.

## 2026-10-03 — La famille à deux étages : mesurée, et elle tombait juste

`ZipDownloader::extractWithSystemUnzip` lance `unzip -qq <fichier> -d <chemin>`
**sans `-o`**, donc tout code de sortie non nul déclenche `$tryFallback` :
trois avertissements, puis `extractWithZipArchive` **par-dessus l'arbre déjà
partiellement écrit** (ZipDownloader.php:51 et 146-179). La référence de ces
archives est à deux étages — ce que la révision 1 du plan v0.20 avait manqué, et
que la méta avait relevé.

Mesuré sur trois archives fabriquées, sans réseau :

| archive | `unzip` | Composer | vivacity |
|---|---|---|---|
| deux entrées du même nom (mode stocké 0600) | sort 1 (invite `replace …?`, EOF = `[N]one`) | repli ZipArchive, install **réussi** (code 0) : la dernière entrée gagne, le mode 0600 survit | **identique**, code 0, même arbre |
| fichier `a` puis entrée `a/b` | sort 2 (`checkdir error`) | ZipArchive échoue aussi, install échoué (code 1), rien d'installé | code 1, rien d'installé |
| entrée `a/b` puis `a` comme fichier | sort 2 | idem | idem |

Le point important du premier cas : `ZipArchive::extractTo` ouvre le fichier en
écriture, donc il **écrase le contenu sans toucher au mode** qu'`unzip` avait
posé. L'arbre final est donc « mode du zip + contenu de la dernière entrée », ce
que vivacity produit déjà en un seul passage. La famille tombait juste ; elle
n'était simplement pas prouvée.

Deux choses ajoutées :

1. `harness/two-stage.sh` la prouve. La stderr de Composer y porte deux jetons
   **aléatoires** (`vendor/composer/tmp-<32 hex>.zip` et
   `vendor/composer/<8 hex>`, `bin2hex(random_bytes(4))`,
   ArchiveDownloader.php:71) : elle n'est pas reproductible d'une exécution à
   l'autre, même chez Composer. Le banc retire les cinq lignes du repli **en
   exigeant de les trouver** — une ligne qui disparaîtrait amont fait échouer le
   banc au lieu de passer inaperçue — puis compare le reste sans filtre. Écart
   assumé et écrit : nous n'imprimons aucune des cinq.
2. La collision fichier/répertoire est refusée **par son nom** plutôt que par un
   `EEXIST`. Avant : `Error: I/O error at <store>/.tmp-W4bFVb/a: File exists
   (os error 17)`. Maintenant : `hostile archive refused for acme/collide
   (1.0.0): entry a/b would be written under a, which the archive wrote as a
   file`, et dans l'autre ordre `entry a is a file where the archive already
   made a directory`. Même code de sortie, même absence d'écriture : seul le
   message change, et il dit lequel des deux cas s'est produit.

Vérifié : banc 12 cas verts ; contre le binaire 0.19.1 publié, les dix premiers
passent (l'arbre et les codes étaient déjà bons) et les deux contrôles de message
sont rouges. Trois cas unitaires de plus dans `extract.rs`, dont celui qui
vérifie qu'une entrée **répertoire** portant le nom d'un répertoire déjà fait
n'est pas un conflit.

## 2026-10-03 — Windows : mesurer le vrai outil plutôt que raisonner, et le vérifier localement

La question laissée ouverte le 2026-09-30 (`7z x -y` recrée-t-il une entrée de
lien *en tant que lien*, ou lui faut-il `-snl` ?) ne se tranche pas par la
lecture : elle décide de notre branche sur une machine qui a 7-Zip et pas
`unzip`, et personne ici n'a de Windows.

Donc le test la mesure. `a_symlink_entry_follows_whatever_tool_composer_would_use`
(`#[cfg(windows)]`) : il fabrique un zip avec une entrée de lien, cherche l'outil
**dans l'ordre de Composer** (`7z.exe` sur le PATH puis dans
`C:\Program Files\7-Zip`, sinon `unzip.exe`), le lance **avec les arguments de
Composer** (`x -bb0 -y %file% -o%path%` ou `-qq %file% -d %path%`), regarde ce
qu'il a produit, sonde si le processus a le droit de créer un lien, puis exige
d'`extract_zip` la même issue : un lien quand l'outil en a fait un et que le
droit existe, les octets de la cible dans un fichier ordinaire sinon. Le job
`windows` le lance avec `-- --nocapture` pour que la réponse du runner soit dans
le journal, pas seulement dans un échec.

Outillage, consigné dans HANDOVER parce que ça change la façon de travailler sur
ce job : le code Windows se **typecheck localement** maintenant
(`brew install mingw-w64`, `rustup target add x86_64-pc-windows-gnu`, puis
`cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings`,
~15 s à chaud). Les deux rouges Windows de ce sprint étaient une erreur de
compilation et un lint, c'est-à-dire exactement ce que ça attrape — pour une
boucle CI qui coûte une heure. L'exécution réelle reste au runner.

## 2026-10-03 — L'extraction sort du store servi, et les orphelins sont ramassés

Fait : `Store::ensure` extrayait dans un `.tmp-XXXX` créé **dans le répertoire
parent de l'entrée**, donc à l'intérieur du store lui-même. L'évasion corrigée
en 0.19.0 y posait son fichier (relevé par la méta : « l'inventaire hors projet
n'aurait pas attrapé l'évasion d'origine, elle atterrit dans le store »), et plus
rien ne l'en enlevait.

Ceinture et bretelles, maintenant que l'extracteur refuse de sortir de son
répertoire : le staging devient `<cache>/store/.staging/`, **hors de la
disposition servie** (`<cache>/store/v2/…`). Une entrée n'est servie que depuis
`v2` et rien d'autre n'est jamais lu, donc ce qu'une extraction écrirait à côté
de son arbre ne peut plus devenir un paquet. Même système de fichiers que `v2`,
ce que le `rename` exige — « hors du store » au sens d'un autre point de montage
casserait l'atomicité, ce qui serait un recul.

Ajouté avec : un balayage des stagings qu'aucun processus vivant ne peut plus
posséder, au passage, à l'entrée d'`ensure`. Seuil **six heures** : bien au-delà
de n'importe quel install (celui d'un staging parallèle a des minutes), assez
court pour qu'un processus tué ne laisse pas un arbre pour un mois. Silencieux
sur toutes les erreurs : c'est de l'entretien, pas le contrat de l'install, et un
autre processus peut être en train de supprimer la même entrée.

Vérifié : le test du store exige que le staging existe, qu'il soit **vide** après
une extraction réussie, qu'un orphelin vieilli de sept heures (`filetime`, déjà
dans le graphe via `tar`, déclaré en dev-dependency pour ne pas attendre six
heures) soit ramassé et qu'un staging frais soit laissé tranquille. Install réel
de Laravel : `store/v2` et `store/.staging` côte à côte, staging vide.

## 2026-10-03 — Le nom d'une entrée est des octets, comme la référence le traite

Fait, mesuré contre `unzip` par `oracle_zip.rs` :

| nom stocké | `unzip` | vivacity avant | maintenant |
|---|---|---|---|
| `café.txt`, **drapeau UTF-8 absent** (zip fabriqué sous Windows) | `café.txt` | **`caf├⌐.txt`** | `café.txt` |
| nom non valide en UTF-8 (`pkg/<0xE9>ONUTF8A`) | sortie **50** : APFS refuse le nom | **ok**, un nom inventé | échec (`Illegal byte sequence`), comme la référence |

Le crate `zip` lit un nom en **cp437** quand le drapeau UTF-8 n'est pas posé, et
nous posions cette lecture. La première ligne n'a rien d'hostile : c'est un
paquet avec un accent dans un nom de fichier, zippé par un outil Windows — et
nous l'installions sous un autre nom. La seconde est pire dans l'autre sens :
nous **réussissions** là où l'install de Composer échoue, en inventant un nom
que le système accepte.

Correctif : sur unix, les octets bruts du nom vont au système de fichiers, et
les mêmes contrôles de composants s'appliquent (pas de `..`, pas d'absolu ;
`enclosed_name` reste consulté pour ce qu'il refuse en plus). `extract_tar`
reçoit le même traitement — il refusait tout nom non UTF-8, donc il refusait des
archives que `PharData` installe (`PharData` donne les octets au système, comme
`unzip`).

Sous Windows, le nom décodé est gardé : un nom de fichier doit y être
convertible en UTF-16, les octets ne sont pas une option. Ce que 7-Zip fait d'un
tel nom là-bas reste **non mesuré**, consigné dans CONTRIBUTING.

Les deux oracles gagnent une forme de comparaison qui le rend mesurable : quand
la référence **refuse** l'archive, ce qui est comparé est le verdict (nous devons
refuser aussi) ; quand elle l'accepte, les arbres sont comparés comme d'habitude.
Sans ça la bonne attente dépendait du système de fichiers (APFS refuse le nom,
ext4 l'accepte) et aucun test n'aurait pu l'exprimer. Et `unzip` est désormais
lancé **sans stdin** dans l'oracle : il pose des questions (écrasement, nom
illisible) et attendait indéfiniment — Composer lui donne des tuyaux, donc il lit
EOF et décide seul. Le premier essai de ce test a bloqué dix minutes avant que je
voie pourquoi.

Vu rouge avant : `café.txt` contre `caf├⌐.txt` dans l'inventaire de l'oracle, et
« unzip exited Some(50); extract_zip said ok ».

## 2026-10-03 — 200 000 entrées : les gardes balayaient là où il fallait chercher

Mesuré (M4 Max, archive `artifact` de 25,4 Mo, 200 001 entrées, caches et store
vierges) :

| | durée | pic mémoire |
|---|---|---|
| Composer (`unzip`) | 24,4 s | 76 Mo |
| vivacity, avant | **325 s** (307 s de CPU) | 209 Mo |
| vivacity, après | **13,8 s** (0,61 s de CPU) | 197 Mo |

Cause : les deux gardes d'extraction — refuser d'écrire **à travers** un lien que
l'archive vient de créer, et **sous** un chemin qu'elle a écrit comme fichier —
parcouraient tout leur ensemble à chaque entrée (`set.iter().find(…)`), soit
O(entrées²). À 200 000 entrées ça fait 2·10¹⁰ comparaisons de chemins.

Correctif : chercher les **ancêtres de l'entrée** dans l'ensemble plutôt que
parcourir l'ensemble, ce qui est O(profondeur) et strictement équivalent (un
ancêtre présent ⟺ un élément dont le chemin est un préfixe de composants). La
durée tombe sous celle de la référence, et le temps CPU s'effondre : tout est
devenu des appels système.

Portée : la garde des fichiers n'a **jamais été publiée** (elle date du
2026-10-03, après la 0.19.1). Celle des liens est dans la 0.19.0, où elle ne
parcourt que les liens créés par l'archive : négligeable sur un dist réel,
quadratique sur un dist qui en livre beaucoup.

Le pic mémoire reste supérieur à celui de Composer (197 contre 76 Mo) : nous
tenons l'annuaire central et la liste des entrées, les deux linéaires, là où
`unzip` écrit en flux. C'est écrit parce que c'est mesuré ; aucun refus n'est
ajouté, Composer n'en ajoute pas non plus.
