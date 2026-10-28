# Installeurs personnalisés qui bloquent le corpus — plan

> Suite de `docs/corpus/2026-10-21.md` : sur 106 projets réels, 30 partent
> à Composer en `--no-dev`, dont 11 à cause de plugins `composer-plugin` qui
> enregistrent un installeur décidant où une famille de types est posée.
> Niveau 2 : parité à l'octet de vendor/, du lock, de stdout, de stderr, du
> code de sortie et des fichiers que les plugins émulés écrivent ; ce qui
> n'est pas reproductible part à Composer, nommé, avant toute écriture.
> Méta-analyse et revue par sous-agents frais.

## Le problème

`InstallationManager::addInstaller` met l'installeur d'un plugin en tête de
liste (`array_unshift`) ; `getInstaller($type)` rend le premier qui
`supports()` le type — le dernier activé gagne. vivacity émule trois de ces
installeurs (composer/installers 2.0.0–2.3.0, roots/wordpress-core-installer,
mnsami/composer-custom-directory-installer) ; tout autre plugin est « not on
vivacity's known-plugin list » et la commande part à Composer. Le scan hors
ligne (`install --check-scope --no-dev`, binaire de `main` 8a48734) des
entrées concernées donne leurs bloqueurs **complets** — une entrée n'est
native que si tous le sont :

| entrée | bloqueurs |
|---|---|
| craftcms-craft | craftcms/plugin-installer |
| getkirby-plainkit | getkirby/composer-installer |
| neos-base-distribution | neos/composer-plugin |
| phplist-phplist3 | davidbarratt/custom-installer |
| themosis | johnpbloch/wordpress-core-installer ; composer/installers v1.12.0 (non porté) |
| thelia-project | thelia/installer ; 13 modules que composer/installers pose sous `vendor/thelia/modules/` (« inside vendor/ », refusé) |
| ilias-elearning-ilias | simplesamlphp/composer-module-installer ; simplesamlphp/composer-xmlprovider-installer |
| contao-managed-edition | contao-components/installer ; **contao/manager-plugin** |
| friendica | oomphinc/composer-installers-extender ; **friendica/json-ld sans dist** |
| omeka-s | omeka/composer-addon-installer (paquet `path` **absent de la fixture**) |
| silverstripe-installer | **silverstripe/recipe-plugin**, **silverstripe/vendor-plugin** ; composer/installers `silverstripe` (logique propre) |
| mautic, mautic-recommended-project | composer/installers `mautic` (logique propre) ; **cweagans/composer-patches**, **mautic/core-composer-scaffold**, **mautic/core-project-message**, paquets sans dist, package-versions 1.11.99.1 |

En gras : des bloqueurs qui ne sont pas des installeurs de chemin et que ce
chantier ne lève pas.

## Environnement de mesure

Linux x86_64 (conteneur), PHP 8.5.12, Composer 2.10.3, utilisateur non
root. api.github.com refuse les zipballs dans cette session : le cache de
fichiers est amorcé par `git archive` du commit verrouillé (même archive
pour les deux outils, `/opt/harness-shim/seed-dists.sh`) ; wordpress.org,
wpackagist.org, packages.drupal.org et asset-packagist.org sont refusés
(themosis imprime en tête « Filter list data could not be fetched … » — une
ligne de cet environnement, pas du plugin).

## Références (sources lues aux versions verrouillées, clones git)

| plugin | version | licence | lu |
|---|---|---|---|
| getkirby/composer-installer | 1.2.1 | MIT | `Plugin`, `Installer::postInstall`, `CmsInstaller`, `PluginInstaller` |
| craftcms/plugin-installer | 1.6.0 | MIT | `Plugin::activate`, `Installer` (`addPlugin`, `savePlugins`) |
| neos/composer-plugin | 2.1.3 (= 2.1.1, 2.1.2) | MIT | `Installer` (préfixes, `deriveFlowPackageName`) |
| davidbarratt/custom-installer | 1.1.0 | Apache-2.0 | `CustomInstaller`, `Configuration`, `ConfigurationAlpha1` |
| johnpbloch/wordpress-core-installer | 2.0.0 | GPL-2.0-or-later | comparé ligne à ligne à roots v4.0.0 (GPL-2.0-or-later lui aussi, déjà émulé — NOTICE le disait MIT : copie retirée et fonction récrite depuis le comportement, DECISIONS 2026-10-28) : une seule différence, la règle de conflit |
| composer/installers | v1.12.0 | MIT | `Installer`, `BaseInstaller`, table par réflexion (`tools/gen-installers-table.php`) |
| thelia/installer | 1.6 | LGPL-3.0-or-later | `TheliaInstaller` (deux dispositions) |
| contao-components/installer | 1.4.1 (= 1.4.0) | LGPL-3.0-or-later | `Plugin`, `LibraryInstaller`, `NoopInstaller` |
| oomphinc/composer-installers-extender | 2.0.1 (= 2.0.0) | MIT | `Installer`, `CustomInstaller` (sur `BaseInstaller` de composer/installers) |
| omeka/composer-addon-installer | « 2.0 » (paquet `path`, omeka-s 95d7f4b) | GPL-3.0 | `AddonInstaller` |
| simplesamlphp/composer-module-installer | v1.4.0 | LGPL-2.1-only | `ModuleInstaller` |
| simplesamlphp/composer-xmlprovider-installer | v1.0.2 | LGPL-2.1-only | `XMLProviderInstaller` |
| contao/manager-plugin | 2.13.6 | LGPL-3.0-or-later | `ManagerPluginInstaller`, `AppAutoloadPlugin`, `ArtifactsPlugin` |

Rien n'est copié des sources GPL (johnpbloch, omeka) ni LGPL (thelia,
contao-components) : leur comportement est décrit ici et réimplémenté. Les
copies de référence (`docs/reference/plugins/`, `docs/reference/
installers-v1.12.0/`) ne prennent que les sources MIT et Apache-2.0
(kirby, neos, davidbarratt, oomphinc, composer/installers 1.12.0).
« = » : tags dont les sources lues sont identiques (`git diff --quiet`) ;
le garde de version n'admet que ceux-là.

## Classement mesuré

**Correspondance de chemin pure** (un type → un répertoire, éventuellement
d'après `extra`) :

- **neos/composer-plugin** : types `typo3-flow-*` et `neos-*` ;
  `package` → `Packages/Application/{nom}`, `plugin` → `Packages/Plugins/`,
  `site` → `Packages/Sites/`, `boilerplate` → `Packages/Boilerplates/`,
  `build` → `Build/`, `package-collection` → `Packages/`, tout autre →
  `Packages/{TypeEnCamelCase}/`. Le nom : `extra.installer-name`, sinon la
  première clé `psr-0` (`\` → `.`), sinon la première clé `psr-4` (sans le
  `.` final), sinon `extra.neos.package-key` valide ou le nom du paquet
  (`foo/bar-baz` → `Foo.Bar.Baz`).
- **davidbarratt/custom-installer** : `extra.custom-installer` de la racine,
  `{motif: [spécifications]}` — `type:X` pour un type, sinon un nom de
  paquet ; forme héritée (alpha1, toutes les valeurs scalaires) : `{type:
  motif}`. `supports` : un type nommé (jamais `metapackage` ni
  `composer-plugin`) ; le motif : par nom (`getName`, puis `getPrettyName`),
  sinon par type ; `strtr` de `{$type}`, `{$vendor}`, `{$name}` ; puis
  `/target-dir`. Mesuré sur phplist : les `library` vont à
  `vendor/{$vendor}/{$name}/` (le chemin même de LibraryInstaller), quatre
  paquets nommés hors de vendor/.
- **johnpbloch/wordpress-core-installer** : `wordpress-core` →
  `extra.wordpress-install-dir` (chaîne, ou carte par nom), défaut
  `wordpress` — la fonction de roots (`layout::wp_core_dir`).
- **composer/installers v1.12.0** : la même logique que 2.x pour ce que
  vivacity porte (`supports`, `BaseInstaller::getInstallPath`, la ligne
  « Deleting … » de `uninstall`), une table de 99 frameworks (39 à logique
  propre → repli, comme en 2.x). **Plus deux lignes** : sur PHP ≥ 8.4, le
  chargement d'`Installer.php` émet deux dépréciations de compilation
  (« Implicitly marking parameter $filesystem/$binaryInstaller as nullable
  ») ; l'`ErrorHandler` de Composer écrit la première avec le chemin absolu
  du fichier, puis « More deprecation notices were hidden, run again with
  `-v` to show them. » (une fois par processus), et sous `-v` des piles
  d'appels. Mesuré sur themosis : juste après « - Installing
  composer/installers (v1.12.0): Extracting archive ».
- **thelia/installer** : disposition Thelia 3 si `extra.installer-paths` de
  la racine contient `type:thelia-module` (gabarits `templates/frontOffice|
  backOffice|email|pdf/`, `thelia-local` → `local/`), Thelia 2 sinon (plus
  `thelia-module` → `local/modules/`, `thelia-core` → racine) ; nom =
  `extra.installer-name` non vide ou la partie après `/`. Mesuré : 4
  gabarits et 2 `local/` posés par lui, 13 modules sous
  `vendor/thelia/modules/` par composer/installers (`installer-paths`).
- **contao-components/installer** : `contao-component` →
  `extra.contao-component-dir` + `/` + `basename(nom)`.
- **oomphinc/composer-installers-extender** : les types de
  `extra.installer-types`, posés par l'entrée `installer-paths` qui les
  nomme (code de `BaseInstaller` de composer/installers) ; sans entrée,
  `InvalidArgumentException` (« Package type … is not supported »).
- **omeka/composer-addon-installer** : `omeka-s-theme` → `themes/{nom}`,
  `omeka-s-module` → `modules/{nom}`, nom = `extra.install-name` ou la
  partie après `/`.
- **simplesamlphp/composer-module-installer** : `simplesamlphp-module|
  assets` → **dans le répertoire d'un autre paquet**
  (`<vendor>/simplesamlphp/simplesamlphp/modules|public/assets/<x>`, ou `.`
  si la racine est simplesamlphp) : l'ordre de la transaction décide de qui
  écrase qui.

**Chemin et effets de bord** :

- **getkirby/composer-installer** : `kirby-cms` → `extra.kirby-cms-path`
  (chaîne ; non-chaîne → vendor/), défaut `kirby` ; `kirby-plugin` →
  `extra.kirby-plugin-path` (défaut `site/plugins`) + `/{nom}`, seulement
  si le paquet exige l'installeur (sinon vendor/). Effet : après
  l'installation ou la mise à jour d'un paquet pris en charge, son
  sous-répertoire `vendor/` est supprimé. Mesuré : la dist de getkirby/cms
  5.5.3 porte 300 entrées sous `vendor/` ; Composer les supprime, l'autoload
  n'y renvoie pas.
- **craftcms/plugin-installer** : rien ne change de chemin (vendor/), mais
  chaque paquet `craft-plugin` installé, mis à jour ou retiré réécrit
  `vendor/craftcms/plugins.php` (`var_export`, alias calculés depuis
  `getcwd()`), et une racine `craft-plugin` s'y inscrit à l'activation.
  Sans aucun paquet `craft-plugin` (craftcms-craft), il ne fait rien.
- **simplesamlphp/composer-xmlprovider-installer** : à chaque install
  (`ensureBinariesPresence`), un lien dur
  `…/composer-xmlprovider-installer/classes/element.registry.<sha1 du chemin
  absolu>.php` : un nom de fichier qui dépend du chemin du projet.
- **contao/manager-plugin** : classe générée à `post-install-cmd` (deux
  lignes « Dumping generated plugins file… »), autoload `App\` ajouté à la
  racine, dépôt d'artefacts.

**Hors de portée de ce chantier** : silverstripe/vendor-plugin (expose des
ressources par liens ou copies dans le projet), silverstripe/recipe-plugin
(copie des fichiers de recettes), composer/installers `mautic` et
`silverstripe` (logique propre).

## Rang : entrées débloquées par unité de travail

Mesure de Composer seul sur les six entrées que les ports peuvent lever
(`--no-dev`, `--ignore-platform-req=ext-* php`, code 0 partout) : les
chemins ci-dessus, et aucune autre ligne d'un plugin que les deux de
dépréciation de themosis.

| port | travail | entrées levées (tous leurs bloqueurs) |
|---|---|---|
| craftcms (inerte sans paquet `craft-plugin`) | très faible : un garde | craftcms-craft |
| johnpbloch + table installers v1.12.0 + deux lignes de dépréciation | faible (fonction de roots, table générée) + moyen (lignes) | themosis |
| neos | faible | neos-base-distribution |
| davidbarratt (+ cible sous vendor/ égale au chemin par défaut) | faible | phplist-phplist3 |
| getkirby (+ suppression de `<chemin>/vendor`) | moyen | getkirby-plainkit |
| thelia (+ cibles sous vendor/ hors de tout paquet, + priorité entre installeurs) | moyen | thelia-project |
| contao-components | faible | aucune (contao/manager-plugin) |
| oomphinc | faible | aucune (friendica/json-ld sans dist — chantier git-sources) |
| omeka | faible (garde par contenu) | aucune (le paquet `path` manque à la fixture) |
| simplesamlphp module + xmlprovider | élevé | ilias — **repli** (imbrication dans un autre paquet, liens durs nommés d'après le chemin absolu) |

Attendu : six entrées natives (craftcms-craft, getkirby-plainkit,
neos-base-distribution, phplist-phplist3, themosis, thelia-project), si
le double install du corpus ne révèle rien d'autre.

## Le modèle porté

1. **Un placeur par installeur** (`crates/vivacity-core/src/
   custom_installers.rs`) : `supports(type, paquet)` et
   `install_path(paquet) -> Result<Option<chemin>>` (`None` : le chemin de
   LibraryInstaller), chacun commenté avec la méthode du plugin qu'il
   reproduit. Activation comme les placeurs existants : verrouillé,
   **autorisé** (`allow-plugins` ; non couvert, le chargement l'a déjà
   refusé), plugins actifs, installed.json et lock d'accord quand un paquet
   d'un type pris en charge existe, **version lue** (sinon repli nommé,
   avant toute écriture).
2. **La priorité** : `getInstaller` rend le dernier installeur activé qui
   prend le type. vivacity ne calcule pas l'ordre d'activation pour cela :
   si plusieurs placeurs actifs prennent le type d'un paquet, tous ses
   chemins sont calculés ; différents, repli nommé (« the installer
   precedence follows the plugins' activation order »). Égaux : natif
   seulement si ce qui accompagne le chemin ne dépend pas du gagnant
   (révisé après la méta-analyse, B1) — repli pour kirby sur un paquet
   installé ou mis à jour (`postInstall`), pour johnpbloch (règle de
   conflit), et au retrait quand l'un des installeurs écrit « Deleting »
   (composer/installers, oomphinc). La règle existante composer/installers
   + mnsami reste.
3. **Cibles sous vendor/** : admises si elles ne sont ni le chemin d'un
   autre paquet, ni dans lui, ni au-dessus de lui, ni sous `vendor/composer`
   ou `bin-dir` ; la cible égale au chemin de LibraryInstaller est ce
   chemin. Le plan de suppression suit `getPackageBasePath` : le chemin
   moins `target-dir` (aujourd'hui `vendor/<nom>` pour tout chemin sous
   vendor/).
4. **getkirby** : après les opérations, pour chaque paquet `kirby-cms`, ou
   `kirby-plugin` qui exige l'installeur, installé ou mis à jour **par ce
   run**, `<chemin>/vendor` supprimé (Composer : `postInstall` dans la
   promesse de l'installation, avant le dump).
5. **craftcms** : actif sans aucun paquet `craft-plugin` (lock voulu,
   installed.json) et racine d'un autre type → rien ; sinon repli nommé
   (`plugins.php` non émulé).
6. **composer/installers v1.12.0** : sa table ; sur PHP ≥ 8.4 (version du
   `php` que la sonde de plateforme lit), les deux lignes, au niveau normal,
   là où Composer charge `Installer.php` : à la tête de la commande
   (plugin chargé depuis installed.json, avec les avertissements de
   chargement) ou juste après la ligne d'installation du plugin ; une mise à
   jour du plugin, `-v` et au-delà → repli nommé.
7. **contao-components** : `extra.contao-component-dir` chaîne → le chemin ;
   `config.component-dir` (avertissement), `config.contao-component-dir`
   (exception), ni l'un ni l'autre (NoopInstaller : rien posé) → repli.
8. **omeka** : non porté (décidé en cours de route) — GPL-3.0, paquet
   `path` que la fixture n'a pas et que le harnais ne peut embarquer :
   aucune entrée levée, aucun cas vérifiable.
9. Rien n'est émulé sous `--no-plugins` (tout dans vendor/, comme Composer) ;
   la transaction cherche chaque paquet au chemin recalculé
   (`LibraryInstaller::isInstalled`), ce qui réinstalle sous vendor/ ce
   qu'un installeur avait posé ailleurs, comme Composer.
10. **Ajouts de la méta-analyse et de la revue** : la ligne « Deleting »
   (B2), le répertoire vide que le téléchargement laisse au chemin de
   LibraryInstaller pour un paquet à `target-dir` posé ailleurs par un
   installeur que la transaction installe, et les rendus listés plus bas.

## Replis nommés (avant toute écriture)

Une version non lue de chaque plugin porté ; craftcms avec un paquet
`craft-plugin` ; kirby, neos, davidbarratt, thelia avec une configuration
hors schéma (non-chaîne là où PHP lèverait ou convertirait) ; oomphinc sans
entrée `installer-paths` (Composer lève) ; contao-components sans
`extra.contao-component-dir` ; deux installeurs aux chemins différents pour
un même paquet ; simplesamlphp/composer-module-installer et
composer-xmlprovider-installer (inconnus, inchangé) ; contao/manager-plugin,
silverstripe, mautic (inchangé).

## Fichiers

1. `harness/custom-installers.sh` (nouveau, dans `ci.yml`).
2. `crates/vivacity-core/src/custom_installers.rs` (nouveau) ;
   `layout.rs` (placeurs, priorité, cibles sous vendor/, suppression) ;
   `scope.rs` (`EMULATED_PLUGINS`) ; `installers.rs` + `assets/installers/
   v1.12.0.json` (`tools/gen-installers-table.php`).
3. `crates/vivacity/src/` : la suppression de kirby après les opérations ;
   les lignes de dépréciation (installation et chargement).
4. Docs : CHANGELOG `[Unreleased]`, DECISIONS (entrée datée), HANDOVER,
   README (liste des plugins émulés), NOTICE (copies de référence MIT et
   Apache-2.0), ce plan.

## Cas de harnais

`harness/custom-installers.sh` : par plugin, un projet qui exige le vrai
plugin à sa version exacte (Packagist) et des paquets synthétiques de ses
types servis par un dépôt `artifact` (zips fabriqués par ZipArchive, comme
`artifact-repo.sh`) ; lock écrit par Composer `--no-plugins`. Étapes :
install (vendor vierge), no-op, mise à jour d'un paquet placé (second lock),
retrait d'un paquet placé, `--no-plugins` ; par étape : code, projet entier
(fichiers, modes, liens, installed.json), stdout et stderr. Plus, sous
`--no-fallback` : une version non lue, chaque configuration hors schéma →
code 3, la raison, rien d'écrit. Rouges sur 0.24.0 (08b309c), verts après.

## Critère « vert »

Les cas du harnais verts ; `cargo fmt --all -- --check` ; clippy
`-D warnings` (Linux et `x86_64-pc-windows-gnu`) ; `cargo test --workspace`
(après `fixtures/make.sh`) ; tous les harnais de `ci.yml` et
`root-manifest.sh` à 0, sauf ce que le réseau de la session interdit (dit,
avec les relances restreintes) ; `harness/corpus.sh --only` sur les entrées
concernées : natives, 0 diff.

## Non-objectifs (vus, laissés)

- `vendor/craftcms/plugins.php` (paquets `craft-plugin`) : génération
  `var_export` portable, mais aucune entrée du corpus ne l'exige.
- simplesamlphp (ilias) : imbrication et liens durs ; contao/manager-plugin ;
  silverstripe ; mautic.
- La fixture omeka-s n'a pas le paquet `path`
  `application/data/composer-addon-installer` (tools/corpus-add.sh ne copie
  que l'arbre de l'autoload) : l'entrée reste en repli.
- roots/wordpress-core-installer et mnsami n'ont pas de garde de version
  (préexistant) : vu, laissé.

## Matrice mesurée (harnais, Composer 2.10.3, PHP 8.5.12)

`harness/custom-installers.sh` sur le binaire 0.24.0 (08b309c) : tous les
cas natifs en repli (code 3 contre 0), les garde-fous déjà verts
(`--no-plugins` des sept projets, les rendus `kirby-dot`, `cc-nodir`,
`oo-nopath`, `cr-plugin`). Ce que Composer imprime en plus, mesuré :

- composer/installers v1.12.0 (jp) : les deux lignes de dépréciation
  **juste après** « - Installing composer/installers (v1.12.0) » sur un
  vendor vierge, **en tête** (avant « Installing dependencies from lock
  file ») quand le plugin est chargé depuis installed.json (no-op, mise à
  jour, retrait).
- Le retrait d'un paquet que pose composer/installers (ou un installeur qui
  en hérite) écrit sur **stdout** « Deleting <chemin> - deleted »
  (`Installer::uninstall`) : chemin **relatif** tel que `templatePath` le
  rend en 1.12.0 et chez oomphinc (`web/plugins/wp-plug/`, `public/libs/np/`),
  **absolu** en 2.x (`getcwd() . '/' . chemin` — `…/vendor/acme/modules/
  Cheque`). vivacity ne l'imprime pas aujourd'hui, 2.x compris : défaut
  préexistant, levé ici.
- davidbarratt, contao-components, kirby, neos, thelia (gabarits) :
  aucune ligne propre.
- Défauts vus en construisant le banc, laissés : vivacity n'imprime pas
  « Skipped installation of bin … file not found in package » pour un
  binaire déclaré absent du paquet ; son store sert un zip `artifact`
  remplacé à nom et version égaux (la dist n'a pas de `reference`, le
  `shasum` du lock n'entre pas dans l'identité) — zone de l'extraction,
  laissée au chantier voisin.

## Méta-analyse (sous-agent frais, plan et code, sans mes conclusions)

2 bloquants, 6 majeurs, 8 mineurs ; ce qui a changé :

- **B1, priorité** : « chemins égaux, l'un ou l'autre » est faux — mesuré :
  un `kirby-plugin` pris par kirby et composer/installers 1.12 garde son
  `vendor/` au premier install (composer/installers, chargé en premier par
  la transaction, gagne… puis kirby au run suivant) ; et seul
  composer/installers écrit « Deleting » au retrait. → règle révisée
  (modèle, point 2), cas `kirby-mix`, `th-nofront`.
- **B2, « Deleting »** : jamais imprimé par vivacity, 2.x compris ; relatif
  en 1.12 et chez oomphinc. → porté (stdout, après le retrait).
- **M1** johnpbloch, « Two packages … cannot share the same directory! »
  (un `wordpress-core` remplacé par un autre au même répertoire) → rendu.
- **M2** binaires d'un paquet posé à un chemin relatif (`realpath`) → rendu
  pour un paquet `path` ; le binaire symlink dans une archive : laissé.
- **M3** dépréciations : état du processus PHP → rendues à côté d'un plugin
  hors de la liste lue, sous `-v`, avec un refus ; chemin sous
  `realpath(vendor-dir)` (vérifié dans `LibraryInstaller::
  initializeVendorDir`, contre la lecture de la méta-analyse : `Config`
  brut).
- **M4** oomphinc charge le `BaseInstaller` de composer/installers quelle
  que soit son autorisation → version gardée des deux côtés, 1.x rendu.
- **M5** le corpus ne comparait le projet entier qu'avec composer/installers
  autorisé → étendu à tout plugin de disposition émulé ; stdout et stderr
  des six entrées comparés à la main (ci-dessous).
- **M6** banc : ajoutés la priorité, le déplacement à la mise à jour
  (`kirby-move`), `dump-autoload` et `remove --no-update` (dépréciations en
  tête), `--no-plugins` sur l'arbre posé (qui a révélé l'écart de
  `isInstalled`), le filtre de la note « emulated natively ».
- Mineurs : 1, 2, 4, 5 (kirby `null`, thelia-core, davidbarratt `"0"`, neos
  `psr-0` vide) étaient déjà conformes ; 7 (installeur global) → rendu ;
  6 (plugin au répertoire absent) et 8 (fork au même tag) : laissés,
  HANDOVER.

## Revue indépendante (sous-agent frais, `git diff` depuis main)

3 majeurs, 8 mineurs ; ce qui a changé :

- **`target-dir`** : l'autoloader de Composer retranche `strlen('/' .
  targetDir)` de tout chemin, et seuls LibraryInstaller et davidbarratt
  l'ajoutent → rendu quand le chemin de l'installeur ne se termine pas par
  `/<target-dir>` (et segments `.`/`..` rendus) ; l'autoloader de vivacity
  retranchait le suffixe autant de fois qu'il se répétait → une fois
  (`kirby-td`).
- **Activation sans pose** : `update --no-install`, `remove --no-update`
  activent les plugins (`config.component-dir` de contao-components :
  ligne ou exception ; `installer-paths` scalaire de thelia ; oomphinc sur
  1.x ; version non lue) → rendus (`cc-config`).
- Mineurs corrigés : `wordpress-install-dir: true` lève ; `package-key`
  neos terminée par `\n` ; chemin oomphinc `"0"` → LibraryInstaller ;
  binaire kirby dans le `vendor/` retiré, classmap à joker ; 1.x sous
  Windows rendu ; `handed_on` vérifie aussi composer.json/lock. Licences :
  la ligne NOTICE de roots disait MIT (GPL-2.0-or-later) → corrigée,
  décision ouverte ; copies de référence et lignes NOTICE ajoutées pour les
  nouvelles origines MIT/Apache.

## Résultat (session de travail, Linux)

`uname -a` : `Linux vm 6.18.44-fc-v180 #1 SMP PREEMPT_DYNAMIC @0 x86_64 x86_64 x86_64 GNU/Linux` ; `nproc` : 4.

**Commits** (branche `custom-installers`, depuis main 8a48734) : 0804904
(début du plan, générateur de table pour 1.x), e1e4606 (plan), 06f92ae
(harnais rouge, ci.yml, matrice), a4fff97 (code), ea0f2ad (versions en
forme de date, corpus sur le projet entier), 26ed8fd (retours de la
revue), e57a73b (docs, copies de référence), 67f31ce (règle de
déplacement), puis ce résultat.

**Classement** : correspondance pure — neos, davidbarratt, johnpbloch,
composer/installers 1.12.0 (plus deux lignes de dépréciation), thelia,
contao-components, oomphinc, omeka, simplesamlphp module-installer
(imbrication) ; chemin et effets — getkirby (`vendor/` retiré),
craftcms (`plugins.php`), simplesamlphp xmlprovider (liens durs nommés par
le chemin absolu), contao/manager-plugin ; hors de portée — silverstripe,
mautic, composer/installers à logique propre.

**Portés** : getkirby 1.2.1, neos 2.1.1–2.1.3, davidbarratt 1.1.0,
johnpbloch 2.0.0, thelia 1.6, contao-components 1.4.0–1.4.1, oomphinc
2.0.0–2.0.1, craftcms 1.6.0 (inerte), composer/installers v1.12.0.
**Replis** : simplesamlphp ×2 (imbrication, liens durs), contao/
manager-plugin, silverstripe, mautic, `plugins.php` de craftcms, omeka
(GPL-3.0, paquet `path` absent de la fixture, 0 entrée) ; et les cas
nommés par la méta-analyse et la revue (sections ci-dessus).

**Corpus** (`harness/corpus.sh --only …`, dev et no-dev) : craftcms-craft,
getkirby-plainkit, neos-base-distribution, phplist-phplist3, themosis,
thelia-project — **natives, 0 diff** (projet entier), 12/12 ; stdout et
stderr comparés à la main : identiques, hors la ligne d'échec réseau de
wpackagist.org (hôte refusé ici, libellé différent) chez themosis.
phplist a d'abord montré un diff (`v2021-04` normalisé `2021.0.0.0`),
corrigé dans `version.rs`.

**Harnais** `custom-installers.sh` : 64 cas, 64 verts ; sur 0.24.0, 53
rouges et 11 déjà verts (les `--no-plugins` sur vendor vierge et les
rendus `kirby-dot`, `cc-nodir`, `oo-nopath`, `cr-plugin`).

**Barrières** : `cargo fmt --all -- --check` ; clippy `-D warnings` Linux et
`x86_64-pc-windows-gnu` ; `cargo test --workspace` 393 tests, 0 échec
(`fixtures/make.sh` sort en 100 : la fixture wordpress demande
downloads.wordpress.org, refusé ; les autres fixtures sont faites).
Harnais de `ci.yml` (utilisateur non root, cache amorcé par `git
archive`) : verts — transitions, path-repos, scripts, tar-dist,
bin-plugin, custom-dirs, custom-installers, package-versions,
plugin-lines, allow-plugins, flex-install, flex-update, flex-recipes,
artifact-repo, compare-selftest, two-stage, transport-exit, verbosity,
fallback-args, root-scan, drift-reference, et root-manifest.sh. Bloqués
par le réseau de la session, relancés restreints et verts : diff-vendor
(×3), update, steps, removal, vendor-dir, boot (sans wordpress/drupal).
Non exécutables ici : wp-core (downloads.wordpress.org), merge-plugin
(authentification github.com), yii2-composer (asset-packagist.org : seule
la ligne d'échec réseau diffère), download-lines (cache froid impossible
sans api.github.com) — les deux derniers échouent à l'identique avec le
binaire de main. Le passage complet a tourné sur 26ed8fd ; après 67f31ce,
les passages touchés par la disposition (custom-dirs, custom-installers,
transitions, path-repos, vendor-dir, removal) ont été relancés, verts.

**Défauts vus, laissés** : la licence de roots/wordpress-core-installer
(GPL-2.0-or-later, NOTICE corrigé ; tranchée ensuite, voir la section suivante) ; « Skipped
installation of bin … » non imprimé ; un zip `artifact` remplacé à nom et
version égaux resservi par le store ; `lock.rs::target_dir` sans la
réécriture des `..` ; plugin au répertoire absent, fork au même tag,
binaire symlink d'une archive sous un chemin relatif (HANDOVER).

## Vérification locale et fusion (2026-10-28)

La fusion dans main a d'abord été faite depuis e57a73b, avant que la
session ne pousse 67f31ce et la section ci-dessus ; la vérification a été
refaite sur `Darwin MacStudio-de-Adel-5.local 25.6.0
arm64` (`sysctl -n hw.ncpu` : 14), PHP 8.5.12, Composer 2.10.3, bash 3.2.

- **Harnais** : `harness/custom-installers.sh` 64/64 sous `/bin/bash` 3.2 ;
  sur le binaire de main 8a48734 : 53 rouges, 11 verts (les garde-fous).
- **Régression trouvée au balayage** : `custom-dirs.sh`, cas `unmapped`
  (`installer-paths` vidé puis `install`), natif sur main, partait à
  Composer : le refus « moves … » visait tout paquet dont le chemin
  change, alors que seul `updateCode` (une mise à jour) renomme. Restreint
  aux paquets mis à jour (94ea424) ; `custom-dirs.sh` 4/4. La session a
  corrigé la même régression de son côté (67f31ce : ancien chemin recalculé
  par les installeurs actuels) ; à la fusion, le commentaire de 67f31ce est
  gardé avec la condition de 94ea424, plus prudente : une mise à jour qui
  change aussi de chemin par la configuration n'a pas de cas de harnais,
  elle part à Composer.
- **Licence** : roots/wordpress-core-installer (GPL-2.0-or-later) — copie
  retirée de `docs/reference/`, `layout::wp_core_dir` récrite depuis le
  comportement mesuré (7cf1646 ; DECISIONS 2026-10-28) ; `wp-core.sh` 5/5,
  bedrock et wordplate natifs au corpus.
- **Ensemble avec `git-sources`** : fmt, clippy (hôte et
  `x86_64-pc-windows-gnu`), 401 tests, les 33 harnais de `ci.yml` et
  `root-manifest.sh` à 0 ; corpus complet (`docs/corpus/2026-10-28.md`) :
  85/106 natifs en `--no-dev`, 83/106 en dev, 0 diff, 0 indisponible. Les
  six entrées visées sont natives, et friendica aussi (oomphinc ici, son
  paquet git seul dans `git-sources`).
- Non exécuté : `windows.yml` (pas de machine Windows ici).
