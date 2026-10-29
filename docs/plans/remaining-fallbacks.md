# Les replis restants du corpus — plan

> Suite de `docs/corpus/2026-10-29-duration-drupol-collector.md` : 93/106
> natives en `--no-dev`, 91/106 en dev, 0 diff. Six entrées dans le champ,
> cinq plugins (plus dealerdirect 0.7.2). Niveau 2 : parité à l'octet de
> vendor/, du lock, de stdout, de stderr et du code de sortie ; ce qui n'est
> pas reproductible part à Composer, nommé, avant toute écriture.
> Méta-analyse et revue par sous-agents frais. Même méthode que
> `docs/plans/plugins-round2.md` et `docs/plans/duration-drupol-collector.md`.
> Hors champ (consigne) : Drupal ×3, mautic ×2, silverstripe, ilias,
> omeka-s, phpstan-src.

## Environnement de mesure

`Darwin MacStudio-de-Adel-5.local 25.6.0 arm64`, PHP 8.5.12 (8.4.27 pour la
double passe des harnais), Composer 2.10.3, bash 3.2, `/usr/bin/patch`
(« patch 2.0-12u11-Apple », sans `--no-backup-if-mismatch`), git 2.54.0.
Binaire de référence : main ad1c338.

## Le problème

| entrée | mode | bloqueur(s) |
|---|---|---|
| roave-backwardcompatibilitycheck | dev | infection/extension-installer 0.1.2 |
| chamilo-lms | dev | dealerdirect/phpcodesniffer-composer-installer v0.7.2 |
| typo3 | les deux | typo3/class-alias-loader v2.0.1 + typo3/cms-composer-installers v5.0.2 |
| typo3-cms-base-distribution | les deux | les deux mêmes |
| concretecms | les deux | mlocati/composer-patcher 1.2.6 |
| contao-managed-edition | les deux | contao/manager-plugin 2.13.6 |

Le verdict du 2026-10-28 (« rendus », une ligne chacun) est réexaminé
plugin par plugin : source lue au tag verrouillé quand la licence le
permet, comportement mesuré contre Composer sur l'entrée du corpus et sur
des projets synthétiques.

## Sources et licences (lues au tag verrouillé, dist du lock)

| plugin | version | licence | traitement |
|---|---|---|---|
| infection/extension-installer | 0.1.2 (`9b351d2`) | BSD-3-Clause | lu, porté ; copie de référence |
| dealerdirect/phpcodesniffer-composer-installer | v0.7.2 (`1c968e5`) | MIT | lu (diff contre v1.2.1) |
| typo3/class-alias-loader | v2.0.1 (`c44de30`) | MIT | lu, porté ; copie de référence |
| typo3/cms-composer-installers | v5.0.2 (`becd622`) | GPL-2.0-or-later | **non lu** (README seul) ; comportement observé |
| mlocati/composer-patcher | 1.2.6 (`266c802`) | MIT | lu, porté ; copie de référence |
| contao/manager-plugin | 2.13.6 (`747e947`) | LGPL-3.0-or-later | **non lu** ; comportement observé |

Pour les deux sources GPL/LGPL : rien n'est copié, rien n'est porté, rien
n'est versé dans le dépôt. Ce que vivacity en reproduit est décrit par des
expériences (entrée, sortie mesurée), à la manière de `wp_core_dir`
(DECISIONS, 2026-10-28).

## Classement mesuré

### A. infection/extension-installer 0.1.2 — porté

`Plugin::process` (`POST_INSTALL_CMD`, `POST_UPDATE_CMD`, priorité 0) :

- `file_exists(__DIR__)` faux → une ligne (« Package not found… ») ; ne se
  produit pas quand le plugin est installé (il est retiré des écouteurs
  quand on le désinstalle : `PluginManager::removePlugin`).
- pour chaque paquet du dépôt local (`getPackages()`, alias compris) de type
  `infection-extension` : `extra.infection.class` absent ou `null`
  (`?? null`, donc aussi `extra.infection` non tableau) → invalide (raison
  fixe) ; sinon `install_path` (`getInstallPath`, absolu, `realpath` du
  vendor comme phpstan), `extra` (`extra.infection`), `version`
  (`getFullPrettyVersion()`), clé = nom, `ksort`.
- `file_put_contents(__DIR__.'/GeneratedExtensionsConfig.php',
  sprintf(TEMPLATE, var_export($data, true)))` **à chaque passage** (pas de
  comparaison md5, au contraire de phpstan) ;
- lignes stdout (`<info>`/`<comment>` retirés) : « No extensions found »
  quand rien ; sinon « Extensions installed » puis « > <nom>: installed » par
  extension ; puis « Invalid extensions: » et « > <nom>. (<raison>) ».
- Entrée du corpus : aucune extension (`roave-backwardcompatibilitycheck`,
  dev) : la ligne « No extensions found » et le fichier au tableau vide —
  le fichier du dist (`= [];`) diffère du généré (`= array (\n);`, sans
  l'en-tête de licence).

Modèle : `extension_installers::infection` à côté de phpstan/rector (même
`absolute_install_path`, même `full_pretty_version`, même écriture par
fichier temporaire et `rename` — le fichier vit dans une copie du store).
Le garde des alias (`aliased_listing_issue`) s'étend à infection : un
paquet listé qui porte un alias dans le dépôt local (la version écrite est
celle de la dernière entrée du nom) part à Composer. `EMULATED_VERSIONS` :
`0.1.2.0` seul (« other versions were not read »). `-v` : une entrée
`VERBOSE_PLUGINS` (`Infection\ExtensionInstaller\Plugin`, `process` ×2).

### B. dealerdirect v0.7.2 — portable, non porté (mesure impossible sur le corpus)

Différences avec v1.2.1 (diff lu) : la recherche part **toujours** de la
racine du projet (`$searchPaths = [cwd]`, pas seulement quand la racine est
de type `phpcodesniffer-standard`) ; `--config-show` par `phpcs` du `PATH`
(le `bin-dir` y est préfixé par `EventDispatcher::ensureBinDirIsInPath`),
lancé dans le `bin-dir` ; `./scripts/phpcs` pour phpcs 2 ; les chemins
relatifs gardés s'ils existent depuis le répertoire courant (comme v1.0–1.2.0)
; les messages `-v` diffèrent.

Mesure sur l'entrée : **la référence échoue**. phpcs charge
`vendor/autoload.php`, dont l'autoload `files` de la racine
(`public/legacy.php`) `require` treize fichiers absents de la fixture ;
ajoutés, ils appellent la classe `Database` d'un répertoire `classmap`
(lui aussi absent). Composer imprime « Failed to set PHP CodeSniffer
installed_paths Config » et `corpus.sh` classe l'entrée `unavailable`.
Porter v0.7.2 ferait passer chamilo (dev) de `fallback` à `unavailable`,
contre la barrière « 0 unavailable ». **Décision : pas de port dans ce
chantier** ; le résultat est consigné (DECISIONS) avec les deux suites
possibles : compléter la fixture (le code legacy de chamilo, plusieurs Mo),
ou porter v0.7.2 et accepter l'entrée hors mesure.

### C. typo3 : class-alias-loader v2.0.1 porté, cms-composer-installers inerte sous la racine `typo3/cms`

**class-alias-loader** (lu) — `pre-autoload-dump`, `onPreAutoloadDump` :

- `packageMap` = racine + paquets canoniques ; un paquet (ou la racine) qui
  déclare `extra.typo3/class-alias-loader.class-alias-maps` → les cartes
  sont `require` (code PHP du paquet) : **rendu**, nommé.
- sans carte et sans `always-add-alias-loader` (de la racine, cast `(bool)`
  PHP) : retourne `false`, rien d'écrit, rien d'imprimé.
- sinon : suffixe = celui de `vendor/autoload.php` **tel qu'il existe au
  `pre-autoload-dump`** (regex `ComposerAutoloaderInit([^:\s]+)::`) sauf
  `config.autoloader-suffix` (qui l'emporte) ; à défaut `md5(uniqid('',
  true))` — **aléatoire sur un vendor vierge** (mesuré : deux installs de
  Composer, deux suffixes). Ce n'est pas le suffixe de Composer (le
  `content-hash`) : le plugin passe avant que Composer écrive autoload.php.
- écrit `vendor/typo3/alias-loader-include.php` (gabarit, jetons
  `{$suffix}` et `{$prepend}` — `prepend-autoloader` false → `false`) et
  `vendor/composer/autoload_classaliasmap_static.php` (`var_export` des deux
  cartes vides) ; ajoute `<vendor-dir relatif>/typo3/alias-loader-include.php`
  à la fin de `autoload.files` de la racine ; stdout « Generating empty class
  alias map file » (« Generating class alias map file » avec cartes) ;
  stderr à `-v` seulement.

**cms-composer-installers** (GPL, non lu) — observé, `install -v` et
comparaison allow `true`/`false`, mêmes entrées :

| projet (synthétique sauf mention) | effet du plugin |
|---|---|
| typo3 (corpus, racine `typo3/cms`, type `typo3-cms-core`) | aucun (seul le suffixe aléatoire diffère entre deux runs) |
| racine `typo3/cms` + paquet `typo3-cms-extension` + `extra.typo3/cms.web-dir` | aucun ; `dump-autoload` : aucun |
| racine `typo3/cms` + `config.vendor-dir` | aucun |
| racine quelconque, sans typo3/cms-core (nu, extension, `web-dir`, typo3/cms-cli, `extra.typo3/cms` d'un paquet, racine de type `typo3-cms-core` ou `typo3-cms-extension`, racine qui `replace` typo3/cms-core) | écrit `vendor/typo3/autoload-include.php` et l'inscrit dans l'autoload `files` de la racine sous un identifiant qui dépend du chemin absolu |
| typo3-cms-base-distribution (corpus, typo3/cms-core installé) | `vendor/typo3/PackageArtifact.php` (objets `TYPO3\CMS\Core\Package\Package` sérialisés, code de typo3/cms-core), `public/_assets/<md5>` (liens), `public/index.php`, `var/` ; **l'ordre de `composerNameToPackageKeyMap` change d'un run de Composer à l'autre** dans le même répertoire (trois runs, trois ordres) |

À `-v` : quatre lignes d'écouteurs (`listen` ×2 à `pre-autoload-dump` et à
`post-autoload-dump`) et « Executing TYPO3 installer scripts ».

Décision : émulé **inerte quand la racine s'appelle `typo3/cms`**, et
seulement là ; partout ailleurs rendu, nommé (« … would write
vendor/typo3/autoload-include.php (not emulated) » ou, avec typo3/cms-core
installé, la raison de base-distribution). `-v` rendu
(`VERBOSE_DELEGATED`) pour les deux plugins (l'ordre des lignes d'écouteurs
de cms-composer-installers autour de la ligne « Register … » de
class-alias-loader n'est pas mesuré). **typo3-cms-base-distribution reste
rendue** : résultat négatif (Composer ne se reproduit pas lui-même, et le
contenu vient du code de typo3/cms-core).

**Le suffixe aléatoire** : vivacity tire le sien (md5 d'une graine
temps/pid, sans nouvelle dépendance) dans les mêmes cas que le plugin.
`harness/lib/compare.sh` ramène la classe `ClassAliasLoaderStaticInit<hex>`
à un marqueur dans ces deux fichiers seulement (ancré, comme
`GENERATED_AT`) ; `compare-selftest.sh` vérifie l'ancrage. Le no-op (suffixe
lu dans autoload.php) reste comparé strictement.

Modèle : `vivacity_core::class_alias_loader` (gabarits, rendu, décision) ;
au `pre-autoload-dump` (`effect_plugins::pre_autoload_dump`, dans l'ordre
des écouteurs) : lire le suffixe, écrire les deux fichiers (temporaire +
`rename`), étendre l'autoload racine ; `dump_depends_on_listeners` vrai
(pas de plan anticipé). Décision dans `effect_plugins::issues` (deux
nouveaux `READ_VERSIONS` : `2.0.1.0`, `5.0.2.0`).

Rendus (avant toute écriture) : une carte d'alias quelque part ; une autre
version ; Windows ; `config.vendor-dir` réglé (le chemin relatif inscrit
n'est pas mesuré) ; `autoload.files` de la racine présent et non liste ;
plugin global ; `-v`.

### D. mlocati/composer-patcher 1.2.6 — porté pour l'application par `patch`

Lu en entier. Écouteurs : `PRE_INSTALL_CMD`/`PRE_UPDATE_CMD` →
`preOperation`, `POST_INSTALL_CMD`/`POST_UPDATE_CMD` → `postOperation`.

- **Collecte** (`PatchCollector::collectPatches`) : la racine
  (`extra.patches`, `extra.patches-file`), puis, si `extra.allow-subpatches`
  (`true` ou liste de noms), chaque paquet du dépôt local autorisé, dans
  l'ordre du dépôt local. Une ligne « Gathering patches from <nom>
  (extra.patches). » par source. Clé `"<nom>[:<contrainte>]"`, valeur
  `{description: chemin | {path, levels}}` ; niveaux par défaut `-p1 -p0 -p2
  -p4` ; chemin relatif au répertoire d'installation de la source.
- **pre** : avec des correctifs, désinstalle un paquet dont
  `extra.patches_applied.hash` diffère du hash courant (ou est vide) pour le
  faire réinstaller → **rendu**.
- **post** : nouvelle collecte (nouvelles lignes « Gathering »), « No
  patches supplied. » si vide ; sinon, pour chaque paquet du dépôt local
  puis la racine, chaque correctif qui le vise : « Applying patch
  <source>/<description> to <nom>... » puis « done. » ou « patch was already
  applied. » (même ligne). Par niveau : `patch <niveau>
  [--no-backup-if-mismatch] -N -d <rép> -i <fichier> -r <tmp>.rej --dry-run`
  (« Skipping patch. » + « N out of N hunks ignored » → déjà appliqué), puis
  sans `--dry-run`. `patch --version` puis `patch --help` une fois.
  Tous les correctifs du paquet passés → `extra.patches_applied = {hash,
  list}` (hash = sha1 des sha1 de fichiers triés, joints par une espace ;
  `from-package` = `getPrettyString()`), installed.json réécrit
  (`LocalRepository::write`).
- Mesuré sur concretecms (double passe du corpus) : quinze correctifs
  appliqués à huit paquets, `patches_applied` dans installed.json, rien
  d'autre ; un no-op : « patch was already applied. » ×15.

Modèle : `vivacity_core::composer_patcher` — collecte, appartenance
(`isForPackage` : nom, puis contrainte contre la version normalisée),
hash, commandes (`sh -c`, **le même binaire `patch`** que Composer
trouverait : `PATH` préfixé du `bin-dir` réel comme
`ensureBinDirIsInPath`), analyse de la sortie ; avant d'appliquer, la copie
du paquet visé est **détachée du store** (chaque fichier recopié, nouvel
inode) ; installed.json réécrit par `state::write_state_files` avec
l'`extra` augmenté (clé ajoutée à la fin). L'ordre des paquets : celui du
dépôt local de vivacity (installed.json pour un no-op — identique à
Composer ; ordre des opérations pour un install vierge, où Composer suit
l'ordre d'achèvement des extractions, non déterministe : le harnais compare
alors les blocs « Applying » triés par paquet, l'ordre interne au paquet
restant strict).

Rendus (avant toute écriture) : autre version ; Windows ; un paquet à
désinstaller au `pre` ; `extra.patches-file` ; un chemin distant
(`scheme://`) ; `patch-temporary-folder` ; `patch-errors-as-warnings`
faux ; une cible avec `.git` (GitPatcher) ou installée depuis la source ;
une cible qui est la racine ; une cible aliasée dans le dépôt local
(version dev, alias du lock) ; un `levels` non liste de chaînes ; un
correctif dont la clé ou la valeur n'a pas la forme attendue ; `-v` ;
`update`/`require`/`remove` (non mesurés : rendus) ; plugin global ; `patch`
introuvable. Un correctif qui **échoue** (après écriture, imprévisible) :
la ligne d'erreur de Composer est reproduite à la forme (le chemin du
`.rej` est aléatoire chez lui) et le paquet n'a pas de `patches_applied`.

### E. contao/manager-plugin 2.13.6 — résultat négatif, mesuré

- `post-install-cmd` → `ManagerPluginInstaller->dumpPlugins` (ligne `-v`),
  deux lignes stdout, un fichier `vendor/contao/manager-plugin/.generated/
  plugins.php` (une liste `'<paquet>' => new \<Classe>()`).
- **Le code des paquets s'exécute** : une instruction ajoutée en tête du
  fichier de la classe `Contao\NewsBundle\ContaoManager\Plugin` (installée)
  laisse sa trace pendant un `composer install` no-op.
- L'ordre de la liste n'est ni celui du lock, ni l'ordre alphabétique, ni
  un ordre topologique des `require` (core-bundle, dont dépendent les sept
  autres, arrive septième sur huit).
- Deux installs vierges de Composer : identiques.

Reproduire l'ordre demanderait d'évaluer ce que les classes des paquets
déclarent à l'exécution. Rien n'est porté.

## Fichiers

- `crates/vivacity/src/extension_installers.rs` (A), `install.rs`,
  `no_install.rs` (branchement `POST_*_CMD`), `verbose_listeners.rs`,
  `crates/vivacity-core/src/scope.rs` (`EMULATED_PLUGINS`,
  `EMULATED_VERSIONS`, garde des alias).
- `crates/vivacity-core/src/class_alias_loader.rs` (nouveau),
  `effect_plugins.rs` (cœur et appli), `dump.rs`.
- `crates/vivacity-core/src/composer_patcher.rs` (nouveau), son
  branchement `PRE_*`/`POST_*_CMD` dans `install.rs`.
- `docs/reference/plugins/{infection-extension-installer,class-alias-loader,
  composer-patcher}/` (sources lues, licence) ; NOTICE.md.
- `harness/plugins-round3.sh` (A, C), `harness/composer-patcher.sh` (D),
  `harness/lib/compare.sh` + `compare-selftest.sh` (suffixe), `ci.yml`.
- Docs : CHANGELOG, DECISIONS, HANDOVER, README, NOTICE, ce plan.

## Cas de harnais (contre Composer 2.10.3, PHP 8.5 puis 8.4)

- **infection** : 0.1.2 + deux extensions synthétiques (une valide, une sans
  `class`) + une sans extension ; install, no-op, mise à jour, retrait,
  `--no-plugins` posé et vierge, `--no-dev` (plugin en dev), `-v`, `update` ;
  rendus : 0.1.1, extension aliasée.
- **typo3** : racine `typo3/cms` avec class-alias-loader 2.0.1 +
  cms-composer-installers 5.0.2 (Packagist), `always-add-alias-loader` ;
  install vierge (suffixe masqué), no-op (strict), `dump-autoload`,
  `--no-plugins`, `config.autoloader-suffix`, `prepend-autoloader: false`,
  sans `always-add` (inerte) ; rendus : racine autre que `typo3/cms`, carte
  d'alias, `-v`, 2.0.0.
- **patcher** : 1.2.6 + un paquet fournisseur de correctifs (artifact) +
  deux cibles (une contrainte de version qui correspond, une qui ne
  correspond pas), `allow-subpatches` liste ; install vierge (blocs triés),
  no-op (strict : « already applied »), mise à jour d'une cible (correctif
  réappliqué), `--no-dev`, `--no-plugins` ; correctif qui échoue ; rendus :
  `patches-file`, URL distante, hash changé (désinstallation), `-v`,
  `update`.
- Vus rouges : chaque harnais d'abord contre le binaire de main ad1c338
  (code 3 là où il attend 0, motif absent là où il attend un repli).

## Critère « vert »

`cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, les
harnais de `ci.yml` touchés et les nouveaux (8.5 et 8.4), corpus complet
(deux modes) : 0 diff, 0 unavailable, et natives attendues : roave (dev),
typo3 (deux modes), concretecms (deux modes) — 95/106 en `--no-dev`,
94/106 en dev si tout tient (compte corrigé par la méta-analyse : roave
n'est bloquée qu'en dev).

## Non-objectifs

- typo3-cms-base-distribution, contao-managed-edition : négatifs mesurés.
- dealerdirect 0.7.2 : décision consignée, pas de code.
- Les correctifs par `git apply`, les fichiers de correctifs JSON, les
  correctifs distants : rendus.

## Méta-analyse (sous-agent frais, plan et code ; sources GPL/LGPL non ouvertes)

Huit failles, toutes retenues :

| faille | gravité | correctif retenu |
|---|---|---|
| `--run-scripts` : `composer run-script <événement>` rejoue les écouteurs des plugins ; les trois nouveaux impriment (ou écrivent) à chaque passage, et le `preOperation` du patcher peut désinstaller une cible au milieu du run (`scripts.rs:1-12`, précédent `drupol_replay_issue`) | majeure | repli décidé avant le premier script : infection et `post-install-cmd` ; patcher et `pre-install-cmd`/`post-install-cmd` ; class-alias-loader et `pre-autoload-dump` (install et `dump-autoload`) ; un cas de harnais par plugin |
| Cache de classmap par entrée du store (`generator.rs:296-357`) : une copie patchée n'est plus l'entrée du store ; un no-op `-o` ou un `dump-autoload -o` servirait l'enregistrement non patché | majeure (latente) | `mutable_copies` reçoit toute cible d'un correctif collecté et tout paquet d'installed.json qui porte `patches_applied` ; cas de harnais : un correctif qui ajoute une classe sous `classmap`, no-op `-o`, `dump-autoload -o` |
| Comptes attendus faux | moyenne | 95/106 `--no-dev`, 94/106 dev |
| Fournisseur de correctifs aliasé (collecté deux fois, `from-package` de l'alias) ; fournisseur mis à jour dans le run alors qu'une cible inchangée porte déjà `patches_applied` (le `pre` lit l'ancien, le `post` empile) | moyenne | rendus avant toute écriture ; les entrées du `pre` sont installed.json (purgé) et les fichiers de l'ancien fournisseur, pas le lock |
| Détachement du store sur une cible `path` (lien symbolique vers les sources) | moyenne | cible `path` rendue ; le harnais vérifie l'inode et le contenu de l'entrée du store après un correctif |
| Ordre des lignes entre écouteurs de `post-install-cmd` (roave : dealerdirect 1.2.1 + infection) non testé ; `$invalidInfectionExtensions` suit l'ordre du dépôt local (non trié) | moyenne | dealerdirect 1.2.1 ajouté au harnais infection (vierge et no-op) ; deux extensions invalides au no-op seulement (à l'install vierge, l'ordre de Composer est celui de l'achèvement des extractions) |
| Sous-processus `patch` : stdin hérité → « File to patch: » bloque ; un niveau raté donne « Skipping patch. 1 out of 1 hunk ignored » (lu « déjà appliqué ») | mineure | `Stdio::null()` ; environnement transmis tel quel ; un cas où `-p1` rate |
| Suffixe : `resolve_suffix` n'exige pas `::` ; `!$config->get('autoloader-suffix')` suit la vérité PHP (`"0"` faux) ; la graine aléatoire est sans importance sous le masque. dealerdirect 0.7.2 : la vraie raison est que `phpcs` exécute le PHP du projet (`autoload.files`), dont vivacity ne peut prédire l'échec | mineure | lecture propre au plugin ; vérité PHP ; la raison de B reformulée |

### Le modèle, révisé

- A : + co-écouteur dealerdirect 1.2.1 dans le harnais ; repli `--run-scripts`.
- C : lecture du suffixe propre au plugin ; repli `--run-scripts`.
- D : + rendus (fournisseur aliasé, fournisseur mis à jour avec une cible
  déjà patchée, cible `path`) ; `mutable_copies` ; stdin nul ; repli
  `--run-scripts`.
- B : la raison consignée est l'exécution du PHP du projet par `phpcs`
  (l'échec de la référence sur la fixture en est l'illustration mesurée).

## Revue indépendante (sous-agent frais, `git diff ad1c338..HEAD`, sources GPL non ouvertes)

Onze points ; suites :

| point | gravité | suite |
|---|---|---|
| Correctifs fournis par la racine : `from-package` bâti sur le champ `version` brut (Composer : `getPrettyString()` de la racine — version détectée, ou celle de l'alias), nom vide au lieu de `__root__` ; aucun cas natif au harnais | majeure | la racine passe par `RootPackage::detect` (`composer_patcher::root_entry`) ; noms en `getName()` (minuscules) dans les lignes, nom joli dans `patches_applied` ; deux cas natifs (racine nommée, racine sans nom) |
| Ordre des lignes du patcher après une mise à jour (Composer : installed.json moins les mises à jour, puis les opérations) ; la tolérance triée déplaçait toutes les autres lignes | moyenne | ordre du dépôt local (`install::local_order`, celui de pest) pour le patcher et infection ; tri limité à chaque suite contiguë de lignes « Applying » ; cas `pt-update-order` (une cible mise à jour passe en dernier) |
| infection : extensions invalides en ordre `packages` puis `packages-dev` au no-op | moyenne | même ordre du dépôt local (`update --no-install` : installed.json) ; cas prod/dev croisés au no-op ; à l'install vierge, l'ordre de Composer reste l'achèvement des extractions (documenté, comme pest) |
| class-alias-loader : `vendor-dir` venu de l'environnement ou de la config globale non vu | moyenne | repli sur `Dirs::resolve(…).vendor_rel() != "vendor"` |
| Contrainte de correctif sur une version de branche (`1.x-dev`) : jamais appliquée par vivacity | moyenne/mineure | repli nommé |
| GitPatcher annoncé rendu au plan, absent du code | mineure | porté (`git -C  <dir> apply …`, le même essai à blanc de `patch`) ; cas `pt-git` (un dist qui porte un `.git`) |
| `pre-install-cmd` sur un installed.json non purgé | mineure | `present_installed` : les paquets dont le répertoire existe (le plugin compris) |
| cms-composer-installers : garde plus large que l'observé (`--no-autoloader` hors `typo3/cms`, typo3/cms-core installé sous `typo3/cms`, pas de garde Windows) | mineure | inerte seulement sous `typo3/cms` sans typo3/cms-core ; tout le reste rendu ; garde Windows |
| installed.php : `self.version` des paquets *aliasés* du dépôt local | mineure, préexistant | non reproduit, non corrigé : HANDOVER |
| `--run-scripts` : phpstan imprime à chaque passage | mineure, préexistant | ajouté à `replay_issue` (la table annonçait couvrir ces plugins) |
| Le suffixe vérifié après tout le run, pas juste après le no-op | mineure | une vérification juste après un no-op |

## Résultat (session de travail, macOS)

| plugin | issue |
|---|---|
| infection/extension-installer 0.1.2 | porté |
| typo3/class-alias-loader v2.0.1 | porté (cartes d'alias rendues) |
| typo3/cms-composer-installers v5.0.2 | inerte sous `typo3/cms` sans typo3/cms-core (observé) ; le reste rendu |
| mlocati/composer-patcher 1.2.6 | porté (`patch`, et `git apply` pour un dist à `.git`) ; `update`, distant, `patches-file`, etc. rendus |
| contao/manager-plugin 2.13.6 | négatif mesuré |
| typo3-cms-base-distribution | négatif mesuré (PackageArtifact.php non reproductible par Composer lui-même) |
| dealerdirect v0.7.2 | non porté : la référence échoue sur la fixture chamilo |

- Harnais : `plugins-round3.sh` 52 cas, `composer-patcher.sh` 30 cas,
  verts sous PHP 8.5.12 et 8.4.27 ; contre le binaire de main ad1c338 :
  48 lignes FAIL / 5 OK et 28 FAIL / 3 OK (les OK : `--no-plugins`,
  `--no-dev` sans le plugin dev, le store intact). Les premières versions
  (avant la revue) : 43 FAIL / 6 OK et 21 FAIL / 3 OK.
- `compare-selftest.sh` : `cal-suffix` rouge avec le comparateur de main,
  vert ensuite ; 4 cas d'ancrage.
- Un test unitaire du détachement du store, rouge en neutralisant
  `detach`.
- Les 37 lancements de harnais de `ci.yml` verts sur le binaire d'avant la
  revue ; après la revue, les nouveaux et ceux qu'elle touche
  (plugin-lines, yii2-composer, plugins-round2, composer-packages,
  scripts, compare-selftest, package-versions, removal) verts.
- `cargo fmt --check`, `clippy -D warnings` (macOS et
  `x86_64-pc-windows-gnu`), `fixtures/make.sh`, `cargo test --workspace` :
  441 tests, 0 échec.
- Corpus (`docs/corpus/2026-10-29-remaining-fallbacks.md`) : 95/106
  natives en `--no-dev`, 94/106 en dev, 0 diff, 0 unavailable.
- Non vérifié ici : la voie GNU patch (Linux, la CI) — aucun `patch` GNU
  sur cette machine.
