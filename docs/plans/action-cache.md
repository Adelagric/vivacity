# Ce que l'action met en cache — plan

> Niveau 2 : `action.yml` est un contrat public (`uses:
> Adelagric/vivacity@vX.Y.Z`). Deux analyses indépendantes en lecture seule
> (2026-10-29) ont conclu que l'action met en cache ce qu'il ne faut pas.

## Le problème

`action.yml` met en cache, avec `actions/cache@v4`, six chemins écrits en
dur : les trois répertoires de cache de vivacity (store, enregistrements de
classmap) et les trois de Composer, sous la clé
`vivacity-<os>-<arch>-<hashFiles('**/composer.lock')>`.

1. **Le store restauré est mort depuis le 27/10.** `store.rs` est passé de
   `LAYOUT` `v2` à `v3` (05193b7) ; l'artefact restauré ne contient que
   `store/v2`, jamais lu, et la clé primaire touchée empêche toute nouvelle
   sauvegarde. Run 40857206667 (29/10), job `action (ubuntu-latest)` :
   40 Mo restaurés, puis `0 from store, 109 from cache`. Même chose du 30/09
   (v1→v2, 29cb00e) au 22/10.
2. **Vivant, il ne paie pas.** Runs du workflow `action` (Laravel, 109
   paquets, `-o` par configuration), restauration + install sur clé trouvée :
   ubuntu 3,0 s (store vivant) contre 2,7 s (mort) ; Windows 26,1 s contre
   21,6 s (store sur `C:`, projet sur `D:` : copie, pas de lien) ; macOS
   noyé dans le bruit du téléchargement (même artefact restauré en 6,2 s ou
   12,0 s). Un run par cas : indication. `bench/M7-ci-cache.md` allait dans
   le même sens, mesuré hors `actions/cache`.
3. **Les chemins sont figés.** Un `COMPOSER_CACHE_DIR`, `COMPOSER_HOME` ou
   `XDG_CACHE_HOME` défini par le projet déplace le cache de Composer hors
   des six chemins : rien n'est mis en cache.
4. **`actions/cache@v4`** n'est pas épinglé par SHA ; la v4 tourne sur
   node20, retiré des runners hébergés.

## Ce qui change

- **Seul le cache de Composer** est mis en cache (zips `files/`, miroirs
  `vcs/`, métadonnées `repo/`) : vivacity le lit et l'alimente
  (`fetch.rs`, « both read AND fed »), et en reconstruit son store sans
  réseau. Le store et les enregistrements de classmap n'y sont plus.
- **Le chemin est résolu** par une étape bash, avant la restauration, selon
  `Factory::getCacheDir` tel que `vivacity_core::fetch::composer_cache_dir`
  le porte : `COMPOSER_CACHE_DIR` ; sinon `$COMPOSER_HOME/cache` ; Windows
  `%LOCALAPPDATA%/Composer` (barres obliques, comme Composer) sinon
  `<home>/cache` ; macOS `~/Library/Caches/composer` ; ailleurs
  `~/.composer/cache` si le home de Composer est `~/.composer` et que ce
  répertoire existe, sinon `${XDG_CACHE_HOME:-~/.cache}/composer` si une
  variable `XDG_*` existe, sinon `<home>/cache`. Exposé en sortie
  `composer-cache-dir` de l'action.
- **`actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0`** :
  `action.yml` de v4.3.0 à v6.1.0 ne diffère que par `node20` → `node24` ;
  runner ≥ 2.327.1 requis (hébergés : 2.338.0 dans les logs).
- La clé ne change pas de forme ; changer `path` change la « version » du
  cache d'`actions/cache` : chaque dépôt aura un échec de cache unique.
- Description de l'entrée `cache` et nom de l'étape mis à jour.

## Non-objectifs

- Mettre en cache les enregistrements de classmap (`<cache>/classmap/`) :
  séparables du store et petits, mais leur clé contient le chemin du store
  (donc `LAYOUT`) et leur gain avec `-o` n'est pas mesuré. Mesure dédiée
  ensuite (zips seuls contre zips + classmap, Laravel et Sylius, `-o`,
  trois OS).
- `config.cache-dir` / `cache-files-dir` / `cache-vcs-dir` d'un
  `composer.json` ou du `config.json` global : l'action ne connaît pas le
  répertoire du projet ; documenté.
- Le nettoyage du cache de Composer (vivacity n'en fait jamais, Composer
  si : 6 mois, 300 Mio), la suppression des anciens `store/vN`, la clé
  `hashFiles('**/composer.lock')` qui hache tous les locks du dépôt :
  constatés, hors de ce changement (HANDOVER).
- Dependabot.

## Fichiers

`action.yml` ; `.github/workflows/action-test.yml` ; `README.md` (section
GitHub Actions) ; `CHANGELOG.md` ; `DECISIONS.md` ; `HANDOVER.md` ;
`bench/M7-ci-cache.md` (addendum daté : sa lecture comptait la sauvegarde à
chaque job) ; ce plan.

## Vérification

- **Rouge d'abord** : un pas d'`action-test.yml` compare la sortie
  `composer-cache-dir` à `composer config --global cache-dir` (Composer
  2.10.3 installé par setup-php), deux fois : environnement du runner, puis
  `COMPOSER_CACHE_DIR` pointé ailleurs. Avec l'ancienne action la sortie
  est vide : rouge.
- En local : la fonction bash contre `composer config --global cache-dir`
  sous macOS et dans un conteneur Linux, avec `COMPOSER_CACHE_DIR`,
  `COMPOSER_HOME`, `XDG_CACHE_HOME`, `XDG_*` absent, `~/.composer/cache`
  existant.
- `actionlint` si disponible ; le workflow `action` vert sur les trois OS
  après poussée, avec dans le log le chemin résolu restauré ou sauvé.

## Méta-analyse (sous-agent frais, plan et code, Composer 2.10.3 exécuté)

Onze points ; ceux qui changent le plan :

1. **(haute) Composer post-traite le chemin, vivacity non.** `Config::get('cache-dir')`
   (Config.php:372-397, 650-657) relit `COMPOSER_CACHE_DIR`, retire les `/` et `\`
   finaux, développe `~`, `$VAR`, `%VAR%`, et résout un chemin relatif. Mesuré avec
   `composer config --global cache-dir` : `/tmp/zz/` → `/tmp/zz`, `~/zz` et `$HOME/zz`
   développés, vide → chaîne vide, `COMPOSER_HOME=/tmp/hh/` → `/tmp/hh//cache`.
   `fetch.rs:159-163` prend la valeur brute : un `COMPOSER_CACHE_DIR: ~/.composer-cache`
   en YAML (où `~` n'est pas développé) fait écrire vivacity dans `./~/.composer-cache`,
   sans erreur, quand Composer et `actions/cache` utilisent `$HOME/.composer-cache`.
   `git_source.rs:768-812` connaît déjà le cas et rend la main à Composer.
2. **(haute) `useXdg` vaut aussi vrai si `/etc/xdg` est un répertoire**
   (Factory.php:713-715) ; `fetch.rs:103-105` ne regarde que les variables `XDG_*`.
   Runners ubuntu hébergés épargnés (`XDG_CONFIG_HOME` posé par l'image), conteneurs
   et Linux auto-hébergés touchés.
3. **(moyenne) Le test vise le mauvais contrat**, et le plan ajoutait une troisième
   copie de l'algorithme (PHP, Rust, bash). Ce qui compte : le répertoire sauvé est
   celui où vivacity écrit. → demander le chemin au binaire.
4. **(moyenne) Résolu une fois, à l'appel de l'action** : des `COMPOSER_*` posés plus
   tard (étape, `$GITHUB_ENV`) ou un relatif avec `working-directory:` échappent. →
   documenter « au niveau du job, avant l'action », journaliser le chemin.
5. **(moyenne) Prémisse fausse : node20 n'est pas retiré**, les runners hébergés le
   forcent en node24 avec un avertissement. Passer en v6.1.0 échange cet
   avertissement contre l'exigence d'un runner ≥ 2.327.1 (GHES, auto-hébergés
   anciens). Les autres `actions/cache@v4` et `checkout@v4` des workflows du dépôt
   ont le même avertissement.
6. **(moyenne-basse) « sans réseau »** ne vaut que pour les dists (sources git
   resynchronisées pour une référence sans branche ni tag ; Windows rend les sources
   git à Composer ; Flex fait un GET conditionnel).
7-11. **(basses)** le résolveur doit tourner même avec `cache: false` ; ne pas
   normaliser les barres d'un `COMPOSER_CACHE_DIR` Windows (Composer les garde) mais
   `cygpath -m` une valeur MSYS en `/c/…` ; Windows sans `LOCALAPPDATA` ni `APPDATA` :
   Composer lève, vivacity prend `./cache` ; croissance du cache par restore-key
   (une action composite n'a pas d'étape `post:` pour élaguer) ;
   `COMPOSER_CACHE_FILES_DIR`/`COMPOSER_CACHE_REPO_DIR` ignorés par vivacity.

## Le modèle, révisé

- **A — cœur** (`fetch.rs`) : `composer_cache_dir` suit `Config::get('cache-dir')`
  (barres finales retirées, `~`/`$VAR`/`%VAR%` développés, relatif résolu comme
  Composer) ; `use_xdg` ajoute `/etc/xdg`. Ce que vivacity ne reproduit pas (valeur
  vide ou `0`, Windows sans `LOCALAPPDATA` ni `APPDATA`, `COMPOSER_CACHE_FILES_DIR`,
  `COMPOSER_CACHE_REPO_DIR`) part à Composer, nommé, avant toute écriture, comme
  `COMPOSER_CACHE_VCS_DIR` aujourd'hui. Tests unitaires et un harnais contre Composer
  (là où Composer écrit vraiment ses zips).
- **B — `vivacity config --global cache-dir`** : imprime ce que vivacity utilise, à
  l'octet de `composer config --global cache-dir` ; toute autre forme de `config`
  reste rejetée (exit 2), comme aujourd'hui. Harnais sur la même matrice d'env.
- **C — action** : installer le binaire d'abord, puis résoudre par le binaire (toujours,
  même `cache: false`), sortie `composer-cache-dir` journalisée, puis restaurer ce seul
  répertoire. Valeur MSYS → `cygpath -m`. `actions/cache` v6.1.0 épinglé par SHA, avec
  l'exigence runner ≥ 2.327.1 au CHANGELOG. Les autres workflows du dépôt : hors champ,
  listés au HANDOVER.
- **Test** (`action-test.yml`) : sortie = `composer config --global cache-dir` (lancé en
  dernier, il crée des répertoires) ; second appel avec `cache: 'false'` et
  `COMPOSER_CACHE_DIR: ${{ runner.temp }}/cc` → sortie identique ; après
  `vivacity install`, `<sortie>/files` non vide. Rouge avec l'ancienne action.
- **Doc** : README (section GitHub Actions : ce qui est mis en cache, `COMPOSER_*` au
  niveau du job avant l'action, ne pas cacher soi-même le même répertoire, runner ≥
  2.327.1) ; « sans réseau » limité aux dists.
