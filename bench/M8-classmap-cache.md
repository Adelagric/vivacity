# M8 — Les enregistrements de classmap dans le cache CI (2026-10-29)

Question : l'action (`action.yml`), qui ne met en cache que le répertoire
de cache de Composer, doit-elle y ajouter les enregistrements de classmap
de vivacity (`<cache vivacity>/classmap/`, `classmap-files/`) ? Plan et
méta-analyse : `docs/plans/classmap-cache.md`. Banc : `bench/classmap-cache.sh`
(trois OS : `.github/workflows/classmap-cache.yml`, `workflow_dispatch`).

## Ce que dit le code

- `classmap/` (`CacheSlot`, `crates/vivacity-autoload/src/classmap.rs`) :
  clé = `sha1(CACHE_FORMAT, chemin de l'entrée de store, sous-répertoire)`,
  le chemin étant `<cache vivacity>/store/<LAYOUT>/<vendor>/<pkg>/<version>-<12 caractères de la référence>`
  (`Store::entry_path`). Rien du contenu ni de la date de l'entrée : un
  store reconstruit depuis les zips sur un runner neuf redonne les mêmes
  chemins, donc les mêmes clés. **Un enregistrement restauré sert** — si le
  cache de vivacity est au même chemin absolu (runners hébergés : oui) et
  que `LAYOUT` et `CACHE_FORMAT` n'ont pas bougé. La prémisse « il ne peut
  pas servir après une reconstruction du store » est fausse.
- `classmap-files/` (`FileCacheSlot`) : sources du projet et tout chemin
  hors store, servis fichier par fichier sur `(mtime ns, taille)`. Un
  checkout neuf donne des mtimes neufs : rien ne sert (mesuré : 4 sur 4
  réécrits sur Laravel). Exception notée par la méta-analyse, hors du
  périmètre de l'action : un projet qui restaure lui-même `vendor/` (tar
  garde les mtimes) passe tous ses paquets par `classmap-files/`.

Preuve que la clé tient au chemin et à lui seul : les mêmes 117
enregistrements (Laravel `-o`) restaurés sous un autre `VIVACITY_CACHE_DIR`
ne servent à rien (117 nouveaux écrits, les 117 restaurés intacts) ; au
même chemin, 117 sur 117 servis.

## Mesure (macOS)

M4 Max, 14 cœurs, APFS, macOS 26.6.2 ; vivacity 0.25.0 (5768dc9, `--release`) ;
`bench/classmap-cache.sh`, hyperfine 10 runs + 1 de chauffe par bras.
Runner froid simulé : `vendor/`, store, `classmap/`, `classmap-files/` et
`platform-probe.json` supprimés, zips de Composer chauds, sources
`touch`ées ; le bras « + classmap » recopie les enregistrements d'un install
précédent, au même chemin. Machine partagée avec d'autres sessions (disque
tombé plein pendant la première passe, refaite) : écarts-types élevés.

| fixture | autoloader | zips seuls | zips + classmap | gain (médianes) | `dump scan` (seuls → + classmap) | enregistrements servis | `classmap/` | restauration (zstd \| tar) |
|---|---|---|---|---|---|---|---|---|
| laravel (109) | normal | 1 528 (σ 349, 1 290–2 538) | 1 110 (σ 329, 966–2 098) | 418 | 212 → 8 | 32/32 | 244 Ko | 14 ms |
| laravel | `-o` | 2 384 (σ 208, 2 210–2 965) | 1 026 (σ 65, 975–1 173) | **1 358** | 1 940 → 19 | 117/117 | 968 Ko | 42 ms |
| sylius (276) | normal | 5 451 (σ 1 666, 4 044–9 917) | 4 336 (σ 462, 4 075–5 373) | 1 115 | 167 → 22 | 34/34 | 236 Ko | 15 ms |
| sylius | `-o` | 8 178 (σ 1 036, 7 037–10 340) | 5 382 (σ 2 297, 4 254–12 212) | **2 796** | 4 175 → 123 | 301/301 | 2,9 Mo | 110 ms |

Temps en ms : médiane (écart-type, min–max). `dump scan` : un run tracé
par bras (`VIVACITY_TRACE=1`), une indication.

Lecture.

- **Avec `-o`, le gain est net** : 1,4 s sur Laravel (hors du bruit, σ
  ≤ 0,21 s), 2,8 s sur Sylius (min à min : 7,0 → 4,3 s), porté par la
  phase de scan (1,9 s → 0,02 s ; 4,2 s → 0,1 s). Coût en plus dans
  l'artefact : ≤ 3 Mo et 14-110 ms d'extraction, sans aller-retour réseau
  si c'est la même entrée de cache.
- **Sans `-o`, le gain est petit** : ce qui est scanné se limite aux
  `classmap` des paquets (stubs des polyfills, nette) ; ~0,2 s sur Laravel
  (le scan), et le 1,1 s de Sylius est du bruit (scan 0,15 s, min à min
  0,03 s).
- **APFS lit en séquentiel** (`parallel_io` faux hors Linux, DECISIONS
  2026-09-15) : ce scan de 2-4 s est celui d'un Mac ; sous Linux il est
  parallèle et devrait être bien plus court. Pas de chiffre Linux local :
  le démon Docker de ce Mac répond par des erreurs d'E/S depuis que le
  disque est tombé plein. Le workflow `classmap-cache` (trois OS) le
  donne ; Windows y garde le store sur `C:` et le projet sur `D:`, comme un
  vrai runner.

## Verdict : l'action reste inchangée — pour une raison de correction

Le gain existe, mais un enregistrement restauré en CI peut décrire
d'autres fichiers que ceux du store qui vient d'être reconstruit, sans que
rien ne le voie (méta-analyse, point 1). Sa clé ne porte que nom, version,
12 caractères de la référence, `LAYOUT` et `CACHE_FORMAT` ; le store, lui,
est réextrait par le binaire du jour depuis le zip du jour. Sur un poste,
l'entrée et son enregistrement vieillissent ensemble ; en CI, la
restore-key ramène des enregistrements écrits par un autre lock ou un
autre vivacity :

- **même référence, autre archive** : l'URL passe de GitHub à un miroir
  (Private Packagist, Satis) qui applique `archive.exclude` — le cache de
  Composer est clé sur l'URL, l'enregistrement non ;
- **règles d'extraction changées sans bump de `LAYOUT`** : c'est arrivé
  (7e53145 et ed02f07, noms Windows, entre 29cb00e et 05193b7).

Résultat possible : une classmap qui liste des fichiers absents ou en
oublie, dans un `install --no-dev -o` qui fabrique un artefact de
déploiement. Ce risque n'existe pas aujourd'hui ; un gain de 1-3 s ne le
paie pas.

## Ce qu'il faudrait pour les mettre en cache

1. **Cœur** : une clé d'enregistrement qui porte l'identité de l'archive
   (l'URL de la dist, comme le cache de Composer) et des règles
   d'extraction ; bump de `CACHE_FORMAT`.
2. **Action** : une clé liée au binaire (condensat du binaire, pas la
   chaîne `vivacity 0.25.0` que partagent les builds de dev), sans
   restore-key qui traverse les versions ; une entrée de cache séparée
   éviterait de ressauver les zips à chaque montée de version, au prix
   d'un aller-retour réseau à mesurer.
3. **Les jobs en matrice** partagent une clé : le premier à sauver fixe le
   jeu d'enregistrements (`-o` ou non, `--no-dev`) jusqu'au changement de
   lock — à documenter ou à mettre dans la clé.
4. **Le chiffre Linux et Windows** du workflow `classmap-cache`, qui dit si
   le gain vaut ce chantier là où tournent la plupart des CI.

Relevé en passant (méta-analyse, point 12, vérifié dans le code) : le
générateur cherche l'entrée de store par `dist_reference()`
(`generator.rs`), l'installeur l'écrit par `store_reference()` (repli sur
le sha1 de l'URL quand la dist n'a pas de référence, `lock.rs`). Une dist
sans référence (tar d'asset-packagist) n'a donc jamais d'enregistrement de
store : elle est rescannée à chaque install. Non corrigé ici.

## Le run trois OS du 2026-10-29 (run 40915941883) : temps nuls

Le workflow `classmap-cache` (commit c9d20fb, `BENCH_RUNS` 10) est passé
vert sur ubuntu, macOS et Windows, mais **ses colonnes de temps sont
vides et doivent être tenues pour nulles** : « zips seuls », « zips +
classmap », le gain (0 partout, une soustraction de deux vides) et la
restauration. `taiki-e/install-action` avait installé hyperfine 2.0.0
(sorti le 2026-10-07), dont le JSON (schéma 2) range le nom de `-n` dans
`name` et les statistiques sous `summary.time_wall_clock` ; le script
cherchait `command` et `median` (schéma 1), ne trouvait rien, et
n'échouait pas. Les mesures elles-mêmes ont tourné (bash passé par
`--shell`, préparation comprise), mais leurs JSON n'étaient ni imprimés
ni téléversés : les temps sont perdus.

Corrigé (même jour) : lecture des deux schémas, un résultat illisible ou
absent arrête le banc (vu rouge sur un JSON au schéma inconnu ; le script
d'avant, rejoué sur un JSON au schéma 2, ressort exactement les cellules
vides et le gain 0 du run, en vert), hyperfine épinglé à 2.0.0 dans le
workflow. Les trois chiffres restent à reprendre en relançant le workflow.

Ce qui reste lisible dans ce run ne vient pas de hyperfine : un run tracé
par bras (`VIVACITY_TRACE=1`) pour `dump scan`, le compte des
enregistrements et la taille de `classmap/`. Une indication, un run par
case :

| OS (runner) | fixture | autoloader | `dump scan` (seuls → + classmap) | servis | `classmap/` |
|---|---|---|---|---|---|
| ubuntu, 4 cœurs | laravel | normal | 28 → 1 ms | 32/32 | 248 Ko |
| ubuntu | laravel | `-o` | 156 → 3 ms | 117/117 | 980 Ko |
| ubuntu | sylius | normal | 29 → 1 ms | 34/34 | 240 Ko |
| ubuntu | sylius | `-o` | 447 → 8 ms | 301/301 | 2 948 Ko |
| macOS arm64, 3 cœurs | laravel | normal | 332 → 15 ms | 32/32 | 244 Ko |
| macOS | laravel | `-o` | 1 502 → 13 ms | 117/117 | 968 Ko |
| macOS | sylius | normal | 474 → 8 ms | 34/34 | 236 Ko |
| macOS | sylius | `-o` | 6 963 → 163 ms | 301/301 | 2 924 Ko |
| Windows (build 26100), 4 cœurs | laravel | normal | 763 → 6 ms | 32/32 | 204 Ko |
| Windows | laravel | `-o` | 1 090 → 30 ms | 117/117 | 873 Ko |
| Windows | sylius | normal | 651 → 7 ms | 34/34 | 182 Ko |
| Windows | sylius | `-o` | 3 626 → 607 ms | 301/301 | 2 793 Ko |

Lu avec prudence (un run, pas de dispersion) : sous Linux le scan que
les enregistrements évitent est court (0,16 s Laravel `-o`, 0,45 s Sylius
`-o` — le scan parallèle de `parallel_io`), bien plus que sur le runner
macOS (1,5 s et 7,0 s) ou Windows (1,1 s et 3,6 s). Ce n'est pas le gain
d'install, que seules les colonnes de temps diraient.
