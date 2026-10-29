# La durée de la résolution, drupol/composer-packages, l'attribute-collector — plan

> Trois chantiers, dans cet ordre. Niveau 2 (contrat de sortie : stdout,
> stderr, vendor/, fichiers générés). Méta-analyse et revue par sous-agents
> frais. Même méthode que `docs/plans/plugins-round2.md`, dont la section
> « Effets non reproductibles à l'octet » est le point de départ.

## Environnement de mesure

`Darwin MacStudio-de-Adel-5.local 25.6.0 arm64`, PHP 8.5.12 (8.4.27 pour la
double passe), Composer 2.10.3, bash 3.2. Binaire de référence : main
5735721.

## A. `Dependency resolution completed in N seconds` (install, `-v`)

### Ce que mesure Composer

`Installer::doInstall` (sans `$alreadySolved`) construit le pool verrouillé
puis appelle `Solver::solve`. Dans `solve` (Solver.php:205-233) : la carte
des paquets fixés, `RuleSetGenerator::getRulesFor`, les contrôles des
exigences racine et des versions retirées par la liste de blocage, le graphe
de surveillance, `makeAssertionRuleDecisions` ; **puis seulement**
`$before = microtime(true); $this->runSat();` et la ligne. Le minuteur ne
couvre que `runSat` : ni le pool, ni la liste de blocage (réseau), ni la
plateforme, ni les règles.

### Ce que mesure vivacity

`headline` prend `Instant::now()` avant « Verifying lock file contents… »,
`join_verification` imprime `since.elapsed()` après la jointure du fil de
vérification : liste de blocage (attente réseau), sonde de la plateforme,
version racine reçue, pool, règles, SAT, et tout ce que le fil principal
fait en parallèle (transaction locale, portée, plan du dump).

### Mesures (laravel, `fixtures/projects/laravel`, 76 paquets)

| | valeur imprimée |
|---|---|
| Composer 2.10.3, 4 runs | 0.000 (×4) |
| Composer, `runSat` à 6 décimales (phar extrait, format seul modifié), 5 runs | 0.000231 – 0.000371 s |
| vivacity 5735721, 4 runs (vierge puis no-op) | 0.239, 0.074, 0.065, 0.069 |

### Décision

Le travail équivalent s'isole **sans toucher au fil** : `Solver::solve`
(vivacity-resolver) chronomètre son propre `run_sat` et garde la durée
(`Solver::sat_time`) ; `verify_install` la rend dans
`InstallVerification` (`resolution: Duration`), y compris pour un lock
insoluble (`Problems`) et un paquet invalide (`Invalid`), où Composer imprime
la ligne avant l'exception ; `join_verification` imprime cette durée.
`headline` ne rend plus d'`Instant`.

Hors champ, noté : la ligne de `update` (session.rs) chronomètre tout
`Solver::solve` (règles comprises), pas seulement `runSat`. Même cause, même
correctif possible ; laissé (consigne : A porte sur `install`).

### Validation

Mesure avant/après sur laravel (`-v`, vierge puis no-op) : la valeur
imprimée et la valeur à 6 décimales (trace temporaire, retirée). Les harnais
masquent la valeur (`completed in <t> seconds`) : aucun ne change ; un test
unitaire du résolveur vérifie que `sat_time` est rempli et inférieur à la
durée totale de `solve`.

## B. drupol/composer-packages 2.0.0 (bolt-project, `--no-dev`)

### Source lue

Dist du lock (`795e7c5`, tag 2.0.0), licence **MIT** (Pol Dellaiera) :
portable avec attribution (NOTICE.md, copie de référence
`docs/reference/plugins/composer-packages/`). Seule la 2.0.0 est lue ; 1.x
ont un autre code (garde de version).

### Comportement (Composer 2.10.3, mesuré sur bolt-project)

- `getSubscribedEvents` : `post-autoload-dump` → `regeneration` (statique).
  Deux lignes stdout autour : « `drupol/composer-packages: Regenerating
  classes...` », « `drupol/composer-packages: Done.` » (balises `<info>`).
  Ordre : écouteurs de priorité 0 dans l'ordre d'activation ; sur bolt,
  après les deux lignes de package-versions-deprecated.
- Cinq classes, chacune écrite dans `<install path>/src/../build/<Classe>.php`
  (fichier temporaire `<dest>_<uniqid>`, `chmod 0664`, `rename`), dans
  l'ordre Packages, Types, Directories, Versions, Dependencies, depuis
  cinq gabarits Twig (`autoescape` html par défaut ; une fin de ligne
  retirée après chaque `%}`, l'indentation avant un `{% %}` gardée).
- Données : `Locker::getLockData()` (le lock lu, ou réécrit par l'update
  qui précède, ou le lock virtuel de `config.lock: false` —
  `parseJson(encode())`), `packages` puis `packages-dev` **quel que soit
  `--no-dev`** :
  - Packages : `name => entrée brute`, `ksort` (octets : un nom contient
    `/`, jamais numérique), `var_export` ; regex par préfixe.
  - Types : quatre types de base puis ceux rencontrés, entrées brutes.
  - Directories : `getName()` (minuscules) `=>
    InstallationManager::getInstallPath` : `realpath(vendor-dir)/
    <prettyName>[/<target-dir>]` (LibraryInstaller, PluginInstaller),
    `NULL` pour un métapaquet (et un `symfony-pack` sous Flex,
    `SymfonyPackInstaller extends MetapackageInstaller`).
  - Versions : `packageNames` dans l'ordre du lock (liste), regex depuis
    `name => version` trié.
  - Dependencies : `getName() => clés de getRequires()` (minuscules) sauf
    `php` et `ext-*`, `+=` (le premier gagne), `array_values`.
  - `GENERATED_AT = time()` : **seul aléa** ; `ROOT_PACKAGE_NAME` =
    `getName()` de la racine.
- `class_exists(ClassGenerator::class)` : vrai tant que le paquet est là.
- `uninstall` : `glob('…/build/')` rend le répertoire, `unlink` d'un
  répertoire → avertissement → `ErrorException` de Composer (à mesurer) :
  le retrait du plugin chargé est rendu.

### Le modèle porté

1. `vivacity_core::composer_packages` : `render(lock, root_name, vendor_real,
   flex_packs, now) -> [(classe, contenu); 5]` — port des exportateurs et des
   cinq gabarits (texte fixe des gabarits recopié, boucles écrites à la
   main avec la règle d'espaces de Twig), `var_export` existant
   (`runtime_stub::php_var_export`), `htmlspecialchars(ENT_QUOTES |
   ENT_SUBSTITUTE)` sur chaque `{{ }}` non marqué sûr, `camelize`
   (`preg_replace('/[^a-z0-9]+/i', ' ')`, `ucwords`, sans espaces,
   `lcfirst`). `write(build_dir, files)` : temporaire puis `rename`, 0664
   (jamais d'écriture en place : la copie du store est liée).
2. `effect_plugins` (cœur) : `DRUPOL` dans `READ_VERSIONS` (`2.0.0.0`,
   « 1.x generate other classes ») — garde de version au lock et chargée,
   plugin global ; `drupol_issues` ; `Context` reçoit le lock brut
   (`lock: Option<&Value>`).
3. Branchement : `post_autoload_dump_printers` (install, install après
   update, dump-autoload) reçoit drupol dans l'ordre d'activation, avec la
   ligne `-v` de `verbose_listeners` ; `VERBOSE_PLUGINS` reçoit
   `drupol\ComposerPackages\Plugin` / `regeneration`, version `2.0.0` ;
   `EMULATED_PLUGINS`, `RESOLUTION_INERT` (aucun écouteur à la résolution).
4. **Le cache de classmap par entrée du store** : un no-op de Composer
   scanne `build/` avec les cinq classes générées par le run précédent
   (`optimize-autoloader` sur bolt) ; l'enregistrement du store (copie
   vierge, `.gitkeep` seul) les ignorerait. `ClassmapCacheConfig` reçoit une
   liste de paquets dont la copie change sous vendor/
   (`drupol/composer-packages`, inconditionnellement — aussi sous
   `--no-plugins` ou plugin retiré, les fichiers restent) : ils passent au
   cache par fichier (mtime, taille).

### Replis nommés (avant toute écriture)

- une autre version (lock ou chargée), un plugin global ;
- Windows (effets de fichiers, chemins absolus) ;
- le retrait d'un drupol chargé (`uninstall`) ;
- un plugin installeur actif (composer/installers, wordpress-core-installer,
  mnsami, les huit installeurs personnalisés) : `getInstallPath` d'autres
  installeurs, non porté ;
- dg/composer-cleaner ou liborm85 actifs en même temps (non mesuré
  ensemble) ;
- drupol depuis un dépôt `path` ou installé depuis la source ;
- `dump-autoload` sans composer.lock (`LogicException`) ;
- le lock : `packages-dev` absent ou non liste, une entrée sans `name`
  chaîne à exactement un `/`, sans `version` ou `type` chaîne, un type
  numérique (clé entière), un nom en double, un octet NUL (`var_export`
  le découpe) ;
- twig/twig hors 3.x dans le dépôt local (le rendu mesuré est celui de
  Twig 3).

### Corpus et comparaison

`harness/lib/compare.sh` : dans `*/drupol/composer-packages/build/
{Packages,Types,Directories,Versions,Dependencies}.php` seulement, la ligne
`    public const GENERATED_AT = <entier>;` ignorée (`diff -I`, ancrée),
comme le préfixe APCu ; le chemin du projet est déjà ramené à `<project>`.
`harness/compare-selftest.sh` : la tolérance ne masque ni une autre ligne
de ces fichiers, ni la même ligne dans un autre paquet.

### Harnais `harness/composer-packages.sh` (nouveau, `ci.yml` par
`net-retry.sh`)

Un projet qui exige drupol 2.0.0 (Packagist) et package-versions-deprecated
(ordre des lignes), des paquets synthétiques par dépôt `artifact` : un
métapaquet, un `target-dir`, un nom à majuscules, des types à caractères
HTML et à tirets, un `require` vers `ext-*`, `php`, `lib-*`, des flottants
et des clés numériques dans `extra` ; `optimize-autoloader`. Le projet dans
un répertoire dont le nom porte `'` et `&` (échappements `var_export` et
HTML des chemins). Étapes contre Composer : install (vierge, dev), no-op
(classmap des classes générées), mise à jour, retrait d'un paquet,
`--no-dev`, `--no-plugins` (posé et vierge), `dump-autoload`,
`dump-autoload -o`, `-v`, `update` avec install (Packagist), `update` avec
`config.lock: false`, plugin non autorisé. Par étape : code, stdout, stderr,
projet entier — les cinq fichiers comparés octet par octet hors
`GENERATED_AT`. Rendus (`--no-fallback`, code 3, raison, rien d'écrit) :
1.1.2, retrait du plugin, composer/installers actif, liborm85, Twig 2,
dump-autoload sans lock. Vu rouge sur 5735721, puis vert ; PHP 8.5 et 8.4.

## C. ondrejmirtes/composer-attribute-collector 1.2.0 (phpstan-src) — spike

Borné (~2 h, fait avant ce plan). Source lue au tag 1.2.0 (BSD-3-Clause).

- `onPostAutoloadDump` fait `require_once vendor/autoload.php` : les
  `files` de la racine et de tous les paquets s'exécutent dans le processus
  de Composer.
- Sous PHP ≥ 8.0, `ClassAttributeCollector::collectAttributes` passe par
  **la réflexion** : `new ReflectionClass($class)` charge (exécute) la
  classe et ses parents, `getMethods()` / `getProperties()` héritées,
  `ReflectionAttribute::getArguments()` **évalue** les expressions
  constantes (constantes de classe d'autres fichiers, `::class`, arguments
  nommés), puis `var_export` des valeurs. Le chemin par l'AST (nikic) ne
  sert que sous PHP 7.
- phpstan-src au commit du corpus (0e91c3d) : 1 152 fichiers de `src/`
  contiennent `#[` ; arguments du type `tag: self::EXTENSION_TAG`,
  `RegistryFactory::COLLECTOR_TAG`, `as: Foo::class`, `factory: '@…'`.
- Le fichier n'est donc reproductible qu'avec un analyseur PHP, la
  résolution des noms, l'héritage (parents, traits, interfaces, y compris
  dans vendor/) et un évaluateur d'expressions constantes : hors de portée
  sans port d'un analyseur PHP. **Résultat négatif**, aucun code.
- Le cas vide (aucun fichier à `#[`) donne un fichier déterministe —
  mesuré sur la fixture du corpus, dont `src/` minimal ne contient aucune
  classe : `attributes.php` à quatre `array (\n)` ; mais ses deux lignes
  stdout portent une durée (`microtime`), qu'il faudrait elle aussi
  masquer. L'émuler ne rendrait natif que l'artefact du corpus (le vrai
  projet a 1 152 fichiers à attributs) et passerait sur l'exécution des
  `files` de l'autoloader (même catégorie que les cartes d'alias de
  typo3/class-alias-loader, rendu au tour précédent). Écarté ; consigné
  dans DECISIONS.

## Fichiers

1. `crates/vivacity-resolver/src/solver.rs` (`sat_time`), `session.rs`
   (`InstallVerification::resolution`), `crates/vivacity/src/install.rs`
   (`headline`, `join_verification`).
2. `crates/vivacity-core/src/composer_packages.rs` (nouveau),
   `effect_plugins.rs`, `scope.rs`, `lib.rs`.
3. `crates/vivacity/src/install.rs`, `dump.rs`, `update.rs`,
   `no_install.rs` (Context), `verbose_listeners.rs`,
   `crates/vivacity-autoload/src/generator.rs` (paquets à copie mutable).
4. `harness/composer-packages.sh` (nouveau), `harness/lib/compare.sh`,
   `harness/compare-selftest.sh`, `.github/workflows/ci.yml`.
5. `docs/reference/plugins/composer-packages/` (copie, LICENSE), NOTICE.md,
   CHANGELOG `[Unreleased]`, DECISIONS (2026-10-29), HANDOVER, README
   (compteurs, liste des plugins), `docs/corpus/2026-10-29-*.md`, ce plan.

## Critère « vert »

`cargo fmt --check` ; clippy `-D warnings` ; `cargo test --workspace` ;
`harness/composer-packages.sh` vert sous PHP 8.5 et 8.4 (vu rouge
d'abord) ; `compare-selftest.sh`, `plugins-round2.sh`, `plugin-lines.sh`,
`verbosity.sh` (lignes `-v`), `diff-vendor.sh` (classmap), `transitions.sh`
à 0 ; corpus `--only bolt-project,phpstan-src` dans les deux modes,
bolt-project natif 0 diff (stdout et stderr comparés par script), compteurs
mis à jour (93/106 `--no-dev` attendu ; dev selon le scan).

## Méta-analyse (sous-agent frais, plan et code, sans mes conclusions)

0 bloquant, 5 majeurs, 7 mineurs ; ce qui change :

- **M1 chemins échappés** : dans `Directories.php` le chemin du projet
  n'apparaît jamais brut (`var_export` : `\'` ; Twig : `&#039;`, `&amp;`),
  la normalisation `<project>` de `compare.sh` ne le retrouve pas. → le
  banc fait tourner Composer et vivacity **au même chemin absolu**, l'un
  après l'autre (répertoires renommés entre les passages) : les chemins
  écrits sont alors identiques, sans normalisation.
- **M2 flottants de `var_export`** : `php_var_export` réutilise
  `php_double` de l'encodeur JSON, qui écrit `e+20` ; `var_export` écrit
  `1.0E+20`, `1.5E-7`, `1.8446744073709552E+19` (mesuré par le relecteur).
  → `E` majuscule sur le chemin `var_export`, test unitaire sur ces
  valeurs (mesurées sous PHP), et des flottants de ces formes dans le banc.
- **M3 `dump-autoload` avec `config.lock: false`** : le `Locker` lit
  `/dev/null`, `getLockData()` lève une `LogicException` dans le plugin —
  Composer échoue, vivacity écrirait. → repli quand le lock est désactivé
  hors de l'install qui suit un update (lock virtuel) ; et sans
  composer.json.
- **M4 preuve de A** : le test unitaire prévu ne peut pas échouer. → un
  test de `verify_install` où la version racine arrive 200 ms après le
  départ du fil : la durée rendue reste sous 50 ms ; vu rouge en
  rebranchant temporairement l'ancien point de départ. La ligne de
  `update` reste hors champ (consigne), consignée.
- **M5 `-v` sous `require`/`remove`** : les classes renommées
  `_composer_tmpN` partagent un compteur ; drupol dans `VERBOSE_PLUGINS`
  le décale pour les autres. → étapes `require -v` et `remove -v` avec
  package-versions-deprecated chargé.
- **m6 masquage trop large** : `diff -I` laisserait passer une ligne
  `GENERATED_AT` absente ou dédoublée. → seuls les chiffres de
  `^    public const GENERATED_AT = [0-9]+;$` remplacés par `<t>` des deux
  côtés, diff strict ; auto-test « ligne supprimée ».
- **m7 chemins d'installation** : `Package::getTargetDir()` nettoie les
  segments `.`/`..` et ne retire que les `/` de tête (un `/` final reste),
  et `"0"` est faux pour `$targetDir ? …` ; le `realpath` de PHP garde la
  casse donnée là où `canonicalize` la corrige (macOS). → `getTargetDir`
  porté (regex PCRE2 de Composer), `realpath` de PHP émulé (liens résolus,
  casse gardée).
- **m8** `ROOT_PACKAGE_NAME` en minuscules, `__root__` sans nom (cas au
  banc) ; noms en double comparés sans casse (repli).
- **m9** une clé de `require` entière (`0 => 123` chez Dependencies) ou
  une contrainte non chaîne (sautée par `parseLinks`) → repli.
- **m10** `--run-scripts` avec un script racine `post-autoload-dump` :
  `run-script` relance les écouteurs des plugins, drupol écrirait deux
  fois → repli.
- **m11** les classes générées entrent dans la classmap au no-op (500 Kio
  de `var_export`) : le banc met dans descriptions et `extra` `class Foo`,
  `?>`, `/*`, `#`, `<<<`, antislashs et guillemets.
- **m12** C : formulation du cas vide corrigée (durée sur stdout) ; en dev,
  phpstan-src est de toute façon rendu (cweagans/composer-patches).

## Non-objectifs

- La ligne de `update` (règles comprises dans la durée).
- L'attribute-collector (résultat négatif ci-dessus).
- drupol avec d'autres installeurs, sous Windows, avec les nettoyeurs.

## Revue indépendante (sous-agent frais, `git diff 5735721..HEAD`)

1 majeur, 4 mineurs, rendus mesurés contre Composer dans une copie ; ce qui
a changé :

- **M1 `--run-scripts`** : le repli « run-script relancerait l'écouteur »
  tombait après le script `pre-install-cmd`, que Composer relançait (fichier
  témoin écrit deux fois). → décidé avant le premier script
  (`drupol_replay_issue`, devenue `replay_issue` le 2026-10-29, à `install` et à `dump-autoload`) ; le banc pose un
  `pre-install-cmd` à effet visible. Le même ordre vaut pour tout repli de
  l'analyse de portée sous `--run-scripts` : défaut préexistant, laissé,
  consigné.
- **m2 masquage** : n'importe quels chiffres passaient. → le banc exige que
  la valeur de vivacity tombe dans la fenêtre de son passage.
- **m3 `symfony-pack`** : le chemin vient de `SymfonyPackInstaller`, absent
  des vieux Flex. → repli quand un pack est dans le lock.
- **m4 échecs après écriture** : chemin du projet non UTF-8 (Linux) ou
  `vendor-dir` illisible → repli décidé avant.
- **m5** une contrainte invalide dans `packages-dev` d'un lock retouché, sous
  `--no-dev` : Composer lèverait dans le plugin. Non mesuré, laissé,
  consigné.
- Vus par la revue, préexistants, laissés : `target-dir: "0"`, un
  `vendor-dir` à travers un répertoire absent, un zip `artifact` refabriqué
  à version égale servi par le store.

## Résultat (session de travail, macOS)

`Darwin MacStudio-de-Adel-5.local 25.6.0 arm64`, PHP 8.5.12 et 8.4.27,
Composer 2.10.3, bash 3.2.

**Commits** (branche `duration-drupol-collector`, depuis main 5735721) :
6d1e675 (plan), 8e29606 (méta-analyse), 0191558 (A), 9fc3266 (banc rouge,
masquage, ci.yml), 957b2b7 (port), 8923583 (retours de la revue), puis les
docs.

**A** : 0.000 des deux côtés sur laravel (vivacity 0.065–0.239 avant) ; SAT
de vivacity 10–24 µs, `runSat` de Composer 231–371 µs.

**B** : porté. `harness/composer-packages.sh` : 29 cas ; avant le port,
18 étapes rouges sur 18 (rendues : plugin inconnu), puis 29 verts sous
PHP 8.5 et 8.4. Mutations vues rouges : sans `mutable_copies` (classmap du
no-op), sans l'échappement de `&` (Directories). bolt-project natif dans
les deux modes, stdout et stderr identiques par script (install vierge et
no-op, `--no-ansi`, lignes `Downloading` mises à part).

**C** : résultat négatif (ci-dessus), aucun code.

**Corpus** (`docs/corpus/2026-10-29-duration-drupol-collector.md`, complet) :
**93/106 natifs en `--no-dev`, 91/106 en dev, 0 diff, 0 indisponible**
(92 et 90 avant) ; seule bolt-project change de seau. Passage complet sur
957b2b7 ; après la revue, `--only bolt-project,phpstan-src` rejoué sur
8923583 : même verdict dans les deux modes.

**Barrières** : `cargo fmt --check` 0 ; clippy `-D warnings` hôte et
`x86_64-pc-windows-gnu` 0 ; `fixtures/make.sh` puis `cargo test
--workspace` 0 (431 tests) ; les lancements de `ci.yml` et
`root-manifest.sh` à 0 sous PHP 8.5 par `net-retry.sh` (sur 957b2b7 ;
après la revue, rejoués : `composer-packages`, `scripts`, `plugins-round2`,
`compare-selftest`, `transitions`, `plugin-lines`, `package-versions`, et
sous PHP 8.4 `composer-packages`, `plugins-round2`, `plugin-lines`,
`custom-installers`).
