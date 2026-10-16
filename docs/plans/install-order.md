# Ordre des événements d'`install` et d'`update` — `COMMAND` de bamarni, funding, `POST_*_CMD` — plan

> Suite des « Défauts vus » 2 et 3 de `docs/plans/split-run-install.md`.
> Niveau 2 : le contrat est la parité à l'octet de stderr et de stdout, dans
> le même ordre ; `run_install` sert `install`, `update`, `require` et
> `remove`. Méta-analyse et revue par sous-agents frais.

## Le problème

Deux défauts notés au découpage de `run_install`, et ce que la vérification
du chemin `update` a trouvé à côté. Tous mesurés sur `main` (d63b462) contre
Composer 2.10.3, sortie standard et sortie d'erreur **fusionnées** (`2>&1` :
Composer écrit sans tampon, vivacity ligne à ligne, l'ordre entre les deux
flux est donc déterministe des deux côtés).

1. **`COMMAND` de bamarni/composer-bin-plugin à la mauvaise place.**
   - `install --run-scripts` (plugin déjà installé, `pre-install-cmd` qui
     écrit) : Composer imprime la ligne `[bamarni-bin] …` avant la ligne
     `> echo …` du script ; vivacity après.
   - `update`, `update --dry-run` : Composer l'imprime avant « Loading
     composer repositories with package information » ; vivacity après
     « Writing lock file » (ou « Nothing to modify in lock file »), au début
     de sa phase d'installation.
   - `update --no-install` : Composer l'imprime ; vivacity jamais (pas de
     phase d'installation).
   - `require` : Composer l'imprime après « Running composer update … » ;
     `remove` : avant « Running composer update … ». vivacity : dans la
     phase d'installation, comme pour `update`.
2. **Les écouteurs `POST_INSTALL_CMD` / `POST_UPDATE_CMD` avant le funding.**
   Le seul émulé qui écrit en cas nominal est codeception/c3 (stdout :
   « Copying c3.php… », « c3.php is already up-to-date »). vivacity le joue
   dans les plugins `post-autoload-dump` (avant le script
   `post-autoload-dump`, avant le funding, et pas du tout sous
   `--no-autoloader`) ; Composer après le funding. Mesuré : install, no-op
   (fixture yii2-composer, 17 paquets avec `funding`), et `update` d'un
   projet c3 + un paquet avec `funding`. dealerdirect et les
   extension-installers sont joués avant le funding aussi ; ils écrivent
   chez Composer (voir les non-objectifs : phpstan imprime « Extensions
   installed » à chaque install, mesuré), **pas chez vivacity**, qui
   n'imprime aucune de leurs lignes — l'ordre corrigé ici est celui où
   elles iront. Flex est déjà après le funding. `post-install-cmd` est déjà après le funding (le cas « funding +
   script qui écrit » de la consigne est **vert sur `main`** : c'est un
   garde-fou, pas un rouge).
3. **Trouvé en vérifiant le chemin `update` : les suggestions (et les
   abandonnés) après le dump.** `Installer::run` imprime « N package
   suggestions were added by new dependencies… » et les paquets abandonnés
   avant « Generating autoload files » ; vivacity les imprime après toute la
   phase d'installation (`print_post_update`). Mesuré : `require
   monolog/monolog:3.9.0` (10 suggestions). Même fonction, même cause : la
   fin d'`Installer::run` d'une mise à jour est imprimée hors de
   `run_install`. Corrigé avec le point 2, qui exige de toute façon de
   ramener le funding d'`update` dans `run_install`.

4. **Trouvé par le cas `funding-script` (ci-dessous) : « Generating autoload
   files » après `pre-autoload-dump`.** `Installer::run` imprime la ligne
   (l. 377-381) avant `AutoloadGenerator::dump`, qui dispatche
   `PRE_AUTOLOAD_DUMP` ; `dump_autoloader` lance le script
   `pre-autoload-dump` d'abord. Visible avec `--run-scripts` et un
   `pre-autoload-dump` déclaré (la fixture scripts en a un) : le journal des
   événements était identique, l'ordre des lignes jamais comparé.
   `dump-autoload` (`dump.rs`) a déjà le bon ordre. Deux lignes échangées.

**Non-objectifs** (vus, mesurés quand c'est indiqué, laissés ; détail et
suites proposées dans le résultat) :

- les lignes stdout que vivacity n'imprime pas du tout : phpstan
  (`Plugin.php:198`, « phpstan/extension-installer: Extensions installed »
  à chaque `POST_*_CMD`, puis `> nom: installed` quand le fichier change —
  **mesuré** : un projet phpstan/extension-installer 1.4.3 +
  phpstan-strict-rules 2.0.4, deux lignes chez Composer, aucune chez
  vivacity), rector (`PluginInstaller.php:100-104`, quand le fichier
  change), dealerdirect (`Plugin.php:304-305`, « PHP CodeSniffer Config
  installed_paths set to … » quand il écrit) ;
- l'ordre de ces émulations entre elles : Composer suit l'activation des
  plugins (`PackageSorter::sortPackages` sur le dépôt local, locaux avant
  globaux, un plugin installé pendant la commande réenregistré à ce
  moment, donc après les autres) ; vivacity garde dealerdirect, phpstan,
  rector, c3, Flex. Seuls c3 et Flex écrivent chez vivacity : c3 avant Flex
  est l'ordre de Composer quand les deux étaient installés (poids égaux,
  ordre alphabétique), pas quand c3 arrive pendant la commande ;
- les écouteurs de plugins rejoués par `composer run-script` sous
  `--run-scripts` (mesuré : yii2-composer + `post-install-cmd` déclaré,
  « [codeception/c3] c3.php is already up-to-date » en plus chez vivacity),
  et `pre-install-cmd` lancé avant la décision de portée (un lock rendu à
  Composer le rejoue) ;
- `update --no-install` : Composer dispatche `POST_UPDATE_CMD`, donc
  dealerdirect, les extension-installers et c3 **écrivent des fichiers**
  (c3.php copié s'il manque ou diffère, `askForUpdateV2`) ; vivacity non ;
- `forward-command: true` et un `extra.bamarni-bin` mal typé sur le chemin
  `update` (Composer renvoie `update` à vendor-bin/ ou s'arrête sur
  l'exception ; vivacity résout) ;
- les avertissements de `BaseCommand::initialize`
  (`COMPOSER_IGNORE_PLATFORM_REQ(S)`), imprimés par vivacity dans
  `resolve_and_lock` : après « has been updated »/« Running composer
  update » pour `require`/`remove`, et jamais quand `prepare_update`
  refuse ;
- le manifeste passé à `COMMAND` : fusionné par composer-merge-plugin pour
  `install`, brut pour update/require/remove (Composer fusionne à `INIT`,
  avant) ;
- les plugins globaux filtrés par les `allow-plugins` de la racine
  (`active_plugins`), limite existante que `COMMAND` étend à
  update/require/remove ;
- une résolution rendue à Composer après coup (Flex,
  `hand_resolution_over`) : la ligne `[bamarni-bin]` sort une fois chez
  vivacity puis une fois chez Composer, comme « Loading composer
  repositories » aujourd'hui.

## Références Composer 2.10.3

Lues dans le phar du harnais (`/opt/homebrew/Cellar/composer/2.10.3`,
extrait hors dépôt ; `stripWhitespace` garde les numéros de ligne,
`docs/reference/Installer.php` est identique à blancs près).

- `Command/InstallCommand.php:111-112` : `PluginEvents::COMMAND` dispatché
  dans `execute`, avant `Installer::create` (l. 114) et `$install->run()`
  (l. 151).
- `Command/UpdateCommand.php:250-251` : idem, après les refus de
  contraintes temporaires (l. 183-193) et de `--lock` avec des paquets
  (l. 244-248), avant `Installer::create` (l. 255) et `run()` (l. 306).
- `Command/RequireCommand.php:328` « has been updated », `:459` « Running
  composer update … », `:461-462` `COMMAND`, `:496` `run()`.
- `Command/RemoveCommand.php:210` « has been updated », `:239-240`
  `COMMAND` (avant le retrait des `allow-plugins` du paquet, l. 242-252),
  `:275` « Running composer update … », `:299` `run()`.
- `Command/RunScriptCommand.php:103-141` : pas de `COMMAND` ; `:140`
  `dispatchScript`, donc écouteurs de plugins **et** scripts.
- `Command/BaseCommand.php:276-281` : l'avertissement
  `COMPOSER_IGNORE_PLATFORM_REQS` dans `initialize`, avant `execute`.
- `Installer.php` (`run`) : `:309-315` `PRE_*_CMD` ; `:322-327`
  `doUpdate`/`doInstall` ; `:343-353` suggestions (update seulement) ;
  `:356-373` abandonnés du lock ; `:375-399` dump (« Generating … »,
  `POST_AUTOLOAD_DUMP` dans `AutoloadGenerator::dump`) ; `:412-436`
  funding ; `:438-442` `POST_UPDATE_CMD`/`POST_INSTALL_CMD`, sous
  `$this->runScripts` (faux seulement en `--dry-run`, l. 292-298 ;
  `--no-scripts` ne coupe que les scripts, voir ci-dessous). `:508-510`
  la mise à jour partielle sans lock (dans `doUpdate`, donc après
  `COMMAND`), `:513` « Loading composer repositories… ».
- `EventDispatcher/EventDispatcher.php:537-540` `addListener` (par
  priorité, ordre d'ajout) ; `:563-578` `addSubscriber` ; `:583-596`
  `getListeners` : `krsort` des priorités, puis les scripts de
  composer.json **ajoutés à la fin de la priorité 0**
  (`array_merge($listeners[0], $scriptListeners)`), et seulement si
  `runScripts` (`Factory.php:377`, `setRunScripts(!$disableScripts)`).
- `PluginManager.php:490-535` (`loadRepository`) : activation dans l'ordre
  de `PackageSorter::sortPackages`, donc ordre d'ajout des écouteurs.
- Abonnements (sources sous `docs/reference/plugins/`, Flex sous
  `fixtures/work/symfony/vendor/symfony/flex/src/Flex.php`) :
  dealerdirect `Plugin.php:161-171` (`POST_INSTALL_CMD`/`POST_UPDATE_CMD`,
  priorité 0) ; phpstan `Plugin.php:83-89` et rector `Plugin.php:53-59`
  (les deux, priorité 0) ; c3 `Installer.php:54-65` (`copyC3V2` /
  `askForUpdateV2`, priorité 0) et `:24-27` (`uninstall` → `deleteFile`) ;
  Flex `Flex.php:893-913` (`install`/`update`, priorité 0) ; bamarni
  `BamarniBinPlugin.php:78-84` (`COMMAND`, `POST_AUTOLOAD_DUMP`), `:117-138`
  (`onEvent` : les dépréciations à chaque événement, quel que soit le nom de
  la commande ; le renvoi seulement pour `install`/`update`, l. 37).

Ordre à obtenir, donc, pour une commande de Composer :

```
[refus de la commande, avertissements de BaseCommand]
COMMAND (bamarni)                      ← InstallCommand:111 / UpdateCommand:250 / RequireCommand:461 / RemoveCommand:239
PRE_INSTALL_CMD | PRE_UPDATE_CMD       ← Installer:309-315
doInstall | doUpdate(+doInstall)
suggestions (update)                   ← Installer:343-353
abandonnés                             ← Installer:356-373
dump + POST_AUTOLOAD_DUMP              ← Installer:375-399
funding                                ← Installer:412-436
POST_INSTALL_CMD | POST_UPDATE_CMD     ← Installer:438-442
   plugins (priorité 0, ordre d'activation) : dealerdirect, phpstan, rector, c3, Flex…
   puis les scripts de composer.json   ← EventDispatcher:583-596
```

## Fichiers, dans l'ordre

1. `harness/scripts.sh`, `harness/bin-plugin.sh`, `harness/yii2-composer.sh` :
   les cas rouges d'abord (ci-dessous), joués sur le binaire de `main`.
2. `crates/vivacity/src/install.rs`
   - `bamarni_command_event(project, manifest, plugins_enabled)` : le corps
     de `command_listeners`, partagé (il était dupliqué dans `dump.rs`).
   - `run_install` : `command_listeners` **avant** `pre_install_cmd`, et
     seulement hors `after_update()` (la commande de résolution l'a déjà
     dispatché).
   - `report_before_dump` (fin d'`execute_operations` et `dry_run`) :
     les abandonnés du lock pour un install (inchangé), les lignes du
     rapport de mise à jour (suggestions puis abandonnés) après un update,
     au même point ; le dry run imprime ensuite le funding dans les deux
     modes (un dry run ne dispatche pas `POST_*_CMD`).
   - `dump_autoloader` : « Generating … autoload files » avant le script
     `pre-autoload-dump` (point 4).
   - `post_autoload_dump_plugins` : c3 n'y garde que la suppression
     (`emulate_c3_uninstall`), la copie part dans `post_install`.
   - `post_install` : funding (les deux modes), puis dealerdirect, phpstan,
     rector, la copie c3, Flex (`emulate_flex_install`, ou
     `print_flex_post_update` après une mise à jour), puis
     `post-install-cmd`. Commentaires : l'ordre réel, `Installer::run`
     l. 412-442, `EventDispatcher::getListeners`.
   - `emulate_c3` coupée en deux : `emulate_c3_uninstall` (suppression,
     au même moment qu'aujourd'hui) et `emulate_c3` (copie).
   - `funding_shown` : `COMPOSER_FUND` lu comme `intval` (méta, nit).
3. `crates/vivacity/src/lib.rs` : `InstallArgs.post_update:
   Option<Box<UpdateReport>>` (`#[arg(skip)]`) **remplace** `after_update:
   bool` (méthode `after_update()`) et `handed_over: Cell<bool>` ;
   `UpdateArgs.command_dispatched: bool` (`#[arg(skip)]`, faux pour
   `vivacity update`).
4. `crates/vivacity/src/update.rs`
   - `UpdateReport` (lignes, manifeste, Flex actif, rappel thanks) bâti
     depuis `Resolved` ; `install_after_update` le reçoit, le pose dans
     `InstallArgs` et rend le statut (`InstallPhase` disparaît : savoir si
     Composer a pris la main ne sert plus, la phase imprime le rapport
     elle-même ou ne l'atteint pas).
   - `resolve_and_lock` : `bamarni_command_event` quand
     `!args.command_dispatched`, après les avertissements de plateforme,
     juste avant « Loading composer repositories… » ; et, sur le refus
     « partial update without a lock » (exit 3, `doUpdate`, après
     `COMMAND`), juste avant ce refus.
   - `run_update_resolved` : `print_post_update` seulement sous
     `--no-install`.
5. `crates/vivacity/src/require.rs` : `bamarni_command_event` juste après
   « Running composer update … » ; `command_dispatched: true` ;
   `print_post_update` seulement sous `--no-install`.
6. `crates/vivacity/src/remove.rs` : `bamarni_command_event` après
   « has been updated » et le retour `--no-update`, avant le retrait des
   `allow-plugins` (manifeste `updated`, relu après l'édition) ;
   `command_dispatched: true`.
7. `crates/vivacity/src/dump.rs` : appelle `bamarni_command_event`
   (même point, même code).
8. Commentaires faux corrigés : `extension_installers.rs` (phpstan écoute
   `POST_*_CMD`, pas `post-autoload-dump`), `scope.rs` (« identical
   output »).
9. Docs dans le même commit : `CHANGELOG.md`, `DECISIONS.md`,
   `HANDOVER.md`, `docs/plans/split-run-install.md`, ce plan.

## Cas de harnais (rouges sur `main`, verts après)

Comparaison : sortie fusionnée `2>&1`, moins les lignes `^vivacity: ` (le
résumé de vivacity) et `^  - Downloading ` (cache froid de Composer, comme
les autres harnais).

- `scripts.sh` 5 — **`bamarni-cmd`** : fixture bin-plugin, `pre-install-cmd`
  qui écrit sur stdout et sur stderr ; `install --run-scripts` (le plugin
  s'installe : pas de `COMMAND` pour lui) puis no-op (**rouge** : la ligne
  `[bamarni-bin]` après le script).
- `scripts.sh` 6 — **`funding-script`** : fixture scripts, `funding` ajouté
  à psr/log dans le lock (copié dans installed.json), `post-install-cmd` qui
  écrit en plus ; install puis no-op. Le funding y est déjà avant le script
  sur `main` (garde-fou du déplacement), mais le cas est **rouge** sur
  `main` pour le point 4 : « Generating autoload files » après la ligne
  `> @php scripts/log.php 'pre-autoload-dump'`.
- `yii2-composer.sh` — ordre fusionné comparé à chaque étape (**rouges**
  sur `main` : `install`, `no-op`, `re-dev` — c3 avant le funding — et,
  après la revue, `no-dev` — « Deleting c3.php » après le dump).
- `yii2-composer.sh` — **`c3-update`** : projet en ligne
  (symfony/polyfill-ctype 1.33.0 avec `funding`, codeception/c3 2.9.0 en
  dev), `update` avec installation (**rouge** : c3 avant le funding), un
  `install --no-autoloader` après suppression de c3.php (**rouge** :
  vivacity ne copie pas sans dump), puis (revue)
  `c3-uninstall-no-autoloader`, `install --no-dev --no-autoloader`
  (**rouge** : vivacity ne supprime pas sans dump). Packagist en ligne,
  versions exactes.
- `bin-plugin.sh` — `update psr/log`, `update --no-install`, `update
  --dry-run`, `require monolog/monolog:3.9.0` (aussi les suggestions),
  `remove monolog/monolog`, puis (méta) `update psr/log` sans lock (exit 3,
  refus de `doUpdate` après `COMMAND`) : stderr entière comparée (le `step`
  existant), `--no-audit --no-security-blocking` des deux côtés
  (**rouges** tous les six). Packagist en ligne, versions exactes ; le
  nombre de suggestions de monolog dépend des extensions PHP de la
  machine (les mêmes des deux côtés).
- Assertions de cohérence sur la sortie de Composer (méta) : « package
  suggestions » pour `require`, le refus pour `partial-no-lock`,
  « looking for funding » et « [codeception/c3] Copying » pour
  `c3-update`, la copie pour `c3-no-autoloader`, « looking for funding »
  pour `funding-script` — un cas qui ne montre plus ce qu'il teste échoue
  au lieu de passer.

## Critère « vert »

- Les rouges ci-dessus verts.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace` (308 sur `main`, plus le test de
  `funding_shown`).
- **Tous** les harnais de la CI (et `root-manifest.sh`), chacun à 0 : la
  méta a montré que `two-stage`, `download-lines`, `vendor-dir`,
  `custom-dirs`, `wp-core`, `artifact-repo`, `tar-dist` comparent aussi la
  sortie d'une installation.

## Méta-analyse adversariale, première passe (auteur du plan)

- **Le rapport d'`update` dans `run_install`** élargit la portée. Mais le
  point 2 sur le chemin `update` (c3 après le funding) ne se corrige pas
  sans : le funding d'une mise à jour est imprimé par `print_post_update`,
  après la phase d'installation, alors que les écouteurs `POST_UPDATE_CMD`
  sont dans la phase. Les suggestions viennent avec, au même coût.
- **Les cas qui rendent la main** (`handed_over`) : rien ne change —
  Composer imprime son propre rapport, vivacity n'ajoute rien.
- **Un statut 0 sans `post_install`** en `after_update` : seulement le dry
  run (qui imprime le rapport lui-même) ; `--check-scope` n'existe pas en
  `after_update`. À relire dans `native_layout`.
- **`--no-scripts`** : ne coupe pas les écouteurs de plugins chez Composer
  (`getListeners`), ni chez vivacity. Inchangé.
- **Plusieurs émulations qui écrivent** (c3 et Flex ensemble) : l'ordre
  entre elles dépend de `PackageSorter` et de l'activation au fil de
  l'installation ; gardé tel quel (c3 avant Flex, comme aujourd'hui).

## Méta-analyse (sous-agent frais, sur le code, 2026-10-16) — et ce qui change

Le relecteur a lu le plan, le code (install, update, require, remove,
dump, fallback, scripts, extension_installers, bamarni_bin, scope) et les
sources de Composer et des plugins. Pas de bloquant ; trois majeurs, six
mineurs, quatre nits. Ce qui en est fait :

- **`COMMAND` d'`update` et le refus « partial update without a lock »**
  (majeur). `prepare_update` porte deux refus de part et d'autre de
  `COMMAND` chez Composer : les contraintes temporaires avant
  (`UpdateCommand.php:183-193`), la mise à jour partielle sans lock après
  (`Installer.php:508-510`). **Adopté** : la ligne est imprimée juste avant
  ce second refus (exit 3), et le cas `partial-no-lock` ajouté à
  `bin-plugin.sh`. Le relecteur proposait d'imprimer `COMMAND` juste après
  `prepare_update` : **écarté**, les avertissements
  `COMPOSER_IGNORE_PLATFORM_REQ(S)` (imprimés par vivacity après
  `prepare_update`) viennent avant `COMMAND` chez Composer
  (`BaseCommand::initialize`) ; la ligne reste après eux. L'index Flex
  (`flex::versions`, avant « Loading ») peut échouer avant `COMMAND` chez
  vivacity : erreur réseau seulement, noté.
- **Prémisse fausse sur dealerdirect et les extension-installers**
  (majeur) : ils écrivent chez Composer en cas nominal. **Adopté** :
  mesuré pour phpstan (deux lignes absentes chez vivacity), inscrit en
  non-objectif avec les sources, phrase du plan corrigée, commentaires
  faux de `extension_installers.rs` et `scope.rs` corrigés. Les émuler est
  une suite à part (nouvelles lignes, pas un ordre).
- **L'ordre fixe des émulations** (majeur) : vrai seulement tant que
  dealerdirect et les extension-installers sont muets. **Documenté** en
  non-objectif, avec la règle de Composer (`PackageSorter`, réenregistrement
  d'un plugin installé pendant la commande).
- **Doublon `[bamarni-bin]` sur une résolution rendue à Composer après
  coup** (mineur) : documenté (même doublon que « Loading » aujourd'hui).
- **Avertissements de `BaseCommand` pour require/remove** (mineur) :
  non-objectif documenté.
- **Harnais fragiles** (mineur) : **adopté** — `--no-security-blocking`,
  assertions de cohérence côté Composer, dépendance aux extensions PHP
  écrite dans le harnais.
- **Liste de non-régression incomplète** (mineur) : **adopté**, tous les
  harnais de la CI.
- **`update --no-install` + c3 écrit un fichier** (mineur) : précisé dans
  le non-objectif. Partager le bloc des écouteurs avec `print_post_update`
  demande la disposition (`Layout`), calculée par la phase d'installation
  seulement : suite à part.
- **`pre-install-cmd` rejoué sur un renvoi à Composer** (mineur) :
  non-objectif, suite proposée.
- **`after_update` et `post_update` redondants** (nit) : **adopté** —
  `post_update` seul, `after_update()` en méthode ; `handed_over` et
  `InstallPhase` tombent avec (plus personne ne lit l'information).
  Faire porter au rapport les seules suggestions (les abandonnés sont
  `abandoned_warnings` du même lock) : **écarté**, `print_post_update`
  (`--no-install`) les lit au même endroit, une seule source.
- **`COMPOSER_FUND` fractionnaire** (nit) : **adopté**, mesuré
  (`COMPOSER_FUND=0.5` et `-0.9` cachent la ligne chez Composer) ;
  `funding_shown` et son test.
- **Plugins globaux et `allow-plugins` de la racine** (nit) : non-objectif.
- **Numéros de ligne décalés** (nit) : **corrigés** (abandonnés l. 356-373,
  dump l. 375-399, « Generating » l. 377-381, funding l. 412-436,
  `POST_*_CMD` l. 438-442).

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-16) — et ce qui a changé

Le relecteur a relu le diff contre `main` et les sources de Composer, et
rejoué lui-même `scripts`, `bin-plugin`, `yii2-composer`, `steps` et
`flex-update` sur le binaire de la branche et sur celui de `main` (rouges
annoncés, pour la bonne raison). Il a vérifié en plus que Composer
n'imprime pas `COMMAND` avant le refus `--prefer-source` +
`--prefer-install` ni avant un lock illisible de `require`. Pas de
bloquant. Ce qui en est fait :

- **c3 copie sans dump mais ne supprime pas** (majeur, préexistant, rendu
  asymétrique par le diff) : mesuré par lui — `install --no-dev
  --no-autoloader` laisse c3.php chez vivacity. **Corrigé au-delà de sa
  proposition** : la suppression est faite au point de Composer, juste
  avant la ligne `- Removing codeception/c3` (`PluginInstaller::uninstall`
  appelle le plugin avant que `LibraryInstaller` retire le code et imprime
  la ligne), dans `execute_operations`, donc aussi sans dump. Le
  non-objectif « Deleting c3.php » tombe ; `no-dev` rejoint la comparaison
  d'ordre ; cas `c3-uninstall-no-autoloader` ajouté.
- **Le contrôle final `no-plugins` de `yii2-composer.sh` vidé** par le bloc
  c3 qui réaffectait `ref`/`viv` (mineur) : **corrigé**, le contrôle suit son
  étape.
- **`bamarni` sans assertion côté Composer** (mineur) : **ajoutée**.
- **CHANGELOG faux pour `bin-plugin.sh`** (stderr seule) : **corrigé**.
- **Commentaire « in activation order »** de `post_install` (mineur) :
  **corrigé** (ordre fixe de vivacity, règle de Composer citée).
- **DECISIONS non à jour** (mineur) : écrit dans le même commit (prévu à
  l'étape 9). Les entrées antérieures qui listent ces défauts comme
  « laissés » restent telles quelles, la nouvelle les solde.
- **`Printed(3)` couplé au code de sortie** (mineur) : **gardé** — 3 est
  `Installer::ERROR_NO_LOCK_FILE_FOR_PARTIAL_UPDATE`, seul refus de ce code
  dans `prepare_update` ; un genre d'erreur dédié toucherait
  `vivacity-resolver` pour un seul appelant.
- **Manifeste fusionné (composer-merge-plugin) pour `install`, brut pour
  update/require/remove** (mineur) : noté en non-objectif (un
  `extra.bamarni-bin` venu d'un fichier inclus).
- **Plan non suivi par git** : ajouté au commit.
- **Job Windows** (`windows.yml` lance `yii2-composer.sh` et
  `bin-plugin.sh`) : pas exécutable ici ; clippy `x86_64-pc-windows-gnu`
  vert, les nouvelles comparaisons fusionnées n'ont tourné que sur macOS.
- **Nits** : commentaires restés sur l'ancien ordre (`dump_autoloader`,
  `execute_operations`, `pre_install_cmd`, `print_funding`, deux dans
  `update.rs`) **corrigés** ; `command_listeners` (enrobage d'une ligne)
  **retiré** ; clone de `report_before_dump` **retiré** ; `Clone` de
  `UpdateReport` **retiré** — le `Box` reste, clippy
  (`large_enum_variant` sur `Cli`) l'exige ; `COMPOSER_FUND=1e400`
  (`intval(INF)` vaut 0) **corrigé**, avec `inf`/`nan` non numériques pour
  PHP, vérifié par `php -r` ; en-tête et message du cas 6 de `scripts.sh`
  **corrigés**. `crate::bamarni_command_event` contre le nom nu : laissé
  (`update.rs` importe `install` par `super::*`, pas `require`/`remove`).


## Résultat (2026-10-16)

**Rouges sur `main`** (binaire de d63b462, `VIVACITY_BIN` pour les harnais
qui l'acceptent) : `scripts.sh` 4 OK, 2 FAIL (`bamarni/no-op` : la ligne
`[bamarni-bin]` après `> echo pre-install-cmd …` ; `funding/install` :
« Generating autoload files » après `> @php scripts/log.php
'pre-autoload-dump'`) ; `bin-plugin.sh` 6 OK, 6 FAIL (`update`,
`update-no-install`, `update-dry-run`, `require` — la ligne et les
suggestions —, `remove`, `partial-no-lock`) ; `yii2-composer.sh` 4 OK,
7 FAIL (`install`, `no-op`, `no-dev`, `re-dev`, `c3-update`,
`c3-no-autoloader`, `c3-uninstall-no-autoloader`).

**Verts** sur le binaire final : `scripts.sh` 6/6, `bin-plugin.sh` 12/12,
`yii2-composer.sh` 11/11.

**Portes** : `cargo fmt --all -- --check` ; clippy `-D warnings` macOS et
`x86_64-pc-windows-gnu` ; `cargo test --workspace` 309 réussis, 0 échec
(36 binaires ; 308 sur `main`, plus `funding_shown`, vu rouge avec
l'ancienne règle). **Harnais** (binaire final, chacun dans son
répertoire, supprimé après) : les 30 passages sortent à 0 — diff-vendor,
diff-vendor `--with-autoloader` deux fois, update, steps (270), removal,
transitions, path-repos, vendor-dir, merge-plugin, scripts, tar-dist,
yii2-composer, bin-plugin, wp-core, custom-dirs, package-versions,
flex-install, flex-update (26), download-lines, artifact-repo,
compare-selftest, two-stage, transport-exit, verbosity (76),
fallback-args, root-scan, boot, drift-reference (129 fichiers
identiques), root-manifest (35) — 659 lignes `OK`, 0 `FAIL`. Le job
Windows (`windows.yml`, qui lance `yii2-composer.sh` et `bin-plugin.sh`)
n'a pas tourné ici.

**Suites proposées** (une par plan) : émuler les lignes stdout de
phpstan/rector/dealerdirect, puis l'ordre `PackageSorter` des émulations ;
ne plus rejouer les écouteurs des plugins dans `composer run-script` et
décider la portée avant `pre-install-cmd` ; les écouteurs `POST_UPDATE_CMD`
sous `update --no-install` ; `forward-command` et un `extra.bamarni-bin`
mal typé sur `update` ; les avertissements de plateforme de
`require`/`remove` en tête.
