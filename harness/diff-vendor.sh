#!/usr/bin/env bash
# Harness différentiel M2/M4 : pour chaque fixture, compare le vendor/ produit
# par `composer install --no-plugins --no-scripts [--no-autoloader]` et celui
# produit par `vivacity install [--no-autoloader]` sur des copies nues.
#
# La comparaison (écarts tolérés, inventaire des modes et des liens) est
# dans harness/lib/compare.sh, partagée avec harness/corpus.sh.
#
# Usage : harness/diff-vendor.sh [--with-autoloader] [fixture...]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="$ROOT/target/release/vivacity"
# Windows (Git Bash) : l'artefact cargo est vivacity.exe — le préférer quand
# il existe, pour qu'un binaire unixy résiduel ne le masque pas. Gardé par
# uname : sous WSL, l'interop binfmt rend un .exe « exécutable » aussi.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}"
AUTOLOAD_FLAG="--no-autoloader"
if [ "${1:-}" = "--with-autoloader" ]; then AUTOLOAD_FLAG=""; shift; fi
FIXTURES=("$@"); [ ${#FIXTURES[@]} -eq 0 ] && FIXTURES=(laravel symfony sylius rector wordpress drupal)

[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK"
harness_git_env "$WORK"
status=0
for fx in "${FIXTURES[@]}"; do
  src="$ROOT/fixtures/work/$fx"
  ref="$WORK/ref-$fx"; viv="$WORK/viv-$fx"
  rm -rf "$ref" "$viv"; mkdir -p "$ref" "$viv"
  # Projet complet sans les sorties d'un install (vendor/, node_modules, var,
  # et ce que les plugins émulés produisent : web/, wp-content/, recipes/, le
  # .editorconfig/.gitattributes scaffoldés) — sinon les fichiers copiés puis
  # committés par le harness seraient « trackés et inchangés » des deux côtés
  # et le scaffold n'aurait rien à prouver. Les règles d'autoload de la racine
  # (src/Kernel.php, app/…) restent présentes.
  (cd "$src" && tar --exclude=./vendor --exclude=./node_modules --exclude=./var --exclude=./web --exclude=./wp-content --exclude=./recipes --exclude=./.editorconfig --exclude=./.gitattributes -cf - .) | (cd "$ref" && tar -xf -)
  (cd "$src" && tar --exclude=./vendor --exclude=./node_modules --exclude=./var --exclude=./web --exclude=./wp-content --exclude=./recipes --exclude=./.editorconfig --exclude=./.gitattributes -cf - .) | (cd "$viv" && tar -xf -)
  # Dépôt git identique des deux côtés (même arbre, même auteur/date → même SHA) :
  # Composer devine la version racine depuis git, vivacity doit faire pareil.
  for d in "$ref" "$viv"; do
    (cd "$d" && git init -q -b main && git add -A >/dev/null && \
      GIT_AUTHOR_NAME=vivacity GIT_AUTHOR_EMAIL=v@v GIT_AUTHOR_DATE="2026-09-10T00:00:00Z" \
      GIT_COMMITTER_NAME=vivacity GIT_COMMITTER_EMAIL=v@v GIT_COMMITTER_DATE="2026-09-10T00:00:00Z" \
      git commit -q -m fixture)
  done
  # Fixture à plugin de layout émulé (composer/installers autorisé) : la
  # référence tourne AVEC le plugin et la comparaison couvre le projet entier,
  # puisque des paquets vivent hors vendor/.
  if jq -e '.config["allow-plugins"]["composer/installers"] == true' "$src/composer.json" >/dev/null 2>&1; then
    plugin_flag=""; scope_ref="$ref"; scope_viv="$viv"; what="projet"
  else
    plugin_flag="--no-plugins"; scope_ref="$ref/vendor"; scope_viv="$viv/vendor"; what="vendor/"
  fi
  (cd "$ref" && composer install --no-interaction $plugin_flag --no-scripts $AUTOLOAD_FLAG --quiet)
  # Même régime de plugins des deux côtés : la référence tourne --no-plugins
  # sauf pour les fixtures à composer/installers ; vivacity aussi.
  if ! (cd "$viv" && "$VIVACITY" install $AUTOLOAD_FLAG $plugin_flag --offline 2>"$WORK/$fx.vivacity.log"); then
    echo "FAIL $fx : vivacity install a échoué :"; tail -20 "$WORK/$fx.vivacity.log"; status=1; continue
  fi
  if compare_vendor "$scope_ref" "$scope_viv" "$WORK/$fx.diff"; then
    echo "OK   $fx : $what identique"
  else
    echo "FAIL $fx : $what diffère ($(wc -l < "$WORK/$fx.diff" | tr -d ' ') lignes, $WORK/$fx.diff)"
    status=1
  fi
done
exit $status
