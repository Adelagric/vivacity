#!/usr/bin/env bash
# Le corpus : des projets PHP réels (fixtures/corpus/<nom>/ : composer.json,
# composer.lock, l'arbre minimal de l'autoload racine, PROVENANCE — voir
# fixtures/corpus/README.md et tools/corpus-add.sh) pour mesurer quelle part
# du monde réel `vivacity install` sert nativement.
#
# Deux temps, deux modes (avec les paquets dev, et --no-dev) :
#   --scan   hors ligne : `vivacity install --check-scope` classe chaque
#            entrée (natif prévu / fallback + raisons) sans rien télécharger ;
#            relève aussi les `scripts` déclarés, les exigences de plateforme
#            et les plugins ignorés comme bibliothèques.
#   (défaut) le scan, puis pour les entrées prévues natives le double install :
#            `composer install --no-scripts` (plugins chargés — la référence est
#            un Composer SANS scripts, qui coupe aussi les écouteurs de plugins
#            sur ces événements) contre `vivacity install --no-fallback`, avec
#            --ignore-platform-req=ext-* des deux côtés (le `php` reste :
#            platform_check.php est exercé) ; vendor/ comparé par
#            harness/lib/compare.sh (diff + inventaire modes/liens).
# Trois seaux : native (0 diff), fallback (vivacity s'est arrêté avant toute
# écriture, raisons), diff (vivacity a écrit autre chose ou a échoué là où
# Composer a réussi — un bug). `unavailable` : Composer lui-même a échoué
# (réseau, PHP…) — rapporté, pas compté.
#
# Sortie : une ligne JSON par (entrée, mode) dans $WORK/corpus/results.jsonl ;
# tools/corpus-report.py en fait docs/corpus/<date>.md. Le cache Composer est
# partagé par les deux outils (COMPOSER_CACHE_DIR explicite) ; ref/ et viv/
# sont supprimés après chaque comparaison.
#
# Usage : harness/corpus.sh [--scan] [--only <nom>[,<nom>…]] [--modes dev,no-dev]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="$ROOT/target/release/vivacity"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/corpus"
scan_only=0; only=""; modes="dev,no-dev"
while [ $# -gt 0 ]; do
  case "$1" in
    --scan) scan_only=1 ;;
    --only) only="$2"; shift ;;
    --modes) modes="$2"; shift ;;
    *) echo "argument inconnu : $1"; exit 1 ;;
  esac
  shift
done
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK/cache" "$WORK/home"
harness_git_env "$WORK"
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$WORK/cache" COMPOSER_NO_INTERACTION=1 COMPOSER_TESTS_ARE_RUNNING=1
results="$WORK/results.jsonl"; : > "$results"
composer_version="$(composer --version --no-ansi 2>/dev/null | sed -n 's/^Composer version \([^ ]*\).*/\1/p')"
vivacity_version="$("$VIVACITY" --version | awk '{print $2}')"
IFS=',' read -r -a mode_list <<< "$modes"

entries=()
for d in "$ROOT"/fixtures/corpus/*/; do
  n="$(basename "$d")"
  if [ -n "$only" ]; then case ",$only," in *",$n,"*) ;; *) continue ;; esac; fi
  entries+=("$n")
done
[ ${#entries[@]} -gt 0 ] || { echo "corpus vide"; exit 1; }

# Une copie de l'entrée avec un dépôt git identique des deux côtés (version
# racine devinée, comme diff-vendor.sh).
stage() { # nom, destination
  rm -rf "$2"; mkdir -p "$2"
  (cd "$ROOT/fixtures/corpus/$1" && tar -cf - --exclude=PROVENANCE .) | (cd "$2" && tar -xf -)
  (cd "$2" && git init -q -b main && git add -A >/dev/null 2>&1 && \
    GIT_AUTHOR_NAME=vivacity GIT_AUTHOR_EMAIL=v@v GIT_AUTHOR_DATE="2026-09-10T00:00:00Z" \
    GIT_COMMITTER_NAME=vivacity GIT_COMMITTER_EMAIL=v@v GIT_COMMITTER_DATE="2026-09-10T00:00:00Z" \
    git commit -q -m fixture)
}

json_escape() { jq -Rs . ; }

# Le scan d'une entrée dans un mode : seau prévu et raisons, depuis la
# sortie de `install --check-scope` (code 0 natif, 3 hors périmètre).
scan_one() { # nom, mode
  local n="$1" mode="$2" d="$WORK/scan-$1" flags=() code=0
  [ "$mode" = "no-dev" ] && flags+=(--no-dev)
  stage "$n" "$d"
  # Périmètre seulement : la plateforme locale (extensions, version de PHP)
  # n'entre pas dans le seau prévu ; elle est relevée à part.
  (cd "$d" && "$VIVACITY" install --check-scope --ignore-platform-reqs "${flags[@]+"${flags[@]}"}" >"$d/scope.out" 2>"$d/scope.err") || code=$?
  # La colonne plateforme : ce que cette machine ne satisfait pas (version
  # de PHP, extension absente) — le double install ignorera `ext-*` et, si
  # une exigence `php` échoue ici, `php` aussi (noté).
  local platform_failures
  platform_failures="$(cd "$d" && "$VIVACITY" install --check-scope "${flags[@]+"${flags[@]}"}" 2>&1 | grep -E '^  - .*: (Missing|Mismatch|Unsupported)' | sed 's/^  - //' | jq -Rs 'split("\n") | map(select(. != ""))')"
  local bucket reasons benign
  reasons="$(grep -E '^  - ' "$d/scope.err" | sed 's/^  - //' | jq -Rs 'split("\n") | map(select(. != ""))')"
  benign="$(grep -E '^Note: plugin ' "$d/scope.err" | sed -E 's/^Note: plugin ([^ ]+) .*/\1/' | jq -Rs 'split("\n") | map(select(. != ""))')"
  case "$code" in
    0) bucket="native" ;;
    3) bucket="fallback" ;;
    *) bucket="error" ;;
  esac
  local manifest="$ROOT/fixtures/corpus/$n/composer.json" lock="$ROOT/fixtures/corpus/$n/composer.lock"
  jq -n -c \
    --arg name "$n" --arg mode "$mode" --arg predicted "$bucket" --argjson reasons "$reasons" --argjson benign "$benign" --argjson platform_failures "$platform_failures" \
    --arg provenance "$(cat "$ROOT/fixtures/corpus/$n/PROVENANCE")" \
    --argjson scripts "$(jq -c '.scripts // {} | keys' "$manifest")" \
    --argjson platform "$(jq -c '[(.require // {}), (.["require-dev"] // {})] | add | to_entries | map(select(.key | test("^(php|ext-|lib-)"))) | map(.key + " " + .value)' "$manifest")" \
    --argjson packages "$(jq -c "[.packages[], (if \"$mode\" == \"dev\" then .[\"packages-dev\"][] else empty end)] | map(.name + \"@\" + .version)" "$lock")" \
    --argjson plugins "$(jq -c "[.packages[], (if \"$mode\" == \"dev\" then .[\"packages-dev\"][] else empty end)] | map(select(.type == \"composer-plugin\") | .name)" "$lock")" \
    '{name: $name, mode: $mode, predicted: $predicted, reasons: $reasons, benign_plugins: $benign, platform_failures: $platform_failures, scripts: $scripts, platform: $platform, packages: $packages, plugins: $plugins, provenance: $provenance}'
  rm -rf "$d"
}

# Le double install d'une entrée prévue native : seau final, temps, diff.
run_one() { # nom, mode, ligne JSON du scan
  local n="$1" mode="$2" scan="$3" ref="$WORK/ref-$1" viv="$WORK/viv-$1" flags=() ref_code=0 viv_code=0 t0 t_ref t_viv bucket="native" detail=""
  [ "$mode" = "no-dev" ] && flags+=(--no-dev)
  # `ext-*` toujours ignoré ; `php` seulement quand cette machine ne le
  # satisfait pas (PHP local trop récent pour le lock), d'après le scan.
  local ignore=(--ignore-platform-req='ext-*')
  if echo "$scan" | jq -e '.platform_failures | map(select(startswith("php "))) | length > 0' >/dev/null; then ignore+=(--ignore-platform-req=php); fi
  stage "$n" "$ref"; stage "$n" "$viv"
  t0=$(date +%s)
  (cd "$ref" && composer install --no-scripts --no-interaction --no-ansi "${ignore[@]}" "${flags[@]+"${flags[@]}"}" >"$WORK/$n.$mode.composer.log" 2>"$WORK/$n.$mode.composer.err") || ref_code=$?
  t_ref=$(( $(date +%s) - t0 )); t0=$(date +%s)
  (cd "$viv" && "$VIVACITY" install --no-fallback "${ignore[@]}" "${flags[@]+"${flags[@]}"}" >"$WORK/$n.$mode.vivacity.log" 2>"$WORK/$n.$mode.vivacity.err") || viv_code=$?
  t_viv=$(( $(date +%s) - t0 ))
  if [ "$ref_code" != 0 ]; then
    bucket="unavailable"; detail="$(tail -5 "$WORK/$n.$mode.composer.err")"
  elif grep -q "Failed to set PHP CodeSniffer" "$WORK/$n.$mode.composer.log"; then
    # Le plugin de référence a échoué sur cette machine (phpcs lui-même,
    # sous le PHP local) : Composer n'a pas écrit ce qu'il aurait écrit.
    bucket="unavailable"; detail="reference plugin failed: $(grep -m1 "Failed to set PHP CodeSniffer" "$WORK/$n.$mode.composer.log")"
  elif [ "$viv_code" = 3 ]; then
    bucket="fallback"; detail="$(grep -E '^  - ' "$WORK/$n.$mode.vivacity.err")"
  elif [ "$viv_code" != 0 ]; then
    bucket="diff"; detail="vivacity exit $viv_code: $(tail -5 "$WORK/$n.$mode.vivacity.err")"
  else
    local scope_ref="$ref/vendor" scope_viv="$viv/vendor"
    if jq -e '.config["allow-plugins"]["composer/installers"] == true' "$ref/composer.json" >/dev/null 2>&1; then scope_ref="$ref"; scope_viv="$viv"; fi
    if ! compare_vendor "$scope_ref" "$scope_viv" "$WORK/$n.$mode.diff" >/dev/null; then
      bucket="diff"; detail="$(head -20 "$WORK/$n.$mode.diff")"
    fi
  fi
  echo "$scan" | jq -c --arg bucket "$bucket" --arg detail "$detail" --argjson t_ref "$t_ref" --argjson t_viv "$t_viv" --arg ignored "${ignore[*]}" \
    '. + {bucket: $bucket, detail: $detail, seconds_composer: $t_ref, seconds_vivacity: $t_viv, ignored: $ignored}'
  rm -rf "$ref" "$viv"
}

echo "corpus : ${#entries[@]} entrées, modes ${modes}, Composer $composer_version, vivacity $vivacity_version"
for mode in "${mode_list[@]}"; do
  for n in "${entries[@]}"; do
    scan="$(scan_one "$n" "$mode")"
    predicted="$(echo "$scan" | jq -r .predicted)"
    if [ "$scan_only" = 1 ] || [ "$predicted" != "native" ]; then
      echo "$scan" | jq -c '. + {bucket: .predicted}' >> "$results"
      printf '%-10s %-8s %s\n' "$predicted" "$mode" "$n$( [ "$predicted" = native ] || echo " — $(echo "$scan" | jq -r '.reasons | join("; ")')")"
      continue
    fi
    line="$(run_one "$n" "$mode" "$scan")"
    echo "$line" >> "$results"
    printf '%-10s %-8s %s (composer %ss, vivacity %ss)\n' "$(echo "$line" | jq -r .bucket)" "$mode" "$n" "$(echo "$line" | jq -r .seconds_composer)" "$(echo "$line" | jq -r .seconds_vivacity)"
  done
done
echo "résultats : $results"
jq -r '"\(.mode) \(.bucket)"' "$results" | sort | uniq -c
