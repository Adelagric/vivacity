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

## 2026-10-03 — Noms réservés Windows et chemins > MAX_PATH : demander à l'outil

Deux classes qui ne se tranchent pas par le raisonnement :

- **Les noms de périphériques** (`CON`, `NUL`, `COM1`…, et les noms à point ou
  espace final) : écrire dans `NUL` **réussit** et jette tout. Un extracteur peut
  donc annoncer un succès pour un fichier qui n'existe pas — c'est le piège, et il
  ne se voit pas sans regarder.
- **Un chemin de plus de 260 caractères** : Windows le refuse sans l'option de
  chemins longs, et 7-Zip serait capable de passer outre avec le préfixe `\\?\`.

Les deux tests (`#[cfg(windows)]`) lancent donc **l'outil que Composer
lancerait** — découverte et arguments factorisés avec le test des liens —
impriment ce qu'il a produit entrée par entrée (`--nocapture` dans le job
`windows`, pour que la réponse du runner soit dans le journal) et exigent
l'accord : si l'outil refuse l'archive, nous devons refuser aussi ; s'il la pose,
chaque entrée doit exister des deux côtés ou d'aucun.

Pourquoi « exiger l'accord » plutôt qu'affirmer un comportement : la réponse
dépend de la machine (7-Zip ou `unzip`, option de chemins longs, droit de créer
des liens). Un test qui affirme une issue serait vrai sur le runner et faux chez
un utilisateur ; un test qui exige l'accord est vrai partout — et quand il casse,
il raconte ce que la machine a répondu.

Vérifié localement par `cargo clippy --target x86_64-pc-windows-gnu
--all-targets -- -D warnings` (voir HANDOVER) ; l'exécution appartient au runner.

## 2026-10-07 — Noms impossibles sous Windows : 7-Zip renomme, il ne saute pas

La mesure demandée le 2026-10-03 a répondu, dans le journal du runner :

    tool laid out:     ["_COM1", "_CON", "_NUL", "ok.txt", "trailing_"]
    vivacity laid out: ["COM1",  "CON",           "ok.txt", "trailing"]

Donc 7-Zip — que Composer préfère sous Windows — **renomme** : un nom de
périphérique réservé prend un `_` devant, et un point ou espace final devient
`_`. Le `trailing_` unique dit autre chose au passage : `trailing.` et
`trailing ` atterrissent sur le **même** nom, donc deux entrées se télescopent et
la dernière gagne — ce qui est déjà la règle de l'extraction.

Ce que vivacity faisait : écrire `CON` et `COM1` tels quels (les API que Rust
utilise le permettent), **perdre `NUL` dans le périphérique** — un fichier
annoncé installé qui n'existe pas — et laisser Windows retirer le point et
l'espace finals. Quatre noms que la référence ne produit jamais.

Correctif : `on_disk` applique la règle **par composant de chemin**, donc un
répertoire au nom réservé est renommé aussi (cas ajouté à l'échantillon, il
n'était pas mesuré). La liste de noms réservés est celle de Composer, celle que
porte `hasPackageNamingError` pour les noms de paquets ; elle est recopiée ici
parce que le résolveur dépend de ce crate et pas l'inverse.

Le test ne compare plus un nom à la fois mais **l'arbre entier** contre celui de
l'outil, et imprime les deux listings : une forme que personne n'a anticipée
apparaît dans le journal au lieu de passer en silence. L'échantillon s'élargit à
`con` (minuscule), `CON.txt`, `LPT1`, `PRN`, `AUX`, `two..` et `CON/inside.txt`.

Écart qui reste, écrit dans CONTRIBUTING : la règle est celle de 7-Zip. Ce que
fait l'`unzip` d'Info-ZIP des mêmes noms sur une machine sans 7-Zip n'est pas
mesuré — et le test le dira là-bas, puisqu'il compare à l'outil qu'il trouve.

## 2026-10-07 — La casse du dernier gagne aussi (dernière des trois mesures Windows)

Les trois questions séparées ont répondu une par une, et c'est bien le découpage
qui les a rendues lisibles : tant que les trois tenaient dans un seul
échantillon, le cas qui faisait échouer l'outil masquait les deux autres.

1. **Noms réservés et points/espaces finals** : la règle de `on_disk` est
   désormais exacte (le test compare l'arbre entier à celui de l'outil et il
   passe), y compris `two..` → `two__` — la suite est remplacée caractère par
   caractère.
2. **Nom réservé en composant de répertoire** : l'outil renomme puis se heurte à
   son propre `checkdir error` (sortie 2) et nous refusons. Accord.
3. **Deux entrées ne différant que par la casse** (`CON` puis `con`) : un seul
   fichier des deux côtés, mais

       tool laid out:     ["_con"]
       vivacity laid out: ["_CON"]

   L'outil garde le nom de la **dernière** entrée, nous celui de la première —
   parce que réécrire un fichier existant sous une autre casse conserve le nom
   d'origine sous Windows. Donc 7-Zip supprime avant de créer, et sa casse suit
   la dernière entrée comme son contenu.

Correctif : sous Windows, le fichier existant est supprimé avant l'écriture.
Rien n'est perdu à le faire là-bas, puisque les modes n'y sont pas appliqués —
contrairement à unix, où c'est justement la survie du mode à travers l'écrasement
qui fait l'arbre du cas « doublon » (DECISIONS 2026-10-03), d'où le `cfg`.

## 2026-10-09 — Le cache de la sonde ne garde que ce qu'il voit

La known issue publiée avec la 0.20.0 : la clé de `platform-probe.json` (binaire
PHP, `.ini`, quatre variables) ignorait les bibliothèques que PHP charge et ses
extensions partagées ; après `brew upgrade pcre2`, `lib-pcre` restait à 10.47
quand PHP disait 10.49. Plan et méta adversariale dans
`docs/plans/v0.21-platform-probe-cache.md`.

**Le piège qui rendait le correctif évident faux**, mesuré : Homebrew garde
l'ancien keg (`Cellar/pcre2/10.47_1` et `Cellar/pcre2/10.49` coexistent) et ne
déplace que le lien `opt/pcre2`. Une clé faite des chemins résolus des images
chargées — ce que donnent `/proc/self/maps` et la trace de dyld — restait valide
après la mise à jour, puisque le vieux fichier n'avait pas bougé.

**Ce que la méta a cassé dans la révision 1**, vérifié ici :
- un wrapper `#!/bin/sh exec php "$@"` donne **0 image** (SIP efface `DYLD_*` à
  travers `/bin/sh`), alors que la sonde voit le vrai binaire : le plan r1
  déduisait le marqueur Homebrew des images, donc sans image le bug restait
  entier derrière n'importe quel wrapper ou shim ;
- `Probed` exige `name` et `version` : une entrée `kind: images` dans la sortie
  aurait cassé `platform_packages` dès le second run ;
- `install` fait `probe().ok()` : toute erreur nouvelle serait devenue « pas de
  PHP », contrôle de plateforme sauté en silence ;
- `COMPOSER_ALLOW_XDEBUG` était lu par la sonde et absent de la clé.

**Tranché** :
1. la sonde rapporte ses faits **à côté** de ses entrées (`binary`, `ext_dir`,
   `maps`), jamais dedans ;
2. **échec fermé** : rien n'est mis en cache si la sonde n'est pas le binaire
   localisé ou, sous unix, si ses images n'ont pas été vues — la sonde tourne
   alors à chaque fois, sans erreur. 40 ms contre un lock que Composer
   n'écrirait pas ;
3. la clé : chaque image sur disque avec mtime, taille, **ctime et inode** ; la
   **cible du lien `opt/<formule>`** de chaque keg chargé (exact sur le
   repointage, aveugle à un `brew install` sans rapport — préféré au mtime du
   répertoire `opt` entier) ; les marqueurs système ; dix variables
   d'environnement ; un numéro de format **comparé** et le hachage de la sonde ;
4. course : marqueurs et répertoire `opt` relevés avant et après la sonde ;
   différents → rien n'est écrit.

Mesuré sur Laravel no-op (`VIVACITY_TRACE`, phase « platform check », M4 Max) :
0.20.0 3,3–4,2 ms, nouvelle clé 4,0–4,7 ms, sans cache 41,8–47,3 ms. La clé de
ce Mac : 54 images, 39 liens de keg, 24 marqueurs.

Vérifié par sept tests, dont le rouge d'abord (le lien repointé, l'ancien keg
intact : la 0.20.0 sert 10.47), le remplacement en place, le wrapper jamais mis
en cache (deux appels, deux lancements), le vrai php vu et dont le cache égale
une sonde fraîche, un cache d'ancien format qui ne valide jamais, et les deux
formats de trace de dyld. Le canal Linux (`/proc/self/maps`) et le canal
Windows ne s'exécutent que sur leurs runners.

### 2026-10-09 (suite) — Ce que la revue indépendante a trouvé dans le correctif

Une revue par un sous-agent qui n'avait pas écrit le code, avant le push. Trois
régressions de la règle « ne jamais transformer une sonde qui marchait en
erreur » — chacune aurait fait sauter le contrôle de plateforme en silence,
puisque `install` fait `probe().ok()` :

- `disable_functions=realpath` (ou `file`) **tuait la sonde entière** : sortie
  255, mesuré ici. Le bloc des faits est désormais sous `try { … } catch
  (\Throwable)` et remet tout à null en cas d'échec — null veut dire « pas de
  cache », pas « pas de PHP ». Test : un wrapper qui désactive `realpath`, `file`
  et `ini_get` rend les mêmes entrées qu'une sonde directe ;
- un chemin non UTF-8 dans `maps` faisait rendre `false` à `json_encode`, donc
  **aucune sortie** : les faits sont encodés à part et annulés s'ils échouent,
  sans toucher à l'encodage des entrées ;
- sous macOS, un wrapper qui fusionne stderr dans stdout recevait la trace dyld
  dans le JSON : une sortie illisible sous la trace est relancée une fois sans
  elle, et ce run n'est pas mis en cache.

Et deux défauts qui rendaient le cache inutile ou partiel :

- sous Linux, `opcache.enable_cli=1` crée un segment de mémoire partagée qui
  apparaît comme `/dev/zero (deleted)` — que la sonde prenait pour un fichier
  supprimé, donc **jamais de cache** sur une image Docker courante. Les
  pseudo-fichiers de mémoire partagée (`/dev/zero`, `/SYSV…`, `/memfd:`,
  `/dev/shm/`) sont ignorés avant le test ;
- sous `open_basedir`, `is_file` répond faux hors des répertoires permis : des
  bibliothèques sortaient de la clé **en silence**. Le filtre est retiré de la
  sonde ; le Rust garde ce qui existe.

Plus : les marqueurs relevés **une fois avant** la sonde et stockés tels quels
(la course prouve qu'ils n'ont pas bougé), la course vérifie aussi les
répertoires `opt` des préfixes Homebrew standards, la branche Windows échoue
fermé quand un répertoire ne se lit pas ou que `extension_dir` est relatif, et
le test du vrai php **prouve** maintenant que le second appel est servi par le
cache (le fichier n'est pas réécrit) — l'égalité des deux réponses seule aurait
tenu même si le cache ne validait jamais.

Reste écrit : FreeBSD et les autres unix sans `/proc` n'ont pas de liste
d'images, donc pas de cache — 40 ms par run de plus qu'en 0.20.0, le prix de
l'échec fermé.

## 2026-10-09 — Le canal de dérive se taisait derrière son premier échec, et les oracles testaient un Composer figé

**Le rapport hebdomadaire était incomplet depuis des semaines.** L'étape 2 de
`drift.yml` (`cargo test` puis six harnais) tournait sous `bash -e` : le premier
test en échec contre `2.11-dev` — un écart attendu, `plugin-api-version` 2.9.0 →
2.11.0 — terminait l'étape, et **aucun harnais ne tournait derrière**. Le rapport
disait « tests + harness : failure » sans dire qu'il n'avait presque rien
regardé. Corrigé : chaque commande tourne quoi que dise la précédente, `cargo
test --no-fail-fast`.

Rejoué en local contre `2.11-dev+43f74517` pour ne pas attendre une semaine :
**8 tests et 127 cas de harnais** diffèrent, là où le rapport en montrait un. Tous
se ramènent à trois causes, mesurées et confirmées dans la source de 2.11-dev —
`PLUGIN_API_VERSION` à `2.11.0`, un nouveau champ de lock `published-time`
(`ArrayDumper.php:98-99`, `ArrayLoader.php:259-260`), et semver #187. L'install,
elle, est inchangée (diff-vendor, removal, transitions, boot, root-manifest
verts). La liste de ré-épinglage est dans `docs/plans/repin-composer-2.11.md`.

**Les onze oracles PHP copiaient le phar de Composer une fois pour toutes.**
Chacun portait son propre bloc « si `$TMPDIR/vivacity-oracle-composer.phar`
n'existe pas, copier `which composer` » : après une mise à jour de Composer, ils
continuaient de tester l'ancien, sans un mot — la même classe de défaut que le
cache de sonde corrigé le même jour. Trouvé parce que tester contre le snapshot
exigeait un `TMPDIR` à part. Un seul helper maintenant, `tools/oracle_phar.rs`,
inclus par `#[path]` dans les trois crates : la copie est nommée d'après
l'identité de la source (chemin canonique, taille, mtime), et écrite par un
renommage atomique puisque les binaires de test tournent en parallèle. Vérifié :
le Composer 2.10.3 et le snapshot donnent deux copies distinctes ; 296 tests
verts.

## 2026-10-14 — `lib.rs` découpé par commande, la ligne de commande laissée en place

Point de forme de la critique externe du 2026-09-30 : `crates/vivacity/src/lib.rs`
faisait 4 649 lignes, les cinq commandes et tout ce qu'elles partagent dans un
fichier. Il en fait 548 : `install.rs` (1 407), `update.rs` (1 090),
`flex_install.rs` (667), `dump.rs` (305), `remove.rs` (300), `manifest.rs` (208),
`fallback.rs` (191). Carte et mécanique dans `docs/plans/split-lib-rs.md`.

**Un déplacement, pas une réécriture.** Chaque item part avec ses commentaires
de doc et ses attributs, devient `pub(crate)`, et chaque module commence par
`use super::*;` — `require.rs`, qui importe par `crate::…`, n'a pas bougé.
Vérifié mécaniquement : le multiensemble des lignes non vides de l'ancien
`lib.rs` et celui des huit fichiers ne diffèrent que par `pub(crate)`, les
signatures que rustfmt a repliées en devenant plus longues, et les en-têtes de
module. Une seule correction de forme au passage : trois commentaires de doc
empilés sur `PlatformOutcome` (ceux de `run_update` et de `http_transport`,
détachés lors d'un ancien déplacement) sont revenus sur leur fonction.

**La ligne de commande reste dans `lib.rs`, exprès.** `Cli`, les cinq structures
d'arguments, `run`, la dispatch et les codes de sortie : le patch de Luther
(ePHPm, options globales `-n`/`-q`/`-v`/`--no-ansi`, `ephpm/vivace#2`) touche
exactement ces lignes et attend d'être proposé ici. Les déplacer maintenant lui
imposerait un rebase pour rien ; un `cli.rs` viendra après sa PR.

**Hors périmètre, noté :** `run_install` fait encore ~720 lignes. Le couper,
c'est toucher à la logique, pas la déplacer — un autre chantier, avec son plan.

Vérifié : `cargo fmt --check`, clippy `-D warnings` sur macOS et
`x86_64-pc-windows-gnu`, 296 tests verts, et tous les harnais de la CI contre
Composer 2.10.3 — 27 passages (dont `diff-vendor --with-autoloader` cache
froid puis chaud), tous à 0, 488 cas `OK`.

## 2026-10-15 — `-q` et `-v` écrivent ce que Composer écrit ; `-vv` et les plugins sous `-v` vont à Composer

La PR #7 de Luther (ePHPm) fait accepter `-n`/`-q`/`-v`/`--no-ansi` à toutes
les commandes, sans effet. Or la sortie d'erreur est le contrat : sous `-q`
ou `-v`, vivacity disait autre chose que Composer. Plan, méta-analyse et
revue dans `docs/plans/v0.21-verbosity.md` — deux lectures indépendantes de la
source de Composer, qui ont chacune renversé une hypothèse du plan.

**`-q` ne fait pas taire Composer.** Le plan généralisait un seul cas (pas de
composer.json, écrit au niveau normal). La plupart des erreurs sont écrites à
`IOInterface::QUIET` (`Installer.php:508, 548, 551, 730, 775, 818, 826`) et
les encadrés d'exception de Symfony aussi : la tête d'un ensemble insoluble,
l'avertissement `--no-dev`, le lock périmé restent sous `-q`. D'où trois
niveaux d'écriture (`vivacity_core::io` : `say_always!`, `say!`,
`say_verbose!`, `out!`), et chacun des ~115 sites d'écriture classé d'après
son homologue chez Composer. Les raisons propres à vivacity suivent la même
logique : QUIET quand la commande s'arrête en 3 (jamais muette), normal quand
Composer prend la main.

**Le niveau se calcule comme Symfony, pas comme clap.** `-v -v` vaut `-v`
(recherche de jetons par préfixe, pas un compte), `-q -vvv` vaut `-q`,
`SHELL_VERBOSITY` compte (avec le cast `(int)` de PHP : `"1e1"` vaut 10),
`-vvvv` vaut `-vvv`. Mesuré, puis lu dans `Application::configureIO` et
`ArgvInput::hasParameterOption`. Un niveau global au processus (atomique), posé
par `run` et remis à normal à la fin : un thread-local raterait le thread de
politique d'`install`, et ePHPm embarque `run` dans son processus.

**Les lignes `-v` ne s'écrivent pas depuis le solveur.** À l'install vivacity
ne résout pas (la transaction vient du lock) ; à l'update Composer intercale
« Nothing to modify in lock file » entre les deux résolutions, et la commande
peut encore rendre la main à Composer (Flex) après la résolution. La session
retient donc ses lignes d'après-résolution (`SolveReport::events`, avec leur
niveau) et la commande les écrit une fois sûre que la run reste ici ; sur les
chemins d'erreur, la session les écrit elle-même avant de rendre l'erreur.
Les compteurs `Analyzed N packages / N rules` sont identiques à ceux de
Composer (laravel 767/4487, symfony 952/6778, drupal 385/1652) : une preuve de
plus que le pool et le jeu de règles sont les siens.

**`-vv`, `-vvv`, et `-v` avec un plugin actif : délégation entière.** Les
niveaux de débogage de Composer (226 et 585 lignes sur laravel, requêtes et
lectures de fichiers) ne se reproduisent pas ; et sous `-v` Composer écrit une
ligne par écouteur de plugin qu'il dispatche (`EventDispatcher.php:230`), que
les plugins émulés complètent de leurs propres lignes — la revue l'a mesuré
sur bamarni (deux lignes manquantes), là où le plan ne visait que Flex et
dealerdirect. La délégation se fait en tête de `run`, avant toute lecture, par
un processus fils (jamais `exec` : ePHPm), avec l'argv d'origine moins les
options propres à vivacity. Tout processus Composer que vivacity lance reçoit
le niveau (`SHELL_VERBOSITY`, ce que Symfony exporte lui-même) : la revue a
trouvé que le fallback d'`install`, qui reconstruit sa ligne, ignorait
`-q`/`-v`. Sa remise à plat sur le modèle de la nouvelle délégation est notée,
hors périmètre.

**En passant, deux écarts au niveau normal corrigés :** `dump-autoload`
n'écrivait pas les deux lignes de Composer sur stdout (« Generating … » /
« Generated … containing N classes ») ; un composer.json absent donnait une
chaîne `Error: cannot read …` au lieu du message de `Factory::createComposer`.

Vérifié : `harness/verbosity.sh` (en CI) — 23 scénarios sous normal, `-q`,
`-v`, un sous `-vv`, un avec plugins actifs, 67 cas, stdout et stderr entiers
identiques à Composer 2.10.3, durées et chemin du projet normalisés ; suite
complète et tous les harnais existants verts : la sortie normale n'a pas bougé.

## 2026-10-16 — Le fallback rejoue la ligne de commande, en processus fils

Noté hors périmètre par la revue de la 0.21 (`-q`/`-v`), fait ici. Plan,
méta-analyse (sous-agent frais) et ce qu'elle a changé :
`docs/plans/fallback-argv.md`.

**Mesuré d'abord, avec un faux `composer` qui enregistre ses arguments.**
`install --dry-run --working-dir proj --offline` sur la fixture merge-plugin
privée de son fichier requis : Composer recevait `install --no-scripts` — une
**installation réelle** pour un dry run, sans `--offline` ni `--working-dir`.
Le fallback d'`install` reconstruisait sa ligne de neuf champs et faisait
`exec()` sous unix (chez ePHPm, l'hôte disparaissait) ; celui des commandes de
résolution rejouait bien l'argv, mais lançait Composer *dans* le projet avec
le même `--working-dir` relatif — appliqué deux fois — et perdait `--offline`.

**Décision : un seul modèle, celui de la délégation `-vv` de la 0.21.**
L'argv d'origine moins les options propres à vivacity (`--no-fallback`,
`--check-scope`, `--offline` → `COMPOSER_DISABLE_NETWORK=1`, `--run-scripts`
dont l'absence devient `--no-scripts`, `--no-autoloader` sur
`require`/`remove` — alias `r`/`rm` compris, que la délégation `-vv`
ratait), un processus fils dans le répertoire de vivacity (Composer applique
`--working-dir` lui-même, `Application::doRun`), son code de sortie rendu. Une
fonction (`hand_over`) pour les trois chemins. Écarté : garder `exec` hors
embarquement — deux comportements pour un même fallback, et le processus fils
est déjà ce que font `-vv` et `--run-scripts`.

**La phase d'installation d'un `update`/`require`/`remove` : une ligne
construite, pas l'argv.** À ce point composer.lock est écrit (sauf dry run ou
`config.lock: false`), composer.json est édité pour `require`/`remove`,
vendor/ n'est pas touché ; Composer, lui, enchaîne `doInstall` sur le lock
qu'il vient de résoudre, et `composer install` relit ce même lock. Rejouer la
commande la ferait résoudre une seconde fois, et `remove` trouverait son
paquet déjà retiré. Traduire l'argv demanderait une table par commande
(`--update-no-dev` → `--no-dev`, le `--dev` de `require` à jeter, les options
à valeur séparée, les courtes groupées `-oW`) : fragile. D'où
`after_update_line`, depuis les arguments analysés, chaque option de l'update
que l'install lit, sous le nom d'`install`. Écarté exprès : l'audit — `install
--audit` pose aussi `setErrorOnAudit` (sortie 5 sur un avis) là où `update`
audite sans échouer ; ce n'est pas le même régime, et vivacity n'audite pas
nativement non plus. Après une installation rendue, la commande n'imprime plus
son rapport d'après-update : `composer install` a imprimé les abandonnés et le
funding (`Installer::run`), seules manquent les suggestions, que seul un
update imprime. Sans cela, quitter `exec` aurait fait imprimer à `update` deux
fois ces lignes. `require` garde sa mise à jour des contraintes devinées après
l'install, comme `RequireCommand` (elle écrit composer.json).

**`--dry-run` sur un lock hors portée : reste natif.** Ce que la portée
détecte (disposition, sources, plugins à l'install) ne change pas la liste
d'opérations, lue dans le lock. Mesuré : laravel avec `preferred-install:
source`, `install --dry-run --no-plugins` — stderr et stdout identiques à
Composer 2.10.3, 112 lignes. Les fallbacks atteints avant le retour du dry run
(plugin de fusion) transmettent désormais `--dry-run`.

**Ce que la méta a renversé.** (1) La délégation Flex décidée sur le lock
résolu (`resolve_and_lock`) rendait un statut que l'appelant prenait pour le
sien : sur un succès de Composer, `update` réinstallait, `require` réécrivait
ses contraintes, `remove` restaurait ou vérifiait par-dessus le résultat de
Composer — et `require`/`remove` lui passaient un composer.json **déjà
édité**. La délégation remonte maintenant à l'appelant
(`Resolved::hand_over`), qui défait son édition en silence, puis rien ne suit.
(2) Le plan disait `--prefer-*` hors sujet ; or `RequireCommand` les lit pour
son install, et vivacity les acceptait sans les lire : une install depuis les
sources posée en dists. Ils passent maintenant à la phase d'installation
(champs internes de `UpdateArgs` ; `update` ne les accepte toujours pas), et
une préférence impossible est refusée avant la résolution, fichiers restaurés,
là où `doUpdate` appelle `getPreferredInstallOptions`. La revue indépendante a
ajouté : les valeurs de la ligne construite en un seul mot (`--working-dir=x`,
Symfony ne prendrait pas une valeur séparée qui commence par `-`), le
`require` rendu après la résolution ne touche plus au lock (pas encore écrit),
et le scénario Flex joué avec un Composer qui réussit — le cas où l'ancien code
continuait.

Vérifié : `harness/fallback-args.sh` (en CI, ubuntu et macos), 29 cas qui
comparent l'enregistrement entier (argv, répertoire, `SHELL_VERBOSITY`,
`COMPOSER_DISABLE_NETWORK`, manifeste de départ) ; 21 rouges sur le binaire
0.21.0, les 8 autres sans défaut à montrer (le dry run natif, les six
`--no-fallback`/`--check-scope`, le `-vv` d'`install`) ; tests unitaires de `forwarded_arguments` et de
`after_update_line`. Noté, hors périmètre : le fallback de portée d'`install`
est décidé après que vivacity a imprimé l'en-tête et « Package operations »,
que Composer réimprime ; sous `--run-scripts`, `pre-install-cmd` tourne avant
cette décision puis encore dans Composer.

## 2026-10-16 — `run_install` coupé en phases nommées, sans changer un octet

Suite notée le 2026-10-14 : après le découpage de `lib.rs`, `run_install`
faisait encore 737 lignes d'un seul tenant, quinze étapes de Composer et une
quarantaine de variables vivantes de bout en bout. Il en fait 45 : une suite
d'appels qui se lit contre `Installer::run`/`doInstall`, vingt et une phases
en dessous, dans l'ordre d'exécution. Plan, méta-analyse (sous-agent frais),
revue indépendante et carte des phases : `docs/plans/split-run-install.md`.

**Un découpage, pas une correction.** Un contexte `Run` (arguments, `t0`,
projet, texte de composer.json, manifeste effectif, racine fusionnée, lock,
`with_dev`, `Runner`) fixé avant la première impression ; chaque phase qui
peut arrêter la commande rend `Result<ControlFlow<i32, T>>`, et une macro
`proceed!` fait pour `ControlFlow` ce que `?` fait pour `Result`. Le runtime
tokio vit jusqu'à la fin de `run_install`, comme avant (son `drop` attend les
tâches bloquantes) ; le thread de politique est lancé et joint aux mêmes
points. Écarté : un enum de sortie maison (`ControlFlow` dit déjà « continuer
ou s'arrêter avec ») ; un module frère pour les phases (tout le diff en
déplacement pour rien) ; fusionner les trois rapports de la jointure —
leur ordre est justement un défaut noté ci-dessous, mieux visible en trois
appels.

**La preuve compare des séquences, pas des ensembles.** La méta a montré
qu'un multiensemble de littéraux laisse passer un déplacement de part et
d'autre d'un `?`, un `say!` devenu `say_always!` ou un code de sortie changé.
Comparés à l'ancien fichier : la séquence des 250 littéraux, celle des paires
(macro ou fonction appelée, littéral), celle des sorties et celle des 116
appels à effet — identiques, aux écarts de placement près (le `return Ok(0)`
du dry run devenu la valeur de `dry_run`, deux appels placés dans une autre
fonction). Deux mutations (`say!` → `say_always!`, une sortie 2 → 1) font
échouer les contrôles. Deux choix du premier jet sont revenus pour garder la
séquence intacte : la préférence d'installation reste en ligne dans
`read_manifests`, les « abandonné » de l'installation réelle restent à la fin
des opérations. Et `VIVACITY_TRACE` donne les mêmes libellés dans le même
ordre, ancien contre nouveau binaire, sur douze modes d'`install`.

**Défauts vus en route, laissés** (préexistants, à reprendre chacun avec son
plan) : l'ordre de la jointure — vivacity rapporte politique (2), plateforme
(4) puis exigences manquantes, alors que `doInstall` teste les exigences
manquantes avant le pool et le solveur ; le `COMMAND` de
bamarni/composer-bin-plugin passe après `pre-install-cmd` (avant
`Installer::run` chez Composer, visible sous `--run-scripts`) ; dealerdirect
et les extension-installers émulés avant le funding, que Composer imprime
avant `POST_INSTALL_CMD` (visible seulement sur leur échec) ; deux
commentaires qui attribuent à `doInstall` ce qui est dans `run()` ou dans un
autre ordre.

Vérifié : `cargo fmt --check`, clippy `-D warnings` sur macOS et
`x86_64-pc-windows-gnu`, 304 tests verts, et tous les harnais contre
Composer 2.10.3 sur le binaire final — 29 passages (dont `diff-vendor
--with-autoloader` cache froid puis chaud), tous à 0, 584 cas `OK`. No-op
laravel (Mac, hyperfine, binaires alternés) : `install --no-plugins` médiane
114,0 ms sur `main` contre 113,8 ms (5 passages), 47,3 contre 47,3 ms hors
ligne (30 passages) — dans le bruit.

## 2026-10-16 — `install` résout le lock comme `doInstall` : un solveur, une liste de problèmes

Défaut 1 noté par le découpage de `run_install`, fait ici. Plan, méta-analyse
(sous-agent frais), revue indépendante et résultat :
`docs/plans/install-verify.md`. L'entrée « plateforme → code 4 » plus haut
décrivait le comportement d'alors ; elle reste telle quelle.

**Mesuré d'abord** (binaire de `main`, laravel, `install --dry-run`) :
`install` ne vérifiait que les sections `platform`/`platform-dev` du lock,
avec son propre texte et le code 4, et les listes de blocage à part.
`Installer::doInstall` *résout* le lock. Dix cas divergeaient : une
extension absente requise par la racine (lock inchangé) installait là où
Composer sort en 2 ; `platform: {php: ^99}` dans le lock sous une racine
`php: ^8.3` échouait là où Composer installe (la contrainte de la racine
l'emporte) ; un `conflict` ou un `replace` de la racine visant un paquet verrouillé passait ;
un paquet verrouillé que la stabilité du lock refuse s'installait (Composer :
2 s'il est requis, 4 « without changes » sinon) ; un lock bloqué *et* en
échec de plateforme ne montrait que le paquet bloqué.

**Décision : la requête, le pool et la résolution de `doInstall`, sur le fil
qui attend déjà le réseau.** Chaque paquet verrouillé fixé, la racine figée
(sans `require`, avec `conflict`/`provide`/`replace`), la plateforme sous les
`platform-overrides` du lock, `requireName` pour les exigences de plateforme
de la racine puis pour celles du lock qu'elle ne déclare pas, la stabilité et
les préférences du lock, `Solver::solve` sous le filtre
`--ignore-platform-req(s)`, le texte de `SolverProblemsException` déjà porté
pour `update`. Ordre de Composer : exigences manquantes (4) avant tout pool,
puis la jointure. Rien de tout cela après un `update` (`$alreadySolved`).
Le fil fait le filtre de liste sur les seuls paquets verrouillés d'abord (la
requête réseau part aussi tôt qu'avant), puis la sonde, puis la version
racine que le fil principal calcule une fois pour les exigences manquantes
(git ne tourne qu'une fois) et lui envoie par un canal. Écarté : la version
racine calculée avant le lancement (git, ~9 ms, devant la requête) ou une
seconde fois sur le fil ; un pool provisoire autour d'une racine provisoire
(premier jet : la méta y a trouvé un écart, le drapeau de stabilité du nom
racine).

**Ce que la méta a changé.** La branche « without changes » que le plan
disait inatteignable l'est (paquet refusé par la stabilité du lock, présent
mais hors du pool) — deux cas de harnais, verts sans toucher au code ;
`COMPOSER_IGNORE_PLATFORM_REQ(S)`, que l'`install` de vivacity ignorait, lus
comme `BaseCommand::initialize` les lit (avertissement compris), pour la
vérification et pour `platform_check.php` ; un fil abandonné (sortie avant la
jointure) s'arrête après son attente réseau au lieu de sonder et résoudre
dans le processus d'un embarqueur ; la ligne `could not be fully loaded` d'un
dépôt dégradé est gardée par le fil et imprimée à la jointure ; sans php, les
paquets `composer-*` restent et seules les exigences php/extensions/
bibliothèques sont ignorées ; `prefer-stable` absent d'un vieux lock retombe
sur celui de la racine.

**Sans php** : pas de référence (Composer ne tourne pas sans php) ;
l'avertissement de vivacity reste, à la même condition.

**La revue indépendante** (sous-agent frais, sur le diff) n'a rien trouvé de
bloquant ; corrigé sur sa lecture : le drapeau d'abandon, lu une seule fois,
laissait le fil résoudre après une sortie au 4 — `report_missing` le lève
maintenant avant de sortir ou de rendre la main, et le fil le relit avant de
résoudre ; une erreur de `validatePackage` sort après les avertissements et la
ligne de durée, comme chez Composer ; la version racine n'est plus devinée
après un `update` ; un test du verdict « without changes » et un de la
recopie des enregistrements du filtre ; du code mort et trois commentaires.

**Deux fixtures de harnais que Composer refusait déjà**, corrigées :
`transitions.sh` fabriquait un lock `dev-main` sans drapeau de stabilité
(Composer : 4, avant la décision de portée que le cas visait), `boot.sh`
installait rector sans sa version racine `dev-main` (conflit de rector-src
avec `rector/rector <2.0`, Composer : 2).

**Défaut vu, laissé** (avec huit autres, préexistants, au résultat du plan) :
`problem::strip_output_styles` double le texte quand un `<` n'a pas de `>`
après lui (`out.push_str(rest)` au lieu de `&rest[start..]`) — un problème
`conflicts with rector/rector <2.0 (…)` sort deux fois, le premier coupé au
`<`. Touche aussi `update`. Correctif d'une ligne, hors périmètre ici.

Vérifié : `cargo fmt --check`, clippy `-D warnings` sur macOS et
`x86_64-pc-windows-gnu`, 308 tests verts (304 sur `main`, six partis avec
`check_install`, dix ajoutés), et tous les harnais contre Composer 2.10.3 sur
le binaire final — 29 passages, tous à 0 (update relancé une fois :
`packages.drupal.org`, dépôt vivant, avait rendu un corps vide), steps 269
cas dont les dix rouges sur `main`, verbosity 76. No-op laravel (Mac,
binaires alternés, 60 passages, médianes) : `install --no-plugins` 114,6
contre 113,4 ms en ligne, 48,2 contre 47,8 ms hors ligne, `--no-blocking`
hors ligne 46,7 contre 46,0 ms — inchangé.

## 2026-10-16 — Les événements d'`install` et d'`update` dans l'ordre de Composer

Défauts 2 et 3 notés par le découpage de `run_install`, faits ici, avec ce
que leur vérification a trouvé à côté. Plan, méta-analyse (sous-agent
frais), revue indépendante et résultat : `docs/plans/install-order.md`. Les
entrées plus haut qui les listent comme « laissés » restent telles quelles.

**Mesuré d'abord** (binaire de `main`, sortie standard et sortie d'erreur
fusionnées, contre Composer 2.10.3) :

- le `COMMAND` de bamarni/composer-bin-plugin : `InstallCommand::execute`
  le dispatche avant `Installer::run` (InstallCommand.php:111), donc avant
  `pre-install-cmd` ; vivacity après. Sur `update`, `UpdateCommand.php:250`
  le dispatche avant « Loading composer repositories » ; vivacity
  l'imprimait au début de sa phase d'installation, après « Writing lock
  file », et pas du tout sous `--no-install`. `require` le dispatche après
  « Running composer update » (RequireCommand.php:461), `remove` avant
  (RemoveCommand.php:239). Le refus d'une mise à jour partielle sans lock
  vient après lui (`Installer::doUpdate`, l. 508-510) ;
- `Installer::run` imprime le funding (l. 412-436) **avant** de dispatcher
  `POST_INSTALL_CMD`/`POST_UPDATE_CMD` (l. 438-442), où les écouteurs des
  plugins passent avant les scripts de composer.json
  (`EventDispatcher::getListeners` les ajoute à la fin de la priorité 0).
  codeception/c3 copiait c3.php avant le funding (et avant les scripts
  `post-autoload-dump`) chez vivacity : visible sans `--run-scripts` sur la
  fixture yii2-composer (17 paquets avec `funding`). Sa suppression, au
  retrait du plugin, venait après le dump ; Composer la fait dans
  `PluginInstaller::uninstall`, juste avant la ligne `- Removing
  codeception/c3`. Sous `--no-autoloader`, vivacity ne faisait ni l'une ni
  l'autre (elles étaient dans les plugins `post-autoload-dump`) ;
- trouvé en vérifiant `update` : les suggestions et les abandonnés
  (l. 343-373) sont imprimés avant « Generating autoload files » ; vivacity
  les imprimait après toute la phase d'installation ;
- trouvé par le nouveau cas de `scripts.sh` : « Generating autoload files »
  (l. 377-381) précède `AutoloadGenerator::dump`, donc le script
  `pre-autoload-dump` ; vivacity lançait le script d'abord.

**Fait.** La fin d'`Installer::run` d'une mise à jour est imprimée par la
phase d'installation elle-même, aux points de Composer : `InstallArgs`
porte un `UpdateReport` (suggestions et abandonnés, manifeste et état de
Flex), qui remplace `after_update` (méthode) et `handed_over` — une phase
rendue à Composer n'atteint plus ces impressions, il n'y a plus rien à
savoir après coup. `print_post_update` ne sert plus que sous
`--no-install`. `bamarni_command_event` est appelé par chaque commande au
point de la sienne (`install`, `update` dans `resolve_and_lock` après les
avertissements de plateforme, `require`, `remove`, `dump-autoload`).
`post_install` imprime le funding, puis les émulations (dealerdirect,
extension-installers, copie c3, Flex), puis `post-install-cmd`. La
suppression de c3 se fait juste avant sa ligne `- Removing`.
`COMPOSER_FUND` est lu comme `intval` (`0.5`, `1e400` cachent la ligne,
mesuré ; `inf` n'est pas numérique pour PHP).

**Écarté.** Imprimer le `COMMAND` d'`update` juste après `prepare_update`
(proposé par la méta) : les avertissements `COMPOSER_IGNORE_PLATFORM_REQ(S)`,
que vivacity imprime après, viennent avant `COMMAND` chez Composer
(`BaseCommand::initialize`). Un genre d'erreur dédié au refus « partial
update without a lock » : le code 3 est la constante de Composer et le
seul refus de ce code dans `prepare_update`.

**Prémisse corrigée.** Le défaut 3 disait dealerdirect et les
extension-installers muets hors échec. Faux : phpstan/extension-installer
imprime « Extensions installed » à chaque `POST_*_CMD` (`Plugin.php:198`),
rector et dealerdirect quand ils écrivent. vivacity n'imprime aucune de ces
lignes — mesuré sur un projet phpstan/extension-installer 1.4.3 +
phpstan-strict-rules 2.0.4 : deux lignes de stdout chez Composer, aucune
chez vivacity. Laissé (ce sont des lignes à émuler, pas un ordre), avec les
autres défauts vus : ordre des émulations entre elles (Composer :
`PackageSorter`, un plugin installé par la commande en dernier), écouteurs
rejoués par `composer run-script` sous `--run-scripts` (mesuré : c3
« already up-to-date » en plus), `pre-install-cmd` lancé avant la décision
de portée, écouteurs `POST_UPDATE_CMD` absents sous `update --no-install`
(c3.php et les fichiers générés non écrits), `forward-command` sur `update`,
avertissements de plateforme de `require`/`remove` après leurs premières
lignes, manifeste non fusionné pour le `COMMAND` des commandes de
résolution, doublon `[bamarni-bin]` sur une résolution rendue à Composer
après coup.

Vérifié : rouges sur `main` d'abord (`scripts.sh` 2, `bin-plugin.sh` 6,
`yii2-composer.sh` 7 cas), verts ensuite ; `cargo fmt --check`, clippy
`-D warnings` sur macOS et `x86_64-pc-windows-gnu`, 309 tests verts (308 sur
`main`, plus `funding_shown`), et les 30 passages de harnais contre
Composer 2.10.3 sur le binaire final, tous à 0 (659 cas `OK`). Le job
Windows n'a pas tourné ici.

## 2026-10-16 — L'avertissement de version racine, tenu jusqu'à la première ligne

Écart connu de `docs/plans/v0.21-verbosity.md`, que tous les harnais
neutralisaient en posant `COMPOSER_ROOT_VERSION`. Plan, matrice mesurée,
méta-analyse (sous-agent frais), revue indépendante et résultat :
`docs/plans/root-version-warning.md`.

**Mesuré d'abord** (Composer 2.10.3, projet sans réseau hors de tout dépôt
git) : `RootPackageLoader::load` (`RootPackageLoader.php:91-117`) écrit
« Composer could not detect the root package (…) version, defaulting to
'1.0.0' » par `IOInterface::warning` — `writeError` au niveau `NORMAL`
(`BaseIO.php:250-297`), stderr, muet sous `-q` — quand il n'y a ni
`version`, ni `COMPOSER_ROOT_VERSION` (vraie au sens PHP : `0` et la chaîne
vide ne comptent pas), ni version devinée par git, sauf pour une racine sans
`name` ou de type `project` (comparaison exacte). `BaseCommand::initialize`
crée le Composer (`:240`) avant les avertissements
`COMPOSER_IGNORE_PLATFORM_REQ(S)` (`:280`, `:289`) et avant tout ce que la
commande imprime (refus de `--no-install`, « Not enough arguments ») : la
ligne vient en tête. `require` et `remove` recréent le Composer après « has
been updated » (`RequireCommand.php:409-410`, `RemoveCommand.php:216-222`) :
une seconde ligne, avant « Running composer update », pas sous
`--no-update`. `dump-autoload` et `run-script` : une.

**Fait.** Un tampon d'une ligne « retenue » dans `vivacity_core::io`
(`hold`, `release_held`, `drop_held`) : chaque macro d'écriture l'écrit
d'abord, quelle que soit la première ligne de la commande ; une délégation
à Composer la jette avant de lancer le processus (première instruction de
`hand_over`, et avant les raisons quand Composer prendra la main), puisque
Composer écrit la sienne ; `composer run-script` la reçoit écrite avant sa
sortie. Chaque commande la retient en premier
(`hold_root_version_warning`), `require`/`remove` la réécrivent au point de
la seconde création. Les trois `eprintln!` à niveau calculé passent par un
nouveau `say_at!`. La décision est le port des conditions de `load`
(`vivacity_resolver::root::undetected_version_warning`) avec un prédicat git
dédié (`root_version::version_guessable`).

**Écarté.** Imprimer la ligne directement en tête de chaque commande : chaque
délégation précoce (plugin inconnu, plugin de fusion, `-vv`) l'aurait fait
sortir deux fois, et avant les raisons du fallback. Décider la portée
d'`install` avant l'en-tête pour supprimer le double des délégations
tardives : change l'ordre des décisions, suite à part. Appeler
`guess_version` pour décider : la méta a mesuré ≈ 60 ms sur une branche de
fonctionnalité à sept branches parentes candidates (un `git rev-list` par
candidate) ; le prédicat s'arrête à `git branch` dès qu'une ligne courante
donne une version. Partager le résultat avec la détection de la version
plus loin (cache de processus, ou version passée à `install`, à la session
de résolution et à `dump`) : le coût restant est d'un appel git sur le seul
cas d'une bibliothèque nommée sans version ni variable, mesuré à ≈ 5 ms sur
un no-op (aucun scénario du banc, tous `project`, n'est touché) ; un cache
qu'un embarqueur garderait d'une run à l'autre se tromperait si la branche
change entre deux runs.

**Écarts laissés** (non-objectifs du plan) : doublons des délégations
tardives et de la phase d'installation rendue à `composer install` ; une
ligne par `composer run-script` lancé sous `--run-scripts` ; trois lignes
chez Composer pour un manifeste refusé après la ligne (auto-exigence, nom de
lien) ; le composer.json global sans composer.json de projet ; ce que
Composer peut imprimer avant la ligne (root/sudo, Xdebug, script qui
masquerait une commande, `disable-tls`, « dubious ownership ») ; le schéma
`LAX_SCHEMA` non porté (`type: "Project"`) ; hg/fossil/svn ; un
`composer` trouvé mais impossible à lancer perd la ligne (jetée avant le
lancement). Une `version: null` ne donne pas de ligne (revue) : Composer la
refuse par son schéma, vivacity dans `load_with_version`.

## 2026-10-16 — Les recettes de symfony/flex appliquées nativement : une simulation, puis un plan

**Contexte.** Depuis la tranche C, `update` avec install sur un projet Flex
n'était natif que si aucune recette ne s'appliquait ; chaque `update` qui
faisait entrer un paquet à recette (ou à classe de bundle) partait chez
Composer. Mesure sur les recettes que les fixtures emploient (83, index
fixé du 2026-09-22) : `copy-from-recipe` 74, `bundles` 50 (+ 11 recettes
auto-générées), `conflict` 24, `add-lines` 15, `gitignore` 14, `env` 12,
`docker-compose` 4, `dockerfile` 3, `dotenv` 3, `composer-scripts` 3,
`container` 1, les trois autres 0 (`docs/plans/flex-configurators.md`).

**Fait.** `Flex::install` avec recettes est reproduit pour `bundles` (et la
recette auto-générée), `copy-from-recipe`, `env`, `dotenv`, `gitignore`,
`add-lines`, `composer-scripts`, et les deux configurateurs Docker quand
`extra.symfony.docker` est faux (`crates/vivacity/src/flex_recipes.rs`,
`flex_install::flex_install_decision`, `flex::RecipeIndex`). Les opérations
dans l'ordre de `recordOperations` (le port de `Transaction`, présent = les
noms de `symfony.lock`) ; l'index complet par endpoint (`_links`, `branch`,
`is_contrib`) ; les `recipe-conflicts` ÉVALUÉS (`findPackage` sur
`getLockedRepository(true)`, alias compris) — ignorés jusqu'ici parce qu'ils
ne pouvaient que provoquer un repli inutile, ce qui n'est plus vrai dès
qu'on applique ; le fichier de recette récupéré comme `Downloader::get`
(URL relative à l'endpoint, cache Flex partagé, écrit seulement avec
`last-modified` comme `parseJson`) ; le `conflict` du manifeste évalué.

**Pourquoi une simulation puis un plan.** La décision doit précéder
l'écriture du lock (contrat), mais Flex agit à `POST_UPDATE_CMD`, après
l'install. Tous les configurateurs tournent donc d'abord sur un système de
fichiers virtuel (disque + écritures du plan), qui produit les lignes et
l'état final de chaque fichier avec l'état qu'il avait ; à l'événement, le
plan vérifie ces états puis écrit en place (inode et mode gardés, comme
`file_put_contents`). Le système virtuel note chaque chemin regardé : un
chemin dans une zone que l'install écrit avant l'événement (vendor-dir,
bin-dir, paquets hors vendor/, `c3.php`) rend la main à la décision — la
vérification à l'application n'est plus qu'une assertion interne.
Recalculer à l'événement aurait ouvert une seconde porte de repli après
l'écriture du lock.

**`config/bundles.php` reconnu, pas évalué.** Flex le `require` ; vivacity
l'analyse strictement dans la forme de `buildContents` et exige que le rendu
redonne les mêmes octets, sinon repli. Un fichier tenu par Flex passe, un
fichier édité à la main (commentaire, `\Foo::class`) part chez Composer.

**`updateComposerLock` reproduit.** `composer-scripts` réécrit composer.json
sans condition ; si ses octets changent, Flex relit composer.lock en
tableaux PHP et le réécrit : `scripts` n'entre pas dans le content-hash,
l'effet visible est le réencodage (`"stability-flags": {}` → `[]`),
reproduit par `phpjson` (cas `framework`, octet pour octet).

**Laissé en repli, nommé.** `container`, `makefile`, `composer-commands`,
`copy-from-package` ; Docker activé ou non décidé (Flex pose une question,
vivacity est non interactif) ; une recette contrib sans `allow-contrib`
(`SYMFONY_ALLOW_CONTRIB` prioritaire) ; `%generate(secret)%` qui serait
réellement écrit ; un conflit de recette qui correspond ; `root-dir` autre
que `.` ; un endpoint `api.github.com` (en-tête `Accept` de Flex non porté
— Sylius) ou `file://` (Flex ne le lit pas) ; `--offline` avec une recette
à télécharger ; une désinstallation dont l'index liste le paquet ;
`SYMFONY_LOCKFILE` ; une version de `symfony.lock` que `normalize` refuse ;
un message `add-lines` de niveau normal que le formateur de la console
pourrait transformer. `require` (alias et versions devinées,
`PackageResolver`) et `remove` avec install (`Unconfiguring`) inchangés.

**Corrigé en chemin.** `strip_version` ne faisait qu'une substitution là
où `preg_replace('/^dev-|^v|\.x-dev$|-dev$/', '', …)` est globale
(`v1.x-dev` → `1`) ; la comparaison de versions remplissait de zéros
(`5.3` = `5.3.0`) là où `version_compare` dit `<`. Sans effet visible
avant (une comparaison ratée rendait la main), décisif dès qu'on applique.

**Délégation tardive.** Si la phase d'installation rend encore la main à
`composer install` après l'écriture du lock (plugin de fusion, portée,
émulation refusée), Composer exécute Flex à `POST_INSTALL_CMD` — sans
recette. Le plan est alors appliqué après ce `composer install` réussi
(revue indépendante, défaut 1) : le projet finit comme après l'`update` de
Composer ; la stderr d'une délégation tardive diffère déjà (écart
documenté).

**Corrigé aussi (cas `no-plugins`).** Sous `--no-plugins`, un
`symfony-pack` posé par Flex (sans `install-path`) est installé par
Composer comme une bibliothèque : `loadInstalledPlugins` précède
`purgePackages`, et sans Flex l'installateur est `LibraryInstaller`.
vivacity le tenait pour installé (stderr seule fausse).

**Dépendance.** `base64` ajouté au crate `vivacity` (contenu de fichier de
recette encodé) : déjà dans le graphe par `vivacity-core`, rien de nouveau
à compiler.

**Oracle.** `fixtures/flex/configurators/` : les index réels de
`symfony/recipes` et `recipes-contrib` (commits du 2026-09-22) réduits aux
paquets des cas, et leurs fichiers de recette tels quels ; la liste de
symfony/framework-bundle s'arrête à 7.4 (la recette 8.1 copie un fichier
qu'on ne reproduit pas ici) — PROVENANCE. `harness/flex-recipes.sh`
affirme que l'oracle applique bien des recettes (« Symfony operations »,
fichier attendu créé ou modifié) avant de comparer.

## 2026-10-16 — Les lignes d'avant la commande, retenues ensemble ; `dump-autoload` lit les variables de plateforme

Contexte : `docs/plans/install-verify.md`, défaut 2 ; `docs/plans/install-defects.md`,
point 3. `BaseCommand::initialize` (BaseCommand.php:276-291) lit
`COMPOSER_IGNORE_PLATFORM_REQS` (`(bool)` PHP) puis, l'option toujours
fausse, `COMPOSER_IGNORE_PLATFORM_REQ` (toute chaîne non vide) pour toute
commande qui a les deux options — `DumpAutoloadCommand` les a (l. 40-41) et
passe le filtre à `AutoloadGenerator` (l. 114). L'avertissement de chacune
sort après la création du `Composer` et avant `execute`.

Décision : une fonction (`platform_requirement_env`, manifest.rs) pour les
cinq commandes, appelée là où `initialize` tourne — après la ligne de
version racine — et qui **retient** ses avertissements. `vivacity_core::io`
ne retenait qu'une ligne ; il en retient une file, écrite dans l'ordre à la
première ligne propre de vivacity, jetée par une délégation précoce. La
dépréciation `--no-suggest` de `require` (première ligne de
`RequireCommand::execute`) y entre aussi : sur une délégation précoce,
Composer l'écrit lui-même. `resolve_and_lock` (update, require, remove)
lit toujours les variables, sans plus rien écrire.

Écarté : imprimer ces lignes directement — c'est ce qui faisait le doublon
(`g-merge-env`, `g-require-no-suggest` de `fallback-args.sh`, qui comptaient
une ligne de vivacity avant la délégation ; zéro maintenant). Les
délégations tardives gardent les leurs (portée d'`install` décidée après
l'en-tête : hors de ce lot).

Après la méta-analyse : sans composer.json, l'avertissement sort aussi chez
Composer avant « Composer could not find a composer.json file »
(`initialize` avant `execute`) — `dump-autoload` et `update` le lisaient
après leur refus ; ramené avant (trois cas `@nojson`, deux rouges).

Mesuré rouge sur db349c1 : `dump-autoload` avec `COMPOSER_IGNORE_PLATFORM_REQS=1`
ou `COMPOSER_IGNORE_PLATFORM_REQ=php` (avertissement absent,
`platform_check.php` différent), `require`/`remove` (avertissement après
« Running composer update »), et les trois assertions de `fallback-args.sh`.

## 2026-10-16 — `COMPOSER_NO_DEV` vrai au sens de PHP, pour `install` et `dump-autoload`

Contexte : `docs/plans/install-verify.md`, défaut 3 ; `install-defects.md`,
point 4. `BaseCommand::initialize` (l. 255-274) pose `--no-dev` (ou
`--update-no-dev`) quand `(bool) Platform::getEnv('COMPOSER_NO_DEV')` : toute
chaîne sauf `""` et `"0"`. `update`, `require` et `remove` le lisaient déjà
ainsi ; `install` n'acceptait que `1`, `dump-autoload` ne lisait rien. Une
fonction (`env_no_dev`, manifest.rs) pour les deux. Rouge sur db349c1 :
`install --dry-run` avec `yes` (109 paquets contre 76) et `dump-autoload`
avec `true` (`autoload_static.php` différent).

## 2026-10-16 — Un lock sans `packages-dev` refusé après l'en-tête

Contexte : `install-verify.md`, défaut 7 ; `install-defects.md`, point 5.
`Installer::doInstall` (Installer.php:753-757) écrit l'en-tête puis appelle
`Locker::getLockedRepository($this->devMode)`, qui lève une
`RuntimeException` quand `isset($lockData['packages-dev'])` est faux
(Locker.php:157-165) : code 1, message même sous `-q`, avant « Verifying… ».
vivacity le découvrait en construisant la transaction locale, après
« Verifying… », sous `Error: composer.lock: …`. Le test est dans `headline`,
juste après l'en-tête ; le message est imprimé sans l'encadré de Symfony
Console, comme les refus de `RootPackageLoader` (même déviation, déjà
assumée). Pas après un `update` : son lock a toujours la clé. Rouge sur
db349c1 (`root-manifest.sh`, `lock-no-packages-dev`, `lock-null-packages-dev`).

## 2026-10-16 — `require` sans contrainte lit aussi les dépôts `path`

Contexte : relecture tardive de `root-version-warning.md` ; `install-defects.md`,
point 8. `PackageDiscoveryTrait::getRepos` (l. 44-55) compose la plateforme
et `RepositoryFactory::defaultReposWithDefaultManager` — tous les dépôts du
manifeste — que `VersionSelector::findBestCandidate` interroge par
`RepositorySet::findPackages`. `UpdateSession::find_packages_for_require` ne
lisait que les dépôts `composer` : sur la fixture path-repos, `require
acme/beta` sortait en 1 (« Could not find a matching version ») sur `main`
comme sur db349c1 — défaut réel, pas une régression. Les dépôts `path` sont
lus comme des `ArrayRepository` (`array_repository_load_packages`, déjà
celui du `PoolBuilder`), dans l'ordre du jeu.

## 2026-10-16 — Les `ignoredTypes` d'`Installer` dans les pools d'`update` et d'`install`

Contexte : `install-verify.md`, défaut 8 ; `install-defects.md`, point 2.
`Installer::$ignoredTypes = ['php-ext', 'php-ext-zend']` (Installer.php:196)
est passé à `createPool` par `doUpdate` (l. 534) et `doInstall` (l. 808) ;
`PoolBuilder::loadPackagesMarkedForLoading` (resolver/PoolBuilder.php:461)
saute un paquet chargé de ces types (pas les paquets fixés ou verrouillés).
`RepositorySet::createPool` les prend vides par défaut : `create_pool` reçoit
la liste en paramètre, ses deux appelants (la session d'`update`, la
vérification d'`install`) passent `INSTALLER_IGNORED_TYPES`.
`createPoolWithAllPackages` (extraction dev) n'est pas concerné.

Oracle : aucune fixture n'avait de paquet de ces types ; le registre
synthétique `solver-problems` gagne `acme/phpext` (1.0.0 `library`, 2.0.0
`php-ext`), régénéré par `fixtures/make-solver-problems.py` — les deux locks
de la fixture sortent identiques. Rouge sur db349c1 : `@require:acme/phpext=*`
verrouillait 2.0.0 (Composer 1.0.0) ; `^2.0` réussissait (Composer : code 2,
« found acme/phpext[2.0.0] but these were not loaded »).

## 2026-10-16 — Les annotations de GitHub Actions, là où vivacity a le texte de Composer

Contexte : `install-verify.md`, défaut 4 ; `install-defects.md`, point 6.
`GithubActionError::emit` (src/Composer/Console/GithubActionError.php) écrit
sur stdout, niveau normal, `::error ::` et le message échappé
(`%`, `\r`, `\n`) quand `GITHUB_ACTIONS` est vrai et
`COMPOSER_TESTS_ARE_RUNNING` faux (`Platform::getEnv`, vérité PHP).
Appelants : `Installer::doUpdate` (l. 554), `extractDevPackages` (l. 735),
`doInstall` (l. 829) — `$err."\n".$prettyProblem`, sans l'avertissement
`--no-dev` ni les deux lignes d'aide de l'extraction —, et
`Application::doRun` pour le message de toute exception (l. 482 ; l. 307
pour une `ParsingException` du manifeste, avec `file=`/`line=`).

Décision : `vivacity_core::io::github_action_error`, appelée aux trois
points du solveur (`SessionError::annotation` porte le texte exact pour
`update` et l'extraction dev) et pour les exceptions dont vivacity
reproduit le message à l'identique (refus de `RootPackageLoader::load`, lock
sans `packages-dev`). Écarté : annoter les `Error: …` de vivacity — leur
texte n'est pas celui de Composer, l'annotation serait fausse elle aussi ;
le JSON invalide (`ParsingException`, `file=`/`line=`) a son propre texte
chez vivacity.

Effet sur les harnais : ceux qui comparent stdout côté vivacity sans poser
`COMPOSER_TESTS_ARE_RUNNING` des deux côtés n'ont pas de cas d'erreur
(steps.sh jette stdout, verbosity.sh pose la variable pour les deux) ;
`root-manifest.sh` compare désormais stdout, vérifié sous
`GITHUB_ACTIONS=true` sans la variable (le régime du runner : tous les cas,
43 à la fin de la branche).
Rouge sur db349c1 : les trois cas `GITHUB_ACTIONS=true` de verbosity.sh
(normal et `-v`) et `gha-self-require`, `gha-lock-no-packages-dev`.

## 2026-10-16 — `--offline` comme `COMPOSER_DISABLE_NETWORK`, et le mode dégradé selon la requête

Contexte : `install-verify.md`, défaut 9 ; `install-defects.md`, point 7.
Le fallback traduit `--offline` en `COMPOSER_DISABLE_NETWORK=1`. Sous cette
variable, `HttpDownloader::startJob` (l. 328-338) répond 304 à une requête
qui porte `If-Modified-Since` et rejette les autres
(`TransportException('Network disabled, request canceled: '.Url::sanitize($url), 499)`) ;
les `file://` passent par `RemoteFilesystem`, sans ce contrôle.

Le transport hors ligne de vivacity (`network_disabled`, update.rs) fait de
même. Ce que Composer fait ensuite de l'erreur dépend de la requête
(`ComposerRepository`) : `fetchFile` (packages.json sans `last-modified`,
inclusions) prend toute copie en cache, avec l'avertissement ;
`fetchFileIfLastModified` l'avertissement et la copie ; `asyncFetchFile`
(fichiers `p2`, résumé des listes) **écrit l'avertissement dans tous les
cas**, rend la copie si la requête était conditionnelle, lit le 499 comme un
404 (`['packages' => []]`, d'où « Filter summary URL … returned 404 » pour
le résumé), et relance toute autre erreur. `settle` ne connaissait qu'un cas
(copie avec `last-modified`) : il prend la saveur de la requête
(`asynchronous`), et reconnaît le refus par son texte
(`repository::NETWORK_DISABLED`, seul le transport hors ligne l'écrit).
Effet en ligne aussi : un packages.json en cache sans `last-modified` sert
maintenant en mode dégradé, comme chez Composer, au lieu d'échouer.
L'avertissement perd le préfixe `Warning: ` (Composer écrit la balise
`<warning>`, pas le mot) et son URL est assainie.

Mesuré (Packagist réel, laravel, `install --dry-run`) : cache chaud, Composer
muet, vivacity « Warning: https://repo.packagist.org could not be fully
loaded (offline: …) » ; cache vide, « - offline: cannot fetch … » au lieu de
« - Network disabled, request canceled: … ». Harnais : section 4 de
`transport-exit.sh` (dépôt servi par `php -S` avec un routeur qui pose ou
non `Last-Modified`), rouge sur db349c1 dans les trois états du cache.

Laissé : `COMPOSER_DISABLE_NETWORK` posé par l'utilisateur ne met pas
vivacity hors ligne (seul `--offline` le fait) ; hors ligne, un dist absent
du cache échoue chez vivacity par « missing cache for … in offline mode »,
chez Composer par « Network disabled, request canceled: <url> » ; le POST
de l'API des avis suit la règle (pas de 304 possible).

## 2026-10-16 — `config.lock: false` et `install` sans lock : la résolution de Composer, rien d'écrit

Contexte : `install-verify.md`, défaut 5 ; `install-defects.md`, point 1 et
la méta-analyse (deux bloquants). `Factory::createComposer`
(Factory.php:423-429) : `config.lock` faux et un composer.lock présent → la
ligne « ./composer.lock is present but ignored… », et un `Locker` sur
`/dev/null`, donc `isLocked()` faux partout. `Installer::run` (l. 277-285) :
une install sans lock dit qu'elle met à jour et passe en `doUpdate($localRepo,
true)` ; `writeLock` faux, `setLockData(..., false)` garde le lock en mémoire
et `doInstall` l'installe ; pas d'en-tête « Installing dependencies from lock
file » ni de « Lock file operations » (l. 609, 753). `RequireCommand.php:492`,
`RemoveCommand.php:295` : liste d'autorisation seulement si `isLocked()` ;
`RemoveCommand.php:99` : `--unused` exige un lock.

Décisions :
- la ligne de `Factory` est retenue après celle de version racine
  (`hold_ignored_lock_warning`), réécrite par `require`/`remove` avec elle ;
- `UpdateOptions::ignore_lock` : la session n'ouvre pas composer.lock ;
  `require` et `remove` décident la mise à jour partielle sur `isLocked()`
  (lock activé) ; le refus de `require` sans `--no-install` tombe ;
- `Resolved::virtual_lock` : l'installation qui suit une résolution non
  écrite lit le lock résolu (comme un `--dry-run`), pour `update`, `remove`
  et `require` ;
- `install` sans lock utilisable : `install_without_lock` construit des
  `UpdateArgs` avec les seules options d'`install` (les variables
  `COMPOSER_PREFER_*`, `COMPOSER_WITH_*`, `COMPOSER_MINIMAL_CHANGES` que lit
  `run_update` ne s'appliquent pas à `InstallCommand`) et entre par
  `resolution_fallback` puis `run_update_resolved` ; la ligne d'`Installer::run`
  sort après l'événement `COMMAND`, avant « Loading composer repositories ».
  `--check-scope` (ni réseau ni écriture promis) et `--run-scripts` (les
  scripts d'`update`, que l'`update` de vivacity ne joue pas) : 3 avec la
  raison, ou Composer. Flex n'arme pas son rappel `symfony/thanks` (commande
  `install`, Flex.php:177). Un lock illisible reste l'erreur de Composer
  (`ParsingException`).

Lock sans `packages-dev` (méta, majeur, suite du point 5) : `install
--no-dev` passe `doInstall` puis lève à la liste des abandonnés
(Installer.php:356, `getLockedRepository(true)`), paquets posés, sans
autoloader, code 1 ; `update`/`require`/`remove` lèvent dans `doUpdate`
(l. 496), `require` rétablissant d'abord ses fichiers. Une erreur typée
`ComposerException` (lib.rs) porte les messages de Composer reproduits mot
pour mot : imprimés seuls (pas de préfixe `Error: `) et annotés sous GitHub
Actions. La première session de `require` (choix de version) n'ouvre plus le
lock, que Composer ne lit qu'à la mise à jour.

Mesuré rouge sur db349c1 (`verbosity.sh` : onze cas dont trois vraies
installations comparées sur composer.lock et installed.json ; `steps.sh` :
`remove --unused` 1 contre 0, `update a/b` 3 contre 0, `install @badlock`
0 contre 1 ; `root-manifest.sh` : quatre cas `run_lock`).

Laissé : une délégation tardive de la phase d'installation rend un
`composer install` qui, lock désactivé, résout à nouveau. (`dump-autoload`
lisait composer.lock pour sa disposition : corrigé après la revue, entrée
suivante.)

## 2026-10-16 — hg, fossil et svn : rendre la main là où Composer devinerait

Contexte : relecture tardive de `root-version-warning.md` ; `install-defects.md`,
point 9 et la méta-analyse (majeur : la décision doit vivre à côté de la
détection de version, pas de l'avertissement). `VersionGuesser::guessVersion`
(VersionGuesser.php:84-104) essaie après git `hg branch` (l. 239),
`fossil branch list` puis `fossil tag list` (l. 377, 384), `svn info --xml`
(l. 405), dans le répertoire du projet ; un code 0 donne une version. Binaire
absent : `ProcessExecutor` rend un code non nul, la version reste nulle.
Mesuré : laravel rendu bibliothèque hors git, un `.hg`, un faux `hg` qui
répond `default` — Composer exécute `hg branch`, `hg branches`, `hg bookmarks`
et se tait ; vivacity avertissait et prenait `1.0.0+no-version-set`. Sans
`hg` : les deux avertissent.

Décision : porter hg, fossil et svn serait porter trois pilotes ; la
version vient de Composer. `root_version::foreign_vcs_version` : une copie
de travail cherchée en remontant (`.hg`, `_FOSSIL_`/`.fslckout`, `.svn`, là
où chaque outil la cherche), puis la commande de Composer, sortie jetée ; un
code 0 → la raison. `root_version_from_foreign_vcs` (racine sans `version`
ni `COMPOSER_ROOT_VERSION`, git sans version) est appelée par `run`, avant
`dispatch`, pour les cinq commandes, tout type de racine (la version atteint
installed.php) et tout niveau (l'avertissement n'est pas la question) : la
délégation précoce de `-vv` (`delegate_for_verbosity`, qui sort aussi en 3
sous `--check-scope`). Les paquets `path` sans version (`path_repo.rs`,
même devinette) : `RepoErrorKind::Unsupported` → `SessionErrorKind::Unsupported`
→ `Resolved::hand_over` (avant toute ligne d'`update`, édition défaite pour
`require`/`remove`) ou `delegate_resolution` pour la session de choix de
version de `require`. Coût hors de ces cas : quelques `stat` par ancêtre,
aucun processus.

Rouge sur db349c1 : `fallback-args.sh` section h (`h-hg-library`,
`h-hg-no-fallback`, `h-hg-project`, `h-hg-path`) ; témoins verts des deux
côtés : `h-hg-fails` (faux `hg` en échec), `h-hg-env`
(`COMPOSER_ROOT_VERSION`), `h-hg-path-fails` (`dev-main`).

## 2026-10-16 — Les réponses de git gardées le temps d'une exécution

Contexte : relecture tardive de `root-version-warning.md` (« git tourne deux
fois ») ; `install-defects.md`, point 10. Mesuré par la méta-analyse puis
ici (une enveloppe de git qui compte) : un `install` sans opération d'une
bibliothèque sans version lance `git branch -a --no-color --no-abbrev -v`
trois fois (avertissement, version pour la vérification, installed.php) et,
sur une branche de fonctionnalité, chaque `git rev-list main..feature-x`
deux fois.

Décision : `root_version::git` garde ses réponses par (répertoire,
arguments) — pas la version calculée, qui dépend du manifeste
(`non-feature-branches`) —, toutes en lecture seule. Le cache n'existe
qu'entre `remember_git_answers` et `forget_git_answers`, que `vivacity::run`
appelle autour de chaque exécution (un garde, levé même sur une panique) :
un programme qui embarque vivacity voit le dépôt changer entre deux
exécutions, et hors d'une exécution (tests qui committent entre deux
devinettes) git tourne toujours. Deux exécutions simultanées dans un même
processus partagent le cache (comme le niveau de verbosité) : l'une peut le
vider ou l'éteindre pour l'autre, qui relance alors git — plus lent, jamais
faux (clés par répertoire et arguments).

Mesuré (hyperfine `-N`, 40 passages, `install --no-plugins --offline
--no-blocking` sans opération, laravel rendu bibliothèque) : branche de
fonctionnalité 57,3 → 47,0 ms ; `main` 50,9 → 45,1 ms. Un appel de chaque
commande au lieu de trois et deux.

## 2026-10-16 — Les restes du filtre de liste : dépôts du plugin de fusion, avertissements perdus, fil attendu

Contexte : `install-verify.md`, défauts 6 et 10 ; `install-defects.md`,
point 11.

- **6a.** `ExtraPackage::prependRepositories` (plugins/composer-merge-plugin/ExtraPackage.php:217-245)
  met en tête du `RepositoryManager` chaque dépôt (avec `type`) des fichiers
  fusionnés, URL `path` préfixée par le répertoire du fichier
  (`fixRelativePaths`), le dernier ajouté en premier ; le filtre de liste de
  `doInstall` les interroge. `Merged::repository_definitions` les garde dans
  cet ordre, `InstallVerifyInput::prepended_repositories` les met devant ceux
  de la configuration. La résolution (`update`…) rend toujours la main pour
  ces dépôts (inchangé). Cas : `merge-plugin.sh install-repos-filter` (un
  dépôt `composer` local dont la liste malware vise le psr/log verrouillé :
  Composer 2, deux problèmes ; vivacity 0 sur db349c1).
- **10.** Les avertissements gardés d'un dépôt dégradé
  (`ComposerRepository::hold_output`) partaient avec l'erreur du filtre :
  `SessionError::preface` les porte, la jointure les écrit avant l'erreur.
  Reproduit sans réseau par un seul dépôt (section 4 de `transport-exit.sh`,
  `ignore-unreachable` faux, cache sans `Last-Modified`) : l'avertissement
  du résumé refusé, puis l'exception « Filter summary URL … returned 404 ».
- **6b.** Le repli du plugin de fusion décidé après le lancement du fil
  (`report_missing`, vendor vierge) attend désormais le fil
  (`abandon_and_wait`) avant de lancer Composer : jamais deux processus sur
  le cache de dépôt. Coût (méta) : l'attente réseau du fil, sur un chemin
  qui rend la main de toute façon.

## 2026-10-16 — Ce que la revue indépendante a changé (branche `install-defects`)

La revue (sous-agent frais, sur le diff de `main`) n'a trouvé aucun
bloquant ; ses deux majeurs se reproduisaient contre Composer.

- **`dump-autoload` sous `config.lock: false`** construisait sa disposition
  depuis composer.lock — périmé (autoloader faux, code 0) ou absent
  (`Error: failed to read`). `AutoloadGenerator::dump` lit le dépôt local :
  sans lock à lire (désactivé ou absent), installed.json devient la source
  (`local_repository_as_lock`, paquets répartis par `dev-package-names`).
  Deux cas `verbosity.sh` (lock périmé par `@lockjq:`, lock absent),
  rouges avant.
- **`install` lisait et analysait composer.lock deux fois** (le test
  « lock utilisable ? » puis la lecture) : une seule lecture, une seule
  analyse ; un lock présent mais illisible reste une erreur.
- **Mineurs corrigés** : la ligne de `Factory` sur le lock ignoré ne sort
  plus devant un refus de `RootPackageLoader` (cas `root-manifest.sh`) ;
  l'exception du `Locker` dans `update` vient après l'événement `COMMAND`
  (cas `bin-plugin.sh no-dev-info`) ; les dépôts du plugin de fusion ne
  sont interrogés que si le plugin est installé (`prependRepositories`
  suit l'activation ; cas `install-repos-filter-bare`) ; le cache de git
  est levé par un garde, panique comprise ; `--check-scope` se nomme
  lui-même dans le refus de la délégation hg/fossil/svn, et le remède sans
  composer y est `COMPOSER_ROOT_VERSION` ; `ignore_lock` n'écrase plus la
  valeur de l'appelant ; les `stat` avant la lecture de composer.json.
- **Comptes et renvois** de CHANGELOG/DECISIONS/HANDOVER corrigés ;
  commentaire de `steps.sh` rectifié (la stderr des refus n'est pas
  comparée).
- **Noté, laissé** : `install`/`update --no-suggest` refusés par clap
  (Composer avertit) — antérieur.


## 2026-10-21 — Les lignes des installeurs de plugins, dans l'ordre d'activation de Composer

Contexte : non-objectifs de `docs/plans/install-order.md` (lignes stdout de
phpstan/rector/dealerdirect, ordre fixe des émulations) ; plan
`docs/plans/plugin-lines.md` (matrice mesurée, références, méta-analyse et
revue).

- **Les lignes.** phpstan/extension-installer 1.4.3 (`Plugin::process`,
  Plugin.php:198-214) : « Extensions installed » à chaque `POST_*_CMD`,
  puis `installed` / `not supported` / `ignored` quand GeneratedConfig.php
  change (md5 de l'ancien fichier contre le nouveau contenu) ;
  rector/extension-installer 0.11.2 (`PluginInstaller::install`,
  PluginInstaller.php:95-104) : « rector/rector-installer: Extensions
  installed » et `installed`, seulement quand le fichier change ;
  dealerdirect v1.2.1 (`saveInstalledPaths`, Plugin.php:263-315) : « set
  to <chemins> » ou « delete » quand il sauve ; package-versions-deprecated
  1.11.99.5 (`writeVersionClassToFile`, Installer.php:200-231) : deux lignes
  à chaque dump. Toutes `$io->write`, niveau normal, stdout ; `-v` reste
  rendu à Composer (lignes `> event: Classe->méthode`).
- **L'ordre.** Porté, pas supposé : les écouteurs de priorité 0 sont appelés
  dans l'ordre d'activation des plugins (EventDispatcher.php:537-596) —
  installed.json tel qu'écrit, trié par `PackageSorter::sortPackages` avec
  les poids de `PluginManager::loadRepository` (-10000 pour les plugins qui
  changent les chemins ; un alias de branche compte deux fois dans les
  usages), chargé avant la purge (Factory.php:448 puis :457), le dépôt
  global ensuite ; puis la transaction, un lot par plugin
  (InstallationManager.php:300-323) : installé → en fin, mis à jour →
  désactivé puis réactivé en fin, désinstallé → retiré
  (PluginInstaller.php:80-120). `crates/vivacity/src/plugin_order.rs`.
  L'ordre fixe de vivacity coïncidait sur un retour en dev ou une mise à
  jour de plugin, pas sur un vendor vierge (transaction), ni sur un no-op
  où c3 précède phpstan (noms), ni quand dealerdirect est requis par des
  standards (poids). `PackageSorter` gagne `$weights`
  (`sort_packages_weighted`). Alternative écartée : prouver que l'ordre
  fixe suffit — le harnais montre neuf cas où il ne suffit pas.
- **Versions.** Les écouteurs émulés sont désormais liés aux versions lues
  (`scope::EMULATED_VERSIONS`) : phpstan 1.4.0–1.4.3, rector 0.11.0–0.11.2
  et 1.0.0, dealerdirect v1.0.0, v1.1.1, v1.1.2, v1.2.0, v1.2.1,
  package-versions 1.11.99.4–1.11.99.5 ; toute autre (dont une branche dev)
  va à Composer, nommée, avant toute écriture — et, sur
  `update`/`require`/`remove` avec installation, dès la résolution quand le
  plugin installé est hors liste (sinon la phase d'installation rendait
  `composer install`, donc `POST_INSTALL_CMD`). Raisons lues dans les
  sources : phpstan 1.3.x n'écrit pas `PHPSTAN_VERSION_CONSTRAINT`, rector
  0.10.x a un autre gabarit, dealerdirect 0.x cherche les rulesets dans tout
  le projet et v1.1.0 demande à `InstalledVersions` où est phpcs,
  package-versions < 1.11.99.4 génère un autre Versions.php. Porté :
  `$alternativePath` de v1.0.0–v1.2.0 (un chemin enregistré relatif au
  projet est gardé ; v1.2.1 le retire) ; `installed_paths` vide lu comme
  aucun chemin. Corpus : chamilo (v0.7.2, `unavailable`) passe au repli ;
  les deux projets en package-versions 1.11.99.1 (mautic,
  mautic-recommended-project) étaient déjà en repli. Corpus non rejoué.
- **Alias.** phpstan et rector gardent pour chaque nom la version de la
  *dernière* entrée du dépôt local. Mesuré : un alias en ligne du lock est
  réajouté après le paquet chargé à chaque exécution — vendor vierge, la
  version du paquet ; no-op suivant, celle de l'alias et une ligne
  `installed` (le fichier différait déjà chez vivacity) ; un paquet `path`
  est posé avant son alias (l'alias gagne, mesuré par la revue) ; l'alias
  de branche par défaut d'une archive ne change rien (fixture rector).
  Décision : un paquet listé par ces plugins et aliasé (en ligne, ou alias
  de branche autre que celui d'une archive par défaut) va à Composer,
  nommé. Suite possible : porter l'ordre du dépôt local (synchrone/
  asynchrone par type de dist) au lieu de rendre la main.
- **`remove` d'un plugin.** Trouvé par le harnais : `RemoveCommand` retire
  les entrées `allow-plugins` des paquets retirés *après* que son
  `Composer` a chargé les plugins (RemoveCommand.php:216-222 puis
  :242-252), et le `PluginManager` garde les règles lues ; vivacity
  relisait le manifeste édité — `remove codeception/c3` ne supprimait pas
  c3.php, un plugin retiré mais encore requis n'écrivait plus ses lignes.
  `UpdateArgs.allow_plugins_at_load` rend à la phase d'installation
  l'`allow-plugins` du projet tel qu'il était, ordre compris (la première
  règle qui correspond gagne : remettre seulement les entrées, en fin,
  faisait gagner un `codeception/*: false` placé après — vu par la revue). Écart antérieur à ce chantier.
- **drupal/core-project-message** (liste bénigne) imprime à
  `POST_INSTALL_CMD` son message configuré ou `include-keys` : un `install`
  où il écrirait va à Composer, nommé (GPL-2.0-or-later : pas d'émulation,
  NOTICE.md). Aucun projet du corpus touché (tous ont aussi le scaffold).
- **Références.** `docs/reference/{PluginInstaller,InstallationManager,EventDispatcher}.php`
  ajoutés, surveillés par `drift-reference.sh`.
- **Harnais** `harness/plugin-lines.sh` (46 cas, Packagist en ligne,
  COMPOSER_HOME isolé) : 38 rouges sur `main`, 15 sur un binaire
  intermédiaire (lignes, ordre fixe) dont neuf d'ordre, tous verts ensuite.
- **Noté, laissé** : `update --no-install` (Composer joue `POST_UPDATE_CMD`,
  phpstan y écrit sa ligne et les plugins leurs fichiers ; vivacity rien —
  non-objectif d'install-order, désormais l'écart le plus visible de
  phpstan) ; dealerdirect quand phpcs ne tourne pas sous le PHP de la
  machine (« Failed to set … » chez Composer, vivacity écrit) ; v1.1.1
  (`InstalledVersions::isInstalled`, diffère seulement si phpcs est
  remplacé ou fourni) ; la ligne « not writable » de package-versions ; les
  plugins émulés installés globalement (ordre calculé, émulation absente) ;
  la ligne « Dependency … is also a root requirement » de `remove`, après
  « Updating dependencies » chez vivacity, avant chez Composer (autre
  famille, vue par la revue).

## 2026-10-22 — Les écouteurs `POST_UPDATE_CMD` sous `update --no-install`

Contexte : non-objectif « `update --no-install` » de
`docs/plans/plugin-lines.md` ; plan `docs/plans/no-install-listeners.md`
(matrice mesurée contre Composer 2.10.3, références, méta-analyse et revue).

- **Le constat.** `Installer::run` avec `setInstall(false)` saute le dump
  (Installer.php:305-307) et rend 0 après le lock (`doUpdate`, :684-689),
  mais imprime le funding (:412-436) et dispatche `POST_UPDATE_CMD`
  (:438-441) ; `--no-scripts` ne coupe que les scripts de composer.json
  (EventDispatcher.php:585), `--dry-run` coupe tout (:292-298). `update`,
  `require` et `remove` avec `--no-install` sont concernés.
- **Ce que lisent les écouteurs : le dépôt local.** Mesuré, et lu dans les
  sources (phpstan `Plugin.php:124`, rector `PluginInstaller.php:60-105`,
  dealerdirect `Plugin.php:503-534`, c3 `Installer.php:114-126`) :
  installed.json tel qu'il est, purgé des répertoires disparus
  (Factory.php:457), dev compris quel que soit `--no-dev`, sans les alias en
  ligne du lock, un alias de branche chargé avant son paquet
  (ArrayRepository.php:215-231 : la version écrite reste celle du paquet).
  Les plugins sont ceux qu'installed.json charge (Factory.php:448) : vendor
  vierge ou plugin ajouté par la commande, rien ; plugin retiré par la
  commande, il joue encore ; c3 copie le c3.php de la version **installée**.
  Émulé (`crates/vivacity/src/no_install.rs`) : les émulations existantes
  de dealerdirect, phpstan, rector, c3 et la sortie de Flex, dans l'ordre
  d'activation d'installed.json (`plugin_order`, sans transaction), sur ce
  dépôt et sa disposition (`Layout::resolve`), avec les `allow-plugins` du
  chargement pour `remove`. Et `yiisoft/yii2-composer::activate`, qui
  recrée `extensions.php` au chargement — sans installation et en dry run
  (vu par la méta-analyse).
- **Rendu à Composer, nommé, avant toute écriture.** Une version non lue de
  dealerdirect/phpstan/rector installée (`EMULATED_VERSIONS`, désormais
  contrôlée aussi sans installation ; package-versions n'est pas concerné,
  `POST_AUTOLOAD_DUMP` ne part pas ; c3 n'a pas de garde : `Installer.php`
  identique de 2.6.1 à 2.9.0, 2.6.0 ne diffère que sur le chemin Composer 1,
  les versions antérieures exigent `composer-plugin-api ^1.0`) ; un de ces
  écouteurs installé globalement ; un plugin actif ni émulé ni bénin
  (composer-patches, installers-extender : `RESOLUTION_INERT` pour le lock,
  pas pour le reste du run) ; une disposition non calculable ; et
  **php-http/discovery** dès qu'il surveille une abstraction PSR ou dans
  une version non lue. Pour ce dernier, mesuré : son `postUpdate` édite
  composer.json et relance un `Installer` cloné, sans converger sous
  `--no-install` (25 écritures du lock en 25 minutes, coupé) ; la première
  version du plan le gardait natif quand le dépôt local égale le lock — la
  méta-analyse a montré (mesuré) que c'est faux pour une racine `project`
  dont l'implémentation est en dev. Alternative écartée : porter
  `getMissingRequires` (table `PROVIDE_RULES`, contraintes de version,
  règles dev/projet, épinglage par `extra.discovery` depuis 1.18.0) pour ne
  rendre que lorsqu'il agirait — plus de code qu'il n'en faut pour un
  plugin qu'on rend ; l'épinglage n'est pas lu (une abstraction épinglée
  compte comme surveillée). Décidé en tête de `resolve_and_lock`, sur le
  manifeste que la commande résout, avant toute ligne.
- **`activate` de yii2-composer** au chargement (Factory.php:448) : joué
  dans `resolve_and_lock` où le run reste celui de vivacity — erreurs
  imprimées après chargement, ensemble insoluble, succès —, jamais avant un
  passage de main (Composer l'y fait lui-même) ; et au dry run
  d'`install`. Trouvé par la méta-analyse, placé par la revue.
- **Seuls les plugins chargés jouent** : `listeners_in_order` ajoute en fin
  les noms émulés absents ; sans filtre, un plugin autorisé mais non
  installé faisait calculer la disposition du dépôt local et pouvait faire
  échouer la commande après l'écriture du lock (revue, reproduit).
- **Harnais** : `harness/plugin-lines.sh` gagne une section
  `--no-install` (les nombres de cas, rouges et verts sont dans le
  Résultat du plan).
- **Environnement de mesure** : Linux x86_64, PHP 8.5.12 (conda-forge,
  `gd` compilé à part), Composer 2.10.3 ; les zipballs d'`api.github.com`
  refusés par le proxy de la session, le cache de fichiers de Composer
  amorcé par `git archive` du commit verrouillé (les deux outils lisent la
  même archive) ; `*.wordpress.org` refusé : la fixture wordpress n'a pas
  pu être construite.
- **Noté, laissé** : un plugin installé qu'`allow-plugins` ne mentionne pas
  (Composer lève `PluginBlockedException`, sort en 1 ; vivacity continue —
  toutes les commandes de résolution, écart antérieur) ; php-http/discovery
  sur `update` **avec** installation (même `getMissingRequires`, liste
  « inerte » qualifiée sur `install` seulement ; non mesuré ici) ;
  dealerdirect v1.1.x (`InstalledVersions::isInstalled` lit l'installed.php
  non purgé) ; la purge suit l'`install-path` enregistré, Composer le
  `getInstallPath` courant.

## 2026-10-22 — `-v` avec des plugins actifs : les lignes des écouteurs, plus la délégation par défaut

Contexte : décision 4 et bloquant de la revue de `docs/plans/v0.21-verbosity.md`
(tout plugin autorisé actif rendait `-v` à Composer) ; plan
`docs/plans/verbose-plugins.md` (matrice mesurée, versions lues, méta-analyse,
revue).

- **Ce que `-v` ajoute pour un plugin.** Une ligne par écouteur appelé,
  `> <événement>: <classe>-><méthode>` sur stderr au niveau VERBOSE
  (`EventDispatcher::doDispatch`, EventDispatcher.php:230), dans l'ordre de
  priorité puis d'ajout (`getListeners`, :583-596) — donc l'ordre
  d'activation des plugins déjà porté (`plugin_order`) ; plus les écritures
  VERBOSE propres à quelques plugins. Mesuré sur Composer 2.10.3 pour chaque
  plugin des listes (`install` vierge, no-op, `--no-dev`, `update` avec et
  sans installation, `require`/`remove`, `dump-autoload`, `--dry-run`).
- **Décision : reproduire ces lignes, et ne rendre la main que pour ce qui
  ne l'est pas.** `crates/vivacity/src/verbose_listeners.rs` porte la table
  (plugin, versions lues, classe, écouteurs) et la raison de chaque
  délégation restante ; `fallback::active_plugin` rend cette raison.
  Alternative écartée : élargir seulement la liste des plugins « muets » —
  aucun plugin à écouteur n'est muet sous `-v`, la ligne `>` est le cas
  général, pas l'exception.
- **Le nom de classe.** `PluginManager::registerPackage`
  (PluginManager.php:286-307) recharge une classe déjà déclarée sous
  `<classe>_composer_tmp<N>`, `N` compteur du processus : `require` et
  `remove` recréent leur `Composer` (toutes les classes des plugins chargés,
  muets compris, dans l'ordre de `loadRepository`), et une mise à jour de
  plugin recharge les siennes. Porté comme un ensemble de classes déclarées
  (minuscules), remis à zéro à chaque `run` (méta-analyse, point 9).
- **Versions.** La table lie chaque plugin aux tags lus un par un (classe,
  `getSubscribedEvents`, écritures VERBOSE hors tests), **branche par
  branche** (`>=7.4.0 <=7.4.21 || …`, revue, point 3 : une borne unique
  acceptait les correctifs à venir d'une branche maintenue) ; une version
  publiée après, une branche dev, ou 1.15–1.16 de php-http/discovery (une
  table d'écouteurs différente) rendent `-v` à Composer, nommées. Contrôlées :
  version installée, version du lock, version résolue (`resolved_issue`,
  avant l'écriture du lock). Coût accepté : une sortie de symfony/runtime
  renvoie `-v` des projets Symfony… mais Flex les y renvoie déjà. Mise à
  jour : relire les nouveaux tags (même balayage), étendre la borne.
- **Restent rendus, raison nommée** : symfony/flex (écouteurs de pool, de
  paquet, configurateurs verbeux), dealerdirect (sortie de `phpcs
  --config-set`), wikimedia/composer-merge-plugin (`init`,
  `pre-install-cmd`, `post-package-install`, journal VERBOSE),
  yiisoft/yii2-composer et symfony/thanks (`post-package-update`, dispatché
  entre les opérations), metasyntactical/composer-plugin-license-check, tout
  autre plugin, un plugin global (Composer le juge par l'`allow-plugins`
  global), `--run-scripts`, et phpstan / rector / c3 sous `--no-install`
  (Composer joue leur effet, vivacity non : la ligne `>` seule aurait
  aggravé l'écart — méta-analyse, point 2). *(Levé le 2026-10-27 : les
  écouteurs sont joués sous `--no-install` depuis la 0.24.0, et la ligne
  de chacun précède désormais ce qu'il écrit — entrée du 2026-10-27.)*
- **Délégation après résolution, sans doublon** (revue, point 1). Sous
  `-v`, un `update`/`require`/`remove` qui installe garde ses lignes
  (`vivacity_core::io::defer`, les deux flux dans l'ordre) jusqu'au lock
  résolu : relâchées si la run reste à vivacity, jetées si un plugin qu'elle
  activerait la rend à Composer — qui les écrit lui-même. Sinon ces lignes
  (« has been updated », « Updating dependencies »…) sortaient deux fois,
  alors que `main` rendait ces runs avant toute ligne. Coût : rien ne
  s'affiche pendant la résolution sous `-v`. L'`install` sans lock (résolu
  comme un `update`) garde aussi ses lignes depuis le 2026-10-27.
- **`--dry-run`** ne coupe que les événements de script : `command` part
  quand même (bamarni) et `require`/`remove` rechargent (méta-analyse,
  point 1).
- **Harnais.** `harness/verbosity.sh`, seconde partie `online` : projets à
  plugins actifs (Packagist, lock écrit sous `--no-plugins`, versions
  exactes ; `fixtures/projects/laravel-pest` figé), stdout et stderr entiers,
  sortie fusionnée (ordre), projet entier, et vivacity ne doit PAS déléguer
  (sauf `@delegates:`) ; la stderr de Composer doit contenir au moins une
  ligne `>` (sauf `@silent`).
- **Noté, laissé** : `COMPOSER_DEBUG_EVENTS` (« Dispatching … » au niveau
  normal) ; l'auto-installation de php-http/discovery ; les effets des
  écouteurs sous `update --no-install` au niveau normal (écart de
  plugin-lines.md, inchangé).

## 2026-10-26 — Les harnais en ligne relancés une fois sur une panne réseau, pas tolérés

Deux CI rouges de suite sur le job macOS (22 et 23 octobre, `plugin-lines.sh`
puis `verbosity.sh`) : le runner a perdu sa résolution DNS (« Could not
resolve host: repo.packagist.org ») pendant les parties qui résolvent contre
le vrai Packagist. Composer et vivacity retombent alors sur leurs caches avec
des messages différents (curl d'un côté, le client HTTP de l'autre), ou la
référence échoue tout court : le harnais est rouge sur le réseau, pas sur
vivacity.

Écartés : classer chaque cas « réseau indisponible » comme le corpus (des
dizaines de points d'échec à toucher dans quatre harnais, et un cas sauté
en silence finit par masquer une régression) ; figer ces projets sur un
instantané hors ligne (c'est ce que font les fixtures ; les parties en ligne
existent justement pour exercer des versions de plugins que l'instantané n'a
pas, et le refaire coûte un chantier).

Retenu : `harness/net-retry.sh`, qui enveloppe chaque harnais dans `ci.yml`
et `windows.yml`. Un harnais qui passe tourne une fois. Un harnais en échec
dont la sortie porte une erreur réseau (curl 6/7/28, résolveur, erreur
d'envoi du client de vivacity) est relancé une fois après une minute, et
c'est le second passage qui décide. Une vraie régression échoue deux fois et
reste rouge ; le coût n'est payé que sur panne. Vérifié sur quatre harnais
simulés sous le bash 3.2 de macOS : réseau puis vert (deux passages, 0),
différence sans réseau (un passage, 1), réseau deux fois (deux passages, 1),
vert (un passage, 0).

## 2026-10-27 — `-v` sous `--no-install` : la ligne de chaque écouteur avant ce qu'il écrit

Contexte : `docs/plans/verbose-plugins.md` rendait `-v` à Composer pour
phpstan, rector et c3 sous `--no-install`, faute d'y jouer leurs écouteurs ;
`docs/plans/no-install-listeners.md` les joue depuis la 0.24.0, mais
`print_post_update` écrivait d'abord toutes les lignes `>`, puis les
écouteurs — l'ordre de Composer est l'inverse de cette séparation
(`EventDispatcher::doDispatch` écrit la ligne juste avant l'appel,
EventDispatcher.php:230), d'où la délégation maintenue.

- **Décision** : `no_install::post_update_listeners` parcourt les plugins
  chargés dans l'ordre d'activation et, pour chacun, écrit sa ligne `-v`
  puis joue son émulation ; `UNPLAYED_WITHOUT_INSTALL` et le paramètre
  `no_install` de la décision `-v` disparaissent.
- **Harnais** : `harness/verbosity.sh`, six cas `-v` (phpstan `update
  --no-install` avec et sans GeneratedConfig.php, `require --no-install`,
  rector, c3 avec c3.php perdu, la combinaison) — rouges sur `main`
  (f3f5524 : délégation, ou lignes dans le mauvais ordre), verts ensuite.

## 2026-10-27 — Les modes des répertoires d'un zip, jusqu'à vendor/

Vu en reconstruisant des archives WordPress pour le harnais : un zip d'hôte
unix dont les répertoires portent 0775 s'installe en 0775 chez Composer,
en 0755 chez vivacity. Deux pertes :

- **la racine** : `ArchiveDownloader::install` (ArchiveDownloader.php:172-182)
  renomme le répertoire de tête extrait par `unzip` en `$path` quand celui-ci
  est neuf — le mode suit ; vivacity sautait l'entrée de ce répertoire (le
  retrait de la racine la rend vide) et la racine de l'entrée de store
  prenait le défaut. Elle prend maintenant la règle de `unzip_mode` de cette
  entrée ;
- **le clone** : `link_or_copy_tree` créait chaque répertoire avec le défaut
  (0777 & ~umask) ; il recopie le mode de la source, une fois le répertoire
  rempli (0500 interdirait d'y écrire), seulement s'il diffère du défaut lu
  une fois par clone. Sous macOS `clonefile` les gardait déjà.

Le segment du store passe à `v3` (précédent : `v2`, DECISIONS du
2026-09-30) : une entrée extraite avant n'est plus servie. Coût mesuré sur
une réinstallation chaude de Laravel (1 306 répertoires), quinze passes
alternées : médiane 346 ms contre 365 ms pour 0.24.0, dans le bruit.
Harnais : `artifact-repo.sh` gagne une racine 0775 et un `docs/` 0700 —
rouge sur 0.24.0 (vendor/ différent, 755 sur les deux), vert ensuite ; test
unitaire `directory_modes_follow_the_source`. Les zipballs de GitHub (hôte
FAT, `git archive`) ne portent aucun mode de répertoire : sans effet sur
elles.

## 2026-10-27 — `allow-plugins` jugé au chargement des plugins, et php-http/discovery porté

Deux écarts silencieux (`docs/plans/plugin-allow-discovery.md`). Composer
juge chaque plugin installé au chargement (`PluginManager::loadRepository`),
avant toute commande : non couvert et requis, `PluginBlockedException`,
code 1 ; plus requis, une ligne d'avertissement. vivacity installait,
résolvait, écrivait, code 0. Et php-http/discovery, après un `update` qui
installe, peut réécrire composer.json et relancer un update : vivacity ne
faisait rien.

- **Refus reproduits, pas rendus.** Le message de Composer sans l'encadré
  de Symfony, code 1, l'annotation GitHub Actions — le précédent des refus
  de `RootPackageLoader` (`harness/root-manifest.sh`). Écarté : rendre à
  Composer (exact, mais un `--no-fallback` sortirait en 3 là où Composer
  sort en 1, pour un cas entièrement déterminé). La méta-analyse a montré
  qu'une création de `Composer` échouée recommence (`initialize` avalé, la
  commande, `hintCommonErrors` plugins coupés) : les lignes retenues et
  les avertissements sont réimprimés comme chez Composer
  (`io::held_lines`), mesuré par `gone-then-blocked-install` et
  `lockfalse-blocked-install`.
- **`remove` juge après l'édition.** Le premier chargement est avalé
  (`tryComposer`) ; le refus vient du `requireComposer` qui suit « has been
  updated », composer.json laissé édité. Retirer le plugin bloqué lui-même
  réussit donc — le plan initial l'aurait refusé (bloquant de la
  méta-analyse).
- **Rendus, nommés, avant toute écriture** : ce que vivacity ne voit pas
  à l'avance sans le rejouer — un plugin non couvert que l'installation
  poserait (Composer s'arrête au milieu de la transaction, paquets
  précédents posés), un `require` qui rend requis un tel plugin (refus au
  second chargement puis composer.json rétabli), un plugin dont Composer
  vérifie l'API avant de le juger.
- **php-http/discovery : le prédicat porté plutôt qu'une règle large.**
  `getMissingRequires` (identique de 1.19.0 à 1.20.0) et le `array_filter`
  qui le suit, particularités PHP comprises (`$devPackages += $names` sans
  effet, `isset` d'un `false`, `array_diff_key` avant l'aplatissement),
  dans `crates/vivacity/src/discovery.rs` (il lui faut le parseur de
  contraintes du résolveur). Évalué sur le lock résolu avec installation,
  sur installed.json sans : il remplace « rendu dès qu'il surveille une
  abstraction » de `--no-install`, gardé pour 1.15.0–1.18.1 ; toute autre
  version reste rendue. Écarté : émuler son action (édition de
  composer.json puis update imbriqué, réseau compris) — c'est la commande
  entière de Composer.
- **Laissés** : le mode interactif (jamais chez vivacity) ; l'écart
  préexistant de `scope::active_plugins`, qui lit le dépôt global sans les
  conditions du Composer global ; un plugin autorisé dont l'API ne
  correspond pas (Composer l'ignore avec un avertissement).

## 2026-10-28 — Installeurs personnalisés : les chemins portés, les effets reproduits ou rendus

Contexte : 11 entrées du corpus (`docs/corpus/2026-10-21.md`) partaient à
Composer à cause d'un plugin qui enregistre un installeur ; plan
`docs/plans/custom-installers.md` (classement mesuré, matrice, méta-analyse
et revue).

- **Un placeur par installeur, gardé par version lue.** getkirby
  1.2.1, neos 2.1.1–2.1.3, davidbarratt 1.1.0, johnpbloch 2.0.0, thelia 1.6,
  contao-components 1.4.0–1.4.1, oomphinc 2.0.0–2.0.1 (tags aux sources
  identiques, `git diff --quiet`), craftcms 1.6.0 inerte sans paquet ni
  racine `craft-plugin`, et la table de composer/installers v1.12.0
  (`tools/gen-installers-table.php`, qui lit aussi le `getLocations()` sans
  argument de 1.x). Une autre version, des deux côtés de la transaction
  (installed.json et lock), part à Composer, nommée, avant toute écriture —
  aussi pour les commandes qui ne posent rien (`update --no-install`,
  `remove --no-update`), où l'activation du plugin s'exécute quand même
  (`config.component-dir` de contao-components, `installer-paths` scalaire
  de thelia, oomphinc sur composer/installers 1.x).
- **Plusieurs installeurs sur un type : le chemin seul ne suffit pas.**
  Écarté (méta-analyse, bloquant) : « chemins égaux, peu importe lequel ».
  Le gagnant de `getInstaller` suit l'ordre d'activation, et ce qui
  accompagne le chemin diffère : kirby retire le `vendor/` du paquet,
  composer/installers écrit « Deleting … » au retrait, johnpbloch refuse un
  répertoire partagé. Retenu : chemins différents → rendu ; chemins égaux →
  natif seulement quand ces effets ne dépendent pas du gagnant (pas de
  kirby sur un paquet installé ou mis à jour, pas de johnpbloch, pas de
  retrait par un installeur qui écrit). thelia-project reste natif : seuls
  ses gabarits sont pris par deux installeurs, posés au même endroit, jamais
  retirés.
- **Les effets reproduits.** kirby : le `vendor/` du paquet supprimé après
  la pose (`postInstall`) ; composer/installers et oomphinc : la ligne
  « Deleting <chemin> - deleted » sur stdout (absolu en 2.x, relatif tel que
  le gabarit le rend en 1.x et chez oomphinc — vivacity ne l'imprimait pas,
  2.x compris) ; composer/installers 1.x sur PHP ≥ 8.4 : les deux lignes de
  dépréciation de l'`ErrorHandler`, en tête quand le plugin est chargé
  depuis installed.json, juste après sa ligne d'installation sinon, le
  chemin sous `realpath(vendor-dir)` ; un paquet à `target-dir` posé
  ailleurs par un installeur que la transaction installe : le répertoire
  vide que le téléchargement laisse au chemin de LibraryInstaller
  (l'installeur est capturé au téléchargement, `cleanup` ne retire que ce
  chemin et `vendor/<vendeur>`).
- **Rendus, nommés** : un paquet que ses propres métadonnées déplacent à
  la mise à jour (`installer-name` changé : `updateCode` renomme le chemin
  initial vers le nouveau ; un chemin que la configuration déplace sur
  un paquet inchangé est, lui, une installation neuve au nouveau chemin,
  l'ancien répertoire laissé, émulée — `custom-dirs.sh` `unmapped`, qu'une
  première version trop large de la règle rendait ; le même déplacement
  sur un paquet mis à jour, sans cas de harnais, est rendu aussi) ; une `target-dir` que l'installeur n'ajoute pas
  au chemin (l'autoloader de Composer en retranche la longueur quand même —
  il la retranche désormais une seule fois chez vivacity, comme `substr`) ;
  un paquet `path` à binaires posé à un chemin relatif (`realpath` du
  binaire) ; un binaire dans le `vendor/` que kirby retire ; composer/
  installers 1.x à côté d'un plugin qui pourrait émettre la première
  dépréciation, sous `-v`, avec un refus, ou sous Windows ; un installeur
  global (COMPOSER_HOME, enregistré après ceux du projet).
- **`isInstalled` par le chemin recalculé.** Sous `--no-plugins` (ou un
  plugin qui n'est plus actif), Composer cherche chaque paquet au chemin de
  LibraryInstaller et réinstalle ceux qui n'y sont pas ; la transaction de
  vivacity lisait l'`install-path` d'installed.json. Elle suit maintenant
  le chemin que la disposition calcule.
- **Cibles sous vendor/** : admises (les modules de thelia-project sous
  `vendor/thelia/modules/`) hors de `vendor/composer`, du `bin-dir` et de
  `vendor/autoload.php`, et sans recouvrir un autre paquet.
- **Licences.** johnpbloch (GPL-2.0-or-later), thelia et contao-components
  (LGPL-3.0-or-later) : comportement décrit et réimplémenté, aucune source
  copiée ni vendue. Les sources MIT et Apache-2.0 lues sont vendues sous
  `docs/reference/` avec leur licence (NOTICE.md). En vérifiant, la ligne
  NOTICE de roots/wordpress-core-installer disait MIT : son `LICENSE.md` et
  son composer.json disent GPL-2.0-or-later. Corrigé dans NOTICE, puis
  tranché le jour même (entrée suivante). johnpbloch réutilise la fonction de chemin de vivacity
  (`wp_core_dir`), mesurée identique à son comportement.
- **Hors de portée** : simplesamlphp (modules imbriqués dans un autre
  paquet, liens durs nommés d'après le chemin absolu), contao/manager-plugin,
  silverstripe, mautic, `vendor/craftcms/plugins.php` ; omeka (paquet `path`
  GPL-3.0 absent de la fixture : aucun gain mesurable).

## 2026-10-28 — roots/wordpress-core-installer : copie GPL retirée, `wp_core_dir` récrite depuis le comportement

**Constat.** L'émulation de roots/wordpress-core-installer (v0.17,
2026-09-22) avait été écrite en lisant sa source, vendorée sous
`docs/reference/plugins/wordpress-core-installer/` et déclarée MIT dans
NOTICE. Son `LICENSE.md` et son composer.json disent GPL-2.0-or-later : le
même cas que l'émulation de `drupal/core-composer-scaffold`, retirée en
0.6.0, contraire à la règle « rien n'est porté d'un logiciel GPL ».

**Décision** (de l'auteur, 2026-10-28) : garder l'émulation, mais sans rien
de la source. La copie vendorée est retirée ; `layout::wp_core_dir` est
récrite depuis le comportement observé seul — les règles que les cas de
`harness/wp-core.sh` et `harness/custom-installers.sh` mesurent contre
Composer (réglage de la racine en chaîne ou en carte par nom, puis celui du
paquet, puis `wordpress` ; valeurs « vides » à la PHP ignorées ; `true`,
`.` et le répertoire vendor refusés avec le message de Composer) — avec
une autre structure et des commentaires qui décrivent ce comportement, pas
le code du plugin. johnpbloch/wordpress-core-installer, GPL lui aussi,
réutilise cette fonction comme avant. Les sorties ne changent pas : mêmes
tests unitaires, mêmes harnais.

**Alternative écartée** : retirer l'émulation (bedrock, wordplate et
themosis repartiraient à Composer) — rien ne l'impose une fois la source
hors du projet, le comportement d'un installeur n'étant pas protégé.

## 2026-10-28 — Les paquets sans dist servis par git, installés comme Composer les installe

Le corpus du 2026-10-21 rendait six entrées pour « no usable dist ». Le
classement (`docs/plans/git-sources.md`) : trois paquets git seuls
(ampache, kovah-linkace, friendica) et trois paquets `path` dont la
fixture du corpus n'a pas le répertoire (mautic, drupal, omeka-s) — le
message les confondait, il les nomme désormais.

- **Le binaire `git`, les commandes de `GitDownloader`, rien d'autre.**
  Un `.git` ne peut être celui de Composer que par construction : même
  binaire, mêmes commandes, même ordre, mêmes arguments (le chemin du
  cache avec sa barre finale, `--` devant chaque URL), même environnement
  (`Git::cleanEnv`), même `process-timeout`. Écartées : une bibliothèque
  git en Rust (une dépendance, et un autre `.git` : packs, config,
  reflogs) ; une extraction de l'arbre sans `.git` (ce n'est pas ce que
  laisse Composer).
- **Ce que « identique à l'octet » veut dire pour un `.git`.** Mesuré :
  deux installs de Composer à une seconde d'écart ne diffèrent que par
  `index` (cache stat) et les horodatages des reflogs ; la méta-analyse a
  montré un troisième écart, les packs refaits par `clone --dissociate`
  d'un miroir à objets libres (`repack` en plusieurs threads : trois noms
  de pack pour trois clones). Décision : tout `.git` sous le vendor
  comparé à l'octet sauf ces trois familles, comparées par ce qu'elles
  signifient (`ls-files --stage` et `status`, reflogs horodatage masqué,
  ensemble des objets) ; `objects/info/` à l'octet hors noms de packs (un
  `alternates` vers le cache serait une vraie différence). Les modes
  aussi. Le banc donne à chaque côté son cache VCS au même chemin (le
  reflog du clone le porte).
- **Tout ce qui peut échouer passe avant la première écriture.** La
  synchronisation du miroir (phase de téléchargement), puis le clone et
  `updateToCommit` dans un répertoire de travail sous vendor/ ; le
  placement n'est qu'un `rename`, vérifié sur un seul système de
  fichiers. Un échec rend la main à Composer, qui produit sa propre
  erreur, sans rien laisser (pas même un vendor/ créé par le run) — la
  revue a trouvé que vendor/ était créé trop tôt. Écarté : reproduire les
  erreurs de git et la boucle d'authentification/protocoles de
  `Git::runCommand`.
- **Natif seulement là où le banc compare.** `https://` sans
  identifiants, chemin local (`isLocalPath`, `realpath`, `file://`),
  Linux/macOS, cache non configuré, réseau permis, installation neuve
  sous le vendor ou paquet laissé tel quel. Rendus, nommés, avant toute
  écriture : autres VCS et schémas, `github-protocols`/`github-domains`,
  `cache-dir`/`cache-vcs-dir`/`COMPOSER_CACHE_VCS_DIR`,
  `COMPOSER_DISABLE_NETWORK`/`--offline`, `single_use_clone`, `mirrors`,
  références et branches dérivées hors `[A-Za-z0-9._/-]` (une version
  `dev---detach` deviendrait une option de `git checkout`), métapaquet
  et `symfony-pack` git seuls, chemin d'installation hors du vendor, et
  la mise à jour ou la suppression d'un paquet installé depuis la source
  (`cleanChanges` interroge le clone et peut poser une question) — vue
  par `--check-scope`.
- **`--prefer-source` / `preferred-install: source` restent rendus.** Ce
  n'est pas le même travail : passer en source des paquets qui ont une
  dist, c'est cloner Packagist depuis GitHub (authentification, `mirrors`,
  repli source → dist sur échec avec ses avertissements). Seul ajustement :
  un paquet sans dist ne fait plus refuser la préférence (il vient de la
  source quoi qu'elle dise).
- **Laissés** : un dépassement de `process-timeout` ne tue que `git`, pas
  `git-remote-https` (tuer le groupe demanderait `unsafe`, ou un groupe de
  processus à part qui couperait le Ctrl-C) ; un `vendor/.vivacity-git-*`
  après un arrêt brutal ; Windows.

Mesures, machine : macOS 26 (Apple Silicon, 14 cœurs), git 2.54.0,
Composer 2.10.3. Corpus rejoué sur les six entrées : ampache et
kovah-linkace natives en 0 diff (dev et `--no-dev`), friendica ne garde
que oomphinc/composer-installers-extender, mautic, drupal et omeka-s leurs
plugins et leurs répertoires `path` absents. `harness/git-sources.sh` :
25 cas, rouges sur 0.24.0 (code 3, ou motif de repli absent), verts
ensuite ; `compare-selftest.sh` : 8 cas `.git`, dont 3 rouges avec le
`compare.sh` de 0.24.0 (qui ignorait tout `.git`).

## 2026-10-28 — Plugins à effets de bord : sept portés, six rendus (docs/plans/plugins-round2.md)

Douze entrées du corpus ne tenaient plus qu'à un ou deux plugins qui
écrivent ou suppriment des fichiers à des événements de script.

- **Portés** (sources lues aux versions verrouillées, tags aux sources
  identiques admis, toute autre version rendue nommée — chargée au début
  de la commande comprise) : dg/composer-cleaner v2.2.2 (`Cleaner::clean`
  sur l'arbre réel, `fnmatch` POSIX porté), liborm85/composer-vendor-cleaner
  1.7.1 (deux temps, `Glob::toRegex($g, false)` sur PCRE2),
  cakephp/plugin-installer 2.0.2 (autoload racine au `pre-autoload-dump`,
  `cakephp-plugins.php` au `post-autoload-dump`, `-v` reproduit),
  laminas/laminas-skeleton-installer 1.2.0–1.3.0 (installation minimale
  non interactive, priorité 1000, composer.json réécrit en place par
  l'encodage de `JsonFile`) ; inertes là où ils ne font rien :
  spiral/composer-publish-plugin, sculpin/sculpin-theme-composer-plugin,
  laminas/laminas-component-installer.
- **Rendus** : drupol/composer-packages (`GENERATED_AT = time()` dans cinq
  classes ; le reste est déterministe mais lourd — Twig, `var_export` du
  lock, chemins absolus), ondrejmirtes/composer-attribute-collector (durée
  sur stdout, analyse PHP), typo3/class-alias-loader (suffixe aléatoire sur
  vendor vierge, cartes d'alias PHP exécutées), mlocati/composer-patcher
  (`patch`/`git apply`, installed.json réécrit), contao/manager-plugin et
  typo3/cms-composer-installers (code des paquets exécuté).
- **Le retrait de laminas-skeleton-installer est rendu** : `uninstall`
  rappelle l'installation manager sur lui-même, Composer boucle (mesuré par
  la méta-analyse). `require`/`remove` avec la configuration présente aussi
  (`RequireCommand` relit composer.json pour le hash).
- **Le cache de classmap par entrée du store** suppose une copie intacte.
  Un nettoyeur la contredit : l'enregistrement n'est plus écrit que d'une
  copie qui a exactement les fichiers de l'entrée (un second parcours, sans
  lecture, du répertoire du store à chaque enregistrement froid), et il
  n'est servi, quand liborm85 est configuré, verrouillé ou installé, que si
  chacun de ses fichiers existe encore. Alternative mesurée et écartée :
  vérifier chaque répertoire de chaque enregistrement à chaque install,
  +8 % sur un no-op d'invoiceninja en `-o` (72 430 classes, 0,28 → 0,30 s,
  MacStudio M-series 14 cœurs, trois séries de dix). Risque assumé : un
  vendor nettoyé par liborm85 dont le plugin et la configuration ont
  disparu, paquets non réinstallés depuis, peut encore recevoir les classes
  de fichiers supprimés (classmap ou `-o` seulement).
- **Hors du sujet, corrigés parce que le banc et le corpus les ont
  montrés** : la ligne `Downloading` d'un métapaquet à dist ; la violation
  PSR qu'une règle suivante lève (`ClassMap::addClass`, owncloud-core en
  dev) ; `Glob::toRegex` élargissait en Latin-1 les octets non ASCII.
- **Vus, laissés** : `vendor/bin` vide que Composer recrée quand une mise à
  jour sans binaire suit le retrait qui l'a vidé (vivacity le supprime :
  l'ordre des `removeBinaries` n'est pas rejoué ; le banc l'évite) ;
  `update`/`require` sur un dépôt `artifact` s'arrêtent en erreur au lieu
  de partir à Composer.

## 2026-10-28 — Une ligne de stderr, une écriture

Contexte : un vrai déploiement Deployer 7.5.12 (recette Laravel, localhost)
affichait `Dependency resolution completed in` et sa durée sur deux lignes.
`eprintln!` écrit le stderr non tamponné morceau par morceau (le texte
autour de chaque valeur, puis le saut de ligne) ; Deployer préfixe chaque
lecture du pipe. Les octets étaient ceux de Composer, leur découpage non :
PHP envoie chaque ligne en un `fwrite`.

- **Décision** : les lignes de stderr du module `io` (émises, différées,
  retenues) sont formatées puis écrites d'un `write_all`. Un échec
  d'écriture est ignoré, comme Composer l'ignore (avant : panique
  d'`eprintln!`). Risque assumé (relevé par la revue) : un stderr que le
  parent a mis en non bloquant peut perdre la fin d'une ligne sans bruit,
  comme sous PHP ; un stdout fermé panique encore (`println!`).
- **Mesure** (lecteur de pipe qui compte les lectures qui ne finissent pas
  une ligne, install Laravel 13 `-v --no-progress`, MacStudio M4 Max) :
  avant 5 à 15 lectures coupées par run, Composer 0 sur 3 runs, après 0 sur
  5 runs.
- **Laissé** : stdout reste sur `println!` (tamponné par ligne) ; les traces
  `VIVACITY_TRACE` gardent `eprintln!` (diagnostic, pas de pendant
  Composer).
- **Vu, laissé** : la durée de la ligne `Dependency resolution completed`
  mesure, chez vivacity, toute la vérification du lock jusqu'à la jointure
  de son fil (0,08–0,11 s sur ce projet) ; celle de Composer ne mesure que
  `Solver::solve` (0,000 s). Les octets diffèrent par la valeur, que les
  harnais masquent.

**Addendum du 2026-10-29 — stdout aussi.** `println!` passe par le
`LineWriter` de stdout (tampon de 1 Kio) : une ligne plus longue partait
en plusieurs écritures. Compté avec un intercepteur de `write(2)` sur un
lock insoluble avec `GITHUB_ACTIONS=1` : l'annotation `::error ::` de
11 529 octets sortait en 10 + 11 518 + 1 octets chez vivacity, en une
écriture chez Composer ; un lecteur de pipe l'a vue coupée 1 fois sur
100 runs.

- **Décision** : stdout passe par le même `write_line` que stderr (formatée,
  puis un `write_all` ; tampon vide, le `LineWriter` la donne entière au flux
  brut). `#![deny(clippy::print_stdout)]` sur les bibliothèques `vivacity` et
  `vivacity-core` empêche un `println!` de revenir (vu rouge sur un
  `println!` planté).
- **Échec d'écriture ignoré sur stdout aussi** (relevé par la revue
  adverse ; le plan gardait la panique de `println!`) : `dump-autoload`
  vers un pipe déjà fermé paniquait (101) avant d'écrire
  `vendor/autoload.php` ; Composer (`@fwrite`, `SIGPIPE` ignoré) finit en 0.
  Le risque assumé le 28/10 (« un stdout fermé panique encore ») est levé ;
  un embarqueur (ePHPm) ne reçoit plus cette panique. Reste : une ligne
  peut être perdue sans bruit, comme sous PHP.
- **Ce que la mesure dit et ne dit pas** : la même forme d'écriture que
  Composer, pas « aucune ligne coupée » : un lecteur qui lit moins qu'une
  ligne, ou un pipe presque plein, coupe aussi celles de Composer.
- **Laissé** : les traces `VIVACITY_TRACE` gardent `eprintln!` (diagnostic ;
  paniquent si stderr échoue) ; pas de test automatique du nombre
  d'écritures (intercepteur macOS seulement ; un portage `LD_PRELOAD` ou
  `strace -e write` sous Linux reste possible). Les tests en processus
  affichent désormais les lignes de stdout que libtest capturait.

## 2026-10-29 — La durée de la résolution, drupol/composer-packages, l'attribute-collector (docs/plans/duration-drupol-collector.md)

**La ligne `Dependency resolution completed in N seconds` d'`install -v`
ne mesure plus que le SAT.** Le minuteur de Composer n'entoure que
`runSat` (`Solver.php:225-228`) : ni le pool, ni la liste de blocage, ni la
plateforme, ni les règles. Celui de vivacity partait de « Verifying… » et
s'arrêtait à la jointure du fil de vérification : attente réseau, sonde de
PHP, version racine, pool, règles, et ce que le fil principal faisait en
parallèle.

- **Décision** : `Solver::solve` garde la durée de son `run_sat`
  (`sat_time`), `verify_install` la rend (`InstallVerification::resolution`)
  quel que soit le verdict — Composer écrit la ligne avant de lever ses
  problèmes —, `join_verification` l'imprime. Le fil n'a pas eu à changer.
- **Mesure** (fixtures/projects/laravel, 76 paquets, MacStudio M4 Max,
  PHP 8.5.12) : Composer imprime 0.000 (4 runs ; son `runSat` à six
  décimales, phar extrait au format seul modifié : 0,000231–0,000371 s) ;
  vivacity imprimait 0.239 (vierge), 0.074, 0.065, 0.069 ; il imprime
  0.000 (SAT : 0,000010–0,000024 s, 5 runs).
- **Preuve** : un test de `verify_install` où la version racine arrive
  200 ms après le départ du fil exige une durée sous 50 ms ; vu rouge
  (208,9 ms) avec l'ancien point de départ rebranché.
- **Laissé (consigne)** : la ligne de `update` (session.rs) chronomètre
  tout `Solver::solve`, règles comprises. `sat_time` permettrait la même
  correction en trois lignes.

**drupol/composer-packages 2.0.0 est émulé** (MIT, Pol Dellaiera ; copie
de référence et gabarits Twig verbatim, NOTICE.md). À `post-autoload-dump`,
ses cinq classes sous `vendor/drupol/composer-packages/build/`, depuis les
données du lock (`packages` puis `packages-dev`, quel que soit
`--no-dev`), et ses deux lignes stdout dans l'ordre d'activation.

- **Les gabarits gardés tels quels**, rendus par un port du sous-ensemble
  de Twig qu'ils emploient (texte, `{{ }}` échappé en HTML, filtres
  `export` et `camelize`, `for`, la fin de ligne qui suit un `%}` retirée,
  l'indentation qui précède gardée) plutôt que réécrits à la main : la règle
  d'espaces de Twig s'applique alors au vrai texte.
- **`var_export` écrit l'exposant en `E`** (`1.0E+20`), `json_encode` en
  `e` : `runtime_stub::php_var_export` reprenait l'encodeur JSON. Corrigé
  pour tous ses appelants (le stub de symfony/runtime aussi), test sur des
  valeurs mesurées sous PHP.
- **Chemins** : `getInstallPath` de LibraryInstaller sous le `realpath` de
  PHP — liens suivis, casse donnée gardée (`std::fs::canonicalize` prend
  celle du disque sur macOS) — et `Package::getTargetDir()` porté (regex de
  Composer, `"0"` faux) ; `NULL` pour un métapaquet.
- **Le cache de classmap par entrée du store** : au no-op suivant, le dump
  de Composer trouve les cinq classes générées sous `build/` (bolt est en
  `optimize-autoloader`) ; l'enregistrement du store, fait d'une copie
  vierge, les ignorait. Le paquet n'est plus jamais rattaché à son entrée
  du store (`ClassmapCacheConfig::mutable_copies`), aussi sous
  `--no-plugins` ou plugin retiré : le cache par fichier le sert. Vu rouge
  sans (le no-op du banc perd les quatre classes de la classmap).
- **Comparaison** : `compare.sh` remplace par `<t>` les seuls chiffres de
  `    public const GENERATED_AT = <entier>;` dans ces cinq fichiers du
  vendor, puis compare strictement (une ligne absente ou dédoublée diffère
  encore ; auto-tests). Le banc vérifie en plus que la valeur de vivacity
  tombe dans la fenêtre de son passage.
- **Rendus, nommés, avant toute écriture** : une autre version (au lock ou
  chargée), un plugin global, Windows, son retrait (son `uninstall` fait
  `unlink` du répertoire `build/` : Composer échoue, code 1, mesuré), un
  plugin installeur (composer/installers, wordpress-core-installer, mnsami,
  les huit installeurs personnalisés) ou un nettoyeur (dg, liborm85) actif,
  un paquet `symfony-pack` dans le lock (le chemin dépend de la version de
  Flex), un dépôt `path` ou une installation depuis la source, aucun lock
  lisible (`dump-autoload` sans lock ou sous `config.lock: false` : le
  plugin lève une `LogicException`), `--run-scripts` avec un script racine
  `post-autoload-dump` (`run-script` relancerait l'écouteur ; décidé avant
  le script `pre-install-cmd`), Twig hors 3.x, et un lock que l'on ne
  relit pas comme PHP (clé `packages-dev` absente, nom sans exactement un
  `/`, type numérique, nom en double, octet NUL, zéro négatif — `-0`
  entier et `-0.0` flottant sont indiscernables une fois lus).
- **Banc** `harness/composer-packages.sh` : 29 cas, Composer et vivacity au
  même chemin absolu (le chemin entre dans `Directories.php` sous
  `var_export` et sous l'échappement HTML) ; 18 étapes comparées (install,
  no-op, mise à jour, retrait, `dump-autoload` ×3, `--no-plugins` ×2,
  `--no-dev` ×2, `-v`, sans nom racine, plugin refusé, `update`,
  `require -v`, `remove -v` — classes renommées `_composer_tmpN` —,
  `config.lock: false`) et 11 rendus.

**ondrejmirtes/composer-attribute-collector 1.2.0 reste rendu (résultat
négatif, aucun code).** Spike borné (moins de deux heures) sur la source au
tag (BSD-3-Clause) et phpstan-src au commit du corpus.

- Sous PHP ≥ 8.0 le plugin collecte par **réflexion** :
  `new ReflectionClass($class)` charge (exécute) chaque classe et ses
  parents, `getMethods()`/`getProperties()` héritées, et
  `ReflectionAttribute::getArguments()` **évalue** les arguments — des
  expressions constantes — avant de les `var_export`er. L'AST de
  nikic/php-parser ne sert que sous PHP 7. Avant tout,
  `require_once vendor/autoload.php` exécute les `files` de la racine et
  de tous les paquets dans le processus de Composer.
- phpstan-src (0e91c3d) : 1 152 fichiers de `src/` contiennent `#[`, avec
  des arguments comme `tag: self::EXTENSION_TAG`,
  `RegistryFactory::COLLECTOR_TAG`, `as: Foo::class`. La parité du fichier
  généré demande un analyseur PHP, la résolution des noms, l'héritage
  (parents, traits, interfaces, dans vendor/ aussi) et un évaluateur
  d'expressions constantes : hors de portée sans port d'un analyseur.
- **Écarté aussi** : le cas vide. La fixture du corpus n'a pas de classe
  sous `src/` : `attributes.php` y est fixe (quatre `array (\n)`), mais
  ses deux lignes stdout portent une durée, l'autoloader est exécuté, et
  l'émuler ne rendrait natif que l'artefact du corpus — le vrai projet a
  1 152 fichiers à attributs. En dev, phpstan-src est de toute façon rendu
  pour cweagans/composer-patches.

**Vus, laissés (sans lien avec ces trois chantiers, mesurés par le banc ou
la revue)** : un chemin de projet qui contient `'` fait écrire à vivacity
`autoload_static.php` en chemins absolus là où Composer écrit
`__DIR__ . '/..' . …` ; un nom de paquet à majuscules est écrit tel quel
comme clé d'installed.php (Composer : `getName()`, en minuscules, à sa
place triée) ; `-0` dans un lock devient `-0` dans installed.json (Composer
le relit comme l'entier 0) ; sous `--run-scripts`, un repli décidé par
l'analyse de portée arrive après le script `pre-install-cmd`, que Composer
relance ; `target-dir: "0"` et un `vendor-dir` à travers un répertoire
absent divergent ; un zip `artifact` refabriqué à version égale reste servi
par le store.

## 2026-10-29 — Les replis restants du corpus : trois plugins portés, un inerte observé, trois résultats négatifs (docs/plans/remaining-fallbacks.md)

Les verdicts du 2026-10-28 (« rendus », une ligne chacun) ont été repris
plugin par plugin : source lue au tag verrouillé quand la licence le
permet, comportement mesuré contre Composer 2.10.3 sur l'entrée du corpus
et sur des projets synthétiques (macOS 26, Apple Silicon, PHP 8.5.12 et
8.4.27, `/usr/bin/patch` « 2.0-12u11-Apple », git 2.54.0).

- **infection/extension-installer 0.1.2 — porté** (BSD-3-Clause). Le
  verdict « inconnu » ne tenait qu'à l'absence d'étude : c'est l'écriture
  de phpstan/extension-installer sans comparaison md5 (le fichier et les
  lignes à chaque passage). La liste des extensions invalides suit l'ordre
  du dépôt local, non trié : après un install, celui de vivacity est
  l'ordre des opérations, celui de Composer l'achèvement des extractions
  (même limite que pest-plugin, comparée au no-op seulement).
- **typo3/class-alias-loader v2.0.1 — porté** (MIT). « Suffixe aléatoire
  sur vendor vierge » est exact mais ne bloque pas : le plugin tire
  `md5(uniqid('', true))` parce que `vendor/autoload.php` n'existe pas
  encore au `pre-autoload-dump` (deux installs de Composer : deux
  suffixes) ; vivacity tire le sien et le comparateur masque ces deux
  lignes-là, ancrées, comme `GENERATED_AT` de drupol. Dès qu'autoload.php
  existe, le suffixe est celui de Composer, vérifié strictement par le
  harnais. « Cartes d'alias PHP exécutées » tient : un paquet (ou la
  racine) qui déclare `class-alias-maps` est rendu ; l'entrée typo3 n'en a
  pas (`always-add-alias-loader` seul), typo3-cms-base-distribution en a
  quatre.
- **typo3/cms-composer-installers v5.0.2 — inerte sous la racine
  `typo3/cms`, sa source non ouverte dans ce chantier** (GPL-2.0-or-later ;
  le tour du 2026-10-28 l'avait lue pour la classer). La règle vient
  d'installs `-v` avec le plugin autorisé puis refusé, comparés : la racine
  `typo3/cms` (le dépôt TYPO3, l'entrée du corpus ; une racine synthétique
  de ce nom avec un paquet `typo3-cms-extension`, `extra.typo3/cms.web-dir`,
  un `vendor-dir` ; `dump-autoload`) donne le même arbre ; toute autre
  racine (nue, extension, `web-dir`, typo3/cms-cli, `extra.typo3/cms` d'un
  paquet, racine de type `typo3-cms-core` ou `typo3-cms-extension`, racine
  qui `replace` typo3/cms-core) reçoit `vendor/typo3/autoload-include.php`,
  inscrit dans l'autoload de la racine sous un identifiant qui dépend du
  chemin absolu. Rendus : toute autre racine, typo3/cms-core installé,
  `-v` (quatre lignes d'écouteurs et « Executing TYPO3 installer scripts »).
- **typo3-cms-base-distribution — négatif.** Avec typo3/cms-core installé,
  le plugin écrit `vendor/typo3/PackageArtifact.php` (des objets
  `TYPO3\CMS\Core\Package\Package` sérialisés : le code de typo3/cms-core,
  GPL lui aussi), publie les ressources des extensions
  (`public/_assets/<md5>`) et crée `public/index.php` et `var/` ; et
  **l'ordre de `composerNameToPackageKeyMap` change d'un run de Composer à
  l'autre** dans le même répertoire (trois runs, trois ordres). La parité
  n'a pas de cible.
- **mlocati/composer-patcher 1.2.6 — porté** (MIT). « `patch`/`git apply`,
  installed.json réécrit » est exact et ne bloque pas : vivacity lance **le
  même binaire avec les mêmes lignes de commande, dans le même ordre**
  (`patch --version`, `patch --help`, puis par niveau un essai à blanc et
  l'application, le compteur des fichiers `.rej` compris ; `git -C … apply`
  pour un paquet qui porte un `.git`) — rien d'un correctif n'est
  interprété. Seul le nom du répertoire temporaire du plugin
  (`tempnam(…, 'PCH')`) diffère, dans les messages d'échec de `patch` ; le
  harnais le ramène à un marqueur. Mesuré : le `patch` de macOS ne
  reconnaît pas un correctif déjà appliqué comme le plugin l'attend (pas de
  « Skipping patch. ») — un no-op y imprime un échec par correctif, chez
  Composer comme chez vivacity ; GNU patch (Linux, la CI) le reconnaît.
  Avant que `patch` écrive, la copie du paquet est détachée du store
  (chaque fichier lié recopié, nouvel inode — un test unitaire, vu rouge en
  neutralisant le détachement ; sous macOS le clonage est déjà en
  copie-sur-écriture) ; ses classes sont relues sur disque (le cache de
  classmap du store ne sert plus une copie corrigée). Rendus : `update`,
  `-v`, `patches-file`, un correctif distant, une racine ou un paquet
  aliasé visés, une cible `path` ou installée depuis la source, une
  contrainte sur une version de branche, une cible que le
  `pre-install-cmd` désinstallerait (hash changé), `--run-scripts` avec un
  script d'install, Windows.
- **contao/manager-plugin 2.13.6 — négatif** (LGPL-3.0-or-later, non lu).
  Une instruction ajoutée en tête du fichier de la classe
  `Contao\NewsBundle\ContaoManager\Plugin` installée laisse sa trace
  pendant un `composer install` no-op : le `post-install-cmd` du plugin
  charge le code des paquets. L'ordre de `.generated/plugins.php` n'est ni
  celui du lock, ni l'alphabétique, ni un tri topologique des `require`
  (core-bundle, dont les sept autres dépendent, arrive septième sur huit).
- **dealerdirect v0.7.2 — portable, non porté.** Le diff avec v1.2.1 est
  petit (recherche toujours depuis la racine du projet, `phpcs` du `PATH`
  pour `--config-show`, `./scripts/phpcs` pour phpcs 2, messages `-v`).
  Mais sur l'entrée du corpus, **la référence échoue** : phpcs charge
  `vendor/autoload.php`, dont l'autoload `files` de la racine
  (`public/legacy.php`) `require` treize fichiers absents de la fixture ;
  ajoutés, ils appellent `Database`, d'un répertoire `classmap` absent lui
  aussi. Composer imprime « Failed to set PHP CodeSniffer installed_paths
  Config » et le corpus classe l'entrée `unavailable` — contre la barrière
  « 0 unavailable ». La note du HANDOVER qui attribuait cet échec à phpcs
  sous PHP 8.5 est fausse : phpcs 3.13.6 tourne, c'est le PHP du projet qui
  échoue. Plus largement, `phpcs` exécute les `autoload.files` du projet,
  dont vivacity ne peut prédire l'échec (vrai aussi pour 1.x). Suites
  possibles : compléter la fixture (le code legacy de chamilo et ses
  classes, plusieurs Mo) ou porter v0.7.2 et accepter l'entrée hors mesure.
- **installed.php d'une racine aliasée** : Composer parcourt aussi le
  `RootAliasPackage`, et ses `replace` à `self.version` y prennent la
  version de l'alias — la racine typo3 (`dev-main` aliasée en
  `15.0.x-dev`) liste les deux versions pour chacun des cinquante paquets
  qu'elle remplace. vivacity n'en listait qu'une : un écart de cœur que
  seule cette entrée montrait, corrigé parce qu'il bloquait typo3 (la même
  règle pour un paquet aliasé du dépôt local, signalée par la revue, n'est
  pas reproduite ni corrigée : HANDOVER).
- **`--run-scripts`** : la garde de drupol devient `replay_issue`, une
  table des plugins dont l'écouteur imprime ou écrit à chaque passage
  (drupol, infection, phpstan — « Extensions installed » à chaque fois —,
  class-alias-loader, composer-patcher) et des événements que
  `composer run-script` rejouerait, par commande.

Harnais : `harness/plugins-round3.sh` (infection, typo3) et
`harness/composer-patcher.sh`, vus rouges sur main ad1c338 puis verts sous
PHP 8.5 et 8.4 (comptes au plan) ; `compare-selftest.sh` : la tolérance du
suffixe, rouge avec le comparateur de main, verte ensuite, et ancrée (une
autre ligne, un suffixe court, le même fichier ailleurs, la même ligne dans
un autre fichier : vus).

Corpus (`docs/corpus/2026-10-29-remaining-fallbacks.md`, les deux modes,
binaire 5afb474) : **95/106 natives en `--no-dev`, 94/106 en dev** (93 et
91 avant), 0 diff, 0 unavailable ; roave-backwardcompatibilitycheck
(dev), typo3 et concretecms (deux modes) deviennent natives ; restent
rendues dans le champ typo3-cms-base-distribution, contao-managed-edition
et chamilo-lms (dev).

## 2026-10-29 — L'action ne met en cache que le cache de Composer, résolu par le binaire (docs/plans/action-cache.md)

**Constat.** `action.yml` mettait en cache, avec `actions/cache@v4`, six
chemins écrits en dur : les trois répertoires du store et des
enregistrements de classmap de vivacity, les trois du cache de Composer.

- **Le store restauré est mort depuis le 27/10** : `store.rs` est passé de
  `LAYOUT` `v2` à `v3` (05193b7) ; l'artefact ne contient que `store/v2`,
  jamais lu, et la clé primaire touchée empêche toute nouvelle sauvegarde.
  Run 40857206667 (29/10), job `action (ubuntu-latest)` : 40 Mo restaurés,
  puis `0 from store, 109 from cache`. Même chose du 30/09 (v1→v2, 29cb00e)
  au 22/10.
- **Vivant, il ne paie pas.** Runs du workflow `action` (Laravel, 109
  paquets), restauration + install sur clé trouvée : ubuntu 3,0 s (store
  vivant) contre 2,7 s (mort) ; Windows 26,1 s contre 21,6 s (store sur
  `C:`, projet sur `D:` : copie, pas de lien) ; macOS noyé dans le bruit du
  téléchargement (le même artefact restauré en 6,2 s ou 12,0 s). Un run par
  cas : une indication, pas une mesure. `bench/M7-ci-cache.md` allait dans
  le même sens (addendum du jour).
- **Les chemins étaient figés** : un `COMPOSER_CACHE_DIR`, `COMPOSER_HOME`
  ou `XDG_CACHE_HOME` du projet sortait le cache de Composer des six
  chemins.

**Décision.**

- L'action installe le binaire, puis lui demande le répertoire
  (`vivacity config --global cache-dir`, même avec `cache: 'false'`), le
  journalise, l'expose en sortie `composer-cache-dir`, et ne restaure et
  sauve que lui, avec `actions/cache@55cc8345…` (v6.1.0, épinglé par SHA),
  sous la même forme de clé. Une valeur vide, relative ou que Composer
  désactive (`/dev/null`, `nul`) : `::notice::` et pas de cache. Demander
  au binaire plutôt que recopier l'algorithme en bash : le répertoire sauvé
  est celui où vivacity écrit (méta-analyse, point 3).
- **Le cœur suit `Config::get('cache-dir')`**, pas seulement
  `Factory::getCacheDir` : barres finales retirées, `~`, `$VAR`, `%VAR%`
  développés, relatif résolu contre le répertoire de travail, `cache-dir`
  du `config.json` global lu, `useXdg` vrai si `/etc/xdg` est un
  répertoire. `composer_home` devient `Config::get('home')` (développé,
  `0` traité comme absent, `/etc/xdg`). Jumeaux `Config.php` et
  `ConfigCommand.php` ajoutés à `docs/reference/`.
- **Rendu à Composer, nommé, avant toute écriture** (install, update,
  require, remove ; dump-autoload pour les trois premiers) : valeur vide ou
  `0` (Composer écrit `.htaccess` et ses zips dans le répertoire de travail
  ou `./0`, mesuré), Windows sans `APPDATA` ni `COMPOSER_HOME` et `HOME`
  absent (Composer s'arrête), une référence `{$clé}`, un `config.json`,
  `auth.json` ou `COMPOSER_AUTH` invalide (Composer s'arrête) ou une clé de
  cache au premier niveau des deux derniers (`Factory` les fusionne sous
  `config`) ; `COMPOSER_CACHE_FILES_DIR`, `COMPOSER_CACHE_REPO_DIR` (ou
  leurs clés du `config.json`), `cache-dir` / `cache-files-dir` /
  `cache-repo-dir` du `composer.json` du projet ; un relatif sous
  `--working-dir` (Composer le résout après son `chdir`, vivacity ne change
  jamais de répertoire).
- `actions/cache` v6.1.0 : son `action.yml` ne diffère de v4.3.0 que par
  `node20` → `node24`. Prémisse corrigée par la méta-analyse : node20 n'est
  pas retiré des runners hébergés (ils le forcent en node24 avec un
  avertissement) ; passer en v6.1.0 échange cet avertissement contre
  l'exigence d'un runner ≥ 2.327.1 (CHANGELOG, Changed).
- Changer `path` change la « version » du cache d'`actions/cache` : chaque
  dépôt subit un échec de cache unique à la mise à jour.

**Vérifié.** `harness/composer-cache-dir.sh` (sans réseau, dépôt
`artifact`) : `config --global cache-dir` à l'octet (stdout, stderr, code)
et l'endroit où le zip atterrit, sur 20 environnements plus un relatif
sous `--working-dir` (`~\cc` comparé sur la valeur seulement, Composer
échouant ensuite à écrire son cache à travers la contre-oblique hors
Windows) : 20 ok + 1 sauté (`/etc/xdg`) sous macOS, 21 ok en conteneur
Linux root (Composer 2.10.3, `/etc/xdg` créé le temps du cas). Vu rouge
avec le binaire 5ed42ff : 0 ok, 69 lignes d'échec sous macOS, 73 sous
Linux ; l'emplacement du zip faux dans 8 cas sous macOS (`~/cc`,
`$HOME/cc`, `%HOME%/cc` écrits dans `./~/cc`…, `COMPOSER_HOME=0` dans
`./0/cache`, vide, `COMPOSER_CACHE_FILES_DIR`, `cache-files-dir` du projet,
relatif sous `--working-dir`), plus `/etc/xdg` sous Linux. Tests unitaires
sur un hôte injecté (Windows, `/etc/xdg`, `HOME` absent), vus rouges sur le
port de l'ancien algorithme (6 sur 6), puis 3 sur 3 pour les points de la
revue.
`action-test.yml` compare la sortie à `composer config --global
cache-dir` (environnement du runner, puis `COMPOSER_CACHE_DIR` sur un second
appel sans cache) et exige `<sortie>/files` non vide après l'install, ce
`files/` restauré ayant été vidé avant ; avec l'ancienne action la sortie
est vide et les trois pas échouent, et un install qui écrit ailleurs fait
échouer le pas `files/` (simulation locale des pas, `bash -eo pipefail`).
Le workflow lui-même ne tourne qu'en CI.

**Revue indépendante** (sous-agent frais, sur `git diff 5ed42ff..`) :
aucun défaut haut. Intégrés : les clés de cache du `composer.json` du
projet (moyen ; Composer écrivait ses zips ailleurs, vivacity restait
natif), le test `files/` qui ne pouvait pas rougir sur une clé trouvée
(moyen), `~\` que Composer ne développe pas (bas), `auth.json` /
`COMPOSER_AUTH` / JSON global invalide (bas), le `\r\n` de `PHP_EOL` sous
Windows dans la comparaison (soupçon), l'affirmation « stderr à l'octet »
d'une délégation (doc). Laissés, consignés au HANDOVER : `config` refuse
`-q`/`-n`/`--no-ansi`/`-d` (la consigne : toute autre forme sort en 2) ;
`COMPOSER_BIN_COMPAT` invalide ou `./composer.json` invalide, où Composer
sort en 1 et `config` répond (hors du cache) ; la branche `cygpath -m` presque morte (MSYS
convertit déjà `/c/…` pour un binaire natif), gardée comme demandé. La notice à chaque run sous `cache: 'false'` a été retirée à la
vérification : un cache coupé exprès se dit dans le journal, pas en annotation.

## 2026-10-29 — Le jeton du job pour GitHub, aux deux côtés ; permissions en lecture ; un refus de GitHub nommé, pas relancé (docs/plans/ci-github-token.md)

Contexte : run 40894179601, job `parity (macos-latest)`, `flex-update.sh` rouge
sur ses 25 cas, dans la préparation de la référence : « In AuthHelper.php
line 152: Could not authenticate against github.com » (Composer non
interactif après un 401/403 de api.github.com ; la cause exacte n'est pas
imprimée). Mesuré : un zipball anonyme ne décrémente pas le compteur `core`
(`x-ratelimit-used: 0`), donc c'est une autre limite, vraisemblablement par
IP, que les runners macOS partagent. setup-php écrit déjà le jeton du job
dans `~/.composer/auth.json` ; flex-update.sh, à COMPOSER_HOME privé sans
copie de ce fichier et à cache vierge par run (~150 zipballs), téléchargeait
anonymement.

- **`COMPOSER_AUTH` au niveau des jobs `parity`** (ci.yml, windows.yml) :
  `{"github-oauth":{"github.com":<github.token>}}`. Composer et vivacity le
  lisent pareil, donc chaque côté de chaque harnais est authentifié, y
  compris un harnais à home privé à venir. Écartés : recopier `auth.json`
  dans chaque harnais (dépend de ce qu'écrit setup-php, à refaire pour
  chaque nouveau harnais) ; seulement réchauffer les caches (moins de
  volume, mais toujours anonyme sur une IP partagée). Pas de garde
  « jeton vide » : `github.token` est émis pour chaque run, en lecture seule
  sur un PR de fork. corpus.yml retombe sur ce jeton sans son secret.
- **Moins de volume aussi** : flex-update.sh et flex-recipes.sh prennent le
  cache de fichiers global (comme plugin-lines.sh), que `actions/cache`
  restaure avec les dists de la fixture symfony ; les lignes `  - Downloading`
  y étaient déjà hors comparaison.
- **Permissions** : `contents: read` en tête de ci, windows, action-test,
  bench, ci-cache, corpus, drift — les harnais exécutent le PHP de plugins
  tiers avec le jeton à portée (COMPOSER_AUTH, l'auth.json de setup-php, la
  configuration git de checkout). Le dépôt est déjà réglé en lecture
  (`Contents/Metadata/Packages: read` dans le run) ; l'écrire le fige contre
  un changement de réglage et retire `packages`. drift.yml avait
  `issues: write` pour tout le workflow, donc dans l'auth.json lu par les
  plugins de ses harnais : le rapport passe dans un job `report`
  (`issues: write`, `needs: drift`, `if: !cancelled() && échec`), sans
  checkout ni plugin, qui lit un artefact par canal et n'en garde que les
  valeurs que le job de drift peut produire (canal, issues, version au
  motif strict).
- **vivacity envoie le jeton là où Composer l'envoie** (`fetch.rs`) : une
  authentification par origine (`Url::getOrigin` : `*.github.com` sauf
  codeload → github.com, repo.packagist.org → packagist.org, port compris ;
  `findAuthOrigin`), la dernière chargée gagne (bearer > http-basic >
  github-oauth), le jeton de github.com en `token` vers
  `^https?://api.github.com/` SEULEMENT, un domaine GitHub Enterprise en
  `Basic jeton:x-oauth-basic`, et les sources dans l'ordre de Factory
  (auth.json global, du projet, puis COMPOSER_AUTH). Avant : `token` vers
  github.com et tout `*.github.com` (codeload compris), vers les domaines GHE,
  github > bearer > basic par hôte, COMPOSER_AUTH sous l'auth.json du projet.
  Sans effet avec un jeton valide ; avec un jeton révoqué, codeload aurait
  répondu 401 à vivacity et 200 à Composer. `harness/auth-hosts.sh` le
  prouve sans réseau (mandataire HTTP local qui journalise les en-têtes,
  302 api → codeload) : rouge avec le binaire de main 5768dc9 (codeload,
  github.com, objects.github.com, GHE, ordre), vert après ; et le jeton ne
  change ni stdout, ni stderr, ni le projet, d'un côté comme de l'autre.
- **net-retry.sh** : un refus de GitHub (« Could not authenticate against
  github.com », 403 de codeload, 429 non relancé de Composer ; 401/403/429
  du client de vivacity sur api/codeload) n'est **pas relancé** — sa limite
  se compte à l'heure ou par IP, un second passage la dépense encore — et le
  code du harnais est gardé : rouge reste rouge. La raison est écrite (quel
  libellé : celui de Composer, ou celui de vivacity seul, qui peut être
  vivacity envoyant ce que Composer n'enverrait pas ; COMPOSER_AUTH posé ou
  non, jamais sa valeur), `::error::` sous GitHub Actions. Écarté : une
  attente longue puis une relance (jusqu'à une heure de runner macOS pour
  une issue incertaine) ; un vert toléré (masquerait une régression de
  l'authentification de vivacity). `harness/net-retry-selftest.sh` rejoue
  l'extrait capturé : rouge sur l'ancien script (6 cas), vert sur le
  nouveau, les 5 cas de comportement inchangé verts sur les deux.
- **Restent** (HANDOVER) : vivacity ignore `secure-http` (télécharge un
  dist `http://` que Composer refuse, avec ses identifiants s'il y en a —
  préexistant) ; deux types d'auth sur une origine : Composer écrit
  « avoid overwriting already defined auth settings », vivacity non ; les
  clés d'auth posées dans `config` (composer.json, config.json global) non
  lues ; un harnais qui n'imprime pas la stderr de l'outil ne laisse pas
  net-retry voir le refus. Ce qui ne se prouve qu'en CI : que le jeton du
  job passe la limite d'un runner macOS, et ce qu'un zipball authentifié
  coûte au budget de 1 000 requêtes/heure du dépôt — les deux étapes
  « GitHub API budget » de ci.yml lisent `/rate_limit` (gratuit) aux deux
  bouts du job.

## 2026-10-29 — Binaires Linux : plancher glibc 2.17 par cargo-zigbuild, musl statique ajouté (docs/plans/linux-glibc-floor.md)

**Constat.** Les binaires Linux de v0.25.0, liés à la glibc du runner de
build (ubuntu 24.04), exigent `GLIBC_2.39` (`pidfd_spawnp`, `pidfd_getpid`
de la std, références faibles). Mesuré dans Docker, binaires de la
release, `--version` puis un vrai `install` réseau :

| image | glibc | aarch64 v0.25.0 | x86_64 v0.25.0 | 2.17 (zig), les deux |
|---|---|---|---|---|
| centos:7 | 2.17 | refus | refus | OK |
| amazonlinux:2 | 2.26 | refus | refus | OK |
| debian:10, rockylinux:8 | 2.28 | refus | refus | OK |
| ubuntu:20.04, debian:11 | 2.31 | refus | refus | OK |
| rockylinux:9, amazonlinux:2023 | 2.34 | refus | avertissement stderr | OK |
| ubuntu:22.04 | 2.35 | refus | avertissement stderr | OK |
| debian:12 | 2.36 | refus | avertissement stderr | OK |
| ubuntu:24.04, debian:13 | 2.39, 2.41 | OK | OK | OK |

En x86_64 l'entrée `GLIBC_2.39` de `.gnu.version_r` est marquée faible
(drapeau 0x02) : une glibc ≥ 2.34 charge le binaire mais écrit « weak
version 'GLIBC_2.39' not found » sur stderr à chaque lancement — la stderr
identique à Composer est cassée sur les runners `ubuntu-22.04`. En aarch64
l'entrée est forte : refus sous 2.39.

**Choix : `cargo zigbuild --target <triple>.2.17`** (cargo-zigbuild
0.23.4, zig 0.16.0, installés par pip dans un venv, épinglés par hachage :
`tools/zigbuild-requirements.txt`). Aucun changement de code ni de
`Cargo.lock` : PCRE2 préfixé et ring compilent avec `zig cc`. 2.17 est le
plancher de la std Rust pour ces cibles ; les cibles nommées (Ubuntu 20.04
et 22.04, Debian 11 et 12, RHEL 8 et 9, AL2023) demandaient 2.28, et rien
ne coûte entre les deux. Une std ou une dépendance qui exigerait un
symbole absent de 2.17 en référence forte ferait échouer l'édition de liens
de zig : pas de remontée silencieuse. Écartés : construire dans un
conteneur CentOS 7 / manylinux (les actions node n'y tournent pas, rustup à
installer à chaque job, x86_64 seulement en émulation sur le Mac) ; un
runner plus ancien (`ubuntu-22.04` = 2.35, ne couvre ni RHEL 8/9 ni AL2023
ni Debian 11, et se redéprécie).

**musl statique ajouté** (`x86_64-` et `aarch64-unknown-linux-musl`) : même
`cargo zigbuild`, 40 s, binaire statique de 10,3 Mo (aarch64), aucun
changement de code. Banc Laravel (109 paquets) dans `php:8.4-cli` arm64,
VM Docker de 4 vCPU (M4 Max), hyperfine, médianes de deux passes, même
rustc 1.99.0 :

| scénario | gnu natif (gcc) | gnu zig 2.17 | musl zig |
|---|---|---|---|
| no-op | 46,3 / 42,6 ms | 45,6 / 44,6 ms | 40,8 / 41,8 ms |
| warm (vendor/ supprimé) | 69,5 / 70,3 ms | 67,7 / 67,6 ms | 66,8 / 66,7 ms |
| dump-autoload -o | 30,0 / 27,0 ms | 32,5 / 28,3 ms | 30,6 / 30,9 ms |

Pas de ralentissement de l'allocateur de musl mesurable à 4 cœurs ; un
runner à beaucoup de cœurs n'est pas mesuré. Le binaire v0.25.0 (rustc
1.99.1, gcc 13) fait 24,8 / 25,1 ms en dump-o : l'écart suit la chaîne
(rustc, gcc), pas zig — à rustc égal, natif et zig sont dans le bruit. La
reprise à rustc 1.99.1 n'a pas pu être faite (Docker indisponible, plus
bas). `install.sh` choisit musl quand `ldd --version` dit musl (sinon
`/lib/libc.musl-<arch>.so.1`, propre à Alpine) : pas sur la seule présence
de `/lib/ld-musl-*`, que le paquet `musl` de Debian installe aussi.

**Table des symboles.** zig traduit le `strip = "debuginfo"` du profil
release en strip total : plus de `.symtab` (14,2 → 10,9 Mo en aarch64,
15,3 → 12,4 Mo en x86_64), backtraces sans noms. La garder demandait
`strip = "none"` puis `objcopy --strip-debug`, outil différent selon l'hôte
(binutils en CI, absent du Mac) : écarté, noté au CHANGELOG.

**Le contrôle** (`tools/linux-release.sh check`) : le maximum des
`GLIBC_` des « Version References » d'`objdump -p` (pas `objdump -T`, qui
liste des symboles : c'est l'entrée de version, avec son drapeau faible,
que le chargeur vérifie), NEEDED dans les bibliothèques de base ou statique
pour musl ; puis dans `centos:7` ou `alpine` (épinglés par digest),
`--version` à stderr VIDE, `install -vv` rendu à un faux `composer` (un
processus fils, son code 7 relu : sans PHP dans l'image, rien d'autre ne
lance de processus) et un vrai `install` de `fixtures/projects/scripts`.
Vu rouge sur les binaires v0.25.0 (ELF : « requires GLIBC_2.39, above the
floor 2.17 », avec les entrées 2.18…2.39 et leurs drapeaux), vert sur les
quatre binaires zig, rouge sur un binaire d'une autre architecture et sur
un binaire gnu présenté comme musl ; il a trouvé son propre défaut (une
sortie de `set -e` muette quand aucune `GLIBC_` n'est trouvée). En CI :
chaque release (quatre triples, en natif), chaque push (x86_64 gnu et
musl, job `installers`), et un témoin négatif à chaque push qui exige que
le contrôle REFUSE l'asset v0.25.0 (sha256 épinglé) pour la bonne raison.

**Méta-analyse** (sous-agent frais, plan et code) : neuf points, intégrés
au plan — stderr vide exigée (le symptôme x86_64 était un lancement
réussi), version-need au lieu des symboles, texte du témoin négatif,
détection musl, curl absent d'Alpine (`install.sh` le dit ; le test passe
aussi par `file://`), réseau de la release réduit à deux paquets avec une
nouvelle tentative, chemin `spawn` exercé, venv hors de `target/`. En
écrivant le test Alpine : `sha256sum -c --quiet` d'`install.sh` ne passe
pas sous BusyBox (`getopt32(argv, "^scwbt…")`, pas d'option longue) →
sortie jetée.

**Ce que le Mac n'a pas pu prouver.** En cours de chantier le disque de
l'hôte s'est rempli (120 Mio libres, worktrees parallèles), et la VM Docker
est restée en erreur d'E/S (containerd) après la libération de place. La
matrice et le banc ont été faits avant, sur les binaires du spike ; les
binaires finaux de `tools/linux-release.sh build` leur sont identiques à
l'octet (sha256, gnu aarch64 et x86_64, musl aarch64 : la build zig est
reproductible ici), donc la matrice vaut pour eux. Pas faits localement :
le script `check` complet (sa partie conteneur), les binaires musl dans
alpine (le musl aarch64 n'a tourné que dans `php:8.4-cli`, Debian, pour le
banc ; le musl x86_64 nulle part), `install.sh` dans centos:7 et alpine.
Ils sont laissés au run `release` en `workflow_dispatch` et au job
`installers`.

**Revue indépendante** (sous-agent frais, sur `git diff 5768dc9..`) :
aucun défaut haut. Intégrés : `grep -q` sur le tuyau d'`objdump -h` (un
SIGPIPE lu comme « pas de `.interp` » : sortie lue en entier d'abord) ;
nœuds de version non numériques (`GLIBC_PRIVATE`, `GLIBC_ABI_DT_RELR` de
2.36) refusés ; un `objdump` en échec dit `FAIL` et laisse tourner le
conteneur ; copie du fixture gardée ; la ligne `FAILED` d'un hachage faux
rendue sur stderr par `install.sh` ; `log` imprimé quand `install.sh` échoue
dans le conteneur ; clé `rust-cache` par cible dans release.yml (deux lignes
Linux partageaient la même) ; job `installers` nommé par cible — **les
noms de contrôle changent** (`installers (x86_64-unknown-linux-gnu)` au
lieu de `installers (ubuntu-latest)`), à reporter dans une protection de
branche qui les exigerait. Laissé : les zipballs d'`api.github.com` du
fixture, appelés sans jeton depuis le conteneur (2 requêtes par essai,
limite de 60 par heure et par IP).

## 2026-10-29 — Les enregistrements de classmap restent hors du cache de l'action (docs/plans/classmap-cache.md)

**Question.** L'action ne met en cache que le répertoire de cache de
Composer ; y ajouter `<cache vivacity>/classmap/` (et `classmap-files/`)
accélère-t-il l'install d'un runner froid (checkout neuf, `vendor/` absent,
zips restaurés, pas de store) ?

**Ce que dit le code.** La clé d'un enregistrement de store est
`sha1(CACHE_FORMAT, chemin de l'entrée, sous-répertoire)` ; le chemin
(`<cache vivacity>/store/<LAYOUT>/<paquet>/<version>-<12 caractères de la
référence>`) ne dépend ni du contenu ni de la date de l'entrée. Un store
reconstruit depuis les zips redonne les mêmes clés : un enregistrement
restauré au même chemin absolu sert (117 sur 117, Laravel `-o`), sous un
autre chemin non (117 réécrits). `classmap-files/` est clé sur
`(mtime ns, taille)` : un checkout neuf n'y trouve rien (4 sur 4 réécrits).

**Mesure** (`bench/classmap-cache.sh`, `bench/M8-classmap-cache.md` ; M4 Max,
APFS, hyperfine 10 runs + chauffe, médianes) : avec `-o`, 2 384 → 1 026 ms
sur Laravel, 8 178 → 5 382 ms sur Sylius (le scan : 1,9 s → 0,02 s ;
4,2 s → 0,1 s) ; sans `-o`, ~0,2 s sur Laravel, dans le bruit sur Sylius.
Coût : ≤ 3 Mo d'artefact, 14-110 ms d'extraction. Pas de chiffre Linux
local (disque du Mac plein pendant le banc, démon Docker en erreur d'E/S
depuis) ; le workflow `classmap-cache` (`workflow_dispatch`, trois OS,
Windows avec les caches sur `C:` et le projet sur `D:`) le donnera.

**Décision.** `action.yml` ne change pas — pour une raison de correction,
pas de performance (méta-analyse, point 1). En CI, le store est réextrait
par le binaire du jour depuis le zip du jour, et la restore-key ramène des
enregistrements écrits par un autre lock ou un autre vivacity ; leur clé
ne porte ni l'URL de la dist (un miroir qui applique `archive.exclude` sert
la même référence avec d'autres fichiers) ni les règles d'extraction
(7e53145 et ed02f07 ont changé les noms Windows sans bump de `LAYOUT`).
Un enregistrement servi n'est pas recontrôlé contre `vendor/` (sauf plugin
nettoyeur) : la classmap listerait des fichiers absents ou en oublierait,
peut-être dans l'artefact d'un `install --no-dev -o`. Sur un poste,
l'entrée et son enregistrement vieillissent ensemble ; le risque est propre
à la CI, et 1-3 s ne le paient pas. La règle de décision écrite avant la
mesure (« un gain sous Linux au moins ») n'est pas remplie non plus.

Conditions d'une inclusion future (bench M8) : une clé d'enregistrement
qui porte l'identité de l'archive et des règles d'extraction (cœur, bump
de `CACHE_FORMAT`) ; une clé de cache liée au binaire, sans restore-key
entre versions ; les jobs en matrice qui partagent une clé (le premier à
sauver fixe le jeu `-o`/`--no-dev`) ; un gain Linux mesuré.

La phrase du README « restored, they did not make the install faster » ne
valait que pour le store : M7 mesurait store et enregistrements ensemble.
Corrigée.

**Méta-analyse** (sous-agent frais, plan et code) : quinze points. Outre
le point 1 : le banc ne tournait pas sous Windows (`cygpath -w` donne un
chemin à espace et contre-obliques que hyperfine découpe ; corrigé en
barres obliques entre guillemets, vu rouge puis vert sur un chemin à
espace), Windows mesurait tout sur `D:` (caches déplacés sous
`%LOCALAPPDATA%`), « servis » compte aussi les enregistrements non
utilisés (la colonne ne vaut qu'avec la chute de `dump scan` ; le banc
s'arrête si rien n'est écrit ou si l'install est rendue à Composer, deux
contrôles vus rouges). Relevé et vérifié, non corrigé (HANDOVER) : le
générateur cherche l'entrée par `dist_reference()`, l'installeur l'écrit
par `store_reference()` ; une dist sans référence n'a jamais
d'enregistrement de store.

## 2026-10-29 — chamilo-lms : dealerdirect v0.7.2 porté, la fixture complétée au moment de la mesure (docs/plans/chamilo-dealerdirect.md)

**Constat.** chamilo-lms (dev) restait rendue : dealerdirect v0.7.2 non
émulé, et la référence échouait sur la fixture (phpcs charge
`vendor/autoload.php`, dont `public/legacy.php` `require` treize fichiers
absents, puis la classe `Database` d'un répertoire `classmap` absent).
Porter seul aurait fait passer l'entrée de `fallback` à `unavailable`.
Choix de l'utilisateur : les deux.

**La fixture.** Mesuré : les treize fichiers + `database.lib.php`
suffisent ; la référence écrit alors « set to
../../mpdf,../../slevomat/coding-standard » (`vendor/mpdf/mpdf/ruleset.xml`,
une bibliothèque, à la profondeur 3 : la recherche dans tout le projet
propre à 0.7.2). Ces fichiers sont GPL-3.0-or-later : ils ne sont **pas
commités**. `fixtures/corpus/chamilo-lms/UPSTREAM` porte le commit
épinglé et la somme SHA-256 de chaque fichier ; `harness/corpus.sh` les
télécharge (`raw.githubusercontent.com`) au moment du double install,
avant le commit git de la copie, les range en cache après vérification et
les revérifie à chaque usage ; un fichier manquant ou altéré classe
l'entrée `unavailable`, nommée. Quinze fichiers, 903 Ko : `legacy.php`
lui-même (commité jusque-là, il reste dans l'historique) y passe. Le scan
hors ligne n'en a pas besoin (vérifié). `tools/corpus-upstream.sh` écrit
la liste ; `tools/corpus-add.sh` la réécrit au nouveau commit d'un
rafraîchissement. Alternative écartée (proposée par la méta-analyse) : des
fichiers vides à la place — sans réseau ni GPL, et Composer écrit la même
chose, mais la fixture mesurerait un projet qui n'existe pas ; la consigne
était les vrais fichiers au commit épinglé, et deux classes de plus
entrent dans la classmap (comparée à l'octet).

**Le port.** Lu au tag (`1c968e5`, MIT, copie sous
`docs/reference/plugins/phpcodesniffer-composer-installer-v0.7.2/`), diff
contre 1.2.1 : la racine du projet est toujours un chemin de recherche ;
`$alternativePath` comme v1.0–1.2.0 ; `--config-show` par le `phpcs` du
`PATH`. `phpcs_installer::variant` porte les deux différences. En lisant
le Finder que Composer 2.10.3 embarque (symfony/finder v5.4.45), trois
écarts de `find_rulesets`, vrais pour toutes les versions : les
composants à point sont écartés (relativement à chaque racine de
recherche), les liens de répertoire ne sont pas parcourus (une racine qui
est un lien est lue), un `ruleset.xml` lien pendant compte. Invisibles
pour 1.x tant que les standards n'ont ni dossier à point ni lien,
décisifs pour 0.7.2 qui parcourt `.github/`, `node_modules/`… Corrigés
pour toutes les versions (cas 1.2.1 à racine standard au harnais, rouge
sur main).

**Gardes** (rendus nommés, avant toute écriture, aussi sur le lock résolu
d'un `update`) : phpcs 2 avec une version avant v1.2.0 (`./scripts/phpcs`,
non mesuré — v1.0.0 à v1.1.2 ont la même branche) ; phpcs en version de
branche, instable ou aliasé (`isPHPCodeSnifferInstalled('>= 3.0.0')`
suit les contraintes de Composer) ; `extra.phpcodesniffer-search-depth`
autre qu'un entier supérieur à la profondeur minimale (le plugin lève, ou
convertit une chaîne) ; pour 0.7.2, le bin-dir déjà dans le `PATH` après
un autre `phpcs` (`ensureBinDirIsInPath` ne préfixe pas un répertoire
présent : Composer lit alors la conf de cet autre phpcs, mesuré par la
revue) et un `PATH` sans `php`.

**Ce qui reste vrai.** `phpcs` exécute le PHP du projet : quand il plante,
Composer imprime « Failed to set … » sans rien écrire, vivacity écrit la
conf. Imprévisible sans exécuter phpcs, pour toutes les versions ; le
corpus classe ce cas `unavailable` (c'était chamilo). Piste non suivie
ici : exécuter phpcs comme le plugin (vivacity lance déjà `patch` pour
composer-patcher), qui reproduirait l'échec au lieu de le supposer absent.

Méta-analyse (douze failles) et revue indépendante (dix points) : au plan.

Harnais `harness/dealerdirect-07.sh` (35 cas) : vert sous PHP 8.5.12 et
8.4.27 ; contre le binaire de main 5768dc9, sur les 33 cas d’alors : 31 FAIL / 2 OK (`-v` et
v0.7.1, déjà rendus) ; chaque règle du Finder et chaque garde de la revue
vue rouge en la retirant (liens suivis : 13 FAIL, recherche du projet
coupée : 23, `is_file` : 3, filtre des points sur le chemin absolu : 2,
sans filtre : 14, gardes `PATH` et phpcs 2 : 2). `plugin-lines.sh` (118),
`plugins-round3.sh` (52), `allow-plugins.sh` (58), `verbosity.sh` (310)
verts. `cargo test` : 455 tests, 0 échec.

Corpus (`docs/corpus/2026-10-29-chamilo-dealerdirect.md`) : les treize
entrées à dealerdirect rejouées dans les deux modes (chamilo-lms aussi sous
PHP 8.4), les 93 autres reprises du passage précédent : **95/106 natives
en `--no-dev`, 95/106 en dev** (94 avant), 0 diff, 0 unavailable.

## 2026-10-29 — Un banc Windows à part, à la manière de la mesure de Luther (docs/plans/bench-windows.md)

**Constat.** La seule mesure externe de vivacity (Luther Monson,
2026-10-14 : Windows 11, Defender actif, Laravel, vivacity 0.17 dans ePHPm)
n'avait pas d'équivalent rejouable ; `bench.yml` ne tourne que sous Linux,
et le store comme la matérialisation ont changé depuis 0.17.

**Décision : un banc et un workflow dédiés**, `bench/windows-bench.sh` et
`bench-windows.yml` (`workflow_dispatch`), plutôt qu'une ligne
`windows-latest` dans la matrice de `bench.yml`. Raisons : `bench.yml`
tourne à chaque push ou PR qui touche `crates/**`, et une install Composer
`vendor/` supprimé prend des minutes sous Windows (107 s chez Luther) ;
`gate.py` agrège par fixture et scénario, sans OS, et une ligne Windows se
mêlerait à la baseline Linux ; `ci-bench.sh` partage un répertoire entre
les deux outils, lance `composer` par son nom (un `.bat` que hyperfine `-N`
ne lance pas) et ne prouve pas l'égalité des sorties. Généraliser
`ci-bench.sh` aurait fait un script à deux régimes ; refusé.

**Méthode.** Les drapeaux de Luther des deux côtés (`--no-interaction
--no-scripts -q --ignore-platform-reqs`, plugins actifs), plus
`--no-blocking` sur `install` pour la raison de `ci-bench.sh` (2026-09-19) ;
Composer en `php composer.phar`. hyperfine `-N`, une chauffe. Une copie du
projet par outil, renommée au même chemin absolu le temps que son outil
travaille : sur Sylius, rector/extension-installer écrit des chemins
absolus dans `vendor/` (vu au passage macOS, deux copies côte à côte
différaient d'une ligne). Avant les mesures et après, les deux `vendor/`
doivent avoir le même manifeste (sha256 par fichier, cible par lien) et
vivacity ne doit rien avoir délégué, sinon le job échoue sans chiffre ;
les trois arrêts vus rouges. Le résumé relève CPU, Windows, Defender
(`Get-MpComputerStatus`, `Get-MpPreference`), volumes et lecteurs. Deux
dispositions en matrice : `split` (projet sur `D:`, caches sur `C:` : ce
que voit un utilisateur CI, et vivacity copie au lieu de lier) et `same`
(tout sur `C:`, un poste à un disque).

**Non fait.** Pas de gate (le ratio de `bench.yml` bouge de 15–50 %
entre runs identiques sur runner partagé et ne fait que rapporter) ; on ne bascule pas Defender, on relève son
état ; pas d'ePHPm. Passage macOS (M4 Max, `BENCH_RUNS=2`, trois
fixtures) vert : arbres identiques, Laravel `fresh` 3,0 s contre 0,19 s.
Le chemin Windows (cygpath, PowerShell, `rm.exe` sous `-N`, le phar de
setup-php) n'a tourné que sous des doublures sur ce Mac ; les chiffres
Windows viendront du premier run (`bench/M9-windows.md`).

**Correctif après le premier run (run 40927754769, même jour).** Les deux
dispositions se sont arrêtées au premier `--prepare` de hyperfine
(« Command ''true'' terminated with non-zero exit code 1 »), après une
préparation et une preuve Laravel vertes. Cause, lue dans les sources :
le runner a pris **hyperfine 2.0.0** (2026-10-07), qui lance toujours
`--prepare` dans un shell, `cmd.exe /C` sous Windows quand `--shell` n'est
pas donné, même sous `-N` (1.x l'exécutait sans shell) ; et dont le JSON
(schéma 2) range le nom de `-n` dans `name` et les statistiques sous
`summary.time_wall_clock`. Fait : `noop` et `dump-o` restent sous `-N`,
sans préparation ; `fresh` passe par le bash de Git (`--shell`, chemin
absolu), préparation comprise, en 1.x comme en 2.x ; lecture des deux
schémas ; `hyperfine@2.0.0` épinglé dans le workflow. Vu rouge sur le Mac
(1.20.0) : une install qui échoue seulement quand `vendor/` est absent
arrête le bras `fresh`, ce qui prouve aussi que la préparation supprime
bien `vendor/`. Hors de ce correctif, même cause : `bench/classmap-cache.sh`
lit le schéma 1 et son workflow prend hyperfine sans version.

## 2026-10-29 — Le run trois OS de `classmap-cache` (40915941883) est nul pour ses temps ; hyperfine épinglé, lecture des deux schémas

**Constat.** Vert sur les trois OS, mais toutes les cellules de temps
vides et un gain de 0 : hyperfine 2.0.0, installé sans version par
`taiki-e/install-action`, écrit un JSON au schéma 2 (nom de `-n` dans
`name`, statistiques sous `summary.time_wall_clock`) que
`bench/classmap-cache.sh` lisait au schéma 1 sans échouer sur un résultat
absent. Les JSON n'étaient pas téléversés : les temps sont perdus. Ce qui
ne vient pas de hyperfine (un run tracé par bras pour `dump scan`, le
compte et la taille des enregistrements) reste lisible et est consigné
dans M8 comme une indication.

**Fait.** `classmap-cache.sh` et `windows-bench.sh` lisent les deux
schémas et s'arrêtent sur une valeur absente ou non numérique (chaque
valeur lue une fois hors de `$(…)`, où un `exit` n'arrête rien) ;
`hyperfine@2.0.0` épinglé dans `classmap-cache.yml` comme dans
`bench-windows.yml`. Vu : l'ancien script, rejoué sur un JSON au schéma 2,
reproduit exactement les cellules vides et le gain 0 du run, en vert ; le
nouveau lit ce schéma, et s'arrête (exit 1, fichier et nom cités) sur un
schéma inconnu. `ci-bench.sh` et `gate.py` prennent le hyperfine d'apt
(1.x) sur ubuntu : non touchés. Le verdict de M8 (l'action reste
inchangée) tenait à une raison de correction, pas à ces temps : il ne
bouge pas ; la condition « un gain Linux mesuré » reste à remplir en
relançant le workflow.

## 2026-10-29 — Les avertissements du pool à leur place, `secure-http` porté (docs/plans/parity-gaps-026.md)

**Constat.** Trois écarts connus, ni corrigés ni rendus, avant 0.26 :
(1) les avertissements que `PoolBuilder::buildPool` écrit pendant la
construction du pool — la liste d'update qui ne vise rien de verrouillé
(PoolBuilder.php:216), une exigence racine gardée verrouillée (:539, :569)
— sortaient après « Updating dependencies », avec ou sans PHP ; (2) ceux
des filtres de politique (« … could not be fetched … »), jamais mesurés,
aussi ; (3) `secure-http` n'était pas porté : vivacity téléchargeait une URL
`http://` que Composer refuse, avec les identifiants de son origine, et
n'écrivait jamais « Warning: Accessing <hôte> over http… », que quatre
harnais filtraient.

**Lu puis mesuré sur 2.10.3.** Tout ce que `buildPool` écrit sort avant
« Updating dependencies » (Installer.php:534-536), au moment où ça arrive :
liste d'update, puis ce que les dépôts écrivent en chargeant (dépôt
dégradé, avertissement http de la première requête vers un hôte) mêlé aux
exigences racine gardées, puis `PRE_POOL_CREATE` (Flex : la requête de son
index, puis « Restricting… »), puis chaque filtre (avis, listes) et ses
avertissements. Mesuré : `update vendor/zzz` sur un dépôt http →
la ligne de liste, puis l'avertissement ; une `api-url` d'avis en http
→ « Security advisory data could not be fetched … » et le refus de
`secure-http` avant « Updating dependencies ». `secure-http` : refus
(`TransportException`) ou avertissement une fois par hôte et par
`Config` (donc deux fois pour `require`, une par `Composer`), rien hors
ligne (`HttpDownloader` répond avant curl), rien pour un dist servi par le
cache ; la ligne d'un dist sort juste après sa ligne « Downloading », et
après la dernière pour un téléchargement au-delà des 12 tâches de
`HttpDownloader` (`COMPOSER_MAX_PARALLEL_HTTP`) ; Flex lit son index à
`PRE_POOL_CREATE` seulement pour un paquet hors `symfony/symfony` et pas à
sa version verrouillée (PackageFilter.php:91-93), sinon à `POST_UPDATE_CMD`.

**Fait.**

- Le constructeur de pool écrit ses avertissements au moment où il les
  produit (`say!`), plutôt que le port en bloc de 53d5306 : celui-ci les
  aurait tous mis avant (ou tous après) les lignes que les dépôts écrivent
  en chargeant. `Pool::warnings` disparaît ; `create_filtered_pool` écrit
  l'avertissement http de l'index de Flex s'il l'a lu, la notice de Flex,
  puis les avertissements de chaque filtre sitôt passé ; `update` n'écrit
  plus que « Updating dependencies » après.
- `vivacity_core::secure_http` : la condition de `prohibitUrlByConfig`, le
  réglage lu comme `Config::get` (`disable-tls`, `$val !== 'false'`), les
  hôtes avertis par `Config`. Rien n'est lu tant qu'aucune URL n'est non
  sûre : le chemin commun est un test de chaîne, sans verrou.
- `secure-http` vrai : délégation pour ce qui est connu d'avance (dépôt
  `composer` d'une résolution, dists du lock, dists du lock résolu avant
  son écriture, points d'entrée de Flex) ; refus avec le message de
  Composer à la requête d'un dépôt ; le `Fetcher` refuse en dernier
  recours. *Corrigé le 2026-10-30 (entrée suivante)* : plusieurs de ces
  délégations n'étaient pas « avant toute ligne » (dépôt du config.json
  global sous `remove`, dists du lock résolu), l'exemption des paquets
  installés ne s'appliquait jamais, un refus que Composer ne rattrape pas
  était écrit `Error:`, et le réglage lu dans auth.json ou `COMPOSER_AUTH`
  est désormais porté.
- `secure-http` faux : l'avertissement à la requête d'un dépôt (retenu
  avec les lignes du dépôt quand il retient sa sortie), dans la passe des
  lignes « Downloading » avec le plafond de `HttpDownloader`, à la fin
  d'une requête de métadonnées ou après les lignes « Downloading » pour
  une redirection vers un nouvel hôte, et pour Flex à son activation quand
  les arguments de `update`/`remove` lui font lire l'index
  (`PackageResolver::resolve` : un alias, un nom `symfony/*` — mesuré : une
  fois par `Composer`, donc deux pour `remove`), à `PRE_POOL_CREATE` ou au
  début de sa sortie de `POST_UPDATE_CMD`. Une recette de Flex servie
  par un autre hôte non sûr que son point d'entrée est rendue (place non
  reproduite).
- Redirections : au plus 20, `CurlDownloader::$maxRedirects` (lu, non
  exercé).

**Vu rouge puis vert.** `harness/verbosity.sh` : trois cas (liste
d'update, exigence racine gardée, `api-url` d'avis en http) × trois
niveaux, rouges sur c876c38 au niveau normal et sous `-v`.
`harness/secure-http.sh` (nouveau, mandataire local, 32 cas) : 27 rouges
sur c876c38, les 5 autres sont des témoins qui passaient déjà (cache
chaud, `--dry-run`, hors ligne, `remove` à cache chaud). `auth-hosts.sh`
sans son filtre : rouge sur c876c38 (trois cas). `flex-update.sh` : quatre
cas de plus (Flex qui ne lit son index qu'à `POST_UPDATE_CMD` ; à son
activation sur `update symfony/console` et `remove symfony/uid` — vu par
`steps.sh`, dont la queue comparée portait une ligne de trop ;
`secure-http` vrai, rendu). Les filtres de la ligne retirés de
`auth-hosts.sh`, `flex-update.sh`, `flex-recipes.sh`, `plugin-lines.sh`.

**Écarté.** Déléguer tout `secure-http: false` (l'avertissement se
prouvait partout où les bancs l'exercent ; les bancs Flex, qui servent
l'index en http local, seraient tous partis à Composer) ; un drapeau de
comparaison entière dans `steps.sh` (`verbosity.sh` compare déjà tout, sous
trois niveaux).

**Laissé, écrit.** Non vérifiés faute de serveur `https` local : la
redirection d'`https` vers une URL non sûre et la limite de 20. Les
délégations tardives : voir l'entrée du 2026-10-30.
La délégation sur un dépôt refusé est large : un `remove` que Composer
mène à bien sans requête de résolution (les filtres rattrapent le refus,
mesuré) part aussi à Composer — même sortie, à la vitesse de Composer.
L'inventaire des écarts connus restants est dans le plan.

## 2026-10-30 — `secure-http` : neuf défauts de revue de ebd53b1 (docs/plans/parity-gaps-026.md)

**Constat.** Une revue indépendante de ebd53b1 a trouvé neuf défauts, six
reproduits contre Composer 2.10.3 ; le banc en cachait trois (une
délégation n'y était comparée qu'après sa ligne `delegating to`, et sous
`--no-fallback` rien n'interdisait une ligne avant la raison). Le banc
d'abord, puis chaque point vu rouge sur ebd53b1.

**Fait.**

- Banc : `secure-http.sh` exige une stderr vide avant la raison (sauf cas
  `late`, où ces lignes doivent être le début exact de celles de
  Composer), et sous `--no-fallback` la raison seule, ni requête, ni
  écriture du projet, du cache de fichiers ou de dépôts, ni du store — ce
  qui a montré aussi la sonde de plateforme écrite dans le store avant une
  délégation à l'ouverture de la session ; `flex-update.sh` exige
  l'avertissement côté Composer et compare en entier le défaut rendu. 21
  cas de plus (53), 4 de plus pour Flex (34).
- Le réglage lu sur toutes les couches de `Factory` (config.json et
  auth.json globaux, composer.json, auth.json du projet, `COMPOSER_AUTH`
  en dernier), booléens JSON seulement : porté, plus délégué (une valeur
  d'un autre type, que Composer signale — schéma —, est rendue). Un
  `COMPOSER_AUTH` qui n'est pas un objet JSON n'est pas « vide » en 2.10.3
  (`false === $env || '' === $env`) : Composer le signale, rendu au
  démarrage. `disable-tls` vrai : rendu au démarrage pour toute commande,
  `config` comprise (mesuré : la ligne sort dès qu'un composer.json
  existe).
- Les dépôts d'une résolution sont jugés dans `resolution_fallback`, avant
  l'édition de `require`/`remove`, sur la liste fusionnée
  (`MergedConfig::load` : désactivation par nom comprise) ; un dépôt local
  (`file://`) y est lu : les URL de son packages.json, et ses dists quand
  la commande installe. Le config.json et l'auth.json globaux sont gardés
  pour le run (`fetch::global_json`) : le répertoire de cache, la liste
  fusionnée et le réglage les lisent une fois au lieu de plusieurs.
- Un dépôt `file://` dont les métadonnées sont en `http(s)` est lu
  (`MixedTransport`) au lieu d'échouer.
- Un refus qui remonte jusqu'à la session est un refus que Composer ne
  rattrape pas (son encadré) : délégation au lieu de `Error:`, y compris
  depuis la découverte de version de `require`. Une redirection refusée
  sur un dist : délégation, le `vendor/` créé par le run retiré.
- Déjà installé : l'identité de `Transaction` (`installed_as_locked`) sur
  installed.json purgé comme la transaction locale le purge
  (`present_packages`, une seule fonction pour les deux). Le miroir essayé
  d'abord est le dernier préféré (`getUrls`), pour la délégation comme
  pour l'hôte averti.
- Flex : l'avertissement d'activation est retenu après les lignes de
  `Factory` et des plugins (`io::hold_after_factory`, qui ajoute à la
  suite), donc avant celles des variables de plateforme ; décidé avant
  `prepare_update` ; un point d'entrée refusé rendu avant l'édition de
  `remove` ; plus de condition sur le téléchargeur désactivé.

**Choisi : documenter la délégation tardive des dists du lock résolu**
servis par un dépôt distant, plutôt que la rendre précoce : ces dists ne
sont connus que par les métadonnées, donc après leurs requêtes et
l'écriture de leur cache — aucune stratégie d'affichage ne la rend « avant
toute requête ». Pour un dépôt local, connus sans requête : rendu avant
tout (ci-dessus). Retenir toute la sortie de chaque `update` jusqu'à la
fin de la résolution pour cacher le cas distant aurait retardé les lignes
du chemin commun.

**Vu rouge puis vert.** Sur ebd53b1, rouges : `repo-global-remove`,
`update-file-repo`, `require-file-repo` (lignes avant la raison, cache et
store écrits), `meta-default`, `meta-remove-default` (`Error:` natif),
`dist-auth-env`, `dist-auth-json`, `dist-auth-global`,
`dist-auth-json-wins`, `meta-auth-json` (délégués au lieu de natifs),
`dist-auth-env-wins`, `dist-auth-string`, `dist-held`, `dist-mirrors`,
`tls-install`, `tls-global`, `tls-env`, `tls-update`, `tls-remove`,
`tls-dump`, `repo-global-default` (store écrit) ; Flex : `platform-env`,
`remove-platform-env`, `not-required`, `secure-remove`. Tous verts.

**Laissé, écrit.** `disable-tls` posé dans le seul auth.json du projet,
sans URL non sûre : natif, la ligne de TLS manque (le lire au démarrage
ouvrirait un fichier de plus à chaque run du chemin commun ; la règle de
coût l'interdit). L'hôte d'une redirection est celui de reqwest (en
minuscules), pas le `Location` tel qu'écrit (CurlDownloader.php:553-555) :
la politique de redirection de reqwest ne voit pas l'en-tête. Le miroir
comparé tel qu'écrit, sans `ComposerMirror::processUrl`. Délégations
tardives (dists distants, métadonnées distantes en `http`, refus d'un
filtre de politique sous `ignore-unreachable` faux — leurs URL ne sont pas
jugées d'avance : sous le défaut Composer rattrape le refus, et
`verbosity.sh` a vu un cas natif rendu quand la revue les avait ajoutées —,
redirection refusée) et limite de 20 redirections : non vérifiées, faute de serveur
`https` local auquel vivacity fasse confiance.

## 2026-10-30 — `secure-http`, second passage de revue (docs/plans/parity-gaps-026.md)

**Constat.** Le second passage confirme sept points et en trouve trois :
(N1) le contrôle d'un dépôt local lisait tout l'arbre de son
`metadata-url`, et l'analysait avant tout test, même sous `secure-http`
faux — mesuré sur un grand dépôt `file://` (chiffres au rapport, pas ici) ; (N2) les
archives qu'une installation rendue après une redirection refusée avait
mises dans le cache de fichiers changeaient la sortie de Composer (pas de
« Downloading » pour un dist en cache) ; (D1) le README taisait l'exception
de l'auth.json du projet.

**Fait.** Le contrôle d'un dépôt local ne lit rien si `secure-http` ne
peut pas refuser ; sinon son packages.json, puis, quand la commande
installe, les seuls fichiers des paquets que la résolution lit à coup sûr
(exigences racine, paquets verrouillés, arguments de `require`, filtrés
par `available-packages`/`-patterns`) — *corrigé au troisième passage :
ce n'était pas un sous-ensemble de ce que Composer lit (les paquets
verrouillés d'une mise à jour partielle, toutes les versions d'un
fichier).* Les URL
des filtres de politique d'un dépôt local comptent quand
`ignore-unreachable` est faux (connu avant toute ligne). Les archives qu'un
run a mises dans le cache de fichiers sont retirées quand il rend la main.

**Retenir la sortie pour supprimer les délégations tardives : pas fait,
rapporté.** Les octets resteraient identiques et la durée totale aussi ;
mais la première ligne attendrait la fin de ce qui décide. Mesuré
(`update --dry-run`, fixtures, Packagist en direct, cache chaud) : la
première ligne, retenue jusqu'au lock résolu, attendrait toute la résolution
(chiffres au rapport, pas ici) — sur
chaque `update` avec installation sous `secure-http` vrai, puisque
n'importe quel dépôt distant peut servir des dists `http://`. Pour une
redirection pendant les téléchargements, les trois lignes d'en-tête de
l'installation attendraient la fin des téléchargements (la durée du réseau
à froid). Restreindre la retenue aux projets à dépôt local ne vaut que pour
eux, avec le même effet. Décision du mainteneur.

**Vu rouge puis vert.** Sur a1b6684 : `update-file-repo-unrelated` (rendu
au lieu de natif : l'arbre entier lu), `late-transitive` (la marche
trouvait le dist avant la résolution : le cas tardif n'était pas comparé),
`meta-advisory-fatal` (une ligne, le cache et le store écrits avant la
raison). Verts ensuite ; `meta-advisory` (natif) vert des deux côtés.

## 2026-10-30 — `secure-http`, troisième passage de revue (docs/plans/parity-gaps-026.md)

**Constat.** Cinq points, confirmés en exécutant contre Composer `-vvv` :
(F1) le contrôle d'un dépôt local lisait le fichier de chaque paquet
verrouillé et jugeait chaque version — `update acme/l` (acme/x verrouillé,
fixé par Composer, son fichier non lu) et `update` (la 1.0.0 d'acme/x, dist
`http://`, hors de `^2.0`) partaient à Composer, qui les mène à bien ;
(F4) la fermeture des `require` n'était pas suivie, d'où une délégation
tardive évitable ; (F2) les `includes` locaux n'étaient plus lus
(régression) ; (F3) les dists que le lecteur de Flex met dans le cache de
fichiers échappaient au retrait, et un retrait raté était avalé ; (F5)
`allow_ssl_downgrade` n'était pas lu.

**Fait.** Le contrôle local suit `PoolBuilder::buildPool` : départ des
exigences racine (sans celles que `remove` retire, avec celles que
`require` ajoute) ; en mise à jour partielle (liste et lock), les paquets
verrouillés hors de la liste — élargie le long des `require` du lock pour
`-w`/`-W`, sans les exigences racine pour `-w` — sont fixés : fichiers non
lus, leurs propres `require` marqués à charger ; puis, nom par nom, les
versions qu'admettent la contrainte et les stabilités (`minimum-stability`,
drapeaux racine), leur dist jugé, leurs `require` suivis — sur le
packages.json et les `includes` d'un dépôt complet, ou les fichiers par
paquet d'un `metadata-url` local (`~dev` quand le dev est admis). Restent
plus larges que le téléchargement : les versions chargées que le solveur
ne prend pas, et le mode transitif approché par le lock (Composer déverrouille
pendant le chargement — *corrigé au quatrième passage : l'approximation
pouvait aussi être plus étroite ; le déverrouillage pendant le chargement
est suivi depuis*). Les archives mises dans le cache de fichiers par
ce run, d'où qu'elles viennent, sont notées (un canal, sans verrou) et
retirées avant toute délégation ; une qui ne peut l'être arrête le run sur
cette erreur — choisi plutôt qu'une délégation dont la sortie différerait
sans le dire. `allow_ssl_downgrade` (=== vrai) : rendu avant tout, plutôt
que porté — le réécrire suppose de suivre aussi le retour en `https` sur
une empreinte `sha256` qui ne correspond pas (ComposerRepository.php:
1765-1768), et l'option est rare.

**Vu rouge (fb31ab6) puis vert.** `partial-locked-http`,
`full-outside-constraint` (rendus au lieu de natifs), `transitive-file-repo`
(tardif : deux lignes, cache et store écrits avant la raison),
`satis-includes` (idem), `ssl-downgrade` (natif au lieu de rendu). Le
retrait des archives (F3) n'est pas exerçable par un banc (il faut une
redirection `https`) : un test unitaire vérifie que le lecteur commun
(`dist_bytes_of`, celui de Flex aussi) note l'archive écrite.

**Laissé.** Le filtre des lignes « Downloading » de `flex-update.sh` reste :
le banc partage le cache de fichiers de Composer entre les deux côtés
(pour ne pas retélécharger ~150 archives de GitHub à chaque passage), donc
ces lignes dépendent de l'ordre des côtés ; le resserrer demanderait un
cache par côté.

## 2026-10-30 — `secure-http`, quatrième passage de revue (docs/plans/parity-gaps-026.md)

**Constat.** Aucune rupture de contrat ; six points, chacun une délégation
tardive évitable (deux lignes avant la raison) ou un écart du banc :
(G1) les drapeaux de stabilité implicites (`dev-main`, l'alias racine
`dev-main as 1.0.0` — `RootPackageLoader::extractStabilityFlags`) et les
alias (de branche, racine) n'étaient pas comptés : `~dev` non lu, une
version aliasée jamais jugée ; (G2) un paquet figé par une mise à jour
partielle, verrouillé à un dist `http://` et pas installé, se voyait après
la résolution ; (G3) l'élargissement `-w`/`-W` par les liens du vieux lock
pouvait être plus étroit que Composer, qui déverrouille une dépendance
verrouillée quand une nouvelle version chargée l'exige ; (G4) vitesse —
mesuré par le relecteur sur un grand graphe local : `update` nettement
plus lent sous `secure-http` vrai, chaque version rejugée
sous chaque contrainte et les listes de versions copiées à chaque tour ;
(G5) un retrait d'archive raté dans l'installeur laissait le `vendor/` créé
et sortait en 100, contre 1 dans `fallback::hand_over` ; (G6) aucun cas
`late` vivant, et la reprise des archives de Flex sans cas.

**Fait.** Drapeaux implicites (une contrainte qui nomme une version
instable) et alias (de branche par `branch_alias`, racine par
`<version> as <alias>`) : la version et ses alias sont confrontés à la
contrainte. Les paquets figés d'une mise à jour partielle sont jugés sur le
lock (`installed_as_locked`, `insecure_dist_reason`) quand il est en
mémoire (`remove`) ou lu pour le contrôle local. Le contrôle déverrouille
ce qu'une version chargée exige sous `-w`/`-W` (pas une exigence racine
pour `-w`). Une version n'est jugée qu'une fois, les listes empruntées, et
les fichiers que le contrôle lit sont gardés analysés pour le dépôt qui les
relit ensuite (`fetch_cached`, `prefetch`) : plus de seconde analyse —
mesuré sur le banc du relecteur : `update` sous `secure-http` vrai au
niveau de faux (chiffres au rapport, pas ici). Le
retrait raté : `vendor/` d'abord rendu, puis `Error:` et sortie 1, comme
`hand_over`. Banc : un cas `late` vivant (`late-path-dependency`, un paquet
`path` qui amène un dist `http://` d'un dépôt local) ; un cas Flex qui
appelle vraiment Composer, cache de fichiers vide par côté, dist de psr/log
local (`dist-then-recipe`).

**Laissé.** Un paquet figé non installé à dist `http://` dans une mise à
jour partielle sans dépôt local et hors `remove` : le lock n'est pas lu
d'avance (une lecture de plus sur le chemin commun) — tardif. Le retrait
raté n'est pas exerçable (il faut une redirection `https` et un fichier
qu'on ne peut retirer).

**Vu rouge (f4de2f8) puis vert.** `dev-implicit-flag`, `dev-branch-alias`,
`dev-root-alias`, `partial-fixed-missing`, `transitive-unlock`.
`late-path-dependency` passe des deux côtés (comportement inchangé) : sans
`late`, ou avec un préfixe faussé, il échoue — la comparaison est vivante.
`dist-then-recipe` : rouge sur fb31ab6 (avant la reprise centrale, qui est
dans 83a60b8), vert sur f4de2f8 et après.

## 2026-10-30 — Retenir la sortie quand une délégation `secure-http` peut encore venir (docs/plans/parity-gaps-026.md)

**Décision du mainteneur** (option 2, déclencheur étroit), après le
rapport du second passage : retenir la sortie seulement quand
`secure-http` est vrai et que la configuration fusionnée contient un
dépôt `composer` distant autre que packagist.org — lui seul peut servir
des métadonnées ou des dists en `http` que seuls ses fichiers révèlent.
Pourquoi étroit : les faits du relecteur (Composer ne réécrit pas un dist
`http://` ; GitHub et Bitbucket en `https` en dur ; 209 685 dists de
Packagist, tous en `https`) font d'un projet Packagist seul un cas où rien
ne peut arriver tard, et la retenue y retarderait la première ligne pour
rien.

**Fait.** Une retenue de sortie distincte de celle de `-v`
(`io::hold_output` : stdout et stderr dans l'ordre ; relâchée telle quelle,
dans la retenue `-v` si elle est active ; abandonnée à toute délégation,
`--no-fallback` compris — la retenue `-v`, elle, s'écrit quand le run
s'arrête). `update`/`require`/`remove` : de « Loading… » au pool construit ;
le pool construit, ses dists sont jugés (pas seulement ceux que le solveur
prendra : plus large, une délégation de trop au pire), puis la sortie part —
la retenue ne couvre que le chargement, pas la résolution (l'étape moins
chère du relecteur). `install` : de l'en-tête à la fin des téléchargements,
sous le même déclencheur ou quand un dist du lock est hors des hôtes sûrs
(api.github.com, codeload.github.com, bitbucket.org, gitlab.com) — connu
avant l'en-tête, donc compté ici ; pour une résolution, les dists de
Packagist ne sont connus qu'au chargement, trop tard pour retenir
« Loading… » : un projet Packagist seul à dist `http://` reste tardif.
Les scripts (`--run-scripts`) relâchent la sortie avant de partir.

**Mesuré.** Première ligne et durée totale des projets Packagist seuls
inchangées, durée totale d'un run déclenché inchangée, première ligne d'un
run déclenché à la fin du chargement — chiffres au rapport, pas ici (règle
du mainteneur : aucun chiffre non approuvé dans le dépôt). *Corrigé au
cinquième passage (entrée suivante) : la retenue commençait trop tard,
le pool était jugé sans installation, et la garde par hôte de `install`
retenait des projets Packagist seuls.*

**Vu rouge (ff98df1) puis vert.** Traces : `hold-remote-https` (dépôt
distant `https`, retenu), `hold-install-other-host` (retenu) ; retenue
imposée : `hold-native`, `-v`, `-q`, `hold-install`, `-v`,
`hold-install-dists` (pas de marqueur avant), `hold-handover` (deux lignes
avant la raison avant, rien après). `hold-packagist-only`,
`hold-install-packagist` : vus rouges sur un déclencheur faussé exprès
(exclusion de packagist.org, liste d'hôtes vidée), verts sinon ;
`hold-remote-https-off` (`secure-http` faux) est un témoin, vert avant
comme après. `verbosity.sh`, `steps.sh`, `flex-update.sh` passés
aussi avec la retenue imposée partout.

**Laissé.** Les chemins tardifs distants eux-mêmes (un dépôt `https` qui
sert des dists ou des métadonnées `http`, une redirection) ne sont pas
exerçables : pas de serveur `https` local auquel vivacity fasse confiance.
Vérifié à la place : le déclencheur sur un dépôt `https` injoignable, et la
retenue imposée sur les équivalents locaux. Tardifs encore : `require` et
`remove` (leurs lignes et l'édition de composer.json précèdent la
retenue), l'installation qui suit un `update` (le lock est écrit avant),
le miroir préféré d'un paquet du pool.

## 2026-10-30 — Cinquième revue : la retenue plus tôt, sans garde par hôte ; les fonctions non reproduites rendues (docs/plans/parity-gaps-026.md)

**Constat.** (F1) La retenue commençait après l'événement `COMMAND` et la
ligne de l'install sans lock, et après l'édition de `require`/`remove` :
une délégation au pool réimprimait ces lignes. (F2) Le pool était jugé
sous `--no-install`/`--dry-run`, où Composer ne télécharge rien et finit en
0. (F3) La garde par hôte d'`install` retenait des projets Packagist seuls
(downloads.wordpress.org, git.drupalcode.org sont servis par Packagist), et
le déclencheur coûtait une `MergedConfig` et un passage sur le lock à
chaque `install`. (F4) Un paquet figé non installé à dist `http://` n'était
jugé d'avance qu'avec un dépôt local ou sous `remove`. (F5) Une panique
pendant la retenue écrivait les lignes retenues après son message. (F7) Des
chiffres de banc et de délai étaient entrés dans le dépôt sans
approbation, et le CHANGELOG promettait « rien d'écrit » là où ce n'était
pas vrai. Hors de ce delta : `update` sur un dépôt `package` (et les autres
« not supported … yet ») sortait en 1 au lieu de rendre.

**Fait.**

- Retenue : `update` dès le manifeste chargé dans la résolution ;
  `require`/`remove` dès `resolution_fallback` passé, avant toute ligne de
  la commande et l'édition. Une délégation au pool n'écrit rien à l'écran ;
  composer.json, édité, est rétabli à l'octet — *pas* « rien d'écrit » :
  différer l'écriture jusqu'au pool demanderait de résoudre
  `require`/`remove` sur un manifeste en mémoire de bout en bout (choix :
  non fait, dit). Les lignes que `Factory` retient (`io::hold`) restent en
  tête, hors de la sortie retenue : écrites d'abord, par la raison d'un run
  qui s'arrête, ou abandonnées par la délégation, comme sans retenue.
- Le pool n'est jugé que si la commande installe ; la sortie est relâchée
  au pool dans tous les cas. Un miroir de dist préféré déclaré par le
  packages.json d'un dépôt compte (Composer l'essaie d'abord).
- Plus de garde par hôte : `install` retient sous le seul déclencheur. Le
  déclencheur est un test sur le JSON en mémoire (`repositories` de
  composer.json, du config.json global gardé pour le run), plus large que la
  liste fusionnée ; le réglage n'est lu que si un tel dépôt est là. Un dist
  `http://` du lock est toujours rendu avant toute sortie
  (`insecure_dist_reason`, cas `dist-default`).
- Paquets figés : `update` juge sur le lock que la session a lu (après la
  sonde de plateforme et, pour un projet Flex, après la lecture de son
  index — la même requête que Composer), `require` sur le lock lu avant son
  édition (la lecture qu'il faisait plus tard, avancée), `remove` sur le
  sien ; un test de chaîne par dist verrouillé d'abord.
- Panique : un crochet chaîné, sur le seul fil qui retient, écrit la
  retenue `-v`, puis les lignes de `Factory`, puis la sortie retenue — l'ordre
  de `release_output` — avant le message, verrous seulement essayés. Choisi
  plutôt que de les jeter : ce sont des lignes que le run avait écrites
  avant la panique.
- Rendus avant tout (`resolution_fallback`, sur la configuration fusionnée,
  sauf `remove --no-update` qui n'ouvre aucun dépôt) : types de dépôt autres
  que `composer`/`path`, `only`/`exclude`/`canonical`, schéma d'URL non lu,
  dépôt sans URL ; au démarrage : `COMPOSER=` autre fichier, le mode
  minimal-changes (option — désormais acceptée par `update` —,
  `COMPOSER_MINIMAL_CHANGES`, `update-with-minimal-changes` de composer.json
  ou, à défaut, du config.json global, en vérité PHP : `Config::get` laisse
  la clé brute, la chaîne "false" est vraie) ; `update lock|nothing|mirrors`.
  Ce que seul le chargement révèle (protocole v1, `api-url` de filtre,
  liste de politique personnalisée) : délégation tardive au lieu de sortir
  en 1 (retenue sous le déclencheur).
- Docs : tout chiffre de banc ou de délai non approuvé retiré du dépôt
  (gardé au rapport).

**Vu rouge (bd879c9) puis vert.** `hold-require-handover`,
`hold-remove-handover`, `hold-lockless-handover` (lignes avant la raison) ;
`hold-update-no-install`, `-dry-run`, `hold-require-no-install`,
`-dry-run`, `hold-remove-no-install`, `-dry-run` (rendus à tort) ;
`hold-install-other-host` (retenu à tort) ; `partial-fixed-no-local`
(tardif) ; `repo-type-package`, `repo-filter-only`, `alt-manifest`,
`update-lock`, `minimal-changes-env`, `minimal-changes-require`,
`minimal-changes-string`, `minimal-changes-option` (sortie 1 ou 2, ou
natif sans le mode) ; `repo-type-package-no-update` (rendu à tort).
Verts des deux côtés (comportement déjà juste, gardés comme garde-fous) :
`hold-require`, `hold-remove`, `hold-update-install`, `hold-vv`,
`hold-vvv`, les cas Flex et bamarni sous retenue.

**Laissé.** Le message d'une panique n'est pas exercé par un banc. La
retenue ne couvre pas une redirection refusée pendant l'installation qui
suit un `update` (le lock est écrit) ni hors déclencheur. `remove` rend
encore la main après « has been updated » quand son second chargement des
plugins refuse (indépendant de `secure-http`, inventorié).


## 2026-10-30 — Sixième revue : composer.json en mémoire jusqu'au pool ; minimal-changes sur toutes les couches ; `--no-update` natif (docs/plans/parity-gaps-026.md)

**Constat.** (W) Sous la retenue, `require`/`remove` écrivaient composer.json
avant le pool, puis le remettaient à l'octet pour une délégation : le
contrat du mainteneur est plus strict (rien d'écrit avant la délégation),
et la raison `--no-fallback` (« nothing was written ») était fausse.
(H1) `update-with-minimal-changes` posé dans auth.json (global ou du
projet) ou `COMPOSER_AUTH` restait natif ; Composer le lit (`Factory`
fusionne ces couches sous `config`). (H2) `require <paquet>:<version>
--no-update` était rendu sur un dépôt `vcs`/`package`/`artifact` ou à
filtres (régression de la cinquième revue) ; `require`/`remove
--no-update` étaient rendus au démarrage pour minimal-changes, qu'ils ne
lisent pas. (H4) Le déclencheur résolvait `composer_home()` à chaque
commande et deux ou trois fois par run ; `require --no-update` lisait le
lock. (H5) « à chaque niveau » de verbosité et « nothing was written »
disaient plus que vrai.

**Fait.**

- `io::write_held` : sous la retenue, l'édition de composer.json (et le
  fichier que `require` crée) est gardée en mémoire ; `io::read_held` la
  rend à tous les lecteurs entre l'édition et le pool (`prepare_update` de
  la découverte et de la résolution, `resolve_and_lock`, le second
  chargement des plugins de `require`, le manifeste relu de `remove`,
  `JsonConfigSource` y compris sa réécriture de repli). Écrite au
  relâchement de la sortie (pool jugé, ou fin du run sans délégation :
  Composer laisse alors le fichier édité, `--fixed` refusé sur un fichier
  créé par exemple) ; oubliée par une délégation. Hors retenue : écrite à
  l'instant, comme avant. Un fichier non inscriptible est écrit tout de
  suite (l'erreur d'aujourd'hui) ; une écriture différée qui échoue arrête
  la résolution avant le lock.
- Retour en arrière : `revert` oublie l'édition en attente puis écrit
  l'original comme Composer (ou retire le fichier créé et le lock) ;
  `undo_edit` (délégation) n'écrit rien si rien n'a été écrit — édition en
  attente, `--dry-run`, aucun argument de `remove` qui nomme un lien. La
  délégation de la découverte d'un fichier créé passe par `undo_edit` (elle
  supprimait un lock existant et écrivait une ligne avant de rendre).
- Raison `--no-fallback` : « nothing was written to the project », ou ce qui
  l'a été (composer.json édité puis remis, ou créé puis retiré). Le cache de
  dépôts et la sonde du store, hors du projet, ne sont pas couverts par la
  phrase.
- Minimal-changes : les six couches de `Factory`, la dernière gagne ; toute
  valeur non booléenne rend (composer.json échoue à la validation, les
  autres couches avertissent). auth.json du projet par la copie gardée pour
  le run, que `fetch::Auth::load` relit désormais aussi : pas de lecture de
  plus. Ignoré pour `require`/`remove --no-update`.
- `require --no-update` dont chaque argument porte une contrainte : ni
  contrôle de dépôts, ni session de découverte ; seulement le test, en
  mémoire, de ce que `requireComposer()` refuserait en instanciant les
  dépôts (type non enregistré, pas de type, pas d'`url`) — rendu alors.
- `composer_home()` et le déclencheur gardés pour le run (une évaluation
  par commande, install qui suit compris) ; ni lock ni sauvegarde du lock
  lus par `require --no-update`.

**Vu rouge (02ef0a7) puis vert.** `hold-require-handover`,
`hold-remove-handover`, `hold-remove-nomatch-handover` (composer.json
réécrit), `hold-require-created-handover` (fichier créé puis retiré) ;
`minimal-changes-auth-env`, `-auth-json`, `-auth-global`, `-auth-string`
(natifs), `minimal-changes-auth-json-wins` (rendu à tort) ;
`minimal-changes-require-no-update`, `minimal-changes-remove-no-update`,
`repo-type-package-require-no-update` (rendus à tort) ;
`repo-path-no-url-require-no-update` (sortie 1 au lieu de rendre). Le
banc vérifie désormais, pour toute délégation sans ligne avant la raison,
que ni composer.json ni le répertoire du projet `--no-fallback` n'ont été
écrits (dates). Témoins verts des deux côtés : `hold-require-created-fixed`,
`hold-require-unsolvable`.

**Laissé.** H3 et S sortis de cette revue par le mainteneur, inventoriés
(32a, 32b). Le cache de dépôts et la sonde du store restent écrits par une
délégation décidée au chargement.

## 2026-10-30 — Septième revue : `--no-update` valide encore les couches ; dépôt `package` sans définition (docs/plans/parity-gaps-026.md)

**Constat.** `require`/`remove --no-update` sortaient de `Cli::minimal_changes`
avant le contrôle de type : une valeur non booléenne de
`update-with-minimal-changes` (erreur de Composer dans composer.json, sortie
1 ; avertissement dans le config.json global ou un auth.json) restait
native, composer.json édité. Un dépôt `package` sans définition de paquet
(`{"type": "package"}`, `"package": "x"`) arrête Composer à
`requireComposer()` ; `require … --no-update` l'éditait et sortait en 0. La
vidange d'une panique écrivait l'édition retenue sans la compter comme
écrite ; l'erreur d'une écriture différée n'était lue que sur une
résolution réussie.

**Fait.** Le mode reste ignoré sous `--no-update`, le contrôle de type des
six couches non : une valeur non booléenne rend. Un dépôt `package` dont
`package` n'est pas un objet ou une liste non vide d'objets à `name` et
`version` chaînes rend (au moindre doute, sans réécrire le schéma). La
vidange compte l'écriture ; l'erreur d'écriture différée est lue dès le
retour de la résolution, avant son issue (Composer échoue à l'écriture,
avant de résoudre).

**Vu rouge (52ff54e) puis vert.** `minimal-changes-require-no-update-string`,
`-null`, `-global`, `minimal-changes-remove-no-update-zero`,
`repo-package-no-key-require-no-update`,
`repo-package-string-require-no-update` ; la vidange par le test
`io::levels_and_symfony_parsing`. L'erreur d'écriture différée n'est pas
exercée (relue).

**Laissé** (antérieurs à la branche, inventoriés 32c, 32d) : une URL de dépôt
`composer` sans hôte, que Composer refuse dans le constructeur ;
l'avertissement « has a packagist key ».

## 2026-10-30 — Inventaire fermé avant 0.26 : signaux de `require`, définitions de dépôt, dépôts locaux, second chargement de `remove` (docs/plans/parity-gaps-026.md)

**Constat.** (32b) Composer rétablit composer.json sur SIGINT/SIGTERM/SIGHUP
pendant `require` ; vivacity laissait le fichier édité. (32c/32d) Une
définition de dépôt que la création du `Composer` refuse (URL sans hôte,
port invalide, `filter` mal formé…) ou signale (`packagist: false`) passait
en natif. (32a) Le protocole v1, une liste personnalisée et l'`api-url` de
filtre d'un dépôt local étaient rendus après « Loading… ». (18g) Le second
chargement des plugins de `remove` était rendu après « has been updated ».
(18h) Supposé : l'index de Flex lu avant le contrôle des paquets figés.

**Fait.**

- Signaux : `vivacity_core::signals`, `sigaction` de libc (déjà une
  dépendance de vivacity-core sur macOS et Linux) — aucune dépendance
  ajoutée. signal-hook-registry, déjà dans l'arbre par tokio, a été écarté :
  retirer son action laisse le signal ignoré au lieu de rétablir l'action
  par défaut. Gestionnaire minimal (marque + octet dans un tube), un fil
  fait le retour en arrière ; une porte tenue pendant chaque écriture de
  composer.json ou du lock, prise pour de bon par ce fil ; les autres fils
  s'arrêtent à leur ligne suivante ; mort par le signal comme
  `exitWithLastSignal`. Suspendu pendant les opérations sur les paquets
  (le gestionnaire d'`InstallationManager` passe devant chez Composer),
  retiré avant un Composer enfant, posé par le binaire seul (un
  embarquement garde ses signaux). Aucun coût pour les autres commandes
  qu'un chargement atomique par ligne écrite.
- Définitions de dépôt : contrôle au démarrage sur le JSON en mémoire
  (composer.json, config.json global gardé pour le run), rendu sur ce que
  `createRepos`, `createRepository` et les constructeurs refusent ou
  signalent ; `parse_url` reproduit seulement là où c'est bon marché, un
  doute rend.
- Dépôts locaux : packages.json lu une fois (partagé avec le contrôle
  `secure-http` et la session), v1 et liste personnalisée à la résolution,
  liste personnalisée et `api-url` de `malware` pour `install`. Rien sur un
  chemin Packagist seul.
- `remove` : second chargement jugé sur le manifeste édité en mémoire, avant
  toute ligne, et réutilisé.
- 18h : vérifié sans objet (`flex::activation_lines` ne fait pas de
  requête ; l'index est lu après le contrôle).

**Vu rouge (9f1117a) puis vert.** `harness/require-signals.sh` (7 cas :
`int`, `term`, `hup`, `lock`, `created`, `held`, `verbose`) ;
`harness/remove-plugin-load.sh` (2 cas) ; secure-http.sh : `repo-url-no-host`,
`-update`, `-dump`, `repo-url-bad-port`, `repo-filter-int-key`,
`repo-only-string`, `repo-packagist-key`, `-global`, `local-v1`,
`local-custom-list`, `-install`, `local-api-url-install`. Tests unitaires des
règles d'URL et de `filter`.

**Laissé.** Windows (Composer rétablit sur Ctrl-C) ; le `runCleanup`
d'`InstallationManager` ; un Composer enfant attendu sans relais des
signaux (antérieur) ; l'`api-url` d'un dépôt local à la résolution et les
dépôts distants (tardifs) ; l'avertissement « packagist key » des dépôts de
COMPOSER_HOME/composer.json.

## 2026-10-30 — `signals.rs` : pourquoi du `unsafe` (libc brute plutôt que signal-hook), et ses invariants (docs/plans/parity-gaps-026.md)

**Pourquoi le code sûr ne suffit pas.** La bibliothèque standard n'expose ni
`sigaction`, ni `kill`, ni `pipe2`, ni l'adresse d'`errno` : poser un
gestionnaire, rétablir l'action précédente, renvoyer un signal à soi-même
(`exitWithLastSignal`) ou à un enfant passent par la FFI de libc (déjà une
dépendance de vivacity-core sur macOS et Linux). Aucune dépendance ajoutée.

**Invariants.**

- **Le gestionnaire** (`on_signal`) ne fait que : sauver puis rétablir
  `errno` ; `getpid` ; charger et stocker des atomiques (sans verrou) ;
  `write` d'un octet pris sur sa pile vers l'extrémité d'écriture du tube,
  non bloquante. Rien d'autre : ni allocation, ni verrou, ni E/S Rust.
  Tout le reste (retour en arrière, relais à l'enfant, mort par le signal)
  se fait dans un fil ordinaire qui lit le tube.
- **`WRITES`** n'est tenu que par une `WriteGate`, le temps d'une écriture de
  composer.json ou du lock (l'édition, le lock, la mise à jour des
  contraintes, le retour en arrière, les recettes de Flex), et aucune ligne
  n'est écrite sous une `WriteGate` (un fil arrêté à son `checkpoint` la
  tiendrait). Le fil du signal la prend — en essayant pendant cinq secondes
  au plus — et ne la rend jamais : une écriture en cours finit, aucune ne
  suit. Passé ce délai (un fil qui la tiendrait bloqué ailleurs), le retour
  en arrière se fait sans elle, et une écriture alors en cours pourrait
  finir après lui : le prix de ne jamais rester suspendu.
- **Signal pris avant le retrait** : le gestionnaire marque le run
  (`ABORTING`) ; si les gestionnaires sont retirés avant que le fil lise
  l'octet, `release` garde le retour en arrière et le fil le fait quand
  même — le signal n'est ni perdu ni laissé à mi-chemin.
- **Mort par le signal** : action par défaut rétablie puis `raise(3)` (le
  signal va au fil qui l'envoie, pas à un fil qui ne tournerait plus), puis
  `exit(128 + n)` s'il survit.
- **`fork`/`exec`** : un enfant hérite des gestionnaires entre `fork` et
  `exec` ; le gestionnaire compare `getpid` au processus qui les a posés et
  ne fait rien d'autre dans un enfant.
- **CLOEXEC** : `pipe2(O_CLOEXEC)` sous Linux ; macOS n'a pas `pipe2`,
  `pipe` puis `FD_CLOEXEC` (un processus lancé par un autre fil entre les
  deux hérite des descripteurs, sans jamais y écrire).
- **Le tube n'est jamais fermé** : un seul tube et un seul fil pour le
  processus, créés au premier besoin (la boucle de lecture traite une erreur
  par défense : la mort par le signal si un signal était déjà pris). Un gestionnaire qui a lu le
  descripteur juste avant le retrait n'écrit jamais dans un descripteur
  réutilisé ; un octet arrivé après le retrait est ignoré (mode « aucun »).
  Coût : deux descripteurs jusqu'à la sortie, seulement pour un run qui a
  posé des gestionnaires.

**Ce que signal-hook ferait, et ne ferait pas ici** (relevé par la revue).
`Signals`, `flag::register` et `emulate_default_handler` remplaceraient le
tube et le `sigaction`. Mais : noter `SUSPENDED` au moment du signal, dans le
gestionnaire, demande `low_level::register` (non sûr de toute façon) ; le
registre ne rétablit jamais la disposition précédente — après le retrait,
un répartiteur qui ne fait rien reste à la place de SIG_DFL ou SIG_IGN (un
signal ignoré, ou plus mortel, pour la suite du processus et pour un
programme qui embarque vivacity) ; relayer à un enfant demande `kill` de
toute façon ; et rien n'y remplace `WriteGate`, `checkpoint` ni `suspend`.
signal-hook-registry, déjà dans l'arbre par tokio, a le même défaut de
retrait.

**Preuve.** miri ne sait pas exécuter ces appels système : la preuve est
`harness/require-signals.sh`, qui compare à Composer, octet pour octet,
code de sortie (mort par le signal), sortie et fichiers, pour les trois
signaux, un lock, un composer.json créé, la sortie retenue, `-v`, et une
commande rendue à Composer dont vivacity seul reçoit le signal.

## 2026-10-30 — Huitième revue : relais des signaux à un Composer enfant, portes de Flex, durcissements (docs/plans/parity-gaps-026.md)

**Constat.** Une commande rendue à Composer laissait vivacity en parent nu :
un SIGTERM envoyé à vivacity seul (`timeout`, `docker stop`, systemd
`KillMode=process`) le tuait et laissait l'enfant PHP finir seul,
composer.json édité ; un enfant tué par un signal donnait la sortie 1. Les
écritures de composer.json et du lock par les recettes de Flex
contournaient la porte. Le fil du signal pouvait attendre la porte sans
fin ; une erreur de lecture du tube abandonnait la mort par le signal ; le
tube était fermé au retrait (course avec un gestionnaire en vol), `errno`
non rétabli, l'écriture bloquante. `parse_url` acceptait des ports que
vivacity refusait. 18g comparait les clés sans la casse.

**Fait.** Dès avant le lancement d'un Composer enfant (toute délégation)
et jusqu'à sa fin, les trois signaux lui sont relayés (`kill` ; un signal
venu pendant le lancement l'est une fois son pid connu ; l'enfant est
attendu sans être récolté — `waitid(WNOWAIT)` — tant que le relais est
posé, pour ne jamais viser un pid réutilisé) ; il les traite comme Composer
seul puis vivacity meurt du même signal (`128 + n` pour la bibliothèque, et
pour un signal qui produit un core : le core de vivacity n'est pas celui de
Composer). Choix :
relayer plutôt qu'ignorer — ignorer laisserait l'enfant sourd à un signal
adressé au seul vivacity. Un signal envoyé au groupe atteint l'enfant deux
fois : ses gestionnaires (et l'action par défaut) le terminent au premier,
mais la ligne `-vvv` « Received SIG…, aborting when child process is done »
du `ProcessExecutor` de Composer s'écrit alors deux fois (corrigé en partie
le même jour, entrée suivante). Rien n'est posé hors
délégation. Recettes de Flex derrière la porte ; porte attendue cinq
secondes au plus ; tube unique jamais fermé, non bloquant, `pipe2` sous
Linux ; `errno` sauvé ; `raise(3)` pour mourir ; un signal pris juste avant
le retrait des gestionnaires est traité quand même. Ports lus comme `strtol` les lit (`80a`, `+80`,
` 80`), hôtes entre crochets comme PHP. 18g : clés exactes. Sans `pcntl`
dans le PHP sondé par ce run, pas de retour en arrière (Composer n'a pas de
gestionnaire) ; sans `posix`, sortie `128 + n`, pendant les opérations sur
les paquets aussi (le gestionnaire d'`InstallationManager` finit par le
même `exitWithLastSignal`).

**Vu rouge (23ff273) puis vert.** `handed-term`, `handed-int`
(require-signals.sh) ; test unitaire des URL (`80a`, `[::1]x`…). `hold-vv`
stabilisé (`COMPOSER_TEST_SUITE`, le ramasse-miettes tiré au sort de
Composer à `-vv`).

**Laissé.** `runCleanup` d'`InstallationManager` (téléchargements et
extractions partiels retirés) non reproduit ; avant la sonde de plateforme
du run, `pcntl` supposé présent, et la sonde dit « extension chargée » là
où seld teste `function_exists` (des fonctions retirées par
`disable_functions` passent pour présentes) ; sans `pcntl` sous `nohup`,
Composer ignore SIGHUP (disposition héritée) quand vivacity, qui a posé son
gestionnaire, meurt ; Windows.

## 2026-10-30 — Neuvième revue : une ignorance héritée respectée, signal de terminal non relayé sous Linux, relais sans course (docs/plans/parity-gaps-026.md)

**Constat.** Le relais posait son gestionnaire avant de lancer l'enfant :
un signal que l'appelant ignorait (`nohup`, `trap '' HUP`, SIGINT d'une
tâche de fond sans contrôle des tâches) redevenait, à `exec`, l'action par
défaut chez l'enfant — toute délégation mourait d'un signal que le même
Composer lancé directement ignore. Un signal de terminal arrivait deux fois
à l'enfant (la ligne `-vvv` du `ProcessExecutor` deux fois). Un signal
venu pendant le lancement pouvait se perdre entre le fil et le fil
principal ; un `kill` restait possible juste après la récolte. `--run-scripts`
attendait son `composer run-script` sans relais (sortie 1 sur un signal).
La sonde construisait l'ensemble de toutes les extensions pour deux noms.

**Fait.**
- Relais : un signal ignoré par l'appelant n'est pas touché avant le
  lancement (l'enfant hérite de l'ignorance), puis le gestionnaire de relais
  le prend pour passer à l'enfant un signal adressé au seul vivacity. Le
  mode de `require` passe outre une ignorance, comme `pcntl_signal` dans
  seld (vérifié dans SignalHandler.php).
- `SA_SIGINFO` : sous Linux, un signal de terminal (`SI_KERNEL`) n'est pas
  relayé une fois l'enfant lancé — il l'a déjà ; avant, il passe par le
  signal en attente comme un autre (au pire, la double remise documentée ;
  dixième revue, avec `SeqCst` sur la remise `CHILD`/`PENDING`). Sous macOS un tel signal porte le pid du
  meneur de session et aucun code propre (relevé : `si_pid` du processus
  `script` qui tient le terminal, `si_code` 0, comme pour un `kill`) : il
  ne se distingue pas d'un `kill` et reste relayé. Un `kill` adressé au
  groupe par un autre processus, sur les deux systèmes, aussi. Résidu :
  dans ces deux cas, la ligne `-vvv` du `ProcessExecutor` deux fois.
- Signal pendant le lancement : le fil relit le pid après avoir noté le
  signal et le relaie lui-même s'il est arrivé ; plusieurs signaux dans ce
  laps (des microsecondes) se réduisent au dernier.
- Pid réutilisé : « encore en relais ? alors `kill` » et « retrait des
  gestionnaires » sous un même verrou, la récolte après : aucun `kill` ne
  suit la récolte.
- `--run-scripts` : même relais et même fin que pour une délégation.
- Sonde : `pcntl` et `posix` seulement, sans rien construire.

**Vu rouge (fa62aa0) puis vert.** `nohup-handed`, `ignored-int-handed`
(require-signals.sh, un faux `composer` qui dort, SIGHUP ou SIGINT ignorés
par l'appelant et envoyés au groupe). Témoin vert des deux côtés :
`nohup-require` (le mode de `require` passe outre, comme Composer). Le
signal de terminal non relayé (Linux) n'est pas exercé ici (macOS, sans
terminal dans le banc).
