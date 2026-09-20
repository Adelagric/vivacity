#!/usr/bin/env bash
# Plugins that vivacity does not emulate must hand over to Composer BEFORE
# touching the disk, naming the plugin. Case: the Drupal recommended-project,
# whose lock carries drupal/core-composer-scaffold (not emulated: its source
# is GPL-2.0-or-later, see NOTICE.md). `vivacity install --no-fallback` must
# refuse with exit code 3 and leave the tree untouched; the fallback path
# itself is proven by `harness/diff-vendor.sh drupal` (whole project
# identical to Composer's).
#
# Usage: harness/transitions.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
VIVACITY="$ROOT/target/release/vivacity"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/transitions"
harness_git_env "$WORK"
[ -x "$VIVACITY" ] || { echo "binary missing: cargo build --release"; exit 1; }
src="$ROOT/fixtures/work/drupal"
dir="$WORK/drupal-scaffold"
rm -rf "$dir"; mkdir -p "$dir"
(cd "$src" && tar --exclude=./.git --exclude=./vendor --exclude=./web --exclude=./recipes -cf - .) | (cd "$dir" && tar -xf -)
cd "$dir"
before=$( (find . -type d | sort; find . -type f -exec shasum -a 256 {} +) | sort | shasum -a 256)
code=0
"$VIVACITY" install --no-fallback --offline >"$WORK/scaffold.log" 2>&1 || code=$?
if [ "$code" != 3 ]; then
  echo "FAIL drupal: expected exit 3 (out of scope, no fallback), got $code:"; tail -5 "$WORK/scaffold.log"; exit 1
fi
if ! grep -q 'drupal/core-composer-scaffold' "$WORK/scaffold.log"; then
  echo "FAIL drupal: the refusal does not name the plugin:"; tail -5 "$WORK/scaffold.log"; exit 1
fi
after=$( (find . -type d | sort; find . -type f -exec shasum -a 256 {} +) | sort | shasum -a 256)
[ "$before" = "$after" ] || { echo "FAIL drupal: the tree was modified before the refusal"; exit 1; }
echo "OK   drupal: core-composer-scaffold handed over to Composer without touching the disk"

# The resolution commands (plan v0.16): an installed, allowed plugin whose
# effect vivacity does not emulate hands the whole command to Composer
# BEFORE any write; `--no-fallback` refuses with exit 3, naming the plugin,
# the tree untouched (composer.json included: require/remove decide before
# their edit). symfony/flex here: `require` (aliases, recipes) always; the
# symfony-demo fixture has an importmap.php, which Flex would synchronise on
# `POST_UPDATE_CMD` — so `update` and `remove` too (the pool filter itself
# is emulated: `harness/update.sh` and `steps.sh` prove the native path).
src="$ROOT/fixtures/work/symfony"
dir="$WORK/flex-resolution"
rm -rf "$dir"; mkdir -p "$dir"
(cd "$src" && tar --exclude=./.git --exclude=./var --exclude=./node_modules -cf - .) | (cd "$dir" && tar -xf -)
cd "$dir"
jq -e '.config["allow-plugins"]["symfony/flex"] == true' composer.json >/dev/null || { echo "FAIL flex: fixture does not allow symfony/flex"; exit 1; }
grep -q '"name": "symfony/flex"' vendor/composer/installed.json || { echo "FAIL flex: symfony/flex not in installed.json"; exit 1; }
before=$( (find . -type d | sort; find . -type f -exec shasum -a 256 {} +) | sort | shasum -a 256)
for cmd in "update --no-install" "require psr/log --no-install" "remove symfony/uid --no-install"; do
  code=0; log="$WORK/flex-${cmd%% *}.log"
  # shellcheck disable=SC2086
  "$VIVACITY" $cmd --no-fallback --offline >"$log" 2>&1 || code=$?
  if [ "$code" != 3 ]; then
    echo "FAIL flex \`$cmd\`: expected exit 3, got $code:"; tail -5 "$log"; exit 1
  fi
  if ! grep -q 'symfony/flex' "$log"; then
    echo "FAIL flex \`$cmd\`: the refusal does not name the plugin:"; tail -5 "$log"; exit 1
  fi
  case "$cmd" in
    require*) grep -q 'aliases' "$log" || { echo "FAIL flex \`$cmd\`: expected the aliases/recipes reason:"; tail -5 "$log"; exit 1; } ;;
    *) grep -q 'importmap.php' "$log" || { echo "FAIL flex \`$cmd\`: expected the importmap.php synchronisation reason:"; tail -5 "$log"; exit 1; } ;;
  esac
  after=$( (find . -type d | sort; find . -type f -exec shasum -a 256 {} +) | sort | shasum -a 256)
  [ "$before" = "$after" ] || { echo "FAIL flex \`$cmd\`: the tree was modified before the refusal"; exit 1; }
done
echo "OK   flex: require (aliases), update/remove (importmap.php sync) handed over to Composer without touching the disk"

