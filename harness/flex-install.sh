#!/usr/bin/env bash
# Harness v0.19 : `symfony/flex` à l'install (`POST_INSTALL_CMD`). La
# fixture fixtures/projects/flex-install (le plugin, un `symfony-pack`, une
# bibliothèque) est installée par Composer (plugin actif — `--no-scripts`
# ne coupe que les scripts de composer.json) et par vivacity ; projet
# entier ET stderr complète comparés. Ce que Flex fait à cet événement :
# copier `.env` depuis `.env.dist` sous quatre conditions, imprimer une
# ligne vide puis « Run composer recipes… » (si `symfony/flex` est une
# exigence racine), et rien d'autre (`fetchRecipes` n'est appelé qu'à
# POST_UPDATE_CMD). Un `symfony-pack` est posé comme un métapaquet : sa
# ligne d'opération n'a pas de suffixe.
# Cas : .dist seul (copie) ; .env déjà là ; .env.local là ; .dist qui
# mentionne .env.local ; `runtime.dotenv_path` ; flex hors de require
# (pas de lignes) ; --no-plugins.
#
# Usage : harness/flex-install.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/flex-install"
SRC="$ROOT/fixtures/projects/flex-install"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
status=0
stage() { # destination, filtre jq, préparation shell
  rm -rf "$1"; mkdir -p "$1"; cp "$SRC/composer.lock" "$1/"; jq "$2" "$SRC/composer.json" > "$1/composer.json"
  (cd "$1" && eval "${3:-true}")
}
run() { # nom, filtre jq, préparation, drapeaux supplémentaires
  local name="$1" filter="$2" prep="$3"; shift 3
  local ref="$WORK/ref-$name" viv="$WORK/viv-$name" c_code=0 v_code=0
  stage "$ref" "$filter" "$prep"; stage "$viv" "$filter" "$prep"
  (cd "$ref" && composer install --no-scripts "$@" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" install --no-scripts "$@" --no-fallback >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  # `  - Downloading …` : Composer l'imprime sur cache froid, vivacity jamais.
  if ! diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.err") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.err") >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -8 "$WORK/$name.err.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    local env="pas de .env"; [ -f "$viv/.env" ] && env=".env copié"
    [ -f "$viv/config/.env" ] && env="config/.env copié"
    echo "OK   $name : projet et stderr identiques ($env)"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}
D="cp $SRC/env.dist.tpl .env.dist"
run dist-only            "."  "$D"
run env-present          "."  "$D; printf 'APP_ENV=dev\n' > .env"
run local-present        "."  "$D; printf 'APP_ENV=dev\n' > .env.local"
run dist-mentions-local  "."  "cp $SRC/env.dist.tpl .env.dist; printf '# see .env.local\n' >> .env.dist"
run dotenv-path          '.extra.runtime.dotenv_path="config/.env"' "mkdir -p config && cp $SRC/env.dist.tpl config/.env.dist"
# `symfony/flex` hors de require : le téléchargeur est désactivé, les deux
# lignes disparaissent (la copie de .env, elle, a toujours lieu).
# (le lock garde flex : il est installé, mais n'est plus une exigence
# racine — les deux côtés préviennent que le lock n'est plus à jour.)
run flex-not-required    'del(.require["symfony/flex"])' "$D"
run no-plugins           "."  "$D" --no-plugins
if [ -f "$WORK/viv-no-plugins/.env" ]; then echo "FAIL no-plugins : .env copié alors que le plugin n'est pas chargé"; status=1; fi
exit $status
