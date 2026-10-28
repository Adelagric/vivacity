# Plugins à effets de bord, second tour — plan

> Suite de `docs/corpus/2026-10-28.md` : 85 entrées natives sur 106 en
> `--no-dev`, 0 diff. Parmi les 21 autres, douze ne sont bloquées que par un
> ou deux plugins qui ne sont pas des installeurs de chemin : ils écrivent ou
> suppriment des fichiers dans vendor/ et le projet, ou impriment des lignes,
> à des événements de script. Niveau 2 : parité à l'octet de vendor/, du
> lock, de stdout, de stderr, du code de sortie et des fichiers que les
> plugins émulés écrivent ; ce qui n'est pas reproductible part à Composer,
> nommé, avant toute écriture. Méta-analyse et revue par sous-agents frais.
> Même méthode que `docs/plans/custom-installers.md`.

## Le problème

Bloqueurs **complets** (`--no-dev`, run du 2026-10-28) des entrées visées :

| entrée | bloqueurs |
|---|---|
| bolt-project | drupol/composer-packages 2.0.0 |
| cakephp-app | cakephp/plugin-installer 2.0.2 |
| concretecms | mlocati/composer-patcher 1.2.6 |
| contao-managed-edition | contao/manager-plugin 2.13.6 |
| fossbilling | liborm85/composer-vendor-cleaner 1.7.1 |
| mezzio-skeleton | laminas/laminas-component-installer 3.8.0 |
| owncloud-core | dg/composer-cleaner v2.2.2 |
| phpstan-src | ondrejmirtes/composer-attribute-collector 1.2.0 |
| sculpin | sculpin/sculpin-theme-composer-plugin 1.0.3 |
| spiral-app | spiral/composer-publish-plugin v1.1.2 |
| typo3, typo3-cms-base-distribution | typo3/class-alias-loader v2.0.1 + typo3/cms-composer-installers v5.0.2 |
| laminas-mvc-skeleton | laminas-component-installer 3.8.0 + laminas/laminas-skeleton-installer 1.3.0 |

Hors champ (consigne) : Drupal, mautic, silverstripe, ilias, omeka-s.

## Environnement de mesure

`Darwin MacStudio-de-Adel-5.local 25.6.0 arm64`, PHP 8.5.12 (8.4.27 pour
la double passe des harnais), Composer 2.10.3, bash 3.2. Réseau ouvert
(Packagist, GitHub). Binaire de référence : main d2b294c (« Release
0.25.0 »).

## Références (sources lues aux versions verrouillées)

Sources extraites des dists du lock (`dist.url`), tags comparés dans les
dépôts git (`git diff --quiet <tag lu> <tag> -- src`) : « = » marque les
tags aux sources identiques, les seuls que le garde de version admet.

| plugin | version (=) | licence | lu |
|---|---|---|---|
| spiral/composer-publish-plugin | v1.1.2 (= v1.1.1) | MIT | `PublishPlugin`, `Command`, `Downloader` |
| sculpin/sculpin-theme-composer-plugin | 1.0.3 | MIT | `SculpinThemePlugin` |
| laminas/laminas-component-installer | 3.8.0 (= 3.5.0, 3.6.0, 3.7.0) | BSD-3-Clause | `ComponentInstaller`, `ConfigDiscovery` et ses découvertes |
| laminas/laminas-skeleton-installer | 1.3.0 (= 1.2.0) | BSD-3-Clause | `Plugin`, `OptionalPackagesInstaller`, `OptionalPackage`, `ComposerJsonRetrievalTrait` |
| dg/composer-cleaner | v2.2.2 | BSD-3-Clause (composer.json ; pas de fichier de licence au tag) | `Plugin`, `Cleaner` |
| liborm85/composer-vendor-cleaner | 1.7.1 | MIT (+ `Finder/Glob.php` de Symfony, MIT) | tout `src/` |
| cakephp/plugin-installer | 2.0.2 | MIT | `Plugin` |
| drupol/composer-packages | 2.0.0 | MIT | `Plugin`, `ClassGenerator`, exportateurs, gabarits Twig |
| ondrejmirtes/composer-attribute-collector | 1.2.0 | BSD-3-Clause | `Plugin` |
| mlocati/composer-patcher | 1.2.6 | MIT | `Plugin` (et le reste survolé) |
| typo3/class-alias-loader | v2.0.1 | MIT | `Plugin`, `ClassAliasMapGenerator`, `Config`, `IncludeFile` |
| typo3/cms-composer-installers | v5.0.2 | GPL-2.0-or-later | `Installer\Plugin`, `PluginImplementation`, `ScriptDispatcher`, `IncludeFile`, `EntryPoint` (lus, rien copié) |
| contao/manager-plugin | 2.13.6 | LGPL-3.0-or-later | `ManagerPluginInstaller` (lu, rien copié) |

Copies de référence (`docs/reference/plugins/<nom>/`, licence ou note de
licence, ligne NOTICE) : les sept plugins portés, tous MIT ou BSD-3-Clause.
Rien n'est copié des sources GPL/LGPL.

## Classement mesuré

Mesures : `composer install --no-scripts --no-interaction
--ignore-platform-req='ext-*' --ignore-platform-req=php` sur une copie de
chaque entrée (dépôt git, comme `harness/corpus.sh`), dev et `--no-dev`,
`git status` après.

**Fidèles et déterministes** :

- **spiral/composer-publish-plugin** : `post-autoload-dump` →
  `publishFiles` ; sans `extra.publish-cmd` racine, ou sans paquet du dépôt
  local (canonique) à `extra.publish` non vide (`empty()` de PHP), rien.
  Sinon un `Process` lance la commande de publication (code du projet) :
  hors de portée. spiral-app : `publish-cmd` présent, aucun paquet
  `publish` → inerte (mesuré : aucune ligne, aucun fichier).
- **sculpin/sculpin-theme-composer-plugin** : événements de paquet ; un
  paquet de type `sculpin-theme` ou à `extra.sculpin-theme` est copié sous
  `config.theme-dir` (défaut `source/themes`)/<nom>, et retiré au retrait.
  Portable (copie déterministe, `Finder` ignorant VCS et fichiers cachés),
  mais aucune entrée ne l'exige. sculpin : aucun thème → inerte.
- **laminas/laminas-component-installer** : événements de paquet ;
  `if (! $event->isDevMode()) return;` en tête des trois écouteurs → inerte
  en `--no-dev`. En dev, `getAvailableConfigOptions` ne trouve rien si aucun
  des fichiers `config/application.config.php`, `config/modules.config.php`,
  `config/development.config.php.dist`, `config/development.config.php`,
  `config/config.php` n'existe (relatifs au répertoire courant) → inerte
  (mesuré en dev sur mezzio-skeleton et laminas-mvc-skeleton : aucune ligne,
  aucun fichier). Avec un fichier de configuration : injection, invites
  (`ask`) → repli.
- **laminas/laminas-skeleton-installer** : `post-install-cmd` /
  `post-update-cmd` **à la priorité 1000** (avant les écouteurs de priorité
  0) ; avec `extra.laminas-skeleton-installer` (ou `zend-…`) tableau
  non vide d'entrées valides (`name`, `constraint`, `prompt`) et sans
  interaction, la question « minimal install » prend sa valeur par défaut
  `y` : deux lignes sur stdout (« `    Removing optional packages from
  composer.json` », « `    Updating composer.json` ») et composer.json
  réécrit par `JsonFile` (indentation détectée, clés retirées, `extra` vide
  retiré, `\n` final, écriture seulement si le contenu change). Mesuré sur
  laminas-mvc-skeleton (dev et `--no-dev`) : exactement cela.
- **dg/composer-cleaner** : `post-install-cmd` / `post-update-cmd` →
  `Cleaner::clean(vendor-dir)` : pour chaque `vendor/<a>/<b>` (répertoire,
  pas un lien), lit son composer.json ; type `library`, `composer-plugin`
  ou absent : supprime les `exclude-from-classmap` (comptés même absents),
  puis les entrées de premier niveau qui ne sont ni sources de l'autoload
  (premier segment), ni `bin`, ni `composer.json`/`license*`/`LICENSE*`/
  `.phpstorm.meta.php`, ni `extra.cleaner-ignore` (`fnmatch`) ; une ligne
  stdout « `Composer cleaner: Removed N files or directories.` ».
  Mesuré : owncloud-core 202 (`--no-dev`), 285 (dev). Les lignes `-v` sont
  dans l'ordre de `readdir` → repli sous `-v`.
- **liborm85/composer-vendor-cleaner** : avec `extra.dev-files` (objet
  motif-de-paquet → motifs), `pre-autoload-dump` nettoie chaque paquet du
  dépôt local (motifs glob → regex `Glob::toRegex`, tri décroissant,
  répertoire supprimé seulement vide), puis tous les répertoires vides de
  tous les paquets ; `post-install-cmd` nettoie `bin-dir` et imprime le
  bilan. Mesuré fossbilling `--no-dev` : « » / « `Composer vendor
  cleaner: Cleaning packages in vendor directory` » au dump, « `…Cleaning
  vendor binary directory` » / « `…Removed 111 files and 42 (of which 4 are
  empty) directories from 91 packages` » / « » à la fin.
- **cakephp/plugin-installer** : `pre-autoload-dump` ajoute à l'autoload
  racine les plugins applicatifs (`plugins/<Nom>/src`, `plugins/<V>/<Nom>/
  src`) ; `post-autoload-dump` écrit `vendor/cakephp-plugins.php` (paquets
  `cakephp-plugin` du dépôt local + plugins applicatifs, `ksort`, racine
  remplacée par `$baseDir . '`). Mesuré cakephp-app : `Migrations` seul en
  `--no-dev`, quatre en dev.

**Effets non reproductibles à l'octet** :

- **drupol/composer-packages** : cinq classes générées sous
  `vendor/drupol/composer-packages/build/` contiennent `GENERATED_AT =
  time()` : deux runs de Composer diffèrent entre eux.
- **ondrejmirtes/composer-attribute-collector** : « `Generated attributes
  file in <durée> ms` » sur stdout (`microtime`), et l'analyse PHP des
  sources par nikic/php-parser.
- **typo3/class-alias-loader** : sur un vendor vierge, `vendor/autoload.php`
  n'existe pas encore au `pre-autoload-dump` : le suffixe vient de
  `md5(uniqid())` et entre dans deux fichiers générés ; il `require` aussi
  les cartes d'alias PHP des paquets. Les deux entrées l'activent
  (`always-add-alias-loader`, cartes de typo3/cms-*).

**Hors de portée** :

- **mlocati/composer-patcher** : applique des correctifs par `patch` ou
  `git apply` (processus externes) et réécrit installed.json
  (`patches_applied`).
- **contao/manager-plugin** : instancie les classes de plugin des paquets
  (code des paquets) pour ordonner et générer son fichier.
- **typo3/cms-composer-installers** : exécute au `post-autoload-dump` les
  `InstallerScripts` déclarés par les paquets (code des paquets) ; GPL.

## Rang : entrées levées par unité de travail

Une entrée n'est native que si tous ses bloqueurs le sont.

| port | travail | entrées levées (`--no-dev`) |
|---|---|---|
| spiral (garde d'inertie) | très faible | spiral-app |
| sculpin (garde d'inertie) | très faible | sculpin |
| laminas-component-installer (inerte en `--no-dev`, et en dev sans fichier de config) | faible | mezzio-skeleton |
| laminas-skeleton-installer (deux lignes, composer.json réécrit) | moyen | laminas-mvc-skeleton (avec le précédent) |
| dg/composer-cleaner | moyen (fnmatch, parcours) | owncloud-core |
| cakephp/plugin-installer | moyen (fichier, autoload racine) | cakephp-app |
| liborm85/composer-vendor-cleaner | élevé (globs, deux temps, comptes) | fossbilling |
| drupol, attribute-collector, class-alias-loader, patcher, contao, typo3 installers | — | repli (aucune) |

Attendu : sept entrées natives en `--no-dev` (92/106), et les mêmes en dev
si rien d'autre ne les bloque (le scan dev ne les liste qu'avec ces mêmes
plugins).

## Le modèle porté

Commun à tous :

1. **Garde de version** : `scope::EMULATED_VERSIONS` reçoit les sept plugins
   et leurs versions « = » ; un plugin autorisé dans une autre version, au
   lock ou à installed.json (version chargée au début de la commande), →
   repli nommé avant toute écriture.
2. **Activation** comme les émulations existantes : paquet voulu ou
   installé, **autorisé** (`allow-plugins`), plugins actifs ; rien sous
   `--no-plugins` (bibliothèques ordinaires).
3. **Ordre des lignes** : les écouteurs de priorité 0 dans l'ordre
   d'activation (`plugin_order`), skeleton-installer avant eux (priorité
   1000).
4. **`-v`** : cakephp ajouté à `verbose_listeners::VERBOSE_PLUGINS` (aucune
   écriture verbeuse propre) ; les six autres dans `VERBOSE_DELEGATED`, avec
   leur raison (événements de paquet ou `pre-install-cmd` que vivacity ne
   place pas, lignes verbeuses dans l'ordre de `readdir`).
5. **Commandes de résolution** : inertes à la résolution (`RESOLUTION_INERT`)
   — leurs effets sont ceux de l'installation qui suit, émulée là ; sans
   installation (`--no-install`), `post-update-cmd` est quand même
   distribué : les deux nettoyeurs et skeleton-installer configuré → repli
   nommé.
6. **Le cache de classmap par entrée du store** suppose un répertoire de
   paquet intact : quand un nettoyeur est actif, il est contourné (le cache
   par fichier, mtime et taille, reste).

Par plugin :

- **spiral** : actif et racine à `extra.publish-cmd` non nul et un paquet
  (lock voulu ou installed.json) à `extra.publish` non vide → repli
  « would run its publish-cmd » ; sinon rien (install et dump-autoload).
- **sculpin** : un paquet (lock ou installed.json) `sculpin-theme` ou à
  `extra.sculpin-theme` non nul → repli « theme files not emulated » ;
  sinon rien.
- **laminas-component-installer** : dev et l'un des cinq fichiers de
  configuration présent → repli « would inject into config » ; sinon rien.
- **laminas-skeleton-installer** : configuré (tableau non vide après le
  filtre `isValidSpec`), une entrée qui n'est pas un objet/tableau (TypeError
  chez le plugin) → repli ; stdin terminal sans `-n` ni
  `COMPOSER_NO_INTERACTION` → repli (invite) ; sinon, au
  `post-install-cmd`/`post-update-cmd`, les deux lignes et la réécriture
  `JsonFile` (`phpjson`, `detectIndenting`).
- **dg/composer-cleaner** : port de `Cleaner::clean` (fnmatch POSIX sans
  drapeau porté en Rust : `*`, `?`, crochets, `\`), sur l'arbre réel ;
  `cleaner-ignore` de `extra` sinon de `config` ; `true`, chaîne ou liste de
  chaînes, autre → repli ; `-v` → repli ; `--no-install` → repli.
- **liborm85** : port de `Cleaner`/`DevFilesFinder`/`GlobFilter`/
  `Directory` (`Glob::toRegex` généralisé à `strictLeadingDot = false`,
  regex PCRE2 de vivacity, drapeau `i` pour `match-case: false`) ; deux
  temps (`pre-autoload-dump` : paquets ; `post-install-cmd` : bin et bilan) ;
  `dump-autoload` : tout au `pre-autoload-dump`. Repli : `dev-files`
  chaîne (fichier externe) ou hors schéma, `config.dev-files` global,
  dépôt local à alias (version dev ou `aliases` du lock), paquet `path`,
  analyse des motifs inutilisés (commande `install`, plugin chargé au début
  et chaque paquet réinstallé), `--no-autoloader` (le bilan passe par le
  destructeur), `-v`, `--no-install`.
- **cakephp** : `pre-autoload-dump` : psr-4 des plugins applicatifs ajoutés
  au manifeste racine du dump ; `post-autoload-dump` : `cakephp-plugins.php`
  (dépôt local : lock voulu à l'install, installed.json au dump-autoload).
  Repli : `plugin-paths` hors schéma ou absolu, paquet `cakephp-plugin` sans
  espace de noms primaire (RuntimeException), valeur psr-4 non chaîne quand
  le paquet en a plusieurs.

## Replis nommés (avant toute écriture)

Une version non lue de chacun ; les conditions ci-dessus ; drupol,
attribute-collector, class-alias-loader, cms-composer-installers, patcher,
contao/manager-plugin (inchangé : « not on vivacity's known-plugin list »).

## Fichiers

1. `harness/plugins-round2.sh` (nouveau, dans `ci.yml` par `net-retry.sh`).
2. `crates/vivacity-core/src/` : un module par plugin porté
   (`cakephp_plugins.rs`, `composer_cleaner.rs`, `vendor_cleaner.rs`,
   `laminas_skeleton.rs`) et les gardes d'inertie (`inert_plugins.rs`) ;
   `scope.rs` (listes, versions, gardes) ; `path_install.rs`
   (`glob_to_regex` paramétré).
3. `crates/vivacity/src/` : `install.rs` (écouteurs, ordre, priorité 1000),
   `dump.rs` (cakephp, liborm85), `verbose_listeners.rs`, `no_install.rs` /
   `scope.rs` (`--no-install`) ; `vivacity-autoload` (contournement du
   cache du store).
4. `harness/corpus.sh` : projet entier pour skeleton-installer (composer.json)
   et sculpin.
5. Docs : CHANGELOG `[Unreleased]`, DECISIONS (entrée datée), HANDOVER,
   README (liste des plugins émulés), NOTICE (sept copies), ce plan.

## Cas de harnais

`harness/plugins-round2.sh` : par plugin, un projet qui exige le vrai plugin
à sa version exacte (Packagist) et des paquets synthétiques servis par un
dépôt `artifact` (zips ZipArchive) ; locks écrits par Composer
`--no-plugins`. Étapes : install (vendor vierge), no-op, mise à jour d'un
paquet (second lock), retrait (troisième lock), `--no-plugins` ; dump-autoload
pour cakephp et liborm85 ; par étape : code, stdout, stderr, projet entier
(fichiers, modes, liens, installed.json). Plus, sous `--no-fallback` :
version non lue, chaque condition de repli → code 3, la raison, rien
d'écrit. Rouges sur d2b294c, verts après. Chaque harnais touché passe sous
PHP 8.4 et 8.5.

## Critère « vert »

Les cas verts sous les deux PHP ; `cargo fmt --all -- --check` ; clippy
`-D warnings` (hôte et `x86_64-pc-windows-gnu`) ; `fixtures/make.sh` puis
`cargo test --workspace` ; tous les harnais de `ci.yml` et
`root-manifest.sh` à 0 ; corpus complet, rapport si le compte change ;
`--only` sur les sept entrées : natives, 0 diff (projet entier ; stdout et
stderr comparés à la main).

## Non-objectifs (vus, laissés)

- La copie de thème de sculpin, l'injection de configuration de
  laminas-component-installer, les commandes de spiral : portables pour la
  première, mais aucune entrée ne les exige.
- Les six plugins non reproductibles ou hors de portée ci-dessus.
