#!/usr/bin/env bash
# Harness v0.17 : `roots/wordpress-core-installer`. La fixture
# fixtures/projects/wp-core (le plugin, un paquet `wordpress-core`, une
# bibliothèque) installée par Composer (plugin actif) et par vivacity
# (placeur émulé dans `Layout`) ; projet entier comparé. Variantes (jq sur
# composer.json) : chemin en chaîne (web/wp), carte par nom, sans
# `extra` (défaut `wordpress` du plugin), et --no-plugins (le paquet sous
# vendor/ des deux côtés). Puis `wordpress-install-dir: "."` → rendu à
# Composer avec la raison (le plugin lève une exception).
#
# Usage : harness/wp-core.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/wp-core"
SRC="$ROOT/fixtures/projects/wp-core"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
status=0
stage() { rm -rf "$1"; mkdir -p "$1"; cp "$SRC/composer.lock" "$1/"; jq "$2" "$SRC/composer.json" > "$1/composer.json"; }
VARIANTS=(
  "string~.~web/wp"
  "map~.extra[\"wordpress-install-dir\"]={\"roots/wordpress-no-content\":\"public/cms\"}~public/cms"
  "default~del(.extra)~wordpress"
)
for spec in "${VARIANTS[@]}"; do
  IFS='~' read -r name filter dir <<< "$spec"
  ref="$WORK/ref-$name"; viv="$WORK/viv-$name"; stage "$ref" "$filter"; stage "$viv" "$filter"
  c_code=0; v_code=0
  (cd "$ref" && composer install --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" install --no-scripts --no-fallback >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; continue; fi
  if [ ! -f "$viv/$dir/wp-settings.php" ]; then echo "FAIL $name : WordPress absent de $dir"; status=1; continue; fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet identique (WordPress dans $dir/)"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
done
# --no-plugins : pas d'installateur, le paquet sous vendor/.
ref="$WORK/ref-noplug"; viv="$WORK/viv-noplug"; stage "$ref" "."; stage "$viv" "."
(cd "$ref" && composer install --no-scripts --no-plugins --no-interaction --no-ansi >/dev/null 2>&1)
(cd "$viv" && "$VIVACITY" install --no-scripts --no-plugins --no-fallback >/dev/null 2>"$WORK/noplug.vivacity.err") || true
if [ -f "$viv/vendor/roots/wordpress-no-content/wp-settings.php" ] && compare_vendor "$ref" "$viv" "$WORK/noplug.diff" >/dev/null; then
  echo "OK   no-plugins : projet identique (WordPress sous vendor/)"
else
  echo "FAIL no-plugins"; head -6 "$WORK/noplug.diff" 2>/dev/null; status=1
fi
# "." : le plugin lève ; vivacity rend la main avant d'écrire.
viv="$WORK/viv-dot"; stage "$viv" '.extra["wordpress-install-dir"]="."'
v_code=0; (cd "$viv" && "$VIVACITY" install --no-scripts --no-fallback >/dev/null 2>"$WORK/dot.vivacity.err") || v_code=$?
if [ "$v_code" = 3 ] && grep -q "invalid WordPress install directory" "$WORK/dot.vivacity.err" && [ ! -e "$viv/vendor" ]; then
  echo "OK   dot : rendu à Composer avec la raison du plugin, rien d'écrit"
else
  echo "FAIL dot : code $v_code"; tail -3 "$WORK/dot.vivacity.err"; status=1
fi
exit $status
