#!/usr/bin/env bash
# Harness v0.17 : `yiisoft/yii2-composer` et `codeception/c3`. La fixture
# fixtures/projects/yii2-composer (yii2 + quatre extensions `yii2-extension`,
# deux en dev, une avec `extra.bootstrap` ; c3 en dev ; bower assets via
# asset-packagist) est installée par Composer (plugins actifs) et par
# vivacity (émulations) ; projet entier comparé par harness/lib/compare.sh
# — c3.php à la racine compris, vendor/yiisoft/extensions.php clés triées
# (l'ordre de Composer suit l'achèvement des extractions). Étapes :
# install ; no-op ; --no-dev (les deux extensions dev retirées de la carte,
# c3.php supprimé) ; install (elles reviennent, c3.php recopié) ;
# dump-autoload -o ; un vendor vierge sous --no-dev ; --no-plugins.
#
# Usage : harness/yii2-composer.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/yii2-composer"
SRC="$ROOT/fixtures/projects/yii2-composer"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
status=0
fresh() { for d in "$@"; do rm -rf "$d"; mkdir -p "$d"; cp "$SRC/composer.json" "$SRC/composer.lock" "$d/"; done; }
step() { # ref, viv, nom, args composer..., "--", args vivacity...
  local ref="$1" viv="$2" name="$3"; shift 3
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    if [ -f "$viv/vendor/yiisoft/extensions.php" ]; then
      local n; n=$(php -r 'echo count(require $argv[1]);' "$viv/vendor/yiisoft/extensions.php")
      echo "OK   $name : projet identique (extensions.php : $n entrées, comparées triées)"
    else
      echo "OK   $name : projet identique"
    fi
  else
    echo "FAIL $name : projet différent"; head -12 "$WORK/$name.diff"; status=1
  fi
}
ref="$WORK/ref"; viv="$WORK/viv"; fresh "$ref" "$viv"
step "$ref" "$viv" install install --no-scripts -- install --no-scripts --no-fallback
step "$ref" "$viv" no-op   install --no-scripts -- install --no-scripts --no-fallback
step "$ref" "$viv" no-dev  install --no-scripts --no-dev -- install --no-scripts --no-dev --no-fallback
step "$ref" "$viv" re-dev  install --no-scripts -- install --no-scripts --no-fallback
step "$ref" "$viv" dump-o  dump-autoload -o -- dump-autoload -o
ref="$WORK/ref-nodev"; viv="$WORK/viv-nodev"; fresh "$ref" "$viv"
step "$ref" "$viv" bare-no-dev install --no-scripts --no-dev -- install --no-scripts --no-dev --no-fallback
# --no-plugins : Composer ne charge pas le plugin, pas de fichier ; vivacity non plus.
ref="$WORK/ref-noplug"; viv="$WORK/viv-noplug"; fresh "$ref" "$viv"
step "$ref" "$viv" no-plugins install --no-scripts --no-plugins -- install --no-scripts --no-plugins --no-fallback
if [ ! -e "$ref/vendor/yiisoft/extensions.php" ] && [ ! -e "$viv/vendor/yiisoft/extensions.php" ]; then
  echo "OK   no-plugins : pas d'extensions.php des deux côtés"
else
  echo "FAIL no-plugins : extensions.php présent"; status=1
fi
exit $status
