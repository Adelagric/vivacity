#!/usr/bin/env bash
# Runs a harness, and runs it once more when it failed on the network.
#
# Several harnesses resolve against the live Packagist (plugin-lines.sh,
# verbosity.sh's online part, bin-plugin.sh, yii2-composer.sh, merge-plugin.sh…).
# A CI runner that loses its DNS for a minute makes Composer and vivacity both
# fall back to their caches with different wording, or makes the reference
# fail outright: the harness goes red on the network, not on vivacity
# (macOS runners, 2026-10-22 and 2026-10-23).
#
# The rule: if the harness exits non-zero AND its output carries a network
# error (from Composer's curl or vivacity's HTTP client), wait and run it once
# more; the second run's exit code is the result. A regression in vivacity
# fails twice and stays red; a passing harness runs once.
#
# Usage: harness/net-retry.sh harness/<name>.sh [args…]
# Variables: NET_RETRY_WAIT (seconds before the second run, default 60).
set -uo pipefail

[ $# -ge 1 ] || { echo "usage: $0 harness/<name>.sh [args…]" >&2; exit 2; }
log=$(mktemp "${TMPDIR:-/tmp}/net-retry.XXXXXX")
trap 'rm -f "$log"' EXIT

# What a dropped network looks like on either side: Composer's curl errors
# (6 resolve, 7 connect, 28 timeout), the system resolver, vivacity's client.
NETWORK_ERROR='Could not resolve host|curl error (6|7|28) |Temporary failure in name resolution|Network is unreachable|Connection timed out after|error sending request for url|dns error|failed to lookup address'

"$@" 2>&1 | tee "$log"
code=${PIPESTATUS[0]}
[ "$code" = 0 ] && exit 0

if grep -qE "$NETWORK_ERROR" "$log"; then
  wait=${NET_RETRY_WAIT:-60}
  echo "net-retry: $1 failed (exit $code) with network errors in its output:"
  grep -E "$NETWORK_ERROR" "$log" | head -3 | sed 's/^/net-retry:   /'
  echo "net-retry: running it once more in ${wait}s"
  sleep "$wait"
  "$@"
  code=$?
  echo "net-retry: second run of $1 exited $code"
fi
exit "$code"
