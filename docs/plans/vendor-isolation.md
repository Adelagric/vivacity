# Isolation de vendor/ et revalidation de la classmap — plan

> Niveau 2 : contrat de sortie publique (vendor/ identique à Composer
> 2.10.3) et isolation entre projets. Le premier passage a couvert cadrage,
> mesure, plan et méta-analyse ; **P0 est implémenté depuis** (section
> « Exécution de P0 »), P1 ne l'est pas. Les mesures viennent
> d'un spike activé par variables d'environnement, retiré de l'arbre après
> la mesure et jamais commité (annexe A). Ses chiffres exploratoires ne sont
> pas publiés ; les performances sous Linux restent à mesurer sur le correctif
> définitif.

## Le problème (reproduit le 2026-10-31)

Point de départ : README, « Known issue: a file edited in place in
vendor/ » (68fd3b1). Deux défauts reproduits par le mainteneur, quatre
autres trouvés en préparant ce plan.

**P0 — isolation.** `clone.rs` pose vendor/ depuis le store par
`clonefile` (APFS), sinon `FICLONE` (btrfs, xfs), sinon **lien dur**
(ext4, overlayfs, NTFS sur le même volume, macOS hors APFS), sinon copie.
Une écriture en place dans `vendor/` du projet A modifie l'entrée du
store, le projet B, et tout projet C installé ensuite (reproduit sur une
image HFS+, `hdiutil create -fs HFS+`). Composer ne partage jamais un
fichier entre deux projets.

Liens durs vus ici dans les conteneurs Linux de la mesure : `nlink` = 2
sur un fichier de vendor/, sur un volume ext4 comme sur la racine
overlayfs.

Ce qui écrit « en place » (même inode, donc à travers le lien) : la
redirection du shell (`>`), `file_put_contents` de PHP, `dd
conv=notrunc`, nano, vim sur un fichier à plusieurs
liens (`backupcopy=auto` choisit alors la réécriture en place). Ce qui
n'écrit pas en place : `sed -i` (BSD et GNU), `perl -i`, GNU `patch`,
l'« écriture sûre » de JetBrains — ils renomment un nouveau fichier, vérifié
pour `sed -i` de macOS (inode changé). Un cas de harnais doit donc écrire
en place explicitement, pas avec `sed -i`.

Les écrivains internes de vivacity évitent déjà le piège :
`composer_patcher::detach` (copie + rename avant `patch`) et
`extension_installers::write_if_changed` (« the rector fixture's file
surfaced in sylius's warm run » : le défaut a déjà mordu, de l'intérieur).
Les écritures de l'utilisateur et de ses outils ne sont pas couvertes.

**P1 — cache de classmap non revalidé.** `CacheSlot`
(`crates/vivacity-autoload/src/classmap.rs`) : un enregistrement par
(entrée de store, sous-répertoire), servi **sans lister le répertoire ni
lire un `stat`** (sauf `verify_files`, posé pour liborm85). Renommer
`class Str` en `class Stx` dans `vendor/laravel/framework/src/Illuminate/Support/Str.php`
puis `vivacity dump-autoload -o` garde `Illuminate\Support\Str`, que
Composer retire — même taille + mtime restauré (`touch -r`) comme taille
et mtime changés. `VIVACITY_NO_CLASSMAP_CACHE=1` donne le vendor/ de
Composer.

Trouvés en préparant ce plan, reproduits sur APFS avec la fixture
symfony (deux copies du projet, A et B, même cache de vivacity) :

- **P1-b — un fichier ajouté ou supprimé dans vendor/** : une classe
  ajoutée (`vendor/symfony/string/AddedByHand.php`) est absente de la
  classmap, une classe dont on a supprimé le fichier (`ByteString.php`)
  y reste ; cache coupé : l'inverse, comme Composer. Même cause : un
  enregistrement servi ne relit pas le répertoire.
- **P1-c — un enregistrement de store écrit depuis une copie éditée
  contamine les autres projets, même sur APFS.** A installé sans `-o`
  (seuls les `classmap` des paquets sont enregistrés), `class
  UnicodeString` renommé dans `vendor/symfony/string/UnicodeString.php`
  de A, puis `dump-autoload -o` dans A : le répertoire `src` de
  symfony/string n'a pas d'enregistrement, A le scanne et
  `CacheSlot::store` l'enregistre **pour l'entrée de store** —
  `matches_entry` ne compare que la liste des fichiers, pas leur contenu.
  B, dont vendor/ est intact (clonefile, inodes distincts) : `dump-autoload -o`
  avec le cache → `Symfony\Component\String\UnicodeString` absent de la
  classmap de B ; cache coupé → présent. La copie-sur-écriture ne protège
  pas de ce chemin-là : P0 réglé, P1-c reste.
- **P1-d — sources du projet, même taille + mtime restauré.**
  `FileCacheSlot` (clé (mtime ns, taille)) : `class Comment` →
  `class Commenx` dans `src/Entity/Comment.php` de symfony, `touch -r`
  vers une copie d'avant : la classmap garde `App\Entity\Comment` ; cache
  coupé : retirée. C'est le « risque résiduel documenté » de
  `FileCacheSlot` ; le contrat choisi (« même comportement que Composer
  sur l'état courant du projet ») le fait passer de risque à défaut.
- **P1-e (déjà connu, HANDOVER)** : un vendor/ nettoyé par liborm85 dont
  le plugin a disparu sert encore les classes des fichiers supprimés. Cas
  particulier de P1-b.

## Cadrage

**Contrat retenu par le mainteneur.** Même comportement que Composer sur
l'état **courant** du projet, vendor/ compris ; isolation entre projets non
négociable. La documentation n'est pas un correctif ; mtime + taille n'est
pas une preuve d'identité.

**Contraintes.** Respecter le contrat sans perdre la vitesse : chaque
option chiffrée avant/après. Pas d'abandon des liens durs sans banc. Une
nouvelle dépendance : épinglée `=X.Y.Z`, une entrée DECISIONS.

**Non-objectifs.** Protéger le store contre une édition directe de
`<cache>/store/` (répertoire privé de vivacity) ; deux installs
simultanés sur le même vendor/ (Composer ne les protège pas non plus) ;
mettre les enregistrements dans le cache de l'action (M8, tranché
ailleurs) ; le clonage de blocs ReFS sous Windows (suite possible).

**Critères de succès observables.**

1. Une écriture en place dans vendor/ d'un projet ne change ni le store
   (vérifié sur ses octets), ni un autre projet, ni un projet installé
   ensuite — sur ext4, overlayfs, HFS+ et NTFS comme sur APFS (cas rouges
   `iso-*`). Couper le cache de classmap n'y est pour rien : P0 et P1 se
   corrigent et se vérifient séparément.
2. `dump-autoload` (`-o` ou non) et l'install produisent la classmap de
   Composer après toute édition de vendor/ ou des sources : renommage,
   ajout, suppression, même taille + mtime restauré, édition puis retour
   au contenu d'origine (cas rouges `cm-*`).
3. Le coût reste dans la fourchette mesurée ci-dessous, sur macOS et
   Linux, sur les trois fixtures.

## Ce que dit le code (avant mesure)

- `clone.rs` : ordre clonefile → (Linux) FICLONE → `hard_link` →
  `std::fs::copy`. Le test `clones_files_dirs_symlinks_and_exec_bits`
  écrit dans le clone avec `std::fs::write` (troncature en place, même
  inode) et n'affirme l'isolation **que sous macOS** : rendu
  inconditionnel, il serait rouge sur ext4 aujourd'hui.
- `store.rs` : une entrée est réputée immuable ; `LAYOUT` = `v3` ; aucune
  vérification de contenu à la réutilisation (`is_dir()`).
- `classmap.rs` : `CacheSlot` (clé `sha1(CACHE_FORMAT, chemin de
  l'entrée, sous-répertoire)`), écrit après un scan de la **copie sous
  vendor/** si `matches_entry` (même liste de fichiers) ; `FileCacheSlot`
  (clé = chemin canonique, une ligne (mtime ns, taille, classes) par
  fichier, répertoire listé à chaque scan).
- `generator.rs` : un répertoire sous l'installation d'un paquet dont
  l'entrée de store existe prend `CacheSlot`, tout le reste
  `FileCacheSlot` ; `mutable_copies` (drupol, composer-patcher) passe par
  `FileCacheSlot`. L'entrée est cherchée par `dist_reference()`,
  l'installeur l'écrit par `store_reference()` (écart connu, M8).
- `install.rs`, `plan_autoload_early` : sur un install sans opération, le
  dump est **planifié pendant l'attente réseau** (listes de filtres
  `malware`, vérification du lock). Le coût d'un scan y est masqué tant
  qu'il reste sous cette attente — c'est ce que montrent les no-op
  ci-dessous.
- Précédent d'identité forte : la sonde de plateforme
  (`platform-probe.json`, DECISIONS 2026-10-09) clé chaque image sur
  (mtime, taille, ctime, inode), et documente son angle mort.

## Mesures

**Statut (décision du mainteneur, 2026-10-31).** La mesure de cadrage a
été faite sur un spike jetable (annexe A), pas sur le code livré. Ses
chiffres ne sont pas publiés : rien n'entre dans ce plan qui ne soit
reproductible sur le vrai code. Restent le protocole, à rejouer sur le vrai
code, et les hypothèses que le spike a servi à formuler, à confirmer ou à
infirmer par ces mesures. Les résultats se lisent par système de fichiers :
une mesure sous Linux (ext4, overlayfs) ne dit rien du coût sous APFS, ni
l'inverse.

### Matériel, méthode

- **macOS** : M4 Max, 14 cœurs, APFS, macOS 26.6.2 ; rustc 1.99.0,
  `cargo build --release` ; hyperfine 1.20.0, `-N`, 2 de chauffe, 10
  runs ; cache de vivacity à part (`VIVACITY_CACHE_DIR`), zips de
  Composer chauds. Machine partagée (charge 2-4 pendant les runs).
- **Linux** : conteneurs Docker sur ce Mac (VM 4 vCPU, 6 Go, noyau
  6.8.0-100-generic), image existante `vivacity-linux` (php 8.4, rustc
  1.99.1), binaire construit **dans le conteneur** (`cargo build
  --release -p vivacity`, même arbre que macOS) ; hyperfine 1.15.0
  (Debian), mêmes options. Deux systèmes de fichiers : un volume nommé
  (ext4, `/dev/vdb1`) avec store et projets dessus, et la racine
  overlayfs du conteneur avec store et projets dessus. Zips de Composer
  copiés depuis le cache du Mac. Une VM sur Apple Silicon n'est pas un
  runner : l'ordre de grandeur vaut, les absolus non.
- **Date** : 2026-10-31. Arbre : main 68fd3b1 + spike (annexe A).
- Bras du spike : `today` (code de main), `off`
  (`VIVACITY_NO_CLASSMAP_CACHE=1`), `stat` (enregistrement de store servi,
  plus le parcours du répertoire et un `lstat` par fichier — le coût
  système d'une revalidation par métadonnées), `files` (tous les
  répertoires, vendor/ compris, par `FileCacheSlot` : parcours, `stat`,
  lecture et comparaison de l'enregistrement par projet — le plus proche
  du plan retenu), `xxh3` / `sha1` (enregistrement servi, plus parcours,
  lecture et condensat de chaque fichier, en parallèle sous Linux comme
  le scan). Pour P0 : `link` (main) et `copy` (`std::fs::copy` à la place
  de FICLONE + lien dur).
Valeurs à publier : médiane en ms (σ, min–max) ; Δ par rapport au premier
bras de la ligne.

### P0 — lien dur contre copie (Linux)

Protocole :
- install, vendor/ supprimé, store chaud (`--prepare 'rm -rf vendor'`),
  laravel, symfony et sylius, sur le volume ext4 puis sur la racine
  overlayfs ;
- install à store **froid** (vendor/, store et enregistrements supprimés,
  zips chauds — le cas d'un runner CI, M7), ext4, 1 chauffe, 8 runs,
  laravel et sylius ;
- install sans opération (no-op), vendor/ posé par liens puis par copies ;
- référence : `composer install -q --no-scripts`, sylius, vendor/
  supprimé, zips chauds, ext4, 5 runs.

Hypothèses du cadrage, à confirmer : le coût de la copie suit le volume de
données recopié, pas le nombre de paquets ; il pèse plus à store froid
(pages fraîchement extraites encore sales dans le cache de la VM) qu'à
store chaud ; le no-op ne change pas (rien n'est cloné) ; vivacity reste
nettement plus rapide que Composer sur le même cas. Sous macOS APFS,
clonefile reste premier et le chemin ne change pas ; sous HFS+ et sous
Windows, rien n'a été mesuré.

### P1 — revalidation (macOS, APFS)

Protocole : `dump-autoload -o` à cache chaud, chaque bras, sur les trois
fixtures ; la phase `dump scan` sous `VIVACITY_TRACE=1` ; install no-op
(laravel avec `optimize-autoloader`, symfony et sylius sans) ; install
vendor/ supprimé, store chaud, laravel (le dump `-o` suit le clone).

Hypothèses du cadrage, à confirmer : l'écart entre bras est dans le scan,
le reste du dump ne bouge pas ; sur APFS (lectures séquentielles,
`parallel_io` faux), lire pour condenser coûte presque autant que
rescanner, et le condensat ne paierait pas sur Mac ; `stat`/`files`
restent sous l'attente réseau du no-op (le dump est planifié pendant la
vérification du lock), `xxh3` et `off` la dépassent. Sur l'install vendor/
supprimé, `files` sert des hits parce que clonefile garde le mtime et que
la clé n'est que (mtime, taille) ; avec ctime et inode dans la clé, un
clone frais manquerait tout et coûterait autant que le cache coupé — d'où
l'amorçage du plan.

### P1 — revalidation (Linux, ext4)

Protocole : le même que sous macOS, plus l'install vendor/ supprimé de
laravel **en mode copie** (P0 + P1 ensemble) contre main.

Hypothèses du cadrage, à confirmer : sous Linux (lectures parallèles, page
cache), le condensat ne coûte guère plus que les métadonnées ; le no-op
reste dans le bruit.

Le `Cargo.lock` du spike (résolu `--offline` avec xxhash-rust) avait
déplacé des versions de `wasm-bindgen`/`js-sys`/`web-sys`, non compilés
pour ces cibles ; sans effet sur les binaires mesurés. Le bras `today`
porte la lecture de deux variables d'environnement du spike par
répertoire scanné (µs).

## Options

### P0

| option | isole ? | coût attendu (cadrage, non publié) | verdict |
|---|---|---|---|
| **A. Copie à la place du lien dur** (clonefile / FICLONE d'abord, inchangés) | oui, par construction | suit le volume recopié, plus lourd à store froid ; no-op inchangé | **retenue** |
| B. Lien dur + store en lecture seule (`chmod a-w`, `chattr +i`) | non | — | rejetée : les modes de vendor/ font partie de la parité (inventaire des modes du harnais), root ignore les bits (conteneurs CI), `chattr` exige CAP_LINUX_IMMUTABLE, vim `:w!` fait un chmod du même inode |
| C. Lien dur + vérification du store avant chaque clone (condensat) | non : B est modifié à l'instant où A écrit | lit tout le store à chaque install, plus cher que la copie | rejetée |
| D. Lien dur + détection après coup (`nlink > 1` et contenu changé) | non : détecte, ne protège pas | — | rejetée comme correctif ; utile au plus comme diagnostic |
| E. Copie seulement des fichiers « susceptibles d'être édités » | non | — | rejetée : aucun critère ne tient (un `.php`, un `.json`, un binaire de `vendor/bin` s'éditent), le contrat couvre tout fichier |
| F. Plus de store sur FS sans CoW : extraire chaque projet depuis le zip (comme Composer) | oui | > copie (décompression en plus) ; le store resterait rentable même en copiant | rejetée |
| G. Clonage de blocs sous Windows (ReFS / Dev Drive, `FSCTL_DUPLICATE_EXTENTS_TO_FILE`, ou `CopyFile2` qui le fait seul sur Windows 11 24H2) | oui | non mesuré | suite possible, hors de ce chantier |

À mesurer pendant l'exécution, seulement si les chiffres le demandent :
copie parallèle par fichier dans les gros paquets (le fan-out est
aujourd'hui par paquet) ; à store froid, renommer l'arbre extrait dans
vendor/ et recopier vers le store ensuite (même volume écrit, mais hors du
chemin critique du dump).

### P1

| option | preuve | coût `dump -o` macOS (cadrage, non publié) | coût `dump -o` Linux (cadrage, non publié) | verdict |
|---|---|---|---|---|
| a. Cache coupé par défaut | totale | le plus élevé, install vendor/ supprimé compris | le plus élevé | rejetée : le coût le plus haut partout, sans gain de preuve sur b |
| **b. Identité forte par fichier et par projet** (dev, inode, taille, mtime ns, ctime ns), répertoires relistés, amorcée depuis l'enregistrement de store pour ce que l'install vient de poser | tout ce qu'un processus non privilégié peut faire sans changer l'horloge | net, surtout sur sylius ; no-op masqué ; install vendor/ supprimé plus lent | faible ; no-op masqué | **retenue** (fourchette = bras `stat`…`files`) |
| c. Condensat du contenu (xxh3-128) par fichier contre l'enregistrement | totale (au condensat près) | proche d'un rescan | faible | alternative stricte ; trop chère sur macOS, quasi gratuite sous Linux — question ouverte Q1 |
| d. mtime + taille | non (`touch -r`) | — | — | rejetée par le mainteneur |
| e. Statu quo + documentation | non | 0 | 0 | rejetée par le mainteneur |

Pourquoi ctime : sous Unix, aucun appel non privilégié ne fixe le ctime
(`utimensat` le met à l'heure courante), toute écriture, `touch -r`,
`chmod` et changement de nombre de liens le fait avancer —
vérifié sur APFS : `touch -r` restaure le mtime, le ctime avance ; un
clonefile garde le mtime et prend un ctime neuf. L'inode attrape le
remplacement par rename (`sed -i`, `perl -i`, éditeurs « sûrs »). Angles
morts restants, à écrire : changer l'horloge du système, des outils root
(`debugfs`), un FS qui ne tient pas le ctime (certains FUSE, montages
réseau mal configurés), la fenêtre « racy » (traitée ci-dessous).

Windows : la std stable n'expose ni ctime ni identifiant de fichier.
`FILE_ID_EXTD_DIR_INFO` / `FILE_ID_BOTH_DIR_INFO`
(`GetFileInformationByHandleEx` sur le **répertoire**) rendent
`ChangeTime` et `FileId` de chaque entrée pendant le parcours, sans ouvrir
chaque fichier — dépendance `windows-sys` directe (déjà transitive en
0.61.2). Mais `ChangeTime` se règle par `SetFileInformationByHandle`
(`FileBasicInfo`) : plus faible qu'un ctime Unix. Q2.

## Plan recommandé

### Étape 0 — cas rouges d'abord (aucun changement de code produit)

Tous les harnais tournent avec un `VIVACITY_HARNESS_DIR` à part et
Composer 2.10.3 comme oracle sur le même état de projet.

#### Harnais permanent de P0 : `harness/vendor-isolation.sh`

Protocole final du mainteneur (remplace les deux notes précédentes). Le
harnais reste dans le dépôt comme garde de régression, lancé par la CI,
indépendant des bancs de performance.

**Critère.** A accepte l'écriture en place, B garde les octets
d'origine, le store garde les octets d'origine — **dans les deux
ordres** :

- `iso-before` : A et B installés, puis écriture en place externe dans
  un fichier de vendor/ de A ;
- `iso-after` : A installé, écriture en place dans A, **puis** B
  installé depuis le même store — attrape un install neuf qui hériterait
  d'une entrée contaminée, ce que le premier ordre ne voit pas.

« Octets d'origine » : condensat (sha256) égal à celui du fichier dans le
dist, relevé sur une extraction à part de l'archive (pas dans le store,
qui est ce qu'on teste). L'écriture est faite par le shell, hors de
vivacity, réellement en place (`printf … 1<> fichier` ou `dd
conv=notrunc`), même taille ou non.

**Garde 1 — l'écriture a vraiment eu lieu dans A.** Après l'écriture, le
fichier de A porte les nouveaux octets (condensat égal à celui du
contenu écrit), son inode n'a pas changé (preuve que l'écriture était en
place), et son mode est celui que Composer pose (comparé au même fichier
d'une install de Composer : vendor/ reste inscriptible). Un correctif
qui bloquerait ou détournerait l'écriture (store en lecture seule,
fichier non inscriptible, renommage) fait échouer le cas : ce n'est pas
une isolation, c'est une divergence.

**Garde 2 — A et B partagent vraiment un store, sur un FS où vivacity
lierait.**
- Même `VIVACITY_CACHE_DIR` pour A et B ; l'entrée de store du paquet
  existe avant l'install de B ; l'install de B la lit : la ligne de
  résumé (`N from store, 0 from cache, 0 from network`, sans `-q`) le
  dit, ou `VIVACITY_TRACE`.
- Le FS est un FS sans copie sur écriture : image HFS+ montée sous macOS
  (`hdiutil create -fs HFS+ -size 64m`, store, cache de Composer et
  projets dessus, détachée à la fin), ext4 sous Linux (les runners
  `ubuntu-latest` de GitHub le sont : le cas tourne en CI là). Sonde au
  démarrage, permanente (store et projets sur le même FS) : un `ln`
  entre le store et un projet réussit, et un clone de sonde échoue —
  `cp --reflink=always` sous Linux ; sous macOS `cp -c` (que `cp` refuse
  bien hors APFS, à vérifier au premier run, sinon un petit appel
  `clonefile` via `python3 -c` ou `perl`) — sinon le
  harnais **échoue** (« ce FS isole déjà, le cas ne prouverait rien »)
  au lieu de passer.
- Avant le correctif (phase rouge : `VIVACITY_BIN` = binaire de main et
  `ISO_PHASE=red`) : le harnais affirme
  qu'au départ le fichier de A et celui du store partagent un inode
  (`stat` : même `dev:ino`, `nlink` ≥ 2) — un montage accidentellement
  isolé (APFS clonefile, copie) est détecté au lieu de passer. Cette
  affirmation est le témoin de la phase rouge : avec `VIVACITY_BIN` =
  binaire de main, `iso-before` et `iso-after` doivent échouer **sur le
  critère** (octets de B / du store changés), garde 2 tenue ; avec le
  correctif, ils passent, garde 2 tenue par la sonde de FS et la ligne
  « from store », et le harnais affirme alors `nlink` = 1.

**Cas.** Un paquet minuscule servi hors ligne par un dépôt `artifact`
(un zip fabriqué par le harnais, comme `harness/artifact-repo.sh`) : pas
de réseau, quelques secondes par cas. Chaque ordre est joué deux fois,
cache de classmap actif puis `VIVACITY_NO_CLASSMAP_CACHE=1` :
couper le cache ne change rien à la contamination par lien dur, les deux
bras doivent être rouges avant P0.

**CI.** Un pas dans le job `parity` de `.github/workflows/ci.yml`
(matrice `ubuntu-latest`, `macos-latest`) : `harness/vendor-isolation.sh`,
sans `net-retry.sh` puisqu'il ne touche pas le réseau. Sous ubuntu, le
répertoire du runner (ext4) ; sous macOS, l'image HFS+ (création et
montage : quelques secondes, à confirmer au premier run ; si `hdiutil`
n'est pas disponible ou trop lent sur le runner, le cas macOS reste
local et l'ubuntu porte la garde).

#### Autres cas (P0 complémentaires, puis P1)

- `iso-upgrade` (méta-analyse, point 1) : A et B installés par le binaire
  de main (liens durs), passage au nouveau binaire, install **no-op** dans
  A et B, puis l'écriture en place dans A : mêmes vérifications (store et
  fichier de B par condensat avant/après).
- `iso-cow` : les deux ordres sur APFS (et btrfs/xfs si un runner en
  offre) — vert aujourd'hui, il garde le chemin clonefile/reflink. Mode
  à part du harnais (`ISO_FS=cow`), où la sonde exige au contraire la
  copie sur écriture ; jamais confondu avec le cas obligatoire.
- `iso-stale-store` : une entrée `v3` modifiée à la main (simule un store
  contaminé par une version antérieure) ; après le changement, un install
  pose le contenu de l'archive (via le bump de `LAYOUT`).
- `cm-same-size-mtime` : `Str` → `Stx` (laravel), même taille, `touch -r`
  — `dump-autoload -o`, puis install no-op `-o`.
- `cm-edit` : même renommage, taille et mtime changés.
- `cm-added`, `cm-removed` : un fichier de classe ajouté dans un
  répertoire PSR-4 de vendor/, un fichier supprimé (sans liborm85).
- `cm-cross-project` (APFS) : le scénario P1-c (A édité avant son premier
  `-o`, B intact) : B identique à Composer.
- `cm-root-same-size` : P1-d dans `harness/root-scan.sh` (sources du
  projet).
- `cm-revert` : édition puis retour au contenu d'origine (ctime changé,
  contenu d'origine) : identique à Composer — garde contre un état
  « édité » collant.
- Tests unitaires : `clone.rs` — l'assertion d'isolation devient
  inconditionnelle, avec une écriture en place explicite (rouge sur
  ext4) ; `classmap.rs` — identité qui change avec le seul ctime, avec le
  seul inode ; entrée « racy » non servie (horloges injectées).

Chaque cas est vu **rouge** sur main avant toute correction (les quatre
`cm-*` reproduits ici le sont déjà à la main sur APFS ; l'ordre
`iso-before` l'est chez le mainteneur sur HFS+), puis vert.

**P0 et P1 se corrigent et se vérifient séparément.** Les cas `iso-*` ne
regardent que les octets (store, fichiers des projets) ; les cas `cm-*`
ne regardent que la classmap contre Composer, sur un FS à copie sur
écriture d'abord (APFS) pour que P0 ne les masque pas, puis sur ext4.
`VIVACITY_NO_CLASSMAP_CACHE` est un remède de P1 seulement : il ne
défait aucun lien dur.

### Étape 1 — P0 : plus jamais de lien dur depuis le store

1. `crates/vivacity-core/src/clone.rs` : `link_or_copy_entries` →
   clonefile (macOS, arbre entier) / FICLONE (Linux) / `std::fs::copy`
   (copy_file_range sous Linux, fclonefileat puis fcopyfile sous macOS,
   CopyFileEx sous Windows). `std::fs::hard_link` disparaît du clone. Le
   repli Windows des liens symboliques (`clone_symlink_windows`) copie
   aussi au lieu de lier. Renommer en conséquence (`clone_or_copy_*`),
   mettre à jour l'en-tête du module (« pnpm model »).
2. `crates/vivacity-core/src/store.rs` : `LAYOUT` `v3` → `v4`, raison
   écrite au-dessus de la constante (une entrée a pu être modifiée à
   travers un lien dur par une version antérieure). Coût : une
   extraction froide par entrée, une fois ; les arbres `v3` restent sur
   disque jusqu'au vidage (comme aux bumps précédents, HANDOVER).
3. `composer_patcher::detach` et `write_if_changed` : gardés (ceinture),
   commentaire mis à jour.
4. **Projets déjà installés** (méta-analyse, point 1) : un no-op ne
   repose rien (`installer.rs`, `place` ne voit que `to_install`), donc
   les projets posés par une version antérieure gardent leurs liens durs
   entre eux et avec le store `v3`. Une passe unique par projet, au
   premier install (no-op compris) du nouveau binaire : parcours de
   vendor/, chaque fichier régulier à `nlink > 1` reçoit son propre inode
   (le `detach` de `composer_patcher.rs`, déplacé dans `clone.rs` et
   partagé). Un marqueur par projet dans le cache de vivacity (clé :
   chemin canonique de vendor/ + `LAYOUT`) évite de la refaire ; cache
   vidé = passe refaite, sans risque. Coût : un parcours + `stat` de
   vendor/ une fois, plus la copie de ce qui était lié — à mesurer.
   Hors de la passe, un `nlink > 1` vu par le parcours de P1 (étape 2) est
   traité de même.
5. **Disque plein** : la copie rend ce cas plus probable, après le
   retrait de l'ancienne version d'un paquet. Comportement à aligner sur
   celui d'une extraction de Composer qui échoue (message, code) ; un cas
   de harnais sur une petite image disque.
6. Bench avant/après sur Linux (conteneur) : le protocole P0 ci-dessus
   rejoué avec le vrai code contre main, le bras `copy` du spike remesuré
   dans la même série ; vert si chaque médiane reste dans la fourchette
   `copy` + 10 %. Chiffres publiés seulement après accord.

### Étape 2 — P1 : une classmap revalidée sur l'état courant

1. **Identité** (`classmap.rs`) : `FileCacheSlot::identity` devient
   (dev, inode, taille, mtime ns, ctime ns) sous Unix (`MetadataExt`),
   sur `symlink_metadata` du chemin réel. Windows : selon Q2.
   `FILE_CACHE_FORMAT` `r1` → `r2`, `CACHE_FORMAT` `v3` → `v4`.
2. **Fenêtre racy** (règle de git, révisée après la méta-analyse,
   point 4) : l'horloge de référence est prise **sur le FS de vendor/**,
   pas sur celui du cache (Docker Desktop : hôte et VM n'ont pas la même
   horloge) — un fichier sonde créé, `stat`é et supprimé sous
   `vendor/composer/` au moment d'écrire l'enregistrement (précédent :
   `.vivacity-git-*` dans `installer.rs`). Une entrée dont le ctime est ≥
   à ce temps moins une marge est marquée racy **à l'écriture** et relue
   au run suivant. La marge couvre la granularité : ~10 ms sous Linux
   avant les timestamps multigrain, 1 s (HFS+), 2 s (FAT/exFAT) —
   déduite du FS (`statfs`), ou prise à 2 s partout si la détection
   coûte : seuls les fichiers posés dans les 2 dernières secondes sont
   relus une fois.
3. **Un enregistrement par projet pour tout répertoire scanné**, vendor/
   compris : le chemin `FileCacheSlot` (répertoire listé à chaque fois →
   P1-b, P1-e réglés ; fichier par fichier → seuls les fichiers changés
   sont relus). Le `CacheSlot` de store ne sert plus directement un
   répertoire de vendor/.
4. **Manifeste de pose et amorçage** (révisé, méta-analyse point 3) :
   **tout** install qui pose un paquet depuis l'entrée E (`-o` ou non)
   écrit, pour ce projet, l'identité de chaque fichier posé relevée
   juste après la copie, et le nom de E. Un dump ultérieur (install,
   `dump-autoload -o -a` d'un déploiement) sert alors depuis
   l'enregistrement de store de E tout fichier dont l'identité courante
   égale celle du manifeste — sans lecture. C'est le bras `stat` du cadrage,
   à remesurer sur le vrai code ;
   le coût du `stat` après copie pour les installs sans `-o` est à
   mesurer.
5. **Un enregistrement de store ne vient que du répertoire de E
   lui-même** (révisé, méta-analyse point 2) : quand E n'a pas
   d'enregistrement pour un sous-répertoire, on scanne
   `<entrée>/<sous-répertoire>` (lecture des octets du store, même coût
   que la copie) — jamais une copie sous vendor/, même posée par ce run
   (au mode racy près sur HFS+ à la seconde, un script peut écrire entre
   le clone et le scan sans changer l'identité). Les chemins et l'ordre
   restent ceux du parcours de vendor/. Le risque « `dump-autoload -o`
   n'écrit plus d'enregistrement de store » disparaît.
5 bis. **Condensat dans l'enregistrement de store** (méta-analyse,
   point 3) : le scan de E lit déjà chaque fichier ; y ajouter un
   condensat par fichier coûte peu. Sur un manque d'identité (vendor/
   restauré par un cache CI, inodes neufs d'un conteneur, retour au
   contenu d'origine), le fichier est lu et condensé, et la détection de
   classes n'est rejouée que si le condensat diffère. Gain attendu sous
   Linux (la détection y domine le scan, le condensat y coûte peu d'après
   le cadrage) ; faible sous macOS (la lecture domine). Condensat :
   xxh3-128 (nouvelle dépendance, Q1) ou sha1 (déjà là, plus lent que
   xxh3 en tout-condensat sous Linux d'après le cadrage).
5 ter. **Un seul nom pour E** (méta-analyse, point 7) : le générateur
   cherche l'entrée par `dist_reference()` (`generator.rs`), l'installeur
   l'écrit par `store_reference()` (`installer.rs`) ; le générateur passe
   à `store_reference()`, sans quoi une dist sans référence n'est jamais
   amorcée.
6. `verify_files` / `still_present` deviennent inutiles (le parcours voit
   les fichiers supprimés) : retirés avec leur câblage dans `dump.rs`,
   après le vert de `plugins-round2.sh`.
7. README : la section « Known issue » remplacée par le contrat tenu et
   ses angles morts (horloge, root, FS sans ctime) ; HANDOVER (cache de
   classmap, risque résiduel de `FileCacheSlot`) ; DECISIONS (identité,
   amorçage, `LAYOUT` v4) ; CHANGELOG. Chiffres seulement après accord.

### Ordre des fichiers

`harness/vendor-isolation.sh` + son pas dans `.github/workflows/ci.yml`,
`harness/root-scan.sh` (cas `cm-root-same-size`)
→ `crates/vivacity-core/src/clone.rs` (+ test, `detach` partagé) →
`crates/vivacity-core/src/composer_patcher.rs` → `crates/vivacity-core/src/store.rs`
→ `crates/vivacity-core/src/installer.rs` (passe de déliaison, manifeste
de pose) → `crates/vivacity-autoload/src/classmap.rs` (identité, racy,
enregistrement de store depuis E, condensat) →
`crates/vivacity-autoload/src/generator.rs` (sélection des slots,
amorçage, `store_reference`) → `crates/vivacity/src/install.rs` →
`crates/vivacity/src/dump.rs` → docs.

### Critères de vert

- **Obligatoire (mainteneur)** : `harness/vendor-isolation.sh` vert, les
  deux ordres (`iso-before`, `iso-after`) et les deux bras du cache — A
  porte les nouveaux octets avec le mode de Composer, B et l'entrée de
  store gardent les octets du dist au condensat près ; gardes 1 et 2
  tenues (écriture réelle et en place dans A ; store partagé, lu par
  l'install de B, sur un FS sans copie sur écriture vérifié par sonde) ;
  vu rouge avec le binaire de main **sur le critère**, garde d'inode
  partagé tenue. Câblé dans le job `parity` de `ci.yml` (ubuntu, ext4 ;
  macOS sur image HFS+ si le coût le permet) et vert là. P0 est déclaré
  corrigé sur ce critère seul, indépendamment de P1 ; `iso-upgrade` vert
  en plus.
- Tous les cas `iso-*` et `cm-*` verts sur macOS (APFS + image HFS+) et
  Linux (conteneur, ext4 et overlayfs), vus rouges avant ; et sur le
  runner Windows (méta-analyse, point 6) — `cm-same-size-mtime` n'y
  passe qu'avec `ChangeTime` + `FileId` ou un condensat (Q2) : pas de
  livraison de P1 sous Windows sur (mtime, taille).
- Suites existantes (lancées par le mainteneur) : `diff-vendor.sh
  --with-autoloader` (froid puis chaud), `root-scan.sh`,
  `composer-patcher.sh` (store intact), `plugins-round2.sh` (liborm85),
  `composer-packages.sh` (drupol), `path-repos.sh`, `vendor-dir.sh`,
  corpus ; `cargo test`, clippy `-D warnings`, cible Windows typecheckée
  (`x86_64-pc-windows-gnu`).
- Perf : le protocole ci-dessus rejoué avec le vrai code (pas le spike), les
  bras de référence remesurés dans la même série ; vert si P1 reste dans la
  fourchette `stat`…`files` + 10 % et P0 dans `copy` + 10 %. No-op chiffré
  (méta-analyse, point 5) : médiane du no-op au plus +5 ms au-dessus de main
  sur les trois fixtures, macOS et Linux ; plus un no-op `-o` sur sylius et
  sur invoiceninja (corpus, 72 430 classes, où un contrôle plus léger a coûté
  8 %, HANDOVER), où le scan dépasse l'attente réseau — budget à fixer par le
  mainteneur sur ces chiffres.

## Risques

1. **Windows** : la copie y crée chaque fichier de vendor/ à neuf (des dizaines
   de milliers sur sylius) ; avec
   Defender en temps réel, ce coût peut dépasser de loin celui de Linux.
   Non mesuré ici. À mesurer (`bench-windows`, dispositions `same C:` et
   `split D:/C:`) avant de livrer P0 sous Windows.
2. **Disque** : sur un FS sans CoW, chaque projet porte sa copie (en plus du
   store, la taille entière de son vendor/).
3. **Sémantique du ctime** sur des FS exotiques (FUSE, 9p/virtiofs des
   montages Docker Desktop, NFS) : un FS qui ne tient pas le ctime
   retombe dans le cas mtime + taille. À lister dans le README.
4. **Fenêtre racy** : couverte par la règle de l'étape 2.2 ; un écrivain
   concurrent du run reste hors contrat (comme deux installs simultanés).
5. **Bumps** `LAYOUT` / `CACHE_FORMAT` / `FILE_CACHE_FORMAT` : un premier
   run froid pour tout le monde, y compris sur APFS où P0 ne mordait pas ;
   anciens arbres et enregistrements orphelins sur disque.
6. **Croissance du cache** : un enregistrement et un manifeste par
   projet et par chemin, jamais élagués (c'est déjà le cas de
   `classmap-files/`). À traiter avec l'élagage des anciens `store/vN`
   (HANDOVER), hors de ce chantier.
7. **macOS, `dump -o` de sylius** : le `stat` par fichier sur APFS serait cher
   d'après le cadrage. Piste à mesurer : `getattrlistbulk` (attributs de tout
   un répertoire en un appel) ; seulement si le mainteneur juge ce coût trop
   haut.

## Méta-analyse (sous-agent frais, plan et code, 2026-10-31)

Entrées : cadrage et plan seuls (sans les options écartées ni les
mesures), accès en lecture au dépôt, consigne de réfuter avec au moins
deux failles après avoir construit sa propre solution. Sept failles, deux
majeures ; toutes vérifiées dans le code avant d'être reprises.

| # | faille | gravité | suite donnée |
|---|---|---|---|
| 1 | Les projets installés avant le correctif gardent leurs liens durs : un no-op ne repose rien (`installer.rs`, `place` sur `to_install` seulement), le bump de `LAYOUT` ne change que la source des installs neufs | majeure | **pris** : passe unique de déliaison par projet (étape 1.4), cas `iso-upgrade` |
| 2 | L'enregistrement de store écrit depuis une copie « posée par ce run » refait P1-c dans la fenêtre racy (HFS+ à la seconde, script entre clone et scan) ; `matches_entry` lit déjà le répertoire de E | majeure | **pris** : enregistrement de store seulement depuis le répertoire de E (étape 2.5) ; le risque « moins d'enregistrements » disparaît |
| 3 | L'amorçage limité aux paquets posés par le run perd le cache sur `install --no-dev` puis `dump-autoload -o -a` (déploiement), sur un vendor/ restauré par cache CI, sur un overlay dont `st_dev` change | moyenne | **pris** : manifeste de pose écrit par tout install (2.4) ; condensat dans l'enregistrement de store pour éviter la détection sur un manque d'identité (2.5 bis) |
| 4 | La fenêtre racy compare des horloges de deux FS (cache et vendor/), et la marge dépend de la granularité du FS | moyenne | **pris** : sonde sur le FS de vendor/, marquage à l'écriture, marge par FS (2.2) |
| 5 | « Aucune régression du no-op » contredit la relecture des répertoires sur le no-op ; budget non chiffré | moyenne | **pris** : budget chiffré, no-op `-o` sylius et invoiceninja ajoutés aux critères |
| 6 | Windows : critère du cadrage, absent des critères de vert ; la std stable n'a ni ctime ni identifiant de fichier | moyenne | **pris** : `cm-*`/`iso-*` sur le runner Windows exigés ; pas de P1 Windows sur (mtime, taille) |
| 7 | Générateur (`dist_reference`) et installeur (`store_reference`) ne nomment pas E pareil : une dist sans référence n'est jamais amorcée | mineure à moyenne | **pris** (2.5 ter) |
| — | Disque plein plus probable avec la copie, vendor/ partiel | mineure | **pris** (étape 1.5) |
| — | Le croissance non bornée des enregistrements par projet | mineure | noté (risque 6), hors chantier |
| — | La sonde FICLONE serait redondante avec `copy_file_range` (qui reflinke sur btrfs/xfs) | mineure | **gardée** : FICLONE est explicite, mesuré, et ne dépend pas de la version du noyau ni du comportement de `copy_file_range` entre FS ; à revoir seulement avec un banc btrfs/xfs |
| — | `collect_files` (`follow_links(true)`) ferait déjà un `stat` par entrée, d'où un double `stat` avec `identity()` | — | **réfuté** : walkdir 2.5.0 (`src/lib.rs:844`) ne suit (et ne `stat`e) que les liens symboliques ; le type vient de `readdir`. Un seul `stat` par fichier dans le plan |

Ce qui a changé dans le plan : étapes 1.4, 1.5, 2.2, 2.4, 2.5, 2.5 bis,
2.5 ter, cas `iso-upgrade`, critères de vert (Windows, no-op chiffré),
risque 6. La solution du réfutateur et celle du plan convergent
désormais ; reste ouvert le choix identité contre condensat (Q1) et
Windows (Q2).

Ajout du mainteneur après la méta-analyse : le critère obligatoire de P0
(contenu du store vérifié au condensat avant/après une écriture externe
en place, sur deux projets partageant le store) et la séparation
explicite P0/P1 (`VIVACITY_NO_CLASSMAP_CACHE` ne fait rien contre la
contamination par lien dur) ; puis le protocole final de P0, qui
remplace les deux notes : les deux ordres (`iso-before`, `iso-after`),
octets de A, de B et du store contre le dist, garde « l'écriture a eu
lieu, mode de Composer » et garde « store partagé, lu, sur un FS qui
lierait » (sonde de FS, inode partagé vu avec le binaire de main), en
harnais permanent `harness/vendor-isolation.sh` câblé dans `ci.yml` —
étape 0 et critères de vert.

## Questions ouvertes pour le mainteneur

- **Q1** : identité forte (b) partout, ou condensat xxh3 (c) sous Linux (peu
  coûteux d'après le cadrage) et identité forte sous macOS ? (c) partout
  serait cher sous macOS. Si (c) : `xxhash-rust =0.8.15` (feature `xxh3`, sans
  dépendance transitive), entrée DECISIONS.
- **Q2** : Windows — `ChangeTime` + `FileId` par énumération de
  répertoire (`windows-sys` direct) ou condensat ? Et le coût de la copie
  P0 sous Windows, à mesurer avant de choisir.
- **Q3** : bump de `LAYOUT` pour tous (simple, un run froid pour chacun) ?
  L'alternative — vérifier chaque entrée contre son archive une fois — lit
  tout le store et demande l'archive.
- **Q4** : le doublement de l'espace disque sur FS sans CoW est-il
  acceptable tel quel, ou faut-il l'écrire dans le README ?
- **Q5** : `getattrlistbulk` (macOS) dans ce chantier ou après ?

## Décisions du mainteneur (2026-10-31)

- **Q1** (P1, plus tard) : identité par métadonnées, ctime compris ; le
  condensat seulement quand elles changent.
- **Q2** : Windows mesuré avant de choisir ; les résultats Unix ne s'y
  transposent pas.
- **Q3** : oui, bump du format du store (`LAYOUT` `v4`) ; une ancienne
  entrée n'est jamais servie.
- **Q4** : les copies par projet sont acceptées ; leur coût disque est
  écrit dans le README.
- **Q5** : `getattrlistbulk` reporté à un chantier séparé.
- **La justesse avant la vitesse** : 0.26.1 rétablit les garanties même à
  un coût mesuré ; aucune optimisation nouvelle dans ce chantier (la copie
  parallèle par fichier et le renommage à store froid de « Options P0 »
  restent hors de 0.26.1).
- **P0 puis P1**, chacun avec ses commits et ses tests de régression ; le
  cache de classmap n'est pas touché par P0.

### Réserves du mainteneur sur P0, et ce qu'elles changent

1. **La sonde « le clone échoue » n'est pas un critère de justesse.** Elle
   ne sert qu'à reproduire le défaut historique (phase rouge, FS à liens
   durs). Le critère repose sur des propriétés observables — octets de A
   (écriture acceptée), de B et du store (octets d'origine contre une
   extraction à part de l'archive), modes — et les mêmes contrôles d'octets
   passent sur un FS à copie sur écriture (`ISO_FS=cow`). Le harnais
   n'affirme nulle part « pas de clone » ni une technique particulière ;
   `nlink` n'y est qu'une information, et la garde d'inode partagé ne vaut
   que pour la phase rouge.
2. **`iso-upgrade` distingue migration et réparation.** Projet installé
   avant le correctif, fichiers de vendor/ encore liés à une ancienne
   entrée :
   - *migration* : chaque fichier encore lié reçoit sa propre copie,
     **contenu courant préservé** (un correctif local délibéré n'est jamais
     détruit) ;
   - le nouveau store (`v4`) ne se reconstruit que depuis l'archive (cache
     de zips de Composer ou dist, vérifiés par `dist.shasum` quand il n'est
     pas vide — il l'est souvent pour les dists GitHub), jamais depuis une
     ancienne entrée, peut-être contaminée ;
   - *ambiguïté* : un fichier lié dont le contenu diffère de l'archive ne
     s'attribue pas (correctif délibéré ici, ou propagé d'ailleurs). Il
     n'est pas « réparé » en silence. Le signaler sans toucher stdout ni
     stderr (parité) est un **point de décision** du mainteneur : la partie
     sûre (déliaison, contenu préservé) est faite, les options de
     signalement sont décrites dans le rapport d'exécution ;
   - cas testé : une ancienne entrée éditée avant la mise à jour ne devient
     pas la référence (nouveau store = octets de l'archive ; le projet garde
     ses octets courants ; un projet installé ensuite reçoit l'archive).
3. **Le harnais exerce vivacity, pas Composer.** `update` confie les dépôts
   `artifact` à Composer depuis 0.26 : le harnais installe depuis un lock
   (`install`, comme `harness/artifact-repo.sh`) et affirme dans chaque cas
   l'absence de passation (`--no-fallback`, code 0, la ligne de résumé de
   vivacity, aucune ligne `delegating`).

## Exécution de P0 (2026-10-31)

**Fait.**
- `clone.rs` : plus de `hard_link` ; clonefile (macOS) → FICLONE (Linux) →
  `std::fs::copy` ; le repli Windows des liens symboliques copie aussi.
  `detach` (déplacé depuis `composer_patcher.rs`, réexporté) en deux formes :
  stricte avant `patch` (erreur), indulgente pour la passe unique
  (`detach_all` : laisse un fichier qu'elle ne peut pas copier et le dit).
  La copie de déliaison garde octets, mode, mtime et (Unix) propriétaire.
  Nombre de liens sous Windows : `winapi-util =0.1.11` (DECISIONS).
- `store.rs` : `LAYOUT` `v4` ; marqueur `store/.detached/v4/<sha1 du chemin
  canonique de vendor/>`.
- `installer.rs` : `detach_once` après le placement (après le dernier point
  de passation à Composer), sur vendor/ et les paquets posés hors de
  vendor/ (ni liens symboliques ni ancêtres de vendor/) ; jamais d'erreur ni
  de ligne ; marqueur écrit seulement si rien n'est resté lié.
- `harness/vendor-isolation.sh` + pas dans le job `parity` de `ci.yml`.
- Tests unitaires (`clone.rs`) : isolation du parcours de copie par une
  écriture en place, inconditionnelle (rouge avec le lien dur remis) ;
  `detach` garde le contenu courant, le mode et la mtime ; répertoire en
  lecture seule (forme indulgente : rien de cassé, `false` ; stricte :
  erreur).

**Vu rouge puis vert** (harnais, 73 contrôles `OK` au vert) :

| où | binaire de main 9fa38b6 (`ISO_PHASE=red`) | correctif |
|---|---|---|
| macOS, image HFS+ | 24 échecs sur le critère, 0 garde | 0 / 0 |
| macOS, APFS (`ISO_FS=cow`) | `iso-before`/`after` verts (clonefile), `iso-upgrade*` rouges | 0 / 0 |
| Linux (conteneur `vivacity-linux`), volume ext4 | 24 / 0 | 0 / 0 ; et 0 / 0 avec `VIVACITY_OLD_BIN` = main (état d'avant posé par le vrai binaire) |
| Linux, racine overlayfs | 24 / 0 | 0 / 0 ; idem |

Mutants du correctif (macOS HFS+), chacun attrapé par les cas attendus et
eux seuls : lien dur remis (`iso-before`/`after` : B et store) ; passe
retirée (`iso-upgrade*` : B et ancienne entrée après l'écriture) ; `LAYOUT`
resté `v3` (pas de nouvelle entrée ; `iso-upgrade-edited` : C reçoit
l'entrée éditée) ; copie qui perd les modes (modes de A, B, C).

**Revue indépendante (sous-agent frais, sur le diff) et suites.**

| # | constat | suite |
|---|---|---|
| 1 | la passe faisait échouer un install no-op que Composer mène à bout (répertoire en lecture seule, disque plein, fichier verrouillé sous Windows) | **pris** : forme indulgente, ni erreur ni ligne, marqueur non écrit, nouvel essai au prochain install |
| 2 | des liens refaits par un binaire antérieur après le marqueur ne sont pas vus | **noté** : README, DECISIONS, HANDOVER ; le parcours de P1 les verra (étape 1.4) ; `store/.detached/` à effacer pour rejouer |
| 3 | erreurs de lecture avalées puis marqueur écrit | **pris** : un répertoire illisible ou une entrée sans métadonnées rend la passe incomplète |
| 4 | contrôle de mode du harnais imperdable (0644 = défaut de l'umask) | **pris** : `src/Iso.php` en 0640, modes des quatre fichiers comparés à Composer pour A, B, C ; mutant « modes perdus » rouge |
| 5 | harnais : chemin exercé non dit, no-op de B non vérifié, écriture de même taille seulement | **pris** : ligne d'info clone/copie, no-op de B, `iso-after` réécrit par `>` (autre taille, même inode) |
| 6 | une passation après la passe ne laisse plus le projet « tel qu'il était » (inode, mtime, propriétaire sous `sudo`) | **pris** : passe déplacée après le placement ; mtime et propriétaire gardés |
| 7 | docs : « premier install » vrai seulement en natif ; commentaire de `phpcs_installer.rs` ; en-tête du plan | **pris** |
| 8 | `fixtures/work` (lien symbolique) apparaît non suivi | **noté** : jamais ajouté (ajouts nommés) ; `.gitignore` ne le couvre pas, hors de ce chantier |
| 9 | copies inutiles au premier `update` | **pris** par le déplacement (6) |

**Point de décision ouvert : signaler les fichiers liés qui diffèrent de
l'archive** (attribution impossible). Rien n'est imprimé aujourd'hui.
Options, sans toucher stdout ni stderr : (a) une ligne sous
`VIVACITY_TRACE=1` (déjà hors parité) ; (b) un fichier dans le cache de
vivacity, par projet (`store/.detached/v4/<clé>.json` : chemins, condensat
courant, condensat de l'archive), lisible après coup ; (c) les deux ; (d)
une sous-commande ou option de diagnostic (`vivacity doctor`), plus tard.
Toutes demandent de lire l'archive pendant la passe (cache de zips de
Composer, condensat par fichier) : un coût unique par projet, à mesurer.

### Rapport de la passe unique — décision du mainteneur : (c), légère

**Plan.**
1. `clone.rs` : `detach_all(dir, &mut Vec<LinkedFile>)` relève chaque
   fichier régulier à `nlink > 1` AVANT de le délier : chemin, nombre de
   liens, (Unix) périphérique et inode, sha1 du contenu courant (une lecture
   de ce fichier seulement), puis l'issue de la déliaison (faite / échec et
   raison). `detach` strict inchangé (pas de relevé, pas de condensat).
2. ~~`extract.rs` : lecture groupée des entrées d'une archive~~ (abandonné
   après la méta-analyse, ci-dessous : extraction à part par le store).
3. Nouveau module `detach_report.rs` : rattache chaque fichier relevé au
   paquet dont le répertoire le contient (le plus long préfixe), et compare
   au fichier de même chemin relatif dans l'archive du cache de fichiers de
   Composer (`fetch::dist_cache_path_of`, même URL que l'install) — après la
   méta-analyse : d'abord l'entrée `v4` que ce run a extraite — : archive
   lue une fois par paquet ayant des fichiers liés, son sha1 noté, refusée
   comme référence si `dist.shasum` est non vide et diffère. Jamais l'ancien
   store (`v3`), ni une entrée `v4` antérieure au run, jamais de réseau. Issues séparées : déliaison
   faite / échec (raison) ; comparaison identique / différente /
   impossible (raison : pas d'archive dans le cache, archive illisible,
   shasum faux, fichier absent de l'archive, paquet sans archive, contenu
   courant illisible, fichier hors de tout paquet).
4. `installer.rs` `detach_once` : collecte, puis, si au moins un fichier
   était lié, écrit `store/.detached/v4/<clé>.json` (temp + rename) en
   AJOUTANT une observation à celles déjà là (une passe incomplète refaite
   ne trouve plus les fichiers déjà déliés : réécrire perdrait
   l'information ; après la méta-analyse : un fichier par passe dans
   `store/.detached-reports/v4/<clé>/`) ; une ligne sous `VIVACITY_TRACE=1` (chemin, comptes, ou
   l'échec d'écriture) ; rien d'autre, jamais d'erreur.
5. Tests unitaires (identique, différent, archive absente, shasum faux,
   ajout d'une observation) ; harnais : `iso-upgrade*` vérifient le
   rapport, nouveau cas `iso-upgrade-nocache` (archive retirée du cache de
   Composer : « non comparé », fichiers déliés quand même).
6. Docs : README, HANDOVER, CHANGELOG, DECISIONS, ce plan.

**Hors champ** : une entrée `v4` comme référence (rien n'y dit de quelle
archive elle vient) ; le chemin local d'une dist `artifact` (le cache de
Composer seul, pour une seule règle) ; `vivacity doctor`.

**Méta-analyse (sous-agent frais, plan et code), et ce qu'elle a changé.**
Treize constats ; pris : (1, 2) relire l'archive que ce run vient
d'extraire coûte une seconde décompression de tout le projet sur ext4 →
l'entrée `v4` que CET appel de `Store::ensure_made` a extraite devient la
référence (provenance établie : l'archive, son URL, son chemin dans le cache
de Composer) ; seule une entrée déjà présente avant le run reste exclue ;
(5, 6) la lecture par entrée d'archive ne reproduisait ni les liens d'un
tar, ni le renommage Windows → l'archive du cache de Composer est extraite à
part par l'extraction du store (`Store::extract_staged`) au lieu d'un
lecteur groupé (l'étape 2 du plan, `extract.rs`, est abandonnée) ; (3, 4)
un fichier unique enrichi par ajout se perd en course et grossit à chaque
install tant qu'un fichier reste illisible → un fichier par passe, jamais
réécrit, et une passe qui ne délie rien n'écrit rien si un rapport existe ;
(7) effacer `store/.detached/` pour rejouer la passe effacerait les
rapports → `store/.detached-reports/` ; (8) la racine du cache de Composer
est celle du `Fetcher`, l'URL celle du lock ; (9) l'attribution exclut les
paquets que le parcours exclut ; (10) le condensat courant est pris pendant
la copie ; (11) fichiers triés, regroupés par paquet ; (12, 13) notés
(README, DECISIONS), le rapport ne passe jamais par `dist_bytes_of`.

**Exécution (2026-10-31).** `clone.rs` (`LinkedFile`, condensat pendant la
copie par un lecteur qui hache), `store.rs` (`ensure_made`,
`extract_staged`, `detach_reports`), `fetch.rs` (`Fetcher::cache_root`),
`detach_report.rs` (nouveau), `installer.rs` (`detach_once`,
`report_detached`). Revue indépendante (sous-agent frais, sur le diff) :
la première version hachait chaque archive extraite tant que la passe du
projet restait à faire, même sans fichier lié (premier install d'un projet
neuf, APFS) → plus rien n'est haché dans les tâches de téléchargement :
`dist.shasum` quand le lock en porte un (la récupération a vérifié les
octets), sinon le fichier du cache de Composer, lu seulement pour un paquet
qui a des fichiers liés ; nom de fichier à la nanoseconde ; contrôle de
sortie du harnais étendu aux lignes `trace:`. Harnais vu rouge avec le
binaire de la branche sans ce code (18 échecs sur le critère, 0 garde, sur
image HFS+ comme sur APFS), vert avec (`ISO_FS=link` et `cow`) ; deux
mutants attrapés : l'ancien store pris pour référence (`iso-upgrade-edited`,
A), une ligne imprimée hors trace (tous les `iso-upgrade*`).

**Reste à faire avant la release** : le job `parity` sur les runners (le
pas n'a pas encore tourné ; `hdiutil` sur `macos-latest` à confirmer) ;
Windows (copie et `detach` typecheckés, jamais lancés ; coût de la copie
avec Defender à mesurer, `bench-windows`) ; les bancs P0 rejoués avec le
vrai code (critère « `copy` + 10 % ») ; les suites complètes (mainteneur).

## Annexe A — le spike de mesure (non commité)

Quatre fichiers, tout derrière des variables d'environnement ; retiré par
`git checkout -- crates Cargo.lock` après la mesure.

- `clone.rs` : `VIVACITY_SPIKE_CLONE=copy` → `std::fs::copy` au lieu de
  FICLONE + lien dur dans `link_or_copy_entries`.
- `generator.rs` : `VIVACITY_SPIKE_VALIDATE=files` → `cache_slot` rend
  `FileCacheSlot` pour tout répertoire.
- `classmap.rs` : `VIVACITY_SPIKE_VALIDATE=stat|xxh3|sha1` → sur un hit
  de `CacheSlot`, `collect_files(path)` puis par fichier `symlink_metadata`
  (stat) ou `std::fs::read` + `xxh3_128` / `Sha1::digest`, en `par_iter`
  si `parallel_io()`, résultat passé à `black_box`.
- `vivacity-autoload/Cargo.toml` : `xxhash-rust = "=0.8.15"` (xxh3).

Banc macOS : `hyperfine -N --warmup 2 --runs 10` sur `env X=1 vivacity
<cmd> -q` et les bras ; Linux : le même dans `vivacity-linux`, volumes
`vi-iso-*` créés pour la mesure puis supprimés.
