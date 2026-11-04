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
   l'identité par fichier ni l'empreinte. Vu sur Linux, depuis 0.26.1 d'après le code ; une
   collision forcée le reproduit à l'échelle unitaire.

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
| I2c | Forme connue seulement après extraction (lien sortant, dossier) : ni `chmod` ni proxy, la commande continue et écrit la ligne de saut de 2.10.3 | test sur un paquet, comparé au phar sur cette ligne | ancien comportement : erreur d'E/S ou `chmod` | **ordre et répétition après le dump non reproduits, écart ouvert** |
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
