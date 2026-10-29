# chamilo-lms (dev) et dealerdirect v0.7.2 — plan

> Suite de `docs/plans/remaining-fallbacks.md`, section B. Niveau 2 (contrat
> de parité : vendor/, stdout, stderr, code de sortie à l'octet contre
> Composer 2.10.3). Méta-analyse et revue par sous-agents frais.

## Cadrage

**Problème.** Au corpus, `chamilo-lms` (dev) part à Composer : le lock
verrouille `dealerdirect/phpcodesniffer-composer-installer` **v0.7.2**, que
vivacity n'émule pas (1.0.0, 1.1.1, 1.1.2, 1.2.0, 1.2.1 seulement). Le port
avait été jugé faisable mais pas fait, parce que **la référence échoue sur la
fixture** : phpcs charge `vendor/autoload.php`, dont l'autoload `files` de la
racine (`public/legacy.php`) `require` treize fichiers absents de la fixture
(puis la classe `Database` d'un répertoire `classmap`, absente aussi) ;
Composer imprime « Failed to set PHP CodeSniffer installed_paths Config » et
`corpus.sh` classe l'entrée `unavailable`. Porter seul ferait passer chamilo
(dev) de `fallback` à `unavailable`. Choix de l'utilisateur : **les deux**.

**Mesuré avant le plan** (fixture copiée, Composer 2.10.3, PHP 8.5.12) :

- les treize fichiers de `legacy.php` + `public/main/inc/lib/database.lib.php`
  (quatorze fichiers, 902 Ko, tirés de `chamilo/chamilo-lms` au commit de
  PROVENANCE `cbf4567`) suffisent : `phpcs --config-show` passe, l'install de
  référence écrit `CodeSniffer.conf` et imprime « PHP CodeSniffer Config
  installed_paths set to ../../mpdf,../../slevomat/coding-standard » ;
- `../../mpdf` est la recherche dans tout le projet propre à 0.7.2 :
  `vendor/mpdf/mpdf/ruleset.xml` (une bibliothèque, pas un standard) est à
  la profondeur 3 depuis la racine (la borne incluse) ;
- rien n'est écrit hors de vendor/ par le code chargé (`git status`).

**Contraintes.** Référence 2.10.3 (le composer global, inchangé). Aucun code
GPL dans les sources ni dans les fichiers commités de vivacity (chamilo est
GPL) : les fichiers sont des *données* de fixture, téléchargées à un commit
épinglé au moment du corpus. Source de 0.7.2 (MIT) lue au tag, citée dans
NOTICE, copie de référence sous `docs/reference/plugins/`. Tout cas non
reproduit part à Composer, nommé, avant toute écriture.

**Non-objectifs.** Les autres 0.x (0.7.0, 0.7.1, 0.6…) : non lus, rendus.
L'exécution de phpcs par vivacity (l'émulation écrit `CodeSniffer.conf`
elle-même, comme pour 1.x). Linux et Windows (non mesurés ici, déclarés).

**Critères de succès observables.**

1. `harness/dealerdirect-07.sh` vert sous PHP 8.5 et 8.4, et vu rouge contre
   le binaire de main (5768dc9) puis sur un cas cassé exprès.
2. `harness/corpus.sh --only chamilo-lms` : natif, 0 diff, dans les deux
   modes ; rejeu des autres entrées à dealerdirect sans régression.
3. Comptes recalculés, 0 unavailable : attendu 95/106 `--no-dev`, 95/106 dev.
4. `cargo fmt --check`, `clippy --all-targets -D warnings`, `cargo test`.

## Lecture de v0.7.2 (`1c968e5`, `src/Plugin.php`, diff contre v1.2.1)

Mêmes écouteurs (`POST_INSTALL_CMD`, `POST_UPDATE_CMD`, priorité 0), même
ligne au niveau normal (« … set to … », « … delete », « Failed to set … »),
même tri, même dédoublonnage. Ce qui change :

| point | v0.7.2 | v1.2.1 (émulé) | conséquence |
|---|---|---|---|
| chemins de recherche | **toujours** `cwd`, puis les paquets `phpcodesniffer-standard` (alias exclus) | `cwd` seulement si la racine est de type standard | `cwd` toujours cherché ; tout `ruleset.xml` à profondeur ≤ max sous la racine du projet, vendor/ compris |
| racine de type standard | jamais ajoutée (`! instanceof RootPackageInterface`, toujours faux) | ajoutée | sans effet : `cwd` y est déjà |
| `cleanInstalledPaths` | garde `is_dir($path)` (relatif au cwd) **ou** `realpath(phpcs/$path)` | phpcs seul | = v1.0–1.2.0 (`keeps_cwd_relative_paths`) |
| `--config-show` | `phpcs` du `PATH` (bin-dir préfixé par `ensureBinDirIsInPath`), lancé dans le bin-dir | `php ./bin/phpcs` dans phpcs | même fichier lu (`CodeSniffer.conf`), sauf si `bin-dir/phpcs` n'est pas celui du paquet |
| `--config-set` | `./bin/phpcs`, ou `./scripts/phpcs` (phpcs 2) | `./bin/phpcs` | phpcs < 3 : non mesuré → rendu |
| `-v` | « Running PHPCodeSniffer Composer Installer », « Nothing to install or update », sortie de phpcs | autres libellés | `-v` déjà rendu pour dealerdirect (`VERBOSE_DELEGATED`) |
| `getMaxDepth` | appelé dès que phpcs est installé ; lève sur une valeur invalide | appelé seulement s'il y a des chemins | voir la garde ci-dessous |

**Le Finder de Composer 2.10.3** (symfony/finder v5.4.45, lu dans le phar) :
`ignoreDotFiles` est actif par défaut (motif `#(^|/)\..+(/|$)#` sur le chemin
relatif : tout composant qui commence par un point est écarté), les
répertoires liés symboliquement ne sont **pas** parcourus
(`hasChildren(false)`), `files()` n'écarte que ce qui est un répertoire (un
`ruleset.xml` lien pendant passe), les noms VCS sont exclus comme
répertoires. `find_rulesets` aujourd'hui : ne saute pas les composants à
point, suit les liens de répertoire, exige `is_file`. Sans effet visible pour
1.x tant que les standards n'ont ni dossier à point ni lien ; décisif pour
0.7.2, qui parcourt la racine (`.github/`, `.ddev/`, `node_modules/.bin`…).

## Plan

### 1. La fixture chamilo, complétée au moment du corpus (pas de code GPL commité)

- `fixtures/corpus/chamilo-lms/UPSTREAM` : une ligne par fichier,
  `<sha256>  <chemin>` (format `shasum -a 256`), les quatorze fichiers.
  L'origine et le commit sont ceux de PROVENANCE (`git <url> <sha> …`).
- `harness/corpus.sh` : `stage <nom> <dest> [fetch]` ; avec `fetch`, chaque
  fichier d'UPSTREAM est copié depuis un cache
  (`$WORK/upstream/<owner>/<repo>/<sha>/<chemin>`), téléchargé sinon depuis
  `https://raw.githubusercontent.com/<owner>/<repo>/<sha>/<chemin>`, et
  vérifié par sa somme ; **avant** le commit git de la fixture (les deux
  côtés identiques). Le scan (`--scan`) reste hors ligne : pas de `fetch`.
  Échec du téléchargement ou somme fausse → `unavailable`, détail
  « fixture upstream files unavailable: <chemin> ».
- `tools/corpus-upstream.sh <entrée> <chemin>…` : écrit UPSTREAM depuis le
  commit de PROVENANCE (téléchargement + somme) ; `tools/corpus-add.sh`
  avertit quand il écrase une entrée qui avait un UPSTREAM (le commit
  change : la liste est à refaire).
- `fixtures/corpus/README.md` : la règle (fichiers de projet sous licence
  du projet, non commités, tirés au commit épinglé, sommes commitées) ;
  noter que `public/legacy.php` (GPL, 700 octets) était déjà commité par
  l'ajout initial, conformément à la règle « `files` copiés tels quels ».

Coût dans le dépôt : UPSTREAM (~1,4 Ko) + l'outil. Données tirées au corpus :
903 Ko (quinze fichiers, 903 027 octets — `legacy.php` compris), une fois par cache.

### 2. Le port de v0.7.2

- `vivacity-core/src/phpcs_installer.rs` :
  - `Variant` (ou deux booléens) : `search_project` (0.7.2 : vrai ;
    1.x : vrai si la racine est de type standard — l'existant) et
    `cwd_relative` (l'existant) ; `fn variant(plugin_version)` ;
  - `find_rulesets` fidèle au Finder : composants à point écartés, liens de
    répertoire non suivis, entrée acceptée si `ruleset.xml` n'est pas un
    répertoire ; s'applique à 1.x aussi (même Finder ; correction de parité
    latente, notée au CHANGELOG) ;
  - tests unitaires : racine, imbriqué, trop profond, dossier à point, lien
    de répertoire, `search_project`.
- `scope.rs` : `EMULATED_VERSIONS` + `0.7.2.0` (raison des autres :
  « other 0.x versions were not read, v1.1.0 asks InstalledVersions where
  phpcs is ») ; gardes nommées, avant toute écriture, quand dealerdirect
  sera chargé (autorisé, installé dans le mode) :
  - `extra.phpcodesniffer-search-depth` présent et pas un entier JSON
    strictement supérieur à la profondeur minimale (0 pour phpcs ≥ 3,
    1 avant) → rendu (le plugin lève, ou une chaîne castée que vivacity ne
    lit pas) — toutes versions ;
  - 0.7.2 avec `squizlabs/php_codesniffer` < 3.0 → rendu (`./scripts/phpcs`,
    non mesuré) ;
  - test unitaire `listener_plugins_outside_the_versions_read…` : v0.7.2
    lu, v0.7.1 rendu.
- `install.rs` / `no_install.rs` : `emulate_phpcs` passe la variante.
- `docs/reference/plugins/phpcodesniffer-composer-installer-v0.7.2/`
  (`Plugin.php`, `LICENSE.md`) ; NOTICE.

### 3. Le harnais `harness/dealerdirect-07.sh`

Sur le modèle de `plugin-lines.sh` (locks résolus une fois par Composer
`--no-plugins`, `step` : code, sortie fusionnée, projet entier ; `expect`
sur la sortie de Composer pour qu'un cas qui ne montre plus ce qu'il teste
échoue). Paquets : v0.7.2, phpcs 3.13.6 (celui de chamilo), standards
phpcsextra 1.5.1 / phpcsutils 1.2.3 en dev, deux paquets `path`
synthétiques copiés (`symlink: false`) : `acme/lib` (bibliothèque, un
`ruleset.xml` à sa racine — le cas mpdf) et `acme/deep`
(`a/b/ruleset.xml`, profondeur 5 : hors portée).

Cas :

1. install vierge (bibliothèque dans vendor + standards) ; 2. no-op ;
3. `ruleset.xml` à la racine (`../../../`) ; 4. imbriqué
(`config/phpcs/Std/ruleset.xml`, profondeur 3) ; 5. trop profond
(profondeur 4, rien) ; 6. dossier à point (`.github/Std/ruleset.xml`,
rien) ; 7. lien de répertoire vers un standard hors projet (rien) ;
8. ruleset retiré (le chemin reste : le répertoire existe) ; 9. config
perdue (réécrite) ; 10. `--no-dev` (standards dev retirés) ; 11. retour en
dev ; 12. retrait d'`acme/lib` par un lock qui ne l'a plus (le chemin
nettoyé) ; 13. `update` (POST_UPDATE_CMD) ; 14. `update --no-install` ;
15. `--no-plugins` ; 16. plugin retiré du lock (conf laissée telle
quelle) ; 17. `extra.phpcodesniffer-search-depth: 4` (le cas 5 trouvé) ;
18. phpcs 4.0.4 (format de `--config-show`) ; 19. aucun standard nulle
part (rien, ou `delete` après retrait). Rendus (code 3, la raison, rien
d'écrit) : 20. `-v` ; 21. v0.7.1 ; 22. search-depth `"4"` (chaîne) ;
23. phpcs 2.9.2.

`plugin-lines.sh` : le cas « versions non lues » passe à v0.7.1. `ci.yml` :
une étape.

### 4. Vérification et documents

- Harnais rouge contre le binaire de main, puis sur un cas cassé (le filtre
  des dossiers à point retiré) ; vert sous 8.5 et 8.4.
- Corpus : chamilo deux modes, puis les entrées à dealerdirect
  (akeneo, cakephp-app, laminas-mvc-skeleton, mezzio-skeleton, leantime,
  matomo, phpdocumentor, phpmyadmin, shaarli, snipe-it, roave, drupal) ;
  corpus complet si le temps le permet ; rapport
  `docs/corpus/2026-10-29-chamilo-dealerdirect.md`.
- README, HANDOVER, DECISIONS (2026-10-29), CHANGELOG (Unreleased), NOTICE,
  `remaining-fallbacks.md` (B), ce plan (méta, revue, résultat).

## Risques connus avant la méta

- **phpcs exécute le PHP du projet.** Composer échoue (« Failed to set… »,
  rien d'écrit) quand l'autoload du projet ou d'un paquet plante sous
  phpcs ; vivacity écrit la conf. Préexistant pour 1.x, inchangé ici ;
  0.7.2 n'élargit pas ce risque (même chargement). Noté.
- **`phpcs` du PATH.** Si `bin-dir/phpcs` n'est pas le binaire du paquet
  (conflit de nom de binaire), 0.7.2 lit la conf d'un autre phpcs. Non
  gardé ; noté.

## Méta-analyse (sous-agent frais, plan et code)

Douze failles ; suites :

| faille | gravité | suite |
|---|---|---|
| Des fichiers vides (`<?php`) suffiraient : la sortie de Composer ne dépend du contenu de `files` que par la classmap ; ni réseau, ni GPL | majeure | **écartée, raison consignée** : la consigne est de compléter la fixture avec les vrais fichiers au commit épinglé. Une fixture à stubs mesurerait un projet qui n'existe pas : le corpus vaut parce que la référence y exécute ce que chamilo exécute (ici, le vrai `Database` chargé sous phpcs, deux classes de plus dans la classmap). Le réseau est déjà requis par le corpus (Packagist) |
| `public/legacy.php` (GPL) est déjà commité ; le plan l'entérine | moyenne | `legacy.php` passe lui aussi dans UPSTREAM et sort de l'arbre commité : la fixture chamilo ne commite plus que des manifestes. Le scan hors ligne n'en a pas besoin (vérifié : `--check-scope` sans le fichier, même verdict) |
| Cas 12 : `acme/lib` et `acme/deep` partagent `vendor/acme`, le chemin `../../acme` resterait | moyenne | deux vendors (`lib-vendor/lib`, `deep-vendor/deep`) |
| Le filtre des dossiers à point doit porter sur le chemin relatif à chaque racine de recherche ; aucun cas ne distingue relatif et absolu | moyenne | cas : projet sous un répertoire à point ; `vendor-dir: ".vendor"` (la bibliothèque à ruleset n'est plus trouvée par la racine, les standards le sont par leur propre recherche) ; test unitaire |
| Paquets `path` en lien (le défaut) exclus : racine `in()` lue, lien enfant non suivi, jamais testé | moyenne | cas `path` en lien : standard trouvé par sa propre racine (`../../<vendor>`), la source sous `packages/` trouvée par la racine du projet |
| « Rouge contre main » ne prouve rien cas par cas (tout part en code 3) | moyenne | une mutation par comportement : liens suivis, `search_project` faux, `is_file`, filtre sur le chemin absolu |
| La correction du Finder pour 1.x n'a aucun cas 1.x | mineure | cas 1.2.1, racine de type standard avec `.github/Std/ruleset.xml` et un lien |
| Changement de version 0.7.2 ↔ 1.2.1 (`update`, `update --no-install` : l'ancien plugin d'installed.json) | mineure | deux cas, un `ruleset.xml` à la racine pour que la variante se voie |
| Faits : mpdf est à la profondeur 3 (la borne incluse) ; la racine donne `../../../` (slash final de `findShortestPath(…, true)`) | mineure | corrigés ci-dessus et dans les tests |
| phpcs en version de branche ou aliasé : `isPHPCodeSnifferInstalled('>= 3.0.0')` accepte l'alias, vivacity lit la version seule | mineure | rendu quand la version de phpcs n'est pas une version stable analysable (toutes versions du plugin) |
| Téléchargement : cache écrit après vérification (temporaire + `rename`), revérifié à chaque usage ; le commit dans UPSTREAM ; cache sous `$WORK/cache` ; PHP 8.4 au critère 2 | mineure | retenu tel quel |
| `--working-dir` non canonique (`-d /tmp/p` ; Composer : `getcwd()` après `chdir`, `/private/tmp/p`) : avec un vendor hors du projet, `findShortestPath` diverge | mineure, préexistante | rendu quand le répertoire du projet n'est pas son chemin réel et que vendor-dir est hors du projet (toutes versions) |

### Le plan, révisé

- Fixture : UPSTREAM porte le commit (`# <owner/repo>@<sha>`) et quinze
  fichiers (legacy.php compris) ; `stage … fetch` vérifie chaque fichier,
  cache `$WORK/cache/upstream/…` (temporaire, somme, `rename`).
- Port : `search_project` (0.7.2), Finder fidèle (point relatif, liens
  enfants non suivis, « pas un répertoire ») ; gardes : search-depth,
  phpcs < 3 pour 0.7.2, phpcs non analysable, projet non canonique + vendor
  hors du projet.
- Harnais : cas ajoutés ci-dessus, mutations une à une.

La garde « projet non canonique + vendor hors du projet » est **retirée** :
`Dirs::resolve` refuse déjà un `vendor-dir` absolu ou en `..`
(`crates/vivacity-core/src/dirs.rs`), vendor est donc toujours sous le
projet, et tous les chemins comparés partagent le même préfixe — lexical ou
réel, `findShortestPath` donne le même résultat (constat de la revue,
confirmé à la lecture).

## Revue indépendante (sous-agent frais, diff non commité contre 5768dc9)

| point | gravité | suite |
|---|---|---|
| 0.7.2 lit la conf par le `phpcs` du `PATH` ; `ensureBinDirIsInPath` ne préfixe pas un bin-dir déjà présent : un autre `phpcs` placé avant répond (mesuré : Composer écrit les chemins de cet autre phpcs, « Failed to set … ») ; sans `php` dans le `PATH`, le proxy `env php` échoue | moyenne | `phpcs_installer::path_issue` : rendu dans ces deux cas (0.7.2 seulement) ; test unitaire et cas `dd-path-foreign` (Composer y imprime « Failed to set … », vivacity rend) ; commentaire du module corrigé |
| La garde phpcs 2 ne visait que 0.7.2 ; v1.0.0, v1.1.1, v1.1.2 ont la même branche `./scripts/phpcs` et acceptent `^2.0` (lu à leurs tags) | moyenne | garde étendue à toute version < 1.2.0 ; cas `dd-v112-phpcs2` |
| La garde n'était pas évaluée sur le lock résolu d'un `update` (seulement installed.json d'avant et `analyze`, après l'écriture du lock) | moyenne/basse | `scope::phpcs_lock_issue` ajouté aux vérifications du lock résolu (`update.rs`, à côté d'`effect_plugins::issues`) ; le même trou pour `emulated_version_issue` existe avant ce chantier (non traité) |
| Garde « projet non canonique » promise, absente | basse | retirée du plan, raison ci-dessus |
| Gardes conservatrices (phpcs stable ≥ 3 aliasé, `"4"` en chaîne, profondeur invalide sans chemin en 1.x, `isRunningGlobally`) | basse | gardées telles quelles (un repli n'est jamais faux) ; `isRunningGlobally` : préexistant, noté au HANDOVER |
| `corpus.sh` : tout échec de `stage` devenait `unavailable` (un bug du harnais masqué) | basse/moyenne | `stage` rend 1 (fichier UPSTREAM manquant) ou 2 (copie) ; 2 arrête le harnais |
| `corpus-add.sh` : un fichier disparu au nouveau commit laissait l'entrée sans UPSTREAM | basse | l'ancienne liste est remise, avertissement |
| Harnais : stdout et stderr fusionnés seulement ; instantané de `handed` sur les noms ; Windows (`ln -s`) ; pas dans `ci.yml` | basse | `ci.yml` : une étape ; le reste noté (même méthode que `plugin-lines.sh`) |
| Docs en retard (HANDOVER, DECISIONS « non porté », CHANGELOG sans la correction du Finder, 912 Ko au lieu de 903 Ko) | basse | corrigés |
| Chemins d'UPSTREAM non assainis | très basse | un chemin absolu ou à `..` est refusé |

## Résultat (session de travail, macOS)

- Commits : `faceed9` (la fixture : UPSTREAM, `corpus.sh`, outils),
  `11de6a6` (le port, le Finder, les gardes, le harnais), puis les
  documents et le rapport.
- `harness/dealerdirect-07.sh` : 35 cas (31 comparés à Composer, 4
  rendus), verts sous PHP 8.5.12 et 8.4.27. Rouge contre main 5768dc9, sur
  les 33 cas d’alors (31 FAIL, 2 OK : `-v` et v0.7.1, déjà rendus). Mutations, une par
  règle : liens de répertoire suivis 13 FAIL, `search_project` faux 23,
  `is_file` 3, filtre des points sur le chemin absolu 2 (`dd-dot-parent`,
  `dd-dot-vendor-dir`), sans filtre des points 14, gardes de la revue
  retirées 2 (`dd-path-foreign`, `dd-v112-phpcs2`).
- Corpus : UPSTREAM vu rouge (une somme faussée : `unavailable`, le chemin
  nommé). chamilo-lms native dans les deux modes sous PHP 8.5 et 8.4
  (« set to ../../mpdf,../../slevomat/coding-standard » des deux côtés) ;
  les douze autres entrées à dealerdirect inchangées (onze natives dans
  les deux modes, drupal rendue pour ses propres plugins). Comptes :
  **95/106 `--no-dev`, 95/106 dev, 0 diff, 0 unavailable**
  (`docs/corpus/2026-10-29-chamilo-dealerdirect.md` ; les 93 autres
  entrées reprises du passage précédent, non rejouées).
- Harnais existants rejoués : `plugin-lines.sh` 118 OK (binaire final),
  `plugins-round3.sh` 52, `allow-plugins.sh` 58, `verbosity.sh` 310 (binaire
  d'avant la revue, dont les ajouts ne sont que des gardes 0.7.2 et
  phpcs 2).
- `cargo fmt --check`, `clippy --all-targets -D warnings`, `cargo test`
  (455 tests, 0 échec), `fixtures/make.sh`.
- Non vérifié : Linux et Windows ; stdout et stderr séparément (le harnais
  compare la sortie fusionnée, comme `plugin-lines.sh`).
