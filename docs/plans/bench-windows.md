# Banc Windows dans GitHub Actions — plan

> Niveau 1 : un banc, un workflow `workflow_dispatch`, de la doc. Rien du
> binaire ni d'un contrat public ne bouge ; tout se défait par un revert.
> Méta-analyse en auto-checklist, auto-revue.

## Le problème

La seule mesure externe de vivacity est celle de Luther Monson
(<https://luth.io/blog/2026/10/composer-without-php-vivacity/>, 2026-10-14) :
Windows 11 Pro, Ryzen 9 5950X, Defender temps réel actif, Composer 2.10.3
sur PHP 8.3.33 contre ePHPm v0.11.2 qui embarque vivacity 0.17.0, fixture
Laravel (109 paquets), `--ignore-platform-reqs` des deux côtés, caches
chauds. Médianes : install vendor supprimé 107,3 s contre 13,9 s ; no-op
6,7 s contre 0,73 s ; `dump-autoload -o` 6,5 s contre 0,38 s. Deux `vendor/`
de 8 853 fichiers, sha256 identiques.

Ce que l'article dit de sa méthode (relu le 2026-10-29) : Composer lancé en
`php composer.phar <cmd> --no-interaction --no-scripts -q` ; plugins **non**
désactivés ; une copie du projet par outil ; boucle bash, horloge murale ;
3 installs « fresh », 2 no-op (`install -o`), 2 dumps par outil ; pas
d'hyperfine ; lecteur du projet et des caches non précisés ; drapeaux
exacts d'ePHPm non précisés.

Depuis 0.17, la matérialisation parallèle et le store ont changé (store
v3, `parallel_io` vrai seulement sous Linux). Aucun chiffre Windows de
vivacity d'aujourd'hui n'existe ; `bench.yml` ne tourne que sur
ubuntu-latest.

## Contraintes

- Pas de Windows ici : le script doit tourner sous Git Bash sur le runner
  (comme `windows.yml` et `bench/classmap-cache.sh`), et sur macOS pour un
  passage de contrôle.
- Épinglage `tools: composer:2.10.3`, sous la même forme que les autres
  workflows (ré-épinglé avec eux vers 2.11.0 par l'autre chantier).
- Conventions CI : `permissions: contents: read`, `COMPOSER_AUTH` au niveau
  du job avec le jeton du job, appels réseau sous `harness/net-retry.sh`.
- Disque du Mac serré : sorties sous le scratchpad, nettoyées.

## Non-objectifs

- Pas de gate : rien n'échoue sur un ratio (bench.yml l'a appris à ses
  dépens, six fausses alertes). Le job échoue seulement si la mesure est
  invalide (arbres différents, délégation à Composer, commande en échec).
- Pas de démarrage à froid (réseau) : Luther n'en a pas, et c'est du réseau.
- Pas de bascule de Defender par le workflow : on relève son état, on ne
  le change pas (piste ouverte, voir M9).
- Pas d'ePHPm : on mesure le binaire `vivacity` lui-même (ePHPm ajoute son
  propre démarrage ; écart noté dans M9).

## Choix de conception : un banc dédié, pas une ligne de plus dans bench.yml

Alternatives :

1. **Ajouter `windows-latest` à la matrice de `bench.yml`.** `bench.yml`
   tourne sur chaque push/PR qui touche `crates/**` : une install Composer
   vendor supprimé coûte ~2 s sous Linux, mais 1–2 min sous Windows avec
   Defender (Luther : 107 s) ; ×11 runs ×3 fixtures, c'est une heure de
   runner à chaque PR. `gate.py` agrège par `fixture/scénario` sans OS : une
   ligne Windows se mélangerait à la baseline Linux. Et `ci-bench.sh`
   partage un seul répertoire entre les deux outils, lance `composer` par
   son nom (un `.bat` sous Windows, que hyperfine `-N` ne trouve pas) et
   ne prouve pas l'égalité des arbres.
2. **Généraliser `ci-bench.sh`** pour couvrir les deux usages : un script à
   deux régimes (drapeaux de Luther ou de la gate, une ou deux copies,
   preuve ou non) pour gagner quelques lignes ; refusé.
3. **Retenu : `bench/windows-bench.sh` + `.github/workflows/bench-windows.yml`**
   (`workflow_dispatch` seulement). Le script est portable (Linux, macOS,
   Git Bash) ; le workflow ne lance que Windows. Il reprend de `ci-bench.sh`
   hyperfine `-N`, la chauffe, `--no-blocking` et sa raison ; de
   `classmap-cache.sh` la gestion des chemins Windows (`cygpath -m`) et les
   caches sous `%LOCALAPPDATA%` (`C:`).

## Méthode du banc

- **Deux copies par fixture** (`<fixture>/composer`, `<fixture>/vivacity`),
  comme Luther : chaque `vendor/` est écrit par son outil (le piège M6 : un
  `vendor/` posé par Composer laisse le store de vivacity vide).
- **Drapeaux identiques des deux côtés** : `--no-interaction --no-scripts -q
  --ignore-platform-reqs` (ceux de Luther) **+ `--no-blocking`** sur
  `install` (raison de `ci-bench.sh` : sans lui, une requête conditionnelle
  aux listes de blocage, dont la latence est celle du réseau ; Composer
  2.10 ; `dump-autoload` n'a pas le drapeau). Plugins actifs, comme Luther
  (Laravel : pest-plugin et php-http/discovery, émulés par vivacity).
  Composer lancé en `php <composer.phar>` (comme Luther ; pas le `.bat`).
- **Scénarios** : `noop` (`install` sur `vendor/` complet), `fresh`
  (`vendor/` supprimé en préparation, caches chauds — le `warm` de
  `ci-bench.sh`), `dump-o` (`dump-autoload -o`). Laravel a
  `optimize-autoloader: true` dans sa config : son `install` est déjà le
  `install -o` de Luther.
- **hyperfine** `-N --warmup 1 --min-runs $BENCH_RUNS`, une invocation par
  outil et par fixture depuis la copie de l'outil (vivacity n'a pas
  `--working-dir`). `-N` : pas de shell à démarrer (sous Windows, un bash
  coûte des dizaines de ms variables, de l'ordre d'un `dump -o` de
  vivacity). `--prepare 'rm -rf <vendor>'` pour `fresh` (le `rm.exe` de Git).
- **Preuve d'égalité, avant la mesure** : par outil, `vendor/` supprimé puis
  `install` (mêmes drapeaux sans `-q`), manifeste ; puis `dump-autoload -o`,
  manifeste. Manifeste = une ligne par fichier (`sha256  chemin`) et par
  lien (`link chemin -> cible`), trié en locale C. Les deux manifestes de
  chaque étape doivent être identiques, sinon arrêt (et les 20 premières
  lignes du diff). **Après la mesure**, nouveau manifeste de l'état laissé
  par hyperfine (après `dump -o`), comparé à nouveau. Le résumé imprime le
  nombre de fichiers et la sha256 de chaque manifeste.
- **Délégation** : la sortie de vivacity sans `-q` ne doit contenir aucune
  ligne `vivacity: delegating to` (sinon on mesurerait Composer) ;
  contrôle sur install, no-op et dump.
- **Conditions relevées** (résumé + artefact) : date, commit, versions
  (vivacity, Composer, PHP, hyperfine), OS et build, CPU, cœurs logiques et
  physiques, RAM ; sous Windows `Get-MpComputerStatus` (temps réel,
  accès, IOAV, comportement, mode), `Get-MpPreference` (temps réel
  désactivé, exclusions), `Get-Volume C,D` (système de fichiers, taille) ;
  chemin, lecteur et système de fichiers du projet et des caches.
- **Disposition** (`BENCH_LAYOUT`, Windows) : `split` = projet sous
  `RUNNER_TEMP` (`D:`), caches sous `%LOCALAPPDATA%` (`C:`), ce que voit tout
  utilisateur CI (vivacity copie depuis le store au lieu de lier) ; `same` =
  projet et caches sous `%LOCALAPPDATA%` (`C:`), la disposition d'un poste à
  un seul disque (liens durs possibles). Le workflow lance les deux en
  matrice. Hors Windows : tout sous `VIVACITY_BENCH_DIR`.
- **Phases** : `prepare` (copies, caches chauffés : le réseau, sous
  `net-retry.sh`) puis `measure` (hors réseau). `all` enchaîne les deux.

## Fichiers, dans l'ordre

1. `bench/windows-bench.sh` (nouveau).
2. `.github/workflows/bench-windows.yml` (nouveau) : `workflow_dispatch`
   (`runs`, `fixtures`, `layouts`), `permissions: contents: read`,
   `COMPOSER_AUTH` du job, `shell: bash`, `core.longpaths`, setup-php 8.4
   `composer:2.10.3`, hyperfine par `taiki-e/install-action`, build,
   `harness/net-retry.sh bench/windows-bench.sh prepare`, `measure`,
   artefact `if: always()`.
3. `bench/M9-windows.md` : méthode, comparaison honnête avec Luther, section
   « à remplir depuis le run N ».
4. README « Speed » (une phrase), HANDOVER (commande), DECISIONS
   (2026-10-29, français), CHANGELOG (Unreleased), ce plan.

## Validation

- `shellcheck bench/windows-bench.sh`, `actionlint` sur le workflow.
- Passage macOS : `BENCH_RUNS=2 BENCH_FIXTURES="laravel symfony sylius"
  bench/windows-bench.sh all` (cache de Composer existant pour éviter le
  réseau) ; chiffres rapportés.
- **Vus rouges** : la preuve d'égalité (un fichier modifié dans un
  `vendor/` → arrêt), le contrôle de délégation (une délégation forcée →
  arrêt), une commande en échec (hyperfine s'arrête).
- Non vérifiable ici : le chemin Windows (cygpath, PowerShell, `rm.exe`
  sous `-N`, `.bat`) ; premier dispatch par l'utilisateur, itération sur
  les échecs.

## Méta-analyse (auto-checklist, avant exécution)

1. **Composer introuvable sous `-N`** : sous Windows `composer` est un
   `.bat` (setup-php) ; hyperfine `-N` passe par `CreateProcess`, qui ne
   lance pas un `.bat` sans extension. → `php <phar>` des deux côtés de la
   comparaison Luther ; le phar est cherché (`COMPOSER_PHAR`, à côté de
   `composer`, ou `composer` lui-même s'il contient `__HALT_COMPILER`), et
   le script s'arrête s'il ne le trouve pas.
2. **`--prepare` sous `-N`** : hyperfine applique `-N` aussi à la
   préparation ; `rm -rf` doit être un exécutable (Git `usr/bin/rm.exe`,
   dans le `PATH` converti des étapes bash). Chemin absolu en barres
   obliques, entre apostrophes (découpage de hyperfine). Risque résiduel :
   non vérifiable ici.
3. **Ordre des commandes et dérive du runner** : hyperfine lance tous les
   runs d'une commande avant la suivante ; Composer et vivacity sont
   mesurés à des minutes d'écart. Inévitable sans entrelacement ; le
   ratio est rapporté, la dispersion (σ, min–max) aussi.
4. **Le no-op de Luther comprend la requête des listes de blocage**
   (pas de `--no-blocking` chez lui) : nos no-op sont plus courts des deux
   côtés. Dit dans M9, pas corrigé (la consigne est la raison de
   `ci-bench.sh`).
5. **`--ignore-platform-reqs` change ce qu'écrit l'autoloader**
   (`platform_check.php`) : passé aussi à `dump-autoload`, des deux côtés ;
   la preuve d'égalité attrape tout écart.
6. **Délégation masquée par `-q`** : la ligne `delegating to` est au
   niveau normal ; le contrôle tourne sans `-q`. Un `vendor/` identique ne
   prouve pas que vivacity a travaillé (Composer délégué l'écrirait aussi).
7. **La version racine** : hors d'un dépôt git, Composer devine la
   version du projet ; les deux copies sont au même niveau d'arborescence,
   hors dépôt (scratchpad, `RUNNER_TEMP`, `%LOCALAPPDATA%`). La preuve
   d'égalité couvre `installed.php`.
8. **MAX_PATH sur Sylius** : `core.longpaths` pour git ; l'extraction de
   PHP/vivacity sous `D:\a\_temp\vivacity-windows-bench\sylius\composer\vendor\…`
   reste dans la longueur des fixtures de `windows.yml` (même profondeur).
   Risque résiduel.
9. **Temps de job** : Composer fresh à ~1–2 min sous Windows ×(1+5) runs,
   Laravel seul par défaut, Symfony et Sylius en entrée ; `timeout-minutes`
   relevé à 240 ; Sylius estimé ×2–3 Laravel.
10. **L'état de Defender sur le runner** n'est pas celui de Luther (temps
    réel probablement désactivé ou à exclusions sur les images GitHub) :
    relevé, et la comparaison avec Luther le dit avant tout chiffre.
11. **Manifeste et fins de ligne** : `.gitattributes` force LF sur le
    checkout, mais les fixtures sont copiées par `tar` depuis le checkout ;
    les deux copies partagent la même source : sans effet sur l'égalité.
12. **Sur-ingénierie** : la disposition `same`/`split` double le temps du
    workflow ; gardée parce que c'est la variable que Luther ne donne pas
    et qui change le chemin de code (lien dur contre copie).

## Exécution — ce qui a changé en route

- **Deux copies côte à côte ne suffisaient pas.** Premier passage macOS sur
  Sylius : arrêt de la preuve, une ligne de différence,
  `vendor/rector/extension-installer/src/GeneratedConfig.php`, qui porte le
  chemin absolu de chaque extension (`…/sylius/composer/vendor/…` contre
  `…/sylius/vivacity/vendor/…`). Une vraie différence de chemin, pas de
  vivacity : chaque copie est désormais renommée en `<fixture>/app` le
  temps que son outil travaille (renommage retenté cinq fois : sous
  Windows, une analyse antivirus peut tenir un descripteur). La preuve a
  fait son travail dès le premier essai.
- **PowerShell par `-EncodedCommand`** (UTF-16LE, base64) : Windows
  PowerShell 5.1 abîme les guillemets doubles d'un argument `-Command`
  transmis par MSYS, et les propriétés calculées en ont.
- **Méta, point 1** : le phar est cherché via `COMPOSER_PHAR`,
  `composer.phar` dans le `PATH`, à côté de `composer`, ou `composer`
  lui-même s'il contient `__HALT_COMPILER`.

## Vérification

- `shellcheck bench/windows-bench.sh` et `actionlint` : propres.
- macOS, `BENCH_RUNS=2`, Laravel, Symfony, Sylius : vert, arbres
  identiques aux trois étapes (8 845, 11 425, 40 584 entrées) ; chiffres
  dans `bench/M9-windows.md`.
- Vus rouges (exit 1, message nommé) : un octet ajouté à
  `vendor/autoload.php` par une enveloppe de vivacity (« the two vendor/
  trees differ » et le diff du manifeste) ; une ligne `delegating to`
  simulée ; une commande qui échoue sous `-q` (hyperfine s'arrête au
  premier run de chauffe).
- Branche Windows exercée sur ce Mac avec des doublures (`uname` qui
  répond MINGW64, `cygpath` identité, `powershell.exe` qui décode
  l'argument) : les deux dispositions, chemins de projet et de caches
  attendus, les six commandes PowerShell relues intactes après
  l'encodage. Non vérifiable ici : `cygpath` réel, `rm.exe`/`true.exe` et
  `php.exe` sous hyperfine `-N`, le chemin du phar de setup-php, les
  sorties réelles de PowerShell, MAX_PATH sur Sylius.

## Auto-revue

- Pas de code mort ; `ci-bench.sh` et `gate.py` intacts.
- Risque résiduel principal : rien de la branche Windows n'a tourné sur
  Windows. Le premier dispatch est le test ; les échecs probables sont
  nommés (phar introuvable → message ; exécutable introuvable sous `-N` →
  journal hyperfine en fin de log).

## Correctif après le premier run (run 40927754769, 2026-10-29)

**Constat.** Les deux dispositions s'arrêtent au premier `--prepare` :
« Command ''true'' terminated with non-zero exit code 1 ». Préparation
réseau verte, preuve Laravel verte (les deux arbres identiques), échec à
la première commande de hyperfine. Lu dans le journal et dans les sources
de hyperfine :

- `taiki-e/install-action` a installé **hyperfine 2.0.0** (sorti le
  2026-10-07) ; le passage macOS tournait sous 1.20.0.
- **hyperfine 2.0 lance `--prepare` (et setup, conclude, cleanup) toujours
  dans un shell**, même sous `-N` : le shell choisi par `--shell`, sinon
  celui de la plateforme — `cmd.exe /C` sous Windows
  (`RawExecutor::run_intermediate_command_and_measure`). Sous 1.x, `-N`
  s'appliquait aussi à la préparation (découpage à la shell, sans shell).
  `'true'` entre apostrophes n'est pas une commande pour cmd.exe ; et
  `command -v true` rendait `true` (une primitive de bash), pas un chemin.
  `'…/rm.exe' -rf '…'` aurait échoué de la même façon.
- **Le JSON de 2.0 change de schéma** (version 2) : `command` est la ligne
  de commande, le nom donné par `-n` passe dans `name`, et les
  statistiques sous `summary.time_wall_clock`. Le tableau du banc serait
  sorti vide ou faux même après la préparation.

**Plan.**

1. `noop` et `dump-o` : une invocation `-N` **sans** `--prepare` (la
   commande mesurée reste exécutée sans shell sous 1.x comme 2.x, découpée
   à la shell : apostrophes valides).
2. `fresh` : une invocation à part, `--shell` = le bash de Git (chemin
   absolu, `.exe`, entre guillemets ; `bash` ailleurs), `--prepare "rm -rf
   '<vendor>'"`. La préparation passe par ce bash sous 1.x et 2.x ; la
   commande mesurée aussi, et hyperfine retranche le démarrage du shell
   qu'il calibre (bruit de quelques dizaines de ms sur des installs de
   plusieurs secondes : acceptable ; `-N` reste là où il compte, sur les
   scénarios courts).
3. Lecture des JSON compatible avec les deux schémas : `(.name //
   .command)` et `(.summary.time_wall_clock // .)`. Les deux invocations
   fusionnées dans `<fixture>-<outil>.json` (`jq -s`).
4. Workflow : `hyperfine@2.0.0` épinglé (les chiffres nomment une version,
   comme Composer).
5. `native true`/`native rm` retirés (plus de préparation sous `-N`).

**Méta-analyse (auto-checklist).**

- Le passage par bash pour `fresh` change ce qui est mesuré : hyperfine
  retranche la moyenne calibrée de `bash -c ""` ; résidu de variance
  ajouté aux deux outils de la même façon. Dit dans M9.
- `$BASH` sous Git Bash vaut `/usr/bin/bash` → `C:/Program Files/Git/usr/bin/bash`,
  un espace : entre guillemets doubles, découpé par `shell_words` (1.x et
  2.x). Le bash lancé directement (pas par `Git\bin\bash.exe`) reçoit le
  `PATH` Windows de l'étape, où figure `Git\usr\bin` : `rm` s'y trouve.
- L'ordre change : `noop`, `dump-o`, puis `fresh`. L'état « after »
  devient celui d'un install `vendor/` supprimé (Laravel : `-o` par sa
  config) ; la comparaison des deux outils reste valable.
- Pas de Windows ici, encore : 2.0 n'est pas installé sur ce Mac (pas de
  téléchargement de binaire sans accord) ; le schéma v2 est testé sur un
  JSON écrit d'après le CHANGELOG de hyperfine, le chemin 1.x sur le Mac.
- Hors périmètre, signalé : `bench/classmap-cache.sh` lit `.command` et
  `.median` (schéma 1) et le workflow `classmap-cache` installe hyperfine
  sans version : il tombera sur le même changement. `ci-bench.sh`/`gate.py`
  prennent le hyperfine d'apt sur ubuntu (1.x) : intacts tant qu'il l'est.

**Vérification.** `shellcheck`, `actionlint` propres. Mac, hyperfine
1.20.0, `BENCH_RUNS=2`, Laravel et Sylius : vert, arbres identiques aux
trois étapes (8 845 et 40 584 entrées). Lecture JSON testée sur un fichier
fusionné schéma 2 + schéma 1 (noms et statistiques justes). Vu rouge : une
install qui échoue seulement sous `-q` et sans `vendor/` arrête le bras
`fresh` à sa chauffe (exit 1) — la préparation par bash a donc bien
supprimé `vendor/`. `-N` contre `--shell bash` sur le `fresh` de vivacity
(Laravel, 5 runs) : 415 ± 167 contre 349 ± 159 ms sur un Mac à charge 17,
pas d'écart lisible. Non vérifié ici : hyperfine 2.0 lui-même, et tout le
chemin Windows au-delà de la première commande de hyperfine.
