# Les configurateurs de recettes de symfony/flex, émulés nativement — plan

> Niveau 2 : le contrat est la parité à l'octet avec Composer 2.10.3 +
> symfony/flex 2.11.0 (la version de la fixture) sur des fichiers du projet
> que Flex écrit (`symfony.lock`, `config/bundles.php`, `.env`,
> `.gitignore`, `phpunit.dist.xml`, composer.json, composer.lock…) et sur
> stderr. Méta-analyse et revue par sous-agents frais.

## Le problème

Depuis la tranche C (`docs/plans/v0.19-flex-slice-c.md`), `update` avec
install sur un projet Flex est natif quand aucun paquet que Flex enregistre
n'a de recette ni de classe de bundle. Dès qu'une recette s'appliquerait —
c'est-à-dire chaque fois qu'un `update` fait entrer un paquet absent de
`symfony.lock` qui a une recette dans l'index, ou qui embarque un bundle —
vivacity rend la main avec « would apply the recipe of … (not emulated) » ou
« would register …'s bundle in config/bundles.php (not emulated) »
(`crates/vivacity/src/flex_install.rs`, `flex_install_reason`). La tranche C
listait en non-objectifs : les quatorze configurateurs, les recettes qui
s'appliquent, la recette auto-générée d'un bundle, les entrées de
`symfony.lock` ajoutées ou retirées, `--reset`.

## Mesure : quels configurateurs servent vraiment

Les recettes que les fixtures emploient, lues à la révision de l'index fixé
(voir « L'oracle ») : pour chaque paquet du lock de `fixtures/projects/symfony`
et `fixtures/projects/sylius`, la version de recette que
`Downloader::getRecipes` choisirait (index `symfony/recipes`, puis
`recipes-contrib`, et pour sylius l'endpoint `Sylius/SyliusRecipes` en tête),
puis les clés de son manifeste (script hors dépôt, `git show` des fichiers de
recette).

| clé de manifeste | symfony (29 recettes) | sylius (54) | total (83) |
|---|---:|---:|---:|
| `copy-from-recipe` | 27 | 47 | 74 |
| `bundles` | 15 | 35 | 50 |
| `conflict` (pas un configurateur : `Flex::doesRecipeConflict`) | 10 | 14 | 24 |
| `post-install-output` (pas un configurateur) | 5 | 11 | 16 |
| `add-lines` (post-install) | 6 | 9 | 15 |
| `gitignore` | 6 | 8 | 14 |
| `env` | 4 | 8 | 12 |
| `docker-compose` | 2 | 2 | 4 |
| `dockerfile` | 1 | 2 | 3 |
| `dotenv` | 1 | 2 | 3 |
| `composer-scripts` | 2 | 1 | 3 |
| `container` | 0 | 1 | 1 |
| `makefile`, `composer-commands`, `copy-from-package` | 0 | 0 | 0 |

Plus les recettes **auto-générées** (paquet sans recette, avec une classe de
bundle) : dans les `symfony.lock` des fixtures, 1 entrée côté symfony
(`twig/extra-bundle`), 10 côté sylius — toutes `bundles` seul.

Lecture : `copy-from-recipe` et `bundles` couvrent presque tout ; `conflict`
est dans 29 % des recettes, donc à évaluer, pas à refuser ; `add-lines`,
`gitignore`, `env` viennent ensuite. Le reste est marginal.

Un fait qui simplifie : `post-install-output` et le bandeau « What's next? »
vont dans `Flex::$postInstallOutput`, qui n'est imprimé que par
`executeAutoScripts` (l'événement `auto-scripts`, déclenché par le script
`@auto-scripts` de composer.json) ou par `reinstall`. Sous `--no-scripts` —
le contrat de vivacity, et `vivacity update` n'a pas de `--run-scripts` —
ils ne s'impriment jamais (mesuré, voir l'oracle). Les lignes `write()` des
configurateurs sont au niveau VERBOSE, et sous `-v` un projet à plugin actif
part déjà chez Composer (`fallback::active_plugin`) : elles sont portées pour
la fidélité, sans être atteintes aujourd'hui.

## Références symfony/flex 2.11.0 (fixture `fixtures/work/symfony/vendor/symfony/flex/src`)

- `Flex.php:330-355` `recordOperations` (`PRE_OPERATIONS_EXEC`) : une
  `Transaction` synthétique entre les paquets de `symfony.lock` et
  `resultPackageMap` de la transaction locale (le lock, au mode dev de
  l'install) ; `:789-821` `shouldRecordOperation` (install d'un nom absent de
  `symfony.lock`). L'ORDRE des opérations enregistrées est celui de
  `Transaction::calculateOperations` (DFS depuis les racines, plugins
  devant) — c'est lui qui ordonne les recettes et donc `config/bundles.php`.
- `Flex.php:400-543` `install` : `.env` (déjà émulé), `fetchRecipes`, le
  rappel thanks, la ligne vide, puis avec des recettes : `:438` `Symfony
  operations: N recipe(s) (<session>)` (session = `bin2hex(random_bytes(16))`,
  `Downloader.php:95` : aléatoire, normalisé par le harnais), `:439-492` les
  recettes contrib sans `allow-contrib`, `:494-511` `  - Configuring
  <origin>` puis `Configurator::install`, `:521-528` `postInstall`
  (`add-lines`) après TOUTES les recettes, `:542` `finish`.
- `Flex.php:545-553` `finish` : `synchronizePackageJson` (déjà décidé par
  `flex_sync_reason`), `Lock::write`, et `updateComposerLock` (`:823-832`)
  si le md5 de composer.json a changé.
- `Flex.php:621-726` `fetchRecipes` : `getRecipes`, la boucle
  `doesRecipeConflict` (`:657-672`, `:915-930`), `Lock::set` (`:674-685`),
  la recette auto-générée (`:699-722`, `SymfonyBundle::getClassNames`,
  envs `['dev','test']` pour un paquet de `packages-dev` du lock, sinon
  `['all']` ; `Lock::set(name, {version: prettyVersion})`), l'ordre final
  (`:725` : flex, packs, métapaquets, puis `symfony/framework-bundle` en
  tête des autres).
- `Downloader.php:133-308` `getRecipes` : `recipe-conflicts` appliqués à
  l'index (`:137-151`), la version choisie (`:196-233` : alias de branche,
  `major.minor`, plus haute version ≤), l'URL (`recipe_template_relative`
  résolu contre l'endpoint, `:222-230`), l'entrée de lock (`:205-206`,
  `:282-287` : `version`, `recipe: {repo, branch, version, ref}`), les
  fichiers (`:289-291` : tableau → `implode("\n")`, sinon base64), l'origine
  (`:297-300`). `:347-417` `get` (échec → « Failed to download recipe »),
  `:419-435` `parseJson` (`warning`/`info` imprimés).
- `Configurator.php:37-53` l'ordre : `bundles`, `copy-from-recipe`,
  `copy-from-package`, `env`, `dotenv`, `container`, `makefile`,
  `composer-scripts`, `composer-commands`, `gitignore`, `dockerfile`,
  `docker-compose` ; post-install : `add-lines`. `:56-77` `install` /
  `postInstall` (clé présente → `configure`).
- `Configurator/AbstractConfigurator.php:46-55` `write` (VERBOSE, préfixe 4
  espaces), `:57-65` `isFileMarked` / `markData`, `:80-114` `updateData`.
- `Configurator/BundlesConfigurator.php:23-28`, `:60-98`
  (`configureBundles`, FrameworkBundle fusionné, sinon pas d'écrasement
  d'un bundle déjà là), `:100-108` `load` (`require` du fichier),
  `:125-139` `buildContents`.
- `Configurator/CopyFromRecipeConfigurator.php:23-29`, `:75-129`
  (`copyFiles`/`copyDir`/`copyFile` : `Options::shouldWriteFile`, `mkdir
  0777`, `expandTargetDir` sur le CONTENU, `chmod | 0111` si `executable`,
  `getLocalFilePath` pour `files`).
- `Configurator/EnvConfigurator.php:34-151` (`.env.dist` puis `.env`,
  guillemets, `#n`, `%generate(secret)%` aléatoire, phpunit via
  `DOMDocument::saveXML`), `DotenvConfigurator.php:20-26` (un
  `EnvConfigurator` par suffixe).
- `Configurator/GitignoreConfigurator.php:23-28`, `:59-76`.
- `Configurator/ComposerScriptsConfigurator.php:26-31`, `:68-78`
  (`JsonManipulator::addSubNode('scripts', 'auto-scripts', array_merge)`,
  écriture inconditionnelle).
- `Configurator/AddLinesConfigurator.php:35-44`, `:80-136`,
  `:167-211` (`bottom`, `top`, `after_target`), `:232-251`
  (`isPackageInstalled` : dépôt local, `findPackage(name, constraint)`).
- `Configurator/DockerComposeConfigurator.php:97-144`
  `shouldConfigureDockerRecipe` (`extra.symfony.docker` via
  `FILTER_VALIDATE_BOOLEAN`, sinon question interactive, sinon
  `SYMFONY_DOCKER`) ; `DockerfileConfigurator.php:25-32` la même porte.
- `Options.php:39-65` `expandTargetDir` (`%NAME%` → option
  `str_replace('_','-',strtolower)`, `rtrim('/')`, l'échange
  `phpunit.xml.dist`/`phpunit.dist.xml`), `:67-104` `shouldWriteFile`
  (`writtenFiles`, fichier présent → non écrit sans `force`).
- `Path.php:26-40` `relativize` / `concatenate`.
- `Lock.php:38-78` (`add` = `array_merge`, `set`, `write` : `ksort` puis
  `JsonFile::write`, indentation du fichier lu).
- Composer : `ArrayRepository::findPackage` (`docs/reference/resolver/
  ArrayRepository.php:100-119`), `JsonFile::write`/`encode`
  (`docs/reference/JsonFile.php:138-165`, `:279-296`).

## Tranches

### Retenue (ce chantier) : `update` avec install, recettes appliquées

1. **Le pipeline** — sans lui aucun configurateur ne sert :
   - les opérations enregistrées dans l'ordre de `recordOperations`
     (`vivacity_resolver::transaction::Transaction::new`, présent = les noms
     de `symfony.lock`, résultat = le lock résolu au mode dev de l'install) ;
   - l'index complet (`_links`, `branch`, `is_contrib` par endpoint), les
     `recipe-conflicts` ÉVALUÉS (aujourd'hui ignorés : sens « repli
     inutile » tant qu'on ne faisait que détecter ; dès qu'on applique, une
     version retirée par un conflit changerait la recette appliquée) ;
   - la version choisie exactement (une comparaison non numérique → repli,
     au lieu du « suppose qu'une recette s'applique » d'aujourd'hui) ;
   - le fichier de recette récupéré comme `Downloader::get` (URL relative à
     l'endpoint, cache Flex partagé), `warning`/`info`/échec → repli ;
   - le `conflict` de manifeste évalué contre le lock (`findPackage` sur
     `getLockedRepository(true)`) : aucun conflit → on continue ; un conflit
     → repli (la boucle de recherche d'une recette plus ancienne n'est pas
     portée) ;
   - l'entrée de `symfony.lock`, l'ordre des recettes, les lignes
     `Symfony operations` et `Configuring`, l'écriture de `symfony.lock`.
2. **Configurateurs portés** (fréquence × simplicité) : `copy-from-recipe`,
   `bundles` (et la recette auto-générée), `gitignore`, `env`, `dotenv`,
   `add-lines`, `composer-scripts` ; `dockerfile` et `docker-compose`
   seulement quand `extra.symfony.docker` vaut faux au sens de
   `FILTER_VALIDATE_BOOLEAN` (les deux sont alors des no-op sans ligne).
3. **Ordre de coupe** si le chantier déborde : `composer-scripts`, puis
   `add-lines`, sortent en premier (ils restent des replis motivés) ; le
   pipeline + `copy-from-recipe` + `bundles` + `gitignore` + `env`/`dotenv`
   est le minimum livrable.

### Laissées en repli, avec la raison (chacune nommée dans la ligne)

- `container` (1/83) : édite `services.yaml` ligne à ligne selon un état
  déjà édité par l'utilisateur ; `makefile`, `composer-commands`,
  `copy-from-package` (0/83).
- `docker-compose` / `dockerfile` quand `extra.symfony.docker` est vrai
  (écriture de `compose*.yaml`, analyse YAML maison) ou absent (question
  interactive, `  -  IGNORING  …`, puis composer.json réécrit avec
  `symfony.docker` — et vivacity est non interactif là où Composer sur un
  TTY demande).
- Une recette `contrib` sans `allow-contrib` (ni `SYMFONY_ALLOW_CONTRIB`) :
  question interactive.
- `%generate(secret)%` sur un fichier `.env*` qui existe : aléatoire.
- Un `conflict` de manifeste qui correspond ; un index dont TOUTES les
  versions de recette d'un paquet sont retirées par `recipe-conflicts`
  (« Skipping recipe »).
- `extra.symfony.root-dir` ou `extra.root-dir` autre que `.` (tous les
  chemins de Flex deviennent relatifs à un autre répertoire ;
  `relativize` change) — rare, repli.
- Un chemin que Flex regarde dans une zone que l'install écrit avant
  l'événement (`bin-dir`, chemins hors vendor/, `c3.php`).
- Une désinstallation dont l'index liste le paquet ; `--offline` avec une
  recette à télécharger ; `SYMFONY_LOCKFILE`.
- Un endpoint `api.github.com` (en-tête `Accept: …v3.raw` non porté par le
  fetcher) ; un endpoint « legacy ».
- `config/bundles.php` qui n'est pas exactement de la forme que
  `buildContents` écrit (le fichier est `require`-é par PHP : on ne
  l'évalue pas, on le reconnaît — analyse stricte, puis le rendu doit
  redonner les mêmes octets, sinon repli).
- `require` (Flex résout des alias et devine des versions,
  `PackageResolver`) et `remove` avec install (`Unconfiguring`) : inchangés.
- `--reset`, `recipes:update` (`Update/*`) : hors périmètre (commandes que
  vivacity n'a pas).

## Conception

Décision avant toute écriture, comme aujourd'hui : dans `resolve_and_lock`,
entre la résolution et l'écriture du lock. `flex_install_reason` devient
une **simulation complète** de `Flex::install` sur un système de fichiers
virtuel (lecture : surcouche, sinon disque) qui rend soit une raison de
repli, soit un plan : la liste ordonnée des fichiers touchés avec leur
état AVANT (absent / octets) et APRÈS (octets, bit exécutable à ajouter),
les lignes stderr avec leur niveau, `symfony.lock` après, composer.json
après, et le composer.lock après (`updateComposerLock` : le lock que
vivacity va écrire, décodé comme `JsonFile::read`, `content-hash` remplacé,
réencodé par `phpjson` avec son indentation — l'écrasement des `{}` en `[]`
de PHP compris).

Le plan s'applique à `POST_UPDATE_CMD` (`post_install`, là où
`print_flex_post_update` imprime aujourd'hui) : chaque fichier touché est
d'abord VÉRIFIÉ dans son état AVANT (sinon erreur franche : l'install a
modifié un fichier que Flex lit — ne devrait jamais arriver, jamais une
écriture silencieusement différente), puis écrit en place (pas de
temp+rename : on garde l'inode et le mode d'un fichier existant, comme
`file_put_contents`), répertoires créés comme `mkdir(…, 0777, true)`, bit
exécutable ajouté (`fileperms | 0111`).

Pourquoi précalculer : l'état du projet que Flex lit (bundles.php, .env,
.gitignore…) n'est pas touché par l'install, et la décision doit précéder
toute écriture ; recalculer après l'install ouvrirait une seconde porte de
repli trop tard.

## Fichiers, dans l'ordre

1. `fixtures/flex/configurators/{recipes,contrib}/` : `index.json` par
   endpoint (`_links`, `branch`, `is_contrib`, `versions` tels que captés ;
   `recipes` et `recipe-conflicts` limités aux paquets des cas) et les
   fichiers de recette choisis, `git show` de `symfony/recipes` et
   `recipes-contrib` à la révision de l'index. `fixtures/flex/PROVENANCE`
   (commits, dates, licence MIT, attribution), `NOTICE.md` si la forme
   l'exige.
2. `harness/lib/fixture.sh` : servir plusieurs endpoints (la racine servie
   est déjà `fixtures/flex/`).
3. `harness/flex-recipes.sh` (nouveau) : les cas ci-dessous, ROUGES sur le
   binaire de `main` (repli observé), l'oracle affirmé (Composer imprime
   `Symfony operations` et écrit les fichiers attendus).
4. `harness/flex-update.sh` : les cas qui basculent (`bundle-class`,
   `dist-bundle`, `real-lock-gap` : recette auto-générée → natifs, comparés
   à Composer), `recipe-in-index` (le fichier de recette n'est pas servi à
   côté d'`index-recipes.json` → repli « cannot download »).
5. `crates/vivacity/src/flex.rs` : l'index complet par endpoint, les
   `recipe-conflicts` évalués, la sélection exacte, l'URL et la
   récupération d'un fichier de recette, les classes de bundle (liste, pas
   booléen).
6. `crates/vivacity/src/flex_recipes.rs` (nouveau) : `Options`
   (`expandTargetDir`), le système de fichiers virtuel, les configurateurs,
   le plan et son application.
7. `crates/vivacity/src/flex_install.rs` : `flex_install_reason` →
   `flex_install_plan` (raison de repli ou plan), l'ordre des opérations
   enregistrées.
8. `crates/vivacity/src/update.rs`, `install.rs` : le plan porté par
   `Resolved`/`UpdateReport`, appliqué à `POST_UPDATE_CMD`.
9. Docs : `CHANGELOG.md`, `DECISIONS.md`, `HANDOVER.md`, `README.md`,
   `crates/vivacity-core/src/scope.rs`, `docs/plans/v0.19-flex-slice-c.md`,
   CI (`.github/workflows/ci.yml` : le nouveau harnais).

## Cas de harnais (rouges sur `main`, verts après)

Fixture symfony, instantané Packagist figé, état convergé par un `composer
update` de préparation avec un `symfony.lock` complet (aucune recette), puis
le `symfony.lock` réécrit SANS les paquets du cas et la forme du projet
posée. Composer d'abord (la référence), vivacity ensuite (`--no-fallback`).
Comparés : projet entier (`compare_vendor` : contenu, modes, liens),
`composer.lock`, `symfony.lock`, stderr (la session de `Symfony operations`
normalisée). Chaque cas natif affirme que la référence imprime `Symfony
operations` et a créé le fichier attendu.

| cas | paquets retirés de `symfony.lock` (forme du projet) | ce qui est prouvé | attendu |
|---|---|---|---|
| `copy-exec` | symfony/console | copy-from-recipe, `bin/console` exécutable | natif |
| `bundles-existing` | symfony/twig-bundle (bundles.php sans TwigBundle) | bundles ; `conflict` et `recipe-conflicts` évalués sans effet | natif |
| `bundles-absent` | symfony/twig-bundle, sans bundles.php | bundles.php créé | natif |
| `auto-bundle` | twig/extra-bundle (pas de recette) | recette auto-générée | natif |
| `gitignore-new` / `gitignore-append` | friendsofphp/php-cs-fixer | `.gitignore` créé / complété | natif |
| `env-phpunit` | symfony/routing (.env et phpunit.dist.xml sans son bloc) | env : `#n`, PHPUnit `<env/>` | natif |
| `env-quoting` | doctrine/doctrine-bundle (docker à false) | guillemets (`!`, `&`), échappement XML, commentaires ; docker muet | natif |
| `framework` | symfony/framework-bundle (composer.json sans `auto-scripts`, services.yaml présent) | fusion FrameworkBundle, fichier existant non écrasé, dotenv sans `.env.dev`, composer-scripts + `updateComposerLock` (lock de la référence réécrit, affirmé) | natif |
| `add-lines` | phpunit/phpunit | copie puis `after_target`, `requires`, exécutable | natif |
| `phpunit-swap` | phpunit/phpunit (phpunit.xml.dist présent) | échange de `expandTargetDir`, add-lines sur le fichier échangé | natif |
| `asset-mapper` | symfony/asset-mapper (sans paquet symfony-ux) | importmap.php créé, synchronisation qui suit sans effet, add-lines `warn_if_missing` au niveau normal | natif |
| `docker-off` | symfony/mailer (`docker: false`) | env, copy ; aucun `compose*.yaml` (affirmé) | natif |
| `contrib` | phpstan/phpstan | recette contrib, origine | natif |
| `several` | six paquets dont framework-bundle et une contrib | ordre (framework-bundle en tête, puis opérations) | natif |
| `no-sync` | symfony/console (`synchronize_package_json: false`) | l'avis après les recettes | natif |
| `quiet` | symfony/console, `-q` | rien sur stderr, fichiers écrits | natif |
| `already-locked` | aucun | aucune recette (affirmé) | natif |
| `no-dev` | phpunit/phpunit, `--no-dev` | non enregistré (affirmé) | natif |
| `no-plugins` | symfony/console, `--no-plugins` | Flex absent (affirmé) ; le pack symfony réinstallé (défaut trouvé, voir Résultat) | natif |
| `install-recipe` | friendsofphp/php-cs-fixer (retiré de vendor/ et d'installed.json, content-hash faussé) | la recette s'applique à un paquet que l'install pose ; composer.lock réécrit par l'update avant l'événement | natif |
| `docker-unset` | symfony/mailer | Docker non décidé | repli |
| `dotenv-secret` | symfony/framework-bundle (`.env.dev` présent) | `%generate(secret)%` | repli |
| `contrib-denied` | phpstan/phpstan (`allow-contrib: false`) | question contrib | repli |
| `asset-mapper-ux` | symfony/asset-mapper | importmap.php créé, paquets symfony-ux : synchronisation | repli |
| `bin-dir` | symfony/console (`config.bin-dir: bin`) | chemin dans une zone de l'install | repli |

## Critère « vert »

- Les 26 cas : natifs identiques ou repli avec la raison attendue et rien
  d'écrit ; l'oracle affirmé sur chaque cas.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings` (macOS et `--target x86_64-pc-windows-gnu`), `cargo test
  --workspace`.
- Tous les harnais de `ci.yml` à 0 (30 passages avec le nouveau), plus
  `harness/root-manifest.sh` ; en particulier `transitions.sh` et
  `vendor-dir.sh`.

## Méta-analyse (sous-agent frais, sur le code, 2026-10-16) — et ce qui change

Verdict du relecteur : « pas exécutable tel quel » — deux bloquants qui
feraient écrire à vivacity ce que Composer n'écrit pas, un troisième qui
transforme une situation atteignable en erreur après l'écriture du lock.
Vingt et une remarques ; chacune tranchée ci-dessous (ce qui était déjà
juste dans le code commencé est dit tel).

1. **Bloquant — la synchronisation package.json / importmap.php décidée
   sur le disque d'avant les recettes.** `finish` synchronise APRÈS les
   configurateurs ; asset-mapper crée `importmap.php`. Changé :
   `flex_sync_reason_in` lit le système de fichiers virtuel après les
   recettes ; cas `asset-mapper-ux` (repli « symfony-ux ») et
   `asset-mapper` (sans paquet symfony-ux : natif).
2. **Bloquant — le téléchargeur désactivé.** `fetchRecipes` rend `[]`
   (symfony/flex hors des exigences racine) : aucune recette, aucune
   recette auto-générée, aucun « unconfigure ». Changé : décision `Native
   (None)` en tête. Pas de cas de harnais : un `update` qui retire
   symfony/flex des exigences le désinstalle ; seul un Flex global
   l'atteint (laissé, noté).
3. **Majeur — l'état AVANT vérifié à l'application peut échouer
   légitimement** (proxys de `bin-dir: bin`, chemins d'installation hors
   vendor/, `c3.php`). Changé : le système virtuel note chaque chemin
   regardé (lu, testé, écrit) ; la décision rend la main quand l'un tombe
   dans une zone que l'install écrit (`install_areas` : vendor-dir,
   bin-dir, chemins hors vendor du layout, `c3.php`). La vérification à
   l'application reste, comme assertion interne. Cas `bin-dir`.
4. **Majeur — `add-lines` + `warn_if_missing` imprime au niveau NORMAL.**
   Porté : le bloc tel que `write()` le compose (quatre espaces devant
   chaque message, seule la première ligne d'un contenu multiligne
   indentée) ; repli si le texte contient `<` ou `\` (le formateur de la
   console pourrait le transformer). Cas `asset-mapper`.
5. **Majeur — `%generate(secret, N)%` et le `evaluateValue` de
   `configurePhpUnit`.** Déjà couverts par le code (les deux formes ; le
   repli là où la valeur serait réellement générée, `.env*` comme PHPUnit).
   Cas `dotenv-secret`.
6. **Majeur — PHPUnit XML.** Échappement de libxml mesuré (`&amp; &lt;
   &gt; &quot;`, `&#9; &#10; &#13;`, `&#xE9;` hors ASCII) ; `$` et `\`
   dans les lignes insérées → repli (chaîne de remplacement de
   `preg_replace`) ; `\s+` qui avale les lignes vides, toutes les
   occurrences, réécriture même sans correspondance : portés et testés. Cas
   `env-quoting` (doctrine-bundle : `DATABASE_URL` avec `!` et `&`,
   commentaires, docker à false).
7. **Majeur — `%VAR%`.** Toute la carte `extra` (déjà) ; `extra.root-dir`
   de premier niveau prioritaire (déjà : `array_merge`) puis refusé s'il
   n'est pas `.` ; valeur non chaîne → repli ; `.` qui avale `\r` (oui) ;
   contenu binaire (non UTF-8) → repli.
8. **Majeur — les désinstallations passent aussi par `getRecipes`.**
   Changé : une désinstallation d'un nom absent de `symfony.lock` que
   l'index liste → repli (Flex télécharge sa recette, et un échec ou un
   avertissement s'imprime).
9. **Majeur — le choix de version.** Déjà : parcours inverse de l'ordre de
   l'index (pas « le maximum »), `version_compare` exact sur des nombres
   pointés (`5.3` < `5.3.0`), sinon repli. Corrigé : `strip_version` était
   une seule substitution là où `preg_replace` est globale (`v1.x-dev` →
   `1`) — défaut ANTÉRIEUR, présent depuis la tranche B. Ajouté : une
   version de recette entière (clé renumérotée par `array_reverse`) →
   repli.
10. **Majeur — le dépôt des conflits.** Déjà : `getLockedRepository(true)`
    avec les alias, `recipe-conflicts` de tous les endpoints sur l'index
    fusionné, contrainte illisible → repli.
11. **Majeur — `config.lock: false` et `updateComposerLock`.** Déjà : repli.
12. **Majeur — contrib.** Déjà : `SYMFONY_ALLOW_CONTRIB` remplace
    `allow-contrib`, `FILTER_VALIDATE_BOOL` des deux côtés.
13. **Majeur — l'ordre de stderr avec recettes.** Déjà :
    `print_flex_post_update` a sa branche (pas d'indice « Run composer
    recipes », l'avis de synchronisation après les configurateurs). Cas
    `no-sync`.
14. **Mineur — cache Flex.** Changé : la réponse n'est mise en cache que
    si elle porte `last-modified` (la condition de `parseJson`, `etag`
    non suivi par le fetcher) ; `--offline` avec une recette à appliquer →
    repli (Flex passerait en « degraded mode » avec deux avertissements).
15. **Mineur — composer-scripts.** Déjà : `auto-scripts` chaîne ou liste →
    repli ; le retour de `addSubNode` ignoré et `getContents` écrit.
    Corrigé dans le plan : `scripts` n'entre pas dans le content-hash ; ce
    que `updateComposerLock` change est le réencodage (`{}` → `[]`) — le
    cas `framework` affirme que la référence a bien réécrit composer.lock.
16. **Mineur — forme du JSON de recette.** Déjà : repli sur chaque clé
    manquante.
17. **Mineur — harnais.** Changé : les cas sans recette affirment
    l'ABSENCE de « Symfony operations » ; les replis comparent l'arbre
    entier hors vendor/ ; la session est vérifiée des deux côtés avant
    normalisation ; `docker-off` affirme qu'aucun `compose*.yaml` n'est
    écrit ; `several` met framework-bundle et une recette contrib dans le
    lot ; ajoutés `phpunit-swap`, `bin-dir`, `asset-mapper`.
18. **Mineur — existence dans le système virtuel.** Déjà (`exists`,
    `is_file`, répertoires virtuels) ; liens symboliques et chemins qui
    sortent du projet → repli. Non traité : la casse d'un système de
    fichiers insensible (deux chemins de casse différente dans le même
    lot).
19. **Mineur — `requires` d'add-lines.** Déjà : le dépôt local d'après
    l'install (mode dev, alias).
20. **Mineur — formulation.** Corrigé : la copie de `.env` à l'`update`
    n'est pas émulée (`flex_write_guard` rend la main). Sylius lit
    `api.github.com` : ses 54 recettes ne comptent pour rien dans cette
    tranche (repli « GitHub API »).
21. **Mineur — `symfony.lock`.** Déjà : `str_replace` de toutes les
    occurrences de `./`, doublons et fichiers non copiés gardés dans
    `files`, `ksort` de tout le fichier. Changé : une version de
    `symfony.lock` que `VersionParser::normalize` refuse → repli
    (`recordOperations` lève) ; `SYMFONY_LOCKFILE` → repli quand des
    recettes s'appliquent.

Constat en chemin, hors des 21 : framework-bundle 8.1 (la recette que
l'index réel choisit pour la v8.1.0 de la fixture) copie un fichier dont le
nom ne peut pas figurer dans ce dépôt (règle du projet sur les mentions
d'outils). L'index servi s'arrête donc à 7.4 pour ce paquet — les deux
outils lisent le même index, la comparaison porte sur la recette 7.4
(PROVENANCE le dit).

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-16) — et ce qui a changé

Douze remarques : un défaut, quatre risques, cinq détails, deux trous de
documentation ou de harnais. Aucun mot interdit, aucun `unwrap`/`expect`
hors tests.

1. **Défaut — une délégation tardive perd les recettes.** La phase
   d'installation peut encore rendre la main à `composer install` (plugin
   de fusion, portée, refus d'une émulation) alors que le lock est écrit ;
   Composer applique Flex à `POST_INSTALL_CMD`, donc sans recette. Changé :
   après un `composer install` délégué qui réussit, `fallback_or_fail`
   imprime et applique le plan (les états vérifiés comme en natif) ; le
   projet finit comme après l'`update` de Composer (la stderr d'une
   délégation tardive diffère déjà, écart documenté). De plus, le plugin
   de fusion actif avec des recettes rend la main avant le lock (Flex lit
   ses options dans la racine fusionnée).
2. **Risque — ce que le plan n'a fait que regarder n'était pas revérifié.**
   Changé : chaque chemin regardé garde son état disque (absent,
   répertoire, octets) et l'application le revérifie ; les chemins que
   l'install supprime (`Layout::removals`) rejoignent les zones de
   l'install.
3. **Risque — la purge des `symfony-pack` sous `--no-plugins` (défaut
   trouvé par le cas `no-plugins`, voir Résultat) testait le lock.**
   Composer charge les plugins d'installed.json AVANT `purgePackages`.
   Changé : la condition est « Flex parmi les plugins actifs »
   (installed.json, autorisé) ; `exists()` au lieu de `is_dir()`
   (`Filesystem::isReadable`).
4. **Risque — une erreur de `Layout::resolve` avalée.** Changé : repli.
5. **Risque — aucun cas où le run pose le paquet enregistré et réécrit
   composer.lock ; aucun `recipe-conflicts` qui correspond.** Ajouté :
   `install-recipe`. Laissé : un conflit qui correspond n'existe pas dans
   les données réelles pour la fixture ; il faudrait un endpoint inventé
   (refusé : les recettes servies sont les vraies) — test unitaire
   seulement.
6. **Détail — le harnais.** Changé : un repli vérifie aussi une empreinte de
   vendor/ ; la stdout est comparée.
7. **Doc — PROVENANCE disait les conflits ignorés.** Corrigé.
8. **Détail — `formatted_origin`.** Corrigé : la version peut contenir `:`,
   `$` avant un saut final ; une origine `null` est vide.
9. **Détail — `FILTER_VALIDATE_BOOLEAN` ne retire pas NUL.** Corrigé
   (mesuré).
10. **Détail — `%generate(secret,N)%` suivi d'un saut de ligne génère
    aussi.** Corrigé (mesuré).
11. **Détail — l'API GitHub testée sur l'URL de l'endpoint, pas sur celle
    de la recette ; `executable` vérifié trop tard.** Corrigé : l'URL de
    la recette ; `executable` avant `shouldWriteFile` (les arguments de
    `copyFile` sont typés).
12. **Détail — robustesse.** Corrigé : `concatenate` sans indexation, la
    vérification des zones après la synchronisation.

## Résultat (2026-10-16)

Livré : le pipeline et sept configurateurs (`bundles` et recette
auto-générée, `copy-from-recipe`, `env`, `dotenv`, `gitignore`,
`add-lines`, `composer-scripts`), Docker muet à `docker: false`.
`harness/flex-recipes.sh` : 26 cas. Sur le binaire de `main` (4b68941),
24 rouges — les 18 cas à recettes rendus à Composer (« would apply the
recipe of … » ou « would register …'s bundle in config/bundles.php », code
3), les 5 replis avec la raison générique au lieu de la leur, `no-plugins`
(stderr, défaut ci-dessous) — et 2 verts par construction
(`already-locked`, `no-dev` : aucune recette, l'absence affirmée). Après :
26/26. `harness/flex-update.sh` : 4 cas changent (`recipe-in-index` :
nouvelle raison ; `bundle-class`, `dist-bundle`, `real-lock-gap` : natifs
et comparés), rouges sur `main`, 26/26 après.

Portes : `cargo fmt --check`, `clippy -D warnings` (macOS et
`x86_64-pc-windows-gnu`), `cargo test --workspace` (326 tests, 0 échec) ;
les 30 passages de `ci.yml` (dont `diff-vendor.sh` trois fois) et
`root-manifest.sh` : tous à 0.

Trouvé en chemin et corrigé : sous `--no-plugins`, un `symfony-pack` posé
par Flex (sans `install-path` dans installed.json) était tenu pour
installé ; Composer, sans le plugin, l'installe comme une bibliothèque
(`Package operations: 1 install` et `Extracting archive` manquaient à la
stderr de vivacity — vendor/ était juste, l'installateur du cœur le posait
quand même).

## Non-objectifs (vus, laissés)

Listés plus haut sous « Laissées en repli ». De plus : le texte de
« Failed to download recipe » (on rend la main avant), les en-têtes
`If-None-Match` (le cache Flex n'est pas comparé), Windows
(`DIRECTORY_SEPARATOR` dans `getLocalFilePath` : porté, pas exercé).
