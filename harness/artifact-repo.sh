#!/usr/bin/env bash
# Dépôts `artifact` : un répertoire de zips, que Composer lit sur le disque.
#
# Le `dist.url` d'un tel paquet est un **chemin de fichier**, pas une URL
# http(s) : `HttpDownloader::addJob` le confie à `RemoteFilesystem`, qui
# l'ouvre avec les flux de PHP. vivacity échouait dessus (« HTTP failure …
# builder error », trois tentatives) là où Composer installe — donc toute
# cette famille, dont les installs hors réseau et les paquets privés, partait
# en erreur. Le banc exige des caches vierges par côté, sinon la lecture
# locale n'est jamais exercée (le premier côté remplit le cache de fichiers,
# le second y lit l'archive).
#
# Les zips portent exprès des modes que les zipballs réelles n'ont pas (0600,
# 0640, 0666, 0700) : `unzip` pose le mode stocké sans l'umask, et le banc
# compare les modes, donc il tient aussi la règle de modes de l'extraction.
#
# Aucun réseau. Usage : harness/artifact-repo.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/artifact-repo"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication des zips)"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK/artifacts" "$WORK/seed"
status=0

# Deux paquets, fabriqués avec ZipArchive : un nom de fichier par paquet, tel
# que `ArtifactRepository` l'attend (<vendor>-<name>-<version>.zip), une racine
# unique à retirer, et des modes variés.
php -r '
$dir = $argv[1];
$make = function (string $file, string $root, array $entries) {
    $z = new ZipArchive();
    $z->open($file, ZipArchive::CREATE | ZipArchive::OVERWRITE);
    foreach ($entries as $path => [$content, $mode]) {
        $z->addFromString("$root/$path", $content);
        $z->setExternalAttributesName("$root/$path", ZipArchive::OPSYS_UNIX, ($mode | 0100000) << 16);
    }
    $z->close();
};
$make("$dir/acme-widget-1.0.0.zip", "acme-widget", [
    "composer.json" => [json_encode(["name" => "acme/widget", "version" => "1.0.0", "autoload" => ["psr-4" => ["Acme\\\\Widget\\\\" => "src/"]]], JSON_PRETTY_PRINT), 0644],
    "src/Widget.php" => ["<?php namespace Acme\\Widget; class Widget {}", 0600],
    "bin/run.sh" => ["#!/bin/sh\necho run\n", 0700],
    "LICENSE" => ["MIT\n", 0666],
    "docs/readme.md" => ["# widget\n", 0640],
]);
$make("$dir/acme-gadget-2.1.0.zip", "acme-gadget", [
    "composer.json" => [json_encode(["name" => "acme/gadget", "version" => "2.1.0", "require" => ["acme/widget" => "^1.0"]], JSON_PRETTY_PRINT), 0644],
    "Gadget.php" => ["<?php class Gadget {}", 0664],
]);
' "$WORK/artifacts" || { echo "FAIL : fabrication des zips"; exit 1; }

cat > "$WORK/seed/composer.json" <<JSON
{
    "repositories": [
        { "type": "artifact", "url": "$WORK/artifacts" }
    ],
    "require": {
        "acme/gadget": "^2.1"
    }
}
JSON

if ! (cd "$WORK/seed" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/seedcache" \
      composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >"$WORK/seed.log" 2>&1); then
  echo "FAIL : Composer n'a pas résolu le projet témoin"; sed -n '1,6p' "$WORK/seed.log"; exit 1
fi
# Sans cette vérification le banc pourrait comparer deux installs de rien.
if ! grep -q '"acme/widget"' "$WORK/seed/composer.lock" || ! grep -q '"acme/gadget"' "$WORK/seed/composer.lock"; then
  echo "FAIL : le lock ne contient pas les deux paquets"; exit 1
fi
if ! grep -q "$WORK/artifacts" "$WORK/seed/composer.lock"; then
  echo "FAIL : le lock ne porte pas les chemins locaux attendus"; exit 1
fi

for side in ref viv; do
  d="$WORK/$side"; mkdir -p "$d"
  cp "$WORK/seed/composer.json" "$WORK/seed/composer.lock" "$d/"
done
# Cache de fichiers vierge PAR CÔTÉ, et store vierge : sans ça la lecture
# locale n'est pas exercée du tout.
(cd "$WORK/ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-ref" COMPOSER_NO_AUDIT=1 \
  composer install --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/ref.err")
ref_code=$?
(cd "$WORK/viv" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-viv" \
  VIVACITY_CACHE_DIR="$WORK/store-viv" "$VIVACITY" install --no-scripts --no-fallback >/dev/null 2>"$WORK/viv.err")
viv_code=$?

if [ "$ref_code" != 0 ] || [ "$viv_code" != 0 ]; then
  echo "FAIL : codes de sortie $ref_code (composer) vs $viv_code (vivacity)"
  tail -4 "$WORK/viv.err"; status=1
else
  echo "OK   install : codes de sortie 0 des deux côtés, sans réseau"
fi

# La lecture locale a bien eu lieu : rien ne vient du store, rien du cache.
if grep -q "(0 from store, 0 from cache, 2 from network)" "$WORK/viv.err"; then
  echo "OK   les deux dists sont lus sur le disque (aucun cache ne les servait)"
else
  echo "FAIL : provenance inattendue — $(grep -o '([^)]*from network)' "$WORK/viv.err")"; status=1
fi

if compare_vendor "$WORK/ref" "$WORK/viv" "$WORK/vendor.diff" >/dev/null; then
  echo "OK   vendor/ identique, modes compris"
else
  echo "FAIL : vendor/ différent"; head -12 "$WORK/vendor.diff"; status=1
fi

# Le mode d'un fichier, quelle que soit la variante de `stat`. L'ordre compte :
# `stat -f` existe des deux côtés — sous GNU il affiche les infos du SYSTÈME DE
# FICHIERS et sort 0 — donc un repli « BSD d'abord, GNU ensuite » ne se
# déclenche jamais sous Linux et compare un pavé de statistiques à un mode.
if stat -c '%a' . >/dev/null 2>&1; then
  mode_of() { stat -c '%a' "$1"; }
else
  mode_of() { stat -f '%Lp' "$1"; }
fi

# Les modes stockés doivent survivre à l'install, des deux côtés.
for f in src/Widget.php:600 bin/run.sh:700 LICENSE:666 docs/readme.md:640 composer.json:644; do
  path="${f%%:*}"; want="${f##*:}"
  a=$(mode_of "$WORK/ref/vendor/acme/widget/$path")
  b=$(mode_of "$WORK/viv/vendor/acme/widget/$path")
  if [ "$a" != "$want" ]; then
    echo "FAIL : Composer pose $a sur $path, le zip stocke $want — la fixture ou la référence a changé"; status=1
  elif [ "$b" != "$want" ]; then
    echo "FAIL : vivacity pose $b sur $path au lieu de $want"; status=1
  else
    echo "OK   $path : $want des deux côtés"
  fi
done

# stderr : identique, la ligne de résumé de vivacity exceptée.
if diff "$WORK/ref.err" <(grep -v '^vivacity: ' "$WORK/viv.err") >"$WORK/err.diff"; then
  echo "OK   stderr identique (la ligne de résumé exceptée)"
else
  echo "FAIL : stderr différente"; head -10 "$WORK/err.diff"; status=1
fi
exit $status
