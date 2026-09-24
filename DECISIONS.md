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

Vérifié : flex-update 18/18 (les 13 nouveaux cas rouges avant), transitions,
vendor-dir, flex-install 7/7, update 19/19, steps 240/240, 256 tests.
