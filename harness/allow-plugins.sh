#!/usr/bin/env bash
# Harness des plugins qu'`allow-plugins` ne couvre pas, et de
# php-http/discovery à `POST_UPDATE_CMD` (docs/plans/plugin-allow-discovery.md).
#
# `Factory::createComposer` charge les plugins installés pour TOUTE commande
# (`PluginManager::loadInstalledPlugins`) et juge chacun
# (`isPluginAllowed`, non interactif) : requis par la racine et non couvert,
# `PluginBlockedException`, code 1, rien d'écrit ; plus requis, une ligne
# d'avertissement, pas chargé ; un lock antérieur à l'API 2.2.0 sans
# `allow-plugins`, `RuntimeException`. Un plugin non couvert que la
# transaction installe arrête Composer en pleine installation
# (`PluginInstaller::prepare`) : rendu à Composer. php-http/discovery
# (`Plugin::postUpdate`, 1.19.0–1.20.0) réécrit composer.json et relance un
# update quand il manque une implémentation : rendu ; sinon natif.
#
# Par cas : code de sortie ; pour un refus, le TEXTE de l'encadré de Composer
# (extrait, COLUMNS large, comme root-manifest.sh) contre les lignes de
# vivacity, stdout identique (vide, ou l'annotation GitHub Actions) et rien
# d'écrit (témoin `find -newer`) ; sinon la sortie fusionnée entière et le
# projet entier ; pour un passage de main, code 3 sous --no-fallback, la
# raison, rien d'écrit. Packagist en ligne, versions exactes, COMPOSER_HOME
# isolé.
#
# Usage : harness/allow-plugins.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/allow-plugins"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v jq >/dev/null || { echo "jq requis"; exit 1; }
composer --version --no-ansi | tail -1
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
HOME_DIR="$(composer config --global home 2>/dev/null || true)"
mkdir -p "$WORK/home"
[ -n "$HOME_DIR" ] && [ -f "$HOME_DIR/auth.json" ] && cp "$HOME_DIR/auth.json" "$WORK/home/"
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION=1.0.0
unset COMPOSER_TESTS_ARE_RUNNING GITHUB_ACTIONS
status=0
R=(--no-scripts --no-audit --no-security-blocking)
I=(install --no-scripts --no-security-blocking)

# lock <nom> <composer.json> : écrit par Composer sans plugins.
lock() {
  local d="$WORK/locks/$1"
  rm -rf "$d"; mkdir -p "$d"; printf '%s\n' "$2" > "$d/composer.json"
  (cd "$d" && composer update --no-install --no-plugins "${R[@]}" --no-interaction --no-ansi -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; exit 1; }
}
# base <nom> <lock> [filtre jq] : chaque outil installe sa copie (plugins
# autorisés par le lock), puis le filtre modifie `allow-plugins` ou les
# exigences des deux côtés — hors content-hash pour `allow-plugins`.
base() {
  local b="$WORK/base/$1" src="$WORK/locks/$2"
  rm -rf "$b"; mkdir -p "$b/ref" "$b/viv"
  for s in ref viv; do cp "$src/composer.json" "$src/composer.lock" "$b/$s/"; done
  (cd "$b/ref" && composer "${I[@]}" --no-interaction -q) || { echo "FAIL base $1 : install de Composer"; status=1; }
  (cd "$b/viv" && "$VIVACITY" "${I[@]}" -q) || { echo "FAIL base $1 : install de vivacity"; status=1; }
  if [ -n "${3:-}" ]; then
    for s in ref viv; do jq --indent 4 "$3" "$b/$s/composer.json" > "$b/$s/c.tmp" && mv "$b/$s/c.tmp" "$b/$s/composer.json"; done
  fi
}
copy() { # <base> <nom> : les deux copies du cas
  rm -rf "$WORK/$2-ref" "$WORK/$2-viv"
  cp -a "$WORK/base/$1/ref" "$WORK/$2-ref"; cp -a "$WORK/base/$1/viv" "$WORK/$2-viv"
}
box_text() {
  sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/,/^$/p' "$1" \
    | sed 's/^ *//; s/ *$//' | grep -v '^$' | grep -vE '^In [A-Za-z0-9_]*\.php line [0-9]*:$' || true
}
runboth() { # <nom> <args...> : codes dans c_code, v_code
  local name="$1"; shift
  c_code=0; v_code=0
  touch "$WORK/$name.marker"; sleep 1
  (cd "$WORK/$name-ref" && COLUMNS=400 composer "$@" --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$WORK/$name-viv" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
}
# refused <base> <nom> <motif attendu dans l'encadré> <args…>
refused() {
  local b="$1" name="$2" pat="$3"; shift 3
  copy "$b" "$name"; runboth "$name" "$@"
  box_text "$WORK/$name.composer.err" > "$WORK/$name.composer.msg"
  grep -v '^$' "$WORK/$name.vivacity.err" > "$WORK/$name.vivacity.msg" || true
  if ! grep -qE -- "$pat" "$WORK/$name.composer.msg"; then
    echo "FAIL $name : « $pat » absent de l'encadré de Composer (cas mal construit)"; status=1; return
  fi
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; head -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  if ! diff "$WORK/$name.composer.msg" "$WORK/$name.vivacity.msg" >"$WORK/$name.msg.diff"; then
    echo "FAIL $name : message différent"; head -6 "$WORK/$name.msg.diff"; status=1; return
  fi
  if ! diff -q "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >/dev/null; then
    echo "FAIL $name : stdout différente"; diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" | head -4 || true; status=1; return
  fi
  local w; w=$(find "$WORK/$name-viv" "$WORK/$name-ref" -newer "$WORK/$name.marker" -print -quit)
  if [ -n "$w" ]; then echo "FAIL $name : écrit $w"; status=1; return; fi
  echo "OK   $name : code $c_code, message identique, rien d'écrit des deux côtés"
}
# same <base> <nom> <motif attendu chez Composer ou -> <args…> : sortie
# fusionnée (stdout puis stderr séparées, comparées chacune) et projet.
same() {
  local b="$1" name="$2" pat="$3"; shift 3
  copy "$b" "$name"; runboth "$name" "$@"
  if [ "$pat" != "-" ] && ! grep -qE -- "$pat" "$WORK/$name.composer.err" "$WORK/$name.composer.out"; then
    echo "FAIL $name : « $pat » absent chez Composer (cas mal construit)"; status=1; return
  fi
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  local same_out=0
  diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.err") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.err") >"$WORK/$name.err.diff" || same_out=1
  diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >"$WORK/$name.out.diff" || same_out=1
  if [ "$same_out" != 0 ]; then
    echo "FAIL $name : sortie différente"; head -8 "$WORK/$name.err.diff" "$WORK/$name.out.diff"; status=1; return
  fi
  if compare_vendor "$WORK/$name-ref" "$WORK/$name-viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : code $c_code, sortie et projet identiques"
  else
    echo "FAIL $name : projet différent"; head -8 "$WORK/$name.diff"; status=1
  fi
}
# handed <base> <nom> <motif de raison> <args…> : vivacity seul, --no-fallback.
handed() {
  local b="$1" name="$2" pat="$3"; shift 3
  copy "$b" "$name"; local v=0
  touch "$WORK/$name.marker"; sleep 1
  (cd "$WORK/$name-viv" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.all" 2>&1) || v=$?
  if [ "$v" = 3 ] && grep -qE -- "$pat" "$WORK/$name.vivacity.all" && [ -z "$(find "$WORK/$name-viv" -newer "$WORK/$name.marker" -print -quit)" ]; then
    echo "OK   $name : rendu à Composer, la raison, rien d'écrit"
  else
    echo "FAIL $name : code $v"; tail -4 "$WORK/$name.vivacity.all"; status=1
  fi
}
# composer_acts <base> <nom> <args…> : chez Composer seul, composer.json
# réécrit (le cas montre bien ce qu'il teste).
composer_acts() {
  local b="$1" name="$2"; shift 2
  rm -rf "$WORK/$name-cref"; cp -a "$WORK/base/$b/ref" "$WORK/$name-cref"
  cp "$WORK/$name-cref/composer.json" "$WORK/$name.json.before"
  (cd "$WORK/$name-cref" && timeout 600 composer "$@" --no-interaction --no-ansi >"$WORK/$name.composer.all" 2>&1) || true
  if cmp -s "$WORK/$name.json.before" "$WORK/$name-cref/composer.json"; then
    echo "FAIL $name : Composer n'a pas réécrit composer.json (cas mal construit)"; status=1
  fi
}

BLOCKED='contains a Composer plugin which is blocked by your allow-plugins config'
WARN='^The "phpstan/extension-installer" plugin was not loaded as it is not listed in allow-plugins'

# --- phpstan/extension-installer installé, `allow-plugins` qui ne le couvre pas
lock ps '{"name": "vivacity/allow-ps", "require": {"phpstan/extension-installer": "1.4.3", "phpstan/phpstan": "2.3.1"},
  "config": {"allow-plugins": {"phpstan/extension-installer": true}}}'
base none ps 'del(.config["allow-plugins"])'
refused none none-install "$BLOCKED" "${I[@]}"
refused none none-update "$BLOCKED" update "${R[@]}"
refused none none-update-ni "$BLOCKED" update --no-install "${R[@]}"
refused none none-update-dry "$BLOCKED" update --dry-run "${R[@]}"
refused none none-require "$BLOCKED" require psr/log:3.0.2 "${R[@]}"
refused none none-require-ni "$BLOCKED" require psr/log:3.0.2 --no-install "${R[@]}"
refused none none-remove-ni "$BLOCKED" remove phpstan/phpstan --no-install "${R[@]}"
refused none none-dump "$BLOCKED" dump-autoload
refused none none-quiet "$BLOCKED" update --no-install "${R[@]}" -q
same none none-no-plugins - update --no-install "${R[@]}" --no-plugins
(export GITHUB_ACTIONS=true; refused none gha-none-install "$BLOCKED" "${I[@]}"; exit $status) || status=1
[ -s "$WORK/gha-none-install.composer.out" ] || { echo "FAIL gha-none-install : pas d'annotation chez Composer (cas mal construit)"; status=1; }
base other ps '.config["allow-plugins"] = {"other/x": true}'
refused other other-install "$BLOCKED" "${I[@]}"
base dirgone ps 'del(.config["allow-plugins"])'
rm -rf "$WORK/base/dirgone/ref/vendor/phpstan/extension-installer" "$WORK/base/dirgone/viv/vendor/phpstan/extension-installer"
refused dirgone dir-gone-install "$BLOCKED" "${I[@]}"
# Un lock antérieur à l'API de plugins 2.2.0, `allow-plugins` absent puis vide.
base oldlock ps 'del(.config["allow-plugins"])'
for s in ref viv; do jq --indent 4 '."plugin-api-version" = "2.1.0"' "$WORK/base/oldlock/$s/composer.lock" > "$WORK/base/oldlock/$s/l.tmp" && mv "$WORK/base/oldlock/$s/l.tmp" "$WORK/base/oldlock/$s/composer.lock"; done
refused oldlock old-lock-install 'generated before the allow-plugins security feature' "${I[@]}"
refused oldlock old-lock-update-ni 'generated before the allow-plugins security feature' update --no-install "${R[@]}"
for s in ref viv; do jq --indent 4 '.config["allow-plugins"] = {}' "$WORK/base/oldlock/$s/composer.json" > "$WORK/base/oldlock/$s/c.tmp" && mv "$WORK/base/oldlock/$s/c.tmp" "$WORK/base/oldlock/$s/composer.json"; done
refused oldlock old-lock-empty-install 'generated before the allow-plugins security feature' "${I[@]}"
# Requis seulement par un paquet (dealerdirect, via phpcsutils).
lock trans '{"name": "vivacity/allow-trans", "require": {"phpcsstandards/phpcsutils": "1.2.3", "squizlabs/php_codesniffer": "4.0.4",
  "dealerdirect/phpcodesniffer-composer-installer": "v1.2.1"},
  "config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true}}}'
base trans trans 'del(.config["allow-plugins"]) | del(.require["dealerdirect/phpcodesniffer-composer-installer"])'
refused trans transitive-install "$BLOCKED" "${I[@]}"
refused trans transitive-update-ni "$BLOCKED" update --no-install "${R[@]}"

# --- plus requis : l'avertissement, le plugin pas chargé
base gone ps 'del(.config["allow-plugins"]) | del(.require["phpstan/extension-installer"])'
same gone gone-install "$WARN" "${I[@]}"
same gone gone-update "$WARN" update "${R[@]}"
same gone gone-update-ni "$WARN" update --no-install "${R[@]}"
same gone gone-dump "$WARN" dump-autoload
same gone gone-require "$WARN" require psr/log:3.0.2 "${R[@]}"
same gone gone-remove-ni "$WARN" remove phpstan/phpstan --no-install "${R[@]}"
same gone gone-quiet - "${I[@]}" -q
base falsegone ps '.config["allow-plugins"]["phpstan/extension-installer"] = false | del(.require["phpstan/extension-installer"])'
same falsegone false-gone-install "$WARN" "${I[@]}"
# Requis et `false` : pas chargé, rien d'imprimé.
base falsereq ps '.config["allow-plugins"]["phpstan/extension-installer"] = false'
same falsereq false-required-install - "${I[@]}"

# --- non couverts mais silencieux : `plugin-optional` (php-http/discovery),
# composer/package-versions-deprecated.
lock discopt '{"name": "vivacity/allow-discopt", "require": {"php-http/discovery": "1.20.0", "psr/http-client": "1.0.3"},
  "config": {"allow-plugins": {"php-http/discovery": true}}}'
base discopt discopt 'del(.config["allow-plugins"])'
same discopt discovery-optional-install - "${I[@]}"
same discopt discovery-optional-update-ni - update --no-install "${R[@]}"
lock pv '{"name": "vivacity/allow-pv", "require": {"composer/package-versions-deprecated": "1.11.99.5"},
  "config": {"allow-plugins": {"composer/package-versions-deprecated": true}}}'
base pv pv 'del(.config["allow-plugins"])'
same pv package-versions-install - "${I[@]}"
same pv package-versions-dump - dump-autoload

# --- un plugin global que la configuration globale ne couvre pas
GH="$WORK/home-global"; mkdir -p "$GH"
[ -f "$WORK/home/auth.json" ] && cp "$WORK/home/auth.json" "$GH/"
printf '{"require": {"phpstan/extension-installer": "1.4.3", "phpstan/phpstan": "2.3.1"}, "config": {"allow-plugins": {"phpstan/extension-installer": true}}}\n' > "$GH/composer.json"
(cd "$GH" && COMPOSER_HOME="$GH" composer update --no-plugins "${R[@]}" --no-interaction -q) || { echo "FAIL global : install globale"; status=1; }
jq 'del(.config)' "$GH/composer.json" > "$GH/c.tmp" && mv "$GH/c.tmp" "$GH/composer.json"
lock plain '{"name": "vivacity/allow-plain", "require": {"psr/log": "3.0.2"}}'
base plain plain
(export COMPOSER_HOME="$GH"; refused plain global-install "installed globally" "${I[@]}"; exit $status) || status=1

# --- vendor vierge : la transaction installerait le plugin non couvert
# (`PluginInstaller::prepare` : Composer s'arrête en pleine installation).
mkdir -p "$WORK/base/bare"; for s in ref viv; do rm -rf "$WORK/base/bare/$s"; cp -a "$WORK/base/none/$s" "$WORK/base/bare/$s"; rm -rf "$WORK/base/bare/$s/vendor"; done
handed bare bare-install 'phpstan/extension-installer.*allow-plugins' "${I[@]}"
handed bare bare-update 'phpstan/extension-installer.*allow-plugins' update "${R[@]}"
same bare bare-update-ni - update --no-install "${R[@]}"

# --- php-http/discovery à `POST_UPDATE_CMD`
DA='"config": {"allow-plugins": {"php-http/discovery": true}}'
lock disc-dev "{\"name\": \"vivacity/allow-disc-dev\", \"type\": \"project\", \"require\": {\"php-http/discovery\": \"1.20.0\", \"psr/http-client-implementation\": \"*\"}, \"require-dev\": {\"guzzlehttp/guzzle\": \"7.10.0\"}, $DA}"
base discdev disc-dev
composer_acts discdev disc-dev-update update "${R[@]}"
handed discdev disc-dev-update 'php-http/discovery' update "${R[@]}"
lock disc-unknown "{\"name\": \"vivacity/allow-disc-unknown\", \"require\": {\"php-http/discovery\": \"1.20.0\", \"psr/http-client-implementation\": \"*\", \"php-http/mock-client\": \"1.6.1\"}, $DA}"
base discunknown disc-unknown
composer_acts discunknown disc-unknown-update update "${R[@]}"
handed discunknown disc-unknown-update 'php-http/discovery' update "${R[@]}"
handed discunknown disc-unknown-update-ni 'php-http/discovery' update --no-install "${R[@]}"
lock disc-plain "{\"name\": \"vivacity/allow-disc-plain\", \"type\": \"project\", \"require\": {\"php-http/discovery\": \"1.20.0\", \"psr/http-client-implementation\": \"*\", \"guzzlehttp/guzzle\": \"7.10.0\"}, $DA}"
base discplain disc-plain
same discplain disc-plain-update 'Nothing to modify in lock file' update "${R[@]}"
same discplain disc-plain-update-ni 'Nothing to modify in lock file' update --no-install "${R[@]}"
exit $status
