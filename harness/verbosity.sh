#!/usr/bin/env bash
# Harness de verbosité : la même commande jouée par Composer et par vivacity
# sous `-q`, `-v` (et au niveau normal, en témoin), sur le même instantané
# Packagist figé. Contrairement à steps.sh, **stdout et stderr sont comparés
# en entier** : sous `-q` il n'y a pas de ligne d'ancrage, et ce que `-v`
# ajoute vient souvent avant elle (durée de résolution, `Analyzed …`).
# Plan : docs/plans/v0.21-verbosity.md.
#
# Normalisations, chacune ancrée :
#   - `in 0.123 seconds` → `in N seconds` (durées mesurées, jamais égales) ;
#   - le chemin du projet (physique et tel que donné) → `<project>` ;
#   - côté vivacity, les lignes `^vivacity: ` (résumé d'une installation,
#     « delegating to ») et `^Note: ` sont retirées de stderr aux niveaux
#     normal et -v, comme dans steps.sh — elles n'ont pas d'équivalent chez
#     Composer ; sous `-q` rien n'est retiré : elles ne doivent pas sortir.
# Environnement des deux côtés : `CI=1` et `--no-progress` (pas de barre de
# progression, dont le format change sous `-v`), `COMPOSER_TESTS_ARE_RUNNING`
# (pas de lignes `::error` de GitHub Actions).
#
# Seconde partie, `online` (docs/plans/verbose-plugins.md) : des projets à
# plugins ACTIFS résolus sur Packagist en ligne (lock écrit une fois par
# Composer sous `--no-plugins`, versions exactes à la racine ; le projet
# Laravel a son lock figé, fixtures/projects/laravel-pest),
# COMPOSER_HOME isolé, cache de fichiers partagé. Sous `-v`, Composer écrit une
# ligne `> <événement>: <classe>-><méthode>` par écouteur de plugin qu'il
# appelle (EventDispatcher.php:230) : chaque cas compare stdout et stderr
# entiers, puis la sortie FUSIONNÉE d'une seconde copie (l'ordre entre les
# lignes `>`, sur stderr, et celles des plugins sur stdout), le projet entier
# (harness/lib/compare.sh), et exige que vivacity n'ait PAS délégué (aucune
# ligne `vivacity: delegating` ni « outside what vivacity » sur sa stderr
# brute) — sauf `@delegates:<motif>`, où il doit déléguer avec cette raison.
#
# Usage : harness/verbosity.sh [fixture...]   (`online` : la seconde partie)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/registry.sh
. "$ROOT/harness/lib/registry.sh"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/verbosity"
FIXTURES=("$@"); [ ${#FIXTURES[@]} -eq 0 ] && FIXTURES=(laravel symfony solver-problems solver-conflict path-repos online)
# "fixture|arguments" : chaque cas est joué sous chacun des NIVEAUX. Les mots
# finaux en `@…` préparent la copie (sous-ensemble de ceux de steps.sh) :
#   @nolock  @nojson  @require:a/b=^1  @jq:expr  @env:NAME=VALUE
#   @vendor   vendor/ installé au préalable par Composer (`install
#             --no-autoloader`), le même des deux côtés
#   @warm:update  idem par un `update` jetable (les dists de la résolution
#             fraîche : une install sans lock, `config.lock: false`)
#   @warm     le cache de l'instantané rempli au préalable par un install
#             Composer jetable : les deux côtés lisent les dists du cache
#             (les lignes « Downloading » ne dépendent pas de l'ordre)
#   @plugins  plugins actifs des deux côtés (par défaut `--no-plugins` des
#             deux côtés : le même régime, sinon la comparaison ne dirait
#             rien des plugins)
#   @levels:… les niveaux joués pour ce cas (par défaut normal, -q, -v),
#             séparés par des virgules ; `-vv` rend la commande à Composer
#   @unset:NOM  la variable retirée de l'environnement des deux commandes
#             (dans leur sous-shell : l'export global reste pour les autres
#             cas) — `@unset:COMPOSER_ROOT_VERSION` laisse Composer deviner
#             la version de la racine
#   @rootwarn:N  la sortie d'erreur de Composer doit contenir N fois
#             l'avertissement « could not detect the root package » (0 sous
#             -q) : un cas qui ne montre plus ce qu'il teste échoue
#   @lockjq:expr  l'expression jq appliquée à composer.lock
#   @files:a[,b]  ces fichiers du projet comparés à l'octet après la commande
#             (absents des deux côtés : égaux ; d'un seul : différents) —
#             ce que la commande écrit sans le dire
CASES=(
  # install depuis le lock, sans rien écrire : durée de résolution, `Installs:`.
  "laravel|install --dry-run"
  "laravel|install --dry-run --no-dev"
  # Lock périmé : l'avertissement est écrit au niveau QUIET chez Composer.
  "laravel|install --dry-run @require:psr/log=^3.0"
  # Lock insoluble sur la plateforme (`doInstall` le résout) : tête QUIET,
  # problèmes au niveau normal, durée de résolution sous -v.
  "laravel|install --dry-run @require:ext-nope=*"
  # … que les variables d'environnement font ignorer, avec leur avertissement.
  "laravel|install --dry-run @require:ext-nope=* @env:COMPOSER_IGNORE_PLATFORM_REQS=1"
  "laravel|install --dry-run @require:ext-nope=* @env:COMPOSER_IGNORE_PLATFORM_REQ=ext-nope,ext-other"
  # `BaseCommand::initialize` lit ces variables pour toute commande qui a les
  # options : `dump-autoload` aussi (son `platform_check.php` les suit), et
  # leur avertissement précède ce que `require`/`remove` impriment.
  "laravel|dump-autoload @vendor @env:COMPOSER_IGNORE_PLATFORM_REQS=1 @files:vendor/composer/platform_check.php"
  "laravel|dump-autoload @vendor @env:COMPOSER_IGNORE_PLATFORM_REQ=php @files:vendor/composer/platform_check.php"
  "laravel|require symfony/uid --dry-run --no-install @env:COMPOSER_IGNORE_PLATFORM_REQS=1"
  "laravel|remove laravel/tinker --dry-run --no-install @env:COMPOSER_IGNORE_PLATFORM_REQ=ext-nope"
  "laravel|require symfony/uid --dry-run --no-install --no-suggest"
  # `config.lock: false` (Factory : « ./composer.lock is present but
  # ignored… », `Locker` sur /dev/null) et `install` sans lock
  # (`Installer::run` passe en update) : la résolution sans dépôt verrouillé,
  # aucun lock écrit avec `config.lock: false`, l'installation de la
  # résolution (pas du lock sur le disque), la ligne de Factory deux fois
  # pour `require`/`remove`.
  "laravel|install --dry-run @jq:.config.lock=false"
  "laravel|install --dry-run @nolock"
  "laravel|update --dry-run --no-install @jq:.config.lock=false"
  "laravel|dump-autoload @vendor @jq:.config.lock=false"
  # … dont `AutoloadGenerator::dump` lit le dépôt local : un lock périmé ou
  # absent n'y change rien (autoload_static.php porte un suffixe aléatoire
  # sans lock ni autoload.php : comparés, les fichiers sans suffixe).
  "laravel|dump-autoload @vendor @jq:.config.lock=false @lockjq:.packages|=map(select(.name!=\"laravel/tinker\")) @files:vendor/composer/autoload_psr4.php,vendor/composer/autoload_classmap.php"
  "laravel|dump-autoload @vendor @jq:.config.lock=false @nolock @files:vendor/composer/autoload_psr4.php,vendor/composer/autoload_classmap.php"
  "laravel|require symfony/uid --dry-run --no-install @jq:.config.lock=false"
  "laravel|remove laravel/tinker --dry-run --no-install @jq:.config.lock=false"
  "laravel|require symfony/uid --dry-run @jq:.config.lock=false"
  "laravel|install @warm:update @jq:.config.lock=false @files:composer.lock,vendor/composer/installed.json"
  "laravel|install @warm:update @nolock @files:composer.lock,vendor/composer/installed.json"
  "laravel|update @warm:update @jq:.config.lock=false @files:composer.lock,vendor/composer/installed.json"
  "laravel|update --dry-run @env:COMPOSER_NO_DEV=yes"
  # … avant que la commande ne découvre l'absence de composer.json.
  "laravel|dump-autoload @nojson @env:COMPOSER_IGNORE_PLATFORM_REQS=1"
  "laravel|update --dry-run --no-install @nojson @env:COMPOSER_IGNORE_PLATFORM_REQS=1"
  "laravel|install --dry-run @nojson @env:COMPOSER_IGNORE_PLATFORM_REQ=ext-nope"
  # `COMPOSER_NO_DEV` vaut `--no-dev` dès qu'il est vrai au sens de PHP
  # (`(bool)` d'une chaîne : tout sauf "" et "0"), pour `install` comme pour
  # `dump-autoload`.
  "laravel|install --dry-run @env:COMPOSER_NO_DEV=yes"
  "laravel|install --dry-run @env:COMPOSER_NO_DEV=0"
  "laravel|dump-autoload @vendor @env:COMPOSER_NO_DEV=true @files:vendor/composer/autoload_static.php"
  # `GithubActionError::emit` : sous GitHub Actions, sans
  # COMPOSER_TESTS_ARE_RUNNING, l'annotation `::error ::…` d'un ensemble
  # insoluble sur stdout (niveau normal) — résolution d'`update` (sans
  # l'avertissement `--no-dev`), vérification du lock d'`install` ; `0` ne
  # compte pas, `false` si (une chaîne non vide, pour PHP).
  "solver-problems|update --no-install @nolock @require:acme/missing-dep=^1.0 @unset:COMPOSER_TESTS_ARE_RUNNING @env:GITHUB_ACTIONS=true"
  "solver-problems|update --no-install --no-dev @nolock @require:acme/nope=^1.0 @unset:COMPOSER_TESTS_ARE_RUNNING @env:GITHUB_ACTIONS=true"
  "laravel|install --dry-run @require:ext-nope=* @unset:COMPOSER_TESTS_ARE_RUNNING @env:GITHUB_ACTIONS=true"
  "laravel|install --dry-run @require:ext-nope=* @unset:COMPOSER_TESTS_ARE_RUNNING @env:GITHUB_ACTIONS=0"
  "laravel|install --dry-run @require:ext-nope=* @unset:COMPOSER_TESTS_ARE_RUNNING @env:GITHUB_ACTIONS=false"
  # Pas de composer.json : message au niveau normal (muet sous -q), exit 1.
  "laravel|install --dry-run @nojson"
  # Une vraie installation (cache chaud), puis un install sans opération.
  "laravel|install @warm"
  "laravel|install @vendor"
  # -vv : rendu à Composer en entier, avant toute lecture.
  "laravel|install --dry-run @levels:-vv"
  # dump-autoload : ses deux lignes sont sur STDOUT chez Composer.
  "laravel|dump-autoload @vendor"
  "laravel|dump-autoload -o @vendor"
  "laravel|dump-autoload --classmap-authoritative @vendor"
  # update : deux résolutions (+ extraction dev), `Analyzed …` ×2, `Updates:`.
  "laravel|update --dry-run --no-install"
  "laravel|update --dry-run --no-install @nolock"
  "laravel|update laravel/pint --dry-run --no-install"
  "laravel|remove laravel/tinker --dry-run --no-install"
  "laravel|require symfony/uid --dry-run --no-install"
  # Ensemble insoluble : tête QUIET, problèmes au niveau normal, explication
  # détaillée sous -v (`acme/many`).
  "solver-problems|update --no-install @nolock @require:acme/missing-dep=^1.0"
  "solver-problems|update --no-install @nolock @require:acme/many=^1.0"
  "solver-problems|update --no-install --no-dev @nolock @require:acme/nope=^1.0"
  # Un plugin verrouillé et autorisé (symfony/flex, symfony/runtime) :
  # natif au niveau normal, rendu à Composer sous -v à cause de Flex (ses
  # écouteurs de pool et de paquet, ses configurateurs), raison nommée.
  "symfony|install --dry-run @plugins @delegates:symfony/flex"
  # Conflit à la résolution principale (tête QUIET, `--no-dev` aussi).
  "solver-conflict|update --no-install @nolock"
  "solver-conflict|update --no-install --no-dev @nolock"
  # Versions dev avec référence (`dev-main abc1234`) dans `Installs:`.
  "path-repos|update --no-install @nolock"
  "path-repos|update --no-install"
  # `RootPackageLoader::load` sans version devinable (copie hors de tout
  # dépôt git, `COMPOSER_ROOT_VERSION` retirée) : « Composer could not detect
  # the root package (…) version », sur stderr au niveau normal, avant toute
  # autre ligne — et avant les avertissements de `BaseCommand::initialize`.
  # laravel rendu bibliothèque (le type `project` coupe la ligne).
  "laravel|install --dry-run @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  "laravel|install @vendor @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  "laravel|install --dry-run @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @env:COMPOSER_IGNORE_PLATFORM_REQS=1 @rootwarn:1"
  "laravel|install --no-install @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  "laravel|update --dry-run --no-install @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  "laravel|dump-autoload @vendor @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  # `require` et `remove` recréent le Composer après « has been updated »
  # (`resetComposer` + `requireComposer`) : une seconde ligne, avant
  # « Running composer update » ; pas avec `--no-update`.
  "laravel|require symfony/uid --dry-run --no-install @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:2"
  "laravel|require symfony/uid --no-update @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  "laravel|remove laravel/tinker --dry-run --no-install @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:2"
  # Pas de type du tout (`solver-conflict`) : la ligne, puis le conflit.
  "solver-conflict|update --no-install @nolock @unset:COMPOSER_ROOT_VERSION @rootwarn:1"
  # Rendu à Composer avant toute ligne : la sienne seule.
  "laravel|install --dry-run @jq:.type=\"library\" @unset:COMPOSER_ROOT_VERSION @levels:-vv @rootwarn:1"
  # Témoins sans ligne : type `project`, racine sans nom, version écrite.
  "laravel|install --dry-run @unset:COMPOSER_ROOT_VERSION @rootwarn:0"
  "laravel|install --dry-run @jq:del(.name)|.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:0"
  "laravel|install --dry-run @jq:.version=\"1.0.0\"|.type=\"library\" @unset:COMPOSER_ROOT_VERSION @rootwarn:0"
  # `Platform::getEnv` lu comme PHP : `0` et la chaîne vide ne comptent pas.
  "laravel|install --dry-run @jq:.type=\"library\" @env:COMPOSER_ROOT_VERSION=0 @rootwarn:1"
  "laravel|install --dry-run @jq:.type=\"library\" @env:COMPOSER_ROOT_VERSION= @rootwarn:1"
  # Un dépôt git (path-repos, projet commité sur `develop`) : la version est
  # devinée, pas de ligne.
  "path-repos|update --no-install @unset:COMPOSER_ROOT_VERSION @rootwarn:0"
)
LEVELS=("" "-q" "-v")
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK"
harness_git_env "$WORK"
# `COMPOSER_ROOT_VERSION` : sans lui, une racine qui n'est pas de type
# `project` hors dépôt git (solver-*) fait écrire à Composer « Composer
# could not detect the root package… » ; les cas qui testent cette ligne la
# retirent (`@unset:COMPOSER_ROOT_VERSION`), les autres restent sur la même
# version des deux côtés.
export CI=1 COMPOSER_TESTS_ARE_RUNNING=1 COMPOSER_ROOT_VERSION=1.0.0
# `COMPOSER_TEST_SUITE` : sans lui, `Cache::gcIsNecessary` tire au sort (une
# fois sur 51) un ramasse-miettes du cache, et `FileDownloader` écrit
# « Running cache garbage collection » au niveau `-vv` — d'un seul côté des
# cas `-vv`, où les deux sorties sont celles de Composer. Vu une fois sur
# `laravel install --dry-run -vv`. C'est la seule lecture de la variable
# dans Composer 2.10.3 (Cache.php:245).
export COMPOSER_TEST_SUITE=1
# Les variables `@unset:` du cas, retirées dans le sous-shell d'une commande.
unset_case_vars() {
  local u
  for u in "${unsets[@]+"${unsets[@]}"}"; do unset "$u"; done
  return 0
}
# normalize <fichier> <répertoire du projet> : durées, et chemin du projet
# (physique et tel que donné) ramené à `<project>` — chaque côté a le sien.
normalize() {
  local real; real="$(cd "$2" && pwd -P)"
  sed -E -e 's/ in [0-9]+\.[0-9]{3} seconds/ in N seconds/' -e "s#$real#<project>#g" -e "s#$2#<project>#g" "$1"
}
status=0
for fx in "${FIXTURES[@]}"; do
  [ "$fx" = online ] && continue
  archive="$ROOT/fixtures/registry/$fx.tar.gz"
  [ -f "$archive" ] || { echo "SKIP $fx : pas d'instantané (tools/snapshot-packagist.sh $fx)"; continue; }
  reg="$WORK/registry-$fx"; rm -rf "$reg"; mkdir -p "$reg"
  tar -C "$reg" -xzf "$archive"
  write_snapshot_packages_json "$reg"
  home="$WORK/home-$fx"; rm -rf "$home"; mkdir -p "$home"
  printf '{"repositories": %s}\n' "$(snapshot_repositories_json "$reg")" > "$home/config.json"
  n=0
  for spec in "${CASES[@]}"; do
    [ "${spec%%|*}" = "$fx" ] || continue
    read -r -a sargs <<< "${spec#*|}"
    preps=()
    while [ ${#sargs[@]} -gt 0 ]; do
      last=$(( ${#sargs[@]} - 1 ))
      case "${sargs[$last]}" in
        @*) preps+=("${sargs[$last]}"); unset "sargs[$last]" ;;
        *) break ;;
      esac
    done
    levels=("${LEVELS[@]}"); plugins=(--no-plugins); unsets=(); rootwarn=""; files=(); delegates=""
    for prep in "${preps[@]+"${preps[@]}"}"; do
      case "$prep" in
        @delegates:*) delegates="${prep#@delegates:}" ;;
        @levels:*) IFS=, read -r -a levels <<< "${prep#@levels:}" ;;
        @plugins) plugins=() ;;
        @unset:*) unsets+=("${prep#@unset:}") ;;
        @rootwarn:*) rootwarn="${prep#@rootwarn:}" ;;
        @files:*) IFS=, read -r -a files <<< "${prep#@files:}" ;;
      esac
    done
    for level in "${levels[@]}"; do
      n=$((n + 1)); saved=()
      for prep in "${preps[@]+"${preps[@]}"}"; do
        case "$prep" in
          @env:*)
            kv="${prep#@env:}"; name="${kv%%=*}"
            # La valeur d'avant est rétablie après le cas (une variable
            # exportée pour tous, comme COMPOSER_ROOT_VERSION, le reste).
            if [ -n "${!name+set}" ]; then saved+=("$name=${!name}"); else saved+=("$name"); fi
            export "$name=${kv#*=}" ;;
        esac
      done
      for prep in "${preps[@]+"${preps[@]}"}"; do
        if [ "$prep" = @warm ] && [ ! -f "$home/warmed" ]; then
          w="$WORK/warm-$fx"; stage_project "$fx" "$w"
          (cd "$w" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" composer install --no-scripts --no-plugins --no-autoloader --no-progress -q) && touch "$home/warmed"
        fi
        if [ "$prep" = @warm:update ] && [ ! -f "$home/warmed-update" ]; then
          w="$WORK/warm-update-$fx"; stage_project "$fx" "$w"
          (cd "$w" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" composer update --no-scripts --no-plugins --no-autoloader --no-progress --no-audit -q) && touch "$home/warmed-update"
        fi
      done
      for side in ref viv; do
        d="$WORK/$side-$fx-$n"; stage_project "$fx" "$d"
        # `@vendor` d'abord : le lock de la fixture installé tel quel, avant
        # que les autres préparations ne touchent au manifeste ou au lock ;
        # dans l'environnement d'avant les `@env:` du cas — elles sont celles
        # de la commande comparée, pas de l'installation qui la prépare.
        for prep in "${preps[@]+"${preps[@]}"}"; do
          [ "$prep" = @vendor ] || continue
          (for e in "${saved[@]+"${saved[@]}"}"; do case "$e" in *=*) export "$e" ;; *) unset "$e" ;; esac; done
            cd "$d" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" composer install --no-scripts --no-plugins --no-autoloader --no-progress -q)
        done
        for prep in "${preps[@]+"${preps[@]}"}"; do
          case "$prep" in
            @env:*|@warm|@warm:update|@vendor|@levels:*|@plugins|@unset:*|@rootwarn:*|@files:*|@delegates:*) ;;
            @nolock) rm -f "$d/composer.lock" ;;
            @nojson) rm -f "$d/composer.json" "$d/composer.lock" ;;
            @require:*) kv="${prep#@require:}"; jq --arg p "${kv%%=*}" --arg c "${kv#*=}" '.require[$p] = $c' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json" ;;
            @jq:*) jq "${prep#@jq:}" "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json" ;;
            @lockjq:*) jq "${prep#@lockjq:}" "$d/composer.lock" > "$d/l.tmp" && mv "$d/l.tmp" "$d/composer.lock" ;;
            *) echo "prep inconnu : $prep"; exit 1 ;;
          esac
        done
      done
      flags=(); [ -n "$level" ] && flags=("$level")
      # Options propres à chaque commande : `install` n'audite que sur
      # demande, `dump-autoload` n'a ni --no-audit ni --no-progress.
      audit=(--no-audit); progress=(--no-progress)
      case "${sargs[0]}" in
        install) audit=() ;;
        dump-autoload) audit=(); progress=() ;;
      esac
      ref_code=0
      (cd "$WORK/ref-$fx-$n" && unset_case_vars && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" \
        composer "${sargs[@]}" "${audit[@]+"${audit[@]}"}" "${progress[@]+"${progress[@]}"}" --no-scripts "${plugins[@]+"${plugins[@]}"}" --no-interaction "${flags[@]+"${flags[@]}"}" \
        >"$WORK/$fx-$n.composer.out" 2>"$WORK/$fx-$n.composer.err") || ref_code=$?
      viv_code=0
      (cd "$WORK/viv-$fx-$n" && unset_case_vars && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" \
        "$VIVACITY" "${sargs[@]}" "${progress[@]+"${progress[@]}"}" "${plugins[@]+"${plugins[@]}"}" "${flags[@]+"${flags[@]}"}" \
        >"$WORK/$fx-$n.vivacity.out" 2>"$WORK/$fx-$n.vivacity.err") || viv_code=$?
      for e in "${saved[@]+"${saved[@]}"}"; do
        case "$e" in *=*) export "$e" ;; *) unset "$e" ;; esac
      done; saved=()
      label="$fx ${sargs[*]} ${level:-(normal)}"; [ ${#preps[@]} -gt 0 ] && label="$label (${preps[*]})"
      ok=1
      if [ -n "$rootwarn" ]; then
        want="$rootwarn"; [ "$level" = -q ] && want=0
        got=$(grep -c "^Composer could not detect the root package " "$WORK/$fx-$n.composer.err" || true)
        if [ "$got" != "$want" ]; then
          echo "FAIL $label : Composer écrit $got fois l'avertissement de version racine, attendu $want (cas mal choisi)"; ok=0
        fi
      fi
      if [ "$ref_code" != "$viv_code" ]; then
        echo "FAIL $label : code retour composer=$ref_code vivacity=$viv_code"; ok=0
      fi
      # `@delegates:` : sous -v, vivacity rend la commande avec cette raison
      # (sans quoi la comparaison serait Composer contre Composer).
      if [ -n "$delegates" ] && [ "$level" = -v ] && ! grep -qE -- "^vivacity: delegating to .*$delegates" "$WORK/$fx-$n.vivacity.err"; then
        echo "FAIL $label : vivacity ne rend pas la commande avec la raison « $delegates »"; ok=0
      fi
      for stream in out err; do
        normalize "$WORK/$fx-$n.composer.$stream" "$WORK/ref-$fx-$n" > "$WORK/$fx-$n.composer.$stream.n"
        # Les lignes propres à vivacity (résumé, notes, « delegating to »)
        # sont tolérées sur stderr seulement, et jamais sous -q, où rien
        # d'elle ne doit sortir.
        if [ "$stream" = err ] && [ "$level" != -q ]; then
          normalize "$WORK/$fx-$n.vivacity.$stream" "$WORK/viv-$fx-$n" | grep -v -E '^(vivacity|Note): ' > "$WORK/$fx-$n.vivacity.$stream.n" || true
        else
          normalize "$WORK/$fx-$n.vivacity.$stream" "$WORK/viv-$fx-$n" > "$WORK/$fx-$n.vivacity.$stream.n"
        fi
        if ! diff -q "$WORK/$fx-$n.composer.$stream.n" "$WORK/$fx-$n.vivacity.$stream.n" >/dev/null; then
          echo "FAIL $label : std$stream diffère"
          diff "$WORK/$fx-$n.composer.$stream.n" "$WORK/$fx-$n.vivacity.$stream.n" | head -12 || true; ok=0
        fi
      done
      for f in "${files[@]+"${files[@]}"}"; do
        r="$WORK/ref-$fx-$n/$f"; v="$WORK/viv-$fx-$n/$f"
        if [ -e "$r" ] || [ -e "$v" ]; then
          if ! { [ -e "$r" ] && [ -e "$v" ] && cmp -s "$r" "$v"; }; then
            echo "FAIL $label : $f diffère"
            diff "$r" "$v" 2>&1 | head -8 || true; ok=0
          fi
        fi
      done
      if [ "$ok" = 1 ]; then
        echo "OK   $label : stdout et stderr identiques (code $ref_code, $(wc -l < "$WORK/$fx-$n.composer.err" | tr -d ' ') lignes)"
      else
        status=1
      fi
    done
  done
done
# --- Seconde partie : projets en ligne, plugins actifs ------------------------
# Un projet par plugin dont vivacity reproduit la ligne d'écouteur, les muets,
# une combinaison (ordre d'activation, compteur `_composer_tmpN` de
# `PluginManager::registerPackage`), le squelette Laravel avec Pest
# (fixtures/projects/laravel-pest, lock figé), et des plugins rendus à
# Composer sous -v avec la raison. Versions exactes partout : une version
# publiée demain hors des versions lues rendrait la commande à Composer.
# Le manifeste de chaque projet, dans une variable `PROJ_<nom>` : pas de
# tableau associatif (bash 4), le bash de macOS est en 3.2.
proj_var() { printf 'PROJ_%s' "$(printf '%s' "$1" | tr -c 'A-Za-z0-9' '_')"; }
set_project() { printf -v "$(proj_var "$1")" '%s' "$2"; }
# allow <nom>… : la clé `config` qui les autorise.
allow() { local out="" n; for n in "$@"; do out="$out${out:+,}\"$n\":true"; done; printf '"config":{"allow-plugins":{%s}}' "$out"; }
set_project runtime '{"name":"vivacity/verbose-runtime","require":{"symfony/runtime":"v7.4.14"},'"$(allow symfony/runtime)"'}'
set_project pest '{"name":"vivacity/verbose-pest","require":{"pestphp/pest-plugin":"v4.0.0"},'"$(allow pestphp/pest-plugin php-http/discovery)"'}'
set_project discovery '{"name":"vivacity/verbose-discovery","require":{"php-http/discovery":"1.20.0"},'"$(allow php-http/discovery)"'}'
set_project discovery-old '{"name":"vivacity/verbose-discovery-old","require":{"php-http/discovery":"1.15.0"},'"$(allow php-http/discovery)"'}'
set_project pv '{"name":"vivacity/verbose-pv","require":{"composer/package-versions-deprecated":"1.11.99.5"},'"$(allow composer/package-versions-deprecated)"'}'
set_project pv-dev '{"name":"vivacity/verbose-pv-dev","require-dev":{"composer/package-versions-deprecated":"1.11.99.5"},'"$(allow composer/package-versions-deprecated)"'}'
set_project phpstan '{"name":"vivacity/verbose-phpstan","require":{"phpstan/extension-installer":"1.4.3","phpstan/phpstan":"2.3.1","phpstan/phpstan-strict-rules":"2.0.4"},'"$(allow phpstan/extension-installer)"'}'
set_project rector '{"name":"vivacity/verbose-rector","require":{"rector/extension-installer":"0.11.2"},"require-dev":{"rector/rector-symfony":"1.1.0"},'"$(allow rector/extension-installer)"'}'
set_project c3 '{"name":"vivacity/verbose-c3","require":{"codeception/c3":"2.9.0","symfony/runtime":"v7.4.14"},'"$(allow codeception/c3 symfony/runtime)"'}'
set_project c3-old '{"name":"vivacity/verbose-c3","require":{"codeception/c3":"2.8.1","symfony/runtime":"v7.4.14"},'"$(allow codeception/c3 symfony/runtime)"'}'
set_project message '{"name":"vivacity/verbose-message","require":{"drupal/core-project-message":"11.4.6"},'"$(allow drupal/core-project-message)"'}'
set_project unpack '{"name":"vivacity/verbose-unpack","require":{"drupal/core-recipe-unpack":"11.4.6"},'"$(allow drupal/core-recipe-unpack)"'}'
set_project bamarni-dev '{"name":"vivacity/verbose-bamarni-dev","require-dev":{"bamarni/composer-bin-plugin":"1.9.1"},"extra":{"bamarni-bin":{"bin-links":false}},'"$(allow bamarni/composer-bin-plugin)"'}'
set_project bamarni '{"name":"vivacity/verbose-bamarni","require":{"bamarni/composer-bin-plugin":"1.9.1"},"extra":{"bamarni-bin":{"bin-links":false}},'"$(allow bamarni/composer-bin-plugin)"'}'
set_project installers '{"name":"vivacity/verbose-installers","require":{"composer/installers":"v2.3.0","roots/wordpress-core-installer":"v4.0.0"},'"$(allow composer/installers roots/wordpress-core-installer)"'}'
# mnsami avec composer/installers est rendu à Composer dès le niveau normal
# (précédence des installeurs) : à part, avec pest-plugin pour le compteur.
set_project customdir '{"name":"vivacity/verbose-customdir","require":{"mnsami/composer-custom-directory-installer":"2.0.0","pestphp/pest-plugin":"v4.0.0"},'"$(allow mnsami/composer-custom-directory-installer pestphp/pest-plugin)"'}'
set_project combo '{"name":"vivacity/verbose-combo","require":{"symfony/runtime":"v7.4.14","pestphp/pest-plugin":"v4.0.0","php-http/discovery":"1.20.0","composer/package-versions-deprecated":"1.11.99.5","phpstan/extension-installer":"1.4.3","phpstan/phpstan":"2.3.1","codeception/c3":"2.9.0","bamarni/composer-bin-plugin":"1.9.1","drupal/core-project-message":"11.4.6","composer/installers":"v2.3.0"},"require-dev":{"phpstan/phpstan-strict-rules":"2.0.4"},"extra":{"bamarni-bin":{"bin-links":false}},'"$(allow symfony/runtime pestphp/pest-plugin php-http/discovery composer/package-versions-deprecated phpstan/extension-installer codeception/c3 bamarni/composer-bin-plugin drupal/core-project-message composer/installers)"'}'
set_project phpcs '{"name":"vivacity/verbose-phpcs","require":{"dealerdirect/phpcodesniffer-composer-installer":"v1.2.1","squizlabs/php_codesniffer":"4.0.4","phpcsstandards/phpcsutils":"1.2.3"},'"$(allow dealerdirect/phpcodesniffer-composer-installer)"'}'
set_project flex '{"name":"vivacity/verbose-flex","require":{"symfony/flex":"v2.11.0"},'"$(allow symfony/flex)"'}'
# "projet|arguments @préparations" — préparations propres à cette partie :
#   @vendor:plugins  vendor/ posé au préalable par Composer, plugins actifs
#                    (le même état des deux côtés)
#   @from:<projet>   vendor/ posé par Composer depuis le lock d'un autre projet,
#                    puis le composer.json et le lock de celui-ci (une mise à
#                    jour de plugin)
#   @rm:<chemin>     ce chemin du projet supprimé avant la commande
#   @nolock          composer.lock retiré avant la commande
#   @env:NOM=VALEUR  pour les deux commandes comparées
#   @expect:<motif>  la sortie de Composer, quand elle est verbeuse, contient
#                    le motif (un cas qui ne montre plus ce qu'il teste échoue)
#   @silent          Composer n'écrit aucune ligne d'écouteur ; sans lui, une
#                    sortie verbeuse de Composer doit en contenir au moins une
#                    (des plugins inactifs des deux côtés ne prouvent rien)
#   @delegates:<motif>  vivacity rend la commande à Composer, raison incluse
#   @levels:…        comme plus haut (`normal` : le niveau sans option)
ONLINE_CASES=(
  # Le squelette Laravel : pest-plugin (post-autoload-dump) et discovery
  # (pre-autoload-dump, post-update-cmd), aux trois niveaux.
  "laravel-pest|install @expect:^>.post-autoload-dump:.Pest.Plugin.Manager->registerPlugins$"
  "laravel-pest|install @vendor:plugins @expect:^>.pre-autoload-dump:.Http.Discovery"
  "laravel-pest|install --no-dev @vendor:plugins"
  "laravel-pest|update php-http/discovery @vendor:plugins @expect:^>.post-update-cmd:.Http.Discovery.Composer.Plugin->postUpdate$"
  "laravel-pest|update php-http/discovery --no-install @vendor:plugins @expect:^>.post-update-cmd:"
  "laravel-pest|require psr/clock:1.0.0 @vendor:plugins @expect:Plugin_composer_tmp1->preAutoloadDump$"
  "laravel-pest|require psr/clock:1.0.0 --no-install @vendor:plugins @levels:-v @expect:_composer_tmp1->postUpdate$"
  "laravel-pest|remove laravel/tinker @vendor:plugins @expect:Manager_composer_tmp0->registerPlugins$"
  "laravel-pest|dump-autoload @vendor:plugins @expect:^>.post-autoload-dump:.Pest"
  "laravel-pest|dump-autoload -o @vendor:plugins @levels:-v"
  "laravel-pest|install --no-autoloader @vendor:plugins @levels:-v @silent"
  # Aucun événement de script sous --dry-run (Installer.php:292-294).
  "laravel-pest|install --dry-run @levels:-v @silent"
  "laravel-pest|install --no-plugins @levels:-v @silent"
  "laravel-pest|install @vendor:plugins @levels:normal @env:SHELL_VERBOSITY=1 @expect:^>.post-autoload-dump:.Pest"
  # Le répertoire d'un plugin perdu : pas chargé au démarrage, réinstallé et
  # activé en dernier, sans suffixe.
  "laravel-pest|install @vendor:plugins @rm:vendor/pestphp/pest-plugin @levels:-v"
  "laravel-pest|install @vendor:plugins @levels:-vv @delegates:-vv/-vvv"
  # Un plugin à la fois, sous -v.
  "runtime|install @levels:-v @expect:Runtime.Internal.ComposerPlugin->updateAutoloadFile$"
  "runtime|dump-autoload @vendor:plugins @levels:-v"
  "pest|install @levels:-v"
  # Un plugin installé par `require` : activé à son installation, sans
  # suffixe ; une version non reproduite : rendue AVANT d'écrire le lock.
  "pest|require php-http/discovery:1.20.0 @vendor:plugins @levels:-v @expect:Http.Discovery.Composer.Plugin->postUpdate$"
  "pest|require php-http/discovery:1.15.0 @vendor:plugins @levels:-v @delegates:php-http/discovery.*1\.15\.0"
  "discovery|update @vendor:plugins @levels:-v"
  "discovery|update --no-install @vendor:plugins @levels:-v"
  # `install` sans lock : une résolution, donc `post-update-cmd`.
  "discovery|install @nolock @levels:-v @expect:->postUpdate$"
  "pv|install @levels:-v @expect:PackageVersions.Installer->dumpVersionsClass$"
  "pv|install @vendor:plugins @levels:-v"
  # En require-dev sous --no-dev : chargé (la ligne `>` sous -v), mais
  # `getVersions` lit le lock sans packages-dev (COMPOSER_DEV_MODE=0), ne
  # s'y trouve pas et rend la main sans rien écrire.
  "pv-dev|dump-autoload --no-dev @vendor:plugins @levels:normal,-v @expect:PackageVersions.Installer->dumpVersionsClass$"
  "pv-dev|install --no-dev @vendor:plugins @levels:normal,-v @silent"
  "phpstan|install @levels:-v @expect:^>.post-install-cmd:.PHPStan"
  "phpstan|install @vendor:plugins @levels:-v"
  # Sous --no-install, `post-update-cmd` part quand même : la ligne de
  # chaque écouteur, aussitôt suivie de ce qu'il écrit (phpstan : « Extensions
  # installed » ; GeneratedConfig.php perdu : réécrit et listé ; c3.php
  # perdu : recopié), dans l'ordre d'activation.
  "phpstan|update --no-install @vendor:plugins @levels:normal,-v @expect:Plugin->process$"
  "phpstan|update --no-install @vendor:plugins @rm:vendor/phpstan/extension-installer/src/GeneratedConfig.php @levels:-v @expect:^>.phpstan/phpstan-strict-rules:.installed$"
  "phpstan|require psr/clock:1.0.0 --no-install @vendor:plugins @levels:-v @expect:Plugin_composer_tmp0->process$"
  "rector|update --no-install @vendor:plugins @rm:vendor/rector/extension-installer/src/GeneratedConfig.php @levels:-v @expect:^rector/rector-installer:.Extensions.installed$"
  "c3|update --no-install @vendor:plugins @rm:c3.php @levels:-v @expect:Copying.c3.php"
  "combo|update --no-install @vendor:plugins @rm:c3.php @levels:-v @expect:Installer->askForUpdateV2$"
  "rector|install @levels:-v @expect:Rector.RectorInstaller.Plugin->process$"
  "rector|install --no-dev @vendor:plugins @levels:-v"
  "c3|install @levels:-v @expect:Installer->copyC3V2$"
  "c3|update @vendor:plugins @levels:-v @expect:Installer->askForUpdateV2$"
  # Une mise à jour de plugin (PluginInstaller::update) : sa classe est
  # rechargée sous un autre nom — après le rechargement de `require`.
  "c3|install @from:c3-old @levels:-v @expect:Installer_composer_tmp0->copyC3V2$"
  "c3-old|require codeception/c3:2.9.0 @vendor:plugins @levels:-v @expect:Installer_composer_tmp2->askForUpdateV2$"
  "message|install @levels:-v @expect:MessagePlugin->displayPostCreateMessage$"
  "unpack|update @vendor:plugins @levels:-v @expect:RecipeUnpack.Plugin->unpackOnRequire$"
  # Le plugin retiré n'est plus là à `post-update-cmd` : aucune ligne.
  "unpack|remove drupal/core-recipe-unpack @vendor:plugins @levels:-v @silent"
  "bamarni|install @levels:-v"
  "bamarni|install @vendor:plugins @levels:-v @expect:^.bamarni-bin..Calling.onCommandEvent"
  "bamarni|dump-autoload @vendor:plugins @levels:-v"
  "bamarni|update --no-install @vendor:plugins @levels:-v"
  "bamarni|require psr/clock:1.0.0 @vendor:plugins @levels:-v @expect:BamarniBinPlugin_composer_tmp0->onCommandEvent$"
  # `command` part même sous --dry-run (seuls les événements de script sont
  # coupés), et `require`/`remove` rechargent les plugins.
  "bamarni|install --dry-run @vendor:plugins @levels:-v @expect:^>.command:"
  "bamarni|update --dry-run @vendor:plugins @levels:-v @expect:^>.command:"
  "bamarni|require psr/clock:1.0.0 --dry-run @vendor:plugins @levels:-v @expect:BamarniBinPlugin_composer_tmp0->onCommandEvent$"
  "bamarni|remove bamarni/composer-bin-plugin --dry-run @vendor:plugins @levels:-v @expect:BamarniBinPlugin_composer_tmp0->onCommandEvent$"
  "bamarni|require psr/clock:1.0.0 --no-update @vendor:plugins @levels:-v @silent"
  # Chargé, donc appelé, quel que soit le mode du dump (en require-dev sous
  # --no-dev) ; ses classes absentes du disque : pas chargé au démarrage (pas
  # de `command`), réinstallé puis appelé. (Le même `dump-autoload` sur le
  # répertoire perdu : ni chargé ni appelé, et le paquet purgé de
  # l'autoloader comme `Factory::purgePackages` le fait.)
  "bamarni-dev|dump-autoload --no-dev @vendor:plugins @levels:normal,-v @expect:Calling.onPostAutoloadDump"
  "bamarni|install @vendor:plugins @rm:vendor/bamarni/composer-bin-plugin @levels:normal,-v"
  "bamarni|dump-autoload @vendor:plugins @rm:vendor/bamarni/composer-bin-plugin @levels:normal,-v @silent"
  # Une bibliothèque au répertoire perdu : hors de l'autoloader régénéré.
  "laravel-pest|dump-autoload @vendor:plugins @rm:vendor/laravel/tinker @levels:normal"
  # Sans écouteur : rien de plus sous -v, mais leurs classes comptent dans
  # le compteur de `require` (custom-directory-installer 2.0.0 en déclare
  # trois : pest-plugin, chargé après lui, prend le quatrième suffixe).
  "installers|install @levels:-v @silent"
  "installers|require psr/clock:1.0.0 @vendor:plugins @levels:-v @silent"
  "customdir|install @levels:-v"
  "customdir|require psr/clock:1.0.0 @vendor:plugins @levels:-v @expect:Manager_composer_tmp3->registerPlugins$"
  # Plusieurs plugins : l'ordre d'activation à chaque événement, le compteur.
  "combo|install @levels:-v"
  "combo|install @vendor:plugins @levels:-v"
  "combo|install --no-dev @vendor:plugins @levels:-v"
  "combo|install --no-autoloader @vendor:plugins @levels:-v @expect:^>.post-install-cmd:"
  "combo|update @vendor:plugins @levels:-v"
  "combo|require psr/clock:1.0.0 @vendor:plugins @levels:-v @expect:ComposerPlugin_composer_tmp8->updateAutoloadFile$"
  "combo|remove drupal/core-project-message @vendor:plugins @levels:-v"
  "combo|remove bamarni/composer-bin-plugin @vendor:plugins @levels:-v"
  "combo|dump-autoload @vendor:plugins @levels:-v"
  # Rendus à Composer sous -v, la raison nommée.
  "phpcs|install @levels:-v @delegates:dealerdirect/phpcodesniffer-composer-installer.*phpcs"
  "flex|install @levels:-v @delegates:symfony/flex"
  "discovery-old|install @levels:-v @delegates:php-http/discovery.*1\.15\.0"
)
ONLINE_WORK="$WORK/online"
online_lock() { # <projet> : écrit une fois par passage, ou figé (fixtures/projects/<projet>) sans manifeste ici
  local d="$ONLINE_WORK/locks/$1"
  [ -f "$d/composer.lock" ] && return 0
  rm -rf "$d"; mkdir -p "$d"
  local pv; pv=$(proj_var "$1")
  if [ -z "${!pv+set}" ]; then
    cp "$ROOT/fixtures/projects/$1/composer.json" "$ROOT/fixtures/projects/$1/composer.lock" "$d/"
    return 0
  fi
  printf '%s\n' "${!pv}" > "$d/composer.json"
  (cd "$d" && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" composer update --no-install --no-plugins --no-scripts --no-audit --no-security-blocking -n -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; return 1; }
}
online_stage() { # <projet> <répertoire> <préparations…>
  local p="$1" d="$2"; shift 2
  rm -rf "$d"; mkdir -p "$d"
  local prep from=""
  for prep in "$@"; do case "$prep" in @from:*) from="${prep#@from:}" ;; esac; done
  if [ -n "$from" ]; then
    online_lock "$from"
    cp "$ONLINE_WORK/locks/$from/composer.json" "$ONLINE_WORK/locks/$from/composer.lock" "$d/"
    (cd "$d" && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" composer install --no-scripts --no-progress -n -q) || return 1
  fi
  cp "$ONLINE_WORK/locks/$p/composer.json" "$ONLINE_WORK/locks/$p/composer.lock" "$d/"
  for prep in "$@"; do
    case "$prep" in
      @vendor:plugins) (cd "$d" && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" composer install --no-scripts --no-progress -n -q) || return 1 ;;
    esac
  done
  for prep in "$@"; do
    case "$prep" in
      @rm:*) rm -rf "${d:?}/${prep#@rm:}" ;;
      @nolock) rm -f "$d/composer.lock" ;;
    esac
  done
}
online_filter() { # <niveau> : les lignes propres à vivacity, tolérées hors -q
  if [ "$1" = -q ]; then cat; else grep -v -E '^(vivacity|Note): ' || true; fi
}
# Les lignes `  - Downloading` dépendent du cache, pas de l'outil : la
# référence passe la première et télécharge ce que la machine n'a jamais vu,
# vivacity le trouve ensuite en cache. Hors comparaison des deux côtés, comme
# dans scripts.sh et yii2-composer.sh (download-lines.sh les compare à part).
online_cold() { grep -v -E '^  - Downloading [^ ]+ \(' || true; }
if [[ " ${FIXTURES[*]} " == *" online "* ]]; then
  rm -rf "$ONLINE_WORK"; mkdir -p "$ONLINE_WORK/home"
  ONLINE_HOME="$ONLINE_WORK/home"
  # Le cache de fichiers habituel (téléchargements partagés), les jetons
  # d'auth.json ; rien d'autre de la configuration globale.
  CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
  GLOBAL_HOME="$(composer config --global home 2>/dev/null || true)"
  [ -n "$GLOBAL_HOME" ] && [ -f "$GLOBAL_HOME/auth.json" ] && cp "$GLOBAL_HOME/auth.json" "$ONLINE_HOME/"
  n=0
  for spec in "${ONLINE_CASES[@]}"; do
    # `VERBOSITY_ONLINE_ONLY=<motif>` : seulement les cas qui y correspondent.
    [[ -z "${VERBOSITY_ONLINE_ONLY:-}" || "$spec" =~ ${VERBOSITY_ONLINE_ONLY} ]] || continue
    p="${spec%%|*}"
    read -r -a sargs <<< "${spec#*|}"
    preps=()
    while [ ${#sargs[@]} -gt 0 ]; do
      last=$(( ${#sargs[@]} - 1 ))
      case "${sargs[$last]}" in
        @*) preps+=("${sargs[$last]}"); unset "sargs[$last]" ;;
        *) break ;;
      esac
    done
    levels=("${LEVELS[@]}"); expect=""; delegates=""; silent=0; envs=()
    for prep in "${preps[@]+"${preps[@]}"}"; do
      case "$prep" in
        @levels:*) IFS=, read -r -a levels <<< "${prep#@levels:}" ;;
        @expect:*) expect="${prep#@expect:}" ;;
        @delegates:*) delegates="${prep#@delegates:}" ;;
        @silent) silent=1 ;;
        @env:*) envs+=("${prep#@env:}") ;;
        @vendor:plugins|@from:*|@rm:*|@nolock) ;;
        *) echo "prep inconnu : $prep"; exit 1 ;;
      esac
    done
    online_lock "$p" || { status=1; continue; }
    # Options propres à chaque commande, des deux côtés.
    extra=(--no-progress)
    case "${sargs[0]}" in
      update|require|remove) extra=(--no-progress --no-audit --no-security-blocking) ;;
      dump-autoload) extra=() ;;
    esac
    for level in "${levels[@]}"; do
      [ "$level" = normal ] && level=""
      n=$((n + 1)); flags=(); [ -n "$level" ] && flags=("$level")
      label="online $p ${sargs[*]} ${level:-(normal)}"; [ ${#preps[@]} -gt 0 ] && label="$label (${preps[*]})"
      # Verbeux pour Composer : -v, ou SHELL_VERBOSITY=1 sans option.
      verbose=0
      if [ "$level" = -v ] || { [ -z "$level" ] && [[ " ${envs[*]+"${envs[*]}"} " == *" SHELL_VERBOSITY=1 "* ]]; }; then verbose=1; fi
      ok=1
      for mode in split merged; do
        for side in ref viv; do
          online_stage "$p" "$ONLINE_WORK/$side-$mode-$n" "${preps[@]+"${preps[@]}"}" \
            || { echo "FAIL $label : préparation ($side, $mode) en échec"; ok=0; }
        done
        ref_code=0; viv_code=0
        if [ "$mode" = split ]; then
          (cd "$ONLINE_WORK/ref-$mode-$n" && for e in "${envs[@]+"${envs[@]}"}"; do export "$e"; done && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" \
            composer "${sargs[@]}" "${extra[@]+"${extra[@]}"}" --no-scripts --no-interaction "${flags[@]+"${flags[@]}"}" \
            >"$ONLINE_WORK/$n.composer.out" 2>"$ONLINE_WORK/$n.composer.err") || ref_code=$?
          (cd "$ONLINE_WORK/viv-$mode-$n" && for e in "${envs[@]+"${envs[@]}"}"; do export "$e"; done && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" \
            "$VIVACITY" "${sargs[@]}" "${extra[@]+"${extra[@]}"}" "${flags[@]+"${flags[@]}"}" \
            >"$ONLINE_WORK/$n.vivacity.out" 2>"$ONLINE_WORK/$n.vivacity.err") || viv_code=$?
        else
          (cd "$ONLINE_WORK/ref-$mode-$n" && for e in "${envs[@]+"${envs[@]}"}"; do export "$e"; done && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" \
            composer "${sargs[@]}" "${extra[@]+"${extra[@]}"}" --no-scripts --no-interaction "${flags[@]+"${flags[@]}"}" \
            >"$ONLINE_WORK/$n.composer.all" 2>&1) || ref_code=$?
          (cd "$ONLINE_WORK/viv-$mode-$n" && for e in "${envs[@]+"${envs[@]}"}"; do export "$e"; done && COMPOSER_HOME="$ONLINE_HOME" COMPOSER_CACHE_DIR="$CACHE_DIR" \
            "$VIVACITY" "${sargs[@]}" "${extra[@]+"${extra[@]}"}" "${flags[@]+"${flags[@]}"}" \
            >"$ONLINE_WORK/$n.vivacity.all" 2>&1) || viv_code=$?
        fi
        if [ "$ref_code" != "$viv_code" ]; then
          echo "FAIL $label : code retour ($mode) composer=$ref_code vivacity=$viv_code"; ok=0
        fi
      done
      # Délégation : exigée par `@delegates:` (la raison sur une ligne de
      # vivacity), refusée sinon (sous -v, la sortie de Composer passerait
      # des deux côtés et ne prouverait rien).
      if [ -n "$delegates" ]; then
        if ! grep -q '^vivacity: delegating to ' "$ONLINE_WORK/$n.vivacity.err" \
           || ! grep -E '^(vivacity: |  - plugin )' "$ONLINE_WORK/$n.vivacity.err" | grep -qE -- "$delegates"; then
          echo "FAIL $label : vivacity ne rend pas la commande avec la raison « $delegates »"; grep -E '^(vivacity: |  - plugin )' "$ONLINE_WORK/$n.vivacity.err" | head -3 || true; ok=0
        fi
        # Avant la ligne « delegating to », seules les lignes propres à
        # vivacity (sa raison) : une ligne de la commande elle-même serait
        # écrite deux fois, Composer la réécrivant. Ce qui suit est comparé.
        for f in err all; do
          before=$(sed -n '/^vivacity: delegating to /q;p' "$ONLINE_WORK/$n.vivacity.$f" | grep -v -E '^(vivacity: |  - plugin |Note: )' || true)
          if [ -n "$before" ]; then
            echo "FAIL $label : vivacity écrit avant de rendre la main (Composer le réécrit)"; printf '%s\n' "$before" | head -4; ok=0
          fi
          sed -n '/^vivacity: delegating to /,$p' "$ONLINE_WORK/$n.vivacity.$f" | tail -n +2 > "$ONLINE_WORK/$n.vivacity.$f.cut"
          mv "$ONLINE_WORK/$n.vivacity.$f.cut" "$ONLINE_WORK/$n.vivacity.$f"
        done
      elif grep -qE '^vivacity: (delegating to|.*outside what vivacity)' "$ONLINE_WORK/$n.vivacity.err"; then
        echo "FAIL $label : vivacity rend la commande à Composer"; grep '^vivacity' "$ONLINE_WORK/$n.vivacity.err" | head -2 || true; ok=0
      fi
      if [ "$verbose" = 1 ] && [ -n "$expect" ] && ! grep -qE -- "$expect" "$ONLINE_WORK/$n.composer.all"; then
        echo "FAIL $label : « $expect » absent chez Composer (cas mal construit)"; ok=0
      fi
      if [ "$verbose" = 1 ] && [ -z "$delegates" ]; then
        listeners=$(grep -c '^> ' "$ONLINE_WORK/$n.composer.err" || true)
        if [ "$silent" = 1 ] && [ "$listeners" != 0 ]; then
          echo "FAIL $label : $listeners ligne(s) d'écouteur chez Composer, aucune attendue (cas mal construit)"; ok=0
        elif [ "$silent" = 0 ] && [ "$listeners" = 0 ]; then
          echo "FAIL $label : aucune ligne d'écouteur chez Composer (plugins inactifs ? cas mal construit)"; ok=0
        fi
      fi
      for stream in out err; do
        normalize "$ONLINE_WORK/$n.composer.$stream" "$ONLINE_WORK/ref-split-$n" | online_cold > "$ONLINE_WORK/$n.composer.$stream.n"
        if [ "$stream" = err ]; then
          normalize "$ONLINE_WORK/$n.vivacity.$stream" "$ONLINE_WORK/viv-split-$n" | online_filter "$level" | online_cold > "$ONLINE_WORK/$n.vivacity.$stream.n"
        else
          normalize "$ONLINE_WORK/$n.vivacity.$stream" "$ONLINE_WORK/viv-split-$n" > "$ONLINE_WORK/$n.vivacity.$stream.n"
        fi
        if ! diff -q "$ONLINE_WORK/$n.composer.$stream.n" "$ONLINE_WORK/$n.vivacity.$stream.n" >/dev/null; then
          echo "FAIL $label : std$stream diffère"
          diff "$ONLINE_WORK/$n.composer.$stream.n" "$ONLINE_WORK/$n.vivacity.$stream.n" | head -12 || true; ok=0
        fi
      done
      normalize "$ONLINE_WORK/$n.composer.all" "$ONLINE_WORK/ref-merged-$n" | online_cold > "$ONLINE_WORK/$n.composer.all.n"
      normalize "$ONLINE_WORK/$n.vivacity.all" "$ONLINE_WORK/viv-merged-$n" | online_filter "$level" | online_cold > "$ONLINE_WORK/$n.vivacity.all.n"
      if ! diff -q "$ONLINE_WORK/$n.composer.all.n" "$ONLINE_WORK/$n.vivacity.all.n" >/dev/null; then
        echo "FAIL $label : sortie fusionnée différente (ordre)"
        diff "$ONLINE_WORK/$n.composer.all.n" "$ONLINE_WORK/$n.vivacity.all.n" | head -12 || true; ok=0
      fi
      if ! compare_vendor "$ONLINE_WORK/ref-split-$n" "$ONLINE_WORK/viv-split-$n" "$ONLINE_WORK/$n.diff" >/dev/null; then
        echo "FAIL $label : projet différent"; head -8 "$ONLINE_WORK/$n.diff"; ok=0
      fi
      if [ "$ok" = 1 ]; then
        echo "OK   $label : stdout, stderr, sortie fusionnée et projet identiques (code $ref_code, $(grep -c '^> ' "$ONLINE_WORK/$n.composer.err" || true) ligne(s) d'écouteurs)"
      else
        status=1
      fi
      rm -rf "$ONLINE_WORK/ref-split-$n" "$ONLINE_WORK/viv-split-$n" "$ONLINE_WORK/ref-merged-$n" "$ONLINE_WORK/viv-merged-$n"
    done
  done
fi
exit $status
