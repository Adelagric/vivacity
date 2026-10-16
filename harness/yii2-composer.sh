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
# À chaque étape, l'ordre des lignes, sortie standard et sortie d'erreur
# fusionnées : la copie de c3.php (`POST_INSTALL_CMD`, `POST_UPDATE_CMD`)
# après le funding, comme `Installer::run` les imprime ; « Deleting
# c3.php » (`PluginInstaller::uninstall`) juste avant la ligne `- Removing
# codeception/c3`. Puis un projet c3 sans yii2-composer (Packagist en
# ligne, versions exactes) : `update` avec installation, puis, sans dump
# (`--no-autoloader`), c3 qui copie et c3 qui supprime.
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
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >"$WORK/$name.composer.all" 2>&1) || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >"$WORK/$name.vivacity.all" 2>&1) || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.all"; status=1; return; fi
  # L'ordre des lignes : le résumé `vivacity: …` et les `  - Downloading`
  # (cache froid de Composer) hors comparaison.
  if ! diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.all") <(grep -v '^vivacity: ' "$WORK/$name.vivacity.all") >"$WORK/$name.order.diff"; then
    echo "FAIL $name : ordre des lignes différent"; head -10 "$WORK/$name.order.diff"; status=1; return
  fi
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
# c3 sans yii2-composer (dont l'`update` avec installation va à Composer) :
# un paquet avec `funding` et c3 en dev, résolus sur Packagist.
ref="$WORK/ref-c3"; viv="$WORK/viv-c3"
for d in "$ref" "$viv"; do
  rm -rf "$d"; mkdir -p "$d"
  printf '{"name": "vivacity/c3-order", "version": "1.0.0", "require": {"symfony/polyfill-ctype": "1.33.0"}, "require-dev": {"codeception/c3": "2.9.0"}, "config": {"allow-plugins": {"codeception/c3": true}}}\n' > "$d/composer.json"
done
step "$ref" "$viv" c3-update update --no-scripts --no-audit --no-security-blocking -- update --no-scripts --no-audit --no-security-blocking --no-fallback
grep -q 'looking for funding' "$WORK/c3-update.composer.all" && grep -q '^\[codeception/c3\] Copying' "$WORK/c3-update.composer.all" || { echo "FAIL c3-update : ni funding ni copie chez Composer (cas mal construit)"; status=1; }
rm -f "$ref/c3.php" "$viv/c3.php"
step "$ref" "$viv" c3-no-autoloader install --no-scripts --no-autoloader -- install --no-scripts --no-autoloader --no-fallback
grep -q '^\[codeception/c3\] Copying' "$WORK/c3-no-autoloader.composer.all" || { echo "FAIL c3-no-autoloader : pas de copie chez Composer (cas mal construit)"; status=1; }
step "$ref" "$viv" c3-uninstall-no-autoloader install --no-dev --no-scripts --no-autoloader -- install --no-dev --no-scripts --no-autoloader --no-fallback
grep -q '^\[codeception/c3\] Deleting' "$WORK/c3-uninstall-no-autoloader.composer.all" && [ ! -e "$ref/c3.php" ] || { echo "FAIL c3-uninstall-no-autoloader : Composer n'a pas supprimé c3.php (cas mal construit)"; status=1; }
exit $status
