# Les enregistrements de classmap dans le cache de l'action — plan

> Niveau 1 tant que rien ne change dans `action.yml` (mesure, banc,
> workflow `workflow_dispatch`) ; niveau 2 si la mesure conduit à modifier
> l'action (contrat public). Point ouvert du HANDOVER (« les
> enregistrements de classmap hors du cache ») et non-objectif de
> `docs/plans/action-cache.md`.

## La question

L'action ne met en cache que le répertoire de cache de Composer. Faut-il y
ajouter les enregistrements de classmap de vivacity (`<cache vivacity>/classmap/`
et `classmap-files/`) ? Situation à mesurer : un runner CI froid —
checkout neuf, `vendor/` absent, cache de Composer restauré, pas de store.

## Ce que dit le code (avant toute mesure)

- **`classmap/`** (`CacheSlot`, `crates/vivacity-autoload/src/classmap.rs`) :
  clé = `sha1(CACHE_FORMAT "\0" chemin de l'entrée de store "\0" sous-répertoire)`.
  Le chemin de l'entrée est `Store::entry_path` : `<cache vivacity>/store/<LAYOUT>/<vendor>/<pkg>/<version>-<12 premiers caractères de la référence>`.
  Il ne dépend que du cache de vivacity, de `LAYOUT` et du lock — **pas du
  contenu ni de la date de l'entrée**. Un store reconstruit depuis les zips
  sur un runner neuf redonne exactement les mêmes chemins, donc les mêmes
  clés : un enregistrement restauré **peut** servir. Conditions :
  1. l'entrée existe au moment du dump (`generator.rs` : `entry.is_dir()`) —
     l'install reconstruit le store avant le dump, c'est le cas ;
  2. le répertoire de cache de vivacity est au même chemin absolu d'un run
     à l'autre (runners hébergés : même `HOME` / `LOCALAPPDATA`) ;
  3. `LAYOUT` et `CACHE_FORMAT` inchangés : sinon toutes les clés changent,
     les anciens enregistrements ne servent plus jamais et, la clé primaire
     d'`actions/cache` étant trouvée, rien n'est ressauvé — le piège exact
     du store v2 restauré sous v3. La clé de cache doit donc porter la
     version de vivacity.
- **`classmap-files/`** (`FileCacheSlot`) : un enregistrement par répertoire
  hors store (sources du projet, paquets `path`), servi fichier par fichier
  si `(mtime ns, taille)` sont inchangés. Un checkout neuf donne des mtimes
  neufs : **aucun fichier ne peut servir**. Rien à gagner, à exclure.
- Spike (M4 Max, Laravel `-o`, store et `vendor/` supprimés, sources
  `touch`ées, `classmap/` gardé) : 0 enregistrement de `classmap/` réécrit
  sur 146 (tous servis), 4 sur 4 de `classmap-files/` réécrits (aucun
  servi). `VIVACITY_TRACE=1` : `dump scan` 24 ms avec les enregistrements,
  1 088 ms sans. La prémisse « ils ne peuvent pas servir après une
  reconstruction du store » est donc fausse ; reste à chiffrer le gain.

## Plan

1. **`bench/classmap-cache.sh`** (bash, Linux / macOS / Git Bash Windows) :
   pour Laravel et Sylius, `optimize-autoloader` faux puis vrai (posé dans
   `config` de la copie : hors du `content-hash`), un cache vivacity par
   cas ; préchauffe (zips par le réseau, une fois), puis un install pour
   produire les enregistrements, gardés à part. hyperfine, warm-up 1,
   `--min-runs` N, deux commandes identiques qui ne diffèrent que par la
   préparation :
   - *zips seuls* : `rm -rf vendor store classmap classmap-files platform-probe.json`, `touch` des sources ;
   - *zips + classmap* : la même chose, puis `classmap/` recopié du
     snapshot.
   Plus le coût de l'artefact en plus : taille, `tar | zstd -3` produit et
   restauré (hyperfine), comme `bench/ci-cache.sh`. Sortie : tableau
   Markdown (médiane, écart-type, min/max), ajouté au résumé du job.
2. **Mesure locale** : macOS (M4 Max, APFS) et conteneur Linux (docker,
   ext4 de la VM Docker Desktop — indication, pas un runner).
3. **Workflow `classmap-cache.yml`** (`workflow_dispatch` seul, trois OS)
   qui construit le binaire et lance le banc, si le résultat local laisse
   une question que seuls les trois OS tranchent (Windows : store sur `C:`,
   projet sur `D:`, copie au lieu de clonage ; NTFS).
4. **Décision** :
   - gain d'install (médiane zips seuls − zips + classmap) supérieur au
     coût de restauration en plus et hors du bruit (écart > 2 écarts-types
     combinés) sur Linux au moins → modifier `action.yml` (niveau 2 :
     méta par sous-agent frais, revue indépendante) ;
   - sinon résultat négatif : bench + DECISIONS, code de l'action inchangé.
5. **Si l'action change** — forme envisagée, à passer en méta :
   - un seul `actions/cache` (pas un second aller-retour réseau), `path`
     sur deux lignes : le cache de Composer et `<cache vivacity>/classmap` ;
   - clé `vivacity-<os>-<arch>-<version vivacity>-<hash des locks>`,
     restore-key `vivacity-<os>-<arch>-` : une montée de version fait un
     échec de clé primaire, donc une nouvelle sauvegarde (le piège
     `LAYOUT`) ;
   - le répertoire de cache de vivacity : voir « Révision » ci-dessous ;
   - `action-test.yml` prouve que les enregistrements servent : après
     restauration, `store/` supprimé, l'install réécrit 0 enregistrement.

## Révision après la mesure macOS (2026-10-29)

- **Linux local impossible** : le disque de ce Mac est tombé plein pendant
  le banc (≈ 120 Mo libres, d'autres sessions écrivent en parallèle) et le
  démon Docker répond depuis par des erreurs d'E/S sur son magasin
  containerd. Pas de chiffre Linux local ; la règle « Linux au moins » ne
  peut être appliquée que par le workflow `classmap-cache` (trois OS).
- **macOS mesuré** (tableau dans `bench/M8-classmap-cache.md`) : gain net,
  bien au-delà du bruit, avec `-o` ; coût de restauration de l'ordre de
  10-40 ms.
- **Proposition soumise à la méta** : deux commits, le second changeant
  l'action sur la foi de la mesure macOS et d'un argument de structure ; le
  répertoire de cache de vivacity recopié en bash (`platform::cache_dir`),
  prouvé par `action-test.yml`. **Abandonnée** après la méta (ci-dessous).

## Méta-analyse (sous-agent frais, plan et code, 2026-10-29)

Quinze points ; ceux qui changent le plan :

1. **(haute) Un enregistrement restauré peut être servi contre une entrée
   de store au contenu différent, sans contrôle.** La clé ne porte ni l'URL
   de la dist, ni les octets de l'archive, ni les règles d'extraction ; en
   CI le store est réextrait par le binaire du jour depuis le zip du jour,
   la restore-key ramène des enregistrements d'un autre lock ou d'un autre
   vivacity. Cas réels : même référence servie par un miroir qui applique
   `archive.exclude` ; règles d'extraction changées sans bump de `LAYOUT`
   (7e53145, ed02f07, vérifiés : `extract.rs`, 07/10, entre 29cb00e et
   05193b7). Localement l'entrée et son enregistrement vieillissent
   ensemble. → **décision : l'action ne change pas.**
2. **(haute) Le banc ne tournait pas sous Windows** : `cygpath -w` donne
   `C:\Program Files\…`, que hyperfine découpe comme un shell (espace,
   contre-obliques) ; sorties jetées sous `set -e`. → chemin en barres
   obliques, entre guillemets (vu rouge sans guillemets, vert avec, sur un
   chemin à espace) ; stderr gardée.
3. **(moyenne) Windows ne mesurait pas le cas réel** : tout sous
   `RUNNER_TEMP` (`D:`). → caches sous `%LOCALAPPDATA%` (`C:`), projet sous
   `RUNNER_TEMP`.
4. **(moyenne) « Servis » compte aussi ce qui n'a pas été utilisé.** →
   documenté : la colonne ne vaut qu'avec la chute de `dump scan` ; le banc
   s'arrête si aucun enregistrement n'est écrit ou si l'install est rendue à
   Composer (deux contrôles vus rouges).
5. **(moyenne) Le test proposé pour `action-test.yml` aurait été rouge par
   construction** (premier run, changement de lock) : sans objet, l'action
   ne change pas.
6. **(moyenne) Règle de décision affaiblie après coup** (« Linux au moins »
   → macOS) : retenu, c'est l'autre raison de ne pas changer l'action.
7. **(moyenne) `classmap-files/` sert à qui restaure `vendor/` lui-même**
   (tar garde les mtimes) : hors de l'action, noté au bench.
8-11. **(moyenne/basse)** recopie bash de `platform::cache_dir` (valeurs
   vides, `LOCALAPPDATA`, relatif) ; jobs en matrice qui partagent la clé ;
   chaîne de version partagée par les builds de dev ; croissance sans
   élagage et `*.tmp` orphelins : conditions d'une future inclusion,
   consignées au bench.
12. **(basse, bug latent, vérifié)** le générateur cherche l'entrée par
   `dist_reference()`, l'installeur l'écrit par `store_reference()` : une
   dist sans référence n'a jamais d'enregistrement de store. Hors de ce
   changement (HANDOVER).
13-15. Chemin manquant à la sauvegarde ; README ligne 52 jamais mesurée pour
   les enregistrements (M7 mesurait store et enregistrements ensemble) →
   corrigée ; bras hyperfine non entrelacés, trace sur un run → indiqué.

## Exécution (2026-10-29)

- Code de l'action inchangé (`action.yml`, `action-test.yml`).
- `bench/classmap-cache.sh` et `.github/workflows/classmap-cache.yml`
  (`workflow_dispatch`, trois OS) ; `actionlint` vert, vu rouge sur une
  copie cassée ; `shellcheck` vert.
- Mesure macOS : `bench/M8-classmap-cache.md`. Linux local : impossible
  (disque plein, démon Docker en erreur d'E/S).
- Docs : README (la phrase non mesurée), DECISIONS, HANDOVER, CHANGELOG,
  bench M8, ce plan.
6. **Docs** : bench (fichier daté), DECISIONS (2026-10-29), HANDOVER (point
   ouvert fermé ou mis à jour), CHANGELOG (Unreleased), README si l'action
   change, ce plan.

## Non-objectifs

- `classmap-files/` (prouvé inutile sur un checkout neuf, ci-dessus).
- Le store (`bench/M7-ci-cache.md`, DECISIONS 2026-10-29).
- `platform-probe.json` (~40 ms, clé sur l'inode du binaire php, que
  setup-php peut réinstaller) : supprimé dans les deux bras du banc.
- Toute modification du code Rust de la classmap.

## Auto-checklist initiale (avant la mesure)

- **Le banc mesure-t-il le runner froid ?** Page cache chaud en local : les
  zips et le store fraîchement écrits sont en mémoire, sur le runner aussi
  (le store vient d'être extrait dans le même job). Les sources sont
  `touch`ées comme un checkout. Le `vendor/` et le store sont supprimés
  dans les deux bras. Biais restant : la restauration `actions/cache`
  réelle (téléchargement) n'est pas dans la boucle ; on mesure le surcoût
  d'extraction de `classmap/` à part, le téléchargement de ~1 Mo en plus
  dans un artefact de ~90 Mo est sous le bruit de M7.
- **Même chemin absolu** : le snapshot est restauré au même
  `VIVACITY_CACHE_DIR`, comme le `HOME` stable d'un runner. Un runner où ce
  chemin changerait (auto-hébergé éphémère à home variable) ne verrait
  aucun gain : à documenter, pas à mesurer.
- **Hypothèse que le gain vient du scan** : vérifiée par la trace et par le
  compte des enregistrements réécrits, pas supposée.
- **Sur-ingénierie** : un banc, un workflow manuel ; pas de nouvelle
  sous-commande tant que la mesure ne l'exige pas.
- **Windows** : Git Bash, `jq`, `tar` présents sur windows-latest ;
  hyperfine et zstd via une action d'installation. Le banc doit tourner
  là aussi, sinon le troisième OS reste non mesuré : à voir rouge en local
  sur un cas cassé avant de compter sur son vert.
