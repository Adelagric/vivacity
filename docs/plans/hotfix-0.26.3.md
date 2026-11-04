# Correctif 0.26.3 : `bin` hors du paquet et enregistrement de classmap périmé

> Niveau 2 : sécurité (un paquet agit sur un fichier extérieur) et contrat de parité.
> Base : `main` `1147d4a` (0.26.2 publiée, tag `v0.26.2` = `d853100`). Référence de parité de
> cette branche : **Composer 2.10.3**, pas 2.11.x.

## Cadrage

### Problème

Deux défauts sont dans la version publiée :

1. **`bin` :** `install_bins` (`crates/vivacity-core/src/binproxy.rs`) calcule
   `package_dir.join(bin)`. `Path::join` jette le dossier du paquet devant un chemin absolu, donc
   `bin: ["/un/fichier"]` atteint ce fichier : `chmod` de `0600` à `0755` et proxy écrit vers lui.
   `.//fichier` fait pareil une fois `./` retiré. Un lien du paquet qui sort du paquet l'atteint
   par `chmod`. Composer 2.10.3 fait `ltrim($bin, '/')` (`ArrayLoader`), construit
   `$installPath.'/'.$bin` et saute un `bin` résolu hors du paquet
   (`BinaryInstaller::isBinPathInsidePackage`).
2. **Classmap :** un dossier du store retiré puis réextrait dans le même tic garde son inode et son
   ctime, donc la clé de son enregistrement. Un paquet placé depuis cette entrée reçoit alors les
   classes de l'ancien contenu : le chemin « placé » ne relit aucun octet et ne contrôle ni
   l'identité par fichier ni l'empreinte. Vu sur Linux. Mesuré sur macOS (voir Résultats) : sur tous
   les tags essayés de v0.1.0 à v0.26.0, une entrée réextraite 3 s plus tard sert déjà les anciennes
   classes, la clé n'ayant pas d'identité ; v0.26.1 et v0.26.2 ne les servent plus que sur l'emplacement
   forcé, seul le cas du même tic restant. Une collision forcée le reproduit à l'échelle unitaire.

### Contraintes

- Identité à l'octet avec **2.10.3** sur tout ce que la 0.26.2 exécutait déjà correctement.
- Aucun comportement de 2.11 (`isSafeBinPath`, lignes d'avertissement propres à 2.11, plancher du
  délégué, lecteur effectif) dans cette branche.
- Sources : `repin-2.11.1` traite `bin` avec beaucoup plus de code (`bin_entries`, `5489a22`, `52ddfc2`,
  `3be46db`), lié à l'ordre des lignes de 2.11. Le correctif `bin` n'en reprend rien : il est écrit ici,
  petit, et se compare au phar 2.10.3. La classmap reprend `1c283ee`, `ed68a60`, `2b8b562`, adaptés.
- Composer global et `~/.composer` restent intacts ; espace dédié `~/vivace-wt/c2103`
  (`COMPOSER_HOME`, `COMPOSER_CACHE_DIR`, `VIVACITY_CACHE_DIR`, `VIVACITY_HARNESS_DIR`,
  `CARGO_TARGET_DIR`).
- Rien n'est poussé, tagué ni publié sans accord sur les textes.

### Non-objectifs

Le passage à 2.11.1, le lecteur effectif, toute autre remise à Composer, le bump du store.

### Invariants et critères observables

| # | Invariant | Preuve | Scénario négatif | Vérification diversifiée |
|---|---|---|---|---|
| I1 | Aucun fichier hors du paquet n'est modifié et aucun proxy ne le vise, quelle que soit la forme de `bin` | test d'intégration : fichier témoin `0600` hors du paquet | rouge sur `v0.26.2` | phar 2.10.3 sur les mêmes formes |
| I2a | Une forme de `bin` lisible dans le lock et qui n'est pas ordinaire part à Composer avant toute écriture ; sous `--no-fallback`, sortie 3 sans rien écrire | table de formes, `install` et `update` : Composer atteint et résultat égal au phar 2.10.3 | absolue, `.//`, `a//b`, `.`, `./`, `a/.`, `..` et `a/..`, `\\`, vide, chaîne, objet, non-chaîne ; aussi dans `installed.json` | sources `x2.10.3` lues |
| I2b | Formes ordinaires : aucune sortie ni aucun octet ne change par rapport à 0.26.2 | suite, harnais, corpus | `bin/x`, `./bin/x`, doublon de nom | comparaison à `v0.26.2` et au phar |
| I2c | Forme connue seulement après extraction (lien sortant, dossier) : ni `chmod` ni proxy, la commande continue et écrit la ligne de saut de 2.10.3 | test sur un paquet, comparé au phar sur cette ligne | ancien comportement : erreur d'E/S ou `chmod` | **répétition après le dump reproduite ; ordre entre plusieurs dists zip : le phar varie, écart tenu par un test** |
| I3 | Une entrée réextraite ne sert jamais l'enregistrement de l'ancien contenu | test forçant le même emplacement, avec un fichier de même taille | rouge sur `v0.26.2` | passage Linux, seule plate-forme où la collision réelle a été vue |
| I4 | Les enregistrements écrits par 0.26.2 ne servent plus | changement de format, test forçant le même emplacement avec un enregistrement `v4` | retirer le changement : rouge | lecture de la clé (`dev`, `ino`, ctime) |
| I5 | Pas de perte sur les cas courants : parité et réutilisation du cache | suite complète, tous les harnais, corpus dev et no-dev | comparaison à `v0.26.2` : mêmes comptes natifs | `classmap-revalidate`, `vendor-isolation` |

## Plan

Commits petits, un par sujet, sur la branche `hotfix-0.26.3`.

1. **Témoins rouges sur 0.26.2.** Nouveau `crates/vivacity/tests/bin_forms.rs`, comparant au phar 2.10.3.
   Montrer le rouge sur le code de `1147d4a`, dont le témoin `600 → 755`.
2. **Correctif `bin`, trois morceaux.**
   - *Garde dans `install_binaries` :* `target` hors du paquet après résolution des liens, comme
     `isBinPathInsidePackage` : ni proxy ni `chmod`. Dossier : saut au lieu de l'erreur d'E/S. Cette garde
     protège quel que soit le chemin d'entrée.
   - *Lecture stricte avant écriture :* une forme non ordinaire ou un `bin` non-liste, dans le lock, les
     métadonnées d'un paquet retenu ou `installed.json`, part à Composer. Les lecteurs `bins()` et
     `previous_bins` restent sur la liste de chaînes. Tous les lecteurs à vérifier :
     `lock.rs`, `layout.rs:1037`, `layout.rs:1074`, `installer.rs:924`, `installer.rs:951`,
     `vendor_cleaner.rs:423`, `lockfile.rs` (`validate_package`).
   - *Lignes de saut :* copiées de la sortie du phar 2.10.3 pour le lien sortant et le dossier. Un `bin`
     manquant reste silencieux comme en 0.26.2 (écart déjà connu, consigné).
3. **Correctif classmap.** Écriture d'un enregistrement réservée à une entrée plus vieille que la
   fenêtre racy ; horloge sondée aussi sur le périphérique du store ; `CACHE_FORMAT` à `v5` et test
   de clé (`dev`, `ino`, les deux parties du ctime). `crates/vivacity-autoload/src/classmap.rs`,
   `generator.rs`.
4. **Report vers `repin-2.11.1`.** La branche n'est pas modifiée ici. À l'intégration, y passer le format
   de classmap à `v6` : les deux versions ne se partagent alors aucun enregistrement, sans avoir à
   prouver que leurs analyses coïncident.
5. **Portée de l'avis.** Mesurer avec des binaires de tags : versions touchées par `bin` et par la
   classmap, comportement du phar 2.10.3, ce qu'un paquet atteint (droits de l'utilisateur,
   `install` depuis un lock, `update`, paquet transitif, chemin du store). Rapport factuel, sans
   texte d'avis : le texte se rédige à part, après validation.
6. **Docs et version.** `Cargo.toml`, `Cargo.lock`, versions internes, `CHANGELOG.md`, `HANDOVER.md`,
   `DECISIONS.md`, README si un compteur bouge. Ce plan, avec ses résultats.

## Méta-analyse

Relecteur frais, code lu à `1147d4a`, phar 2.10.3 exécuté, un binaire 0.25.0 pour le témoin. Retenu :

- Reproduire toutes les lignes de saut demanderait l'ordre et la seconde passe `ensureBinariesPresence`,
  mesurés instables sur le phar 2.11.1 (`3be46db`). D'où la remise avant écriture et l'écart ouvert I2c.
- La lecture proposée élargissait `bins()` sans le dire. Écarté : un `bin` non-liste part à Composer.
- `..` manquait à la table. Ajouté.
- Coût de la fenêtre racy : une entrée fraîche n'est pas enregistrée, un second projet dans les 2 s la
  rescanne. Les harnais qui comptent les scans sur deux exécutions rapprochées sont à surveiller.
- Hors de cause d'après le code lu : `chmod` sur le store (aucun lien dur dans `clone.rs`), sauf une entrée
  liée à un ancien store v3 dont le détachement n'aurait pas abouti, non servie par le store v4.

Alternatives écartées : reprendre `5489a22` et ses suites (trop de code, comparé à 2.11.1) ; refuser au
lieu de remettre (contraire au contrat) ; attendre la 2.11.1 (les défauts sont publiés).

## Vérification

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`.
- Tous les harnais de `ci.yml`, contre le phar 2.10.3, sur un export figé du commit.
- Corpus dev et no-dev, à comparer à `v0.26.2` : mêmes comptes natifs ou écart expliqué un par un.
- Linux : uniquement si le disque de la VM est réparé et s'il reste au moins 30 Gio. Sinon, I3 reste
  **non démontré** sur Linux, et la CI Linux du push est la première preuve à lire.
- Windows : CI seulement. Le comportement du `basename` à deux séparateurs est lu et testé
  unitairement, non exécuté localement.
- Revue indépendante du diff par un agent frais qui n'a pas écrit le code.

## Résultats

Mesures du 2026-11-04, macOS (APFS), Composer 2.10.3, PHP 8.5.12. Ce qui suit est ce qu'ont donné
les commandes ; rien n'est publié.

### Tests

- `crates/vivacity/tests/bin_forms.rs`, comparés au phar. Version `b6d1138`, 60 tests, sur les sources de
  `1147d4a` : 50 rouges, dont le témoin (`left: "755"`, `right: "600"`) ; 10 verts, qui sont les témoins de
  non-régression (`ordinary_*`, `name_conflict_*`, `a_dev_package_is_not_read_under_no_dev`,
  `an_unselected_*`, `a_non_string_left_out_stops_only_the_phar`). Vus rouges par mutation : `./` retiré de
  l'ordinaire (`ordinary_dot_slash_form`), garde coupée (`link_out_form`, `755`).
- Version `4c81068`, 69 tests, sans le filtre `expected_native_stderr` : sur les sources de `b6d1138`, 10
  rouges sur 69, la passe après le dump. Vert ensuite. Mutation : sans le tri par dépôt local,
  `several_packages::path_packages_…` rouge.
- Version actuelle, 81 tests. `shared_proxy_name::*` : 6 sur 6 rouges sur les sources de `1147d4a`, verts
  après. `path_and_zip::*` : rouges sans le tri `path` d'abord. Les comparaisons se font sous `--no-progress` et
  **ne couvrent pas la barre de progression**, que vivacity n'écrit pas (écart connu du CHANGELOG).
- **Stabilité.** `gone_*` échouait sous charge sur `0/2` contre `0/1`. Cause : la barre du phar, pas vivacity.
  `Loop::wait` part des tâches actives à cet instant et redessine toutes les 0,1 s. Phar seul, même entrée,
  30 exécutions : 29 et 1 au repos ; 23 et 7 puis 25 et 5 sous 42 `yes` sur 14 cœurs. Instantané du projet et
  des caches (contenu, mode, mtime) pris quand la remise lance Composer : aucun changement depuis la
  réinitialisation, sur 170 exécutions sur 170. Avec `--no-progress` des deux côtés, 40 exécutions sous charge
  donnent une seule sortie : rien n'est normalisé. Passes complètes en `--release` sous la même charge : `b6d1138`
  échoue 5 fois sur 6 (2 à 4 tests, tous `gone_*`), `4c81068` passe 12 fois sur 12, 69 sur 69, charge jusqu'à 91,79.
- Classmap : `the_same_slot_on_a_re_extracted_entry_serves_no_stale_classes` (`["Old"]` au lieu de
  `["New"]`) et `a_record_of_the_previous_format_is_not_read` (`["Stale"]` au lieu de `["Real"]`), rouges
  sur `1147d4a`. Après : verts. Format remis à `v4` : le second rouge. Enregistrement toujours écrit : le
  premier rouge. Chaque partie de l'identité retirée de la clé : `every_part_of_the_identity…` rouge.

### Point commun de la lecture stricte

`binproxy::non_ordinary_bin`, lu par `fallback::bin_form_issue` (lock, installed.json, avant la première
ligne) et par `UpdateSession::strict_bin` (`transaction.all`, avant l'écriture du lock). Raisons et preuves
dans DECISIONS 2026-11-04. `validate_package` tourne sur `install`, mais son échec est une erreur : sur `..`,
`1147d4a` écrit `Error: acme/tool has an invalid bin ..` et sort en 1 sans atteindre Composer.

### Écarts restants avec le phar

Chacun a un test nommé (liste et raisons dans HANDOVER) : `bin` manquant silencieux ; ordre entre plusieurs
dists zip, sur lequel le phar varie ; alias de branche et alias en ligne, ligne doublée chez le phar après le dump ;
deux paquets posés par une même exécution pour un même nom de proxy, cible différente, déjà ainsi sur `v0.26.2` ; zip à
lien sortant, refusé ici ; `.bat` déjà là sous `COMPOSER_BIN_COMPAT=full`, déjà ainsi sur `v0.26.2` ; remise tardive
d'`update` ; candidat non retenu non-chaîne, `TypeError` chez le phar. Fermés : la passe après le dump d'un dossier ou
d'un lien sortant, placé ou sans opération ; la ligne sous l'opération d'un paquet `path` ; l'ordre d'un paquet `path`
avant les archives (phar stable sur 8 exécutions sur 8 : arrivées, montées de version ; zip a, b et path p : p d'abord,
a et b dans les deux ordres) ; un proxy partagé avec un paquet qui part (`vendor/bin` sans lui sur `v0.26.2` et `114ba0e`).

### `classmap-files/`

`FILE_CACHE_FORMAT` reste `r2`, mais la clé de ces fichiers commence par `CACHE_FORMAT`. Mesuré : projet posé puis
`dump-autoload` par v0.26.2 (`1 served`), son enregistrement réécrit avec une classe fausse, que v0.26.2 sert (`Bad`) ; le
correctif sur le même projet : `0 served, 0 from store, 0 read, 1 scanned`, classe `Old`, l'ancien fichier toujours
sur le disque, non lu, un second fichier écrit, puis `1 served`.

### Portée de `bin`

Binaires des tags construits à part, projet témoin, dépôt `composer` local, dist zip.

| Version | Effet sur un fichier extérieur |
|---|---|
| v0.1.0 | aucun proxy, mode inchangé |
| v0.2.0, v0.3.0, v0.4.0, v0.5.0, v0.6.0, v0.9.0 | proxy vers le fichier, mode inchangé |
| v0.10.0, v0.13.0, v0.17.0, v0.21.0, v0.25.0, v0.26.0, v0.26.1, v0.26.2 | proxy, et mode 600 → 755 |

Non mesurés : v0.7.0, v0.8.0, v0.11.0, v0.11.1, v0.12.0, v0.14.0 à v0.16.0, v0.18.0 à v0.20.0, v0.22.0 à v0.24.0.
`update` n'existe pas avant v0.4.0, les dépôts `path` pas avant v0.9.0. Le `chmod` vient de `efc67ed`, dont
v0.10.0 est le premier tag.

| Cas, sur v0.26.2 | v0.26.2 | phar 2.10.3 |
|---|---|---|
| `install`, lock, absolu | 755, proxy | 600, `file not found` |
| `install`, lock, `.//<absolu>` | 755, proxy | 600, `file not found` |
| `update`, métadonnée, absolu | 600, pas de proxy (`dump_package` retire le `/`) | 600, `file not found` |
| `update`, métadonnée, `.//<absolu>` | 755, proxy | 600, `file not found` |
| `install`, lock, paquet transitif, `.//<absolu>` | 755, proxy | 600, `file not found` |
| `update`, paquet transitif, `.//<absolu>` | 755, proxy | 600, `file not found` |
| `install`, `update`, dépôt `path`, lien sortant | 755, proxy | 600, `outside of the package` |
| `update`, composer.json d'un paquet `path`, `.//<absolu>` | 755, proxy | 600, `file not found` |
| `install`, source git, lien sortant | 755, proxy | 600, `outside of the package` |
| `install`, dist zip, lien sortant | sortie 1, `hostile archive`, 600 | 600, `outside of the package` |
| `install`, dist tar, lien sortant | 600, proxy | 600, proxy |
| `install`, `.//etc/hosts` (à root) | sortie 1, `Operation not permitted`, mode inchangé, proxy `vendor/bin/hosts` laissé | `file not found` |
| `install`, `.//<fichier du store>` | 644 → 755, proxy | sans objet |

La limite est celle des droits de l'utilisateur : `chmod` échoue sur un fichier d'un autre. Le `bin` ordinaire
d'un paquet ne touche pas sa copie du store sur APFS : `bin/x` à 755 dans `vendor/`, 644 dans l'entrée, relevé sur
v0.26.2, v0.26.1, v0.26.0 (`store/v3`), v0.25.0, v0.21.0, v0.13.0, v0.10.0. **Non mesuré** : les liens durs des systèmes
sans copie sur écriture jusqu'à 0.26.0, où ce `chmod` passerait par le lien d'après le code (la VM Linux est arrêtée).

### Portée de la classmap

Sur le même binaire, deux projets `install --optimize-autoloader`.

| Version | Emplacement forcé (fichier de l'entrée réécrit en place, même taille, identité de la racine inchangée) | Entrée retirée, réextraite 3 s plus tard d'une autre archive |
|---|---|---|
| v0.1.0 à v0.6.0, v0.13.0, v0.17.0, v0.21.0, v0.25.0, v0.26.0 | `Old` (fichier de B : `New`) | `Old` |
| v0.26.1, v0.26.2 | `Old` | `New` |
| ce correctif | `New` | `New` |

La collision réelle dans un même tic n'a pas été reproduite : les ctime d'APFS sont à la nanoseconde et l'inode
n'a pas été réutilisé dans cette mesure. **Non mesuré sur Linux.**

### Non vérifié ici

Harnais et corpus (lancés par le mainteneur), Linux, Windows, sonde de l'horloge sur un second périphérique.
