#!/usr/bin/env bash
# Harness v0.17 : `mnsami/composer-custom-directory-installer`. La fixture
# fixtures/projects/custom-dirs (deux bibliothèques nommées dans
# `extra.installer-paths`, une hors carte) installée par Composer (plugin
# actif) et par vivacity (placeur émulé dans `Layout`) ; projet entier
# comparé. Étapes : install ; no-op ; carte vidée (les deux paquets
# réinstallés sous vendor/, l'ancien chemin laissé tel quel — comme
# Composer) ; --no-plugins.
#
# Usage : harness/custom-dirs.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/custom-dirs"
SRC="$ROOT/fixtures/projects/custom-dirs"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
status=0
stage() { rm -rf "$1"; mkdir -p "$1"; cp "$SRC/composer.lock" "$1/"; jq "$2" "$SRC/composer.json" > "$1/composer.json"; }
ref="$WORK/ref"; viv="$WORK/viv"
step() { # nom, args composer..., "--", args vivacity...
  local name="$1"; shift
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet identique"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}
stage "$ref" "."; stage "$viv" "."
step install install --no-scripts -- install --no-scripts --no-fallback
[ -f "$viv/modules/Log/composer.json" ] && [ -f "$viv/lib/psr-container/composer.json" ] || { echo "FAIL install : chemins personnalisés absents"; status=1; }
step no-op install --no-scripts -- install --no-scripts --no-fallback
for d in "$ref" "$viv"; do jq '.extra["installer-paths"]={}' "$d/composer.json" > "$d/c.json" && mv "$d/c.json" "$d/composer.json"; done
step unmapped install --no-scripts -- install --no-scripts --no-fallback
# (Composer réinstalle sous vendor/ et laisse l'ancien modules/Log en place ; vivacity pareil.)
[ -f "$viv/vendor/psr/log/composer.json" ] || { echo "FAIL unmapped : psr/log pas revenu sous vendor/"; status=1; }
ref="$WORK/ref-noplug"; viv="$WORK/viv-noplug"; stage "$ref" "."; stage "$viv" "."
step no-plugins install --no-scripts --no-plugins -- install --no-scripts --no-plugins --no-fallback
exit $status
