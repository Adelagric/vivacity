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
# Usage : harness/verbosity.sh [fixture...]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/registry.sh
. "$ROOT/harness/lib/registry.sh"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
VIVACITY="$ROOT/target/release/vivacity"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/verbosity"
FIXTURES=("$@"); [ ${#FIXTURES[@]} -eq 0 ] && FIXTURES=(laravel symfony solver-problems solver-conflict path-repos)
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
  # natif au niveau normal, rendu à Composer sous -v (les lignes de ses
  # écouteurs), identique des deux côtés.
  "symfony|install --dry-run @plugins"
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
    levels=("${LEVELS[@]}"); plugins=(--no-plugins); unsets=(); rootwarn=""; files=()
    for prep in "${preps[@]+"${preps[@]}"}"; do
      case "$prep" in
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
            @env:*|@warm|@warm:update|@vendor|@levels:*|@plugins|@unset:*|@rootwarn:*|@files:*) ;;
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
exit $status
