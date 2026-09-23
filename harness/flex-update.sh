#!/usr/bin/env bash
# Harness v0.19 (tranche B) : `update` AVEC install sur un projet Flex.
# Flex applique ses recettes à POST_UPDATE_CMD d'après les opérations que
# `recordOperations` reconstruit depuis `symfony.lock` — une opération
# d'install par paquet du lock résolu absent de `symfony.lock`. vivacity
# n'émule que le cas où l'install ne pose rien de neuf : tout est alors
# déjà extrait à la bonne version, donc « une recette s'applique-t-elle ? »
# se décide avant d'écrire. Les deux côtés partent d'un vendor/ complet et
# d'un `symfony.lock` fabriqué ici, l'index est servi en local avec de
# vraies entrées `recipes`.
# Cas : tout dans symfony.lock (rien d'enregistré) ; un paquet sans recette
# ni bundle manquant (natif) ; symfony/console manquant (recette dans
# l'index → repli) ; symfony/twig-bundle manquant (pas de recette dans cet
# index, mais une classe de bundle → repli) ; un paquet retiré de vendor/
# (l'install poserait quelque chose → repli).
#
# Usage : harness/flex-update.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
# shellcheck source=lib/registry.sh
. "$ROOT/harness/lib/registry.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/flex-update"
FX=symfony
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
# L'instantané Packagist figé : sans lui les deux outils résoudraient en
# direct, les versions bougeraient et la transaction ne serait jamais vide.
archive="$ROOT/fixtures/registry/$FX.tar.gz"
[ -f "$archive" ] || { echo "SKIP : pas d'instantané pour $FX"; exit 0; }
reg="$WORK/registry"; mkdir -p "$reg"; tar -C "$reg" -xzf "$archive"
write_snapshot_packages_json "$reg"
home="$WORK/home"; mkdir -p "$home"
printf '{"repositories": %s}\n' "$(snapshot_repositories_json "$reg")" > "$home/config.json"
status=0
flex_index_serve index-recipes.json
trap 'flex_index_stop' EXIT
root_version="${COMPOSER_ROOT_VERSION:-dev-main}"

# Un `symfony.lock` couvrant tous les paquets sauf ceux passés en argument.
write_symfony_lock() { # projet, paquets à omettre...
  local d="$1"; shift
  local omit="$*"
  php -r '
    $lock = json_decode(file_get_contents($argv[1] . "/composer.lock"), true);
    $omit = array_filter(explode(" ", $argv[2]));
    $out = [];
    foreach (array_merge($lock["packages"], $lock["packages-dev"] ?? []) as $p) {
      if (in_array($p["name"], $omit, true)) { continue; }
      $out[$p["name"]] = ["version" => $p["version"]];
    }
    ksort($out);
    file_put_contents($argv[1] . "/symfony.lock", json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
  ' "$d" "$omit"
}

# Les deux côtés partent du même état convergé : un `composer update` de
# préparation (l'instantané est figé, donc le lock qui en sort est stable),
# puis le `symfony.lock` fabriqué ici. Le `update` mesuré ensuite ne change
# donc rien — c'est le cas que la tranche B couvre.
prepare() { # nom, paquets à omettre de symfony.lock
  local name="$1"; shift
  for side in ref viv; do
    local d="$WORK/$side-$name"
    stage_project "$FX" "$d"
    seed_flex "$d" "$FX" "$FLEX_INDEX_URL" || return 1
    # Un symfony.lock complet AVANT la convergence : aucune opération
    # n'est enregistrée, donc Flex ne télécharge aucune recette.
    cp "$ROOT/fixtures/projects/$FX/composer.lock" "$d/composer.lock"
    write_symfony_lock "$d"
    (cd "$d" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
      composer update --no-scripts --no-interaction --no-audit --no-ansi --no-blocking --quiet >"$WORK/$name.$side.prepare.log" 2>&1) || {
        echo "FAIL $name : la préparation a échoué ($side)"; tail -3 "$WORK/$name.$side.prepare.log"; status=1; return 1; }
    write_symfony_lock "$d" "$@"
  done
}

run() { # nom, attendu (native|fallback), motif attendu
  local name="$1" expect="$2" reason="${3:-}"
  local ref="$WORK/ref-$name" viv="$WORK/viv-$name" c=0 v=0
  cp "$viv/composer.lock" "$WORK/$name.lock.before"; cp "$viv/symfony.lock" "$WORK/$name.symfony.before"
  # Sur un cas de repli, Composer appliquerait la recette (réseau) : seule
  # la décision de vivacity est mesurée, et elle doit ne rien écrire.
  if [ "$expect" = native ]; then
    (cd "$ref" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
      composer update --no-scripts --no-interaction --no-audit --no-ansi --no-blocking >/dev/null 2>"$WORK/$name.composer.err") || c=$?
  fi
  (cd "$viv" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
    "$VIVACITY" update --no-fallback --no-blocking >/dev/null 2>"$WORK/$name.vivacity.err") || v=$?
  if [ "$expect" = fallback ]; then
    # Rien ne doit avoir bougé : ni le lock, ni symfony.lock.
    if ! cmp -s "$viv/composer.lock" "$WORK/$name.lock.before" || ! cmp -s "$viv/symfony.lock" "$WORK/$name.symfony.before"; then
      echo "FAIL $name : quelque chose a été écrit avant le repli"; status=1; return
    fi
    if [ "$v" = 3 ] && grep -q "$reason" "$WORK/$name.vivacity.err"; then
      echo "OK   $name : rendu à Composer ($reason)"
    else
      echo "FAIL $name : attendu un repli sur « $reason », code $v"; tail -3 "$WORK/$name.vivacity.err"; status=1
    fi
    return
  fi
  if [ "$c" != "$v" ]; then echo "FAIL $name : codes $c vs $v"; tail -4 "$WORK/$name.vivacity.err"; status=1; return; fi
  for f in composer.lock symfony.lock; do
    if ! cmp -s "$ref/$f" "$viv/$f"; then echo "FAIL $name : $f diffère"; diff "$ref/$f" "$viv/$f" | head -6; status=1; return; fi
  done
  # `Warning: Accessing 127.0.0.1 over http…` : artefact du banc (l'index
  # est servi en http local avec `secure-http: false`), pas du produit —
  # les endpoints réels sont en https. `  - Downloading …` : cache froid.
  if ! diff <(grep -vE '^  - Downloading |^Warning: Accessing 127\.0\.0\.1 over http ' "$WORK/$name.composer.err") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.err") >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -8 "$WORK/$name.err.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet, composer.lock, symfony.lock et stderr identiques"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}

prepare all-locked        && run all-locked native
prepare no-recipe         symfony/polyfill-ctype && run no-recipe native
prepare recipe-in-index   symfony/console        && run recipe-in-index   fallback "would apply the recipe of symfony/console"
prepare bundle-class      symfony/twig-bundle    && run bundle-class      fallback "would register symfony/twig-bundle's bundle"
prepare missing-package   && rm -rf "$WORK/viv-missing-package/vendor/psr/log" && run missing-package fallback "lays out nothing new"
exit $status
