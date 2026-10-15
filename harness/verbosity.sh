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
#   @warm     le cache de l'instantané rempli au préalable par un install
#             Composer jetable : les deux côtés lisent les dists du cache
#             (les lignes « Downloading » ne dépendent pas de l'ordre)
#   @plugins  plugins actifs des deux côtés (par défaut `--no-plugins` des
#             deux côtés : le même régime, sinon la comparaison ne dirait
#             rien des plugins)
#   @levels:… les niveaux joués pour ce cas (par défaut normal, -q, -v),
#             séparés par des virgules ; `-vv` rend la commande à Composer
CASES=(
  # install depuis le lock, sans rien écrire : durée de résolution, `Installs:`.
  "laravel|install --dry-run"
  "laravel|install --dry-run --no-dev"
  # Lock périmé : l'avertissement est écrit au niveau QUIET chez Composer.
  "laravel|install --dry-run @require:psr/log=^3.0"
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
)
LEVELS=("" "-q" "-v")
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK"
harness_git_env "$WORK"
# `COMPOSER_ROOT_VERSION` : sans lui, Composer avertit qu'il ne devine pas la
# version du projet racine (« Composer could not detect the root package… »),
# une ligne que vivacity n'écrit pas encore — écart connu, hors verbosité.
export CI=1 COMPOSER_TESTS_ARE_RUNNING=1 COMPOSER_ROOT_VERSION=1.0.0
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
    preps=(); envs=()
    while [ ${#sargs[@]} -gt 0 ]; do
      last=$(( ${#sargs[@]} - 1 ))
      case "${sargs[$last]}" in
        @*) preps+=("${sargs[$last]}"); unset "sargs[$last]" ;;
        *) break ;;
      esac
    done
    levels=("${LEVELS[@]}"); plugins=(--no-plugins)
    for prep in "${preps[@]+"${preps[@]}"}"; do
      case "$prep" in
        @levels:*) IFS=, read -r -a levels <<< "${prep#@levels:}" ;;
        @plugins) plugins=() ;;
      esac
    done
    for level in "${levels[@]}"; do
      n=$((n + 1))
      for prep in "${preps[@]+"${preps[@]}"}"; do
        case "$prep" in @env:*) kv="${prep#@env:}"; export "${kv%%=*}=${kv#*=}"; envs+=("${kv%%=*}") ;; esac
      done
      for prep in "${preps[@]+"${preps[@]}"}"; do
        if [ "$prep" = @warm ] && [ ! -f "$home/warmed" ]; then
          w="$WORK/warm-$fx"; stage_project "$fx" "$w"
          (cd "$w" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" composer install --no-scripts --no-plugins --no-autoloader --no-progress -q) && touch "$home/warmed"
        fi
      done
      for side in ref viv; do
        d="$WORK/$side-$fx-$n"; stage_project "$fx" "$d"
        for prep in "${preps[@]+"${preps[@]}"}"; do
          case "$prep" in
            @env:*|@warm|@levels:*|@plugins) ;;
            @vendor) (cd "$d" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" composer install --no-scripts --no-plugins --no-autoloader --no-progress -q) ;;
            @nolock) rm -f "$d/composer.lock" ;;
            @nojson) rm -f "$d/composer.json" "$d/composer.lock" ;;
            @require:*) kv="${prep#@require:}"; jq --arg p "${kv%%=*}" --arg c "${kv#*=}" '.require[$p] = $c' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json" ;;
            @jq:*) jq "${prep#@jq:}" "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json" ;;
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
      (cd "$WORK/ref-$fx-$n" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" \
        composer "${sargs[@]}" "${audit[@]+"${audit[@]}"}" "${progress[@]+"${progress[@]}"}" --no-scripts "${plugins[@]+"${plugins[@]}"}" --no-interaction "${flags[@]+"${flags[@]}"}" \
        >"$WORK/$fx-$n.composer.out" 2>"$WORK/$fx-$n.composer.err") || ref_code=$?
      viv_code=0
      (cd "$WORK/viv-$fx-$n" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" \
        "$VIVACITY" "${sargs[@]}" "${progress[@]+"${progress[@]}"}" "${plugins[@]+"${plugins[@]}"}" "${flags[@]+"${flags[@]}"}" \
        >"$WORK/$fx-$n.vivacity.out" 2>"$WORK/$fx-$n.vivacity.err") || viv_code=$?
      for e in "${envs[@]+"${envs[@]}"}"; do unset "$e"; done; envs=()
      label="$fx ${sargs[*]} ${level:-(normal)}"; [ ${#preps[@]} -gt 0 ] && label="$label (${preps[*]})"
      ok=1
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
      if [ "$ok" = 1 ]; then
        echo "OK   $label : stdout et stderr identiques (code $ref_code, $(wc -l < "$WORK/$fx-$n.composer.err" | tr -d ' ') lignes)"
      else
        status=1
      fi
    done
  done
done
exit $status
