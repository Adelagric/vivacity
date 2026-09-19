#!/usr/bin/env bash
# Benchmarks reproductibles vivace vs Composer sur les fixtures (hyperfine).
# Produit un tableau Markdown sur stdout (et dans $GITHUB_STEP_SUMMARY en CI).
# Scénarios, caches chauds : no-op ; warm (vendor supprimé) ; dump-autoload -o.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACE="$ROOT/target/release/vivacity"
WORK="${VIVACE_BENCH_DIR:-/tmp/vivace-bench}"
RUNS="${BENCH_RUNS:-10}"
# --no-blocking des deux côtés : sans lui, l'install depuis le lock fait une
# requête conditionnelle (listes de blocage) dont la latence ne dépend pas de
# la vitesse du runner — Composer 446–926 ms de no-op d'un run à l'autre pour
# 104–140 ms chez vivacity, et le ratio (bench/gate.py) ne se simplifie plus.
# Avec le drapeau (le même chez Composer), les deux côtés sont CPU-bound et
# déterministes : Mac, composer 1,08 s → 0,68 s (σ 21 ms), vivacity 134 → 49 ms
# (σ 0,4). La requête est mesurée à part (DECISIONS 2026-09-19), pas gardée.
C="composer --no-interaction --no-plugins --no-scripts --no-blocking"
V_FLAGS="--offline --no-blocking"
mkdir -p "$WORK"
out="| fixture | scénario | Composer | vivace | gain |\n|---|---|---|---|---|\n"
for fx in laravel symfony sylius; do
  d="$WORK/$fx"; rm -rf "$d"; mkdir -p "$d"
  (cd "$ROOT/fixtures/work/$fx" && tar --exclude=./vendor --exclude=./node_modules --exclude=./var -cf - .) | (cd "$d" && tar -xf -)
  cd "$d"
  $C install --quiet; "$VIVACE" install $V_FLAGS 2>/dev/null   # chauffe caches, store, classmap
  hyperfine -N --warmup 1 --min-runs "$RUNS" --export-json "$WORK/$fx.json" \
    --prepare true -n "composer-noop" "$C install" \
    --prepare true -n "vivace-noop" "$VIVACE install $V_FLAGS" \
    --prepare "rm -rf vendor" -n "composer-warm" "$C install" \
    --prepare "rm -rf vendor" -n "vivace-warm" "$VIVACE install $V_FLAGS" \
    --prepare true -n "composer-dump-o" "$C dump-autoload -o --quiet" \
    --prepare true -n "vivace-dump-o" "$VIVACE dump-autoload -o" >/dev/null 2>&1
  for sc in noop warm dump-o; do
    c=$(jq -r ".results[] | select(.command==\"composer-$sc\") | (.median*1000|round)" "$WORK/$fx.json")
    v=$(jq -r ".results[] | select(.command==\"vivace-$sc\") | (.median*1000|round)" "$WORK/$fx.json")
    gain=$(awk -v c="$c" -v v="$v" 'BEGIN{ if (v>0) printf "%.1f×", c/v; else print "?" }')
    out+="| $fx | $sc | ${c} ms | ${v} ms | $gain |\n"
  done
done
printf "$out"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && { echo "## vivace vs Composer ($(uname -s) $(uname -m), $(nproc 2>/dev/null || sysctl -n hw.ncpu) cœurs)"; printf "$out"; } >> "$GITHUB_STEP_SUMMARY" || true
