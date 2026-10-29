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
# GitHub refusing requests is told apart and NOT retried: its limits are
# counted per hour (or per IP, on shared macOS runners), so a run one minute
# later spends the same budget again and fails the same way (macOS,
# 2026-10-29: "Could not authenticate against github.com" in every case of
# flex-update.sh). The harness keeps its exit code — red stays red — and the
# reason is named: which wording was seen (Composer's, so the reference side
# or a hand-over to Composer; or vivacity's alone, which may be vivacity
# sending a credential Composer would not), and whether COMPOSER_AUTH was set
# (its value is never printed). harness/net-retry-selftest.sh replays the
# captured log.
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

# GitHub refusing a request, as each tool words it when it gives up.
# Composer, non-interactive: a 401/403 from a github-domains origin
# (AuthHelper::promptAuthIfNeeded), a 403 from codeload (its own origin),
# a 429 it does not retry (CurlDownloader::failResponse). vivacity: its
# client's status error on the same hosts. Matched on the log with ALL
# whitespace removed: Composer prints its message in Symfony's error box,
# cut at the terminal width (80 columns without a tty) at any character
# and padded with spaces, so a long message is only whole once its pieces
# are put back together. The patterns are written without spaces.
GITHUB_REFUSAL_COMPOSER='Couldnotauthenticateagainstgithub\.com|'"'"'https?://codeload\.github\.com/[^'"'"']*'"'"'URLcouldnotbeaccessed\(HTTP403\)|"https?://(api|codeload)\.github\.com/[^"]*"filecouldnotbedownloaded\(HTTP/[0-9.]+429'
GITHUB_REFUSAL_VIVACITY='\((401Unauthorized|403Forbidden|429TooManyRequests)\)forurl\(https?://(api|codeload)\.github\.com/'
# What to quote of the log when one matched.
GITHUB_REFUSAL_LINES='Could not authenticate against|could not be accessed \(HTTP|could not be downloaded \(HTTP|Unauthorized\) for url|Forbidden\) for url|Too Many Requests\) for url'

"$@" 2>&1 | tee "$log"
code=${PIPESTATUS[0]}
[ "$code" = 0 ] && exit 0

flat=$(tr -d ' \t\r\n' < "$log")
if printf '%s' "$flat" | grep -qE "$GITHUB_REFUSAL_COMPOSER|$GITHUB_REFUSAL_VIVACITY"; then
  echo "net-retry: $1 failed (exit $code) with GitHub refusing requests; not run again (a second run spends the same limit):"
  grep -E "$GITHUB_REFUSAL_LINES" "$log" | head -3 | sed 's/^ */net-retry:   /'
  if printf '%s' "$flat" | grep -qE "$GITHUB_REFUSAL_COMPOSER"; then
    side="in Composer's wording (the reference, or a hand-over to Composer): GitHub's rate limit or the token, not a difference from vivacity"
  else
    side="in vivacity's wording only: GitHub's rate limit, or vivacity sending a credential Composer would not; read the log before running it again"
  fi
  if [ -n "${COMPOSER_AUTH:-}" ]; then token="COMPOSER_AUTH was set"; else token="COMPOSER_AUTH was not set (anonymous unless an auth.json held a token)"; fi
  echo "net-retry: seen $side; $token"
  if [ "${GITHUB_ACTIONS:-}" = true ]; then
    echo "::error title=GitHub refused requests::$1 failed with GitHub refusing requests, seen $side; $token"
  fi
  exit "$code"
fi

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
