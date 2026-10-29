#!/usr/bin/env bash
# `update`, `require`, `remove` et `install` sans lock SANS PHP sous
# `--ignore-platform-reqs` (docs/plans/update-without-php.md) : vivacity avec
# `VIVACITY_PHP=/nonexistent` contre Composer 2.10.3 avec son PHP, le drapeau
# des deux côtés. Sous IgnoreAll, Composer saute toute règle sur un paquet
# plateforme : son PHP est la référence de vivacity sans PHP.
#
# Registre `file://` synthétique (métadonnées p2, dists zip locaux) dont les
# paquets exigent php >=99, ext-nope, lib-nope, ext-json >=999 et entrent en
# conflit avec ext-json (chargée sur la machine) : rien de tout ça n'est
# satisfait — l'oracle le vérifie, Composer échoue sans le drapeau. Comparés
# pour chaque cas : code de sortie, stdout, stderr entière (la ligne de
# résumé `vivacity: …` exceptée), et le projet entier (composer.json,
# composer.lock, vendor/, `compare_vendor`). Cache de fichiers vierge par
# côté et par cas : les lignes Downloading sont symétriques.
#
# Côté vivacity, un faux `composer` en tête du PATH : une délégation sans
# PHP ne peut pas marcher (Composer en a besoin), elle ne doit jamais avoir
# lieu ; le faux laisse une trace s'il est lancé.
#
# Refus (vivacity seul) : code, message nommé, rien d'écrit, faux composer
# jamais lancé.
#
# Composer reçoit `--no-audit` (vivacity n'audite pas après un update : écart
# connu, hors de ce chantier, comme dans les autres harnais).
#
# Un cas en ligne (Packagist) : composer/installers 1.12.0 sous PHP >= 8.4
# imprime deux lignes de dépréciation chez Composer ; sans PHP vivacity n'en
# imprime aucune (DECISIONS 2026-10-29) — la sortie doit être celle de
# Composer moins exactement ces deux lignes. `NO_PHP_OFFLINE=1` le saute.
#
# Usage : harness/no-php.sh [cas...]   (VIVACITY_BIN pour un autre binaire)
# Sous un autre PHP côté Composer : PATH=/opt/homebrew/opt/php@8.4/bin:$PATH harness/no-php.sh
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
# shellcheck source=lib/registry.sh
. "$ROOT/harness/lib/registry.sh"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/no-php"
ONLY=("$@")
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v zip >/dev/null || { echo "zip requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd -P)"
harness_git_env "$WORK"
echo "Composer : $(composer --version --no-ansi 2>/dev/null | head -1) ; PHP $(php -r 'echo PHP_VERSION;')"
php -r 'exit(extension_loaded("json") ? 0 : 1);' || { echo "FAIL : ext-json absente du PHP de Composer, le conflit ne prouve rien"; exit 1; }
status=0
selected() { [ ${#ONLY[@]} -eq 0 ] || printf '%s\n' "${ONLY[@]}" | grep -qx "$1"; }

# --- Registre -------------------------------------------------------------
REG="$WORK/registry"; mkdir -p "$REG/p2/acme" "$REG/dist"
# pkg <name> <version> <extra json> : un paquet à une classe, son zip, sa version
# dans le fichier p2.
pkg() {
  local name="$1" version="$2" extra="$3" short="${1#acme/}" src
  src="$WORK/src/$short-$version"; mkdir -p "$src/src"
  local class; class="$(printf '%s' "$short" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')"
  printf '<?php\nnamespace Acme;\nclass %s {}\n' "$class" > "$src/src/$class.php"
  jq -n --arg n "$name" --arg v "$version" --argjson x "$extra" \
    '{name: $n, version: $v, type: "library", autoload: {"psr-4": {"Acme\\": "src/"}}} + $x' > "$src/composer.json"
  (cd "$src" && zip -qrX "$REG/dist/$short-$version.zip" .)
  local f="$REG/p2/$name.json"
  [ -f "$f" ] || printf '{"packages": {"%s": []}}\n' "$name" > "$f"
  printf '{"packages": {"%s": []}}\n' "$name" > "$REG/p2/$name~dev.json"
  jq --arg n "$name" --arg u "$REG/dist/$short-$version.zip" --slurpfile p "$src/composer.json" \
    '.packages[$n] += [$p[0] + {dist: {type: "zip", url: $u, reference: null, shasum: ""}}]' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
}
PLATFORM='{"php": ">=99", "ext-nope": "*", "lib-nope": "^1", "ext-json": ">=999"}'
pkg acme/base 1.0.0 "{\"require\": $PLATFORM, \"conflict\": {\"ext-json\": \"*\", \"ext-mbstring\": \"<99\"}, \"suggest\": {\"acme/extra\": \"more\"}}"
pkg acme/base 1.1.0 "{\"require\": $PLATFORM, \"conflict\": {\"ext-json\": \"*\"}, \"suggest\": {\"acme/extra\": \"more\"}}"
pkg acme/dep 1.0.0 '{"require": {"ext-nope": "^2"}}'
pkg acme/dep 1.1.0 '{"require": {"ext-nope": "^2"}}'
pkg acme/util 1.0.0 '{"require": {"acme/dep": "^1.0", "lib-nope": "*"}}'
pkg acme/util 1.1.0 '{"require": {"acme/dep": "^1.1", "php-64bit": ">=99"}}'
pkg acme/extra 1.0.0 '{"require": {"hhvm": "*"}}'
pkg acme/devtool 1.0.0 '{"require": {"ext-nope": "*", "php": "<5"}}'
pkg acme/replacer 1.0.0 '{"replace": {"ext-foo": "*"}}'
pkg acme/suggester 1.0.0 '{"suggest": {"ext-intl": "for locales"}}'
write_snapshot_packages_json "$REG"
HOME_DIR="$WORK/home"; mkdir -p "$HOME_DIR"
printf '{"repositories": %s}\n' "$(snapshot_repositories_json "$REG")" > "$HOME_DIR/config.json"
FAKEBIN="$WORK/fakebin"; mkdir -p "$FAKEBIN"
printf '#!/bin/sh\ntouch "%s/composer-ran"\nexit 0\n' "$WORK" > "$FAKEBIN/composer"; chmod +x "$FAKEBIN/composer"

TEMPLATE='{"name": "acme/app", "require": {"acme/base": "^1.0", "acme/util": "^1.0"}, "require-dev": {"acme/devtool": "^1.0"}}'

# Lock et vendor de départ : Composer, versions épinglées à 1.0.0, puis les
# contraintes rouvertes (le lock reste le point de départ des deux côtés).
SEED="$WORK/seed"; mkdir -p "$SEED"
jq -n '{"name": "acme/app", "require": {"acme/base": "1.0.0", "acme/util": "1.0.0"}, "require-dev": {"acme/devtool": "^1.0"}}' > "$SEED/composer.json"
if ! (cd "$SEED" && COMPOSER_HOME="$HOME_DIR" COMPOSER_CACHE_DIR="$WORK/cache-seed" COMPOSER_ROOT_VERSION=1.0.0 \
      composer update --ignore-platform-reqs --no-interaction --no-ansi >"$WORK/seed.log" 2>&1); then
  echo "FAIL : Composer n'a pas posé le projet de départ"; tail -5 "$WORK/seed.log"; exit 1
fi
jq '.require["acme/base"] = "^1.0" | .require["acme/util"] = "^1.0"' "$SEED/composer.json" > "$SEED/c.tmp" && mv "$SEED/c.tmp" "$SEED/composer.json"

# Oracle non aveugle : sans le drapeau, Composer refuse ce projet.
mkdir -p "$WORK/oracle"; printf '%s\n' "$TEMPLATE" > "$WORK/oracle/composer.json"
if (cd "$WORK/oracle" && COMPOSER_HOME="$HOME_DIR" COMPOSER_CACHE_DIR="$WORK/cache-oracle" COMPOSER_ROOT_VERSION=1.0.0 \
    composer update --no-install --no-interaction --no-ansi >"$WORK/oracle.log" 2>&1) \
   || ! grep -q 'require php >=99' "$WORK/oracle.log"; then
  echo "FAIL : sans --ignore-platform-reqs Composer devait refuser (php >=99, ext-nope, lib-nope, conflit ext-json)"; tail -5 "$WORK/oracle.log"; exit 1
fi

# stage <dir> <kind> : `nolock` (le gabarit seul) ou `seeded` (lock + vendor).
stage() {
  local d="$1" kind="$2"; rm -rf "$d"; mkdir -p "$d"
  case "$kind" in
    nolock) printf '%s\n' "$TEMPLATE" > "$d/composer.json" ;;
    seeded) cp "$SEED/composer.json" "$SEED/composer.lock" "$d/"; cp -R "$SEED/vendor" "$d/vendor" ;;
    *) echo "stage inconnu : $kind"; exit 1 ;;
  esac
}

# La stderr de vivacity sans ses lignes propres, sans équivalent chez
# Composer (tolérées comme dans harness/custom-installers.sh) : le résumé
# `vivacity: …` et les notes d'émulation.
vivacity_lines() {
  grep -v -e '^vivacity: ' -e '^Note: plugin ' -e '^Note: composer/installers .* emulated natively' "$1"
}

# pair <name> <kind> <jq sur composer.json ou '.'> <env NAME=V ou '-'> -- <arguments...>
# Composer et vivacity sur deux copies ; `--ignore-platform-reqs` est dans les
# arguments du cas (le cas de la variable d'environnement le remplace).
pair() {
  local name="$1" kind="$2" edit="$3" envset="$4"; shift 5
  selected "$name" || return 0
  local ref="$WORK/ref-$name" viv="$WORK/viv-$name" envs=()
  for d in "$ref" "$viv"; do
    stage "$d" "$kind"
    [ "$edit" = "." ] || { jq "$edit" "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json"; }
  done
  [ "$envset" = "-" ] || envs=("$envset")
  rm -f "$WORK/composer-ran"
  # `install` n'audite que sur `--audit` (et refuse `--no-audit`).
  local audit=(--no-audit); [ "$1" = install ] && audit=()
  local c_code=0 v_code=0
  (cd "$ref" && env ${envs[@]+"${envs[@]}"} COMPOSER_HOME="$HOME_DIR" COMPOSER_CACHE_DIR="$WORK/cache-ref-$name" COMPOSER_ROOT_VERSION=1.0.0 \
    composer "$@" ${audit[@]+"${audit[@]}"} --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  # `VIV_NO_PHP_ON_PATH=1` : ni VIVACITY_PHP ni php dans le PATH (le faux
  # composer seul) — l'embarqueur sans PHP du tout ; sinon VIVACITY_PHP
  # pointe dans le vide et le php de la machine reste dans le PATH.
  local php_env=(VIVACITY_PHP=/nonexistent PATH="$FAKEBIN:$PATH")
  [ "${VIV_NO_PHP_ON_PATH:-0}" = 1 ] && php_env=(-u VIVACITY_PHP PATH="$FAKEBIN")
  (cd "$viv" && env ${envs[@]+"${envs[@]}"} "${php_env[@]}" VIVACITY_NO_PLATFORM_CACHE=1 \
    COMPOSER_HOME="$HOME_DIR" COMPOSER_CACHE_DIR="$WORK/cache-viv-$name" VIVACITY_CACHE_DIR="$WORK/store-viv-$name" COMPOSER_ROOT_VERSION=1.0.0 \
    "$VIVACITY" "$@" --no-interaction --no-ansi >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
  vivacity_lines "$WORK/$name.vivacity.err" > "$WORK/$name.vivacity.err2"
  local ok=1
  if [ -e "$WORK/composer-ran" ]; then echo "FAIL $name : vivacity a lancé composer"; ok=0; fi
  if [ "$c_code" != "$v_code" ]; then
    echo "FAIL $name : code composer=$c_code vivacity=$v_code"; tail -n 4 "$WORK/$name.composer.err" "$WORK/$name.vivacity.err"; ok=0
  fi
  if ! diff -q "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >/dev/null; then
    echo "FAIL $name : stdout diffère"; diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" | head -10; ok=0
  fi
  if ! diff -q "$WORK/$name.composer.err" "$WORK/$name.vivacity.err2" >/dev/null; then
    echo "FAIL $name : stderr diffère"; diff "$WORK/$name.composer.err" "$WORK/$name.vivacity.err2" | head -20; ok=0
  fi
  if ! compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "FAIL $name : projet différent"; head -20 "$WORK/$name.diff"; ok=0
  fi
  if [ "$ok" = 1 ]; then
    echo "OK   $name : code $c_code, stdout, stderr ($(wc -l < "$WORK/$name.composer.err" | tr -d ' ') lignes) et projet identiques"
  else
    status=1
  fi
}

# refused <name> <kind> <jq|.> <code attendu> <motif attendu> -- <arguments vivacity...>
refused() {
  local name="$1" kind="$2" edit="$3" want="$4" pattern="$5"; shift 6
  selected "$name" || return 0
  local viv="$WORK/viv-$name"
  stage "$viv" "$kind"
  [ "$edit" = "." ] || { jq "$edit" "$viv/composer.json" > "$viv/c.tmp" && mv "$viv/c.tmp" "$viv/composer.json"; }
  local before; before="$(cd "$viv" && find . -type f -print0 | sort -z | xargs -0 shasum | shasum)"
  rm -f "$WORK/composer-ran"
  local code=0
  (cd "$viv" && PATH="$FAKEBIN:$PATH" VIVACITY_PHP=/nonexistent VIVACITY_NO_PLATFORM_CACHE=1 \
    COMPOSER_HOME="$HOME_DIR" COMPOSER_CACHE_DIR="$WORK/cache-viv-$name" VIVACITY_CACHE_DIR="$WORK/store-viv-$name" COMPOSER_ROOT_VERSION=1.0.0 \
    "$VIVACITY" "$@" --no-interaction --no-ansi >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || code=$?
  local after; after="$(cd "$viv" && find . -type f -print0 | sort -z | xargs -0 shasum | shasum)"
  if [ "$code" != "$want" ]; then
    echo "FAIL $name : code $code, attendu $want"; tail -4 "$WORK/$name.vivacity.err"; status=1
  elif ! grep -qF -- "$pattern" "$WORK/$name.vivacity.err"; then
    echo "FAIL $name : message sans « $pattern »"; tail -4 "$WORK/$name.vivacity.err"; status=1
  elif ! printf '%s\n' "$@" | grep -qx -- --no-fallback && ! grep -qF 'VIVACITY_PHP' "$WORK/$name.vivacity.err"; then
    echo "FAIL $name : le message ne nomme pas VIVACITY_PHP"; tail -4 "$WORK/$name.vivacity.err"; status=1
  elif [ "$before" != "$after" ]; then
    echo "FAIL $name : le projet a changé"; status=1
  elif [ -e "$WORK/composer-ran" ]; then
    echo "FAIL $name : vivacity a lancé composer"; status=1
  else
    echo "OK   $name : code $code, « $pattern », rien d'écrit, composer jamais lancé"
  fi
}

I=--ignore-platform-reqs
# --- Parité -----------------------------------------------------------------
pair update-nolock      nolock . - -- update $I
pair update-locked      seeded . - -- update $I
pair partial            seeded . - -- update acme/util $I
pair partial-w          seeded . - -- update acme/util -w $I
pair partial-W          seeded . - -- update acme/util -W $I
pair with               seeded . - -- update --with acme/dep:1.1.0 acme/dep $I
pair no-install         nolock . - -- update --no-install $I
pair dry-run            seeded . - -- update --dry-run $I
pair no-dev             seeded . - -- update --no-dev $I
pair require            seeded . - -- require acme/extra $I
pair require-constraint seeded . - -- require acme/extra:^1.0 $I
pair require-dry-run    seeded . - -- require acme/extra --dry-run $I
pair require-no-update  seeded . - -- require acme/extra --no-update $I
pair remove             seeded . - -- remove acme/util $I
pair remove-dev         seeded . - -- remove --dev acme/devtool $I
pair install-nolock     nolock . - -- install $I
pair platform-config    nolock '.config.platform = {"php": "7.0.0", "ext-json": "1.0.0", "ext-mbstring": false, "lib-nope": "1.2.3"}' - -- update $I
pair env-flag           nolock . COMPOSER_IGNORE_PLATFORM_REQS=1 -- update
pair unsolvable         nolock '.require["acme/base"] = "^9"' - -- update $I
# (Sans --no-update, la liste d'update `composer-plugin-api` fait écrire à
# PoolBuilder « Pattern … matches platform packages » AVANT « Updating
# dependencies », vivacity après — écart antérieur, avec PHP aussi, hors de
# ce chantier.)
pair require-composer-api seeded . - -- require composer-plugin-api --no-update $I
# Une suggestion d'extension que `config.platform` rend connue (versionnée).
pair suggests-known     nolock '.require["acme/suggester"] = "*" | .config.platform = {"ext-intl": "1.0.0"}' - -- update $I
# `remove --no-update` ne résout rien : sans PHP ni drapeau, comme Composer.
pair remove-no-update   seeded . - -- remove acme/util --no-update
# Ni VIVACITY_PHP ni php dans le PATH.
VIV_NO_PHP_ON_PATH=1 pair no-php-on-path nolock . - -- update $I

# --- Refus ----------------------------------------------------------------
refused no-flag          nolock . 1 "--ignore-platform-reqs" -- update
refused partial-list     nolock . 1 "--ignore-platform-reqs" -- update --ignore-platform-req=ext-nope --ignore-platform-req=php
refused platform-php     nolock '.config.platform = {"php": "8.4.0"}' 1 "--ignore-platform-reqs" -- update
refused remove-no-flag   seeded . 1 "--ignore-platform-reqs" -- remove acme/util
refused require-ext      seeded . 3 "ext-foo without a version constraint" -- require ext-foo $I
refused require-php      seeded . 3 "php without a version constraint" -- require php --no-update $I
refused replacer         nolock '.require["acme/replacer"] = "*"' 3 "acme/replacer replaces ext-foo" -- update $I
refused suggests-ext     nolock '.require["acme/suggester"] = "*"' 3 "acme/suggester suggests ext-intl" -- update $I
refused with-platform    seeded '.require["ext-json"] = "*"' 3 "temporary constraint on ext-json" -- update --with ext-json:^99 $I
refused verbose          nolock . 3 "-v without PHP" -- update -v $I
refused very-verbose     nolock . 3 "No PHP to run Composer with" -- update -vv $I
refused verbose-nf       nolock . 3 "-v without PHP" -- update -v --no-fallback $I
refused no-fallback      nolock '.require["acme/replacer"] = "*"' 3 "acme/replacer replaces ext-foo" -- update --no-fallback $I

# --- composer/installers 1.x (Packagist en ligne) ---------------------------
if selected installers-deprecation && [ "${NO_PHP_OFFLINE:-0}" != 1 ]; then
  name=installers-deprecation; ref="$WORK/ref-$name"; viv="$WORK/viv-$name"
  for d in "$ref" "$viv"; do
    rm -rf "$d"; mkdir -p "$d"
    printf '{"name": "acme/app", "require": {"composer/installers": "1.12.0"}, "config": {"allow-plugins": {"composer/installers": true}}}\n' > "$d/composer.json"
  done
  mkdir -p "$WORK/home-online"
  c_code=0; v_code=0
  (cd "$ref" && COMPOSER_HOME="$WORK/home-online" COMPOSER_CACHE_DIR="$WORK/cache-ref-$name" COMPOSER_ROOT_VERSION=1.0.0 \
    composer update $I --no-audit --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && PATH="$FAKEBIN:$PATH" VIVACITY_PHP=/nonexistent VIVACITY_NO_PLATFORM_CACHE=1 \
    COMPOSER_HOME="$WORK/home-online" COMPOSER_CACHE_DIR="$WORK/cache-viv-$name" VIVACITY_CACHE_DIR="$WORK/store-viv-$name" COMPOSER_ROOT_VERSION=1.0.0 \
    "$VIVACITY" update $I --no-interaction --no-ansi >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
  php_minor="$(php -r 'echo PHP_MAJOR_VERSION * 100 + PHP_MINOR_VERSION;')"
  n_dep="$(grep -c '^Deprecation Notice: Composer\\Installers\\Installer::__construct' "$WORK/$name.composer.err")"
  # Ce que vivacity doit imprimer : Composer moins la dépréciation et sa ligne
  # « More deprecation notices… », exactement.
  grep -v -e '^Deprecation Notice: Composer\\Installers\\Installer::__construct' \
          -e '^More deprecation notices were hidden, run again with `-v` to show them\.$' \
    "$WORK/$name.composer.err" > "$WORK/$name.composer.err2"
  vivacity_lines "$WORK/$name.vivacity.err" > "$WORK/$name.vivacity.err2"
  if [ "$c_code" != 0 ]; then
    echo "FAIL $name : composer sort en $c_code (réseau ?)"; tail -4 "$WORK/$name.composer.err"; status=1
  elif [ "$php_minor" -ge 804 ] && [ "$n_dep" != 1 ]; then
    echo "FAIL $name : PHP >= 8.4 et pas de dépréciation chez Composer (cas mal choisi)"; status=1
  elif [ "$php_minor" -lt 804 ] && [ "$n_dep" != 0 ]; then
    echo "FAIL $name : dépréciation chez Composer sous PHP < 8.4"; status=1
  elif [ "$v_code" != 0 ]; then
    echo "FAIL $name : vivacity sort en $v_code"; tail -4 "$WORK/$name.vivacity.err"; status=1
  elif ! diff -q "$WORK/$name.composer.err2" "$WORK/$name.vivacity.err2" >/dev/null \
       || ! diff -q "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >/dev/null; then
    echo "FAIL $name : sorties différentes (dépréciation retirée)"; diff "$WORK/$name.composer.err2" "$WORK/$name.vivacity.err2" | head -10; status=1
  elif ! compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  elif grep -q '^Deprecation Notice' "$WORK/$name.vivacity.err"; then
    echo "FAIL $name : vivacity imprime une dépréciation sans PHP"; status=1
  else
    echo "OK   $name : sans PHP, sortie de Composer (PHP $php_minor) moins ses $((n_dep * 2)) lignes de dépréciation ; projet identique"
  fi
fi

exit $status
