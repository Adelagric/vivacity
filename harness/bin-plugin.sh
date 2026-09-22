#!/usr/bin/env bash
# Harness v0.17 : `bamarni/composer-bin-plugin` sans `forward-command`. La
# fixture fixtures/projects/bin-plugin (le plugin en dev, `bin-links`
# explicite, `forward-command` au défaut) : Composer et vivacity comparés
# sur le projet entier ET sur stderr complète (les lignes `[bamarni-bin]`
# de dépréciation, émises à l'événement COMMAND quand le plugin est déjà
# installé et à POST_AUTOLOAD_DUMP quand il l'est à la fin). Étapes :
# install (une ligne) ; no-op (deux) ; dump-autoload (deux) ; --no-dev (une,
# le plugin part) ; puis `forward-command: true` → rendu à Composer.
#
# Usage : harness/bin-plugin.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/bin-plugin"
SRC="$ROOT/fixtures/projects/bin-plugin"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
status=0
stage() { # destination, filtre jq
  rm -rf "$1"; mkdir -p "$1"; cp "$SRC/composer.lock" "$1/"; jq "$2" "$SRC/composer.json" > "$1/composer.json"
  (cd "$1" && git init -q -b main && git add -A >/dev/null && \
    GIT_AUTHOR_NAME=vivacity GIT_AUTHOR_EMAIL=v@v GIT_AUTHOR_DATE="2026-09-10T00:00:00Z" \
    GIT_COMMITTER_NAME=vivacity GIT_COMMITTER_EMAIL=v@v GIT_COMMITTER_DATE="2026-09-10T00:00:00Z" \
    git commit -q -m fixture)
}
ref="$WORK/ref"; viv="$WORK/viv"; stage "$ref" "."; stage "$viv" "."
step() { # nom, lignes attendues, args composer..., "--", args vivacity...
  local name="$1" expected="$2"; shift 2
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  # `  - Downloading …` : Composer l'imprime sur cache froid (CI), vivacity
  # jamais (feuille de route) ; hors comparaison, comme dans les autres harnais.
  if ! diff <(grep -v '^vivacity: ' "$WORK/$name.vivacity.err") <(grep -v '^  - Downloading ' "$WORK/$name.composer.err") >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -8 "$WORK/$name.err.diff"; status=1; return
  fi
  local n; n=$(grep -c '^\[bamarni-bin\]' "$WORK/$name.vivacity.err" || true)
  if [ "$n" != "$expected" ]; then echo "FAIL $name : $n ligne(s) [bamarni-bin], attendu $expected"; status=1; return; fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet et stderr identiques ($n ligne(s) [bamarni-bin])"
  else
    echo "FAIL $name : projet différent"; head -8 "$WORK/$name.diff"; status=1
  fi
}
step install 1 install --no-scripts -- install --no-scripts --no-fallback
step no-op   2 install --no-scripts -- install --no-scripts --no-fallback
step dump    2 dump-autoload -- dump-autoload
step no-dev  1 install --no-scripts --no-dev -- install --no-scripts --no-dev --no-fallback
# forward-command: true → les installs imbriqués de vendor-bin/, à Composer.
viv="$WORK/viv-forward"; stage "$viv" '.extra["bamarni-bin"]["forward-command"]=true'
v_code=0; (cd "$viv" && "$VIVACITY" install --no-scripts --no-fallback 2>"$WORK/forward.vivacity.err" >/dev/null) || v_code=$?
if [ "$v_code" = 3 ] && grep -q "forward-command true" "$WORK/forward.vivacity.err" && [ ! -e "$viv/vendor" ]; then
  echo "OK   forward-command : rendu à Composer avec la raison, rien d'écrit"
else
  echo "FAIL forward-command : code $v_code"; tail -3 "$WORK/forward.vivacity.err"; status=1
fi
exit $status
