#!/usr/bin/env bash
# Harness v0.18 : `composer/package-versions-deprecated`. La fixture
# fixtures/projects/package-versions (le plugin, une bibliothèque, une en
# dev, deux `replace` dont un `self.version`) est installée par Composer
# (plugin actif : `--no-scripts` ne coupe que les scripts de composer.json,
# pas les écouteurs) et par vivacity (émulation) ; projet entier comparé —
# `Versions.php` est régénéré à POST_AUTOLOAD_DUMP, son contenu et son mode
# (0664) sont dans la comparaison. Étapes : install ; no-op ; --no-dev (la
# carte perd le paquet dev) ; retour en dev ; dump-autoload ; puis deux cas
# où Composer n'écrit rien — `allow-plugins` à false, et `--no-plugins`.
#
# Usage : harness/package-versions.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/package-versions"
SRC="$ROOT/fixtures/projects/package-versions"
VERSIONS_REL="vendor/composer/package-versions-deprecated/src/PackageVersions/Versions.php"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
status=0
stage() { rm -rf "$1"; mkdir -p "$1"; cp "$SRC/composer.lock" "$1/"; jq "$2" "$SRC/composer.json" > "$1/composer.json"; }
step() { # ref, viv, nom, args composer..., "--", args vivacity...
  local ref="$1" viv="$2" name="$3"; shift 3
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    local n="stub"
    grep -q "This class is generated" "$viv/$VERSIONS_REL" 2>/dev/null && n="$(grep -c "' => '" "$viv/$VERSIONS_REL") entrées"
    echo "OK   $name : projet identique (Versions.php : $n)"
  else
    echo "FAIL $name : projet différent"; head -12 "$WORK/$name.diff"; status=1
  fi
}
ref="$WORK/ref"; viv="$WORK/viv"; stage "$ref" "."; stage "$viv" "."
step "$ref" "$viv" install install --no-scripts -- install --no-scripts --no-fallback
step "$ref" "$viv" no-op   install --no-scripts -- install --no-scripts --no-fallback
step "$ref" "$viv" no-dev  install --no-scripts --no-dev -- install --no-scripts --no-dev --no-fallback
step "$ref" "$viv" re-dev  install --no-scripts -- install --no-scripts --no-fallback
step "$ref" "$viv" dump    dump-autoload -- dump-autoload
# Le plugin doit avoir été chargé : sans ça le harnais ne prouve rien.
if ! grep -q "This class is generated" "$viv/$VERSIONS_REL"; then
  echo "FAIL install : Versions.php n'a jamais été régénéré (oracle aveugle)"; status=1
fi
# `allow-plugins: false` : Composer ne charge pas le plugin, le stub reste.
ref="$WORK/ref-blocked"; viv="$WORK/viv-blocked"
stage "$ref" '.config["allow-plugins"]["composer/package-versions-deprecated"]=false'
stage "$viv" '.config["allow-plugins"]["composer/package-versions-deprecated"]=false'
step "$ref" "$viv" blocked install --no-scripts -- install --no-scripts --no-fallback
grep -q "This class is generated" "$viv/$VERSIONS_REL" && { echo "FAIL blocked : Versions.php régénéré alors que le plugin est refusé"; status=1; }
# --no-plugins : idem des deux côtés.
ref="$WORK/ref-noplug"; viv="$WORK/viv-noplug"; stage "$ref" "."; stage "$viv" "."
step "$ref" "$viv" no-plugins install --no-scripts --no-plugins -- install --no-scripts --no-plugins --no-fallback
exit $status
