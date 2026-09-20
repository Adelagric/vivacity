#!/usr/bin/env bash
# Harness du résolveur : `composer update --no-install` et `vivacity update
# --no-install` sur le même instantané Packagist figé (fixtures/registry),
# composer.json intact — le lock produit doit être identique à l'octet, et
# identique au lock de référence capturé avec l'instantané (déterminisme de
# Composer lui-même, vérifié à chaque run).
#
# Le dépôt local est injecté par la configuration globale de Composer
# (COMPOSER_HOME/config.json : repositories + packagist.org: false), que les
# deux outils doivent honorer.
#
# Usage : harness/update.sh [fixture...]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/registry.sh
. "$ROOT/harness/lib/registry.sh"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
VIVACITY="$ROOT/target/release/vivacity"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/update"
FIXTURES=("$@"); [ ${#FIXTURES[@]} -eq 0 ] && FIXTURES=(laravel symfony sylius rector drupal path-repos solver-held-branch)
# Cas de mise à jour partielle : "fixture|arguments de composer update".
PARTIAL=(
  "laravel|laravel/pint"
  "laravel|laravel/framework -W"
  "symfony|doctrine/orm -w"
  "symfony|symfony/* -W"
  "sylius|symfony/console symfony/http-kernel -w"
  "sylius|sylius/sylius -W"
  "rector|phpstan/*"
  # Branches dev tenues au lock avec leur extra.branch-alias, exigées en
  # caret par d'autres paquets tenus : l'alias doit être semé à côté de
  # la base, et les stability-flags du lock rester ceux de Composer.
  "solver-held-branch|psr/log"
  "solver-held-branch|php-http/curl-client -w"
  "solver-held-branch|psr/log -W"
)
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
mkdir -p "$WORK"
harness_git_env "$WORK"
status=0
for fx in "${FIXTURES[@]}"; do
  archive="$ROOT/fixtures/registry/$fx.tar.gz"
  [ -f "$archive" ] || { echo "SKIP $fx : pas d'instantané (tools/snapshot-packagist.sh $fx)"; continue; }
  reg="$WORK/registry-$fx"; rm -rf "$reg"; mkdir -p "$reg"
  tar -C "$reg" -xzf "$archive"
  # packages.json avec l'URL absolue (Composer résout un metadata-url
  # relatif contre la racine du système de fichiers) et les politiques de
  # blocage déclarées comme sur Packagist.
  write_snapshot_packages_json "$reg"
  home="$WORK/home-$fx"; rm -rf "$home"; mkdir -p "$home"
  printf '{"repositories": %s}\n' "$(snapshot_repositories_json "$reg")" > "$home/config.json"
  root_version=""; [ "$fx" = "rector" ] && root_version="dev-main"
  # Drupal : packages.drupal.org (dépôt du projet, non figé) ne sert pour
  # drupal/core que ses avis de sécurité, et chaque avis touchant la version
  # figée rend le lock de référence insoluble par blocage (SA-CORE-2026-013
  # du 2026-09-16 sur 11.4.6) — Composer plante alors sur les avis partiels
  # de l'instantané file:// au lieu d'expliquer. Le blocage est vérifié par
  # les fixtures solver-policies ; ici on le débranche des deux côtés.
  blocking=(); [ "$fx" = "drupal" ] && blocking=(--no-security-blocking)
  for side in ref viv; do
    stage_project "$fx" "$WORK/$side-$fx"
  done
  if ! (cd "$WORK/ref-$fx" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
        composer update --no-install --no-scripts --no-plugins --no-interaction --no-audit --quiet ${blocking[@]+"${blocking[@]}"} 2>"$WORK/$fx.composer.log"); then
    echo "FAIL $fx : composer update a échoué :"; tail -5 "$WORK/$fx.composer.log"; status=1; continue
  fi
  if ! diff -q "$WORK/ref-$fx/composer.lock" "$reg/composer.lock.expected" >/dev/null; then
    echo "FAIL $fx : composer update sur l'instantané ≠ lock de référence (Composer non déterministe ?)"
    diff "$WORK/ref-$fx/composer.lock" "$reg/composer.lock.expected" | head -10 || true; status=1; continue
  fi
  if ! (cd "$WORK/viv-$fx" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
        "$VIVACITY" update --no-install ${blocking[@]+"${blocking[@]}"} 2>"$WORK/$fx.vivacity.log"); then
    echo "FAIL $fx : vivacity update a échoué :"; tail -5 "$WORK/$fx.vivacity.log"; status=1; continue
  fi
  if diff -q "$WORK/ref-$fx/composer.lock" "$WORK/viv-$fx/composer.lock" >/dev/null; then
    echo "OK   $fx : composer.lock identique ($(jq '.packages | length' "$WORK/viv-$fx/composer.lock") paquets)"
  else
    echo "FAIL $fx : composer.lock diffère"
    diff "$WORK/ref-$fx/composer.lock" "$WORK/viv-$fx/composer.lock" | head -20 || true; status=1
  fi
  # Flex actif (plan v0.16 B) : la référence tourne AVEC le plugin — son
  # filtre `PRE_POOL_CREATE` restreint le pool à `extra.symfony.require` —
  # et vivacity l'émule ; l'index est servi aux deux depuis
  # fixtures/flex/index.json. Le lock doit coïncider, et Flex ne doit
  # rien écrire d'autre (`symfony.lock` absent des deux côtés).
  if jq -e '.extra.symfony.require' "$ROOT/fixtures/projects/$fx/composer.json" >/dev/null 2>&1 \
     && jq -e '[.packages[].name] | index("symfony/flex")' "$ROOT/fixtures/projects/$fx/composer.lock" >/dev/null 2>&1; then
    flex_index_serve
    for side in ref viv; do
      d="$WORK/$side-$fx-flex"; stage_project "$fx" "$d"; seed_flex "$d" "$fx" "$FLEX_INDEX_URL" || { flex_index_stop; status=1; continue 2; }
    done
    if ! (cd "$WORK/ref-$fx-flex" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
          composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi ${blocking[@]+"${blocking[@]}"} >"$WORK/$fx.flex.composer.log" 2>&1); then
      echo "FAIL $fx (flex) : composer update a échoué :"; tail -5 "$WORK/$fx.flex.composer.log"; status=1; flex_index_stop; continue
    fi
    if ! grep -q 'Restricting packages listed in "symfony/symfony"' "$WORK/$fx.flex.composer.log"; then
      echo "FAIL $fx (flex) : Composer n'a pas restreint le pool (Flex inactif ? cas mal choisi)"; status=1; flex_index_stop; continue
    fi
    if ! (cd "$WORK/viv-$fx-flex" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
          "$VIVACITY" update --no-install --no-fallback ${blocking[@]+"${blocking[@]}"} >"$WORK/$fx.flex.vivacity.log" 2>&1); then
      echo "FAIL $fx (flex) : vivacity update a échoué :"; tail -5 "$WORK/$fx.flex.vivacity.log"; status=1; flex_index_stop; continue
    fi
    if ! grep -q 'Restricting packages listed in "symfony/symfony"' "$WORK/$fx.flex.vivacity.log"; then
      echo "FAIL $fx (flex) : vivacity n'a pas imprimé la restriction de Flex"; status=1; flex_index_stop; continue
    fi
    if [ -e "$WORK/ref-$fx-flex/symfony.lock" ] || [ -e "$WORK/viv-$fx-flex/symfony.lock" ]; then
      echo "FAIL $fx (flex) : symfony.lock écrit (ref: $([ -e "$WORK/ref-$fx-flex/symfony.lock" ] && echo oui || echo non), viv: $([ -e "$WORK/viv-$fx-flex/symfony.lock" ] && echo oui || echo non))"; status=1; flex_index_stop; continue
    fi
    if diff -q "$WORK/ref-$fx-flex/composer.lock" "$WORK/viv-$fx-flex/composer.lock" >/dev/null; then
      echo "OK   $fx (flex actif) : composer.lock identique, pool restreint des deux côtés"
    else
      echo "FAIL $fx (flex actif) : composer.lock diffère"
      diff "$WORK/ref-$fx-flex/composer.lock" "$WORK/viv-$fx-flex/composer.lock" | head -20 || true; status=1
    fi
    flex_index_stop
  fi
  # Mises à jour partielles (`update a/b [-w|-W]`) depuis le lock de la
  # fixture : les paquets hors liste restent verrouillés, la liste et ses
  # dépendances bougent selon le mode.
  for spec in "${PARTIAL[@]}"; do
    [ "${spec%%|*}" = "$fx" ] || continue
    read -r -a pargs <<< "${spec#*|}"
    for side in ref viv; do
      d="$WORK/$side-$fx-partial"; rm -rf "$d"; mkdir -p "$d"
      cp "$ROOT/fixtures/projects/$fx/composer.json" "$ROOT/fixtures/projects/$fx/composer.lock" "$d/"
    done
    if ! (cd "$WORK/ref-$fx-partial" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
          composer update "${pargs[@]}" --no-install --no-scripts --no-plugins --no-interaction --no-audit --quiet 2>"$WORK/$fx.partial.composer.log"); then
      echo "FAIL $fx update ${pargs[*]} : composer a échoué :"; tail -5 "$WORK/$fx.partial.composer.log"; status=1; continue
    fi
    if ! (cd "$WORK/viv-$fx-partial" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$home/cache" COMPOSER_ROOT_VERSION="$root_version" \
          "$VIVACITY" update "${pargs[@]}" --no-install 2>"$WORK/$fx.partial.vivacity.log"); then
      echo "FAIL $fx update ${pargs[*]} : vivacity a échoué :"; tail -5 "$WORK/$fx.partial.vivacity.log"; status=1; continue
    fi
    if diff -q "$WORK/ref-$fx-partial/composer.lock" "$WORK/viv-$fx-partial/composer.lock" >/dev/null; then
      echo "OK   $fx update ${pargs[*]} : composer.lock identique"
    else
      echo "FAIL $fx update ${pargs[*]} : composer.lock diffère"
      diff "$WORK/ref-$fx-partial/composer.lock" "$WORK/viv-$fx-partial/composer.lock" | head -20 || true; status=1
    fi
  done
done
exit $status
