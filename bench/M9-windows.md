# M9 — vivacity contre Composer sous Windows, sur un runner GitHub (2026-10-29)

Question : la seule mesure externe de vivacity est celle de Luther Monson,
sous Windows, avec vivacity 0.17. Que donne la même mesure avec le
vivacity d'aujourd'hui (store v3, `parallel_io` vrai seulement sous
Linux), dans des conditions publiées et rejouables ? Plan et
méta-analyse : `docs/plans/bench-windows.md`. Banc : `bench/windows-bench.sh`,
workflow `.github/workflows/bench-windows.yml` (`workflow_dispatch`).

## La mesure de Luther (2026-10-14)

<https://luth.io/blog/2026/10/composer-without-php-vivacity/>. Ryzen 9
5950X, 128 Go, SSD ADATA LEGEND 860, Windows 11 Pro (build 26200),
Defender temps réel **actif**. Composer 2.10.3 sur PHP 8.3.33 (NTS x64)
contre `ephpm composer` (ePHPm v0.11.2, vivacity 0.17.0). Fixture
`fixtures/projects/laravel` (Laravel 13, 109 paquets), `--ignore-platform-reqs`
des deux côtés (le lock veut PHP 8.4.1), caches chauds des deux côtés, une
copie du projet par outil.

| scénario | Composer | vivacity 0.17 | rapport |
|---|---|---|---|
| install, `vendor/` supprimé | 107,3 s (110,0 / 99,5 / 107,3) | 13,9 s (14,7 / 13,9 / 13,4) | 7,7× |
| no-op (`install -o`) | 6,7 s (6,7 / 6,8) | 0,73 s (0,76 / 0,70) | 9,2× |
| `dump-autoload -o` | 6,5 s (6,7 / 6,4) | 0,38 s (0,30 / 0,46) | 17× |

Deux `vendor/` de 8 853 fichiers, sha256 identiques. Composer lancé en
`php composer.phar <cmd> --no-interaction --no-scripts -q` ; plugins
actifs ; horloge murale dans une boucle bash, 2 à 3 runs par cas, pas
d'hyperfine. L'article ne dit pas sur quel lecteur étaient le projet et
les caches, ni les drapeaux exacts passés à ePHPm.

## Méthode du banc

- **Mêmes drapeaux des deux côtés**, ceux de Luther : `--no-interaction
  --no-scripts -q --ignore-platform-reqs`, plugins actifs (Laravel :
  pest-plugin et php-http/discovery, émulés par vivacity) ; **plus
  `--no-blocking`** sur `install`, pour la raison de `bench/ci-bench.sh` :
  sans lui, l'install depuis le lock fait une requête conditionnelle aux
  listes de blocage de Packagist dont la latence est celle du réseau
  (DECISIONS 2026-09-19). Composer lancé en `php composer.phar`, comme
  Luther. vivacity est le binaire `vivacity`, pas ePHPm.
- **Scénarios** : `noop` (`install` sur un `vendor/` complet — Laravel
  a `optimize-autoloader: true`, c'est le `install -o` de Luther),
  `fresh` (`vendor/` supprimé avant chaque run, caches chauds ; le `warm`
  de `ci-bench.sh`), `dump-o`.
- **hyperfine** 2.0.0 (épinglé dans le workflow), une chauffe, au moins
  `runs` runs (5 par défaut) ; médiane, écart-type, min–max. `noop` et
  `dump-o` sous `-N` : pas de shell à démarrer, ce qui sous Windows coûte
  des dizaines de millisecondes variables. `fresh` passe par le bash de
  Git (`--shell`), parce que hyperfine 2.0 lance toujours `--prepare`
  dans un shell (`cmd.exe` par défaut sous Windows) et que la
  préparation supprime `vendor/` ; hyperfine retranche le démarrage du
  shell qu'il calibre, un bruit faible devant des installs de plusieurs
  secondes. Le banc lit les deux schémas JSON (hyperfine 1.x et 2.x).
- **Une copie par outil, un seul chemin absolu** : chaque `vendor/` est
  écrit par son outil, mais la copie est renommée en `<fixture>/app` le
  temps que son outil travaille. Vu sur le passage macOS : sur Sylius,
  rector/extension-installer écrit le chemin absolu de chaque extension
  dans `GeneratedConfig.php` ; deux copies côte à côte différaient par
  leur nom de répertoire seul.
- **Preuve d'égalité** avant les mesures (install `vendor/` supprimé, puis
  `dump-autoload -o`) et après (l'état laissé par hyperfine) : un manifeste
  par outil, une ligne par fichier (chemin, sha256) et par lien (cible),
  triés ; les deux doivent être identiques, sinon le job échoue sans
  mesurer. Le résumé donne le nombre d'entrées et la sha256 du manifeste.
  Et aucune ligne `vivacity: delegating to` dans la sortie de vivacity
  (sans `-q`) : un `vendor/` identique ne prouve pas que vivacity a
  travaillé. Ces trois arrêts ont été vus rouges (fichier modifié,
  délégation simulée, commande en échec sous hyperfine).
- **Conditions relevées** dans le résumé : CPU, cœurs, mémoire, version et
  build de Windows, `Get-MpComputerStatus` (temps réel, accès, IOAV,
  comportement, mode), `Get-MpPreference` (temps réel désactivé,
  exclusions), volumes (système de fichiers, taille), disques physiques,
  lecteur du projet, des caches et du cache de Composer.
- **Deux dispositions**, en matrice :
  - `split` : projet sous `RUNNER_TEMP` (`D:`), caches sous
    `%LOCALAPPDATA%` (`C:`). C'est ce que voit tout utilisateur de
    vivacity en CI Windows. Store et `vendor/` sur deux volumes : le lien
    dur échoue, vivacity **copie** chaque fichier (`clone.rs`, un essai de
    lien par fichier avant la copie).
  - `same` : projet et caches sous `%LOCALAPPDATA%` (`C:`), la disposition
    d'un poste à un seul disque, probablement celle de Luther : liens durs.

## Ce qui sépare ce banc de la mesure de Luther

| | Luther | ce banc |
|---|---|---|
| machine | Ryzen 9 5950X (16 cœurs), 128 Go, SSD NVMe grand public | runner `windows-latest` partagé (relevé dans le résumé), disques Azure |
| Windows | 11 Pro, build 26200 | Windows Server de l'image du runner (relevé) |
| Defender | temps réel actif | relevé : l'état de l'image GitHub, peut-être désactivé ou à exclusions |
| PHP | 8.3.33 | 8.4 (setup-php) |
| vivacity | 0.17.0 dans ePHPm v0.11.2 | binaire `vivacity` du commit mesuré |
| no-op | requête des listes de blocage comprise | `--no-blocking` des deux côtés |
| plugins | actifs | actifs |
| runs | 2–3, boucle bash | hyperfine, 1 chauffe + au moins 5 |
| projet / caches | lecteur non précisé | `split` (D:/C:) et `same` (C:) |

Conséquences pour la lecture :

- **Les secondes ne se comparent pas** d'une machine à l'autre ; les
  **rapports** Composer/vivacity, oui, à condition que les deux côtés
  dépendent de la même chose. Le no-op de Luther contient une requête
  réseau que le nôtre n'a pas : ses deux colonnes sont plus longues que
  les nôtres d'une même latence, son rapport est tiré vers 1.
- **Defender est la variable la plus lourde** : il inspecte chaque
  fichier créé, et une install en crée 8 853 (Laravel) à 40 000 (Sylius).
  Un runner sans temps réel mesure un autre régime que le poste de Luther ;
  il faut lire l'état relevé avant tout chiffre.
- **Le store a changé depuis 0.17** : comparer `fresh` de Luther (13,9 s)
  au nôtre mélange machine, Defender, disposition et code. Seul le rapport
  dans un même run isole vivacity de la machine.

## Passage de contrôle sur macOS (pas Windows)

Le même script, M4 Max (14 cœurs, APFS, macOS 26.6.2), PHP 8.5.12,
Composer 2.10.3, vivacity 0.25.0 (2d8c1cc), `BENCH_RUNS=2`, cache de
Composer existant, machine partagée avec d'autres sessions : il dit que
le banc tourne, prouve et mesure, pas ce que vaut vivacity sous Windows.

| fixture (paquets) | scénario | Composer | vivacity | rapport |
|---|---|---|---|---|
| laravel (109) | noop | 1 794 ms (σ 141) | 68 ms (σ 15) | 26× |
| laravel (109) | fresh | 3 003 ms (σ 13) | 191 ms (σ 40) | 16× |
| laravel (109) | dump-o | 1 064 ms (σ 166) | 44 ms (σ 4) | 24× |
| symfony (153) | noop | 442 ms (σ 38) | 51 ms (σ 15) | 8,7× |
| symfony (153) | fresh | 3 986 ms (σ 1 307) | 266 ms (σ 29) | 15× |
| symfony (153) | dump-o | 1 246 ms (σ 410) | 62 ms (σ 5) | 20× |
| sylius (276) | noop | 659 ms (σ 324) | 94 ms (σ 18) | 7,0× |
| sylius (276) | fresh | 8 581 ms (σ 595) | 896 ms (σ 47) | 9,6× |
| sylius (276) | dump-o | 2 940 ms (σ 82) | 115 ms (σ 44) | 26× |

Arbres identiques aux trois étapes : 8 845 entrées pour Laravel (contre
8 853 sous Windows chez Luther : Composer y ajoute un proxy `.bat` à
chacun des 7 binaires du lock ; la huitième entrée est à lire sur le run
Windows), 11 425 pour Symfony, 40 584 pour Sylius. Le no-op Composer de
Laravel a varié de 825 à 1 794 ms entre deux passages : deux runs sur une
machine partagée, une indication.

## Résultats Windows — run 41190108069 (vivacity 0.26.0, f1faa45, 2026-10-31)

Composer 2.10.3, PHP 8.4.27, hyperfine 2.0.0, une chauffe et au moins 5
runs par scénario. Runner `windows-latest` : Windows Server 2025
Datacenter (build 26100), AMD EPYC 7763, 2 cœurs / 4 threads.
**Defender : temps réel désactivé** (`RealTimeProtectionEnabled` faux,
exclusions `C:\` et `D:\`) : l'image GitHub ne mesure pas le régime du
poste de Luther. Temps en ms : médiane (σ, min–max).

### `split` (projet sur D:, caches sur C:)

| fixture (paquets) | scénario | Composer | vivacity | Composer / vivacity |
|---|---|---|---|---|
| laravel (109) | noop | 4 485 (14, 4 464–4 497) | 207 (18, 197–254) | 21,7× |
| laravel (109) | fresh | 10 984 (105, 10 783–11 066) | 4 457 (317, 4 200–4 958) | 2,5× |
| laravel (109) | dump-o | 4 405 (6, 4 396–4 413) | 143 (8, 136–161) | 30,8× |
| symfony (153) | noop | 3 165 (87, 3 074–3 265) | 185 (5, 179–199) | 17,1× |
| symfony (153) | fresh | 11 522 (141, 11 398–11 749) | 5 311 (17, 5 284–5 324) | 2,2× |
| symfony (153) | dump-o | 5 136 (27, 5 128–5 194) | 183 (5, 181–195) | 28,1× |
| sylius (276) | noop | 3 130 (71, 3 109–3 279) | 323 (8, 319–342) | 9,7× |
| sylius (276) | fresh | 27 549 (197, 27 237–27 707) | 19 320 (88, 19 270–19 460) | 1,4× |
| sylius (276) | dump-o | 12 160 (54, 12 113–12 255) | 391 (4, 383–394) | 31,1× |

### `same` (tout sur C:)

| fixture (paquets) | scénario | Composer | vivacity | Composer / vivacity |
|---|---|---|---|---|
| laravel (109) | noop | 5 080 (138, 4 886–5 225) | 227 (8, 217–248) | 22,4× |
| laravel (109) | fresh | 16 811 (6 613, 16 223–28 864) | 8 965 (689, 7 958–9 872) | 1,9× |
| laravel (109) | dump-o | 4 875 (45, 4 850–4 965) | 152 (2, 149–157) | 32,1× |
| symfony (153) | noop | 3 259 (14, 3 241–3 273) | 208 (6, 199–221) | 15,7× |
| symfony (153) | fresh | 18 082 (261, 17 912–18 580) | 13 779 (692, 12 952–14 805) | 1,3× |
| symfony (153) | dump-o | 5 716 (180, 5 696–6 114) | 210 (3, 205–215) | 27,2× |
| sylius (276) | noop | 3 317 (25, 3 301–3 361) | 387 (11, 372–405) | 8,6× |
| sylius (276) | fresh | 59 506 (1 254, 57 757–61 008) | 51 776 (1 448, 49 815–53 748) | 1,1× |
| sylius (276) | dump-o | 16 117 (99, 16 072–16 324) | 472 (7, 462–481) | 34,1× |

Arbres identiques, Composer contre vivacity, aux trois étapes (install,
`dump-autoload -o`, état laissé par hyperfine) dans les deux
dispositions : 8 852 entrées pour Laravel, 11 434 pour Symfony, 40 601
pour Sylius. Aucune ligne `delegating to` côté vivacity.

Un passage antérieur (run 40938579978, 9b69185, 2026-10-29, Intel Xeon
Platinum 8573C) donnait les mêmes ordres de grandeur : noop 8,8× à
22,2×, `dump-o` 24,5× à 37,2×, fresh 1,0× à 2,6×.

### Lecture

- **noop et `dump-autoload -o`** : 8,6× à 22× et 27× à 34×, au niveau ou
  au-dessus des 9,2× et 17× de Luther, sans la requête des listes de
  blocage (`--no-blocking`) que contenait son no-op.
- **Install, `vendor/` supprimé** : 1,4× à 2,5× en `split`, 1,1× à 1,9×
  en `same`, contre 7,7× chez Luther. Sur ce runner, l'écriture de 9 000
  à 40 000 fichiers domine les deux outils ; vivacity n'est jamais plus
  lent, mais sur Sylius en `same` l'écart tombe à 1,1× (1,0× le 29/10).
- **C: est le disque lent du runner** : Composer y perd autant que
  vivacity (Laravel `fresh` : 11,0 s sur D:, 16,8 s sur C:). L'écart
  `split` / `same` mêle donc deux effets, copie contre lien dur et D:
  contre C: ; seul `same` sur D: (piste ci-dessous) les séparerait.
- **8 852 entrées contre 8 853 chez Luther** pour Laravel : 8 845 sous
  macOS plus les 7 proxies `.bat` de Composer font 8 852 ; la 8 853e de
  son `vendor/` n'est pas expliquée.
- Ce runner n'est pas le poste de Luther (2 cœurs contre 16, Defender
  coupé, autre disque) : les secondes ne se comparent pas, les rapports
  d'un même run, oui.

## Pistes ouvertes

- **Defender actif sur le runner** : une entrée du workflow qui active le
  temps réel (`Set-MpPreference -DisableRealtimeMonitoring $false`, si
  l'image le permet) rapprocherait le banc du poste de Luther ; non fait
  ici (on relève l'état, on ne le change pas).
- **`VIVACITY_PARALLEL_IO=1` sous Windows** : `parallel_io` n'est vrai que
  sous Linux, décidé sur une mesure APFS (DECISIONS 2026-09-15) ; NTFS
  n'a jamais été mesuré. Un troisième bras du banc le dirait.
- **La copie inter-volumes** (`split`) essaie un lien dur par fichier
  avant de copier ; le premier refus pourrait suffire pour tout le run,
  comme `reflink` le fait déjà sous Linux. Le chiffre `split` / `same`
  dira si c'est mesurable.
- **`same` sur D:** : séparerait l'effet du lien dur de celui du disque
  (C: et D: n'ont pas les mêmes performances sur les runners).
- **ePHPm** lui-même, pour le démarrage qu'il ajoute.
