#!/usr/bin/env bash
# vivacity against Composer on Windows (Git Bash on a GitHub runner), in the
# shape of the one external measurement there is: Luther Monson's, Windows 11,
# Laravel fixture, warm caches, `--ignore-platform-reqs` on both sides
# (https://luth.io/blog/2026/10/composer-without-php-vivacity/). Also runs on
# Linux and macOS, for a sanity pass. docs/plans/bench-windows.md, bench/M9-windows.md.
#
# Three scenarios per fixture, caches warm: `noop` (install on a complete
# vendor/), `fresh` (vendor/ deleted before each run), `dump-o`
# (dump-autoload -o). One project copy per tool, each vendor/ written by its
# own tool. Before the timings, and again after them, the two vendor/ trees
# must be identical (a sha256 per file, links by target), and vivacity must
# not have handed anything over to Composer: a bench of different outputs, or
# of Composer twice, measures nothing. The runner's conditions (CPU, OS,
# Defender, drives) go into the summary.
#
# Usage: bench/windows-bench.sh [prepare|measure|all]
#   prepare  copies the fixtures, warms Composer's zip cache (network: run it
#            under harness/net-retry.sh) and vivacity's store
#   measure  proves the trees identical, times, proves again; no network
# Variables:
#   BENCH_RUNS (5), BENCH_FIXTURES ("laravel"; also symfony, sylius),
#   BENCH_LAYOUT (Windows only: split = project under RUNNER_TEMP on D:,
#     caches under %LOCALAPPDATA% on C:, what a CI user gets; same = both
#     on C:, a one-disk workstation), VIVACITY (binary), COMPOSER_PHAR,
#   VIVACITY_BENCH_DIR (projects), BENCH_CACHE_DIR (caches),
#   BENCH_COMPOSER_CACHE_DIR (an existing Composer cache to reuse instead),
#   BENCH_RESULTS_DIR (JSON, manifests, logs, summary.md).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PHASE="${1:-all}"
case "$PHASE" in prepare|measure|all) ;; *) echo "usage: $0 [prepare|measure|all]" >&2; exit 2 ;; esac

WIN=0; exe=""; OS_NAME="$(uname -s)"
case "$OS_NAME" in MINGW*|MSYS*|CYGWIN*) WIN=1; exe=".exe"; OS_NAME=Windows ;; esac
# Paths handed to native programs (hyperfine, php, vivacity) in forward
# slashes, drive letter first: RUNNER_TEMP comes as D:\a\_temp, and
# hyperfine splits its commands like a shell would.
winpath() { if [ "$WIN" = 1 ]; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
# A program by absolute path, for hyperfine (no PATHEXT): on Windows
# `command -v php` gives /c/tools/php/php, the file is php.exe.
native() {
  local p; p="$(command -v "$1")" || { echo "$1 is required" >&2; exit 1; }
  case "$p" in /*) ;; *) echo "$1 is not a program here ($p)" >&2; exit 1 ;; esac
  p="$(winpath "$p")"
  if [ "$WIN" = 1 ] && [ -f "$p.exe" ]; then p="$p.exe"; fi
  printf '%s' "$p"
}

RUNS="${BENCH_RUNS:-5}"
FIXTURES="${BENCH_FIXTURES:-laravel}"
VIVACITY="$(winpath "${VIVACITY:-$ROOT/target/release/vivacity$exe}")"
if [ "$WIN" = 1 ]; then
  LAYOUT="${BENCH_LAYOUT:-split}"
  temp="$(winpath "${RUNNER_TEMP:-$TEMP}")"; local_app="$(winpath "$LOCALAPPDATA")"
  case "$LAYOUT" in
    split) proj_default="$temp/vwb" ;;
    same)  proj_default="$local_app/vwb/p" ;;
    *) echo "BENCH_LAYOUT must be split or same" >&2; exit 2 ;;
  esac
  cache_default="$local_app/vwb/c"
else
  LAYOUT="one filesystem"
  base="${RUNNER_TEMP:-/tmp}/vivacity-windows-bench"
  proj_default="$base/p"; cache_default="$base/c"
fi
PROJ="$(winpath "${VIVACITY_BENCH_DIR:-$proj_default}")"
CACHES="$(winpath "${BENCH_CACHE_DIR:-$cache_default}")"
RESULTS="$(winpath "${BENCH_RESULTS_DIR:-$PROJ/results}")"
export COMPOSER_CACHE_DIR="${BENCH_COMPOSER_CACHE_DIR:-$CACHES/composer}"
export VIVACITY_CACHE_DIR="$CACHES/vivacity"
export COMPOSER_NO_INTERACTION=1
unset VIVACITY_TRACE VIVACITY_PARALLEL_IO

# Composer as `php composer.phar`, as Luther ran it, and because hyperfine -N
# cannot start setup-php's composer.bat.
find_phar() {
  local c cands=()
  [ -n "${COMPOSER_PHAR:-}" ] && cands+=("$COMPOSER_PHAR")
  if c="$(command -v composer.phar)"; then cands+=("$c"); fi
  if c="$(command -v composer)"; then cands+=("$(dirname "$c")/composer.phar" "$c"); fi
  for c in ${cands[@]+"${cands[@]}"}; do
    if [ -f "$c" ] && grep -aq '__HALT_COMPILER' "$c"; then winpath "$c"; return; fi
  done
  echo "no composer.phar found (set COMPOSER_PHAR)" >&2; exit 1
}
PHP="$(native php)"; PHAR="$(find_phar)"
[ -f "$VIVACITY" ] || { echo "no vivacity binary at $VIVACITY (cargo build --release)" >&2; exit 1; }

# The same flags on both sides: Luther's (`--no-interaction --no-scripts -q
# --ignore-platform-reqs`, plugins left on) plus `--no-blocking` on install,
# for ci-bench.sh's reason: without it, install from the lock makes a
# conditional request for Packagist's blocking lists whose latency is the
# network's, not the tool's (Composer's no-op 446–926 ms between identical
# runs, DECISIONS 2026-09-19). dump-autoload has no --no-blocking.
# `-q` is added for the timings only: the proof runs without it, so that
# vivacity's `delegating to` line, printed at the normal level, is seen.
INSTALL=(install --no-interaction --no-scripts --ignore-platform-reqs --no-blocking)
DUMP=(dump-autoload -o --no-interaction --no-scripts --ignore-platform-reqs)
tool() { # composer|vivacity args… — from the current directory
  local t="$1"; shift
  if [ "$t" = composer ]; then "$PHP" "$PHAR" "$@"; else "$VIVACITY" "$@"; fi
}
cmdline() { # composer|vivacity args… — one command line for hyperfine -N
  if [ "$1" = composer ]; then printf "'%s' '%s'" "$PHP" "$PHAR"; else printf "'%s'" "$VIVACITY"; fi
  shift; printf ' %s' "$@"
}

copy_fixture() { # fixture dest
  local src="$ROOT/fixtures/projects/$1"
  [ -f "$src/composer.lock" ] || { echo "no fixture at $src" >&2; exit 1; }
  rm -rf "$2"; mkdir -p "$2"
  (cd "$src" && tar --exclude=./vendor --exclude=./node_modules --exclude=./var -cf - .) | (cd "$2" && tar -xf -)
}

# Each tool keeps its own copy (`<fixture>/composer`, `<fixture>/vivacity`)
# but works on it at ONE absolute path, `<fixture>/app`: plugins write
# absolute paths into vendor/ (rector/extension-installer's
# GeneratedConfig.php on Sylius), and two copies side by side would differ
# by their directory name alone. The copy is renamed into place before its
# tool runs and back after. A rename on Windows can meet a handle still
# open on the tree (an antivirus scan): retried.
app() { printf '%s/%s/app' "$PROJ" "$1"; }
rename() { # from to — `to` must not exist
  [ ! -e "$2" ] || { echo "$2 is in the way (a previous run stopped half-way?)" >&2; exit 1; }
  for _ in 1 2 3 4 5; do mv "$1" "$2" 2>/dev/null && return; sleep 2; done
  mv "$1" "$2"
}
enter() { rename "$PROJ/$1/$2" "$(app "$1")"; }   # fixture tool
leave() { rename "$(app "$1")" "$PROJ/$1/$2"; }

prepare() {
  rm -rf "$PROJ"; mkdir -p "$PROJ" "$CACHES"
  for fx in $FIXTURES; do
    for t in composer vivacity; do
      copy_fixture "$fx" "$PROJ/$fx/$t"
      echo "== $fx: warming $t (zips, store, class-map records)"
      enter "$fx" "$t"
      (cd "$(app "$fx")" && tool "$t" "${INSTALL[@]}")
      leave "$fx" "$t"
    done
  done
}

# One line per regular file (`<path>\t<sha256>`) and per link
# (`<path>\tlink -> <target>`), sorted bytewise.
if command -v sha256sum >/dev/null; then SHA=(sha256sum); else SHA=(shasum -a 256); fi
manifest() { # project-dir > manifest
  (
    cd "$1"
    find vendor -type f -print0 | xargs -0 "${SHA[@]}" | awk '{ print substr($0, 67) "\t" $1 }'
    find vendor -type l | while IFS= read -r l; do printf '%s\tlink -> %s\n' "$l" "$(readlink "$l")"; done
  ) | LC_ALL=C sort
}
digest() { "${SHA[@]}" < "$1" | cut -c1-16; }
PROOF=""
same_trees() { # fixture stage — compares the two manifests of a stage
  local a="$RESULTS/$1-composer-$2.manifest" b="$RESULTS/$1-vivacity-$2.manifest" n
  n=$(wc -l < "$a" | tr -d ' ')
  if ! cmp -s "$a" "$b"; then
    echo "$1 ($2): the two vendor/ trees differ; nothing here is worth timing:" >&2
    diff "$a" "$b" | head -20 >&2
    exit 1
  fi
  [ "$n" -gt 0 ] || { echo "$1 ($2): empty vendor/" >&2; exit 1; }
  PROOF+="| $1 | $2 | $n | \`$(digest "$a")\` | identical |\n"
}
not_delegated() { # log
  if grep -q 'vivacity: delegating to' "$1"; then
    echo "vivacity handed the command over to Composer ($1); this would time Composer twice:" >&2
    grep 'vivacity: delegating to' "$1" >&2
    exit 1
  fi
}
step() { # fixture tool scenario args… — one logged run, from app/
  local log="$RESULTS/$1-$2-$3.log"
  (cd "$(app "$1")" && tool "$2" "${@:4}") > "$log" 2>&1 || { cat "$log" >&2; exit 1; }
}
prove() { # fixture — fresh install, no-op, dump -o on each side, without -q
  local fx="$1" t s
  for t in composer vivacity; do
    enter "$fx" "$t"
    rm -rf "$(app "$fx")/vendor"
    step "$fx" "$t" fresh "${INSTALL[@]}"
    manifest "$(app "$fx")" > "$RESULTS/$fx-$t-fresh.manifest"
    step "$fx" "$t" noop "${INSTALL[@]}"
    step "$fx" "$t" dump-o "${DUMP[@]}"
    manifest "$(app "$fx")" > "$RESULTS/$fx-$t-dump-o.manifest"
    leave "$fx" "$t"
  done
  for s in fresh noop dump-o; do not_delegated "$RESULTS/$fx-vivacity-$s.log"; done
  same_trees "$fx" fresh
  same_trees "$fx" dump-o
}

# Two hyperfine runs per tool, because hyperfine 2.0 runs every --prepare
# through a shell even under -N (cmd.exe /C on Windows when no --shell is
# given), where 1.x ran it without one (run 40927754769: `'true'` refused
# by cmd.exe). `noop` and `dump-o` need no preparation: -N, the timed
# command started directly, no shell to subtract. `fresh` needs vendor/
# deleted before each run: its preparation and its command go through this
# same bash (Git's on Windows, by absolute path), in 1.x as in 2.x;
# hyperfine subtracts the shell start-up it calibrates, a few tens of ms of
# noise on installs of several seconds.
time_tool() { # fixture tool
  local fx="$1" t="$2" d log; d="$(app "$1")"; log="$RESULTS/$1-$2.hyperfine.log"
  enter "$fx" "$t"
  (cd "$d" && hyperfine -N --style basic --warmup 1 --min-runs "$RUNS" --export-json "$RESULTS/$fx-$t-steady.json" \
    -n noop "$(cmdline "$t" "${INSTALL[@]}" -q)" \
    -n dump-o "$(cmdline "$t" "${DUMP[@]}" -q)") > "$log" 2>&1 \
    || { echo "hyperfine failed on $fx/$t (noop, dump-o):" >&2; tail -20 "$log" >&2; exit 1; }
  (cd "$d" && hyperfine --shell "\"$SH\"" --style basic --warmup 1 --min-runs "$RUNS" --export-json "$RESULTS/$fx-$t-fresh.json" \
    --prepare "rm -rf '$d/vendor'" -n fresh "$(cmdline "$t" "${INSTALL[@]}" -q)") >> "$log" 2>&1 \
    || { echo "hyperfine failed on $fx/$t (fresh):" >&2; tail -20 "$log" >&2; exit 1; }
  jq -s '{results: (map(.results) | add)}' "$RESULTS/$fx-$t-steady.json" "$RESULTS/$fx-$t-fresh.json" > "$RESULTS/$fx-$t.json"
  leave "$fx" "$t"
}

# PowerShell by -EncodedCommand (UTF-16LE, base64): Windows PowerShell 5.1
# mangles double quotes inside a -Command argument handed over by MSYS.
pwsh_q() {
  local enc; enc="$(printf '%s' "$1" | iconv -f UTF-8 -t UTF-16LE | base64 -w0)"
  powershell.exe -NoProfile -NonInteractive -EncodedCommand "$enc" 2>&1 | tr -d '\r' | sed '/^[[:space:]]*$/d' || true
}
# PowerShell code in single quotes: its `$_` is not bash's.
# shellcheck disable=SC2016
conditions() {
  local commit; commit="${GITHUB_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo '?')}"
  echo "### Conditions"
  echo
  echo "- $(date -u +%Y-%m-%dT%H:%MZ), commit \`${commit:0:12}\`${GITHUB_RUN_ID:+, run $GITHUB_RUN_ID}"
  echo "- $("$VIVACITY" --version 2>&1 | head -1); $(tool composer --version 2>/dev/null | head -1); PHP $("$PHP" -r 'echo PHP_VERSION;'); $(hyperfine --version)"
  echo "- hyperfine -N, 1 warm-up, at least $RUNS runs per scenario; flags on both sides: \`${INSTALL[*]} -q\` / \`${DUMP[*]} -q\`"
  echo "- layout: $LAYOUT — projects \`$PROJ\`, caches \`$CACHES\` (Composer's zips \`$COMPOSER_CACHE_DIR\`)"
  if [ "$WIN" = 1 ]; then
    echo "- projects on ${PROJ%%:*}:, caches on ${CACHES%%:*}:, Composer's zips on ${COMPOSER_CACHE_DIR%%:*}:"
    echo
    echo '```text'
    pwsh_q 'Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed | Format-List'
    pwsh_q 'Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,@{n="RAM_GB";e={[math]::Round($_.TotalVisibleMemorySize/1MB,1)}} | Format-List'
    echo '--- Defender (Get-MpComputerStatus)'
    pwsh_q 'Get-MpComputerStatus | Select-Object AMRunningMode,AMServiceEnabled,AntivirusEnabled,RealTimeProtectionEnabled,OnAccessProtectionEnabled,IoavProtectionEnabled,BehaviorMonitorEnabled,IsTamperProtected | Format-List'
    echo '--- Defender (Get-MpPreference)'
    pwsh_q 'Get-MpPreference | Select-Object DisableRealtimeMonitoring,ExclusionPath,ExclusionProcess,ExclusionExtension | Format-List'
    echo '--- Volumes'
    pwsh_q 'Get-Volume | Where-Object DriveLetter | Select-Object DriveLetter,FileSystemType,FileSystemLabel,DriveType,@{n="SizeGB";e={[math]::Round($_.Size/1GB)}},@{n="FreeGB";e={[math]::Round($_.SizeRemaining/1GB)}} | Format-Table -AutoSize | Out-String -Width 200'
    pwsh_q 'Get-PhysicalDisk | Select-Object FriendlyName,MediaType,BusType,@{n="SizeGB";e={[math]::Round($_.Size/1GB)}} | Format-Table -AutoSize | Out-String -Width 200'
    echo '```'
  else
    local cpu cores
    if [ "$(uname -s)" = Darwin ]; then
      cpu="$(sysctl -n machdep.cpu.brand_string)"; cores="$(sysctl -n hw.physicalcpu) cores / $(sysctl -n hw.logicalcpu) threads"
      echo "- macOS $(sw_vers -productVersion), $cpu, $cores, $(( $(sysctl -n hw.memsize) / 1073741824 )) GB"
    else
      cpu="$(awk -F': ' '/^model name/{print $2; exit}' /proc/cpuinfo)"; cores="$(nproc) threads"
      echo "- $(sed -n 's/^PRETTY_NAME=//p' /etc/os-release | tr -d '"'), kernel $(uname -r), $cpu, $cores"
    fi
    echo "- filesystems: projects $(df -P "$PROJ" | awk 'NR==2{print $1" on "$6}'), caches $(df -P "$CACHES" | awk 'NR==2{print $1" on "$6}')"
  fi
}

# hyperfine 1.x JSON: the -n name in `command`, statistics at the top of a
# result; 2.x (schema 2): the name in `name`, statistics under
# `summary.time_wall_clock`.
ms() {
  jq -r --arg n "$2" --arg f "$3" \
    '.results[] | select((.name // .command) == $n) | (.summary.time_wall_clock // .) | (.[$f] * 1000 | round)' "$1"
}
cell() { echo "$(ms "$1" "$2" median) ($(ms "$1" "$2" stddev), $(ms "$1" "$2" min)–$(ms "$1" "$2" max))"; }

measure() {
  command -v hyperfine >/dev/null || { echo "hyperfine is required" >&2; exit 1; }
  command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
  SH="$(native bash)"
  mkdir -p "$RESULTS"
  local fx t s c v rows="" pkgs
  for fx in $FIXTURES; do
    [ -d "$PROJ/$fx/vivacity" ] || { echo "$fx is not prepared: run '$0 prepare' first" >&2; exit 1; }
    echo "== $fx: proving the two trees identical" >&2
    prove "$fx"
    for t in composer vivacity; do
      echo "== $fx: timing $t" >&2
      time_tool "$fx" "$t"
      manifest "$PROJ/$fx/$t" > "$RESULTS/$fx-$t-after.manifest"
    done
    same_trees "$fx" after
    pkgs=$(jq '(.packages | length) + (.["packages-dev"] // [] | length)' "$PROJ/$fx/composer/composer.lock")
    for s in noop fresh dump-o; do
      c=$(ms "$RESULTS/$fx-composer.json" "$s" median); v=$(ms "$RESULTS/$fx-vivacity.json" "$s" median)
      rows+="| $fx ($pkgs) | $s | $(cell "$RESULTS/$fx-composer.json" "$s") | $(cell "$RESULTS/$fx-vivacity.json" "$s") | $(awk -v c="$c" -v v="$v" 'BEGIN{ if (v>0) printf "%.1f×", c/v; else print "?" }') |\n"
    done
  done
  {
    echo "## vivacity vs Composer — $OS_NAME ($LAYOUT)"
    echo
    echo "Times in ms: median (σ, min–max). \`fresh\` = vendor/ deleted before each run, caches warm."
    echo
    echo "| fixture (packages) | scenario | Composer | vivacity | Composer / vivacity |"
    echo "|---|---|---|---|---|"
    printf "%b" "$rows"
    echo
    echo "Both vendor/ trees, one sha256 per file: after a fresh install and after dump -o (before the timings), and as hyperfine left them (\`after\`)."
    echo
    echo "| fixture | stage | files + links | manifest sha256 | Composer vs vivacity |"
    echo "|---|---|---|---|---|"
    printf "%b" "$PROOF"
    echo
    echo "For reference, Luther Monson's medians (Ryzen 9 5950X, Windows 11, Defender on, vivacity 0.17 in ePHPm v0.11.2, Laravel, 2–3 runs, blocking-list request included): fresh 107.3 s / 13.9 s, no-op 6.7 s / 0.73 s, dump -o 6.5 s / 0.38 s."
    echo
    conditions
  } > "$RESULTS/summary.md"
  cat "$RESULTS/summary.md"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then cat "$RESULTS/summary.md" >> "$GITHUB_STEP_SUMMARY"; fi
}

case "$PHASE" in
  prepare) prepare ;;
  measure) measure ;;
  all) prepare; measure ;;
esac
