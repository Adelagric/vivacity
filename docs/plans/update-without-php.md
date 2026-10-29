# `update`, `require`, `remove` sans PHP sous `--ignore-platform-reqs` — plan

> Niveau 2 : contrat de parité à l'octet (lock, composer.json, vendor/,
> stdout, stderr, code de sortie) avec Composer 2.10.3. Méta-analyse et revue
> par sous-agents frais, sur le code.

## Le problème

`update`, `require` et `remove` (et `install` sans lock, qui est un update)
lancent `VIVACITY_PHP` (défaut `php`) pour sonder la plateforme
(`UpdateSession::prepare_update`, `probe()` inconditionnel), même sous
`--ignore-platform-reqs`. Sans PHP : `Error: cannot run …`, code 1. Mesuré
le 2026-10-29 sur `{"require":{"psr/log":"^3"}}` :
`VIVACITY_PHP=/nonexistent vivacity update --ignore-platform-reqs` → 1.
`install` depuis un lock tourne déjà sans PHP (`verify_install`,
`probe().ok()`). ePHPm comble le trou de son côté (ephpm#558) ; un
programme qui embarque vivacity sans PHP à lui ne peut pas mettre à jour.

## Pourquoi il y a une référence

Sous `--ignore-platform-reqs`, `IgnoreAllPlatformRequirementFilter::isIgnored`
= `PlatformRepository::isPlatformPackage` (composer\*, php\*, hhvm, ext-\*,
lib-\*). Le contenu du dépôt plateforme ne pèse alors sur la résolution
qu'aux points suivants, relevés dans les sources 2.10.3 (`docs/reference/`) :

| Lecteur | Où | Sous IgnoreAll | Traitement |
|---|---|---|---|
| exigences des paquets | `RuleSetGenerator.php:196` | sautées | inerte |
| conflits | `RuleSetGenerator.php:224` | sautés | inerte |
| exigences racine (requête) | `RuleSetGenerator.php:287`, `Solver.php:162` | sautées | inerte |
| exigences racine du pool | `Installer.php:940`, `RepositorySet.php:126` | retirées | inerte |
| chargement par nom | `PoolBuilder.php:371`, `:443` | jamais chargé depuis un dépôt | inerte |
| paquets plateforme fixés | `Installer.php:1022-1036`, `PoolBuilder.php:247-268`, `PoolOptimizer.php:101` | fixés, sans exigences | inerte **sauf** même nom |
| **règle « même nom »** | `RuleSetGenerator.php:176-179`, `:243-248` (`getNames(false)` = nom + `replace`) | **non filtrée** | un paquet du pool qui `replace` un nom plateforme que le vrai PHP fournirait entre en conflit avec lui → **rendu à Composer, nommé** |
| contraintes plateforme du sélecteur | `VersionSelector.php:115` | sautées (`!IgnoreAll`) | inerte |
| version recommandée `ext-*` | `VersionSelector.php:208-214` (`PHP_VERSION`) | lue | `require <plateforme>` sans contrainte → **rendu, nommé** |
| nom et version d'un paquet plateforme à `require` | `PackageDiscoveryTrait.php:307-313` (dépôt plateforme du `RequireCommand.php:196-200`) | lu (nom canonique, `*` ou `^x.y` sous `--no-update`) | idem : **rendu, nommé** |
| `getPlatformExceptionDetails` | `PackageDiscoveryTrait.php:333`, `:439` | gardé par `!IgnoreAll` | inerte |
| explications d'un ensemble insoluble | `Problem.php:258`, `Rule.php:155,178` ; indices d'extension (fichiers ini, extensions chargées) | seulement sur des règles visant un nom plateforme, qui n'existent pas sous IgnoreAll | inerte |
| **compte des suggestions** | `Installer.php:344-352` → `SuggestedPackagesReporter.php:180-210` : une suggestion dont la cible est fournie par le dépôt plateforme (`getNames()`) n'est pas comptée | **lu** | une suggestion comptée vers un nom plateforme inconnaissable sans PHP → **rendu à Composer, nommé** |
| lock : `platform`, `platform-dev`, `platform-overrides` | `Installer.php:581-582`, `:677` | composer.json et `config.platform` | inerte (pas de sonde) |
| `platform_check.php` | `AutoloadGenerator.php:456` | désactivé sous IgnoreAll | inerte |
| filtres de politique | `FilterListPoolFilter.php:212`, `SecurityAdvisoryPoolFilter.php:68` | paquets plateforme exclus | inerte |
| ordre des opérations | `Transaction.php:303,328` | par nom | inerte |
| dépréciations de composer/installers 1.x | `ErrorHandler` sous PHP ≥ 8.4 (`plugin_allow.rs`) | dépend du PHP qui exécute Composer | décision ci-dessous |

« Inconnaissable sans PHP » : un nom plateforme qui n'est ni `composer`,
`composer-plugin-api`, `composer-runtime-api`, ni une clé de
`config.platform` (une clé à `false` est absente des deux côtés, une clé
versionnée présente des deux côtés). Tout le reste (php\*, ext-\*, lib-\*,
hhvm) dépend des extensions chargées.

Les identifiants de paquet du pool se décalent sans les paquets plateforme,
mais ceux-ci sont chargés en tête et en bloc (`PoolBuilder.php:247`) :
l'ordre relatif des autres est inchangé, et un paquet plateforme fixé sans
exigence ni conflit actif n'est qu'une assertion isolée du solveur. C'est
l'argument ; la preuve est le harnais.

## Décisions

1. **Déclencheur** : seulement quand le filtre est IgnoreAll —
   `--ignore-platform-reqs` ou `COMPOSER_IGNORE_PLATFORM_REQS` (même filtre
   chez Composer, `BaseCommand::getPlatformRequirementFilter`) — **et** que
   le PHP ne peut pas être lancé (l'exécutable n'existe pas ou n'est pas
   exécutable : l'erreur de lancement de `run_probe_once`). Un PHP qui se
   lance et échoue reste une erreur. Un PHP présent : rien ne change.
2. **Sans IgnoreAll** (rien, une liste `--ignore-platform-req`, ou
   `config.platform.php` seul, qui ne couvre pas les extensions) : code 1,
   message qui garde l'erreur de lancement et nomme `--ignore-platform-reqs`
   et `VIVACITY_PHP`.
3. **Plateforme sans PHP** : `platform_packages(&[], config.platform)` — les
   trois `composer*` et les surcharges, celle qu'`install` utilise déjà ;
   `php_version` vide ; pas de fichiers ini, pas d'extensions chargées.
4. **Même nom** : sur le pool filtré (sur-ensemble du pool optimisé), avant
   « Updating dependencies », un paquet (racine comprise) qui `replace` un
   nom inconnaissable → `SessionErrorKind::Unsupported` → rendu à Composer
   avec la raison (code 3 sous `--no-fallback`), rien d'écrit.
5. **`require` sans contrainte d'un nom plateforme** autre que les trois
   `composer*` (même surchargé : `findRecommendedRequireVersion` compare
   la version de l'extension à `PHP_VERSION`) → rendu avant toute édition.
6. **Suggestions** : après la résolution, avant les lignes du lock et
   l'écriture, une suggestion que vivacity compterait et dont la cible est
   inconnaissable → rendu, nommé. Même calcul que `post_update_report`
   (factorisé), donc pas de seconde implémentation.
7. **Dépréciations de composer/installers 1.x** : sans PHP, rien n'est
   compilé, rien n'est imprimé (le comportement d'`install` sans PHP depuis
   toujours, `php_minor` → `None`). Écart documenté et testé : la sortie de
   vivacity = celle de Composer sous 8.4/8.5 moins exactement ces deux lignes.

Alternatives écartées : faire comme un PHP sans extension (pas de
référence : la ligne de suggestions et la règle même nom changeraient de
valeur sans que Composer puisse le confirmer) ; ignorer silencieusement la
plateforme hors IgnoreAll (change la résolution) ; deviner `*` pour
`require ext-foo` (faux si l'extension est chargée).

## Fichiers (tels que livrés)

1. `crates/vivacity-resolver/src/platform.rs` : `php_command`,
   `php_launchable` (l'exécutable trouvé, sinon un lancement essayé et
   classé par `io::ErrorKind`), `known_without_php` ; tests.
2. `crates/vivacity-resolver/src/session.rs` : `UpdateOptions::php_absent`,
   `UpdateSession::php_absent`, pas de sonde dans ce mode ; contrainte
   temporaire inconnue, `replaced_platform_name` (règle même nom), indice
   d'extensions (`problem::extension_problem`) → `Unsupported`.
3. `crates/vivacity/src/no_php.rs` : `decide` (au début de chaque commande),
   `decide_early` (dans `run_once`, avant ses délégations), le drapeau du
   run, `unguessable_requirement` ; `fallback.rs` : `which_composer` muet
   dans ce mode, la ligne d'arrêt qui nomme `VIVACITY_PHP`.
4. `crates/vivacity/src/update.rs` : `Unsupported` de `session.update` →
   arrêt nommé ; `counted_suggestions` (extrait de `post_update_report`) et
   le contrôle avant les lignes du lock ; `require.rs`, `remove.rs`,
   `install.rs` : la décision au début (`remove --no-update` exclu).
5. `crates/vivacity/tests/no_php.rs` : binaire sans PHP, dépôt `path`, hors
   réseau (7 tests, rouges sur main).
6. `harness/no-php.sh` + `.github/workflows/ci.yml`.
7. README (Embedding), HANDOVER, DECISIONS, CHANGELOG.

## Harnais `harness/no-php.sh`

Registre `file://` synthétique (p2 + zips locaux), COMPOSER_HOME isolé,
cache par côté et par cas (lignes Downloading symétriques). Composer avec
PHP, vivacity avec `VIVACITY_PHP=/nonexistent VIVACITY_NO_PLATFORM_CACHE=1`,
`--ignore-platform-reqs` des deux côtés. Comparés : code, stdout, stderr
entière, composer.json, composer.lock, vendor/ (`compare_vendor`). Cas :
update (sans lock), update (lock présent), partiel, `-W`, `--with`,
require d'un paquet normal (avec et sans contrainte), remove,
`--no-install`, `--dry-run`, install sans lock, exigences php/ext/lib
insatisfaites + conflit sur une extension chargée (le cas doit échouer chez
Composer SANS le drapeau : oracle non aveugle), `config.platform`,
`COMPOSER_IGNORE_PLATFORM_REQS=1`, suggestion d'un paquet ordinaire.
Refus : `require ext-foo` sans contrainte, un `replace` d'extension, une
suggestion d'extension (code 3 sous `--no-fallback`, raison nommée, rien
d'écrit) ; sans drapeau, liste partielle, `config.platform.php` seul (code
1, message). Dépréciation de composer/installers 1.12.0 (Packagist en
ligne). Vu rouge sur main ; joué sous PHP 8.4 et 8.5 côté Composer.

## Critère vert

Gates (build release, clippy -D warnings, fmt, test) ; harnais 0 FAIL sous
8.4 et 8.5 ; rouge vu sur main (code 1 partout où vivacity doit répondre).

## Méta-analyse (sous-agent frais, sur le code) — et ce qui change

Dix failles relevées ; dispositions :

1. **-v** : `Analyzed N packages` / `Analyzed N rules` (`Installer.php:560-561`,
   `session.rs:1148-1155`) comptent les paquets plateforme fixés et leurs
   règles → sans PHP sous -v, **arrêt nommé** avant toute sortie. Cas au
   harnais.
2. **`remove` édite composer.json avant la sonde** (`remove.rs:210`) : la
   décision PHP/sans PHP est prise **au début** de `update`, `require`,
   `remove` et d'`install` sans lock, avant toute édition ou sortie, et
   transmise à la session (`UpdateOptions::php_absent`) : plus de seconde
   sonde pour décider.
3. **Rendre à Composer ne marche pas sans PHP** (`hand_over` lance
   `composer`, qui a besoin de php ; le harnais garde php dans le PATH) :
   en mode sans PHP, toute délégation (`delegate_resolution`,
   `fallback_or_fail`, `delegate_for_verbosity`) **s'arrête en code 3**
   avec ses raisons et une ligne qui nomme `VIVACITY_PHP`, comme sous
   `--no-fallback`. Remplace « rendu à Composer » aux décisions 4–6.
   Cas au harnais avec un faux `composer` dans le PATH, qui ne doit pas
   être lancé.
4. **Contraintes temporaires** (`--with ext-json:^99`, `update 'ext-*:^1'`) :
   `PoolBuilder.php:292-318` les applique aussi aux paquets plateforme fixés
   (`LogicException` « Fixed package … was not added » avec PHP) → une
   contrainte temporaire (après expansion) sur un nom inconnaissable :
   arrêt nommé, dans `prepare_update`, avant toute sortie.
5. **Dépréciations de composer/installers** : gardé (décision 7), cadré :
   sans PHP, vivacity se comporte comme Composer sous PHP 8.1–8.3 (≥ 8.1 :
   le classmap lit les enums, `cmg-PhpFileParser.php:129` ; < 8.4 : pas de
   dépréciation). C'est déjà le comportement d'`install` sans PHP. Écrit
   dans DECISIONS ; l'alternative (refus) y est notée.
6. **Clé `false`** : `ext-uuid` remplace `lib-uuid`, les bibliothèques
   fournissent/remplacent des noms `lib-*` : une surcharge à `false` ne
   retire que le paquet de ce nom → « connu » = les trois `composer*` et
   les clés **versionnées** de `config.platform` seulement.
7. **`require` refuse trop tard** : le balayage des noms sans contrainte se
   fait avant la création de composer.json, avant la session et avant toute
   ligne.
8. **Classement de l'erreur de lancement par chaîne** : remplacé par
   `locate_php` (l'exécutable existe et est exécutable) au moment de la
   décision ; un PHP trouvé qui échoue reste une erreur (code 1), un échec
   de lancement transitoire ne bascule pas en mode sans PHP.
9. **`--ignore-platform-req=*`** : reste code 1 (le périmètre est le
   filtre IgnoreAll, `instanceof` chez Composer : `VersionSelector.php:115`,
   `AutoloadGenerator.php:456` distinguent les deux). Noté dans DECISIONS.
10. **Préfixe `ext-` sur des paquets ordinaires** (`ext-foo/bar`) :
    `findRecommendedRequireVersion` et `getExtensionProblems` testent
    `strpos(…, 'ext-')` → `require` sans contrainte d'un nom qui commence
    par `ext-` : arrêt nommé ; un ensemble insoluble dont une règle exige un
    nom en `ext-` (l'indice liste les fichiers ini) : arrêt nommé.

Confirmé par la méta : règle même nom non filtrée (la vérifier sur tout le
pool filtré est plus large que nécessaire, assumé), filtre des suggestions
par le dépôt plateforme, indices de `Problem.php:258` gardés, décalage des
identifiants sans effet (`DefaultPolicy.php:191-195`, plateforme en bloc
après la racine).

## Revue indépendante (sous-agent frais, sur le diff) — et ce qui change

1. **`remove --no-update` sans PHP** : la décision bloquait une commande qui
   ne résout rien (régression) → décision seulement sans `--no-update` ;
   cas de parité au harnais.
2. **Dépréciations de composer/installers** : choix assumé (décision 7),
   README précisé (« PHP inférieur à 8.4 », non mesuré sous 8.1–8.3 : le
   harnais compare à 8.4 et 8.5 moins ces deux lignes).
3. **Délégations de `run_once` avant la décision** (`-vv`, `-v` avec
   plugins, `cache-dir`, version racine hors git) : elles lançaient le
   `composer` du PATH → `no_php::decide_early` dans `run_once`, avant elles ;
   cas `-vv` au harnais.
4. **`locate_php` ≠ le lancement réel** (`.exe` ajouté à un chemin Windows
   explicite, répertoires système, chemin par défaut d'`execvp`) →
   `php_launchable` essaie le lancement quand la recherche échoue et ne
   conclut à l'absence que sur `NotFound`/`PermissionDenied` ; test unitaire
   (introuvable, non exécutable, `/usr/bin/false`).
5. **Refus `-v` sous `--no-fallback`** et sur `require --no-update` (qui
   n'imprime pas les comptes) → `no_fallback` transmis, refus seulement si
   la commande résout.
6. Branche morte de `delegate_for_verbosity` : vivante depuis 3.
7. Angles morts du harnais → cas ajoutés : ni `VIVACITY_PHP` ni php dans le
   PATH, `remove --no-update`, suggestion rendue connue par
   `config.platform`, `-vv`, `-v --no-fallback`.
8. Docs : les deux arrêts manquants (indice d'extensions d'un insoluble,
   suggestion de la racine à la première installation) ajoutés ; plan remis
   au code livré.
9. `known_without_php` : la DERNIÈRE clé de même nom (casse ignorée) décide,
   comme `PlatformRepository` ; `require php` avec `config.platform.php`
   reste refusé (sur-refus assumé, sans perte).

Écart trouvé par le harnais, hors chantier puis corrigé à part : les
avertissements de `PoolBuilder` (« Package … listed for update is not
locked », « Pattern … matches platform packages », « Dependency … is also
a root requirement ») sont écrits par Composer pendant `buildPool`, avant
« Updating dependencies » (`PoolBuilder.php:216`, `:539`, `:569` ;
`Installer.php:533-536`), vivacity les écrivait après — avec PHP aussi.
Corrigé dans un commit à part : `Pool::filter_warnings` sépare les
avertissements des filtres de politique (dont la place ne change pas) de
ceux du constructeur, écrits avant la notice de Flex et « Updating
dependencies » ; test d'intégration et trois cas de harnais, rouges avant.
