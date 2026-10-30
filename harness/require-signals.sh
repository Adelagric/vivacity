#!/usr/bin/env bash
# `require` interrompu par un signal (docs/plans/parity-gaps-026.md, 32b),
# sans réseau.
#
# Composer pose un gestionnaire SIGINT/SIGTERM/SIGHUP dès qu'il a lu ses
# sauvegardes (RequireCommand.php:160-164) : il rétablit composer.json (et
# le lock), ou supprime le fichier qu'il a créé, écrit « Installation
# failed, … », puis meurt par le même signal. Chaque cas bloque les deux
# outils au même endroit : le fichier p2 du paquet demandé est une FIFO,
# lue après « Loading composer repositories… ». Le signal part une fois
# cette ligne écrite ; la FIFO est ensuite nourrie (Composer ne traite le
# signal qu'au retour de la lecture). Comparés : code de sortie (128 + n,
# mort par le signal), stdout, stderr, composer.json et composer.lock à la
# fin (contenu, ou absence).
#
# Usage : harness/require-signals.sh [cas...]
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/require-signals"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
composer --version --no-ansi 2>/dev/null | tail -1
rm -rf "$WORK"; mkdir -p "$WORK"
status=0
SELECTED="$*"
want() { [ -z "$SELECTED" ] || [[ " $SELECTED " == *" $1 "* ]]; }
P2='{"packages": {"acme/l": [{"name": "acme/l", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "library"}]}}'

# run <répertoire> <signal> <env…> -- <commande…>
run() {
  local d="$1" sig="$2"; shift 2
  local envs=()
  while [ "$1" != "--" ]; do envs+=("$1"); shift; done; shift
  rm -f "$d/srv/p2/acme/l.json"; mkfifo "$d/srv/p2/acme/l.json"
  (cd "$d/p" && exec env "${envs[@]+"${envs[@]}"}" COMPOSER_HOME="$d/home" COMPOSER_CACHE_DIR="$d/cache" \
    VIVACITY_CACHE_DIR="$d/store" COMPOSER_NO_INTERACTION=1 "$@" --no-ansi --no-scripts > "$d/out" 2> "$d/err") &
  local pid=$! i
  for i in $(seq 1 100); do grep -q 'Loading composer repositories' "$d/err" 2>/dev/null && break; sleep 0.1; done
  sleep 0.3
  kill -"$sig" "$pid"
  ( printf '%s' "$P2" > "$d/srv/p2/acme/l.json" ) & local w=$!
  wait "$pid" 2>/dev/null; echo $? > "$d/code"
  sleep 0.2; kill "$w" 2>/dev/null; wait "$w" 2>/dev/null
  cp "$d/err" "$d/err.raw"
  grep -v '^trace: ' "$d/err" > "$d/err.t"; mv "$d/err.t" "$d/err"
}

# case_ <nom> <signal> [created] [lock] [env…] -- <arguments de require>
case_() {
  local name="$1" sig="$2"; shift 2
  local created=0 lock=0 held=0 envs=() side
  while [ "$1" != "--" ]; do
    case "$1" in
      created) created=1 ;; lock) lock=1 ;;
      held) held=1; envs+=("VIVACITY_HOLD_OUTPUT=force" "VIVACITY_TRACE=1") ;;
      *) envs+=("$1") ;;
    esac; shift
  done; shift
  for side in ref viv; do
    local d="$WORK/$name/$side"
    mkdir -p "$d/srv/p2/acme" "$d/p" "$d/home"
    printf '{"metadata-url": "file://%s/srv/p2/%%package%%.json", "available-packages": ["acme/l"]}\n' "$d" > "$d/srv/packages.json"
    local repos
    repos=$(printf '[{"type": "composer", "url": "file://%s/srv"}, {"packagist.org": false}]' "$d")
    if [ "$created" = 1 ]; then
      printf '{"repositories": %s}\n' "$repos" > "$d/home/config.json"
    else
      echo '{}' > "$d/home/config.json"
      printf '{"repositories": %s, "require": {}}\n' "$repos" > "$d/p/composer.json"
    fi
    [ "$lock" = 0 ] || printf '{\n    "content-hash": "x",\n    "packages": [],\n    "packages-dev": []\n}\n' > "$d/p/composer.lock"
    if [ "$side" = ref ]; then run "$d" "$sig" "${envs[@]+"${envs[@]}"}" -- composer "$@"
    else run "$d" "$sig" "${envs[@]+"${envs[@]}"}" -- "$VIVACITY" "$@"; fi
  done
  local r="$WORK/$name/ref" v="$WORK/$name/viv" bad=""
  sed "s#$r/#<d>/#g" "$r/err" > "$r/err.n"; sed "s#$v/#<d>/#g" "$v/err" > "$v/err.n"
  [ "$(cat "$r/code")" = "$(cat "$v/code")" ] || bad="$bad codes $(cat "$r/code") vs $(cat "$v/code");"
  cmp -s "$r/out" "$v/out" || bad="$bad stdout différente;"
  cmp -s "$r/err.n" "$v/err.n" || bad="$bad stderr différente;"
  local f
  for f in composer.json composer.lock; do
    if [ -e "$r/p/$f" ] || [ -e "$v/p/$f" ]; then
      sed "s#$r/#<d>/#g" "$r/p/$f" 2>/dev/null > "$r/$f.n"; sed "s#$v/#<d>/#g" "$v/p/$f" 2>/dev/null > "$v/$f.n"
      { [ -e "$r/p/$f" ] && [ -e "$v/p/$f" ] && cmp -s "$r/$f.n" "$v/$f.n"; } || bad="$bad $f différent;"
    fi
  done
  grep -q 'Installation failed' "$r/err" || bad="$bad Composer n'écrit pas « Installation failed » (cas mal choisi);"
  [ "$held" = 0 ] || grep -q '^trace: output held until' "$v/err.raw" || bad="$bad sortie non retenue (pas de marqueur);"
  if [ -z "$bad" ]; then echo "OK   $name : même code ($(cat "$r/code")), même sortie, mêmes fichiers"
  else echo "FAIL $name :$bad"; diff "$r/err.n" "$v/err.n" | head -8; status=1; fi
}

want int && case_ int INT -- require acme/l:1.0.0 --no-install
want term && case_ term TERM -- require acme/l:1.0.0 --no-install
want hup && case_ hup HUP -- require acme/l:1.0.0 --no-install
# Un lock présent : rétabli aussi, et nommé dans la ligne.
want lock && case_ lock INT lock -- require acme/l:1.0.0 --no-install
# Pas de composer.json : celui que `require` a créé est supprimé.
want created && case_ created INT created -- require acme/l:1.0.0 --no-install
# Sortie retenue (`secure-http`, imposée) : les lignes retenues sortent
# avant celle du retour en arrière, comme Composer les avait écrites.
want held && case_ held INT held -- require acme/l:1.0.0 --no-install
want verbose && case_ verbose INT -- require acme/l:1.0.0 --no-install -v
exit $status
