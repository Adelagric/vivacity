#!/usr/bin/env bash
# Harness v0.17 : dists `tar` (asset-packagist → tarballs npm). La fixture
# fixtures/projects/tar-dist (4 paquets tar aux modes variés — 0666, 0664,
# un 0755, une racine `v1.1/` — et un zip) est installée par Composer
# (`TarDownloader` → PharData) et par vivacity (`extract_tar`) ; projet
# entier comparé par harness/lib/compare.sh, modes compris. Étapes :
#   1. install --no-scripts ; 2. no-op (rien ne doit venir du réseau : le
#   cache Composer `.tar` est partagé) ; 3. install --no-dev (le tar dev
#   retiré) ; 4. dump-autoload -o.
# Les dists viennent du réseau (registry.npmjs.org) : pas d'instantané.
#
# Usage : harness/tar-dist.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/tar-dist"
SRC="$ROOT/fixtures/projects/tar-dist"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
ref="$WORK/ref"; viv="$WORK/viv"
for d in "$ref" "$viv"; do mkdir -p "$d"; cp "$SRC/composer.json" "$SRC/composer.lock" "$d/"; done
status=0
step() { # nom, args composer..., "--", args vivacity...
  local name="$1"; shift
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet identique (modes compris)"
  else
    echo "FAIL $name : projet différent"; head -12 "$WORK/$name.diff"; status=1
  fi
}
step install install --no-scripts -- install --no-scripts --no-fallback
# Composer vient de télécharger les dists : vivacity doit les lire dans
# le cache de Composer (`files/<nom>/<sha1>.tar`), pas les retélécharger.
if grep -qE "\(0 from store, 5 from cache, 0 from network\)|\([0-9]+ from store, [0-9]+ from cache, 0 from network\)" "$WORK/install.vivacity.err"; then
  echo "OK   install : dists lus dans le cache de Composer"
else
  echo "FAIL install : $(grep -o '([^)]*from network)' "$WORK/install.vivacity.err")"; status=1
fi
step no-op   install --no-scripts -- install --no-scripts --no-fallback
if grep -q "0 from network" "$WORK/no-op.vivacity.err"; then
  echo "OK   no-op : rien téléchargé"
else
  echo "FAIL no-op : un téléchargement a eu lieu"; grep "from network" "$WORK/no-op.vivacity.err"; status=1
fi
# Le cache de Composer porte les tarballs sous `files/<nom>/<sha1>.tar` :
# vivacity les y lit (et les y écrit) — un seul fichier par dist.
cache="$(composer config --global cache-dir 2>/dev/null)"
n=$(find "$cache/files/npm-asset" -name '*.tar' 2>/dev/null | wc -l | tr -d ' ')
if [ "$n" -ge 4 ]; then echo "OK   cache : $n tarballs sous files/npm-asset/*.tar"; else echo "FAIL cache : $n tarballs (attendu ≥ 4)"; status=1; fi
step no-dev  install --no-scripts --no-dev -- install --no-scripts --no-dev --no-fallback
step dump-o  dump-autoload -o -- dump-autoload -o
exit $status
