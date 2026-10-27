# `-v` avec des plugins actifs — plan

> Suite de la décision 4 et du bloquant de la revue de `docs/plans/v0.21-verbosity.md`
> (vers la ligne 146 et la ligne 220) : depuis la 0.21, `-v` est rendu à Composer
> dès qu'un plugin autorisé est actif (`fallback::active_plugin`, appelé par
> `run_once` dans `crates/vivacity/src/lib.rs:588-595`). La règle visait Flex et
> dealerdirect ; elle s'applique à tout plugin, y compris ceux qui n'écrivent
> rien de plus que la ligne de leur écouteur. Un projet Laravel standard
> (`pestphp/pest-plugin`, `php-http/discovery`) passe donc tout `-v` à Composer —
> ce que Deployer (`--verbose` par défaut) et les jobs de CI font tourner.
> Niveau 2 : le contrat est la sortie entière, stdout et stderr, à `-v`.

## Ce que Composer écrit à `-v` pour un plugin (sources, 2.10.3)

Une seule source, pour tous les plugins : `EventDispatcher::doDispatch`
(`src/Composer/EventDispatcher/EventDispatcher.php:230`) écrit, sur stderr, au
niveau `VERBOSE`, **une ligne par écouteur de plugin qu'il appelle** :

```
> <événement>: <classe de l'objet>-><méthode>
```

`get_class($callable[0])` : la classe de l'instance, donc le nom réel de la
classe chargée. Les écouteurs d'un événement sont pris par priorité
décroissante puis dans l'ordre d'ajout (`getListeners`, :583-596 ;
`addSubscriber`, :563) ; tous ceux qui nous concernent sont à la priorité 0,
donc dans l'**ordre d'activation des plugins**, déjà porté pour le niveau
normal (`crates/vivacity/src/plugin_order.rs`, `docs/plans/plugin-lines.md`).
Le reste est propre à chaque plugin : ses propres écritures `VERBOSE`
(bamarni, dealerdirect, composer-merge-plugin, Flex, recipe-unpack dans un
contexte `require`).

Les points de dispatch (Composer 2.10.3, phar extrait sous le scratchpad, jamais
dans le dépôt ; jumeaux dans `docs/reference/`) :

| événement | où | quand |
|---|---|---|
| `command` | `InstallCommand.php:111`, `DumpAutoloadCommand.php:59`, `UpdateCommand.php:250`, `RequireCommand.php:461`, `RemoveCommand.php:239` | avant `Installer::run` ; plugins **déjà installés** seulement |
| `pre-autoload-dump` | `AutoloadGenerator.php:204` | après « Generating autoload files » (`Installer.php:378-380`) |
| `post-autoload-dump` | `AutoloadGenerator.php:476` | fichiers écrits |
| `post-install-cmd` / `post-update-cmd` | `Installer.php:440` | après le funding ; **aussi sous `update --no-install`** |
| aucun | `Installer.php:292-294` | `--dry-run` : `runScripts = false`, aucun événement de script |

`--no-scripts` ne coupe que les scripts de composer.json (`Factory.php:377` →
`EventDispatcher::getListeners`, :585) : les écouteurs des plugins tournent.

**Le suffixe `_composer_tmpN`** (`PluginManager.php:289-307`) : quand la classe
d'un plugin est déjà déclarée dans le processus, `registerPackage` réévalue son
code sous le nom `<classe>_composer_tmp<N>`, `N` étant un compteur statique du
processus, incrémenté à chaque classe renommée. Deux cas :

- `require` et `remove` recréent le `Composer` (`resetComposer` puis
  `requireComposer`, `RequireCommand.php:409-410`, `RemoveCommand.php:221-222`) :
  **chaque classe de chaque plugin chargé** (les `extra.class` des plugins du
  dépôt local, dans l'ordre de `loadRepository`, autorisés et présents sur le
  disque — muets compris, `composer/installers` compris) prend le suffixe
  suivant ;
- la mise à jour d'un plugin chargé par la transaction (`PluginInstaller::update`,
  `PluginInstaller.php:110-111`) : ses classes prennent les suffixes suivants.

## Environnement de cette mesure (Linux, 4 cœurs)

- PHP 8.4.27 compilé depuis le tag `php-8.4.27` de `php/php-src` (dépôt git ;
  `php.net` et les PPA sont refusés par le proxy), avec les extensions de
  `ci.yml` (mbstring, intl, gd, exif, bcmath, sodium, xml, curl, zip, fileinfo),
  `memory_limit=1G`.
- Composer 2.10.3 : le phar de la release GitHub (`composer --version` :
  2.10.3), `COMPOSER_ALLOW_SUPERUSER=1` (la session tourne en root : sans la
  variable, Composer 2.10 désactive les plugins en mode non interactif, ce qui
  fausserait toute référence « plugins actifs »).
- **Archives** : `api.github.com/…/zipball` et `codeload.github.com` répondent 403
  (proxy), `git` passe. Le cache de fichiers de Composer
  (`~/.cache/composer/files/<nom>/<sha1(url)>.zip`, la clé de
  `FileDownloader.php:129-132`, que vivacity lit aussi) est amorcé par
  `git archive --format=zip` du commit verrouillé (préfixe `<owner>-<repo>-<sha7>/`,
  comme GitHub). Les deux outils lisent la même archive : la comparaison reste
  entre eux. `downloads.wordpress.org` est refusé : les quatre dists de la fixture
  wordpress (sans `shasum`) sont reconstruites (hello-dolly depuis
  `WordPress/hello-dolly`, twentytwentyfour depuis `WordPress/twentytwentyfour`,
  akismet synthétique) ; `roots/wordpress-no-content` porte un `shasum` : 
  **`harness/wp-core.sh` ne peut pas tourner ici**. `harness/download-lines.sh`
  exige un cache vierge et un téléchargement réel : **il ne peut pas tourner ici**
  non plus (le résultat le dira avec la sortie).

## Matrice mesurée

Composer 2.10.3, plugins actifs, `--no-scripts -n --no-ansi`, deux copies
identiques jouées au niveau normal et à `-v` ; colonne = lignes que `-v` ajoute
**en plus** des lignes de Composer déjà reproduites (durées, `Analyzed`,
`Installs:`…). Projets d'un plugin (versions exactes), puis Laravel et une
combinaison. Commandes : `install` sur vendor vierge, no-op, `--no-dev`, retour
dev, `update`, `update --no-install`, `require psr/clock:1.0.0`, `remove
psr/clock`, `dump-autoload`, `install --dry-run`. « — » : rien.
« tmp » : la même ligne avec `_composer_tmpN` (require/remove).

| plugin (version mesurée) | install (vierge / no-op / no-dev) | update | update --no-install | require / remove | dump-autoload | dry-run |
|---|---|---|---|---|---|---|
| composer/installers v2.3.0 | — | — | — | — | — | — |
| roots/wordpress-core-installer v4.0.0 | — | — | — | — | — | — |
| mnsami/composer-custom-directory-installer 2.0.0 | — | — | — | — | — | — |
| ibexa/post-install v5.0.10 | — | — | — | — | — | — |
| ergebnis/composer-normalize 2.53.0 | — | — | — | — | — | — |
| symfony/runtime v5.4.53, v6.4.41, v7.4.14, v8.1.0 | `> post-autoload-dump: Symfony\Component\Runtime\Internal\ComposerPlugin->updateAutoloadFile` | idem | — | idem, tmp | idem | — |
| pestphp/pest-plugin v4.0.0 | `> post-autoload-dump: Pest\Plugin\Manager->registerPlugins` | idem | — | idem, tmp | idem | — |
| php-http/discovery 1.20.0 | `> pre-autoload-dump: Http\Discovery\Composer\Plugin->preAutoloadDump` | + `> post-update-cmd: …->postUpdate` | `> post-update-cmd: …->postUpdate` | les deux, tmp | pre-autoload-dump | — |
| composer/package-versions-deprecated 1.11.99.5 | `> post-autoload-dump: PackageVersions\Installer->dumpVersionsClass` | idem | — | idem, tmp | idem | — |
| phpstan/extension-installer 1.4.3 | `> post-install-cmd: PHPStan\ExtensionInstaller\Plugin->process` (avant ses lignes stdout) | `post-update-cmd` | `post-update-cmd` | idem, tmp | — | — |
| rector/extension-installer 0.11.2 | `> post-install-cmd: Rector\RectorInstaller\Plugin->process` | `post-update-cmd` | `post-update-cmd` | idem, tmp | — | — |
| codeception/c3 2.9.0 | `> post-install-cmd: Codeception\c3\Installer->copyC3V2` | `> post-update-cmd: …->askForUpdateV2` | idem | idem, tmp | — | — |
| drupal/core-project-message 11.4.6 (sans message) | `> post-install-cmd: Drupal\Composer\Plugin\ProjectMessage\MessagePlugin->displayPostCreateMessage` | — | — | — | — | — |
| drupal/core-recipe-unpack 11.4.6 | — | `> post-update-cmd: Drupal\Composer\Plugin\RecipeUnpack\Plugin->unpackOnRequire` | idem | idem, tmp (require : déjà rendu au niveau normal) | — | — |
| bamarni/composer-bin-plugin 1.9.1 | no-op : `> command: Bamarni\Composer\Bin\BamarniBinPlugin->onCommandEvent`, `[bamarni-bin] Calling onCommandEvent().` (avant ses dépréciations) ; puis `> post-autoload-dump: …->onPostAutoloadDump`, `[bamarni-bin] Calling onPostAutoloadDump().` ; vierge : le second seulement | les deux | `command` | les deux, tmp | les deux | `command` (méta, point 1) |
| yiisoft/yii2-composer 2.0.11 | — (mais `post-package-update` sur une mise à jour) | `> post-update-cmd: yii\composer\Plugin->showUpgradeNotes` | idem | idem, tmp | — | — |
| symfony/thanks v1.4.1 | — (mais `post-package-update`) | `> post-update-cmd: Symfony\Thanks\Thanks->displayReminder` | idem | idem, tmp | — | — |
| dealerdirect v1.2.1 | `> post-install-cmd: PHPCSStandards\…\Plugin->onDependenciesChangedEvent`, « Running PHPCodeSniffer Composer Installer », puis la sortie de `phpcs --config-set` (« Using config file: <chemin absolu> », « Config value "installed_paths" added successfully ») ou « No PHPCS standards to install or update » | idem | idem | idem, tmp | — | — |
| wikimedia/composer-merge-plugin v2.1.0 | `init`, `pre-install-cmd`, `post-package-install` (+ `[merge-plugin] composer-merge-plugin installed`), `pre-autoload-dump`, `post-install-cmd` ; sur require, `init` deux fois (avant et après le renommage) | … | … | … | … | — |
| symfony/flex v2.11.0 | `pre-pool-create`, `pre-operations-exec`, `post-package-install`, `post-install-cmd` ; sur update, les lignes verbeuses de ses configurateurs (« Copying files from recipe », `Created "./.env"`) | … | … | … | … | — |
| metasyntactical/composer-plugin-license-check 2.3.0 | non mesuré (le lock ne se résout pas sous PHP 8.4 ici) ; source lue : `command` (priorité 101), `post-package-install`/`-update` (100), trois écritures sous `isVerbose()` (`LicenseCheckPlugin.php:92, 104, 175`) | | | | | |

Projet Laravel (`fixtures/projects/laravel/composer.json` avec `pestphp/pest`
^4.1, `pestphp/pest-plugin-laravel` ^4.0 au lieu de phpunit, `php-http/discovery`
^1.20 ; résolu : pest-plugin v4.0.0, discovery 1.20.0) : vierge et no-op,
`> pre-autoload-dump: Http\Discovery…` puis `> post-autoload-dump: Pest\Plugin\Manager…` ;
`--no-dev` : discovery seul ; `update` : + `> post-update-cmd: Http\Discovery…->postUpdate` ;
`update --no-install` : cette dernière seule ; `require`/`remove` :
`Pest\Plugin\Manager_composer_tmp0`, `Http\Discovery\Composer\Plugin_composer_tmp1`
(pest-plugin est chargé avant discovery : `PackageSorter`, plus de paquets le
requièrent).

Combinaison (runtime, pest, discovery, package-versions, phpstan, c3, bamarni,
core-project-message, installers) : sur vendor vierge, l'ordre de la
transaction (bamarni, package-versions, pest, runtime à `post-autoload-dump` ;
c3, message, phpstan à `post-install-cmd`) ; no-op : l'ordre de
`loadRepository` ; `require` : `installers` tmp0 (poids -10000), bamarni tmp1,
c3 tmp2, package-versions tmp3, message tmp4, pest tmp5, discovery tmp6,
phpstan tmp7, runtime tmp8 — le compteur suit exactement l'ordre de chargement,
plugins muets compris. Mise à jour de c3 2.8.1 → 2.9.0 par `install` :
`> post-install-cmd: Codeception\c3\Installer_composer_tmp0->copyC3V2`.

## Toutes les versions lues

Chaque dépôt cloné en entier ; pour **chaque tag** : la classe `extra.class`, le
corps de `getSubscribedEvents`, et les mentions de `IOInterface::VERBOSE` /
`isVerbose(` dans les sources hors tests (script `scan-tags.sh`, scratchpad).
La table lie chaque plugin aux versions lues dont la table d'écouteurs est
celle mesurée ; hors de ces versions (antérieure, postérieure, branche dev),
`-v` reste rendu à Composer avec la raison :

| plugin | versions (tags lus) | écouteurs | écritures `VERBOSE` |
|---|---|---|---|
| composer/installers | v1.0.0 – v2.3.0 (45) | aucun | 0 |
| roots/wordpress-core-installer | 1.0.0 – v4.0.0 (6) | aucun | 0 |
| mnsami/composer-custom-directory-installer | 1.0.0 – 2.3.0 (12) | aucun | 0 |
| ibexa/post-install | v1.0.0 – v5.0.10 (132) | aucun (`activate` : DEBUG) | 0 |
| ergebnis/composer-normalize | 0.1.0 – 2.54.0 (109) | aucun | 0 |
| symfony/runtime | v5.3.0 – v8.1.9 (130) | `post-autoload-dump` → `updateAutoloadFile` | 0 |
| pestphp/pest-plugin | v0.2.0 – v5.0.0 (12) | `post-autoload-dump` → `registerPlugins` | 0 |
| php-http/discovery | 1.17.0 – 1.20.0 (1.15–1.16 : `post-update-cmd` seul) | `pre-autoload-dump` → `preAutoloadDump`, `post-update-cmd` → `postUpdate` | 0 |
| composer/package-versions-deprecated | 1.8.0 – 1.11.99.5 (11) | `post-autoload-dump` → `dumpVersionsClass` | 0 |
| phpstan/extension-installer | 1.0 – 1.4.3 (15) | `post-install-cmd`, `post-update-cmd` → `process` | 0 |
| rector/extension-installer | 0.10.0 – 1.0.0 (7) | idem | 0 |
| codeception/c3 | 2.6.0 – 2.9.0 (avant : événements de paquet) | `post-install-cmd` → `copyC3V2`, `post-update-cmd` → `askForUpdateV2` | 0 |
| drupal/core-project-message | 8.8.0 – 11.4.11, 12.0.0-beta2 (385) | `post-install-cmd` (et `post-create-project-cmd`) → `displayPostCreateMessage` | 0 |
| drupal/core-recipe-unpack | 11.2.0 – 11.4.11, 12.0.0-beta2 (62) | `post-update-cmd` → `unpackOnRequire` (et `post-create-project-cmd`) | 7, toutes dans le contexte `RequireCommand` (`Plugin.php:100-124`) — déjà rendu au niveau normal |
| bamarni/composer-bin-plugin | 1.8.0 – 1.9.1 | `command` → `onCommandEvent`, `post-autoload-dump` → `onPostAutoloadDump` | `Logger::logDebug` (VERBOSE) : « Calling onCommandEvent(). », « Calling onPostAutoloadDump(). » ; « Command already forwarded… » seulement avec `forward-command` (déjà rendu) |

## Décisions

1. **Natifs sous `-v`, sans ligne** : composer/installers,
   roots/wordpress-core-installer, mnsami/composer-custom-directory-installer,
   ibexa/post-install, ergebnis/composer-normalize (aucun écouteur, à toutes les
   versions lues). Ils comptent quand même pour le suffixe `_composer_tmpN`.
2. **Émulés sous `-v`** (la ligne `> …` à sa place, puis l'effet du plugin
   comme aujourd'hui) : symfony/runtime, pestphp/pest-plugin, php-http/discovery,
   composer/package-versions-deprecated, phpstan et rector/extension-installer,
   codeception/c3, drupal/core-project-message, drupal/core-recipe-unpack (hors
   `require`, déjà rendu), bamarni/composer-bin-plugin (ses deux lignes
   `logDebug` en plus, sur stderr, avant ses dépréciations).
3. **Rendus à Composer sous `-v`, raison nommée dans la ligne de délégation** :
   - symfony/flex : écoute `pre-pool-create`, `pre-operations-exec`,
     `post-package-*`, et ses configurateurs écrivent en `VERBOSE` ;
   - dealerdirect : imprime la sortie de `phpcs --config-set` (texte propre à
     chaque version de phpcs, chemin absolu de CodeSniffer.conf) ;
   - wikimedia/composer-merge-plugin : écoute `init`, `pre-install-cmd`,
     `post-package-install` et journalise en `VERBOSE` ;
   - yiisoft/yii2-composer et symfony/thanks : écoutent `post-package-update`,
     dispatché au milieu des opérations (vivacity ne place pas ces événements) ;
   - metasyntactical/composer-plugin-license-check : `command` et
     `post-package-*`, écritures sous `isVerbose()` ;
   - tout autre plugin (hors listes), une version hors des versions lues, un
     plugin installé globalement (`COMPOSER_HOME` : les émulations ne jouent
     que le dépôt local), et `--run-scripts` avec un plugin à écouteurs
     (`composer run-script` rejoue les écouteurs, écart existant).
   La phrase garde la forme « `-v while <plugin> is active (its listeners'
   lines: <raison>)` », que `harness/plugin-lines.sh` (`all-verbose`) attend
   déjà.
4. **Le suffixe `_composer_tmpN`** est porté : un compteur par exécution
   (remis à zéro par `run`), avancé par le rechargement de `require` / `remove`
   (classes de tous les plugins chargés, dans l'ordre de `loaded_plugins`) et
   par chaque `Update` d'un plugin chargé dans la transaction (ses classes
   déjà déclarées).
5. **`update --no-install`** : Composer dispatche `post-update-cmd` ; vivacity
   imprime les lignes `>` de ses écouteurs (ordre de chargement) après le
   funding. Les **effets** de ces écouteurs restent non joués sous
   `--no-install` : écart du niveau normal, déjà noté (`plugin-lines.md`,
   non-objectifs : phpstan imprime « Extensions installed ») — `-v` n'en ajoute
   pas d'autre.
6. `--dry-run` : aucun événement de **script** (mais `command`, voir la
   méta-analyse, point 1). `-vv`/`-vvv` :
   inchangé, rendu. `--no-plugins` : inchangé, natif. `-q` : inchangé.

## Fichiers

1. `harness/verbosity.sh` : une seconde partie, **projets en ligne** (Packagist,
   COMPOSER_HOME isolé, cache de fichiers partagé, lock écrit par Composer sous
   `--no-plugins`), préparations `@vendor:plugins` (vendor posé par Composer,
   plugins actifs, le même des deux côtés), `@delegates:<motif>` (la raison
   nommée sur la stderr brute de vivacity), `@lockfrom:<projet>` (un vendor
   posé depuis un autre lock — mise à jour d'un plugin) ; chaque cas compare
   stdout et stderr entiers **et** la sortie fusionnée (ordre stdout/stderr :
   les lignes `>` sont sur stderr, celles de phpstan sur stdout).
2. `crates/vivacity/src/verbose_listeners.rs` (nouveau) : la table
   (`VERBOSE_PLUGINS` : nom, versions lues, classe, écouteurs ;
   `VERBOSE_DELEGATED` : nom, raison), `issue` (la raison d'une délégation, ou
   rien), `announce` (la ligne `>` d'un plugin pour un événement, au niveau
   `-v`), le compteur de renommage (`first_load`, `reload_plugins`,
   `plugin_registered`) ; tests
   unitaires.
3. `crates/vivacity/src/fallback.rs` : `active_plugin` → la raison de
   `verbose_listeners::issue` (petit diff : une autre branche y travaille).
4. `crates/vivacity/src/lib.rs` : la délégation `-v` lit cette raison ;
   `--run-scripts` passé.
5. `crates/vivacity/src/install.rs` : `announce` avant chaque effet à
   `post-install-cmd`/`post-update-cmd` (plus message, discovery, recipe-unpack
   dans la liste parcourue), à `post-autoload-dump` (plus runtime et pest), à
   `pre-autoload-dump` (discovery) ; bamarni : ses lignes `logDebug` ;
   `note_plugin_operations` sur la transaction (premier chargement, puis
   chaque opération de plugin).
6. `crates/vivacity/src/dump.rs` : `pre-autoload-dump` et `post-autoload-dump`
   dans l'ordre de chargement.
7. `crates/vivacity/src/update.rs` (`print_post_update` : `post-update-cmd`
   sous `--no-install`, jamais sous `--dry-run`), `require.rs`, `remove.rs`
   (`reload_plugins` au point de `resetComposer`).
8. Docs dans le même commit : `CHANGELOG.md` (`[Unreleased]` → `### Changed`),
   `DECISIONS.md`, `HANDOVER.md`, `README.md` (`-v`),
   `docs/plans/v0.21-verbosity.md` (renvoi ici), ce plan.

## Cas de harnais (rouges d'abord, sur le binaire de `main`)

Projets en ligne de `verbosity.sh` (niveaux normal, `-q`, `-v` sauf mention) :

- `laravel-pest` : `install` (vierge), no-op, `--no-dev`, `update`,
  `update --no-install`, `require psr/clock:1.0.0`, `require … --no-install`,
  `remove laravel/tinker`, `dump-autoload`, `install --dry-run` (`-v`),
  `install --no-plugins` (`-v`), `install` `-vv` (rendu des deux côtés) ;
- un projet par plugin émulé (`-v`) : runtime, pest, discovery, package-versions,
  phpstan, rector, c3, message, recipe-unpack (`update`), bamarni (vierge,
  no-op, `dump-autoload`, `update --no-install`) ;
- muets (`-v`) : installers, wordpress-core-installer, custom-directory ;
- combinaison (`-v`) : vierge, no-op, `--no-dev`, `update`, `require`,
  `remove`, `dump-autoload` — l'ordre et le compteur ;
- mise à jour d'un plugin (`-v`) : c3 2.8.1 → 2.9.0 par `install` ;
- rendus, raison nommée (`-v`, `@delegates:`) : dealerdirect, Flex, une version
  non lue (discovery 1.15.0).

Les rouges attendus : tous les cas `-v` des projets à plugin (vivacity délègue,
la sortie de Composer passe des deux côtés mais `@delegates:` ou l'assertion
« natif » échoue) — d'où une assertion de plus par cas `-v` : **vivacity ne
délègue pas** (aucune ligne `vivacity: delegating` ni « outside what vivacity »
sur sa stderr brute), sauf les cas `@delegates:`.

## Méta-analyse (sous-agent frais, sur le plan et le code, 2026-10-22) — et ce qui change

Un bloquant, six majeurs, cinq mineurs, deux nits. Le relecteur a rejoué
Composer 2.10.3 sur ses propres projets : le modèle du compteur a tenu partout
(plugin à trois classes — mnsami 2.0.0 prend tmp0 à tmp2 —, mise à jour de
plugin dans un `require` — tmp0 à tmp3 au rechargement, puis tmp4 —,
répertoire de plugin perdu — non compté, réinstallé sans suffixe, en dernier —,
plugin retiré — sa ligne `command` garde le suffixe, les suivantes
disparaissent). Ce plan est **amendé** par ce qui suit ; là où ils divergent,
cette section l'emporte.

1. **Bloquant — `--dry-run` n'est pas muet.** Seuls les événements de *script*
   sont coupés (`Installer.php:292-296`) ; `command` part avant
   `Installer::run`, et `require`/`remove` rechargent le `Composer` même en
   dry run. Mesuré : `bamarni … --dry-run -v` écrit `> command: …` (et
   `_composer_tmp0` pour `require`/`remove`). **Adopté** : la décision 6 et la
   colonne dry-run de la matrice sont fausses pour `command` ; vivacity passe
   déjà par `bamarni_command_event` et `reload_plugins` en dry run ; quatre
   cas bamarni `--dry-run` ajoutés.
2. **Majeur — `--no-install` empirait.** Aujourd'hui `-v` rend ces runs à
   Composer, donc la sortie est juste ; native, elle porterait la ligne `>`
   de phpstan sans « Extensions installed » (écart du niveau normal).
   **Adopté** : sous `-v` et `--no-install` (sans `--dry-run`), phpstan,
   rector et c3 rendent la commande, raison nommée (`UNPLAYED_WITHOUT_INSTALL` ;
   levé le 2026-10-27, les écouteurs étant joués sous `--no-install` depuis
   la 0.24.0 — DECISIONS) ;
   cas `phpstan update --no-install` → `@delegates`. discovery,
   recipe-unpack et bamarni n'ont pas d'effet là : natifs.
3. **Majeur — les plugins qu'amène la résolution.** Le contrôle de `run_once`
   ne voit que l'installé et le lock ; un `require`/`update` qui amène un
   plugin non reproduit serait rendu *après* l'écriture du lock, par un
   `composer install -v` dont les lignes diffèrent. **Adopté** :
   `verbose_listeners::resolved_issue` sur le lock résolu, dans
   `resolve_and_lock`, avant l'écriture, via `Resolved::hand_over` ; cas
   `pest require php-http/discovery:1.15.0` (rendu) et `…:1.20.0` (natif,
   plugin activé par le run, sans suffixe).
4. **Majeur — quelle version, et des listes qui vieillissent.** **Adopté** :
   versions installée, cible (lock) et résolue toutes contrôlées. **Gardé** :
   les bornes restent les tags lus (la ligne `>` ne dépend que de la classe et
   de la méthode, stables sur tous les tags lus, mais une version publiée
   demain n'a pas été lue — le contrat est l'octet, la délégation coûte une
   exécution Composer) ; procédure de mise à jour dans DECISIONS. Le libellé
   de 1.15/1.16 de discovery est corrigé : « n'est pas parmi les versions
   reproduites » (elles ont été lues, elles diffèrent).
5. **Majeur — plugins globaux.** Composer les juge par l'`allow-plugins`
   global. **Adopté** tel quel : tout plugin du dépôt global rend `-v`,
   quel que soit l'`allow-plugins` du projet.
6. **Majeur — le harnais pouvait passer sans rien tester.** **Adopté** : sous
   `-v` (ou `SHELL_VERBOSITY=1`), la stderr de Composer doit contenir au moins
   une ligne `>` (sauf `@silent`, qui exige zéro) ; `@expect:` sur les
   suffixes ; le cas Symfony existant (`install --dry-run @plugins`) exige
   `@delegates:symfony/flex`.
7. **Majeur — Laravel en contraintes larges.** **Adopté** :
   `fixtures/projects/laravel-pest` (composer.json et lock figés, plugins
   épinglés : pest-plugin v4.0.0, discovery 1.20.0) ; les cas `update` sont
   partiels (`update php-http/discovery`).
8. **Mineur — couverture du compteur.** **Adopté** : mnsami (trois classes)
   dans la combinaison, `c3-old require codeception/c3:2.9.0` (rechargement
   puis mise à jour), `combo remove bamarni/composer-bin-plugin`, répertoire
   de pest-plugin supprimé (`@rm:`).
9. **Mineur — le renommage suit le nom de classe, pas l'opération.**
   **Adopté** : un ensemble des classes déclarées (minuscules, comme PHP les
   compare) ; `Install` d'un plugin déjà enregistré ignoré ; une classe
   nouvelle d'une version mise à jour chargée sans suffixe.
10. **Mineur — `command` et la présence sur le disque.** **Adopté** :
    `bamarni_command_event` exige aussi le plugin dans `loaded_plugins` (ses
    classes sur le disque), au niveau normal comme sous `-v`.
11. **Mineur — cas manquants.** **Adoptés** : `--no-autoloader`, `install`
    sans lock (`post-update-cmd` alors que `command` dit `install`),
    `require --no-update` (`@silent`), `SHELL_VERBOSITY=1`. Non ajouté :
    `config.lock: false` (même chemin que l'install sans lock côté vivacity).
12. **Mineur — sortie fusionnée.** Déjà ainsi : chaque capture tourne une
    fois, sur sa propre copie, préparée à l'identique.
13. **Nit — `COMPOSER_DEBUG_EVENTS`** (« Dispatching … » au niveau normal,
    `EventDispatcher.php:195-205`) : non-objectif, noté.
14. **Nit — l'auto-installation de discovery** (`postUpdate` qui installe une
    implémentation manquante : `stopPropagation`, `Factory::create`, d'autres
    renommages) : non-objectif — le classement « bénin » de discovery suppose
    déjà un lock complet.

Risque sur les harnais existants jugé faible par le relecteur : `all-verbose`
de plugin-lines.sh reste rendu (dealerdirect) et sa phrase attendue correspond
encore.

## Revue indépendante (sous-agent frais, sur `git diff main`, 2026-10-22) — et ce qui a changé

Pas de bloquant ; trois majeurs, deux mineurs, des nits. Le relecteur a
rejoué la partie `online` du harnais (113 passages, tous verts) et cinq
scénarios à la main (`require --dry-run -v` avec pest et discovery,
`update codeception/c3 -v`, `require` qui amène deux plugins, `require`
avec choix de version, `remove` d'un plugin) : identiques à l'octet.

1. **Majeur — une délégation tardive réimprimait le début de la run.**
   `require php-http/discovery:1.15.0 -v` sur un projet à pest-plugin :
   `resolved_issue` rend la main après la résolution, vivacity avait déjà
   écrit « has been updated », « Running composer update… », « Updating
   dependencies » — que Composer réécrit ; `main` rendait la main avant
   toute ligne. Et le harnais coupait tout ce qui précède « delegating to ».
   **Corrigé** : `vivacity_core::io::defer` (les deux flux gardés dans
   l'ordre) dès `run_once` pour un `update`/`require`/`remove` qui installe
   sous `-v`, relâchés par `resolve_and_lock` une fois le lock accepté,
   jetés par `delegate_resolution` quand Composer prend la main ;
   le harnais n'admet plus avant « delegating to » que les lignes propres à
   vivacity (`vivacity: `, `  - plugin `). Reste : l'`install` sans lock.
2. **Majeur — bamarni : la ligne `>` suivait le chargement, ses lignes le
   mode dev.** `dump-autoload --no-dev` avec bamarni en `require-dev` :
   Composer (plugin chargé) écrit la ligne, « Calling onPostAutoloadDump() »
   et les dépréciations, vivacity la ligne seule ; répertoire supprimé :
   l'inverse. **Corrigé** : ses lignes suivent le chargement (niveau normal
   compris) ; cas `bamarni-dev dump-autoload --no-dev` et `bamarni install
   @rm:vendor/bamarni/…` (normal et `-v`). La même dissymétrie existe pour
   package-versions (ses deux lignes et Versions.php suivent le mode dev) :
   **notée**, laissée (écart du niveau normal, fichier compris ; un
   package-versions en `require-dev` est rare).
3. **Majeur — des bornes uniques acceptaient les correctifs à venir.**
   `>=5.3.0 <=8.1.9` aurait accepté une v7.4.22 non lue. **Corrigé** : une
   plage par branche, jusqu'au dernier tag lu de la branche ; tests
   (`v7.4.22`, `v4.0.1`, `11.4.12`, `12.0.0-beta3` refusés).
4. **Mineur — un test lisait le `COMPOSER_HOME` de la machine.**
   **Corrigé** : le répertoire global est un paramètre d'`issue`, le cas
   « non autorisé » refuse explicitement.
5. **Mineur — README** (la liste « ci-dessus » n'existait pas ; cas rendus
   incomplets). **Corrigé**.
6. **Nits** : CHANGELOG (le correctif de bamarni sous `### Fixed`), noms
   périmés du plan, en-tête du harnais : **corrigés**. Le lock figé
   contenait, par laravel/pao → laravel/agent-detector, des mots-clés amont
   nommant des outils : **retiré** (laravel/pao ôté du squelette Laravel +
   Pest).

Trouvé en écrivant ces cas : `dump-autoload` sur un plugin dont le
répertoire a disparu garde le paquet dans l'autoloader, là où Composer le
purge (`Factory::purgePackages`) — reproduit sur `main`, antérieur, noté.

## Critère « vert »

- Les cas ci-dessus verts (et les rouges constatés rouges sur `main`).
- `cargo fmt --all -- --check` ; `cargo clippy --workspace --all-targets --
  -D warnings` (et `--target x86_64-pc-windows-gnu`) ; `cargo test --workspace`.
- Tous les harnais de `ci.yml` à 0 (32 passages, `diff-vendor.sh` trois fois),
  plus `harness/root-manifest.sh` ; ceux qui ne peuvent pas tourner ici sont
  nommés avec la raison et leur sortie.

## Résultat (2026-10-22)

`Linux vm 6.18.44-fc-v140 #1 SMP PREEMPT_DYNAMIC @0 x86_64 x86_64 x86_64 GNU/Linux` — `nproc` : 4.

**Commits** (branche `verbose-plugins`, depuis `main` fe8ad48) : 34e97b9
(plan et matrice), b5e7b38 (méta-analyse), 896056b (cas rouges), 18caed8
(cas mal construits corrigés), fd1ad39 (code et docs), 01dc0ed (revue :
délégation tardive sans doublon, bamarni, bornes par branche), et le commit
de ce résultat.

**Natifs sous `-v`** : composer/installers, roots/wordpress-core-installer,
mnsami/composer-custom-directory-installer, ibexa/post-install,
ergebnis/composer-normalize (aucune ligne) ; symfony/runtime,
pestphp/pest-plugin, php-http/discovery, composer/package-versions-deprecated,
phpstan/extension-installer, rector/extension-installer, codeception/c3,
drupal/core-project-message, drupal/core-recipe-unpack,
bamarni/composer-bin-plugin (leurs lignes `>`, et les deux `Calling …` de
bamarni), dans les versions lues, branche par branche. **Rendus, raison
nommée** : symfony/flex, dealerdirect, wikimedia/composer-merge-plugin,
yiisoft/yii2-composer, symfony/thanks, metasyntactical/composer-plugin-license-check,
tout autre plugin, une version non lue, un plugin global, `--run-scripts`,
phpstan / rector / c3 sous `--no-install`, un plugin qu'amène la résolution
(sur le lock résolu, avant écriture, lignes gardées jusque-là).

**Rouges** (`harness/verbosity.sh online`, binaire de `main` 0.23.0) :

- premier jeu (avant la méta-analyse) : 65 cas×niveaux, 46 rouges — tous
  les cas `-v` à plugin sauf `--no-plugins`, Flex rendu et `-vv` ;
- jeu amendé, première moitié (laravel-pest, runtime, pest, discovery, pv,
  phpstan, et la partie Symfony) : 49, 27 rouges — 25 « vivacity rend la
  commande à Composer », `discovery install @nolock` (les lignes
  `pre-autoload-dump` / `post-update-cmd` manquaient : sans lock ni
  vendor, `main` ne voyait aucun plugin et restait natif), `phpstan update
  --no-install` (raison sans `--no-install`) ; seconde moitié (rector à
  discovery-old) : 33, 31 rouges — seuls Flex et la version 1.15.0 de
  discovery étaient déjà rendus, avec une raison qui convenait ;
- cas de la revue : 6, 6 rouges, dont les deux `bamarni-dev
  dump-autoload --no-dev` et `bamarni install @rm:` **au niveau normal** (les
  lignes de bamarni manquaient ou étaient de trop).

**Verts** (binaire final) : `verbosity.sh` 296 cas×niveaux, 0 échec — 212
de la partie instantané (inchangée, plus `@delegates:symfony/flex`) et 84
de la partie `online` (64 sous `-v`).

**Portes** : `cargo fmt --all -- --check` ; `cargo clippy --workspace
--all-targets -- -D warnings` (Linux et `--target x86_64-pc-windows-gnu`,
mingw-w64) ; `cargo test --workspace --no-fail-fast` 349 réussis, 0 échec
(36 blocs) ; `tools/check-pcre2-symbols.sh` et `tools/check-clib-deps.sh`
OK.

**Harnais** (binaire final, chacun dans son `VIVACITY_HARNESS_DIR`
supprimé après) — les 32 passages de `ci.yml` et root-manifest.sh :

| passage | résultat |
|---|---|
| diff-vendor ×3 (sans, puis deux fois `--with-autoloader`) | 12, 12, 12 OK, code 0 |
| update | 18 OK, 1 échec réseau (drupal : packages.drupal.org refusé) |
| steps | 277 OK ; drupal `require drupal/core-project-message` : code 100 contre 1 sur un refus du proxy (réseau), et le cas suivant non joué (voir défauts) ; les quatre dernières fixtures rejouées à part : 129 OK |
| removal, transitions, path-repos, vendor-dir, merge-plugin, scripts, tar-dist | 3, 13, 14, 6, 18, 6, 7 OK, code 0 |
| yii2-composer | 5 OK, 6 échecs réseau (asset-packagist.org refusé : l'avertissement du dépôt injoignable porte le texte de curl chez Composer, de reqwest chez vivacity) |
| bin-plugin, custom-dirs, package-versions | 13, 4, 7 OK, code 0 |
| wp-core | **ne peut pas tourner ici** : `roots/wordpress-no-content` a un `shasum`, downloads.wordpress.org est refusé (code 100 des deux côtés) |
| plugin-lines | 46 OK, code 0 |
| flex-install, flex-update, flex-recipes | 7, 26, 26 OK, code 0 |
| download-lines | **ne peut pas tourner ici** : il exige un téléchargement réel sur cache vierge, les archives GitHub sont refusées |
| artifact-repo, compare-selftest, two-stage | 9, 11, 12 OK, code 0 |
| transport-exit | 8 OK, 1 échec propre à root ; **en utilisateur non root : 9 OK, code 0** |
| verbosity | 296 OK, code 0 |
| fallback-args, root-scan, boot | 50, 12, 6 OK, code 0 |
| drift-reference | 132 fichiers identiques, code 0 |
| root-manifest | 37 OK, 7 échecs propres à root ; **en utilisateur non root : 44 OK, code 0** |

Adaptations à cet environnement, aucune dans le dépôt : PHP 8.4.27 compilé
(reconstruit contre la PCRE2 du système : avec la PCRE2 embarquée de PHP, le
test `the_real_php_is_seen_and_its_cache_matches_a_fresh_probe` ne trouvait
pas de bibliothèque pcre2 parmi les images, ce que le PHP de la CI fournit) ;
le cache de fichiers amorcé par `git archive` ; les quatre archives
WordPress sans `shasum` reconstruites (0755/0644) ; pour flex-update,
flex-recipes et verbosity, dont les homes Composer ont un cache privé,
`cache/files` lié au cache amorcé par un script hors dépôt ;
`RUST_BACKTRACE=1` posé par l'environnement retiré pour les passages en
utilisateur non root (il ajoute une pile à `Error: …`). « Propre à root » :
Composer ajoute « Plugins have been disabled automatically as you are
running as root… » à une exception sous `--no-plugins` quand il tourne en
root (Application.php), ce que les runners de la CI ne font pas.

**Défauts vus, laissés** :

- `harness/steps.sh:520` : `tail -3 <deux fichiers>` est refusé par le
  `tail` de GNU (« option used in invalid context ») ; sous `set -e` le
  harnais s'arrête au premier code de sortie différent — sans effet tant
  que tout passe, il masque les fixtures suivantes sinon (`tail -n 3`
  suffirait ; hors du sujet de ce chantier) ;
- vivacity répond 1 là où Composer répond 100 quand le proxy refuse le
  `CONNECT` (drupal, cas réseau) : la classification de cette erreur de
  transport n'est pas celle de Composer ;
- un répertoire de zip au mode 0775 : `unzip` (Composer) le pose, vivacity
  pose 0755 (le mode d'une entrée répertoire est ignoré), vu sur des
  archives reconstruites, jamais sur les archives réelles des fixtures ;
- `dump-autoload` garde dans l'autoloader un paquet dont le répertoire a
  disparu, Composer le purge (`Factory::purgePackages`) — reproduit sur
  0.23.0 ;
- package-versions-deprecated en `require-dev` sous `dump-autoload
  --no-dev` : Composer (plugin chargé) écrit ses deux lignes et
  Versions.php, vivacity suit le mode dev ;
- l'`install` sans lock, résolu comme un `update` : sous `-v`, une
  délégation tardive y réimprime encore ses premières lignes ;
- `COMPOSER_DEBUG_EVENTS`, l'auto-installation de php-http/discovery :
  non portés.
