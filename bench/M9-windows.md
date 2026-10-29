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

## Résultats Windows — à remplir depuis le run N

Lancer : `gh workflow run bench-windows.yml --ref <branche> -f runs=5 -f fixtures=laravel`
(ajouter `symfony sylius` si le temps le permet). Recopier ici, pour chaque
disposition, le tableau des temps, le tableau des manifestes et le bloc
des conditions du résumé du job, avec le numéro du run et le commit.

### `split` (projet sur D:, caches sur C:) — run _à remplir_

_à remplir_

### `same` (tout sur C:) — run _à remplir_

_à remplir_

### Lecture — _à écrire avec les chiffres_

À dire au minimum : l'état de Defender relevé ; le rapport par scénario
face à celui de Luther (7,7× / 9,2× / 17×) ; l'écart `split` / `same`
sur `fresh` (copie contre lien dur) ; le nombre d'entrées de `vendor/`
contre les 8 853 de Luther.

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
