#!/usr/bin/env bash
# Harnais de dealerdirect/phpcodesniffer-composer-installer v0.7.2
# (docs/plans/chamilo-dealerdirect.md) : la recherche des `ruleset.xml`
# dans tout le projet (`$searchPaths = array($this->cwd)`), avec les règles
# du Finder de Composer 2.10.3 (symfony/finder v5.4.45 : composants à point
# écartés relativement à chaque racine de recherche, liens de répertoire
# non parcourus, VCS exclus, profondeur 0 à 3), et ce qui reste commun avec
# 1.x (`cleanInstalledPaths`, tri, lignes « set to » / « delete »).
#
# Locks résolus une fois par Composer (`--no-plugins`) puis copiés des deux
# côtés ; Composer (plugins actifs ; `--no-scripts` ne coupe que les scripts
# de composer.json) contre vivacity. À chaque étape : code de sortie, sortie
# standard + sortie d'erreur fusionnées dans l'ordre (les `  - Downloading`
# et le résumé `vivacity: …` hors comparaison) et projet entier
# (harness/lib/compare.sh, CodeSniffer.conf compris), plus une assertion sur
# la sortie de Composer — un cas qui ne montre plus ce qu'il teste échoue.
#
# Paquets : v0.7.2, phpcs 3.13.6 (celui de chamilo-lms), standards
# phpcsextra 1.5.1 / phpcsutils 1.2.3 en dev ; trois paquets `path`
# synthétiques : vend-lib/lib (bibliothèque, `ruleset.xml` à sa racine,
# copié : le cas de mpdf chez chamilo), deep-vend/deep (`a/ruleset.xml`,
# profondeur 4 depuis la racine : hors de portée par défaut), link-std/std
# (standard, lien symbolique vers `packages/std` du projet).
#
# Usage : harness/dealerdirect-07.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/dealerdirect-07"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
HOME_DIR="$(composer config --global home 2>/dev/null || true)"
mkdir -p "$WORK/home"
[ -n "$HOME_DIR" ] && [ -f "$HOME_DIR/auth.json" ] && cp "$HOME_DIR/auth.json" "$WORK/home/"
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$CACHE_DIR"
status=0; cases=0

# Les paquets `path` (hors projet, copiés ; et `packages/std` dans chaque
# projet, lié).
PKG="$WORK/pkg"
mkdir -p "$PKG/vend-lib" "$PKG/deep/a" "$WORK/template/packages/std/Std"
printf '{"name": "vend-lib/lib", "version": "1.0.0", "type": "library"}\n' > "$PKG/vend-lib/composer.json"
printf '<?xml version="1.0"?>\n<ruleset name="VendLib"/>\n' > "$PKG/vend-lib/ruleset.xml"
printf '{"name": "deep-vend/deep", "version": "1.0.0", "type": "library"}\n' > "$PKG/deep/composer.json"
printf '<?xml version="1.0"?>\n<ruleset name="Deep"/>\n' > "$PKG/deep/a/ruleset.xml"
printf '{"name": "link-std/std", "version": "1.0.0", "type": "phpcodesniffer-standard"}\n' > "$WORK/template/packages/std/composer.json"
printf '<?xml version="1.0"?>\n<ruleset name="Std"/>\n' > "$WORK/template/packages/std/Std/ruleset.xml"
mkdir -p "$WORK/outside/Std"; printf '<ruleset name="Outside"/>\n' > "$WORK/outside/Std/ruleset.xml"

REPOS="\"repositories\": [
    {\"type\": \"path\", \"url\": \"$PKG/vend-lib\", \"options\": {\"symlink\": false}},
    {\"type\": \"path\", \"url\": \"$PKG/deep\", \"options\": {\"symlink\": false}},
    {\"type\": \"path\", \"url\": \"packages/std\"}]"
ALLOW='"config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true}}'
DEV='"require-dev": {"phpcsstandards/phpcsextra": "1.5.1", "phpcsstandards/phpcsutils": "1.2.3"}'
# manifest <version du plugin> <phpcs> [paquets path…] [-- reste du JSON]
manifest() {
  local dd="$1" phpcs="$2"; shift 2; local req="" extra=""
  while [ $# -gt 0 ] && [ "$1" != "--" ]; do req="$req, \"$1\": \"1.0.0\""; shift; done
  [ "${1:-}" = "--" ] && { shift; extra=", $*"; }
  printf '{"name": "vivacity/dd07", %s,\n  "require": {"dealerdirect/phpcodesniffer-composer-installer": "%s", "squizlabs/php_codesniffer": "%s"%s},\n  %s, %s%s}\n' \
    "$REPOS" "$dd" "$phpcs" "$req" "$DEV" "$ALLOW" "$extra"
}
# lock <nom> <composer.json> : le lock écrit par Composer (sans plugins),
# avec `packages/std`, gardé sous $WORK/locks/<nom>.
lock() {
  local d="$WORK/locks/$1"
  rm -rf "$d"; mkdir -p "$d"; printf '%s\n' "$2" > "$d/composer.json"
  if grep -q '"packages/std"' "$d/composer.json"; then cp -R "$WORK/template/." "$d/"; fi
  (cd "$d" && composer update --no-install --no-plugins --no-scripts --no-audit --no-security-blocking --no-interaction --no-ansi -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; exit 1; }
}
# stage <lock> <répertoires…> : composer.json, composer.lock et packages/
# (vendor/ et le reste gardés : une étape enchaîne sur la précédente).
stage() { local src="$WORK/locks/$1"; shift; for d in "$@"; do mkdir -p "$d"; cp "$src/composer.json" "$src/composer.lock" "$d/"; if [ -d "$src/packages" ]; then cp -R "$src/packages" "$d/"; fi; done; }
fresh() { local src="$1"; shift; for d in "$@"; do rm -rf "$d"; done; stage "$src" "$@"; }
both() { local f="$1"; shift; for d in "$ref" "$viv"; do "$f" "$d" "$@"; done; }
put() { mkdir -p "$1/$(dirname "$2")"; printf '<?xml version="1.0"?>\n<ruleset name="X"/>\n' > "$1/$2"; }
conf_lost() { rm -f "$1/vendor/squizlabs/php_codesniffer/CodeSniffer.conf"; }

step() { # nom, args composer..., "--", args vivacity...
  local name="$1"; shift
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0; cases=$((cases + 1))
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >"$WORK/$name.composer.all" 2>&1) || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >"$WORK/$name.vivacity.all" 2>&1) || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.all"; status=1; return; fi
  if ! diff <(grep -vE '^  - Downloading ' "$WORK/$name.composer.all") <(grep -v '^vivacity: ' "$WORK/$name.vivacity.all") >"$WORK/$name.order.diff"; then
    echo "FAIL $name : sortie fusionnée différente"; head -12 "$WORK/$name.order.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet et sortie identiques$(grep -m1 -oE 'installed_paths (set to .*|delete)$' "$WORK/$name.composer.all" | sed 's/^/ — /' || true)"
  else
    echo "FAIL $name : projet différent"; head -12 "$WORK/$name.diff"; status=1
  fi
}
expect() { grep -qE -- "$2" "$WORK/$1.composer.all" || { echo "FAIL $1 : « $2 » absent chez Composer (cas mal construit)"; status=1; }; }
expect_not() { if grep -qE -- "$2" "$WORK/$1.composer.all"; then echo "FAIL $1 : « $2 » présent chez Composer (cas mal construit)"; status=1; fi; }
# handed <nom> <motif de la raison> <args vivacity…> : code 3 sous
# --no-fallback, la raison nommée, rien d'écrit dans vendor/ (instantané
# avant/après).
handed() {
  local name="$1" why="$2"; shift 2; local v_code=0 before after; cases=$((cases + 1))
  before="$(cd "$viv" && find . -path ./.git -prune -o -print 2>/dev/null | LC_ALL=C sort | shasum)"
  (cd "$viv" && "$VIVACITY" "$@" >"$WORK/$name.vivacity.all" 2>&1) || v_code=$?
  after="$(cd "$viv" && find . -path ./.git -prune -o -print 2>/dev/null | LC_ALL=C sort | shasum)"
  if [ "$v_code" = 3 ] && grep -qE -- "$why" "$WORK/$name.vivacity.all" && [ "$before" = "$after" ]; then
    echo "OK   $name : rendu à Composer, nommé, rien d'écrit"
  else
    echo "FAIL $name : code $v_code, raison $(grep -cE -- "$why" "$WORK/$name.vivacity.all" || true), arbre $([ "$before" = "$after" ] && echo intact || echo modifié)"; tail -4 "$WORK/$name.vivacity.all"; status=1
  fi
}
I=(install --no-scripts)
V=(install --no-scripts --no-fallback)
R=(--no-scripts --no-audit --no-security-blocking)
PHPCS='^PHP CodeSniffer Config installed_paths '
SET="${PHPCS}set to "

# --- le projet complet ----------------------------------------------------
lock main "$(manifest v0.7.2 3.13.6 vend-lib/lib deep-vend/deep link-std/std)"
ref="$WORK/main-ref"; viv="$WORK/main-viv"; fresh main "$ref" "$viv"
# vend-lib (bibliothèque, profondeur 3 depuis la racine), la source du
# standard lié sous packages/ (profondeur 3), le standard lié par sa propre
# racine (`in()` lit le lien), les standards dev ; ni deep (profondeur 4),
# ni le lien vendor/link-std/std parcouru depuis la racine.
step dd-install "${I[@]}" -- "${V[@]}"
expect dd-install "${SET}\.\./\.\./\.\./packages/std,\.\./\.\./link-std/std,\.\./\.\./phpcsstandards/phpcsextra,\.\./\.\./phpcsstandards/phpcsutils,\.\./\.\./vend-lib$"
step dd-no-op "${I[@]}" -- "${V[@]}"
expect_not dd-no-op "$PHPCS"
# Un ruleset à la racine : le projet lui-même (`../../../`, slash final de
# findShortestPath).
both put ruleset.xml
step dd-root "${I[@]}" -- "${V[@]}"
expect dd-root "${SET}\.\./\.\./\.\./,"
# Imbriqué à la borne (profondeur 3) : son répertoire parent.
both put config/phpcs/Std/ruleset.xml
step dd-nested "${I[@]}" -- "${V[@]}"
expect dd-nested "${SET}.*\.\./\.\./\.\./config/phpcs,"
# Profondeur 4 : hors de portée ; un dossier à point ; un lien de
# répertoire vers un standard hors du projet ; un dossier VCS.
both put x/b/c/d/ruleset.xml
both put .github/Std/ruleset.xml
both put tools/.cache/Std/ruleset.xml
for d in "$ref" "$viv"; do ln -s "$WORK/outside" "$d/linked"; done
both put CVS/Std/ruleset.xml
step dd-out-of-reach "${I[@]}" -- "${V[@]}"
expect_not dd-out-of-reach "$PHPCS"
# Un `ruleset.xml` lien pendant : un fichier pour le Finder (`isDir()` faux).
for d in "$ref" "$viv"; do mkdir -p "$d/dl/x"; ln -s nowhere "$d/dl/x/ruleset.xml"; done
step dd-dangling "${I[@]}" -- "${V[@]}"
expect dd-dangling "${SET}.*\.\./\.\./\.\./dl,"
# Ruleset retiré : le chemin reste (le répertoire existe toujours).
for d in "$ref" "$viv"; do rm -f "$d/ruleset.xml" "$d/config/phpcs/Std/ruleset.xml"; done
step dd-ruleset-removed "${I[@]}" -- "${V[@]}"
expect_not dd-ruleset-removed "$PHPCS"
# Le répertoire retiré : le chemin nettoyé.
for d in "$ref" "$viv"; do rm -rf "$d/dl"; done
step dd-dir-removed "${I[@]}" -- "${V[@]}"
expect dd-dir-removed "$SET"
expect_not dd-dir-removed "${SET}.*/dl[,$]"
both conf_lost
step dd-conf-lost "${I[@]}" -- "${V[@]}"
expect dd-conf-lost "$SET"
step dd-no-dev "${I[@]}" --no-dev -- "${V[@]}" --no-dev
expect dd-no-dev "$SET"
expect_not dd-no-dev "${SET}.*phpcsstandards"
step dd-re-dev "${I[@]}" -- "${V[@]}"
expect dd-re-dev "${SET}.*phpcsstandards/phpcsextra"
# `update` : POST_UPDATE_CMD, la configuration perdue avant.
both conf_lost
step dd-update update "${R[@]}" -- update "${R[@]}" --no-fallback
expect dd-update "$SET"
# `update --no-install` : POST_UPDATE_CMD sur le dépôt local.
both conf_lost
step dd-update-no-install update --no-install "${R[@]}" -- update --no-install "${R[@]}" --no-fallback
expect dd-update-no-install "$SET"
step dd-no-plugins "${I[@]}" --no-plugins -- "${V[@]}" --no-plugins
expect_not dd-no-plugins "$PHPCS"
# Retrait de vend-lib par un lock qui ne l'a plus : vendor/vend-lib vidé et
# supprimé, son chemin nettoyé.
lock nolib "$(manifest v0.7.2 3.13.6 deep-vend/deep link-std/std)"
stage nolib "$ref" "$viv"
step dd-lib-removed "${I[@]}" -- "${V[@]}"
expect dd-lib-removed "$SET"
expect_not dd-lib-removed "${SET}.*vend-lib"
# Le plugin retiré du lock : plus d'écouteur, la configuration reste.
lock noplugin '{"name": "vivacity/dd07", "require": {"squizlabs/php_codesniffer": "3.13.6"}}'
both put Std2/Std/ruleset.xml
stage noplugin "$ref" "$viv"
step dd-plugin-removed "${I[@]}" -- "${V[@]}"
expect_not dd-plugin-removed "$PHPCS"

# --- sans paquet path : les standards dev seuls, puis plus rien (`delete`) --
lock lean "{\"name\": \"vivacity/dd07-lean\",
  \"require\": {\"dealerdirect/phpcodesniffer-composer-installer\": \"v0.7.2\", \"squizlabs/php_codesniffer\": \"3.13.6\"},
  \"require-dev\": {\"phpcsstandards/phpcsutils\": \"1.2.3\"}, $ALLOW}"
ref="$WORK/lean-ref"; viv="$WORK/lean-viv"; fresh lean "$ref" "$viv"
step dd-lean "${I[@]}" -- "${V[@]}"
expect dd-lean "${SET}\.\./\.\./phpcsstandards/phpcsutils$"
step dd-delete "${I[@]}" --no-dev -- "${V[@]}" --no-dev
expect dd-delete "^PHP CodeSniffer Config installed_paths delete$"
step dd-delete-no-op "${I[@]}" --no-dev -- "${V[@]}" --no-dev
expect_not dd-delete-no-op "$PHPCS"

# --- extra.phpcodesniffer-search-depth: 4 ---------------------------------
lock depth "$(manifest v0.7.2 3.13.6 vend-lib/lib deep-vend/deep link-std/std -- '"extra": {"phpcodesniffer-search-depth": 4}')"
ref="$WORK/depth-ref"; viv="$WORK/depth-viv"; fresh depth "$ref" "$viv"
both put x/b/c/d/ruleset.xml
step dd-depth-4 "${I[@]}" -- "${V[@]}"
expect dd-depth-4 "${SET}.*\.\./\.\./\.\./x/b/c,"
expect dd-depth-4 "${SET}.*\.\./\.\./deep-vend/deep,"

# --- vendor-dir à point : la racine ne voit plus vendor ; les standards
# sont trouvés par leur propre recherche ---------------------------------
lock dotvendor "$(manifest v0.7.2 3.13.6 vend-lib/lib link-std/std | sed 's/"config": {/"config": {"vendor-dir": ".vendor", /')"
ref="$WORK/dotvendor-ref"; viv="$WORK/dotvendor-viv"; fresh dotvendor "$ref" "$viv"
step dd-dot-vendor-dir "${I[@]}" -- "${V[@]}"
expect dd-dot-vendor-dir "${SET}.*phpcsstandards/phpcsextra"
expect_not dd-dot-vendor-dir "${SET}.*vend-lib"

# --- le projet sous un répertoire à point : le filtre est relatif --------
ref="$WORK/.dotparent/main-ref"; viv="$WORK/.dotparent/main-viv"; fresh main "$ref" "$viv"
both put config/phpcs/Std/ruleset.xml
step dd-dot-parent "${I[@]}" -- "${V[@]}"
expect dd-dot-parent "${SET}.*\.\./\.\./\.\./config/phpcs,"

# --- phpcs 4 ----------------------------------------------------------------
lock phpcs4 "$(manifest v0.7.2 4.0.4 vend-lib/lib)"
ref="$WORK/phpcs4-ref"; viv="$WORK/phpcs4-viv"; fresh phpcs4 "$ref" "$viv"
step dd-phpcs4 "${I[@]}" -- "${V[@]}"
expect dd-phpcs4 "${SET}.*vend-lib"
step dd-phpcs4-no-op "${I[@]}" -- "${V[@]}"
expect_not dd-phpcs4-no-op "$PHPCS"

# --- d'une version à l'autre : le ruleset racine dit laquelle a tourné ------
lock v121 "$(manifest v1.2.1 3.13.6 vend-lib/lib)"
lock v072 "$(manifest v0.7.2 3.13.6 vend-lib/lib)"
ref="$WORK/switch-ref"; viv="$WORK/switch-viv"; fresh v072 "$ref" "$viv"
both put ruleset.xml
step dd-switch-072 "${I[@]}" -- "${V[@]}"
expect dd-switch-072 "${SET}\.\./\.\./\.\./,"
# Mis à jour vers 1.2.1 dans le même run, la configuration perdue : c'est
# le nouvel écouteur qui tourne (ni la racine, ni vend-lib).
both conf_lost; stage v121 "$ref" "$viv"
step dd-switch-to-121 "${I[@]}" -- "${V[@]}"
expect dd-switch-to-121 "${SET}\.\./\.\./phpcsstandards/phpcsextra,"
expect_not dd-switch-to-121 "${SET}.*vend-lib"
# `update --no-install` vers 0.7.2 : l'écouteur d'installed.json (1.2.1).
both conf_lost
for d in "$ref" "$viv"; do cp "$WORK/locks/v072/composer.json" "$d/composer.json"; done
step dd-switch-no-install update --no-install "${R[@]}" -- update --no-install "${R[@]}" --no-fallback
expect dd-switch-no-install "${SET}\.\./\.\./phpcsstandards/phpcsextra,"
expect_not dd-switch-no-install "${SET}.*vend-lib"
# Puis l'install du lock mis à jour : 0.7.2 rechargé, la racine revient.
step dd-switch-back "${I[@]}" -- "${V[@]}"
expect dd-switch-back "${SET}\.\./\.\./\.\./,"

# --- 1.2.1 avec une racine de type standard : le même Finder ---------------
lock root-std "$(manifest v1.2.1 3.13.6 -- '"type": "phpcodesniffer-standard"')"
ref="$WORK/root-std-ref"; viv="$WORK/root-std-viv"; fresh root-std "$ref" "$viv"
both put Std/ruleset.xml
both put .github/Std/ruleset.xml
for d in "$ref" "$viv"; do ln -s "$WORK/outside" "$d/linked"; done
step dd-121-root-standard "${I[@]}" -- "${V[@]}"
expect dd-121-root-standard "${SET}\.\./\.\./\.\./,"
expect_not dd-121-root-standard "outside|linked|github"

# --- rendus ------------------------------------------------------------------
ref="$WORK/main-ref"; viv="$WORK/main-viv"; fresh main "$ref" "$viv"
handed dd-verbose 'dealerdirect/phpcodesniffer-composer-installer.*phpcs --config-set at -v' "${V[@]}" -v
# Le bin-dir déjà dans le PATH, après un autre phpcs : 0.7.2 lit la
# configuration de cet autre phpcs (`ensureBinDirIsInPath` ne préfixe pas
# un répertoire déjà présent) — Composer échoue (« Failed to set … »).
mkdir -p "$WORK/foreign"
printf '#!/bin/sh\necho "installed_paths: /usr"\n' > "$WORK/foreign/phpcs"; chmod +x "$WORK/foreign/phpcs"
ref="$WORK/main-ref"
(cd "$ref" && composer "${I[@]}" --no-interaction --no-ansi >/dev/null 2>&1)
both conf_lost
(cd "$ref" && PATH="$WORK/foreign:$(pwd -P)/vendor/bin:$PATH" composer "${I[@]}" --no-interaction --no-ansi >"$WORK/dd-path-foreign.composer.all" 2>&1) || true
expect dd-path-foreign '^Failed to set PHP CodeSniffer installed_paths Config$'
(cd "$viv" && "$VIVACITY" "${V[@]}" >/dev/null 2>&1) || true
both conf_lost
PATH="$WORK/foreign:$(cd "$viv" && pwd -P)/vendor/bin:$PATH" handed dd-path-foreign 'bin-dir already in the PATH after .*another phpcs' "${V[@]}"
lock v112-phpcs2 "$(manifest v1.1.2 2.9.2 | sed 's/"require-dev": {[^}]*}/"require-dev": {}/')"
viv="$WORK/v112-phpcs2-viv"; fresh v112-phpcs2 "$viv"
handed dd-v112-phpcs2 'v1\.1\.2 with squizlabs/php_codesniffer 2\.9\.2 \(phpcs 2 writes' "${V[@]}"
lock v071 "$(manifest v0.7.1 3.13.6)"
viv="$WORK/v071-viv"; fresh v071 "$viv"
handed dd-v071 'dealerdirect/phpcodesniffer-composer-installer v0\.7\.1 is not emulated in this version' "${V[@]}"
lock depth-string "$(manifest v0.7.2 3.13.6 -- '"extra": {"phpcodesniffer-search-depth": "4"}')"
viv="$WORK/depth-string-viv"; fresh depth-string "$viv"
handed dd-depth-string 'extra\.phpcodesniffer-search-depth "4"' "${V[@]}"
lock phpcs2 "$(manifest v0.7.2 2.9.2 | sed 's/"require-dev": {[^}]*}/"require-dev": {}/')"
viv="$WORK/phpcs2-viv"; fresh phpcs2 "$viv"
handed dd-phpcs2 'squizlabs/php_codesniffer 2\.9\.2 \(phpcs 2 writes' "${V[@]}"

echo "dealerdirect-07 : $cases cas"
exit "$status"
