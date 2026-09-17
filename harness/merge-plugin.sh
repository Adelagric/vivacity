#!/usr/bin/env bash
# Harness différentiel v0.13 : `wikimedia/composer-merge-plugin`.
# La fixture fixtures/projects/merge-plugin (racine + trois manifestes
# inclus, dont un requis et un imbriqué) est mise en scène comme dans
# diff-vendor.sh. La référence est l'état stable de Composer : le plugin
# pré-installé seul (manifeste jetable, vendor copié), puis
# `composer install` — plugin actif dès INIT, pas de mise à jour implicite,
# lock inchangé (vérifié). vivacity installe sur un vendor vierge, en natif
# (--no-fallback). Projet entier comparé (harness/lib/compare.sh) après :
#   1. install --no-scripts (stderr comparée à partir de la ligne d'ancrage) ;
#   2. dump-autoload -o ; 3. dump-autoload -a ; 4. install --no-dev.
# Variantes (jq sur composer.json) : ignore-duplicates, replace, merge-dev
# false, recurse false. Puis les deux cas d'exigence fusionnée absente du
# lock : vendor chaud → même code 4 et mêmes lignes que Composer ; vendor
# vierge → vivacity rend la main avec la raison (code 3 en --no-fallback,
# Composer lancerait sa mise à jour implicite) ; et `vivacity update`
# refusé avec la raison.
#
# Usage : harness/merge-plugin.sh [variante...]
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/merge-plugin"
SRC="$ROOT/fixtures/projects/merge-plugin"

VARIANTS=(
  "default~."
  "ignore-duplicates~.extra[\"merge-plugin\"][\"ignore-duplicates\"]=true"
  "replace~.extra[\"merge-plugin\"].replace=true"
  "no-merge-dev~.extra[\"merge-plugin\"][\"merge-dev\"]=false"
  "no-recurse~.extra[\"merge-plugin\"].recurse=false"
)
ONLY=("$@")

[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK"
harness_git_env "$WORK"

# Le plugin seul, à la version du lock, installé une fois : son vendor/
# amorce chaque copie de référence.
plugin_version="$(jq -r '.packages[] | select(.name=="wikimedia/composer-merge-plugin") | .version' "$SRC/composer.lock")"
seed="$WORK/seed"; rm -rf "$seed"; mkdir -p "$seed"
printf '{"require": {"wikimedia/composer-merge-plugin": "%s"}, "config": {"allow-plugins": {"wikimedia/composer-merge-plugin": true}}}\n' "$plugin_version" > "$seed/composer.json"
(cd "$seed" && composer install --no-interaction --no-ansi --quiet 2>"$WORK/seed.err") || { echo "FAIL amorce du plugin :"; tail -5 "$WORK/seed.err"; exit 1; }

stage() { # destination, filtre jq
  local dest="$1" filter="$2"
  rm -rf "$dest"; mkdir -p "$dest"
  (cd "$SRC" && tar -cf - .) | (cd "$dest" && tar -xf -)
  jq "$filter" "$SRC/composer.json" > "$dest/composer.json"
  (cd "$dest" && git init -q -b main && git add -A >/dev/null && \
    GIT_AUTHOR_NAME=vivacity GIT_AUTHOR_EMAIL=v@v GIT_AUTHOR_DATE="2026-09-10T00:00:00Z" \
    GIT_COMMITTER_NAME=vivacity GIT_COMMITTER_EMAIL=v@v GIT_COMMITTER_DATE="2026-09-10T00:00:00Z" \
    git commit -q -m fixture)
}
anchor='^(Installing dependencies from lock file|Nothing to install, update or remove|Required package|Your lock file)'
# La référence a le plugin pré-installé (une opération de moins, jamais
# sa ligne « Installing ») : ces deux lignes sont exclues de la comparaison.
tail_from_anchor() { sed -E -n "/$anchor/,\$p" "$1" | grep -v '^vivacity: \|^Package operations: \|composer-merge-plugin (v' || true; }

status=0
for spec in "${VARIANTS[@]}"; do
  IFS='~' read -r name filter <<< "$spec"
  if [ ${#ONLY[@]} -gt 0 ]; then
    keep=0; for o in "${ONLY[@]}"; do [ "$o" = "$name" ] && keep=1; done; [ $keep = 1 ] || continue
  fi
  ref="$WORK/ref-$name"; viv="$WORK/viv-$name"
  stage "$ref" "$filter"; stage "$viv" "$filter"
  cp -R "$seed/vendor" "$ref/vendor"
  # L'autoload.php de l'amorce porte le suffixe du lock de l'amorce, et
  # Composer reprend le suffixe d'un autoload.php existant avant celui du
  # lock : retiré, pour que la référence parte du content-hash du projet.
  rm -f "$ref/vendor/autoload.php"
  ok=1
  for step in install dump-o dump-a no-dev; do
    case "$step" in
      install) c_args=(install --no-scripts); v_args=(install --no-scripts --no-fallback) ;;
      dump-o)  c_args=(dump-autoload -o);     v_args=(dump-autoload -o) ;;
      dump-a)  c_args=(dump-autoload -a);     v_args=(dump-autoload -a) ;;
      no-dev)  c_args=(install --no-scripts --no-dev); v_args=(install --no-scripts --no-dev --no-fallback) ;;
    esac
    if ! (cd "$ref" && composer "${c_args[@]}" --no-interaction --no-ansi 2>"$WORK/$name.$step.composer.err" >/dev/null); then
      echo "FAIL $name/$step : composer a échoué :"; tail -5 "$WORK/$name.$step.composer.err"; ok=0; break
    fi
    if ! (cd "$ref" && git diff --quiet -- composer.lock); then
      echo "FAIL $name/$step : la référence a réécrit composer.lock (mise à jour implicite du plugin ?)"; ok=0; break
    fi
    if ! (cd "$viv" && "$VIVACITY" "${v_args[@]}" 2>"$WORK/$name.$step.vivacity.err" >/dev/null); then
      echo "FAIL $name/$step : vivacity a échoué :"; tail -5 "$WORK/$name.$step.vivacity.err"; ok=0; break
    fi
    if ! compare_vendor "$ref" "$viv" "$WORK/$name.$step.diff" >/dev/null; then
      echo "FAIL $name/$step : projet différent ($WORK/$name.$step.diff)"; head -20 "$WORK/$name.$step.diff"; ok=0; break
    fi
    if [ "$step" = install ] || [ "$step" = no-dev ]; then
      if ! diff <(tail_from_anchor "$WORK/$name.$step.composer.err") <(tail_from_anchor "$WORK/$name.$step.vivacity.err") >/dev/null; then
        echo "FAIL $name/$step : stderr différente"; diff <(tail_from_anchor "$WORK/$name.$step.composer.err") <(tail_from_anchor "$WORK/$name.$step.vivacity.err") | head -12; ok=0; break
      fi
    fi
  done
  if [ $ok = 1 ]; then echo "OK   $name : install, dump -o, dump -a, --no-dev identiques"; else status=1; fi
done

# Exigence fusionnée absente du lock.
if [ ${#ONLY[@]} -eq 0 ] || printf '%s\n' "${ONLY[@]}" | grep -qx missing; then
  patch='.require["acme/absent"]="^1.0"'
  # a. vendor chaud : Composer et vivacity refusent pareil (code 4).
  ref="$WORK/ref-missing"; viv="$WORK/viv-missing"
  stage "$ref" "."; stage "$viv" "."; cp -R "$seed/vendor" "$ref/vendor"; rm -f "$ref/vendor/autoload.php"
  (cd "$ref" && composer install --no-scripts --no-interaction --no-ansi --quiet 2>/dev/null) || true
  (cd "$viv" && "$VIVACITY" install --no-scripts --no-fallback 2>/dev/null >/dev/null) || true
  for d in "$ref" "$viv"; do jq "$patch" "$d/modules/beta/composer.json" > "$d/b.json" && mv "$d/b.json" "$d/modules/beta/composer.json"; done
  c_code=0; v_code=0
  (cd "$ref" && composer install --no-scripts --no-interaction --no-ansi 2>"$WORK/missing.composer.err" >/dev/null) || c_code=$?
  (cd "$viv" && "$VIVACITY" install --no-scripts --no-fallback 2>"$WORK/missing.vivacity.err" >/dev/null) || v_code=$?
  if [ "$c_code" = 4 ] && [ "$v_code" = 4 ] && diff <(tail_from_anchor "$WORK/missing.composer.err") <(tail_from_anchor "$WORK/missing.vivacity.err") >/dev/null; then
    echo "OK   missing (vendor chaud) : code 4 et lignes identiques"
  else
    echo "FAIL missing (vendor chaud) : composer $c_code, vivacity $v_code"; diff <(tail_from_anchor "$WORK/missing.composer.err") <(tail_from_anchor "$WORK/missing.vivacity.err") | head -10; status=1
  fi
  # b. vendor vierge : vivacity rend la main avec la raison.
  viv="$WORK/viv-missing-bare"; stage "$viv" "."
  jq "$patch" "$viv/modules/beta/composer.json" > "$viv/b.json" && mv "$viv/b.json" "$viv/modules/beta/composer.json"
  v_code=0; (cd "$viv" && "$VIVACITY" install --no-scripts --no-fallback 2>"$WORK/missing-bare.vivacity.err" >/dev/null) || v_code=$?
  if [ "$v_code" = 3 ] && grep -q "composer-merge-plugin.*implicit update" "$WORK/missing-bare.vivacity.err" && [ ! -e "$viv/vendor" ]; then
    echo "OK   missing (vendor vierge) : rendu à Composer avec la raison, rien d'écrit"
  else
    echo "FAIL missing (vendor vierge) : code $v_code"; tail -5 "$WORK/missing-bare.vivacity.err"; status=1
  fi
  # c. update refusé.
  viv="$WORK/viv-update"; stage "$viv" "."
  v_code=0; (cd "$viv" && "$VIVACITY" update --no-install 2>"$WORK/update.vivacity.err" >/dev/null) || v_code=$?
  if [ "$v_code" != 0 ] && grep -q "composer-merge-plugin" "$WORK/update.vivacity.err" && (cd "$viv" && git diff --quiet -- composer.lock); then
    echo "OK   update : refusé avec la raison, lock intact"
  else
    echo "FAIL update : code $v_code"; tail -3 "$WORK/update.vivacity.err"; status=1
  fi
fi
exit $status
