#!/usr/bin/env bash
# The Linux release binaries: built against an old glibc (or statically
# against musl), then checked against the oldest system they promise to run
# on (docs/plans/linux-glibc-floor.md). release.yml and action-test.yml run
# both steps; so can a Mac with Docker, for a dry run.
#
#   tools/linux-release.sh build <triple>
#       cargo zigbuild → target/<triple>/release/vivacity. The toolchain
#       (tools/zigbuild-requirements.txt, pinned by hash) goes into a venv,
#       $ZIGBUILD_VENV, else zigbuild-venv under $RUNNER_TEMP or target/.
#       The Rust target must be installed (rustup target add <triple>).
#   tools/linux-release.sh check <triple> [binary]
#       1. ELF: gnu → no GLIBC_ version above GLIBC_FLOOR among the version
#          references (weak ones included: a glibc that lacks a weak version
#          still prints a warning on stderr), NEEDED within the base
#          libraries; musl → static (no NEEDED, no interpreter).
#       2. Docker, in the oldest supported image of that libc: `--version`
#          with an empty stderr, a `-vv` run handed to a stand-in composer
#          (a child process started and its exit code read back), and a real
#          `vivacity install` of fixtures/projects/scripts over the network.
#       Every check runs; the exit code is 1 if any failed.
#   tools/linux-release.sh image <triple>
#       Prints that image (pinned by digest), for other checks to reuse.
#
# <triple>: x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu,
#           x86_64-unknown-linux-musl, aarch64-unknown-linux-musl
# OBJDUMP: GNU or LLVM objdump (default: objdump).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# The promise: Rust's own floor for these targets (CentOS/RHEL 7).
GLIBC_FLOOR=2.17
# Multi-arch indexes (amd64 + arm64). centos:7 is frozen (EOL); nothing is
# installed in it, the binary only has to start there.
GNU_IMAGE="centos:7@sha256:be65f488b7764ad3638f236b7b515b3678369a5124c47b8d32916d6487418ea4"
MUSL_IMAGE="alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6"

usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0" >&2; exit 2; }
[ $# -ge 2 ] || usage
cmd="$1"; triple="$2"
case "$triple" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) libc=gnu ;;
  x86_64-unknown-linux-musl|aarch64-unknown-linux-musl) libc=musl ;;
  *) echo "linux-release: unsupported target: $triple" >&2; exit 2 ;;
esac
arch="${triple%%-*}"

build() {
  # Not under target/ on a runner: rust-cache would restore a venv whose
  # python may be gone from the next runner image.
  venv="${ZIGBUILD_VENV:-${RUNNER_TEMP:-$ROOT/target}/zigbuild-venv}"
  [ -x "$venv/bin/pip" ] || python3 -m venv "$venv"
  "$venv/bin/pip" install --quiet --disable-pip-version-check --require-hashes \
    --only-binary :all: -r "$ROOT/tools/zigbuild-requirements.txt"
  # gnu: the glibc version zig links against; musl: static.
  t="$triple"; [ $libc = gnu ] && t="$triple.$GLIBC_FLOOR"
  (cd "$ROOT" && PATH="$venv/bin:$PATH" cargo zigbuild --release --locked -p vivacity --target "$t")
  echo "linux-release: built target/$triple/release/vivacity ($t)" >&2
}

status=0
fail() { echo "FAIL $*"; status=1; }

check_elf() {
  local bin="$1" od="${OBJDUMP:-objdump}" headers format sections
  # Read whole, then searched: a `grep -q` on objdump's pipe can SIGPIPE it
  # (141 under pipefail), which an `if` would read as "not found".
  headers="$("$od" -p "$bin" 2>&1)" || { fail "objdump -p: $headers"; return; }
  sections="$("$od" -h "$bin" 2>&1)" || { fail "objdump -h: $sections"; return; }
  format="$("$od" -f "$bin" 2>&1 | sed -n 's/.*file format //p')"
  case "$arch:$format" in
    x86_64:elf64-x86-64|aarch64:elf64-littleaarch64) echo "OK   $format" ;;
    *) fail "$bin is '$format', not a $arch ELF" ;;
  esac
  local needed
  needed="$(echo "$headers" | awk '$1 == "NEEDED" {printf "%s ", $2}')"
  if [ $libc = musl ]; then
    if [ -n "$needed" ] || echo "$sections" | grep -q ' \.interp '; then
      fail "not static: NEEDED [$needed]"
    else
      echo "OK   static (no NEEDED, no interpreter)"
    fi
    return
  fi
  # All the GLIBC_ versions the loader checks, weak ones included.
  local verrefs refs max other
  verrefs="$(echo "$headers" | sed -n '/^Version References:/,/^$/p')"
  refs="$(echo "$verrefs" | { grep -o 'GLIBC_[0-9][0-9.]*' || true; } | sed 's/^GLIBC_//' | sort -uV)"
  # Non-numeric nodes (GLIBC_PRIVATE, GLIBC_ABI_DT_RELR from 2.36) have no
  # place in a binary meant for 2.17.
  other="$(echo "$verrefs" | { grep -oE 'GLIBC_[A-Z][A-Z0-9_]*' || true; } | sort -u)"
  [ -z "$other" ] || fail "non-numeric glibc version nodes: $(echo "$other" | tr '\n' ' ')"
  max="$(echo "$refs" | tail -n 1)"
  if [ -z "$max" ]; then
    fail "no GLIBC_ version reference found (wrong binary, or objdump output not understood)"
  elif [ "$(printf '%s\n%s\n' "$max" "$GLIBC_FLOOR" | sort -V | tail -n 1)" = "$GLIBC_FLOOR" ]; then
    echo "OK   highest glibc version required: GLIBC_$max (floor $GLIBC_FLOOR)"
  else
    fail "requires GLIBC_$max, above the floor $GLIBC_FLOOR:"
    echo "$verrefs" | grep -E 'GLIBC_[0-9.]+' | while read -r _ flags _ v; do
      [ "$(printf '%s\n%s\n' "${v#GLIBC_}" "$GLIBC_FLOOR" | sort -V | tail -n 1)" = "$GLIBC_FLOOR" ] || echo "       $v (flags $flags)"
    done
  fi
  local lib bad=""
  for lib in $needed; do
    case "$lib" in
      libc.so.6|libm.so.6|libpthread.so.0|libdl.so.2|librt.so.1|libgcc_s.so.1|ld-linux-x86-64.so.2|ld-linux-aarch64.so.1) ;;
      *) bad="$bad $lib" ;;
    esac
  done
  if [ -n "$bad" ]; then fail "NEEDED outside the base libraries:$bad"; else echo "OK   NEEDED: $needed"; fi
}

# Runs in the image, with the binary at /opt/vivacity and the fixture at /src.
# shellcheck disable=SC2016 # expanded by the container's shell
SMOKE='
set -u
v=/opt/vivacity
rc=0
# 1. It loads, and the loader writes nothing (a missing weak version only warns).
if "$v" --version > /tmp/version.out 2> /tmp/version.err && [ ! -s /tmp/version.err ]; then
  echo "OK   --version: $(cat /tmp/version.out)"
else
  echo "FAIL --version:"; cat /tmp/version.out /tmp/version.err; exit 1
fi
cp -R /src /p && cd /p || { echo "FAIL fixture copy"; exit 1; }
# 2. A child process: -vv is handed to Composer, here a stand-in.
mkdir -p /tmp/bin
printf "#!/bin/sh\necho \"stand-in composer: \$*\"\nexit 7\n" > /tmp/bin/composer
chmod +x /tmp/bin/composer
PATH="/tmp/bin:$PATH" "$v" install -vv > /tmp/spawn.out 2> /tmp/spawn.err
code=$?
if [ $code -eq 7 ] && grep -q "^stand-in composer: install" /tmp/spawn.out && ! grep -q GLIBC_ /tmp/spawn.err; then
  echo "OK   -vv handed to a child process, its exit code 7 read back"
else
  echo "FAIL -vv hand-over (exit $code):"; cat /tmp/spawn.out /tmp/spawn.err; rc=1
fi
# 3. A real install: TLS, download, extraction, autoloader. One retry for
#    the network (a loader failure does not go away on a retry).
ok=0
for attempt in 1 2; do
  rm -rf vendor
  if "$v" install --no-interaction > /tmp/install.out 2> /tmp/install.err; then ok=1; break; fi
  [ $attempt -eq 1 ] && { echo "install failed, retrying in 15 s:"; tail -n 3 /tmp/install.err; sleep 15; }
done
if [ $ok -eq 1 ] && [ -f vendor/autoload.php ] && [ -f vendor/psr/log/src/LoggerInterface.php ] \
   && [ -f vendor/psr/container/src/ContainerInterface.php ] && ! grep -q GLIBC_ /tmp/install.err; then
  echo "OK   install: $(tail -n 1 /tmp/install.err)"
else
  echo "FAIL install:"; cat /tmp/install.out /tmp/install.err; rc=1
fi
exit $rc
'

check_container() {
  local bin="$1" image platform
  image="$(image)"
  platform=linux/amd64; [ "$arch" = aarch64 ] && platform=linux/arm64
  echo "--   ${image%%@*} ($platform)"
  docker run --rm --platform "$platform" \
    -v "$bin:/opt/vivacity:ro" -v "$ROOT/fixtures/projects/scripts:/src:ro" \
    "$image" sh -c "$SMOKE" || status=1
}

check() {
  local bin="${1:-$ROOT/target/$triple/release/vivacity}"
  [ -f "$bin" ] || { echo "linux-release: binary not found: $bin" >&2; exit 2; }
  bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"
  echo "== $triple: $bin"
  check_elf "$bin"
  check_container "$bin"
  if [ $status -eq 0 ]; then echo "== OK $triple"; else echo "== FAILED $triple"; fi
  exit $status
}

image() { if [ $libc = musl ]; then echo "$MUSL_IMAGE"; else echo "$GNU_IMAGE"; fi; }

case "$cmd" in
  build) build ;;
  image) image ;;
  check) check "${3:-}" ;;
  *) usage ;;
esac
