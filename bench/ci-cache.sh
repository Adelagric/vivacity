#!/usr/bin/env bash
# What a CI cache artifact costs: Composer's zip cache versus vivacity's
# extracted store, both packed the way actions/cache does (tar | zstd), on
# the Sylius fixture. Also the install each one enables afterwards:
# Composer with warm zips, vivacity with warm zips and a cold store (the
# single-install CI job), vivacity with a warm store (the second install on
# the same worker). Markdown table on stdout, appended to the step summary.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="$ROOT/target/release/vivacity"
WORK="${VIVACITY_BENCH_DIR:-/tmp/vivacity-cache-bench}"
RUNS="${BENCH_RUNS:-5}"
FX="${BENCH_FIXTURE:-sylius}"
rm -rf "$WORK"; mkdir -p "$WORK/proj" "$WORK/ccache" "$WORK/vcache" "$WORK/r1" "$WORK/r2"
export COMPOSER_CACHE_DIR="$WORK/ccache" VIVACITY_CACHE_DIR="$WORK/vcache" COMPOSER_NO_INTERACTION=1
(cd "$ROOT/fixtures/projects/$FX" && tar --exclude=./vendor --exclude=./node_modules --exclude=./var -cf - .) | (cd "$WORK/proj" && tar -xf -)
cd "$WORK/proj"
C="composer install --no-plugins --no-scripts --quiet"
V="$VIVACITY install --no-scripts"
$C                                   # fills the zip cache (network), the only network step
rm -rf vendor; $V >/dev/null         # fills the store from the zips
zips_files=$(find "$WORK/ccache/files" -type f | wc -l | tr -d ' ')
store_files=$(find "$WORK/vcache/store" -type f | wc -l | tr -d ' ')
zips_size=$(du -sm "$WORK/ccache/files" | cut -f1)
store_size=$(du -sm "$WORK/vcache/store" | cut -f1)

ms() { jq -r ".results[] | select(.command==\"$1\") | (.median*1000|round)" "$2"; }
hyperfine --warmup 1 --min-runs "$RUNS" --export-json "$WORK/pack.json" \
  -n zips-produce  "tar -cf - -C $WORK/ccache files | zstd -T0 -3 -q -f -o $WORK/zips.tar.zst" \
  -n store-produce "tar -cf - -C $WORK/vcache store | zstd -T0 -3 -q -f -o $WORK/store.tar.zst" >/dev/null 2>&1
hyperfine --warmup 1 --min-runs "$RUNS" --export-json "$WORK/unpack.json" \
  --prepare "rm -rf $WORK/r1/files $WORK/r2/store" \
  -n zips-restore  "zstd -d -q -c $WORK/zips.tar.zst | tar -xf - -C $WORK/r1" \
  -n store-restore "zstd -d -q -c $WORK/store.tar.zst | tar -xf - -C $WORK/r2" >/dev/null 2>&1
zips_art=$(du -m "$WORK/zips.tar.zst" | cut -f1); store_art=$(du -m "$WORK/store.tar.zst" | cut -f1)

hyperfine --warmup 1 --min-runs "$RUNS" --export-json "$WORK/install.json" \
  --prepare "rm -rf vendor" -n composer-warm-zips "$C" \
  --prepare "rm -rf vendor $WORK/vcache/store $WORK/vcache/classmap" -n vivacity-cold-store "$V" \
  --prepare "rm -rf vendor" -n vivacity-warm-store "$V" >/dev/null 2>&1

out="### CI cache artifact, $FX fixture ($(uname -s) $(uname -m), $(nproc 2>/dev/null || sysctl -n hw.ncpu) cores, $(df -T . 2>/dev/null | awk 'NR==2{print $2}'))\n\n"
out+="| | zip cache | extracted store |\n|---|---|---|\n"
out+="| on disk | $zips_size MB, $zips_files files | $store_size MB, $store_files files |\n"
out+="| produce (tar \\| zstd -3) | $(ms zips-produce "$WORK/pack.json") ms | $(ms store-produce "$WORK/pack.json") ms |\n"
out+="| restore | $(ms zips-restore "$WORK/unpack.json") ms | $(ms store-restore "$WORK/unpack.json") ms |\n"
out+="| artifact | $zips_art MB | $store_art MB |\n\n"
out+="| install, vendor removed | median |\n|---|---|\n"
out+="| composer, warm zips | $(ms composer-warm-zips "$WORK/install.json") ms |\n"
out+="| vivacity, warm zips, cold store | $(ms vivacity-cold-store "$WORK/install.json") ms |\n"
out+="| vivacity, warm store | $(ms vivacity-warm-store "$WORK/install.json") ms |\n"
printf "$out"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && printf "$out" >> "$GITHUB_STEP_SUMMARY" || true
