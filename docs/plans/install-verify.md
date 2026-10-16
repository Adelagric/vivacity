# `install` vérifie le lock comme `doInstall` : un solveur, une liste de problèmes — plan

> Suite du découpage de `run_install` (`docs/plans/split-run-install.md`,
> défaut 1 du résultat). Niveau 2 : codes de sortie et texte d'erreur
> publics, et `install` est appelé par toutes les commandes et tous les
> harnais. Méta-analyse et revue par sous-agents frais.

## Le problème, mesuré (binaire de `main`, 494e0d2, 2026-10-16)

`install` ne vérifiait du lock que ses sections `platform`/`platform-dev`
(`check_platform` → `report_platform`, texte à lui, code 4), et les paquets
bloqués par une liste de filtrage sur un fil à part
(`install_policy_problems`, code 2). `Installer::doInstall`
(docs/reference/Installer.php, l. 751-835) fait autre chose : il **résout**
le lock.

1. en-tête, `Verifying lock file contents can be installed on current
   platform.`, avertissement de fraîcheur (QUIET) ;
2. `getMissingRequirementInfo` → lignes, **code 4** sauf
   `allow-missing-requirements` — *avant* tout pool ;
3. une `Request` : chaque paquet verrouillé fixé (`fixLockedPackage`), la
   racine figée (`fixedRootPackage` : sans `require`, avec ses `conflict`,
   `provide`, `replace`), chaque paquet de plateforme fixé sauf ceux que la
   racine fournit à la même version ; `requireName` pour les exigences de
   plateforme **de la racine** (`require`, plus `require-dev` en mode dev,
   `array_merge`), puis pour celles du lock (`getPlatformRequirements`) que
   la racine ne déclare pas ;
4. le pool sur [racine, plateforme, lock] (stabilité minimale et drapeaux
   **du lock**, plateforme surchargée par les `platform-overrides` **du
   lock**), filtré par la liste en portée `install` ;
5. `Solver::solve($request, $this->platformRequirementFilter)` — ligne de
   durée au niveau VERBOSE ;
6. `SolverProblemsException` → `Your lock file does not contain a compatible
   set of packages. Please run composer update.` (QUIET), le
   `getPrettyString` (normal), **code 2** (`max(1, code)`) ; une résolution
   qui donne des opérations → `Your lock file cannot be installed on this
   system without changes. Please run composer update.` (QUIET), **code 4**.

Mesuré sur laravel (`install --dry-run`, `--no-plugins --no-scripts` côté
Composer), sept cas rouges sur `main` au départ (trois de plus ensuite : deux
trouvés par la méta, un `replace` de la racine — voir le résultat) :

| cas | Composer | vivacity (`main`) |
|---|---|---|
| `require` reçoit `ext-nope: *`, lock inchangé | 2, problème `ext-nope` + bloc `.ini` | **0, installe** |
| lock `platform = {php: ^99}`, racine `php: ^8.3` | 0 (la contrainte de la racine l'emporte) | 4, son texte |
| lock `platform = {ext-nope: *}` | 2 | 4, son texte |
| les deux | 2 (`ext-nope`) | 4 (`php`) |
| lock `platform-dev = {ext-nope: *}` | 2 | 4 |
| racine `conflict: {laravel/tinker: *}` | 2, `laravel/laravel 1.0.0+no-version-set conflicts with laravel/tinker v3.0.2.` | **0, installe** |
| lock malware + `ext-nope` (solver-policies) | 2, `ext-nope` en problème 1, le paquet bloqué en 2, bloc `.ini` | 2, le paquet bloqué seul |

## Décisions

1. **Un solveur, une liste.** `install_policy_problems` devient
   `session::verify_install` : la requête, le pool et la résolution de
   `doInstall`, le filtre de liste appliqué au pool (sa partie réseau
   inchangée), `Solver::solve` sous le `PlatformRequirementFilter` de
   `--ignore-platform-req(s)`, `problem::pretty_string` pour le texte
   (le même que celui d'`update`, `.ini` compris). Les problèmes d'exigence
   racine viennent avant ceux des paquets bloqués : c'est l'ordre du solveur
   (`checkForRootRequireProblems` puis
   `checkForFilterListRemovedLockedPackages`), mesuré identique chez
   Composer. `check_platform`, `report_platform`, `PlatformOutcome`,
   `platform::check_install` et `vivacity_core::platform::PlatformFailure`
   (avec `is_ignored`) disparaissent : plus personne ne les lit ; leurs six
   tests unitaires partent avec eux, remplacés par ceux de la session.
2. **L'ordre de `doInstall`.** Exigences manquantes (4) d'abord, puis la
   jointure du fil : avertissements du filtre, ligne de durée (`-v`),
   problèmes (2) ou « without changes » (4).
3. **Le fil** (revu après la méta). Il garde sa place (lancé après
   l'avertissement de fraîcheur, joint avant toute impression au-delà) et
   fait, dans cet ordre : le filtre de liste sur les **seuls paquets
   verrouillés** (il ne juge jamais la racine ni la plateforme : la requête
   réseau part aussi tôt qu'avant, sans attendre ni sonde ni git), puis la
   sonde de plateforme (`platform::probe()`, en cache, ~3 ms, retirée du
   chemin principal), puis la **version racine** (le texte d'un conflit la
   cite : `laravel/laravel 1.0.0+no-version-set`), reçue par un canal : le
   fil principal la calcule une fois, juste après le lancement, pour les
   exigences manquantes et pour le fil — git ne tourne qu'une fois. Le pool
   définitif (PoolBuilder, tout est fixé : pas de réseau) perd les paquets
   que le filtre a ôtés et reçoit leurs enregistrements
   (`filter_list_removed`) — seulement ceux des paquets qui y sont : un
   paquet verrouillé que la stabilité du lock refuse n'entre pas dans le
   pool de Composer, son filtre ne l'a jamais jugé.
4. **`$alreadySolved`.** L'install qui suit un `update` de vivacity ne lance
   pas le fil, ne sonde pas la plateforme (elle le faisait) et ne teste plus
   les exigences manquantes : tout le bloc est sous `if (!$alreadySolved)`
   chez Composer (`doInstall($localRepo, true)` après `doUpdate`).
5. **Sans `php`** (revu après la méta). Composer ne tourne pas sans php :
   pas de référence. Le chemin d'avertissement reste (`Warning: php not
   found, platform requirements were not checked…`, même condition qu'avant :
   le lock a des exigences de plateforme et rien ne les ignore toutes) ; la
   résolution tourne quand même, avec les seuls paquets `composer`,
   `composer-plugin-api`, `composer-runtime-api` (qui ne dépendent pas de
   php) et les exigences sur `php*`, `hhvm`, `ext-*`, `lib-*` ignorées — les
   paquets bloqués, les conflits de la racine et l'API de plugin restent
   vus. L'avertissement passe après ceux du filtre, avant la ligne de durée.
6. **La racine.** Construite comme `RootPackage::load` (même chargeur,
   même version que les exigences manquantes : `root_version::detect` sur le
   manifeste que Composer voit, fusionné par composer-merge-plugin le cas
   échéant — c'est ce que fait déjà la session d'`update`) ; `load` est coupé
   en `load_with_version`, `state::RootPackage::detect` en `with_version`.
   Un manifeste que ce chargeur refuse (contrainte illisible…) l'est aussi
   par Composer avant toute commande : l'erreur remonte par le fil.
7. **Surcharges de plateforme** : celles du lock (`platform-overrides`),
   pas `config.platform` du manifeste — `createPlatformRepo(false)` lit
   `Locker::getPlatformOverrides()`. vivacity lisait le manifeste ; les deux
   ne diffèrent que si `config.platform` a changé depuis le dernier `update`.
   Préférences de la politique : celles du lock, `prefer-stable` de la racine
   pour un lock qui ne les a pas (`createPolicy(false)`, revu après la méta).
8. **`Solver::solve` valide les paquets gardés**
   (`ValidatingArrayLoader::validatePackage`, déjà porté pour `update`) :
   même appel ici, l'erreur remonte.
9. **`COMPOSER_IGNORE_PLATFORM_REQ(S)`** (ajouté après la méta).
   `BaseCommand::initialize` les lit pour toute commande, avec leur
   avertissement, avant toute sortie ; l'`install` de vivacity les ignorait.
   Hors périmètre dans le premier jet, mais ce changement agrandissait
   l'écart : une exigence de la racine absente sort maintenant en 2 là où
   Composer, la variable posée, installe. Lus au début de `read_manifests`
   (avertissement sauf après un `update`, qui l'a déjà imprimé), ils valent
   pour la vérification **et** pour le `platform_check.php` du dump.
10. **Un fil abandonné s'arrête** (ajouté après la méta). Sortir avant la
    jointure (exigences manquantes, fallback du plugin de fusion) laisse le
    fil détaché : un `Drop` sur `Verification` lève un drapeau, que le fil
    lit après son attente réseau et avant de résoudre ; `report_missing` le
    lève lui-même juste avant de sortir au 4 ou de rendre la main (revu après
    la revue) — dans un programme qui embarque vivacity (ePHPm), le fil ne
    sonde plus ni ne résout après le retour de la commande.
    Et la ligne `could not be fully loaded` d'un dépôt dégradé, que le fil
    imprimait lui-même au moment quelconque où il la rencontrait, est gardée
    (`ComposerRepository::hold_output`) et imprimée à la jointure, avant les
    avertissements du filtre — l'ordre de Composer, qui l'écrit en
    construisant le pool.

## Quand la branche « without changes » est-elle atteignable ?

Premier jet (faux, corrigé par la méta) : « inatteignable depuis la ligne de
commande ». Lu dans `Transaction::calculateOperations` : les opérations
comparent les paquets *présents* (`Request::getPresentMap` : dépôt
verrouillé + paquets fixés) au *résultat* (ce que le solveur installe). Le
raisonnement oubliait `PoolBuilder::buildPool` (l. 260-268) : un paquet
verrouillé que la **stabilité du lock** refuse n'entre pas dans le pool
(`unacceptableFixedOrLockedPackages`) et le générateur de règles le saute —
il reste présent sans être au résultat : une désinstallation, donc **code 4**.
Atteignable avec un lock retouché :

- feuille (aucun paquet verrouillé ne la requiert) : rector, lock en
  `minimum-stability: stable` sans drapeaux — ses quatre `rector/rector-*`
  `dev-main` sortent, Composer imprime « cannot be installed on this system
  without changes », code 4 ;
- requis par d'autres : laravel, `psr/log` passé en `3.0.2-beta1` — sept
  problèmes « fixed to 3.0.2-beta1 (lock file version) by a partial update
  but that version is rejected by your minimum-stability », l'indice `-W`,
  code 2.

Les deux sont des cas de harnais, verts du premier coup : le port gardait
déjà les paquets refusés dans la liste des présents. Le reste du
raisonnement tient : tout paquet fixé est installé ou c'est un problème, un
paquet de plateforme que la racine fournit n'est jamais requis seul, et un
cycle A ↔ B ne déclenche rien (`getRootPackages` en garde un membre ; mesuré,
code 0).

## Fichiers, dans l'ordre

1. `crates/vivacity-resolver/src/root.rs` — `load_with_version`.
2. `crates/vivacity-resolver/src/session.rs` — `verify_install`,
   `InstallVerifyInput`/`InstallVerification`/`InstallVerdict`,
   `LockedPool` (requête, jeu de dépôts, pool, résolution : sans sonde ni
   réseau, testable), tests ; `install_policy_problems` retirée.
3. `crates/vivacity-resolver/src/repository.rs` — `hold_output`/`take_held` ;
   `pool_filters.rs` — `locked_removed_problem_text`, sans appelant, retiré.
4. `crates/vivacity-core/src/state.rs` — `RootPackage::with_version`.
5. `crates/vivacity/src/install.rs` — `platform_requirement_options`,
   `spawn_verification`, `root_package`, `join_verification` ; ordre de
   `doInstall`.
6. `crates/vivacity/src/update.rs` — `PlatformOutcome` retiré.
7. `crates/vivacity-resolver/src/platform.rs`,
   `crates/vivacity-core/src/platform.rs` — code mort retiré.
8. `harness/steps.sh` (`@lockjq:`, cas, et la ligne « without changes »
   admise comme sortie de l'oracle), `harness/verbosity.sh` (cas) ; deux
   fixtures de harnais que Composer refusait déjà, corrigées (résultat).
9. CHANGELOG, DECISIONS, HANDOVER, README.

## Tests et critère « vert »

- Unitaires (session) : la requête — exigences de plateforme de la racine
  (`require`, `require-dev` seulement en mode dev, `array_merge` : la
  contrainte de `require-dev` à la place de celle de `require`), celles du
  lock que la racine ne déclare pas, `platform-dev` seulement en mode dev ;
  la contrainte de la racine contre celle du lock ; l'exigence d'un paquet
  verrouillé et le filtre (`IgnoreAll`, nom, `ext-*`) ; les
  `platform-overrides` du lock ; le `conflict` de la racine figée ; les
  paquets `composer-*` sans php ; les préférences absentes d'un vieux lock.
  Vus rouges sur quatre mutations (lock toujours requis, `platform-dev` hors
  mode dev, `conflict` vidé, surcharges ignorées).
- `harness/steps.sh` : les cas du tableau, plus `--ignore-platform-req=…`,
  `--ignore-platform-reqs`, `php+`, `ext-*`, `--no-dev` avec `platform-dev`,
  exigence manquante + échec de plateforme (Composer s'arrête au 4), le
  lock malware + `ext-nope`, les deux paquets refusés par la stabilité, les
  deux variables d'environnement. Vus rouges sur `main`, verts ici.
- `harness/verbosity.sh` : l'échec sous `-q` (tête seule), normal et `-v` ;
  les deux variables d'environnement (leur avertissement comparé).
- Gates : fmt, clippy hôte et `x86_64-pc-windows-gnu`, `cargo test`, tous
  les harnais ; perf no-op (`main` contre branche, hyperfine, médianes).

## Méta-analyse adversariale, première passe (auteur du plan)

- **Le double pool** (abandonné ensuite, voir la méta du sous-agent). Le
  premier jet construisait un pool provisoire autour d'une racine à version
  provisoire pour le seul filtre, puis le pool définitif. Fragile si les
  deux voient d'autres paquets : la méta en a trouvé un (le drapeau de
  stabilité du nom racine, lu par l'acceptation d'un paquet verrouillé qui
  fournit ce nom).
- **Un fil qui attend un canal.** Si le fil principal sort en erreur avant
  d'envoyer la version, l'émetteur est lâché et le fil sort en erreur à son
  tour, détaché comme aujourd'hui. L'envoi est placé juste après le
  lancement, avant toute opération faillible ; l'émetteur est lâché avant la
  jointure.
- **Sortie au code 4 des exigences manquantes sans jointure.** Composer n'a
  pas encore touché au réseau à ce point ; le fil, lui, est parti. Voir la
  décision 10.
- **`after_update` sans exigences manquantes.** Changement observable si une
  résolution de vivacity écrivait un lock qui ne satisfait pas la racine :
  ce serait un défaut de la résolution, que Composer ne verrait pas non plus.
- **Perf.** La sonde quitte le chemin principal ; le fil gagne un pool et
  une résolution (paquets tous fixés : propagation d'assertions). À mesurer,
  hors ligne surtout (pas d'attente réseau pour cacher le travail du fil).

## Méta-analyse (sous-agent frais, sur le code, 2026-10-16) — et ce qui change

Le relecteur a lu le plan, `Installer.php`, `PoolBuilder.php`,
`Transaction.php`, `Request.php`, `BaseCommand.php` et le code de vivacity
(il a trouvé la première implémentation dans l'arbre de travail et l'a lue
aussi). Rien de bloquant ; ce qu'il a trouvé :

- **« Without changes » est atteignable** (majeur) — par un paquet
  verrouillé que la stabilité du lock refuse. **Adopté** : section corrigée,
  deux cas de harnais (rector code 4, laravel code 2), verts sans changer le
  code ; l'oracle aveugle de `steps.sh` admet désormais cette ligne comme
  sortie (elle n'a pas de `  - …`).
- **`COMPOSER_IGNORE_PLATFORM_REQ(S)`** (majeur) : le plan creusait l'écart.
  **Adopté** : décision 9, trois cas (deux dans `steps.sh`, deux dans
  `verbosity.sh`).
- **La sonde mise sur le chemin du réseau** (majeur) : le premier jet sondait
  avant le pool provisoire, donc avant la requête. **Adopté**, et plus loin
  que la suggestion : le filtre tourne sur les paquets verrouillés seuls,
  avant sonde et version racine — plus de pool provisoire ni de racine
  provisoire, ce qui fait aussi tomber son point mineur sur le drapeau de
  stabilité du nom racine.
- **Ce qui se passe avant la jointure** (majeur) : fallback du plugin de
  fusion pendant que le fil écrit le cache, ligne `could not be fully loaded`
  imprimée par le fil à n'importe quel moment, fil détaché qui sonde et
  résout après le retour dans ePHPm. **Adopté** : décision 10. Reste, noté :
  le fallback du plugin de fusion peut tourner pendant l'attente réseau du
  fil (deux processus sur le même cache de dépôt ; Composer écrit ses
  fichiers de cache par renommage).
- **`prefer-stable` absent du lock** (mineur) : **adopté**, `createPolicy(false)`
  retombe sur celui de la racine.
- **Sans php, tout paquet de plateforme tombait** (mineur) : **adopté**,
  décision 5.
- **Le coût sur le fil** (mineur) : relecture du lock et `locked_repository_with`
  (déjà faits par l'ancien fil quand le filtre est actif), un pool, une
  résolution ; rien de nouveau sur le fil principal. Mesuré en ligne, hors
  ligne et sous `--no-blocking` (résultat).
- **Préexistants, notés et laissés** : `GithubActionError::emit` (`::error`
  sur stdout sous GitHub Actions) absent partout ; `config.lock: false` (Composer
  ignore le lock et résout) ; les dépôts ajoutés par composer-merge-plugin ne
  sont pas vus par le filtre ; un lock sans `packages-dev` en mode dev échoue
  après « Verifying… » avec le préfixe `composer.lock:` (Composer : juste
  après l'en-tête) ; le `PoolBuilder` de vivacity n'a pas les
  `ignoredTypes` par défaut d'`Installer` (`php-ext`, `php-ext-zend`) — sans
  effet ici (tout est fixé), pas sans effet pour `update`.
- **Vérifié correct par lui** : les appelants d'`after_update`, l'absence
  d'interblocage du canal, l'ordre manquantes → avertissements → durée →
  problèmes, les lignes `-vv` de Composer (rendues à Composer de toute
  façon), et que les `platform-overrides` du lock ne touchent aucun autre
  chemin (le `platform_check.php` du dump ne lit pas le dépôt de plateforme).

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-16) — et ce qui a changé

Rien de bloquant : il a refait la requête, le jeu de dépôts, la politique,
les surcharges, la recopie des enregistrements du filtre et le verdict contre
`Installer.php`, `PoolBuilder`, `FilterListPoolFilter`, `Solver::solve`,
`LockTransaction`, `RepositorySet` et `BaseCommand::initialize`. Suite donnée :

- **Le fil abandonné ne s'arrêtait qu'à moitié** (majeur) : le drapeau n'était
  lu qu'une fois, juste après le filtre — sans liste active ou sur une réponse
  en cache, le fil l'avait déjà passé quand la commande sortait au 4 ; et sur
  le fallback du plugin de fusion, le drapeau ne se levait qu'au `drop`, après
  la fin du Composer fils. **Corrigé** : `Verification::abandon()` appelé par
  `report_missing` juste avant de sortir au 4 ou de rendre la main, et le fil
  relit le drapeau après la version racine, avant la résolution.
- **Une erreur de `validatePackage`** remontait par la jointure sans les
  avertissements du filtre ni la ligne de durée, que Composer a déjà écrits à
  ce point. **Corrigé** : verdict `Invalid`, imprimé après eux.
- **`locked_removed_problem_text`** (pool_filters.rs) n'avait plus d'appelant :
  **retiré**.
- **La version racine calculée après un `update`** pour rien (git) :
  **corrigé**, la racine n'est détectée que pour la vérification.
- **Sans php, les `platform-overrides` du lock** n'étaient pas appliqués :
  **corrigé** (pas de référence, mais un `php` surchargé reste présent).
- **Deux tests manquaient** : le verdict « without changes » et la recopie des
  enregistrements du filtre (seulement ceux des paquets du pool) — **ajoutés**,
  le second vu rouge en recopiant tout.
- **Commentaires** : `io.rs` (« the policy check »), l'ordre décrit par la doc
  de `spawn_verification`, « git ne tourne qu'une fois » trop large
  (`installer.rs` et la ligne de `install.rs` qui écrit installed.php
  détectent encore la racine) — **corrigés**.
- **Noté, laissé** : des avertissements gardés par un dépôt dégradé sont
  perdus si un autre dépôt échoue ensuite (l'erreur sort sans eux) ; la chaîne
  de version des exigences racine du jeu de dépôts (`setPrettyString` chez
  Composer) n'est pas gardée — un texte qui la citerait est inatteignable ici
  (chaque version verrouillée satisfait son propre `=`) ; `COMPOSER_NO_DEV`
  n'est vrai pour `install` qu'à `1` (Composer : toute valeur vraie pour PHP),
  préexistant.

## Résultat (2026-10-16)

**Le comportement.** Les dix cas rouges sur `main` sont verts, texte
compris : `@require:ext-nope=*` (2), lock `platform` `php: ^99` (0), lock
`platform` `ext-nope` (2), les deux (2), `platform-dev` `ext-nope` (2),
`conflict` de la racine (2), `replace` de la racine (2, ajouté en écrivant le
HANDOVER, qui notait ce piège), `psr/log` en `3.0.2-beta1` (2, sept problèmes et
l'indice `-W`), rector sans drapeaux de stabilité (4, « without changes »),
solver-policies malware + `ext-nope` (2, `ext-nope` en problème 1, le paquet
bloqué en 2, bloc `.ini`). Sous `verbosity.sh`, l'échec `ext-nope` était rouge
aux trois niveaux sur `main` (code 0 contre 2 ; sous `-q` la tête QUIET
manquait). Gardes, verts sur `main` comme ici : `--ignore-platform-req=ext-nope`,
`--ignore-platform-reqs`, `php+`, `ext-*`, `--no-dev` + `platform-dev`,
exigence manquante + `ext-nope` (Composer s'arrête au 4) ; les deux variables d'environnement, vues rouges en neutralisant leur
lecture (six passages de `verbosity.sh` en échec).

**Deux fixtures de harnais que Composer refusait déjà** : `transitions.sh`
fabriquait un lock `dev-main` sans drapeau de stabilité (Composer 2.10.3, mesuré :
« cannot be installed on this system without changes », 4 — le cas visait la
décision de portée, qui vient après) : le drapeau `20` qu'`update` écrit est
ajouté ; `boot.sh` installait rector sans son dépôt git, donc en version
racine `1.0.0+no-version-set`, où rector-src (`replace: rector/rector
self.version`) entre en conflit avec `rector/rector <2.0` (Composer : 2,
mesuré) : `COMPOSER_ROOT_VERSION=dev-main`, comme dans les autres harnais.

**Portes** : `cargo fmt --check` ; clippy `-D warnings` macOS et
`x86_64-pc-windows-gnu` ; `cargo test` 308 réussis, 0 échec (36 binaires ;
304 sur `main` : six tests de `check_install`/`is_ignored` partis avec le code,
dix ajoutés dans la session). **Harnais** (binaire final, chacun dans son
répertoire, effacé après) : 29 passages — diff-vendor 12, `--with-autoloader`
12 puis 12, update 19, steps 269, removal 3, transitions 13, path-repos 14,
vendor-dir 6, merge-plugin 16, scripts 4, tar-dist 7, yii2-composer 8,
bin-plugin 5, wp-core 5, custom-dirs 4, package-versions 7, flex-install 7,
flex-update 26, download-lines 2, artifact-repo 9, compare-selftest 11,
two-stage 12, transport-exit 5, root-scan 12, boot 6, drift-reference (129
fichiers identiques), verbosity 76, fallback-args 29 — tous à 0. Une fois dans
le balayage, update est sorti en 1 sur drupal : `packages.drupal.org` (dépôt
vivant, non figé) a rendu un corps vide (« invalid JSON: EOF ») ; relancé
seul, 19 sur 19.

**Perf** (Mac M4 Max, laravel installé, binaires de `main` et de cette branche
alternés, 60 passages après 3 de chauffe, médianes) : `install --no-plugins
--offline` 48,2 contre 47,8 ms ; `--offline` 51,9 contre 50,9 ms ;
`--no-plugins --offline --no-blocking` 46,7 contre 46,0 ms ; `--no-plugins`
(en ligne) 114,6 contre 113,4 ms puis 114,7 contre 115,6 ms. hyperfine `-N`,
40 passages : `--no-plugins` 114,1 contre 114,8 ms (σ 12,6 et 6,2),
`--offline` 51,9 contre 51,7 ms. Dans le bruit : la sonde a quitté le chemin
principal, la résolution (109 paquets fixés) tient sur le fil.

**Non vérifié** : le chemin sans php (pas de référence ; couvert par un test
unitaire des paquets `composer-*` seulement) ; Windows (clippy seulement) ; un
dépôt réellement dégradé (`could not be fully loaded`) à la jointure ; le
verdict `Invalid` (`validatePackage` sur un lock forgé) hors test.

**Défauts vus, laissés tels quels** (préexistants, suites possibles) :

1. `problem::strip_output_styles` **double le texte** quand un `<` n'a pas de
   `>` après lui : la branche « pas de `>` » repousse `rest` entier, préfixe
   déjà copié compris (`out.push_str(rest)` au lieu de `&rest[start..]`).
   Trouvé par `boot.sh` sur rector sans version racine : le problème
   `conflicts with rector/rector <2.0 (…)` sort deux fois, le premier coupé
   au `<`. Touche aussi `update` (tout texte de problème avec `<` sans indice
   à chevrons après lui). Correctif d'une ligne, hors périmètre —
   **corrigé le 2026-10-16** dans la foulée (`out.push_str(&rest[start..])`,
   cas `rector|install @env:COMPOSER_ROOT_VERSION=1.0.0` dans `steps.sh`,
   rouge sur 4895cef puis vert).
2. `COMPOSER_IGNORE_PLATFORM_REQ(S)` restent ignorées par `dump-autoload`
   (même `BaseCommand::initialize`) ; et sur un fallback, l'avertissement
   imprimé par vivacity l'est une seconde fois par Composer — **corrigé le
   2026-10-16** (`docs/plans/install-defects.md`, point 3 : lues par
   `dump-autoload`, avertissements retenus avec la ligne de version racine
   et jetés par une délégation précoce ; une délégation tardive les répète
   encore).
3. `COMPOSER_NO_DEV` n'est vrai pour `install` qu'à `1` — **corrigé le
   2026-10-16** (`install-defects.md`, point 4 : vrai au sens de PHP, pour
   `install` et `dump-autoload`).
4. `GithubActionError::emit` (`::error` sur stdout sous GitHub Actions) n'est
   porté nulle part.
5. `config.lock: false` : Composer ignore le lock et résout ; vivacity
   l'installe.
6. Les dépôts apportés par composer-merge-plugin ne sont pas lus par le filtre
   de liste ; le fallback du même plugin peut tourner pendant l'attente réseau
   du fil (deux processus sur le cache de dépôt).
7. Un lock sans `packages-dev` en mode dev échoue après « Verifying… », avec le
   préfixe `composer.lock:` (Composer : juste après l'en-tête, son texte) —
   **corrigé le 2026-10-16** (`install-defects.md`, point 5 ; cas
   `run_lock` de `root-manifest.sh`).
8. Le `PoolBuilder` de vivacity n'a pas les `ignoredTypes` par défaut
   d'`Installer` (`php-ext`, `php-ext-zend`) : sans effet sur `install` (tout
   est fixé), pas sur `update` — **corrigé le 2026-10-16**
   (`install-defects.md`, point 2 ; `acme/phpext` dans `solver-problems`).
9. `--offline` : Composer sans réseau (`COMPOSER_DISABLE_NETWORK`) écrit
   « Filter list data could not be fetched… (ignored per
   policy.ignore-unreachable) » là où vivacity écrit « could not be fully
   loaded (offline…) » — vu en mesurant `transitions.sh`, hors périmètre.
10. Les avertissements gardés d'un dépôt dégradé sont perdus si un autre dépôt
    échoue ensuite.
