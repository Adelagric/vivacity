# Plancher glibc des binaires Linux — plan

> Niveau 2 : les assets de release (`vivacity-<tag>-<target>.tar.gz`),
> `install.sh` et l'action sont un contrat public. Mesures du 2026-10-29.

## Le problème (mesuré)

Les binaires Linux de v0.25.0 (`x86_64-unknown-linux-gnu` construit sur
`ubuntu-latest`, `aarch64-unknown-linux-gnu` sur `ubuntu-24.04-arm`)
exigent **GLIBC_2.39** : `objdump -T` montre deux références faibles
versionnées de la std Rust (lancement de processus), `pidfd_spawnp` et
`pidfd_getpid`. NEEDED : `libgcc_s`, `libm`, `libc`, `ld-linux`.

Matrice (conteneurs Docker, binaires de la release, un `--version` puis un
vrai `vivacity install` de deux paquets par le réseau) :

| image | glibc | aarch64 v0.25.0 | x86_64 v0.25.0 |
|---|---|---|---|
| centos:7 | 2.17 | refus du chargeur | refus du chargeur |
| amazonlinux:2 | 2.26 | refus | refus |
| debian:10, rockylinux:8 | 2.28 | refus | refus |
| ubuntu:20.04, debian:11 | 2.31 | refus | refus |
| rockylinux:9, amazonlinux:2023 | 2.34 | refus | **tourne, avertissement sur stderr** |
| ubuntu:22.04 | 2.35 | refus | **tourne, avertissement sur stderr** |
| debian:12 | 2.36 | refus | **tourne, avertissement sur stderr** |
| ubuntu:24.04, debian:13 | 2.39, 2.41 | OK | OK |

Nuance trouvée en mesurant : sur x86_64 l'entrée `GLIBC_2.39` de la section
des versions requises porte le drapeau *weak* (0x02) ; une glibc ≥ 2.34
charge alors le binaire mais écrit
`/v: …/libc.so.6: weak version 'GLIBC_2.39' not found (required by /v)` sur
stderr **à chaque lancement** — la promesse de stderr identique à Composer
est cassée sur les runners `ubuntu-22.04`, RHEL 9, AL2023, Debian 12. Sous
2.34 (pas de `GLIBC_2.34` pour `__libc_start_main` et `pthread_*`), refus.
Sur aarch64 l'entrée n'est pas faible : refus partout sous 2.39.

## Cadrage

- **Problème** : l'action échoue (aarch64) ou pollue stderr (x86_64) sur les
  runners `ubuntu-22.04` ; `install.sh` pose un binaire inutilisable sur
  php:*-bookworm, RHEL 8/9, AL2023, Ubuntu 20.04/22.04, Debian 11/12, et
  rien n'existe pour Alpine (image officielle `composer`, php:*-alpine).
- **Contraintes** : les binaires restent ceux d'aujourd'hui en
  comportement (même code, mêmes dépendances, `Cargo.lock` inchangé) ;
  outils et images épinglés exactement ; diff des workflows ciblé (un autre
  chantier ré-épingle Composer 2.11.0 dans les workflows).
- **Non-objectifs** : macOS et Windows (inchangés) ; Linux 32 bits, riscv,
  ppc ; un allocateur de remplacement ; le plancher du noyau (celui de la
  std Rust, 3.2) ; une build `.deb`/`.rpm` ; épingler la chaîne Rust du
  workflow de release (`dtolnay/rust-toolchain@stable`, inchangé).
- **Critères de succès observables** :
  1. `release.yml` produit des binaires dont la version `GLIBC_` maximale
     est ≤ au plancher, vérifiée par un pas qui échoue sinon ;
  2. le binaire tourne (`--version` + un vrai `vivacity install`) dans le
     plus vieux conteneur supporté, en CI, sur les deux architectures ;
  3. ce contrôle est vu rouge sur le binaire v0.25.0 (local ET en CI) ;
  4. `install.sh` (donc l'action) choisit le bon asset ;
  5. README, HANDOVER, DECISIONS, CHANGELOG, docs/docker.md disent le
     plancher.

## Les options, mesurées

**A — `cargo-zigbuild`, cible versionnée `<triple>.2.17`.** Zig fournit
des bibliothèques glibc de liaison à la version demandée et sert de
compilateur C (`cc`) et d'éditeur de liens.
Spike sur ce Mac (cargo-zigbuild 0.23.4, zig 0.16.0, rustc 1.99.0) :

- `aarch64-unknown-linux-gnu.2.17` en 33 s, `x86_64-unknown-linux-gnu.2.17`
  idem, sans aucun changement de code : PCRE2 préfixé (`cc`) et ring
  (`cc` + assembleur) compilent avec `zig cc`.
- `objdump -T` : `GLIBC_2.17` au plus. Les références faibles de la std
  (`pidfd_spawnp`, `pidfd_getpid`, `statx`, `getrandom`,
  `copy_file_range`, `__cxa_thread_atexit_impl`) deviennent faibles **non
  versionnées** : résolues si la glibc les a, nulles sinon (la std a son
  repli pour chacune).
- NEEDED : `libc`, `libm`, `libpthread`, `libdl` (+ `ld-linux-x86-64`) ;
  plus de `libgcc_s` (zig lie son propre unwinder).
- La matrice ci-dessus, binaire 2.17 : **OK sur les 12 images × 2
  architectures**, centos:7 compris, install réseau (TLS rustls,
  extraction, autoloader) réussie.
- Différence de forme : zig traduit `-Wl,--strip-debug` (le
  `strip = "debuginfo"` par défaut du profil release) en strip **total** :
  plus de `.symtab`, donc des backtraces sans noms de fonction (14,2 → 10,9
  Mo en aarch64). Les binaires v0.25.0 gardaient leur table des symboles.

**B — construire dans un conteneur à vieille glibc** (manylinux2014,
CentOS 7 ; manylinux_2_28, AlmaLinux 8). Écarté : `actions/checkout` et les
actions node ne tournent pas dans un conteneur CentOS 7 (node20/24 exigent
glibc 2.28), il faudrait installer rustup dans l'image à chaque job, et
pour aarch64 un second conteneur ; le Mac de dev ne peut refaire la build
x86_64 qu'en émulation. A se reproduit à l'identique ici et en CI.

**C — runners plus anciens** (`ubuntu-22.04`, glibc 2.35) : ne couvre ni
RHEL 8 (2.28) ni RHEL 9 et AL2023 (2.34) ni Ubuntu 20.04 et Debian 11
(2.31) ; et la prochaine dépréciation de runner remonte le plancher en
silence.

**Choix : A.**

## Le plancher : glibc 2.17

Cibles réelles nommées : Ubuntu 20.04 (2.31) et 22.04 (2.35), Debian 11
(2.31) et 12 (2.36), RHEL 8 (2.28) et 9 (2.34), AL2023 (2.34) ; leur
minimum est 2.28. Le binaire 2.17 tourne sur tout ce qui précède **et**
sur CentOS/RHEL 7, Amazon Linux 2, Debian 10, Ubuntu 18.04 — les images
PHP 7.x (stretch/buster) que l'utilisateur d'un Composer « drop-in » garde
parfois. 2.17 est le plancher de la std Rust elle-même pour ces deux
cibles : rien de plus bas n'est possible en glibc, et rien ne coûte entre
2.17 et 2.28 (même chemin de build, mêmes performances mesurées).

Ce qui ferait remonter le plancher : une version de la std (ou une
dépendance C) qui exigerait un symbole absent de 2.17 en référence forte.
Zig refuse alors l'édition de liens : la release échoue bruyamment, on
remonte la constante. Pas de régression silencieuse possible.

Image de test du plancher : `centos:7` (glibc 2.17, amd64 et arm64v8),
épinglée par digest. Image figée (EOL), mais on n'y installe rien : on y
lance le binaire.

## musl : ajouté (statique)

`x86_64-unknown-linux-musl` et `aarch64-unknown-linux-musl` par le même
`cargo zigbuild` : 50 s, binaire **statique** (10,3 Mo aarch64), sans
changement de code. PCRE2 est construit sans JIT sur aarch64-musl
(`enable_jit`), mais vivacity n'active jamais le JIT (le `RegexBuilder` du
crate est en `JITChoice::Never` par défaut) : aucune différence.

Banc Laravel (109 paquets), conteneur arm64 `php:8.4-cli` (4 vCPU de la VM
Docker d'un M4 Max), hyperfine, médianes de deux passes, mêmes cache et
store :

| scénario | gnu natif (rustc 1.99.0, gcc) | gnu zig 2.17 | musl zig |
|---|---|---|---|
| no-op | 46,3 / 42,6 ms | 45,6 / 44,6 ms | 40,8 / 41,8 ms |
| warm (vendor/ supprimé) | 69,5 / 70,3 ms | 67,7 / 67,6 ms | 66,8 / 66,7 ms |
| dump-autoload -o | 30,0 / 27,0 ms | 32,5 / 28,3 ms | 30,6 / 30,9 ms |

Pas de ralentissement mesurable de l'allocateur musl à 4 cœurs sur ce
banc. Non mesuré : un runner à beaucoup de cœurs (contention de
l'allocateur de musl sous rayon) — risque résiduel, dit au DECISIONS.
Le binaire v0.25.0 (rustc 1.99.1, gcc 13 d'ubuntu 24.04) fait 25 ms en
dump-o : l'écart vient de la chaîne (1.99.0 contre 1.99.1, gcc), pas de
zig — le natif et le zig à rustc égal sont dans le bruit l'un de l'autre.

Écart de parité connu sous musl, déjà au HANDOVER : PHP sous musl n'a pas
`GLOB_BRACE` (Composer y refuse les accolades d'un dépôt `path`, vivacity
les développe). Inchangé par ce chantier, mais désormais atteignable.

## Ce qui change

1. **`tools/linux-release.sh`** (nouveau), deux sous-commandes, les
   constantes au même endroit (`GLIBC_FLOOR=2.17`, images de test
   épinglées par digest) :
   - `build <triple>` : installe dans un venv (sous `target/`) la chaîne
     épinglée par hachage `tools/zigbuild-requirements.txt`
     (`cargo-zigbuild==0.23.4`, `ziglang==0.16.0`, `pip
     --require-hashes`), puis `cargo zigbuild --release --locked -p
     vivacity --target <triple>[.2.17]` (table des symboles : voir le
     modèle révisé).
   - `check <triple> [binaire]` : (a) ELF : gnu → version `GLIBC_`
     maximale de `objdump -T` ≤ plancher, NEEDED dans une liste blanche
     (`libc`, `libm`, `libpthread`, `libdl`, `librt`, `ld-linux-*`,
     `libgcc_s`) ; musl → aucun NEEDED, pas de `.interp` ; (b) conteneur :
     gnu → `centos:7`, musl → `alpine` ; `--version` puis un vrai
     `vivacity install` (sans PHP dans l'image : la vérification de
     plateforme est sautée, le reste — TLS, téléchargement, extraction,
     autoloader — est le vrai chemin). Révisé après la méta (plus bas).
2. **`release.yml`** : les deux lignes Linux passent par
   `tools/linux-release.sh build` puis `check` ; deux lignes musl
   ajoutées (x86_64 sur `ubuntu-latest`, aarch64 sur `ubuntu-24.04-arm` :
   le conteneur de test tourne en natif) ; le reste de la matrice
   inchangé. `package-release.sh` inchangé (son `--version` d'auto-test
   tourne sur le runner).
3. **`action-test.yml`, job `installers`** (chaque push et PR, x86_64
   seulement) : la ligne ubuntu construit comme la release
   (`linux-release.sh build` + `check`), une ligne musl est ajoutée ;
   `install.sh` est lancé **dans** le conteneur de plancher (centos:7 pour
   gnu, alpine pour musl) contre les assets locaux : il prouve la détection
   et l'asset choisi. **Témoin négatif** : le binaire v0.25.0
   x86_64 (asset de la release, sha256 vérifié) doit être REFUSÉ par
   `check`, pour la bonne raison (`GLIBC_2.39`) — le contrôle est vu rouge
   à chaque run.
4. **`install.sh`** : sous Linux, musl détecté (`/lib/ld-musl-*.so.1`
   présent, ou `ldd --version` qui dit musl) → `unknown-linux-musl`, sinon
   `unknown-linux-gnu`. Un tag sans asset musl (≤ v0.25.0) échoue avec un
   message clair au lieu d'un 404 nu. L'action passe par `install.sh` :
   rien d'autre à changer (son `shell: bash` exige bash dans un conteneur
   Alpine : documenté).
5. **Docs** : README (Install), HANDOVER (validation + ce qui n'est pas
   couvert), DECISIONS (2026-10-29, français, les mesures), CHANGELOG
   (Unreleased), docs/docker.md (la ligne « Debian-based only »).

## Vérification

- Rouge d'abord : `tools/linux-release.sh check <triple> <binaire v0.25.0>`
  en local, les deux architectures : ELF rouge (`GLIBC_2.39 > 2.17`),
  conteneur rouge (`version 'GLIBC_2.39' not found`).
- Vert : `tools/linux-release.sh build` puis `check` en local pour les
  quatre triples (zig depuis macOS, conteneurs arm64 natifs et amd64
  émulés) ; `package-release.sh` sur chacun ; `install.sh` dans centos:7,
  debian:12, alpine contre les assets locaux.
- La matrice des 12 images refaite avec les binaires finaux.
- `actionlint` sur les deux workflows ; `shellcheck` sur les scripts
  touchés s'il est là.
- Gates Rust : aucun fichier Rust ni `Cargo.lock` touché → non requis ;
  `cargo build --release` lancé quand même (la build macOS ne bouge pas).
- CI : le run `release` en `workflow_dispatch` (tag fictif) sur la branche
  — à lancer par le mainteneur — doit montrer les six jobs verts, le pas
  `check` des quatre jobs Linux avec `GLIBC_2.17` (ou `static`) et
  l'install dans centos:7 / alpine ; le run `action` du push, le témoin
  négatif refusé et `install.sh` vert dans les deux conteneurs.

## Méta-analyse (sous-agent frais, plan et code)

Neuf points ; ce qui change le plan :

1. **(haute) Le conteneur ne regardait pas stderr.** Le symptôme x86_64 est
   un lancement *réussi* qui écrit « weak version … not found ». → `check`
   exige une stderr vide pour `--version` et l'absence de `GLIBC_` dans
   celles des autres lancements.
2. **(moyenne-haute) `objdump -T` liste des symboles, pas les versions
   requises** ; le chargeur vérifie chaque entrée de `.gnu.version_r`, et
   c'est elle qui porte le drapeau faible. → maximum pris dans « Version
   References » d'`objdump -p` (toutes les bibliothèques, `sort -V` :
   `2.2.5` < `2.17`), entrées au-dessus du plancher listées avec leurs
   drapeaux.
3. **(moyenne) Le Mac n'a ni `objcopy` ni `readelf` GNU.** →
   `OBJDUMP` paramétrable (l'`objdump` d'Apple est llvm-objdump et imprime
   le même format) ; l'`objcopy --strip-debug` du plan est abandonné (voir
   le modèle révisé).
4. **(moyenne) Le texte attendu du témoin négatif était faux** : sur 2.17
   le chargeur cite d'abord une version forte (2.18…2.34), avec un accent
   grave. → `grep -E "version .GLIBC_2\.[0-9]+' not found"`. Gardé :
   l'asset v0.25.0 (5 Mo, sha256 épinglé) plutôt qu'une seconde build gcc
   par run.
5. **(moyenne) musl : différences au-delà du JIT** — textes de `strerror`
   et `gai_strerror`, pas de NSS en statique ; deux noms d'asset de plus à
   garder. → gardé (coût mesuré nul, demande réelle : image `composer`,
   php:*-alpine), différences écrites au HANDOVER.
6. **(moyenne) Détection musl par `/lib/ld-musl-*`** : le paquet `musl`
   de Debian l'installe aussi. → `ldd --version` qui dit musl, sinon
   `/lib/libc.musl-<arch>.so.1` (propre à Alpine), sinon gnu.
7. **(moyenne) Alpine n'a pas curl**, et le test par répertoire local
   passe par `cp`. → `install.sh` dit « curl is required (apk add
   curl) » ; le test CI vérifie ce message, ajoute curl, refait par
   `file://`. Trouvé en écrivant ce test : la vérification `sha256sum -c
   --quiet` d'`install.sh` échoue avec le `sha256sum` de BusyBox (pas de
   `--quiet`) → sortie jetée au lieu de `--quiet`.
8. **(moyenne) La release dépend du réseau** (Packagist, GitHub, Docker
   Hub). → install de `fixtures/projects/scripts` (deux paquets) au lieu de
   Laravel, une nouvelle tentative après 15 s (un refus du chargeur ne
   disparaît pas au second essai).
9. **(basse-moyenne) pip** : hachages des quatre plateformes (Linux x86_64
   et aarch64, macOS arm64 et x86_64) dans le fichier ; `python3 -m venv`
   sur les runners n'est prouvé que par la CI.

Plus petits : clé `rust-cache` séparée pour les builds zig (release) et par
cible (installers) ; venv hors de `target/` sur un runner (rust-cache
restaurerait un venv dont le python a disparu de l'image suivante) ; le pas
« No dynamic PCRE2 » reste sur le chemin zig ; le job `action` teste un
binaire gcc (HANDOVER) ; ne pas toucher la ligne `composer:2.10.3`
d'action-test.yml (autre chantier) ; **le chemin `spawn` n'était pas
exercé** sans PHP dans l'image → `-vv` rendu à un faux `composer` (exit 7
relu).

## Le modèle, révisé

- **Table des symboles : on suit zig.** Garder `.symtab` demandait
  `strip = "none"` puis `objcopy --strip-debug` (binutils en CI,
  llvm-objcopy ou `zig objcopy` sur le Mac) : un outil de plus, différent
  selon l'hôte, pour des noms dans des backtraces que personne ne demande
  (vivacity imprime ses erreurs, pas de panique attendue). Les binaires
  Linux sont désormais entièrement strippés : 14,2 → 10,9 Mo (aarch64),
  15,3 → 12,4 Mo (x86_64). Noté au CHANGELOG et au HANDOVER.
- **`tools/linux-release.sh image <triple>`** imprime l'image épinglée,
  pour que le test d'`install.sh` d'action-test.yml n'en recopie pas le
  digest.
- **Smoke** : `fixtures/projects/scripts` (psr/log, psr/container ; sans
  `--run-scripts`, ses scripts ne tournent pas), trois étapes :
  `--version` à stderr vide, `install -vv` → faux composer, `install`.
- Le reste du plan tient.
