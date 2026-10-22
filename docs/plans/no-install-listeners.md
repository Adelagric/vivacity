# Les écouteurs `POST_UPDATE_CMD` sous `update --no-install` — plan

> Suite du non-objectif « `update --no-install` » de
> `docs/plans/plugin-lines.md` (et d'install-order.md avant lui). Niveau 2 :
> le contrat est la parité à l'octet de stdout et de stderr fusionnées, du
> code de sortie et des fichiers du projet. `update`, `require` et `remove`
> avec `--no-install`. Méta-analyse et revue par sous-agents frais.

## Le problème

`Installer::run` avec `setInstall(false)` saute le dump (Installer.php:305-307)
et l'installation (`doUpdate` rend 0 après le lock, :684-689), mais pas la fin :
le funding puis `POST_UPDATE_CMD` (:438-441) partent quand même. Les plugins
chargés au démarrage y écoutent — phpstan/extension-installer,
rector/extension-installer, dealerdirect/phpcodesniffer-composer-installer,
codeception/c3 (Flex est déjà émulé à ce point par `print_post_update`).
vivacity écrit le même lock mais ne joue aucun de ces écouteurs : ni la ligne
« phpstan/extension-installer: Extensions installed » (imprimée à chaque
fois), ni `GeneratedConfig.php`, `CodeSniffer.conf`, `c3.php` quand ils
manquent ou ont changé. Aucun repli : l'écart est silencieux.

La question centrale, tranchée par la mesure : sans installation, `vendor/`
et installed.json gardent l'état précédent pendant que le lock change. Les
écouteurs lisent **le dépôt local** (`getLocalRepository()`) — l'ancien état,
purgé des paquets dont le répertoire manque —, jamais le lock ; un plugin
absent d'installed.json (vendor vierge, plugin ajouté par la commande) n'est
pas chargé du tout ; un plugin retiré par la commande l'est encore.

## Environnement de mesure (ce qui diffère de la CI)

Linux x86_64 (conteneur), 4 cœurs. PHP 8.5.12 de conda-forge (le dépôt
Ubuntu n'a que 8.3, les PPA sont injoignables), `gd` compilé depuis les
sources PHP 8.5.9 de l'archive Ubuntu contre ses en-têtes ; Composer 2.10.3
(phar de la release GitHub, `composer --version` = 2.10.3), en root avec
`COMPOSER_ALLOW_SUPERUSER=1` comme `harness/linux/Dockerfile`.

Deux limites du réseau de cette session, dites ici plutôt que contournées
en silence :

- `api.github.com` (les zipballs de Packagist) est refusé par le proxy ;
  `github.com` en git ne l'est pas. Le cache de fichiers de Composer
  (`files/<nom>/<sha1(url)>.zip`, la clé de `FileDownloader::getCacheKey`,
  que vivacity lit aussi) a été amorcé avec `git archive` du commit verrouillé
  (script hors dépôt). **Les deux outils lisent la même archive** : la parité
  mesurée reste celle de Composer contre vivacity ; ce qui n'est pas mesuré
  ici, ce sont les octets exacts du zipball de GitHub (attributs du zip) et
  les lignes `Downloading` d'un cache froid (`download-lines.sh`).
- `downloads.wordpress.org` (et tout `*.wordpress.org`) est refusé, sous
  toutes ses formes : la fixture wordpress (cinq archives) ne peut pas être
  construite ici ; les passages qui en dépendent sont dits dans le Résultat.

## Références

Composer 2.10.3 (phar extrait sous /tmp ; `docs/reference/` en porte les
jumeaux) :

- `Installer.php:292-298` (`--dry-run` : `runScripts = false`, donc **aucun**
  `POST_UPDATE_CMD`), `:305-307` (`update && !install` : pas de dump),
  `:325` `doUpdate($localRepo, $this->install)`, `:684-689` (`if
  ($doInstall) return doInstall(...)` ; sinon 0, le dépôt local intact),
  `:412-436` (funding compté sur `$localRepo`), `:438-441`
  (`if ($this->runScripts)` → `dispatchScript(POST_UPDATE_CMD)`).
- `EventDispatcher.php:585` : `--no-scripts` (`setRunScripts(false)` sur le
  dispatcher) ne coupe que les scripts de composer.json ; l'`Installer`
  garde `runScripts` vrai hors dry run. Les écouteurs des plugins partent
  donc sous `--no-install --no-scripts`.
- `Factory.php:448` `loadInstalledPlugins` puis `:457` `purgePackages` ;
  `PluginManager.php:102-112` (dépôt local, puis global).
- `FilesystemRepository.php:105-109` + `ArrayRepository.php:215-231` : un
  paquet à alias de branche est chargé **alias d'abord, paquet ensuite** ;
  installed.json ne garde aucun alias en ligne du lock (seule
  l'installation les réajoute, `MarkAliasInstalled`).
- `UpdateCommand.php:289`, `RequireCommand.php:409-410,480` (le `Composer`
  recréé sur le manifeste édité, plugins rechargés d'installed.json),
  `RemoveCommand.php:221-222,242-252` (règles `allow-plugins` lues au
  chargement), `:305-312` (« Removal failed, … still present » : le dépôt
  local, donc toujours sous `--no-install` — déjà porté, `remove.rs`).

Plugins (versions de `scope::EMULATED_VERSIONS`, sources lues sous
`docs/reference/plugins/` et clones git sous /tmp) :

- **phpstan/extension-installer 1.4.3** `Plugin.php:83-89` (`POST_INSTALL_CMD`
  et `POST_UPDATE_CMD` → `process`), `:124`
  (`getLocalRepository()->getPackages()`, **aucun filtre de mode dev**),
  `:116-120` (`ignore` lu dans l'`extra` de la racine), `:196-214` (écrit,
  « Extensions installed », les lignes si l'empreinte change).
- **rector/extension-installer 0.11.2** `Plugin.php:28-59`,
  `PluginInstaller.php:60-105` (dépôt local, sans filtre ; écrit et parle
  seulement si le contenu change).
- **dealerdirect v1.2.1** `Plugin.php:161-171`, `:181-224`, `:503-516`
  (standards lus dans le dépôt local, alias exclus), `:525-534` (phpcs
  cherché dans le dépôt local).
- **codeception/c3 2.9.0** `Installer.php:54-65`, `:114-126`
  (`askForUpdateV2` : copie `__DIR__/c3.php` — le c3 **installé** — si le
  fichier racine manque ; rien s'il est identique ; un fichier différent
  n'est pas remplacé sans réponse). Lu par `git` : `Installer.php` identique
  de 2.6.1 à 2.9.0, 2.6.0 ne diffère que par un type de paramètre ; avant
  2.6.0 le paquet exige `composer-plugin-api ^1.0` et Composer 2 ne le
  charge pas. Pas de garde de version pour c3.
- Muets sans installation (lus) : yiisoft/yii2-composer (`Plugin.php:77-83`,
  `showUpgradeNotes` ne parle que des mises à jour notées à
  `POST_PACKAGE_UPDATE`), wikimedia/composer-merge-plugin
  (`onPostInstallOrUpdate` : premier install seulement), symfony/thanks
  (rappel armé par `POST_PACKAGE_UPDATE`), package-versions, pest-plugin,
  bamarni, symfony/runtime (`POST_AUTOLOAD_DUMP`, non dispatché).
- **php-http/discovery 1.20.0** `src/Composer/Plugin.php:125-130`
  (`POST_UPDATE_CMD` → `postUpdate`), `:145-240` (`getMissingRequires` sur
  le dépôt local ; si une implémentation manque : composer.json édité, puis
  un `Installer` cloné — `install` faux compris — relancé), `:243-340`.
  Les cinq abstractions surveillées (`PROVIDE_RULES`) sont les mêmes de 1.15.0
  à 1.20.0 : `php-http/async-client-implementation`,
  `php-http/client-implementation`, `psr/http-client-implementation`,
  `psr/http-message-implementation`, `psr/http-factory-implementation`.

## Matrice mesurée (Composer 2.10.3, PHP 8.5.12, Linux)

Sortie fusionnée après « Writing lock file » (et la ligne de funding, quand
il y en a) ; « fichiers » : ce que la commande ajoute ou change hors
composer.json/composer.lock. Projets de `harness/plugin-lines.sh` (mêmes
versions exactes). « perdu » : le fichier généré supprimé avant la commande.

| plugin | état | commande | lignes | fichiers |
|---|---|---|---|---|
| phpstan | vendor à jour | `update` | Extensions installed | — |
| phpstan | config perdue | `update` | Extensions installed ; `> strict-rules: installed` ; `> php-8-stubs: not supported` ; `> deprecation-rules: ignored` | GeneratedConfig.php |
| phpstan | config perdue | `update -q` | — | GeneratedConfig.php |
| phpstan | config perdue | `update --no-dev` | comme sans `--no-dev` (strict-rules, en dev, listé) | GeneratedConfig.php |
| phpstan | config perdue | `--no-plugins`, `--dry-run` | — | — |
| phpstan | config perdue | `-v` | `> post-update-cmd: PHPStan\…->process` puis les lignes (déjà rendu à Composer par vivacity) | GeneratedConfig.php |
| phpstan | extension ajoutée par l'update | `update` | Extensions installed (phpunit absente du dépôt local) | — |
| phpstan | extension ajoutée, config perdue | `update` | les lignes **sans** l'extension ajoutée | GeneratedConfig.php |
| phpstan | extension retirée, config perdue | `update` | les lignes **avec** l'extension retirée | GeneratedConfig.php |
| phpstan | plugin retiré par l'update | `update` | Extensions installed (chargé) | — |
| phpstan | `ignore` retiré de l'`extra` racine | `update` | `> deprecation-rules: installed`, `> strict-rules: installed`, `> php-8-stubs: not supported` | GeneratedConfig.php |
| phpstan | extension ajoutée | `update a/b` | Extensions installed | — |
| phpstan | config perdue | `require x --dev --no-install` | les lignes sans x | GeneratedConfig.php |
| phpstan | config perdue | `remove strict-rules --dev --no-install` | les lignes avec strict-rules, puis « Removal failed », code 2 | GeneratedConfig.php |
| phpstan | config perdue | `remove phpstan/extension-installer --no-install` | idem (le plugin écrit puis « Removal failed », 2) | GeneratedConfig.php |
| phpstan | répertoire d'une extension supprimé (purge) | `update` | les lignes sans elle | GeneratedConfig.php |
| phpstan | vendor vierge | `update` | — (ni funding) | — |
| phpstan | plugin ajouté par l'update | `update` | — | — |
| phpstan | alias en ligne (`2.0.4 as 2.0.99`), vendor à jour | `update` | Extensions installed (rien ne change : pas d'alias dans le dépôt local) | — |
| dealerdirect | à jour, standard ajouté par l'update | `update` | — | — |
| dealerdirect | configuration perdue | `update`, `update --no-dev` | set to ../../phpcsstandards/phpcsextra,../../phpcsstandards/phpcsutils | CodeSniffer.conf |
| dealerdirect | configuration perdue | `-q` | — | CodeSniffer.conf |
| dealerdirect | plugin retiré (avec les standards qui l'exigent), configuration perdue | `update` | set to … (les standards du dépôt local) | CodeSniffer.conf |
| dealerdirect | vendor vierge | `update` | — | — |
| c3 | à jour, ou c3.php modifié | `update` | — | — |
| c3 | c3.php perdu | `update`, `--no-dev` | Copying… ; Include… | c3.php |
| c3 | c3.php perdu | `remove codeception/c3 --dev --no-install` | Copying… ; Include… ; « Removal failed », 2 | c3.php |
| c3 | 2.8.1 installé, lock → 2.9.0, c3.php perdu | `update` | Copying… (le c3.php de 2.8.1) | c3.php |
| c3 | plugin ajouté par l'update, vendor vierge | `update` | — | — |
| rector | à jour | `update` | — | — |
| rector | config perdue | `update` | rector/rector-installer: Extensions installed ; `> rector/rector-symfony: installed` | GeneratedConfig.php |
| rector | `rector-symfony dev-main` (alias de branche par défaut), à jour | `update` | — (le fichier écrit à l'install est celui de l'update : version du paquet, chargé après son alias) | — |
| les quatre | tout perdu | `update` | dealerdirect set to … ; c3 Copying/Include ; phpstan + strict-rules ; rector + rector-symfony | les quatre |
| les quatre | tout perdu | `-q` | — | les quatre |
| php-http/discovery | racine `php-http/discovery` + `psr/http-client-implementation` + guzzle exigés, guzzle absent du dépôt local | `update` | composer.json édité (`symfony/http-client: *`), puis un `Installer` imbriqué par écouteur, sans fin (25 « Writing lock file » en 25 min, coupé) | composer.json, composer.lock |
| php-http/discovery | racine `project`, l'abstraction exigée, guzzle en `require-dev` seulement, dépôt local = lock (méta-analyse) | `update` | « Nothing to modify in lock file », puis guzzle déplacé en `require: "*"`, l'update imbriqué monte guzzle 7.16.0 → 8.3.0, sans fin | composer.json, composer.lock |
| yiisoft/yii2-composer | extensions.php supprimé (méta-analyse) | `update`, `update --dry-run` | — (`Plugin::activate` le recrée au chargement) | vendor/yiisoft/extensions.php |

Ordre (« les quatre ») : celui de `PackageSorter` sur installed.json
(dealerdirect requis par trois paquets d'abord, puis les noms) — le
premier étage de `plugin_order` sans transaction.

## Le modèle porté

À `POST_UPDATE_CMD` sans installation (`--no-install`, pas `--dry-run`,
plugins actifs), après le funding :

1. Ordre : `plugin_order::loaded_plugins` (installed.json tel qu'écrit,
   global ensuite) — aucune transaction n'active ni ne retire de plugin.
2. Dépôt local : installed.json **purgé** (`purge_missing`), tous les
   paquets (dev compris, quel que soit `--no-dev`), sans alias en ligne ;
   un alias de branche passe avant son paquet, donc la version écrite est
   celle du paquet (déjà ce que font les émulations), et un ignoré compte
   deux fois (`local_repository_entries`, alias en ligne vides).
3. Disposition : `Layout::resolve` sur ce dépôt (le manifeste courant,
   comme `getInstallPath` de Composer).
4. Les écouteurs émulés — dealerdirect (`register_standards`), phpstan,
   rector, c3 (`askForUpdateV2` : `after_update`), Flex (sa sortie actuelle,
   désormais à son rang) — dans l'ordre de l'étape 1 (mesuré : dealerdirect,
   c3, phpstan, rector). Chacun vérifie plugin présent et autorisé ;
   `remove` les juge avec les règles du chargement (`allow_plugins_at_load`).
5. Au chargement (méta-analyse) : `yiisoft/yii2-composer::activate` recrée
   `<vendor-dir>/yiisoft/extensions.php` s'il manque — sans installation et
   en dry run aussi (le dry run de la phase d'installation compris). Rien
   d'autre ne s'exécute (lus : les `POST_UPDATE_CMD` de yii2, merge-plugin,
   thanks muets sans opération ; les écouteurs `POST_AUTOLOAD_DUMP` ne
   partent pas).

## Ce qui est rendu à Composer (repli nommé, avant toute écriture)

- **Version non lue** de phpstan/rector/dealerdirect installée, autorisée,
  plugins actifs, sous `--no-install` sans `--dry-run` : même liste que
  l'installation (`EMULATED_VERSIONS`), décidée avant de résoudre
  (`resolution_issues`, qui ne la contrôlait qu'avec installation).
  package-versions n'est pas concerné (`POST_AUTOLOAD_DUMP` ne part pas).
- **Disposition non calculable** pour le dépôt local (`Layout::resolve` en
  erreur) quand l'un de ces trois plugins est chargé.
- **php-http/discovery** chargé et autorisé, dès qu'il surveille une
  abstraction (le `require` de la racine nomme `php-http/discovery` et la
  racine — `require` ou `require-dev` — en exige une des cinq, ou un paquet
  installé qui exige `php-http/discovery` en exige une ; l'épinglage par
  `extra.discovery` n'est pas lu), ou dans une version non lue (hors
  1.15.0–1.20.0) : `getMissingRequires` n'est pas porté, et la suite
  (édition de composer.json, `Installer` imbriqué) ne l'est pas non plus.
  Décidé sur le manifeste que la commande résout (l'édition de `require`
  comprise), en tête de `resolve_and_lock`, avant toute ligne. Conservateur : rendu même quand
  Composer n'aurait rien fait. (Première version du plan : seulement quand
  le dépôt local diffère du lock — insuffisant, voir la méta-analyse.)
- **Un écouteur émulé installé globalement** (dealerdirect, phpstan,
  rector, c3 dans `COMPOSER_HOME`) : chargé aussi, vivacity ne joue que
  ceux du projet.
- **Un plugin actif ni émulé ni bénin** (cweagans/composer-patches,
  mlocati/composer-patcher, oomphinc/composer-installers-extender : inertes
  pour le lock, `RESOLUTION_INERT`) : composer-patches retire à
  `PRE_UPDATE_CMD` les paquets à re-patcher, l'extender déplace les chemins
  que les écouteurs écrivent ; l'install les rend déjà.

Ces trois derniers points et la disposition sont décidés avant de résoudre
(`fallback::resolution_fallback` → `no_install::issues_before_resolution`),
php-http/discovery en tête de `resolve_and_lock`.

## Fichiers

1. `harness/plugin-lines.sh` : la section `--no-install` (rouge d'abord).
2. `crates/vivacity-core/src/scope.rs` : `resolution_issues` contrôle les
   versions de `POST_UPDATE_FILE_LISTENERS` (dealerdirect, phpstan, rector)
   aussi sans installation (paramètre `post_update_cmd` : la commande
   dispatche `POST_UPDATE_CMD`, tout sauf un dry run) ; `DISCOVERY`,
   `DISCOVERY_ABSTRACTIONS`, `discovery_watches` ; tests.
3. `crates/vivacity/src/no_install.rs` (nouveau) : `local_repository`
   (installed.json purgé, ordre d'activation), `issues_before_resolution`
   (plugin global, plugin ni émulé ni bénin, disposition), `discovery_issue`,
   `activate_yii2_composer`, `post_update_listeners`.
4. `crates/vivacity/src/fallback.rs` (`resolution_fallback` :
   `post_update_cmd`), `update.rs` (`print_post_update` joue les écouteurs ;
   `listeners_manifest` ; discovery dans `resolve_and_lock`), `require.rs`,
   `remove.rs`, `install.rs` (`emulate_phpcs` partagé, `purge_missing`
   partagé, yii2 au dry run).
5. Docs dans le même commit : CHANGELOG (`[Unreleased]` → `### Fixed`),
   DECISIONS, HANDOVER, plugin-lines.md (non-objectif traité), README (la
   description des plugins émulés), ce plan.

## Cas de harnais (rouges d'abord)

`harness/plugin-lines.sh`, nouvelle section : chaque projet installé par
son outil, l'état gardé (`cp -a`, même chemin : les chemins absolus de
GeneratedConfig.php ne bougent pas), puis chaque cas part de cet état.
Code de sortie, projet entier, sortie fusionnée, et une assertion sur la
sortie de Composer par cas.

- phpstan : `ni-ps-noop`, `-lost`, `-quiet`, `-no-dev`, `-no-plugins`,
  `-dry-run`, `-ext-added`, `-ext-removed`, `-plugin-removed`,
  `-ignore-dropped`, `-partial`, `-require`, `-remove`, `-ext-dir-gone`,
  `-bare`, `-plugin-added`, `-alias` (alias en ligne, installé par
  Composer des deux côtés, config perdue) ;
- dealerdirect : `ni-pc-noop`, `-lost`, `-no-dev`, `-plugin-removed`,
  `-std-added` ;
- c3 : `ni-c3-noop`, `-lost`, `-edited`, `-remove`, `-updated` (2.8.1 →
  2.9.0), `-plugin-added` ;
- rector : `ni-rc-lost`, `ni-rc-main` (alias de branche par défaut, config
  perdue) ;
- les quatre : `ni-all-noop`, `-lost` (ordre), `-quiet` ;
- replis : `ni-old-versions` (phpstan 1.3.1, rector 0.10.2, dealerdirect
  v0.7.2 installés par Composer : code 3 sous `--no-fallback`, trois
  raisons, rien d'écrit — package-versions 1.11.99.1 présent et **non**
  nommé) ; `ni-discovery-same`, `ni-discovery` (lock changé),
  `ni-discovery-dev` (le cas de la méta-analyse) : code 3, la raison, rien
  d'écrit ; `ni-discovery-idle` (rien de surveillé : natif, comparé) ;
  `ni-global`, `ni-patches` ;
- ajoutés après la méta-analyse : `ni-ps-blocked` (`allow-plugins` à
  false), `ni-ps-plugin-dir-gone`, `ni-ps-lock-false`, `ni-ps-unsolvable`
  (code 2, aucun écouteur), `ni-ps-working-dir`, `ni-ps-vendor-dir`,
  `ni-yii2-lost`, `ni-yii2-dry-run`, `ni-flex` (Flex et phpstan : ordre).

Les cas verts sur le binaire de `main` sont des garde-fous (ce qui ne doit
PAS changer : rien à charger, rien à écrire), pas des rouges ; le Résultat
les sépare.

## Critère « vert »

- Les rouges ci-dessus verts, les garde-fous verts.
- `cargo fmt --all -- --check` ; `cargo clippy --workspace --all-targets --
  -D warnings` (Linux et `--target x86_64-pc-windows-gnu`) ;
  `cargo test --workspace`.
- Tous les harnais de `ci.yml` à 0 (dont `diff-vendor.sh` trois fois,
  `plugin-lines.sh`, `flex-recipes.sh`), plus `harness/root-manifest.sh` —
  sauf ce que le réseau de cette session interdit, dit dans le Résultat.

## Non-objectifs (vus, laissés)

- Un plugin installé qu'`allow-plugins` ne mentionne pas (règles non
  nulles) : Composer lève `PluginBlockedException` et sort en 1 sans rien
  écrire (PluginManager.php, `isPluginAllowed`, non interactif) ; vivacity
  continue — sur toutes les commandes de résolution, avec ou sans
  installation. Écart antérieur, d'une autre famille (le chargement des
  plugins), mesuré par la méta-analyse ; laissé, noté.
- php-http/discovery sur `update` **avec** installation : la méta-analyse
  montre par les sources que `getMissingRequires` peut agir même quand le
  dépôt local est le lock (racine `project`, implémentation en dev) ; la
  liste « inerte » ne repose que sur l'`install` (discovery n'écoute pas
  `POST_INSTALL_CMD`). Non mesuré avec installation, hors de ce chantier :
  noté pour un chantier à part.
- dealerdirect v1.1.1/v1.1.2 (`InstalledVersions::isInstalled`) : sans
  installation, lit l'installed.php précédent, non purgé — phpcs supprimé à
  la main, Composer échoue où l'émulation se tait. Rare, noté.
- La purge suit l'`install-path` enregistré ; Composer, le `getInstallPath`
  courant (un `installer-paths` édité depuis la dernière installation).
  Écart antérieur (install comprise), noté.

- `-v` : déjà rendu à Composer dès qu'un plugin autorisé est actif.
- Plugins émulés installés globalement : ordre calculé, émulation absente
  (écart antérieur, inchangé).
- php-http/discovery sous `--no-install` n'est pas émulé : rendu (voir
  plus haut). Sa boucle chez Composer est le comportement de Composer.
- dealerdirect quand phpcs ne tourne pas sous le PHP de la machine
  (« Failed to set … ») : écart antérieur, inchangé.

## Méta-analyse (sous-agent frais, sur le plan et le code, 2026-10-22) — et ce qui change

Un bloquant, six majeurs, huit mineurs. Le relecteur a vérifié contre les
sources et par des expériences contre Composer 2.10.3 : Installer.php
292-298, 305-307, 412-441, 684-689 ; `--no-scripts` ; chargement avant la
purge ; alias puis paquet ; phpstan sans filtre de dev ; rector ;
dealerdirect ; c3 identique de 2.6.1 à 2.9.0 ; les cinq abstractions de
discovery ; yii2/merge-plugin/thanks muets à `POST_UPDATE_CMD` ; `remove`
et `require` ; `-v` ; toutes les lignes de la matrice sauf une.

- **Bloquant — la condition de php-http/discovery était insuffisante** :
  un dépôt local égal au lock ne rend pas `postUpdate` inerte (racine
  `project`, implémentation en `require-dev` : déplacée en `require`, update
  imbriqué sans fin, mesuré). **Adopté** : rendu dès qu'il surveille une
  abstraction, ou dans une version non lue ; cas `ni-discovery-dev`. Le
  même écart avec installation : **noté** (non-objectif, hors chantier).
- **yii2-composer `activate`** (majeur) : **adopté** — `extensions.php`
  recréé sans installation et en dry run (`no_install::activate_yii2_composer`,
  appelé aussi par le dry run de la phase d'installation) ; cas
  `ni-yii2-lost`, `ni-yii2-dry-run`.
- **Plugins globaux** (majeur) : **adopté** — repli nommé ; cas `ni-global`.
- **Plugin non listé dans `allow-plugins`** (majeur) : écart antérieur de
  toutes les commandes de résolution ; **noté**, non-objectif.
- **composer-patches, installers-extender** (majeur) : **adopté** — sans
  installation, tout plugin actif ni émulé ni bénin est rendu ; cas
  `ni-patches`.
- **Flex à son rang sans cas** (majeur) : **adopté** — cas `ni-flex`.
- **Garde-fous et états manquants** (majeur) : **adopté** — les garde-fous
  séparés des rouges dans le Résultat ; cas ajoutés (liste ci-dessus).
- Mineurs : ligne dealerdirect « plugin retiré » mal construite (les
  standards l'exigeaient) **corrigée** (cas et matrice) ; `remove` et la
  disposition : la vérification d'avant résolution lit le manifeste
  d'avant l'édition, et l'exécution celui du chargement — **vérifié**,
  rien à changer ; l'endroit de la décision **écrit** ; dealerdirect
  v1.1.x et la purge par `install-path` **notés** ; l'opt-in de discovery
  (`require` seulement) et l'ordre de l'étape 4 **reformulés** ; les cas
  « alias, config perdue » mesurés par le harnais lui-même (Composer y
  tourne, comparé).


## Revue indépendante (deux sous-agents frais, sur le diff, 2026-10-22) — et ce qui a changé

Le premier relecteur, lent, a été prié de conclure ; un second a été lancé
avec la même consigne. Aucun bloquant chez le premier ; un bloquant (le
même point, reproduit) chez le second. Tous deux ont vérifié contre les
sources : le flux d'`Installer::run` sans installation, `RequireCommand`
(écouteurs avant `updateRequirementsAfterResolution`), `RemoveCommand`
(règles du chargement), le chargement des plugins et la purge, c3,
package-versions hors garde, `discovery_watches` contre
`getMissingRequires`, les valeurs de `post_update_cmd` à chaque appel, les
règles du dépôt (ni `unwrap`/`expect` hors tests, ni `unsafe`, ni
dépendance).

- **Écouteurs de plugins non chargés** (majeur chez l'un, bloquant chez
  l'autre, reproduit : dealerdirect autorisé mais absent d'installed.json
  et une disposition non calculable → `internal: …` et code 1 après
  « Writing lock file », Composer 0) : `listeners_in_order` ajoute en fin
  les noms émulés qu'il ne trouve pas. **Corrigé** : seuls les plugins
  chargés jouent (`listening`, la même règle que la vérification d'avant
  résolution) ; test unitaire `a_plugin_allowed_but_not_installed_runs_nothing`,
  rouge sans la correction.
- **`activate` de yii2-composer trop tard** (majeur, reproduit : un update
  insoluble, un `require` insoluble — Composer recrée extensions.php) :
  **corrigé** — joué dans `resolve_and_lock` là où le run reste celui de
  vivacity (erreurs imprimées après chargement, insoluble, succès), jamais
  avant un passage de main ; cas `ni-yii2-unsolvable`.
- **Décision de php-http/discovery après la résolution** (mineur : les
  lignes « Loading composer repositories… » imprimées deux fois au passage
  de main) : **corrigé**, en tête de `resolve_and_lock`.
- **Épinglage de discovery** (mineur : `psr/http-message-implementation`
  n'est pas épinglable, `INTERFACE_MAP`) : lu plus loin dans les sources —
  l'épinglage n'existe qu'à partir de 1.18.0 ; **corrigé** autrement :
  l'épinglage n'est plus lu du tout (une abstraction épinglée compte comme
  surveillée : rendu).
- **Deux raisons pour un plugin inconnu** (mineur, les deux) : **corrigé**,
  seulement les plugins de `RESOLUTION_INERT`.
- **CHANGELOG en retard sur le code** (mineur, les deux) : **réécrit**.
- **Le cas Flex ne pouvait pas échouer** (mineur) : **adopté** —
  `ni-flex-weighted` (Flex requis par un paquet `path` installé : poids -1,
  sa sortie avant phpstan ; l'ancien emplacement, Flex en dernier, échoue).
- **Pas de cas sans `--no-scripts`** : **ajouté** (`ni-ps-scripts`).
- `cargo fmt` signalé par le premier relecteur : déjà passé avant la fin de
  sa revue (il lisait un état intermédiaire) ; vérifié à nouveau.
- **Notés, laissés** : les règles `allow-plugins` globales
  (`allowGlobalPluginRules`) pour un plugin global — seul un yii2 global
  serait concerné, les écouteurs globaux émulés sont rendus ;
  `ensure_extensions_file` crée un fichier absent par `std::fs::write` —
  pas de lien dur possible sur un fichier qui n'existe pas, la règle
  temp + rename ne s'applique pas ; Phalcon (`getMissingRequires` agit si
  la classe `Phalcon\Http\Message\RequestFactory` existe) — sans objet :
  discovery est rendu dès qu'il surveille ; un `require`/`remove` avec
  passage de main réel (sans `--no-fallback`) n'a pas de cas (le passage de
  main lui-même est couvert par `fallback-args.sh`) ; `require
  --no-install` couvert pour phpstan seulement.
