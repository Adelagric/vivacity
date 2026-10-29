#!/usr/bin/env bash
# Whether a CI cache should carry vivacity's class-map records
# (`<vivacity cache>/classmap/`) besides Composer's zip cache, measured on
# the cold-runner path: vendor/ absent, Composer's cache warm, no store, the
# project's sources freshly checked out (new mtimes). Two hyperfine arms that
# differ only by their preparation: "zips only" removes the records,
# "zips + classmap" puts back the ones a previous install wrote, at the same
# absolute path (a hosted runner's cache directory does not move between
# runs). Laravel and Sylius, `optimize-autoloader` off then on (set in the
# copy's `config`, outside the content-hash). Also: how many records the
# second arm served (a record rewritten is one that missed), the `dump scan`
# phase of each arm (VIVACITY_TRACE=1), and what the extra directory costs
# as a cache artifact (size, tar | zstd -3 produce and restore).
# The "records served" column counts the records the second arm did not
# rewrite: it is only evidence together with the drop of `dump scan`.
# Markdown on stdout, appended to the job summary. docs/plans/classmap-cache.md
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# hyperfine is a native binary: under Git Bash it is handed this same bash,
# quoted, in forward slashes (it splits --shell like a shell would; a bare
# `bash` could resolve to WSL's). On a Windows runner the caches live on C:
# (%LOCALAPPDATA%) and the project on D:, so the install copies from the
# store instead of linking: the caches go there too.
exe=""; SH=bash; WIN=0
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) exe=".exe"; WIN=1; SH="\"$(cygpath -m "$BASH")\"" ;; esac
VIVACITY="${VIVACITY:-$ROOT/target/release/vivacity$exe}"
WORK="${VIVACITY_BENCH_DIR:-${RUNNER_TEMP:-/tmp}/vivacity-classmap-bench}"
RUNS="${BENCH_RUNS:-10}"
FIXTURES="${BENCH_FIXTURES:-laravel sylius}"
MODES="${BENCH_MODES:-plain opt}"
test -x "$VIVACITY" || { echo "no vivacity binary at $VIVACITY (cargo build --release)" >&2; exit 1; }
command -v hyperfine >/dev/null || { echo "hyperfine is required" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
HAVE_ZSTD=1; command -v zstd >/dev/null || HAVE_ZSTD=0

CACHES="$WORK"
if [ "$WIN" = 1 ]; then
  WORK="$(cygpath -m "$WORK")"   # RUNNER_TEMP comes as D:\a\_temp
  CACHES="$(cygpath -m "$LOCALAPPDATA")/vivacity-classmap-bench"
fi
rm -rf "$WORK" "$CACHES"; mkdir -p "$WORK" "$CACHES/ccache"
export COMPOSER_CACHE_DIR="$CACHES/ccache" COMPOSER_NO_INTERACTION=1
unset VIVACITY_TRACE VIVACITY_NO_CLASSMAP_CACHE

# A statistic of one hyperfine result, in ms. hyperfine 1.x JSON: the -n
# name in `command`, statistics at the top of a result; 2.x (schema 2): the
# name in `name`, statistics under `summary.time_wall_clock`. A missing
# value stops the bench: run 40915941883 (hyperfine 2.0 read as 1.x) printed
# empty cells and a gain of 0 on all three OSes, and stayed green.
ms() {
  local v
  v=$(jq -r --arg n "$2" --arg f "$3" \
    '[.results[] | select((.name // .command) == $n) | (.summary.time_wall_clock // .) | .[$f] | numbers]
     | if length == 1 then (.[0] * 1000 | round) else empty end' "$1") || v=""
  [ -n "$v" ] || { echo "$1: no $3 for '$2' (unreadable hyperfine result; JSON schema?)" >&2; exit 1; }
  printf '%s\n' "$v"
}
# The cells below read ms() inside $(…), where its exit stops nothing:
# every value is read once here first, in this shell.
check_json() { # file name…
  local j="$1" n f; shift
  for n; do for f in median stddev min max; do ms "$j" "$n" "$f" > /dev/null; done; done
}
# A failing install says why (its stderr), instead of a bare exit code.
install_quiet() {
  "$VIVACITY" install >/dev/null 2>"$WORK/last-install.txt" || { cat "$WORK/last-install.txt" >&2; return 1; }
}
scan_ms() { awk '/trace: +dump jobs /{j=$(NF-1)} /trace: +dump scan /{s=$(NF-1)} END{printf "%d", s-j+0.5}' "$1"; }

host="$(uname -s) $(uname -m), $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "${NUMBER_OF_PROCESSORS:-?}") cores"
out="### Class-map records in the CI cache ($host, $("$VIVACITY" --version), hyperfine ${RUNS} runs + 1 warm-up)\n\n"
out+="Cold runner: vendor/ and store removed, Composer's zips warm, sources touched. Times in ms: median (σ, min–max).\n\n"
out+="| fixture | autoloader | zips only | zips + classmap | gain | dump scan (only → +classmap) | records served | classmap/ size | restore (tar\\|zstd) |\n"
out+="|---|---|---|---|---|---|---|---|---|\n"

for fx in $FIXTURES; do
  for mode in $MODES; do
    P="$WORK/$fx-$mode/proj"; VC="$CACHES/$fx-$mode/vcache"; SNAP="$CACHES/$fx-$mode/snap"
    mkdir -p "$P" "$VC" "$SNAP"
    (cd "$ROOT/fixtures/projects/$fx" && tar --exclude=./vendor --exclude=./node_modules --exclude=./var -cf - .) | (cd "$P" && tar -xf -)
    opt=false; [ "$mode" = opt ] && opt=true
    jq --argjson o "$opt" '.config["optimize-autoloader"] = $o' "$P/composer.json" > "$P/composer.json.new"
    mv "$P/composer.json.new" "$P/composer.json"
    export VIVACITY_CACHE_DIR="$VC"
    cd "$P"
    install_quiet                                     # zips (network the first time), store, records
    rm -rf vendor "$VC/store" "$VC/classmap" "$VC/classmap-files"
    install_quiet                                     # the records a cold run writes
    nrec=0; [ -d "$VC/classmap" ] && nrec=$(find "$VC/classmap" -type f | wc -l | tr -d ' ')
    [ "$nrec" -gt 0 ] || { echo "$fx $mode: the install wrote no class-map record" >&2; exit 1; }
    cp -R "$VC/classmap" "$SNAP/classmap"
    size_kb=$(du -sk "$SNAP/classmap" | cut -f1)

    cold="rm -rf vendor '$VC/store' '$VC/classmap' '$VC/classmap-files' '$VC/platform-probe.json'; find . -path ./vendor -prune -o -type f -exec touch {} +"
    warm="$cold; cp -R '$SNAP/classmap' '$VC/classmap'"
    J="$WORK/$fx-$mode/install.json"
    hyperfine --shell="$SH" --style basic --warmup 1 --min-runs "$RUNS" --export-json "$J" \
      --prepare "$cold" -n zips-only "'$VIVACITY' install" \
      --prepare "$warm" -n zips-classmap "'$VIVACITY' install" >/dev/null
    check_json "$J" zips-only zips-classmap

    # One traced run per arm; then count the records the second arm rewrote.
    T="$WORK/$fx-$mode"
    bash -c "$cold"; VIVACITY_TRACE=1 "$VIVACITY" install > /dev/null 2> "$T/trace-only.txt" || { cat "$T/trace-only.txt" >&2; exit 1; }
    bash -c "$warm"; touch "$T/marker"; sleep 1
    VIVACITY_TRACE=1 "$VIVACITY" install > /dev/null 2> "$T/trace-cm.txt" || { cat "$T/trace-cm.txt" >&2; exit 1; }
    rewritten=$(find "$VC/classmap" -type f -newer "$WORK/$fx-$mode/marker" | wc -l | tr -d ' ')
    if grep -q 'delegating' "$WORK/$fx-$mode/trace-only.txt" "$WORK/$fx-$mode/trace-cm.txt"; then
      echo "$fx $mode: the install was handed to Composer; nothing here measures vivacity" >&2; exit 1
    fi
    served=$((nrec - rewritten))

    restore="n/a (no zstd)"
    if [ "$HAVE_ZSTD" = 1 ]; then
      tar -cf - -C "$SNAP" classmap | zstd -T0 -3 -q -f -o "$WORK/$fx-$mode/cm.tar.zst"
      R="$CACHES/$fx-$mode/r"; mkdir -p "$R"
      hyperfine --shell="$SH" --style basic --warmup 1 --min-runs "$RUNS" --export-json "$WORK/$fx-$mode/restore.json" \
        --prepare "rm -rf '$R/classmap'" \
        -n restore "zstd -d -q -c '$WORK/$fx-$mode/cm.tar.zst' | tar -xf - -C '$R'" >/dev/null
      check_json "$WORK/$fx-$mode/restore.json" restore
      restore="$(ms "$WORK/$fx-$mode/restore.json" restore median) ms"
    fi
    a_med=$(ms "$J" zips-only median); b_med=$(ms "$J" zips-classmap median)
    cell() { echo "$(ms "$J" "$1" median) ($(ms "$J" "$1" stddev), $(ms "$J" "$1" min)–$(ms "$J" "$1" max))"; }
    row="| $fx | $([ "$opt" = true ] && echo '-o' || echo 'plain') | $(cell zips-only) | $(cell zips-classmap) | $((a_med - b_med)) | $(scan_ms "$WORK/$fx-$mode/trace-only.txt") → $(scan_ms "$WORK/$fx-$mode/trace-cm.txt") | $served/$nrec | ${size_kb} KB | $restore |\n"
    out+="$row"; printf "%b" "$row" >&2   # each row as soon as it is measured
    cd "$ROOT"
    rm -rf "$P/vendor" "$VC/store"        # ~0.5 GB per case: not kept
  done
done
printf "%b" "$out"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then printf "%b" "$out" >> "$GITHUB_STEP_SUMMARY"; fi
