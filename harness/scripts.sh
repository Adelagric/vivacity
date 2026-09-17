#!/usr/bin/env bash
# Harness v0.14 : `vivacity install --run-scripts` contre `composer install`
# (scripts actifs) sur fixtures/projects/scripts — quatre événements qui
# journalisent (`scripts.log` : événement, COMPOSER_DEV_MODE, callables
# statiques avec l'API Composer), en dev puis --no-dev, puis
# `dump-autoload -o` ; un script qui échoue (code 7) ; `--no-autoloader`
# (aucun événement d'autoload). Journal, vendor/ et codes retour comparés.
# Écart documenté : le drapeau `optimize` des événements d'autoload
# (`run-script` ne transmet aucun drapeau) — le champ est neutralisé.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="$ROOT/target/release/vivacity"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/scripts"
SRC="$ROOT/fixtures/projects/scripts"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
mkdir -p "$WORK"; harness_git_env "$WORK"

stage() { # destination, filtre jq
  rm -rf "$1"; mkdir -p "$1"
  (cd "$SRC" && tar -cf - .) | (cd "$1" && tar -xf -)
  jq "$2" "$SRC/composer.json" > "$1/composer.json"
}
norm_log() { sed -E 's/ optimize=[^ ]+//' "$1"; }
status=0

run_pair() { # nom, étape, args composer..., "--", args vivacity...
  local name="$1" step="$2"; shift 2
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local ref="$WORK/ref-$name" viv="$WORK/viv-$name" c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >"$WORK/$name.$step.composer.out" 2>"$WORK/$name.$step.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >"$WORK/$name.$step.vivacity.out" 2>"$WORK/$name.$step.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name/$step : codes $c_code vs $v_code"; tail -3 "$WORK/$name.$step.vivacity.err"; return 1; fi
  if ! diff <(norm_log "$ref/scripts.log" 2>/dev/null) <(norm_log "$viv/scripts.log" 2>/dev/null) >"$WORK/$name.$step.logdiff"; then
    echo "FAIL $name/$step : scripts.log différent"; cat "$WORK/$name.$step.logdiff" | head; return 1
  fi
  if [ -d "$ref/vendor" ] && ! compare_vendor "$ref/vendor" "$viv/vendor" "$WORK/$name.$step.diff" >/dev/null; then
    echo "FAIL $name/$step : vendor/ différent"; head -10 "$WORK/$name.$step.diff"; return 1
  fi
  return 0
}

# 1. Les quatre événements : install (dev), dump -o, install --no-dev.
stage "$WORK/ref-events" "."; stage "$WORK/viv-events" "."
ok=1
run_pair events install install -- install --run-scripts --no-fallback || ok=0
[ $ok = 1 ] && { run_pair events dump-o dump-autoload -o -- dump-autoload -o --run-scripts || ok=0; }
[ $ok = 1 ] && { run_pair events no-dev install --no-dev -- install --no-dev --run-scripts --no-fallback || ok=0; }
if [ $ok = 1 ]; then
  n=$(wc -l < "$WORK/viv-events/scripts.log" | tr -d ' ')
  echo "OK   events : $n lignes de journal identiques (install, dump -o, --no-dev), vendor/ identique"
else status=1; fi

# 2. Un script qui échoue : même code, même arrêt.
stage "$WORK/ref-fail" '.scripts["post-autoload-dump"]=["@log post-autoload-dump", "@php -r \"exit(7);\""]'
stage "$WORK/viv-fail" '.scripts["post-autoload-dump"]=["@log post-autoload-dump", "@php -r \"exit(7);\""]'
if run_pair fail install install -- install --run-scripts --no-fallback; then
  c=$(cd "$WORK/ref-fail" && composer install --no-interaction --no-ansi >/dev/null 2>&1; echo $?)
  grep -q "post-install-cmd" "$WORK/viv-fail/scripts.log" && { echo "FAIL fail : post-install-cmd a tourné après l'échec"; status=1; } || echo "OK   fail : code $c des deux côtés, post-install-cmd jamais lancé"
else status=1; fi

# 3. --no-autoloader : pas d'événement d'autoload.
stage "$WORK/ref-noal" "."; stage "$WORK/viv-noal" "."
if run_pair noal install install --no-autoloader -- install --no-autoloader --run-scripts --no-fallback; then
  if grep -q "autoload-dump" "$WORK/viv-noal/scripts.log"; then echo "FAIL noal : événement d'autoload sans dump"; status=1; else echo "OK   noal : $(wc -l < "$WORK/viv-noal/scripts.log" | tr -d ' ') lignes (pre-install-cmd, post-install-cmd), identiques"; fi
else status=1; fi

# 4. Sans le drapeau : rien ne tourne.
stage "$WORK/viv-off" "."
(cd "$WORK/viv-off" && "$VIVACITY" install --no-fallback >/dev/null 2>&1)
[ -e "$WORK/viv-off/scripts.log" ] && { echo "FAIL off : des scripts ont tourné sans --run-scripts"; status=1; } || echo "OK   off : aucun script sans --run-scripts"
exit $status
