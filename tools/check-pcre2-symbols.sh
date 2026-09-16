#!/usr/bin/env bash
# Every defined global symbol of vivacity's PCRE2 build must carry the
# `vivacity_` prefix (docs/plans/v0.12-pcre2-prefix.md): otherwise a program
# that embeds vivacity and links another PCRE2 (PHP's libphp.a) gets
# duplicate symbols. Linux and macOS (`nm`); Windows is covered by the link
# test (crates/pcre2-link-test).
# Usage: tools/check-pcre2-symbols.sh [target dir, default target]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$ROOT/target}"
archives=$(find "$TARGET" -path '*/build/vivacity-pcre2-sys-*/out/libvivacity_pcre2.a' 2>/dev/null)
[ -n "$archives" ] || { echo "no libvivacity_pcre2.a under $TARGET (build first)"; exit 1; }
status=0
for a in $archives; do
  # `nm -g --defined-only` prints object headers (`x.o:`) and blank lines
  # between members; a symbol line is `<addr> <type> <name>`.
  syms=$(nm -g --defined-only "$a" | awk 'NF == 3 {print $3}' | sort -u)
  total=$(echo "$syms" | grep -c . || true)
  bad=$(echo "$syms" | grep -v '^_\{0,1\}vivacity_' || true)
  if [ -n "$bad" ]; then
    echo "FAIL $a : $(echo "$bad" | wc -l | tr -d ' ') of $total defined globals without the vivacity_ prefix:"; echo "$bad" | head -20; status=1
  else
    echo "OK   $a : $total defined globals, all prefixed vivacity_"
  fi
done
exit $status
