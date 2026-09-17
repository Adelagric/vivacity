#!/usr/bin/env bash
# Dist extraction must decompress in pure Rust
# (docs/plans/v0.12-pure-rust-extraction.md): a C decompressor's unprefixed
# symbols (BZ2_*, lzma_*, ZSTD_*, inflate/deflate) collide with a host that
# links its own — PHP's static builds bundle bz2 and zlib everywhere, lzma
# and zstd depending on the platform. Three checks on the resolved graph:
#   1. no crate that compiles an unprefixed C library is present at all
#      (lzma-sys, zstd-sys, libz-sys, and their wrappers xz2/zstd);
#   2. bzip2-sys — still named by the zip crate's optional dependency —
#      resolves with the `__disabled` feature only, so its build script
#      compiles nothing;
#   3. libbz2-rs-sys carries `semver-prefix`, so the pure-Rust bzip2
#      exports LIBBZ2_RS_SYS_v…-prefixed symbols, never BZ2_*.
# Usage: tools/check-clib-deps.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
tree=$(cargo tree -e normal --prefix none -f '{p} {f}' | sort -u)
status=0

for crate in lzma-sys zstd-sys libz-sys xz2 zstd zstd-safe; do
  if echo "$tree" | grep -q "^$crate v"; then
    echo "FAIL: $crate is in the dependency graph:"
    cargo tree -i "$crate" -e normal | head -8
    status=1
  fi
done

bzip2_sys=$(echo "$tree" | grep '^bzip2-sys v' || true)
if [ -n "$bzip2_sys" ]; then
  case "$bzip2_sys" in
    *" __disabled") echo "OK   bzip2-sys resolves with __disabled only ($bzip2_sys)" ;;
    *) echo "FAIL: bzip2-sys resolves with features other than __disabled: $bzip2_sys"; status=1 ;;
  esac
fi

libbz2=$(echo "$tree" | grep '^libbz2-rs-sys v' || true)
if [ -z "$libbz2" ]; then
  echo "FAIL: libbz2-rs-sys missing — bzip2 zip entries would not decompress"
  status=1
elif echo "$libbz2" | grep -q 'semver-prefix'; then
  echo "OK   libbz2-rs-sys carries semver-prefix ($libbz2)"
else
  echo "FAIL: libbz2-rs-sys without semver-prefix — exports unprefixed BZ2_*: $libbz2"
  status=1
fi

[ $status -eq 0 ] && echo "OK   no unprefixed C decompressor reaches an embedder's link"
exit $status
