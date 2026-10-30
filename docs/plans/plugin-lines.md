# Les lignes des installeurs de plugins, et leur ordre — plan

> Suite du non-objectif « lignes stdout que vivacity n'imprime pas du tout »
> et de l'« ordre de ces émulations entre elles » de
> `docs/plans/install-order.md`. Niveau 2 : le contrat est la parité à
> l'octet de stdout et de stderr, dans le même ordre ; `post_install` et
> `post_autoload_dump_plugins` servent `install`, `update`, `require`,
> `remove` (et `dump-autoload` pour le second). Méta-analyse et revue par
> sous-agents frais.

## Le problème

Composer 2.10.3 exécute, à `POST_INSTALL_CMD` / `POST_UPDATE_CMD`, les
écouteurs de plugins que vivacity émule ; trois d'entre eux **écrivent sur
stdout** (`$io->write`, niveau normal), vivacity écrit les mêmes fichiers
mais aucune ligne. Mesuré sur `main` (0c56eb6), sortie fusionnée `2>&1`
(cas du harnais `harness/plugin-lines.sh`, ci-dessous) :

- phpstan/extension-installer : « phpstan/extension-installer: Extensions
  installed » **à chaque install** (no-op compris), puis, quand
  `GeneratedConfig.php` change, `> nom: installed`, `> nom: not supported`,
  `> nom: ignored` ;
- rector/extension-installer : « rector/rector-installer: Extensions
  installed » et `> nom: installed`, **seulement quand le fichier change** ;
- dealerdirect/phpcodesniffer-composer-installer : « PHP CodeSniffer Config
  installed_paths set to a,b » ou « … installed_paths delete », seulement
  quand la configuration change.

Trouvé en mesurant, même famille, à `POST_AUTOLOAD_DUMP` :

- composer/package-versions-deprecated : « composer/package-versions-deprecated:
  Generating version class... » puis « …: ...done generating version class »
  **à chaque dump** (install, no-op, `dump-autoload`) ; vivacity n'en
  imprime aucune (le harnais `package-versions.sh` jette stdout).

Et l'ordre, devenu observable : Composer appelle les écouteurs de priorité 0
dans l'ordre où les plugins ont été activés ; vivacity suit un ordre fixe
(dealerdirect, phpstan, rector, c3, Flex à `POST_*_CMD` ; pest, yii2,
bamarni, package-versions à `POST_AUTOLOAD_DUMP`). Mesuré : no-op du projet
« all » — c3 (« already up-to-date ») **avant** phpstan chez Composer, après
chez vivacity ; vendor vierge — c3, rector, dealerdirect, phpstan (ordre de
la transaction) ; retour en dev — dealerdirect, phpstan, rector, puis c3
(installé par la commande, activé à ce moment) ; mise à jour de c3 — c3
**après** phpstan (désactivé puis réactivé). Projet bamarni +
package-versions (requis par jean85/pretty-package-versions) : no-op et
`dump-autoload`, package-versions **avant** `[bamarni-bin]` ; vendor vierge,
bamarni avant.

## Références

Composer 2.10.3 (phar du harnais `/opt/homebrew/Cellar/composer/2.10.3`,
extrait sous le scratchpad, jamais dans le dépôt ; `docs/reference/` en
porte les jumeaux, `drift-reference.sh`) :

- `Factory.php:448` `$pm->loadInstalledPlugins()`, **avant**
  `:457` `purgePackages` : le tri voit installed.json tel qu'écrit.
- `Plugin/PluginManager.php:102-112` `loadInstalledPlugins` : le dépôt local
  puis le global ; `:495-535` `loadRepository` : poids -10000 pour
  `composer/installers` et `extra.plugin-modifies-install-path` (type
  `composer-plugin`), `:509` `PackageSorter::sortPackages($packages,
  $weights)`, puis `registerPackage` dans cet ordre (seulement
  `CompletePackage` : jamais un alias ; types `composer-plugin` /
  `composer-installer`) ; `:169-330` `registerPackage` (non autorisé : pas
  chargé ; classe introuvable : pas chargé, sans erreur) ; `:412-439`
  `addPlugin` → `EventDispatcher::addSubscriber` ; `:351-374`
  `deactivatePackage`, `:376-399` `uninstallPackage` → `removeListener`.
- `Util/PackageSorter.php:74-139` (`docs/reference/PackageSorter.php`) :
  poids = `$weights[$name] ?? 0` moins, par utilisateur (paquet qui le
  requiert), `1 - importance(utilisateur)` ; tri par poids croissant puis
  `strnatcasecmp` des noms. Déjà porté sans `$weights` :
  `crates/vivacity-autoload/src/sorter.rs`.
- `Installer/PluginInstaller.php:80-90` `install` → `registerPackage` après
  la pose ; `:100-111` `update` → `deactivatePackage($initial)` puis
  `registerPackage($target)` (donc **en fin de liste**) ; `:118-120`
  `uninstall` → `uninstallPackage`.
- `Installer/InstallationManager.php:300-323` : chaque opération sur un
  `composer-plugin`/`composer-installer` est son propre lot, exécuté dans
  l'ordre de la transaction — l'activation suit l'ordre des opérations,
  pas l'achèvement des extractions.
- `Repository/FilesystemRepository.php:105-108` (chargement par
  `ArrayLoader::load` : un paquet dev à `branch-alias` devient un
  `CompleteAliasPackage` **plus** son paquet, `ArrayRepository.php:215-231`),
  `:156` installed.json écrit trié par nom.
- `Package/Loader/ArrayLoader.php:429-487` `getBranchAlias` (porté :
  `vivacity_resolver::loader::branch_alias`).
- `EventDispatcher.php:537-596` (install-order.md) : ordre d'ajout par
  priorité ; scripts de composer.json à la fin de la priorité 0.

Plugins (sources des versions verrouillées, téléchargées sous le
scratchpad ; les références `docs/reference/plugins/` sont celles de la
version la plus récente du corpus) :

- **phpstan/extension-installer 1.4.3** `src/Plugin.php:83-89` (les deux
  événements), `:95-98` (« Package not found (probably scheduled for
  removal); extensions installation skipped. » si `src/` manque), `:105-107`
  (empreinte md5 de l'ancien fichier), `:197-198` (écrit toujours, puis la
  ligne « Extensions installed »), `:200-214` (si l'empreinte a changé :
  `installed` dans l'ordre `ksort`, `not supported` dans l'ordre `ksort`,
  `ignored` dans l'ordre `sort` — une entrée par paquet du dépôt local, un
  alias de branche compte deux fois). Versions : 1.4.0, 1.4.2, 1.4.3
  identiques (diff) ; **1.3.1 n'écrit pas `PHPSTAN_VERSION_CONSTRAINT`**
  (le fichier diffère : vivacity l'écrit faux aujourd'hui). Corpus : 1.4.3
  seulement (16 projets).
- **rector/extension-installer 0.11.2** `src/Plugin.php:28-59`,
  `src/PluginInstaller.php:60-105` : `hashEquals(md5 ancien, nouveau)` →
  rien ; sinon écrit, puis « rector/rector-installer: Extensions installed »
  (sic) et `> nom: installed` (`ksort`). 0.11.0, 0.11.1, 1.0.0 identiques
  (1.0.0 : `readonly`, `getInstallPath` nul ignoré — un métapaquet) ;
  **0.10.2 a un autre gabarit**. Corpus : 0.11.2 seulement (2 projets).
- **dealerdirect/phpcodesniffer-composer-installer v1.2.1**
  `src/Plugin.php:161-171`, `:181-224` (`onDependenciesChangedEvent` :
  verbeux seulement, sauf `saveInstalledPaths`), `:263-315`
  (`saveInstalledPaths` : « PHP CodeSniffer Config installed_paths set to
  <chemins triés, joints par des virgules> » ou « … installed_paths
  delete » si la sauvegarde et `verifySaveSuccess` réussissent, « Failed to
  set PHP CodeSniffer installed_paths Config » sinon ; le résultat de phpcs
  en verbeux). Les balises `<info>`/`<comment>` disparaissent (sortie non
  décorée). Corpus : v0.7.2 (chamilo), v1.0.0 (2), v1.1.1 (1), v1.2.1 (12).
  Différences, par diff des sources : **v1.0.0–v1.2.0** gardent un chemin
  relatif enregistré s'il existe relativement au **répertoire courant** OU à
  phpcs (`$alternativePath`, l. 418-429 en 1.0.0, l. 408-418 en 1.2.0) —
  v1.2.1 relativement à phpcs seulement ; v1.1.0–v1.1.1 testent la présence
  de phpcs par `InstalledVersions::isInstalled`, et **v1.1.0 seule** prend
  le chemin de phpcs dans `InstalledVersions::getInstallPath` (l. 558) ; **v0.7.x cherche aussi les
  `ruleset.xml` dans tout le projet** (`$searchPaths = array($this->cwd)`,
  l. 436 : vendor/<v>/<p>/ruleset.xml compris, profondeur ≤ 3), et lance
  `phpcs --config-show` depuis bin-dir.
- **composer/package-versions-deprecated 1.11.99.5**
  `src/PackageVersions/Installer.php:159-161` (`POST_AUTOLOAD_DUMP`),
  `:200-231` (`writeVersionClassToFile` : « Package not found (probably
  scheduled for removal); generation of version class skipped. » si le
  répertoire manque, sinon « Generating version class... », l'écriture,
  « ...done generating version class »).
- bamarni/composer-bin-plugin 1.9.1 `BamarniBinPlugin.php:78-84` ;
  codeception/c3 2.9.0 `Installer.php:54-65` ; symfony/flex
  `Flex.php:893-913` : déjà émulés, lignes déjà imprimées ; seul leur rang
  change.

## Matrice mesurée (Composer 2.10.3, PHP 8.5.10, macOS)

Lignes sur stdout, niveau normal (`$io->write`), toutes après le funding
(ou, pour package-versions, juste après « Generating autoload files »).
« — » : aucune ligne.

| plugin | condition | lignes |
|---|---|---|
| phpstan | vendor vierge | Extensions installed ; `> phpstan/phpstan-strict-rules: installed` ; `> phpstan/php-8-stubs: not supported` ; `> phpstan/phpstan-deprecation-rules: ignored` |
| phpstan | no-op | Extensions installed |
| phpstan | `--no-dev` (extensions en dev) | Extensions installed (le fichier change, plus rien à lister) |
| phpstan | retour en dev | comme vendor vierge |
| phpstan | `allow-plugins` false, `--no-plugins` | — |
| rector | vendor vierge, retour en dev | rector/rector-installer: Extensions installed ; `> rector/rector-symfony: installed` |
| rector | no-op | — |
| rector | `--no-dev` | rector/rector-installer: Extensions installed |
| dealerdirect | vendor vierge, retour en dev | PHP CodeSniffer Config installed_paths set to ../../phpcsstandards/phpcsextra,../../phpcsstandards/phpcsutils |
| dealerdirect | no-op (configuration inchangée) | — |
| dealerdirect | CodeSniffer.conf supprimé puis no-op | set to … (même ligne) |
| dealerdirect | `--no-dev` (standards en dev) | PHP CodeSniffer Config installed_paths delete |
| package-versions | install, no-op, `dump-autoload` | Generating version class... ; ...done generating version class |
| tous | `-q` | — |
| tous | `-v` | une ligne `> post-install-cmd: Classe->méthode` (stderr) par écouteur, plus les lignes verbeuses de dealerdirect : **déjà rendu à Composer** par vivacity (`fallback::active_plugin`) |
| tous | `update` avec installation | comme `install` (vendor vierge : ordre de la transaction) ; `update` sans changement : comme le no-op, sauf c3 (`askForUpdateV2` muet quand c3.php est à jour) |
| drupal/core-project-message | `post-install-cmd-message` configuré, `install` | le message, à `POST_INSTALL_CMD` (rien sans configuration) |

Ordre (projet « all » : phpstan, dealerdirect, rector en `require` ; c3 et
les extensions/standards en `require-dev`, slevomat et phpcsutils
requièrent dealerdirect) :

| condition | ordre chez Composer | ordre fixe de vivacity |
|---|---|---|
| vendor vierge (`install`, `update`) | c3, rector, dealerdirect, phpstan (transaction) | dealerdirect, phpstan, rector, c3 |
| no-op | dealerdirect (poids négatif), c3, phpstan, rector (noms) — seuls c3 et phpstan écrivent | dealerdirect, phpstan, rector, c3 |
| no-op, c3.php et CodeSniffer.conf perdus (`install` et `update`) | dealerdirect « set to », c3 « Copying », phpstan | dealerdirect, phpstan, c3 |
| `--no-dev` | « Deleting c3.php » à la désinstallation ; dealerdirect, phpstan, rector | idem |
| retour en dev | dealerdirect, phpstan, rector (chargés), c3 (installé) | idem (coïncidence) |
| c3 2.8.1 → 2.9.0 | dealerdirect, phpstan, rector, c3 (réactivé) | idem (coïncidence) |
| bamarni + package-versions, no-op / dump | package-versions (poids -1), bamarni | bamarni, package-versions |
| bamarni + package-versions, vendor vierge | bamarni, package-versions (transaction) | idem |

## Le modèle porté

L'ordre des écouteurs d'un événement, pour une commande :

1. `loadInstalledPlugins` (sauf `--no-plugins`) : les paquets
   d'installed.json **tel que lu**, avant la purge ; chaque paquet dev dont
   `getBranchAlias` répond compte deux fois (l'alias et lui) dans la table
   des usages ; poids -10000 aux `composer-plugin` nommés
   `composer/installers` ou à `extra.plugin-modifies-install-path` vrai ;
   `sortPackages` ; on garde, dans cet ordre, les `composer-plugin` /
   `composer-installer` dont les classes sont trouvables — approché par
   « le répertoire de `install-path` existe » (sinon `registerPackage` ne
   trouve pas la classe et ne charge rien, sans ligne). Puis le dépôt
   global (`COMPOSER_HOME`), même règle, sans poids d'usage croisé (Flex
   global est le seul émulé qui en dépende).
2. Les opérations de la transaction, dans l'ordre : `Uninstall` d'un plugin
   → retiré ; `Update` → retiré, puis remis **à la fin** si la cible est un
   plugin ; `Install` d'un plugin → ajouté à la fin.
3. Un plugin non autorisé n'est jamais chargé : les émulations le vérifient
   déjà (`plugin_allowed`), le laisser dans la liste ne change rien.

`post_install` parcourt cette liste et joue, à son rang, l'émulation de
dealerdirect, phpstan, rector, c3, Flex ; `post_autoload_dump_plugins` (et
`dump-autoload`, liste = étape 1 seule) celle de bamarni et de
package-versions — les émulations muettes (pest, yii2-composer,
symfony/runtime) gardent leur place, leur effet ne dépend d'aucune autre.

## Versions non reproduites : repli nommé

Avant toute écriture (`scope::analyze`, plugin présent, autorisé, plugins
actifs), une version dont le code n'a pas été lu va à Composer avec la
raison (`scope::EMULATED_VERSIONS`, liste exacte des versions lues — une
branche dev aussi, son code peut avoir bougé) :

- phpstan/extension-installer : 1.4.0, 1.4.1, 1.4.2, 1.4.3 (1.3.x écrit un
  autre fichier) ;
- rector/extension-installer : 0.11.0, 0.11.1, 0.11.2, 1.0.0 (0.10.x :
  autre gabarit) ;
- dealerdirect/phpcodesniffer-composer-installer : v1.0.0, v1.1.1, v1.1.2,
  v1.2.0, v1.2.1 (0.x cherche dans tout le projet, v1.1.0 demande à
  `InstalledVersions` où est phpcs).

Et porté : la règle `$alternativePath` de v1.0.0–v1.2.0 (un chemin relatif
gardé s'il existe relativement au projet).

Conséquence, notée : sur `update`/`require`/`remove`, une telle version
rencontrée à la phase d'installation (lock déjà écrit par vivacity) rend
`composer install` à Composer — c'est `POST_INSTALL_CMD`, pas
`POST_UPDATE_CMD`, qui part alors (c3 copie avec `copyC3V2`), comme pour
tout repli de la phase d'installation aujourd'hui. Corpus : chamilo
(v0.7.2), classé `unavailable` jusqu'ici (phpcs n'y tourne pas sous PHP
8.5), passe au repli ; le corpus n'est pas rejoué dans ce chantier (il
tourne dans le dépôt principal pour une autre tâche).

## Fichiers, dans l'ordre

1. `harness/plugin-lines.sh` (nouveau, ajouté à `ci.yml`) : les cas rouges
   d'abord (ci-dessous), sur le binaire de `main`.
2. `crates/vivacity-autoload/src/sorter.rs` : `sort_packages_weighted`
   (`$weights`), `sort_packages` l'appelle sans poids ; test.
3. `crates/vivacity/src/plugin_order.rs` (nouveau) : `installed_json_packages`,
   `loaded_plugins` (étape 1, local puis global), `activation_order`
   (étape 2), `listeners_in_order` ; tests unitaires (poids d'usage,
   -10000, alias de branche, plugin absent du disque, global, mise à jour,
   désinstallation).
4. `crates/vivacity/src/extension_installers.rs` : `phpstan` et `rector`
   impriment leurs lignes (`out!`), l'écriture rend « changé ou non » ; les
   doublons `ignored` d'un alias (de branche, de branche par défaut, ou
   alias en ligne du lock). Pas de ligne « Package not found » : sans
   `src/`, Composer ne charge pas le plugin (méta, point 5).
5. `crates/vivacity-core/src/phpcs_installer.rs` : `register_standards`
   rend un `ConfigChange` (`Unchanged`, `Set(chemins)`, `Deleted`, et sa
   ligne) ; `keeps_cwd_relative_paths` (< v1.2.1) ; `installed_paths` vide
   lu comme aucun chemin (`trim`, `!== ''`).
6. `crates/vivacity/src/install.rs` : `LocalTransaction.listeners` (l'ordre
   d'activation, installed.json lu une fois avant la purge,
   `purge_missing`) ; `post_install` → `post_install_listener` dans cet
   ordre ; `post_autoload_dump_printers` (bamarni, package-versions) dans
   cet ordre ; `emulate_package_versions` imprime ses lignes ; la ligne et
   la suppression de c3 sur `remove` (ci-dessous) ; le repli de
   drupal/core-project-message dans `analyze_scope` (`install` seulement).
7. `crates/vivacity/src/dump.rs` : `post_autoload_dump_printers` avec
   l'étape 1 seule.
8. `crates/vivacity/src/{lib,remove,update,require}.rs` :
   `UpdateArgs.allow_plugins_at_load` → `UpdateReport` → le manifeste de
   la phase d'installation — trouvé par le cas `phpstan-remove` :
   `remove codeception/c3` retire l'entrée `allow-plugins` du plugin
   **après** que le `Composer` de la commande l'a chargé
   (RemoveCommand.php:216-222 puis :242-252) ; Composer appelle donc son
   `uninstall` (« Deleting c3.php », fichier supprimé), vivacity non (il
   relisait le manifeste édité). Écart antérieur à ce chantier. Les règles
   sont rendues telles quelles, ordre compris (revue, second passage).
9. `crates/vivacity-core/src/scope.rs` : `EMULATED_VERSIONS` et le repli de
   version ; `core_project_message_issue` ; le commentaire
   d'`EMULATED_PLUGINS` corrigé.
10. `docs/reference/{PluginInstaller,InstallationManager,EventDispatcher}.php`
    (copies du phar) et `harness/drift-reference.sh` (`twin()`) : le modèle
    en dépend, la dérive les surveille (méta, point 14).
11. Docs dans le même commit : `CHANGELOG.md` (`[Unreleased]` → `### Fixed`),
    `DECISIONS.md`, `HANDOVER.md`, `README.md`, `docs/plans/install-order.md`,
    ce plan.

## Cas de harnais

`harness/plugin-lines.sh` (Packagist en ligne, versions exactes, lock écrit
par Composer `--no-plugins` puis copié des deux côtés,
`--no-security-blocking`) : code de sortie, projet entier
(`compare_vendor`), sortie fusionnée moins `^vivacity: `, `^Note: plugin `
(la note de vivacity sur un plugin bénin, comme flex-install.sh) et
`^  - Downloading `, plus une assertion de cohérence par cas sur la sortie
de Composer (motif présent, absent, ou ordre — chaque motif cherché après
le précédent) ; COMPOSER_HOME vide (auth.json recopié, cache de fichiers
habituel). 46 cas :

- phpstan seul : `install`, `no-op`, `quiet`, `no-dev`, `re-dev`,
  `require` (c3 ajouté : activé après phpstan), `remove` (c3 retiré :
  « Deleting », plus d'écouteur ; `codeception/*: false` placé après la
  règle de c3), `not-allowed` ;
- rector seul : `install`, `no-op`, `no-dev`, `re-dev` ;
- dealerdirect seul : `install`, `no-op` (sans changement), `conf-lost`
  (avec changement), `no-dev` (« delete »), `re-dev` ;
- les trois et c3 (« all ») : `install` (vendor vierge), `no-op`,
  `weights` (c3.php et CodeSniffer.conf perdus : dealerdirect, c3,
  phpstan), `no-autoloader`, `quiet`, `verbose` (rendu à Composer),
  `no-dev`, `re-dev`, `c3-old` puis `c3-update` (c3 réactivé en fin),
  `no-plugins`, `update` (vendor vierge), `update-again` (sans changement,
  fichiers perdus) ;
- dealerdirect v1.2.1 : `cwd-relative-dropped` (un chemin relatif au projet
  retiré) ; v1.0.0 : `install`, `cwd-relative` (`$alternativePath`, gardé) ;
- `old-versions` (dealerdirect v0.7.2, phpstan 1.3.1, rector 0.10.2,
  package-versions 1.11.99.1 : code 3 sous `--no-fallback`, quatre raisons,
  rien d'écrit) ; `aliased` (strict-rules `2.0.4 as 2.0.99`, repli) ;
- `remove-required-install`, `remove-required` (`remove` d'un plugin qu'un
  paquet `path` requiert encore : chargé, il écrit, puis « Removal
  failed », code 2 ; l'entrée `allow-plugins` retirée était la seule) ;
- drupal/core-project-message : `message` (repli nommé), `message-none`
  (natif, rien) ;
- package-versions : `install`, `no-op`, `dump`, `quiet` ;
- bamarni + package-versions : `install`, `no-op`, `dump`.

## Critère « vert »

- Les rouges ci-dessus verts, les garde-fous verts.
- `cargo fmt --all -- --check` ; `cargo clippy --workspace --all-targets --
  -D warnings` (macOS et `--target x86_64-pc-windows-gnu`) ;
  `cargo test --workspace`.
- Tous les harnais de `ci.yml` à 0 (dont `diff-vendor.sh` trois fois,
  `package-versions.sh`, `yii2-composer.sh`, `bin-plugin.sh`, `scripts.sh`,
  `flex-*`, `verbosity.sh`), plus `harness/root-manifest.sh` et le nouveau.

## Non-objectifs (vus, laissés)

- `-v` : déjà rendu à Composer dès qu'un plugin autorisé est actif (les
  lignes `> event: Classe->méthode` et les lignes verbeuses de dealerdirect,
  dont la sortie de phpcs) — inchangé.
- dealerdirect quand phpcs ne tourne pas sous le PHP de la machine :
  Composer imprime « Failed to set … » et n'écrit rien ; vivacity n'exécute
  jamais PHP, écrit et imprime « set to ». Écart existant (fichier), noté
  dans HANDOVER ; le corpus classe ces cas `unavailable`.
- dealerdirect v1.1.0–v1.1.1 (`InstalledVersions::isInstalled`) : diffère
  du dépôt local seulement si phpcs est remplacé ou fourni par un autre
  paquet — non porté, noté.
- **traité le 2026-10-22 (`docs/plans/no-install-listeners.md`, commit f8258b6)** —
  `update --no-install` : Composer joue `POST_UPDATE_CMD` (seul le dump
  est coupé, Installer.php:306-308), phpstan y écrit « Extensions
  installed », c3, dealerdirect et les extension-installers leurs
  fichiers ; vivacity ne joue aucun écouteur. Non-objectif
  d'install-order.md, inchangé — désormais l'écart le plus visible de
  phpstan ; suite proposée : jouer les écouteurs sur le dépôt local
  inchangé, ou rendre la main quand l'un d'eux est actif.
- `composer run-script` sous `--run-scripts` : non-objectif
  d'install-order.md, inchangé.
- Le mode exact de l'ordre du dépôt local pour un paquet aliasé (porté
  ici en repli) ; la ligne « not writable » de package-versions ; un
  `vendor-dir` global posé dans `COMPOSER_HOME/config.json` seulement.
- La ligne « Dependency … is also a root requirement » de `remove`
  imprimée après « Updating dependencies » chez vivacity, avant chez
  Composer (vue par la revue, autre famille). Réglé le 2026-10-29
  (docs/plans/parity-gaps-026.md).
- Les plugins globaux émulés (dealerdirect installé dans `COMPOSER_HOME`) :
  les émulations ne lisent que le dépôt local — écart existant, la liste
  d'activation les place correctement mais rien ne les joue.

## Méta-analyse (sous-agent frais, sur le plan et le code, 2026-10-21) — et ce qui change

Pas de bloquant ; trois majeurs, huit mineurs, quatre nits. Le relecteur a
vérifié le modèle d'activation contre les sources (chargement avant la
purge, alias jamais enregistrés, un lot par plugin, `update` en fin,
`require`/`remove` qui rechargent les plugins depuis installed.json) et
toutes les priorités (0 partout, sauf composer-merge-plugin à 50000, muet
hors premier install déjà rendu).

- **dealerdirect v1.2.0 a encore `$alternativePath`** (majeur) : **adopté**,
  la règle vaut jusqu'à v1.2.0, v1.2.1 seule la change.
- **Le harnais prouvait moins l'ordre qu'annoncé** (majeur) :
  `all-update-again` passait avec l'ordre fixe (c3 muet à `POST_UPDATE_CMD`
  quand c3.php est à jour), et le poids de dealerdirect ne se voyait jamais.
  **Adopté** : `all-weights` et `all-update-again` perdent c3.php et
  CodeSniffer.conf avant de tourner (dealerdirect, c3, phpstan écrivent) ;
  ligne de la matrice corrigée.
- **Replis de version sans cas rouge, corpus, branches dev** (majeur) :
  **adopté** — `old-versions`, `phpcs-v100-*`, règle « branche dev →
  repli » (liste exacte des versions lues), conséquence corpus écrite.
- **`update --no-install`** (mineur) : non-objectif gardé, formulation
  rendue exacte.
- **« Package not found » de phpstan** (mineur) : **adopté**, pas de ligne
  (sans classe, pas de plugin chargé).
- **dealerdirect v1.1.0** (`InstalledVersions::getInstallPath`) : **adopté**,
  hors liste.
- **Alias** (mineur) : doublons `ignored` portés (alias de branche, alias en
  ligne) ; la version écrite → voir la revue.
- **drupal/core-project-message** bénin mais bavard (mineur) : **adopté**,
  repli nommé quand il écrirait.
- **Repli de version sur `update`** (mineur) : conséquence écrite ; puis
  (revue) contrôle dès la résolution.
- **Comptes du plan** (mineur) : corrigés.
- **package-versions sans garde de version** (mineur) : **adopté** après
  diff des sources (1.11.99.4 et .5 identiques ; .1 à .3 autre gabarit).
- **Nits** : cas `require`/`remove`, `--no-autoloader`, assertion de
  `all-c3-old` **ajoutés** ; `installed_paths` vide lu comme aucun chemin
  **corrigé** ; jumeaux `PluginInstaller`, `InstallationManager`,
  `EventDispatcher` **ajoutés** à `docs/reference/` ; liste `ignore` non
  textuelle : équivalente (`in_array` strict), laissée ; Flex avec un autre
  plugin qui écrit, `windows.yml` : non ajoutés (le cas Flex demande
  l'index servi ; le harnais est en ligne et n'a tourné que sur macOS).

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-21) — et ce qui a changé

Premier passage : pas de bloquant, trois majeurs, sept mineurs, des nits ;
le relecteur a rejoué Composer 2.10.3 contre une version de débogage de la
branche.

- **`remove` : seul c3 voyait les règles du chargement** (majeur) :
  **corrigé autrement** — les règles `allow-plugins` du chargement sont
  rendues au manifeste de la phase d'installation, pour tout plugin
  (`allow_plugins_at_load`) ; cas `remove-required`.
- **Alias : la version écrite dépend de l'ordre du dépôt local** (majeur) :
  mesuré (alias en ligne d'une archive : vendor vierge → version du paquet,
  no-op suivant → version de l'alias et une ligne ; `path` + alias de
  branche : l'alias ; alias de branche par défaut d'une archive : le
  paquet, avec ou sans `unzip`) ; **repli nommé** pour un paquet listé et
  aliasé, sauf l'alias de branche par défaut d'une archive ; cas `aliased`.
- **`update --no-install`** (majeur) : non-objectif gardé (voir plus haut).
- **package-versions sans garde** (mineur) : **adopté** (déjà décidé à la
  méta).
- **installed.json sans `install-path`** (Composer 1), **plugins globaux
  sans `COMPOSER_HOME/composer.json`**, **`Install` d'un plugin déjà
  chargé** (mineurs) : **corrigés**, tests unitaires.
- **Repli de version après la résolution** (mineur) : **corrigé** —
  `resolution_issues` contrôle les versions installées avant de résoudre
  (conservateur : toute version non lue, quoi que fasse la commande).
- **Harnais non isolé, trous** (mineur) : COMPOSER_HOME isolé ; cas
  `phpcs-cwd-relative-dropped`, `aliased`, `remove-required` ; `all-verbose`
  vérifie que rien n'est écrit. Pas de cas global ni `--no-install`.
- **`include-keys` trop large** (mineur) : **corrigé** (seulement les clés
  que la racine renseigne).
- **Nits** : test de `local_repository_entries` **ajouté** ; `exists()`
  limité aux plugins ; ligne de trace pour un écouteur hors de l'ordre ;
  doc de `ScopeIssue::Layout` élargie ; installed.json relu par
  `dump-autoload` : servi par le cache de `jsonfile`, laissé.

Second passage (sur les corrections) : un mineur — les entrées remises en
fin de `allow-plugins`, alors que la première règle qui correspond gagne
(`{"codeception/c3": true, "codeception/*": false}`) : **corrigé**, l'objet
d'origine est rendu tel quel, et `phpstan-remove` le couvre (rouge sur le
binaire précédent) ; trois nits — commentaire de `resolution_issues`
**corrigé**, alias en ligne rattachés par nom seul comme
`Locker::getLockedRepository` **adopté**, `vendor-dir` de
`COMPOSER_HOME/config.json` **noté**.

## Résultat (2026-10-21, commit e98601c, rebasé sur 82313ce)

**Rouges** (`harness/plugin-lines.sh`, 46 cas) :

- binaire de `main` (0c56eb6) : 38 FAIL, 8 OK — les garde-fous
  `phpstan-quiet`, `rector-no-op`, `phpcs-no-op`, `all-quiet`,
  `all-verbose`, `all-no-plugins`, `message-none`, `pv-quiet` ;
- binaire intermédiaire (lignes imprimées, ordre fixe, sans replis) :
  15 FAIL, dont neuf d'ordre — `all-install`, `all-no-op`, `all-weights`,
  `all-no-autoloader`, `all-c3-old`, `all-update`, `all-update-again`,
  `pvb-no-op`, `pvb-dump` — plus `phpstan-remove` et `phpstan-not-allowed`
  (c3.php laissé par `remove`), `old-versions`, `aliased`, `message`,
  `remove-required` ; `phpstan-require`, `all-re-dev`, `all-c3-update`
  passent avec l'ordre fixe (il y coïncide) ;
- binaire d'avant la revue : 3 FAIL (`old-versions` — package-versions —,
  `aliased`, `remove-required`) ; binaire d'avant le second passage : 2 FAIL
  (`phpstan-remove`, `phpstan-not-allowed` : règle `codeception/*` remise
  devant) ;
- `$alternativePath` : un binaire sans le port écrit et imprime « set to
  ../../phpcsstandards/phpcsutils » là où Composer (v1.0.0) garde
  `vendor/phpcsstandards/phpcsutils` (mesuré à la main, même projet que
  `phpcs-v100-cwd-relative`).

**Verts** sur le binaire final : 46/46.

**Portes** : `cargo fmt --all -- --check` ; clippy `-D warnings` macOS et
`x86_64-pc-windows-gnu` ; `cargo test --workspace` 344 réussis, 0 échec
(36 blocs). **Harnais** (binaire final, chacun dans son répertoire,
supprimé après) : les 32 passages à 0 — diff-vendor (12), diff-vendor
`--with-autoloader` deux fois (12, 12), update (19), steps (279), removal
(3), transitions (13), path-repos (14), vendor-dir (6), merge-plugin (18),
scripts (6), tar-dist (7), yii2-composer (11), bin-plugin (13), wp-core
(5), custom-dirs (4), package-versions (7), **plugin-lines (46)**,
flex-install (7), flex-update (26), flex-recipes (26), download-lines (2),
artifact-repo (9), compare-selftest (11), two-stage (12), transport-exit
(9), verbosity (212), fallback-args (50), root-scan (12), boot (6),
drift-reference (132 fichiers identiques), root-manifest (44) — 913 lignes
`OK`, 0 `FAIL`. Le job Windows n'a pas tourné ici, et `plugin-lines.sh`
n'y est pas ajouté (Packagist en ligne, mesuré sur macOS seulement).

**Suites proposées** : jouer les écouteurs `POST_UPDATE_CMD` sous
`update --no-install` (la ligne de phpstan d'abord — **fait le 2026-10-22,
`docs/plans/no-install-listeners.md`**) ; porter l'ordre du
dépôt local d'un paquet aliasé (asynchrone pour une archive, synchrone pour
`path`, alias en ligne réajoutés) au lieu du repli ; rendre à l'avance
seulement les versions que la commande garde (`resolution_issues`) ; la
ligne « Dependency … is also a root requirement » de `remove` ; un cas
Flex + autre plugin qui écrit ; `plugin-lines.sh` sous Windows.
