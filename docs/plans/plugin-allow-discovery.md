# `allow-plugins` au chargement des plugins, et php-http/discovery à `POST_UPDATE_CMD` — plan

> Suite des « Restent » de HANDOVER (bullet « Ordre et lignes des écouteurs
> de plugins ») et des non-objectifs de `docs/plans/no-install-listeners.md` :
> deux écarts **silencieux** — Composer refuse ou réécrit, vivacity finit en
> 0 sans rien dire. Niveau 2 : parité à l'octet du code de sortie, des
> fichiers et des sorties ; ce qui n'est pas reproductible part à Composer,
> nommé, avant toute écriture. Méta-analyse et revue par sous-agents frais.

## Le problème

1. **Un plugin installé que `allow-plugins` ne couvre pas.** Composer le
   juge au chargement des plugins (`Factory::createComposer` →
   `PluginManager::loadInstalledPlugins`), donc **pour toute commande** :
   `install`, `update`, `require`, `remove`, `dump-autoload`, dry run et
   `-q` compris. Requis par la racine (directement ou non), il lève
   `PluginBlockedException` : code 1, rien d'écrit. vivacity installe,
   résout, écrit le lock et l'autoloader, code 0 — mesuré sur les cinq
   commandes (matrice ci-dessous). HANDOVER ne le notait que pour les
   commandes de résolution ; `install` et `dump-autoload` sont touchées
   aussi. Deux variantes voisines, mêmes conséquences : un lock antérieur à
   l'API de plugins 2.2.0 sans `allow-plugins` (`RuntimeException`), un
   plugin global non couvert par la configuration globale.
2. **Les à-côtés du même code.** Un plugin installé, non couvert et **plus
   requis** par la racine n'est pas chargé : Composer l'écrit (« The "x"
   plugin was not loaded as it is not listed in allow-plugins and is not
   required by the root package anymore. »), vivacity non. Et un plugin
   non couvert que la **transaction** installe ou met à jour arrête
   Composer en pleine installation (`PluginInstaller::prepare`), les paquets
   précédents posés, code 1 ; vivacity installe tout.
3. **php-http/discovery à `POST_UPDATE_CMD` avec installation.** Son
   `postUpdate` (1.20.0) cherche dans le dépôt local les implémentations des
   abstractions PSR exigées ; s'il en manque une, ou si l'implémentation
   d'un projet n'est qu'en dev, il édite composer.json et relance un
   `Installer`. Mesuré : `update` d'un projet dont guzzle n'est qu'en
   `require-dev`, ou dont l'abstraction est fournie par un paquet qu'il ne
   connaît pas — composer.json et le lock réécrits, update imbriqué ;
   vivacity : rien, code 0. Le chantier `--no-install` le rendait à Composer
   **dès qu'il surveille une abstraction**, ce qui est exact mais large ; le
   même calcul porté sert les deux.

## Environnement de mesure

Celui de `docs/plans/no-install-listeners.md` : Linux x86_64, PHP 8.5.12,
Composer 2.10.3, harnais lancés par un utilisateur non root (en root,
Composer ajoute « Plugins have been disabled automatically… » à toute
exception sous `--no-plugins`), cache de fichiers amorcé par `git archive`
(api.github.com refusé par le proxy de la session). Conséquence ici : un
update imbriqué de discovery qui doit télécharger un paquet neuf échoue en
100 chez Composer — le comportement à reproduire est l'édition et le
relancement, pas leur issue réseau ; vivacity les rend à Composer.

## Références (Composer 2.10.3, phar extrait ; plugins par git)

- `Plugin/PluginManager.php:90-91` : règles du projet
  (`parseAllowedPlugins(config allow-plugins, locker)`) et du global
  (`globalComposer` → sa config, sans locker) ; `:690-711`
  `parseAllowedPlugins` : `[]` + lock présent + `plugin-api-version` < 2.2.0
  → `null` ; `true` → tout ; `false` → rien ; sinon motifs
  (`packageNameToRegexp`).
- `:734-825` `isPluginAllowed($package, $isGlobal, $optional, $prompt)` :
  règles `null` et non interactif → `RuntimeException` (« Your
  composer.lock was generated before the allow-plugins security feature was
  introduced… ») ; première règle qui correspond → `$allow === true` ;
  `composer/package-versions-deprecated` → faux ; interactif → question
  (vivacity n'est jamais interactif) ; `$optional` → faux ;
  sinon `PluginBlockedException` (« <nom>[ (installed globally)] contains a
  Composer plugin which is blocked by your allow-plugins config. … You can
  run "composer [global ]config --no-plugins allow-plugins.<nom>
  [true|false]" … See https://getcomposer.org/allow-plugins »).
- `:102-112` `loadInstalledPlugins` : dépôt local (installed.json **avant**
  la purge, Factory.php:448/457), puis global ; `:490-540` `loadRepository` :
  ordre `PackageSorter` (poids -10000), `CompletePackage` de type
  `composer-plugin` / `composer-installer` seulement ; pour le dépôt local,
  `RepositoryUtils::filterRequiredPackages($packages, $root, true)` — un
  plugin hors de cet ensemble et non autorisé (`isPluginAllowed(…, true,
  false)`) imprime l'avertissement et n'est pas chargé ; sinon
  `registerPackage` → `:208` `isPluginAllowed(…, plugin-optional)` (refus
  silencieux, ou exception).
- `Repository/RepositoryUtils.php:36-56` `filterRequiredPackages` :
  `require` + `require-dev` de la racine, puis récursivement le `require`
  de chaque paquet atteint, par `getNames()` (nom, `provide`, `replace`).
- `Config.php:41` (`allow-plugins` vaut `[]` par défaut), `:207-210` (fusion
  des tableaux, clés du projet d'abord).
- `Installer/PluginInstaller.php:55-62` `prepare` : avant d'installer ou de
  mettre à jour un plugin, `isPluginAllowed(…, plugin-optional)`.
- `Console/Application.php` : l'exception rendue en encadré + synopsis,
  code 1 ; vivacity imprime le texte seul (déviation assumée pour les refus
  de `RootPackageLoader`, `harness/root-manifest.sh`) et l'annotation
  GitHub Actions.
- `RequireCommand.php:193` et `:409-410`, `RemoveCommand.php:96` et
  `:221-222` : deux `Composer` créés, donc deux chargements (l'avertissement
  deux fois, mesuré) ; le second sur le manifeste édité, avec les
  `allow-plugins` d'avant l'édition pour `remove` (:242-252).
- php-http/discovery `src/Composer/Plugin.php` (1.20.0) : `:46-90`
  `PROVIDE_RULES`, `:105-122` `INTERFACE_MAP`, `:145-176` `postUpdate`
  (épinglage par `extra.discovery`, puis `getMissingRequires`, puis
  `array_filter` : rien à faire → retour), `:243-366` `getMissingRequires`.
  Identiques (règles, `getMissingRequires`, `postUpdate`) de 1.19.0 à
  1.20.0 (diff git) ; 1.15.0–1.18.1 diffèrent. `extra.plugin-optional:
  true` : non couvert, il n'est simplement pas chargé.

## Matrice mesurée (Composer 2.10.3 contre vivacity 0.24.0+, f3f5524)

Projet phpstan/extension-installer 1.4.3 installé, puis `allow-plugins`
modifié (hors content-hash : le lock reste à jour).

| état | commande | Composer | vivacity (main) |
|---|---|---|---|
| requis, `allow-plugins` absent | install, update, update --no-install, update --dry-run, require x, require x --no-install, dump-autoload | encadré `PluginBlockedException`, code 1, rien d'écrit | natif, code 0, lock / vendor / autoloader écrits |
| idem | `-q` | l'encadré (niveau d'erreur), code 1 | rien, code 0 |
| idem | `--no-plugins` | natif, 0 | natif, 0 (identique) |
| requis, `{"other/x": true}` | les mêmes | les mêmes | les mêmes |
| requis, son répertoire supprimé | install | refus (chargé avant la purge) | réinstalle le plugin, 0 |
| requis seulement par un paquet (dealerdirect via phpcsutils) | install, update --no-install | refus | natif, 0 |
| plus requis, absent ou `false` | install, update, update --no-install, dump-autoload | « The "phpstan/extension-installer" plugin was not loaded as it is not listed in allow-plugins and is not required by the root package anymore. » en tête, puis la commande | la commande sans la ligne |
| idem | require x, remove y --no-install | la ligne **deux fois** (avant et après « ./composer.json has been updated ») | aucune |
| idem | `-q` | aucune | aucune |
| requis, `false` | install | natif (non chargé, rien d'imprimé) | identique |
| lock `plugin-api-version` 2.1.0, `allow-plugins` absent ou `{}` | install, update --no-install | `RuntimeException` « Your composer.lock was generated before… », code 1 | natif, 0 |
| plugin global, `allow-plugins` global absent | install, update --no-install | refus « (installed globally) », « composer global config » | install natif 0 ; update --no-install rendu (plugin global, chantier précédent) |
| vendor vierge, plugin dans le lock | install | « Installing phpstan/phpstan », puis refus au plugin, code 1, phpstan posé | tout installé, 0 |
| idem | update | lock écrit, puis le même refus, code 1 | tout installé, 0 |
| idem | update --no-install | rien de chargé : natif, 0 | identique |
| php-http/discovery non couvert (`plugin-optional`) | install, update --no-install | natif, 0 | identique |
| composer/package-versions-deprecated non couvert | install, update --no-install, dump-autoload | natif (refus silencieux) | identique |
| discovery, racine `project`, guzzle en `require-dev` seulement | update (avec installation) | « Nothing to modify », l'installation, puis composer.json et lock réécrits, update imbriqué (guzzle 7.10 → 8.3.1) | rien, 0 |
| discovery, abstraction fournie par php-http/mock-client | update | composer.json réécrit, update imbriqué (nyholm/psr7, symfony/http-client…) | rien, 0 |
| discovery, guzzle en `require` | update | rien | identique |
| le cas mock-client | update --no-install | réécriture, update imbriqué | rendu (chantier précédent) |

## Le modèle porté

**A. Au chargement** (`plugin_allow::load`, toutes les commandes, plugins
actifs) : les règles — `allow-plugins` du projet fusionné au global
(`Config::merge`) ; `[]` (absent ou objet vide) + lock présent dont
`plugin-api-version` (défaut `1.1.0`) < 2.2.0 → règles nulles ; pour le
global, la config globale (`config.json` puis le `config` de
`COMPOSER_HOME/composer.json`). Puis le dépôt local tel qu'écrit, trié
comme `loadRepository` (plugins seulement), et l'ensemble requis par la
racine (`filterRequiredPackages`, dev compris). Pour chaque plugin, dans
l'ordre : hors de l'ensemble et non autorisé → l'avertissement (niveau
normal, stderr), suivant ; sinon autorisé → chargé ; bloqué, package-
versions, `plugin-optional` → ignoré ; non couvert → refus ; règles nulles
→ `RuntimeException` au premier plugin jugé. Puis les plugins globaux,
mêmes jugements (sans l'avertissement). Le premier refus : son texte, code
1, rien d'écrit. Appelé juste après le refus de `RootPackageLoader` (le
paquet racine est chargé avant les plugins), dans `install`,
`dump-autoload`, `update`, `require` et `remove` ; pour ces deux derniers,
le second chargement (manifeste édité, règles du chargement pour `remove`)
imprime ses avertissements ; un refus qui n'apparaîtrait qu'au second
chargement rend la commande à Composer (non mesuré, voir non-objectifs).

**B. À l'installation** : une opération qui installe ou met à jour un
plugin non couvert (ni `plugin-optional`, ni package-versions) — Composer
s'arrête au milieu de la transaction. Rendu à Composer, nommé : à
`install` avant d'écrire (la transaction est connue), à
`update`/`require`/`remove` sur le lock résolu, avant de l'écrire.

**C. php-http/discovery** : `getMissingRequires` porté (1.19.0–1.20.0) en
prédicat « agirait » (au moins une des trois listes non vide après
`array_filter`), épinglage `INTERFACE_MAP` compris. Entrées : le dépôt
local au moment de `POST_UPDATE_CMD` — avec installation, le lock résolu
(paquets voulus, alias de branche et en ligne, noms dev du lock) ; sans
installation, installed.json purgé et ses `dev-package-names` —, la racine
(`require`, `require-dev`, `type`, `extra.discovery`). Agirait → rendu,
nommé, avant l'écriture du lock ; version hors 1.19.0–1.20.0 → rendu dès
qu'il surveille une abstraction (règle actuelle). La règle `--no-install`
actuelle (rendu dès qu'il surveille) devient ce prédicat : plus étroite,
exacte. Phalcon (`class_exists(Phalcon\Http\Message\RequestFactory)`) ne
peut que supprimer une action : ignoré, au pire un rendu de trop.

## Méta-analyse (sous-agent frais) et ce qui change

Un bloquant, quatre majeurs, cinq mineurs ; tous vérifiés dans les sources
avant d'être pris.

1. **Bloquant — `remove` ne juge qu'après l'édition.** Le premier
   chargement (`BaseCommand::initialize` → `tryComposer`) avale toute
   `RuntimeException` (`Application::getComposer(false)`,
   Application.php:630-645 ; `PluginBlockedException` en hérite).
   `RemoveCommand` édite composer.json, imprime « ./composer.json has been
   updated » (:210), puis `tryComposer` (:216, avalé), `resetComposer`,
   `requireComposer` (:222) qui lève. Retirer le plugin bloqué lui-même
   réussit donc (l'avertissement deux fois **après** l'édition) ; en
   retirer un autre refuse **après** l'édition, composer.json laissé
   édité. Pris : `remove` juge le manifeste édité ; imprimé une fois
   (premier chargement réussi : `tryComposer` rend l'objet en cache) ou
   deux fois (premier chargement refusé). `none-remove-ni` corrigé (le
   refus vient après « has been updated », composer.json édité) ; cas
   `remove-blocked-itself` et `remove-blocked-other` ajoutés.
2. **Majeur — un refus réimprime ce que Factory imprime.** Chaque
   création échouée recommence : `initialize` (avalée), la commande
   (`requireComposer`, lève), puis `hintCommonErrors` → `getComposer(false,
   true)` plugins coupés (Application.php:528-545). Soit, avant le texte :
   (lignes retenues + avertissements des plugins jugés avant le refus) ×2,
   l'annotation GitHub Actions (stdout), les lignes retenues ×1. Pris tel
   quel (lignes retenues : `io::held_lines`) ; cas
   `gone-then-blocked-install` et `lockfalse-blocked-install` (sous
   `GITHUB_ACTIONS`).
3. **Majeur — les règles nulles passent avant toute exemption.**
   `isPluginAllowed` teste `null` avant les règles, package-versions et
   `plugin-optional` (le code porté le fait déjà) ; vaut aussi à
   l'installation (B) : un lock ancien et un vendor vierge → rendu.
   `config.lock` faux → le lock est /dev/null, jamais « locked ». Cas
   `old-lock-optional-dump`, `old-lock-bare-install` (rendu),
   `old-lock-lockfalse` (garde-fou).
4. **Majeur — le Composer global.** Il n'existe que si
   `COMPOSER_HOME/composer.json` existe et se charge
   (`createGlobalComposer` avale l'échec) ; ses règles : `config.json`
   fusionné au `config` de ce composer.json (clés de ce dernier d'abord) ;
   son dépôt sous son `vendor-dir`. Cas `global-no-manifest` (garde-fou).
   `scope::active_plugins` lit le global sans ces conditions : préexistant,
   laissé (non-objectifs).
5. **Majeur — discovery, porte et point de décision.** `--dry-run` coupe
   `runScripts` (Installer.php:291-296) : pas de `POST_UPDATE_CMD`, rien à
   juger (`--no-scripts` ne coupe que les scripts). Avec installation, le
   discovery actif est celui du lock résolu (installé pendant la commande
   compris), autorisé, non désinstallé ; `--no-dev` : `$allPackages` sans
   les paquets dev, noms dev du lock entier. Particularités portées, une
   par test unitaire : `$devPackages += $names` sans effet (clés
   entières), `getNames(false)` sans `provide`, un nom remplacé présent
   (`isset` d'un `false`), `findPackage` par nom exact avec les alias,
   `array_diff_key` sur `[1]` avant l'aplatissement. Cas `disc-update-dry`
   (garde-fou), `disc-nodev-update`.
6. **Mineur — l'API de plugins avant `allow-plugins`.** `registerPackage`
   (PluginManager.php:172-205) teste `composer-plugin-api` (absent →
   autre exception ; incompatible avec 2.9.0 → ignoré avec un
   avertissement ; égal au texte « 2.9.0 » → avertissement) et flex
   < 1.9.8 avant le jugement. Pris : un refus sur un tel plugin est rendu
   à Composer (nommé) au lieu d'être imprimé.
7. **Mineur — l'installation (B).** `--no-plugins` et `--dry-run` en
   sont exempts, les désinstallations ne sont pas jugées. Cas
   `bare-install-dry`, `bare-update-dry` (garde-fous).
8. **Mineur — un refus que seul le second chargement de `require`
   lèverait** (le paquet demandé rend requis un plugin installé non
   couvert) : calculable d'avance ; rendu à Composer. Cas
   `require-makes-required`.
9. **Mineur — étiquettes.** `gone-quiet`, `disc-plain-update`,
   `bare-update-ni`, `none-no-plugins` passent déjà : ce sont des
   garde-fous (le harnais les range ainsi).
10. **Mineur — merge-plugin et `-v`.** L'ensemble requis vient de la
    racine non fusionnée (`INIT` après `loadInstalledPlugins`) : le
    jugement précède `merge_plugin`. Sous `-v`, l'encadré ajoute la classe
    de l'exception ; le harnais compare le texte extrait.

## Revue indépendante (sous-agent frais) et ce qui change

Deux majeurs, cinq mineurs, six détails ; vérifiés, puis :

1. **symfony/flex < 1.9.8** : le saut de `registerPackage` lit
   `getVersion()`, la version normalisée (`1.9.7.0` pour `v1.9.7`) ; le
   port lisait la version écrite et refusait là où Composer réussit.
   Corrigé (`version_normalized`, sinon normalisée) ; cas `old-flex-dump`.
2. **Root sans `COMPOSER_ALLOW_SUPERUSER`** : Composer coupe alors les
   plugins (Application.php:232-320) et ne juge rien ; vivacity ne modélise
   pas cette règle (écart préexistant). Un refus ou un rendu dans cette
   situation part à Composer, nommé.
3. **Ordre avec l'environnement** (`COMPOSER_IGNORE_PLATFORM_REQ(S)`,
   imprimé par `initialize` après le premier chargement) : les
   avertissements se placent juste après les lignes de `Factory`
   (`io::hold_factory`, `hold_after_factory`), et seules ces lignes se
   répètent sur un refus. Cas `env-gone-update-dry`,
   `env-gone-then-blocked-install`.
4. yii2-composer chargé avant le refus écrit extensions.php (`activate`) :
   reproduit. discovery antérieur à 1.15 (simple bibliothèque) : plus de
   rendu. Tests indépendants de `COMPOSER_HOME` (règles globales
   injectées). Le `type` de la racine lu en minuscules ; deux noms de
   tests rectifiés ; `remove` décide d'un rendu avant d'imprimer.

## Fichiers

1. `harness/allow-plugins.sh` (nouveau, ajouté à `ci.yml`) : les rouges
   d'abord.
2. `crates/vivacity/src/plugin_allow.rs` (nouveau) : règles, ensemble
   requis, jugement, messages, refus répété, contrôle à l'installation ;
   tests unitaires.
3. `crates/vivacity/src/discovery.rs` (nouveau) : le prédicat porté et le
   dépôt local qu'il lit (lock résolu ou installed.json) ; tests sur les
   particularités PHP. Dans `vivacity` plutôt que `vivacity-core` : il lui
   faut le parseur de contraintes du résolveur (`findPackage`).
4. `crates/vivacity/src/plugin_order.rs` : le parcours trié de
   `loadRepository` exposé (plugin présent ou non, `plugin-optional`,
   exigence `composer-plugin-api`, version).
5. `crates/vivacity-core/src/layout.rs` : les règles du Composer global
   (`config.json` + le `config` de `COMPOSER_HOME/composer.json`) ;
   `io.rs` : `held_lines` (lire les lignes retenues sans les vider).
6. `crates/vivacity/src/{install,dump,update,require,remove}.rs` : les
   appels ; `no_install.rs` : le prédicat au lieu de « surveille ».
7. Docs : CHANGELOG (`[Unreleased]` → `### Fixed`), DECISIONS, HANDOVER
   (le « Restent » corrigé, compteurs), README si la description des
   plugins change, ce plan.

## Cas de harnais

`harness/allow-plugins.sh` : projets en ligne, versions exactes, lock écrit
par Composer `--no-plugins`, vendor posé par chaque outil plugin autorisé,
puis `allow-plugins` modifié. Par cas : code ; pour un refus, ce que
Composer imprime avant l'encadré puis le texte de l'encadré (comme
`root-manifest.sh`) contre les lignes de vivacity, stdout identique, rien
d'écrit (`find -newer`, sauf composer.json pour `remove`) ; sinon sorties
et projet entiers ; pour un rendu, code 3 sous `--no-fallback`, la raison,
rien d'écrit. 57 cas :

- refus (rouges) : `none-{install,update,update-ni,update-dry,require,require-ni,remove-ni,dump,quiet}`,
  `gha-none-install`, `other-install`, `dir-gone-install`,
  `old-lock-{install,update-ni,empty-install,optional-dump}`,
  `transitive-{install,update-ni}`, `global-install`,
  `gone-then-blocked-install`, `lockfalse-blocked-install` ;
- avertissement (rouges) : `gone-{install,update,update-ni,dump,require,remove-ni}`,
  `false-gone-install`, `remove-blocked-itself` ;
- rendus (rouges) : `bare-install`, `bare-update`, `old-lock-bare-install`,
  `require-makes-required`, `disc-{dev,unknown,bare,nodev}-update`
  (chacun avec Composer seul qui réécrit composer.json), `disc-plain-update-ni`
  devenu natif ;
- garde-fous : `none-no-plugins`, `gone-quiet`, `false-required-install`,
  `discovery-optional-{install,update-ni}`, `package-versions-{install,dump}`,
  `old-lock-lockfalse`, `global-no-manifest`, `bare-{update-ni,install-dry,update-dry}`,
  `disc-unknown-update-ni`, `disc-update-dry`, `disc-libdev-update`,
  `disc-libdev-nodev-update`, `disc-plain-update`.

## Critère « vert »

Rouges verts, garde-fous verts ; `cargo fmt --all -- --check`, clippy
`-D warnings` (Linux et `x86_64-pc-windows-gnu`), `cargo test --workspace` ;
tous les harnais de `ci.yml` à 0 sauf ce que le réseau de la session
interdit (dit dans le Résultat, avec les relances restreintes), plus
`root-manifest.sh` et le nouveau.

## Non-objectifs (vus, laissés)

- Le mode interactif (la question `[y,n,d,?]` et l'écriture
  d'`allow-plugins`) : vivacity n'est jamais interactif.
- symfony/flex < 1.9.8 sauté avant le jugement (avertissement « not
  compatible with Composer 2+ ») : son avertissement n'est pas porté ; non
  autorisé, il est rendu à Composer (revue, point 1).
- L'issue réseau de l'update imbriqué de discovery : rendu à Composer.
- Root sans `COMPOSER_ALLOW_SUPERUSER` (« Do not run Composer as root »,
  plugins coupés) : non modélisé, préexistant ; seuls les refus y sont
  rendus.
- Lancer vivacity depuis `COMPOSER_HOME` même (`isGlobal`) ; le Composer
  global que `initialize` crée après un premier chargement refusé (ses
  propres avertissements) ; les erreurs d'arguments imprimées avant le
  chargement (`install --no-install`, `remove` sans paquet), sans ses
  avertissements.
- Un plugin autorisé dont l'API ne correspond pas (Composer l'ignore avec
  un avertissement) ; `scope::active_plugins`, qui lit le dépôt global sans
  les conditions du Composer global.
