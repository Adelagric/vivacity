#!/usr/bin/env bash
# Les lignes `  - Downloading <nom> (<version>)` de `FileDownloader::download`.
#
# Composer les écrit synchroniquement dans la boucle d'enregistrement de
# `downloadAndExecuteBatch` — `return $download()` exécute son `writeError`
# avant `addCopy`, et `addJob` ne tourne pas la boucle curl — donc l'ordre est
# celui de la transaction et elles forment un bloc avant les lignes
# d'opérations. Elles n'apparaissent que si le cache de fichiers ne sert pas
# l'archive : à cache chaud, aucune.
#
# Ce banc a son propre fichier parce qu'il exige ce que les autres ne peuvent
# pas donner : un cache de fichiers **froid et distinct par côté**. Les harnais
# qui comparent la stderr partagent un cache, donc le premier run le réchauffe
# et le second n'imprime plus rien — c'est ce que leurs filtres
# `^  - Downloading ` compensent. Ici chaque côté part d'un cache vierge, et la
# stderr est comparée sans filtre.
#
# `COMPOSER_ROOT_VERSION` est fixé des deux côtés : sans lui, Composer
# avertit qu'il ne détecte pas la version de la racine (projet sans git) et
# vivacity n'imprime pas cet avertissement — écart réel, consigné à part, mais
# hors du sujet de ce banc.
#
# Réseau requis (deux petites archives, deux fois).
# Usage : harness/download-lines.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/download-lines"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK/seed"
composer --version --no-ansi | tail -1
status=0

cat > "$WORK/seed/composer.json" <<'JSON'
{
    "name": "vivace/download-lines",
    "require": {
        "psr/log": "^3",
        "psr/container": "^2"
    }
}
JSON

# Le lock est produit une fois, sans install : deux paquets, deux dists.
if ! (cd "$WORK/seed" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/seedcache" \
      composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >"$WORK/seed.log" 2>&1); then
  echo "SKIP : impossible de résoudre le projet témoin (réseau ?)"; sed -n '1,4p' "$WORK/seed.log"; exit 0
fi

for side in ref viv; do
  d="$WORK/$side"; mkdir -p "$d"
  cp "$WORK/seed/composer.json" "$WORK/seed/composer.lock" "$d/"
done
# Un cache de fichiers vierge PAR CÔTÉ : c'est tout l'objet du banc.
(cd "$WORK/ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-ref" COMPOSER_NO_AUDIT=1 \
  composer install --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/ref.err")
(cd "$WORK/viv" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-viv" \
  VIVACITY_CACHE_DIR="$WORK/store-viv" "$VIVACITY" install >/dev/null 2>"$WORK/viv.err")

n_ref=$(grep -c '^  - Downloading ' "$WORK/ref.err")
n_viv=$(grep -c '^  - Downloading ' "$WORK/viv.err")
if [ "$n_ref" -lt 2 ]; then
  echo "FAIL : Composer n'a imprimé que $n_ref ligne(s) Downloading — cache pas froid, cas sans valeur"; status=1
elif [ "$n_ref" != "$n_viv" ]; then
  echo "FAIL : $n_ref lignes chez Composer, $n_viv chez vivacity"; status=1
elif ! diff <(cat "$WORK/ref.err") <(grep -v '^vivacity: ' "$WORK/viv.err") >"$WORK/err.diff"; then
  echo "FAIL : stderr différente (la ligne de résumé `vivacity:` exceptée)"; head -8 "$WORK/err.diff"; status=1
else
  echo "OK   cache froid des deux côtés : $n_ref lignes Downloading, stderr identique sans filtre"
fi

# À cache chaud, personne n'imprime : le même projet, réinstallé sur le cache
# que le run précédent a rempli.
for side in ref viv; do rm -rf "$WORK/warm-$side"; mkdir -p "$WORK/warm-$side"; cp "$WORK/seed/composer.json" "$WORK/seed/composer.lock" "$WORK/warm-$side/"; done
(cd "$WORK/warm-ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-ref" COMPOSER_NO_AUDIT=1 \
  composer install --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/warm-ref.err")
(cd "$WORK/warm-viv" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-viv" \
  VIVACITY_CACHE_DIR="$WORK/store-viv" "$VIVACITY" install >/dev/null 2>"$WORK/warm-viv.err")
w_ref=$(grep -c '^  - Downloading ' "$WORK/warm-ref.err")
w_viv=$(grep -c '^  - Downloading ' "$WORK/warm-viv.err")
if [ "$w_ref" != 0 ] || [ "$w_viv" != 0 ]; then
  echo "FAIL cache chaud : $w_ref lignes chez Composer, $w_viv chez vivacity (attendu 0/0)"; status=1
else
  echo "OK   cache chaud des deux côtés : aucune ligne Downloading de part ni d'autre"
fi
exit $status
