#!/usr/bin/env bash
# Harness différentiel v0.11 : `config.vendor-dir` et `config.bin-dir`.
# Une fixture complète (fixtures/work/<fx>, mise en scène comme dans
# diff-vendor.sh) est copiée avec son composer.json modifié par jq — le
# content-hash du lock ignore `config` (sauf `config.platform`, jamais
# touché ici). Trois étapes par variante, projet entier comparé à chaque
# fois (harness/lib/compare.sh, VENDOR_REL) :
#   1. install --no-scripts : arbre + lignes « Skipped installation of bin »
#      (un bin-dir du projet contient déjà bin/console et bin/phpunit) ;
#   2. dump-autoload -o : arbre ;
#   3. install (no-op) : arbre (la sortie d'erreur d'un no-op est celle des
#      plugins de la référence — flex, funding — comparée ailleurs, steps.sh).
# vivacity tourne avec --no-fallback : une variante non couverte échoue au
# lieu d'être rendue à Composer.
#
# Usage : harness/vendor-dir.sh [variante...]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="$ROOT/target/release/vivacity"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/vendor-dir"

# nom~fixture~filtre jq sur composer.json~vendor relatif attendu~env
VARIANTS=(
  "lib-vendor~symfony~.config[\"vendor-dir\"]=\"lib/vendor\"~lib/vendor~"
  "bin~symfony~.config[\"bin-dir\"]=\"bin\"~vendor~"
  "src-vendor-tools~symfony~.config[\"vendor-dir\"]=\"src/vendor\" | .config[\"bin-dir\"]=\"tools/\"~src/vendor~"
  "nested~symfony~.config[\"vendor-dir\"]=\"./vendor/composer/vendor\"~vendor/composer/vendor~"
  "env-placeholder~symfony~.config[\"bin-dir\"]=\"{\$vendor-dir}/tools\"~env-vendor~COMPOSER_VENDOR_DIR=env-vendor"
  "installers-lib-vendor~wordpress~.config[\"vendor-dir\"]=\"lib/vendor\"~lib/vendor~"
)
ONLY=("$@")

[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK"
harness_git_env "$WORK"
status=0
for spec in "${VARIANTS[@]}"; do
  IFS='~' read -r name fx filter vrel env <<< "$spec"
  if [ ${#ONLY[@]} -gt 0 ]; then
    keep=0; for o in "${ONLY[@]}"; do [ "$o" = "$name" ] && keep=1; done; [ $keep = 1 ] || continue
  fi
  src="$ROOT/fixtures/work/$fx"
  ref="$WORK/ref-$name"; viv="$WORK/viv-$name"
  rm -rf "$ref" "$viv"; mkdir -p "$ref" "$viv"
  for d in "$ref" "$viv"; do
    (cd "$src" && tar --exclude=./.git --exclude=./vendor --exclude=./node_modules --exclude=./var --exclude=./web --exclude=./wp-content --exclude=./recipes --exclude=./.editorconfig --exclude=./.gitattributes -cf - .) | (cd "$d" && tar -xf -)
    jq "$filter" "$src/composer.json" > "$d/composer.json"
    (cd "$d" && git init -q -b main && git add -A >/dev/null && \
      GIT_AUTHOR_NAME=vivacity GIT_AUTHOR_EMAIL=v@v GIT_AUTHOR_DATE="2026-09-10T00:00:00Z" \
      GIT_COMMITTER_NAME=vivacity GIT_COMMITTER_EMAIL=v@v GIT_COMMITTER_DATE="2026-09-10T00:00:00Z" \
      git commit -q -m fixture)
  done
  if jq -e '(.config["allow-plugins"] // {}) | if type == "object" then any(.[]; . == true) else . == true end' "$src/composer.json" >/dev/null 2>&1; then
    plugin_flag=""
  else
    plugin_flag="--no-plugins"
  fi
  ok=1
  run_ref() { (cd "$ref" && env ${env:+"$env"} composer "$@" --no-interaction --no-ansi $plugin_flag 2>"$WORK/$name.$step.composer.err" >/dev/null); }
  # --no-fallback : une variante non couverte échoue (dump-autoload n'a pas
  # de repli : il s'arrête de lui-même sur un layout refusé).
  run_viv() { (cd "$viv" && env ${env:+"$env"} "$VIVACITY" "$@" ${nofb:+"$nofb"} 2>"$WORK/$name.$step.vivacity.err" >/dev/null); }
  for step in install dump-o noop; do
    nofb="--no-fallback"
    case "$step" in
      install) args=(install --no-scripts) ;;
      dump-o)  args=(dump-autoload -o); nofb="" ;;
      noop)    args=(install --no-scripts) ;;
    esac
    if ! run_ref "${args[@]}"; then
      echo "FAIL $name/$step : composer a échoué :"; tail -5 "$WORK/$name.$step.composer.err"; ok=0; break
    fi
    if ! run_viv "${args[@]}"; then
      echo "FAIL $name/$step : vivacity a échoué :"; tail -5 "$WORK/$name.$step.vivacity.err"; ok=0; break
    fi
    if ! VENDOR_REL="$vrel" compare_vendor "$ref" "$viv" "$WORK/$name.$step.diff" >/dev/null; then
      echo "FAIL $name/$step : projet différent ($WORK/$name.$step.diff)"; head -20 "$WORK/$name.$step.diff"; ok=0; break
    fi
    if [ "$step" = install ]; then
      if ! diff <(grep 'Skipped installation of bin' "$WORK/$name.$step.composer.err" || true) \
                <(grep 'Skipped installation of bin' "$WORK/$name.$step.vivacity.err" || true) >/dev/null; then
        echo "FAIL $name/$step : lignes « Skipped installation of bin » différentes"
        diff <(grep 'Skipped installation of bin' "$WORK/$name.$step.composer.err" || true) <(grep 'Skipped installation of bin' "$WORK/$name.$step.vivacity.err" || true) | head; ok=0; break
      fi
    fi
  done
  if [ $ok = 1 ]; then
    n=$(find "$ref/$vrel" -maxdepth 2 -mindepth 2 -type d | wc -l | tr -d ' ')
    echo "OK   $name ($fx, $vrel) : install, dump -o, no-op identiques ($n paquets)"
  else
    status=1
  fi
done
exit $status
