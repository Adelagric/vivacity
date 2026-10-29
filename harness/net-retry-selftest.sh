#!/usr/bin/env bash
# What harness/net-retry.sh does with each kind of failure, on fake harnesses
# whose output and exit code are scripted, and on the log captured from the
# run that motivated the GitHub case (ci run 40894179601, parity
# (macos-latest), 2026-10-29: fixtures/net-retry/flex-update-macos-2026-10-29.log,
# the runner's timestamps stripped).
#
# Each case states the exit code net-retry must return, how many times the
# fake harness must have run, and lines its output must (+) or must not (-)
# carry. Under macOS's bash 3.2 as under bash 5.
#
# Usage: harness/net-retry-selftest.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NET_RETRY="${NET_RETRY:-$ROOT/harness/net-retry.sh}"
CAPTURED="$ROOT/fixtures/net-retry/flex-update-macos-2026-10-29.log"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/net-retry-selftest.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export NET_RETRY_WAIT=0
status=0

# fake <name> <script body> : a harness that counts its runs in $WORK/<name>.runs.
fake() {
  printf '#!/usr/bin/env bash\necho x >> "%s/%s.runs"\nn=$(wc -l < "%s/%s.runs" | tr -d " ")\n%s\n' \
    "$WORK" "$1" "$WORK" "$1" "$2" > "$WORK/$1.sh"
  chmod +x "$WORK/$1.sh"
}

# check <name> <expected exit> <expected runs> [+motif|-motif]... (extended regex)
check() {
  local name="$1" want_code="$2" want_runs="$3" code runs out m
  shift 3
  out="$WORK/$name.out"
  "$NET_RETRY" "$WORK/$name.sh" > "$out" 2>&1
  code=$?
  runs=$(wc -l < "$WORK/$name.runs" 2>/dev/null | tr -d ' ')
  local bad=""
  [ "$code" = "$want_code" ] || bad="$bad exit $code (want $want_code);"
  [ "$runs" = "$want_runs" ] || bad="$bad $runs run(s) (want $want_runs);"
  for m in "$@"; do
    case "$m" in
      +*) grep -qE -- "${m#+}" "$out" || bad="$bad missing /${m#+}/;" ;;
      -*) grep -qE -- "${m#-}" "$out" && bad="$bad unexpected /${m#-}/;" ;;
    esac
  done
  if [ -z "$bad" ]; then
    echo "OK   $name"
  else
    echo "FAIL $name:$bad"; sed 's/^/     | /' "$out" | tail -8; status=1; return 1
  fi
}

[ -f "$CAPTURED" ] || { echo "missing $CAPTURED"; exit 1; }

# The captured macOS log: red, run once, the cause named, Composer's side.
# Outside GitHub Actions and without a token, whatever the caller's
# environment (ci.yml sets both).
fake captured "cat '$CAPTURED'; exit 1"
( unset GITHUB_ACTIONS COMPOSER_AUTH
  check captured 1 1 '+GitHub refusing requests; not run again' "+in Composer's wording" \
    '+COMPOSER_AUTH was not set' '-running it once more' '-::error' ) || status=1

# The same under GitHub Actions with a token configured: the annotation, the
# token's presence, never its value.
fake captured-ci "cat '$CAPTURED'; exit 1"
( export GITHUB_ACTIONS=true COMPOSER_AUTH='{"github-oauth":{"github.com":"SELFTEST-SECRET"}}'
  check captured-ci 1 1 '+^::error title=GitHub refused requests::' '+COMPOSER_AUTH was set' '-SELFTEST-SECRET' ) || status=1

# vivacity's client alone refused: named as possibly vivacity's own doing.
fake vivacity-403 "echo 'vivacity: HTTP status client error (403 Forbidden) for url (https://api.github.com/repos/a/b/zipball/c)'; exit 1"
check vivacity-403 1 1 "+in vivacity's wording only" '-running it once more'

# Composer's 429 (not retried by CurlDownloader) and codeload's 403.
fake composer-429 "echo 'The \"https://api.github.com/repos/a/b/zipball/c\" file could not be downloaded (HTTP/2 429 )'; exit 1"
check composer-429 1 1 "+in Composer's wording"
fake codeload-403 "echo \"The 'https://codeload.github.com/a/b/legacy.zip/c' URL could not be accessed (HTTP 403): Forbidden\"; exit 1"
check codeload-403 1 1 "+in Composer's wording"

# The same two messages as Composer really prints them without a tty:
# inside Symfony's error box, cut every 76 characters wherever that falls
# (mid-word, mid-URL), each piece padded.
boxed() { printf '\n  %s\n' "$1" | fold -w 76 | sed 's/^/  /; s/$/    /'; }
boxed 'The "https://api.github.com/repos/some-vendor/some-package/zipball/0123456789abcdef0123456789abcdef01234567" file could not be downloaded (HTTP/2 429 )' > "$WORK/429.boxed"
fake composer-429-boxed "cat '$WORK/429.boxed'; exit 1"
check composer-429-boxed 1 1 "+in Composer's wording" '-running it once more'
boxed "The 'https://codeload.github.com/some-vendor/some-package/legacy.zip/0123456789abcdef0123456789abcdef01234567' URL could not be accessed (HTTP 403): Forbidden" > "$WORK/403.boxed"
fake codeload-403-boxed "cat '$WORK/403.boxed'; exit 1"
check codeload-403-boxed 1 1 "+in Composer's wording" '-running it once more'

# A refusal next to a network error: still not run again.
fake both "echo 'curl error 6 while downloading'; echo 'Could not authenticate against github.com'; exit 1"
check both 1 1 '+GitHub refusing requests' '-running it once more'

# Unchanged behaviour: a network error then green runs twice and passes; a
# network error twice stays red; a difference without network fails once;
# green runs once; a passing harness that prints the GitHub message is green.
fake net-then-green 'if [ "$n" = 1 ]; then echo "Could not resolve host: repo.packagist.org"; exit 1; fi; exit 0'
check net-then-green 0 2 '+running it once more'
fake net-twice 'echo "Could not resolve host: repo.packagist.org"; exit 1'
check net-twice 1 2 '+second run of'
fake diff 'echo "FAIL case : stderr différente"; exit 1'
check diff 1 1 '-net-retry:'
fake green 'echo "OK   case"; exit 0'
check green 0 1 '-net-retry:'
fake green-mentions 'echo "OK   expected: Could not authenticate against github.com"; exit 0'
check green-mentions 0 1 '-net-retry:'

exit $status
