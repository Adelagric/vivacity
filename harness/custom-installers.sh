#!/usr/bin/env bash
# Installeurs personnalisés (docs/plans/custom-installers.md) : les plugins
# qui enregistrent un installeur décidant où un type de paquet est posé.
# Par plugin, un projet qui exige le vrai plugin à sa version exacte
# (Packagist) et des paquets synthétiques de ses types, servis par un dépôt
# `artifact` (zips fabriqués par ZipArchive, sans réseau) ; le lock est
# écrit par Composer `--no-plugins`.
#
# Étapes, chacune contre Composer 2.10.3 (plugins actifs) : install sur
# vendor vierge, no-op, mise à jour d'un paquet placé (second lock), retrait
# d'un paquet placé (troisième lock), `--no-plugins` ; par étape : code de
# sortie, stdout, stderr (les lignes `Downloading` et les notes `vivacity:`
# mises à part) et le projet entier (fichiers, modes, liens, installed.json,
# installed.php, autoloader — harness/lib/compare.sh). Les configurations
# que vivacity rend à Composer : code 3 sous --no-fallback, la raison, rien
# d'écrit.
#
# Usage : harness/custom-installers.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/custom-installers"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication des zips)"; exit 1; }
command -v jq >/dev/null || { echo "jq requis"; exit 1; }
composer --version --no-ansi | tail -1
rm -rf "$WORK"; mkdir -p "$WORK/artifacts" "$WORK/home"; harness_git_env "$WORK"
CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
HOME_DIR="$(composer config --global home 2>/dev/null || true)"
[ -n "$HOME_DIR" ] && [ -f "$HOME_DIR/auth.json" ] && cp "$HOME_DIR/auth.json" "$WORK/home/"
# Le store de vivacity propre au passage : les zips sont refabriqués à
# chaque fois, à nom et version égaux.
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION=1.0.0 VIVACITY_CACHE_DIR="$WORK/vivacity-cache"
unset COMPOSER_TESTS_ARE_RUNNING GITHUB_ACTIONS
status=0
ART="$WORK/artifacts"

# Les paquets synthétiques : <vendor>-<name>-<version>.zip, une racine à
# retirer. `vendor/` dans certains : getkirby/composer-installer le supprime.
php -r '
$dir = $argv[1];
$make = function (string $name, string $version, string $type, array $extra, array $files, array $more = []) use ($dir) {
    $root = str_replace("/", "-", $name);
    $json = ["name" => $name, "version" => $version, "type" => $type] + $more;
    if ($extra) { $json["extra"] = $extra; }
    $z = new ZipArchive();
    $z->open("$dir/$root-$version.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
    $z->addFromString("$root/composer.json", json_encode($json, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES));
    foreach ($files + ["README.md" => "# $name $version\n"] as $path => $content) {
        $z->addFromString("$root/$path", $content);
        $z->setExternalAttributesName("$root/$path", ZipArchive::OPSYS_UNIX, 0100644 << 16);
    }
    $z->close();
};
$psr4 = fn (string $ns, string $dir = "src/") => ["autoload" => ["psr-4" => [$ns => $dir]]];
// getkirby/composer-installer
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/kirbycms", $v, "kirby-cms", [], ["src/App.php" => "<?php namespace Acme\\KirbyCms; class App {}", "vendor/autoload.php" => "<?php // bundled\n", "vendor/acme/dep/Dep.php" => "<?php class Dep {}"], $psr4("Acme\\KirbyCms\\"));
}
$make("acme/kirby-shop", "1.0.0", "kirby-plugin", ["installer-name" => "shop"], ["index.php" => "<?php\n", "vendor/autoload.php" => "<?php // bundled\n"], ["require" => ["getkirby/composer-installer" => "^1.2"]]);
$make("acme/kirby-shop", "1.0.1", "kirby-plugin", ["installer-name" => "store"], ["index.php" => "<?php\n"], ["require" => ["getkirby/composer-installer" => "^1.2"]]);
$make("acme/kirby-legacy", "1.0.0", "kirby-plugin", [], ["index.php" => "<?php\n", "vendor/keep.txt" => "kept\n"]);
$make("acme/kirby-td", "1.0.0", "kirby-plugin", [], ["Td.php" => "<?php class Acme_Td {}"], ["require" => ["getkirby/composer-installer" => "^1.2"], "target-dir" => "Acme/Td", "autoload" => ["psr-0" => ["Acme_Td" => ""]]]);
// neos/composer-plugin
$make("acme/neos-site", "1.0.0", "neos-site", [], ["Classes/Site.php" => "<?php namespace Acme\\Site; class Site {}"], $psr4("Acme\\Site\\", "Classes/"));
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/neos-fw", $v, "neos-framework", [], ["Classes/Fw.php" => "<?php class Acme_Fw {}"], ["autoload" => ["psr-0" => ["Acme_Fw" => "Classes/"]]]);
}
$make("acme/neos-legacy", "1.0.0", "neos-package", [], ["Classes/Legacy.php" => "<?php namespace Acme\\Legacy; class Legacy {}"], ["autoload" => ["psr-0" => ["Acme\\Legacy\\" => "Classes/"]]]);
$make("acme/neos-tools", "1.0.0", "neos-package-collection", ["neos" => ["package-key" => "Acme.Tools"]], ["Readme.txt" => "tools\n"]);
$make("acme/flow-plug", "1.0.0", "typo3-flow-plugin", ["installer-name" => "Acme.Plug"], ["Plug.php" => "<?php\n"]);
$make("acme/neos-build-ess", "1.0.0", "neos-build", [], ["build.xml" => "<project/>\n"]);
$make("acme/neos-odd", "1.0.0", "neos-my-extra", [], ["Odd.php" => "<?php\n"]);
$make("acme/lib", "1.0.0", "library", [], ["src/Lib.php" => "<?php namespace Acme\\Lib; class Lib {}", "bin/acme-lib" => "#!/bin/sh\necho lib\n"], $psr4("Acme\\Lib\\") + ["bin" => ["bin/acme-lib"]]);
// davidbarratt/custom-installer
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lang-help", $v, "library", [], ["help.txt" => "help $v\n"]);
}
$make("acme/addon-x", "1.0.0", "acme-addon", [], ["Addon.php" => "<?php class AddonX {}"], ["target-dir" => "Sub", "autoload" => ["psr-0" => ["AddonX" => ""]]]);
$make("acme/theme-y", "1.0.0", "acme-theme", [], ["style.css" => "body{}\n"]);
// johnpbloch/wordpress-core-installer + composer/installers v1.12.0
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/wpcore", $v, "wordpress-core", [], ["wp-load.php" => "<?php // $v\n"]);
}
$make("acme/wp-plug", "1.0.0", "wordpress-plugin", [], ["plug.php" => "<?php\n"]);
// thelia/installer (+ composer/installers v2.3.0)
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/cheque-module", $v, "thelia-module", ["installer-name" => "Cheque"], ["Cheque.php" => "<?php namespace Cheque; class Cheque {}"], $psr4("Cheque\\", ""));
}
$make("acme/front", "1.0.0", "thelia-frontoffice-template", ["installer-name" => "front"], ["index.html.twig" => "{# front #}\n"]);
$make("acme/pdf", "1.0.0", "thelia-pdf-template", ["installer-name" => "pdf"], ["invoice.html.twig" => "{# pdf #}\n"]);
$make("acme/config", "1.0.0", "thelia-local", ["installer-name" => "config"], ["config.yml" => "a: 1\n"]);
$make("acme/core", "1.0.0", "thelia-core", [], ["core.php" => "<?php\n"]);
// contao-components/installer
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/jquery", $v, "contao-component", [], ["jquery.js" => "/* $v */\n"]);
}
// oomphinc/composer-installers-extender (+ composer/installers v2.3.0)
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/np", $v, "npm-asset", [], ["np.js" => "/* $v */\n"]);
}
// craftcms/plugin-installer
$make("acme/craftplug", "1.0.0", "craft-plugin", ["handle" => "craftplug", "class" => "acme\\craftplug\\Plugin"], ["src/Plugin.php" => "<?php\n"], $psr4("acme\\craftplug\\"));
' "$ART" || { echo "FAIL : fabrication des zips"; exit 1; }

# lock <nom> <composer.json> : écrit par Composer sans plugins (le dépôt
# `artifact` d'abord, Packagist pour les plugins).
lock() {
  local d="$WORK/locks/$1"
  rm -rf "$d"; mkdir -p "$d"
  printf '%s\n' "$2" | jq --arg art "$ART" '.repositories = [{"type": "artifact", "url": $art}] | .name = "vivacity/custom-installers"' > "$d/composer.json"
  (cd "$d" && composer update --no-install --no-plugins --no-scripts --no-audit --no-security-blocking --no-interaction --no-ansi -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; exit 1; }
}
# relock <lock source> <nom> <paquet> <version> : le lock source, la version
# d'un paquet réécrite — une version non lue que la plateforme ne résoudrait
# plus (vivacity s'arrête avant tout téléchargement).
relock() {
  local d="$WORK/locks/$2"
  rm -rf "$d"; mkdir -p "$d"
  jq --arg n "$3" --arg v "$4" '.require[$n] = $v' "$WORK/locks/$1/composer.json" > "$d/composer.json"
  jq --arg n "$3" --arg v "$4" '.packages |= map(if .name == $n then .version = $v else . end)' "$WORK/locks/$1/composer.lock" > "$d/composer.lock"
}
stage() { local src="$WORK/locks/$1"; shift; local d; for d in "$@"; do mkdir -p "$d"; cp "$src/composer.json" "$src/composer.lock" "$d/"; done; }
fresh() { local src="$1"; shift; local d; for d in "$@"; do rm -rf "$d"; done; stage "$src" "$@"; }

step() { # ref, viv, nom, args… (les mêmes des deux côtés)
  local ref="$1" viv="$2" name="$3"; shift 3
  local c_code=0 v_code=0
  (cd "$ref" && composer "$@" --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -4 "$WORK/$name.vivacity.err"; status=1; return; fi
  # Le chemin du projet (composer/installers 2.x écrit « Deleting <chemin
  # absolu> - deleted ») ramené à un marqueur, de chaque côté.
  local rr vr; rr="$(cd "$ref" && pwd -P)"; vr="$(cd "$viv" && pwd -P)"
  if ! diff <(sed "s#$rr#<project>#g" "$WORK/$name.composer.out") <(sed "s#$vr#<project>#g" "$WORK/$name.vivacity.out") >"$WORK/$name.out.diff"; then
    echo "FAIL $name : stdout différente"; head -8 "$WORK/$name.out.diff"; status=1; return
  fi
  if ! diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.err" | sed "s#$rr#<project>#g") <(grep -v '^vivacity: \|^Note: plugin \|^Note: composer/installers .* emulated natively' "$WORK/$name.vivacity.err" | sed "s#$vr#<project>#g") >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -10 "$WORK/$name.err.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : code $c_code, stdout, stderr et projet identiques"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}
# handed <lock> <nom> <motif de raison> <args…> : vivacity seul, --no-fallback.
handed() {
  local src="$1" name="$2" pat="$3"; shift 3
  local d="$WORK/$name-viv" v=0
  fresh "$src" "$d"; touch "$WORK/$name.marker"; sleep 1
  (cd "$d" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.all" 2>&1) || v=$?
  if [ "$v" = 3 ] && grep -qE -- "$pat" "$WORK/$name.vivacity.all" && [ -z "$(find "$d" -newer "$WORK/$name.marker" -print -quit)" ]; then
    echo "OK   $name : rendu à Composer, la raison, rien d'écrit"
  else
    echo "FAIL $name : code $v"; tail -4 "$WORK/$name.vivacity.all"; status=1
  fi
}
# handed_on <projet installé> <lock> <nom> <motif> <args…> : comme handed,
# sur une copie d'un projet que Composer a installé (son installed.json).
handed_on() {
  local from="$1" src="$2" name="$3" pat="$4"; shift 4
  local d="$WORK/$name-viv" v=0
  rm -rf "$d"; cp -a "$from" "$d"; stage "$src" "$d"; touch "$WORK/$name.marker"; sleep 1
  (cd "$d" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.all" 2>&1) || v=$?
  if [ "$v" = 3 ] && grep -qE -- "$pat" "$WORK/$name.vivacity.all" && [ -z "$(find "$d" -newer "$WORK/$name.marker" -print -quit)" ]; then
    echo "OK   $name : rendu à Composer, la raison, rien d'écrit"
  else
    echo "FAIL $name : code $v"; tail -4 "$WORK/$name.vivacity.all"; status=1
  fi
}
I=(install --no-scripts)

# run <préfixe> <lock A> <lock B> <lock C> <fichier attendu> : install,
# no-op, mise à jour (B), retrait (C), --no-plugins (A, vendor vierge) ; le
# fichier (relatif au projet) doit exister chez Composer après l'install.
run() {
  local p="$1" a="$2" b="$3" c="$4" ref="$WORK/$1-ref" viv="$WORK/$1-viv"
  fresh "$a" "$ref" "$viv"
  step "$ref" "$viv" "$p-install" "${I[@]}"
  [ -e "$ref/$5" ] || { echo "FAIL $p : $5 absent chez Composer (cas mal construit)"; status=1; }
  step "$ref" "$viv" "$p-no-op" "${I[@]}"
  stage "$b" "$ref" "$viv"; step "$ref" "$viv" "$p-update" "${I[@]}"
  stage "$c" "$ref" "$viv"; step "$ref" "$viv" "$p-removal" "${I[@]}"
  # --no-plugins sur l'arbre posé par les plugins : rien à installer, les
  # chemins de LibraryInstaller pour l'autoloader.
  cp -a "$ref" "$ref-np"; cp -a "$viv" "$viv-np"
  step "$ref-np" "$viv-np" "$p-installed-no-plugins" "${I[@]}" --no-plugins
  ref="$WORK/$1-noplug-ref"; viv="$WORK/$1-noplug-viv"; fresh "$a" "$ref" "$viv"
  step "$ref" "$viv" "$p-no-plugins" "${I[@]}" --no-plugins
}

# --- getkirby/composer-installer 1.2.1 -------------------------------------
KIRBY='"config": {"allow-plugins": {"getkirby/composer-installer": true}}'
lock kirby-a "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.0\", \"acme/kirby-shop\": \"1.0.0\", \"acme/kirby-legacy\": \"1.0.0\"}, $KIRBY}"
lock kirby-b "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.1\", \"acme/kirby-shop\": \"1.0.0\", \"acme/kirby-legacy\": \"1.0.0\"}, $KIRBY}"
lock kirby-c "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.1\", \"acme/kirby-legacy\": \"1.0.0\"}, $KIRBY}"
run kirby kirby-a kirby-b kirby-c kirby/src/App.php
[ -f "$WORK/kirby-ref/kirby/src/App.php" ] || { echo "FAIL kirby : le paquet kirby-cms pas sous kirby/ (cas mal construit)"; status=1; }
[ ! -e "$WORK/kirby-ref/kirby/vendor" ] || { echo "FAIL kirby : kirby/vendor gardé chez Composer (cas mal construit)"; status=1; }
[ -e "$WORK/kirby-ref/vendor/acme/kirby-legacy/vendor/keep.txt" ] || { echo "FAIL kirby : le plugin sans l'installeur a perdu son vendor/ (cas mal construit)"; status=1; }
lock kirby-path "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.0\", \"acme/kirby-shop\": \"1.0.0\"}, \"extra\": {\"kirby-cms-path\": \"core\", \"kirby-plugin-path\": \"plugins\"}, $KIRBY}"
lock kirby-false "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.0\"}, \"extra\": {\"kirby-cms-path\": false}, $KIRBY}"
for l in kirby-path kirby-false; do
  fresh "$l" "$WORK/$l-ref" "$WORK/$l-viv"; step "$WORK/$l-ref" "$WORK/$l-viv" "$l" "${I[@]}"
done
lock kirby-dot "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.0\"}, \"extra\": {\"kirby-cms-path\": \".\"}, $KIRBY}"
handed kirby-dot kirby-dot 'getkirby/composer-installer' "${I[@]}"
# Le chemin d'un paquet change à la mise à jour (installer-name) :
# LibraryInstaller::updateCode renomme, rendu à Composer.
fresh kirby-a "$WORK/km-ref"; (cd "$WORK/km-ref" && composer install --no-scripts --no-interaction --no-ansi -q)
lock kirby-move "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirbycms\": \"1.0.0\", \"acme/kirby-shop\": \"1.0.1\", \"acme/kirby-legacy\": \"1.0.0\"}, $KIRBY}"
handed_on "$WORK/km-ref" kirby-move kirby-move 'moves from `site/plugins/shop`' "${I[@]}"
# kirby et composer/installers 1.x prennent tous deux kirby-plugin : seul
# kirby retire le vendor/ du plugin, le gagnant suit l'ordre d'activation.
lock kirby-mix "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"composer/installers\": \"v1.12.0\", \"acme/kirby-shop\": \"1.0.0\"}, \"config\": {\"allow-plugins\": {\"getkirby/composer-installer\": true, \"composer/installers\": true}}}"
handed kirby-mix kirby-mix 'postInstall' "${I[@]}"
# Une target-dir que kirby n'ajoute pas au chemin : l'autoloader de
# Composer la retranche quand même.
lock kirby-td "{\"require\": {\"getkirby/composer-installer\": \"1.2.1\", \"acme/kirby-td\": \"1.0.0\"}, $KIRBY}"
handed kirby-td kirby-td 'target-dir `Acme/Td`' "${I[@]}"
relock kirby-a kirby-old getkirby/composer-installer 1.2.0
handed kirby-old kirby-old 'getkirby/composer-installer 1\.2\.0' "${I[@]}"

# --- neos/composer-plugin 2.1.3 (vendor-dir Packages/Libraries) -------------
NEOS='"config": {"vendor-dir": "Packages/Libraries", "bin-dir": "bin", "allow-plugins": {"neos/composer-plugin": true}}'
NEOS_PKGS='"acme/neos-site": "1.0.0", "acme/neos-legacy": "1.0.0", "acme/neos-tools": "1.0.0", "acme/flow-plug": "1.0.0", "acme/neos-build-ess": "1.0.0", "acme/neos-odd": "1.0.0", "acme/lib": "1.0.0"'
lock neos-a "{\"require\": {\"neos/composer-plugin\": \"2.1.3\", \"acme/neos-fw\": \"1.0.0\", $NEOS_PKGS}, $NEOS}"
lock neos-b "{\"require\": {\"neos/composer-plugin\": \"2.1.3\", \"acme/neos-fw\": \"1.0.1\", $NEOS_PKGS}, $NEOS}"
lock neos-c "{\"require\": {\"neos/composer-plugin\": \"2.1.3\", $NEOS_PKGS}, $NEOS}"
VENDOR_REL=Packages/Libraries run neos neos-a neos-b neos-c Packages/Framework/Acme_Fw/Classes/Fw.php
[ -d "$WORK/neos-ref/Packages/Sites/Acme.Site" ] && [ -d "$WORK/neos-ref/Packages/Application/Acme.Legacy." ] && [ -d "$WORK/neos-ref/Packages/MyExtra" ] \
  || { echo "FAIL neos : chemins attendus absents chez Composer (cas mal construit)"; status=1; }
relock neos-a neos-old neos/composer-plugin 2.1.0
handed neos-old neos-old 'neos/composer-plugin 2\.1\.0' "${I[@]}"

# --- davidbarratt/custom-installer 1.1.0 ------------------------------------
DB='"config": {"allow-plugins": {"davidbarratt/custom-installer": true}}'
DB_EXTRA='"extra": {"custom-installer": {"vendor/{$vendor}/{$name}/": ["type:library"], "web/{$type}/{$name}": ["acme/lang-help", "type:acme-addon"]}}'
lock db-a "{\"require\": {\"davidbarratt/custom-installer\": \"1.1.0\", \"acme/lang-help\": \"1.0.0\", \"acme/addon-x\": \"1.0.0\", \"acme/theme-y\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $DB_EXTRA, $DB}"
lock db-b "{\"require\": {\"davidbarratt/custom-installer\": \"1.1.0\", \"acme/lang-help\": \"1.0.1\", \"acme/addon-x\": \"1.0.0\", \"acme/theme-y\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $DB_EXTRA, $DB}"
lock db-c "{\"require\": {\"davidbarratt/custom-installer\": \"1.1.0\", \"acme/addon-x\": \"1.0.0\", \"acme/theme-y\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $DB_EXTRA, $DB}"
run db db-a db-b db-c web/library/lang-help/help.txt
[ -f "$WORK/db-ref/web/acme-addon/addon-x/Sub/Addon.php" ] && [ -d "$WORK/db-ref/vendor/acme/lib/src" ] \
  || { echo "FAIL db : chemins attendus absents chez Composer (cas mal construit)"; status=1; }
lock db-legacy "{\"require\": {\"davidbarratt/custom-installer\": \"1.1.0\", \"acme/addon-x\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"custom-installer\": {\"acme-addon\": \"legacy/{\$name}\"}}, $DB}"
fresh db-legacy "$WORK/db-legacy-ref" "$WORK/db-legacy-viv"; step "$WORK/db-legacy-ref" "$WORK/db-legacy-viv" db-legacy "${I[@]}"
relock db-a db-old davidbarratt/custom-installer 1.0.0
handed db-old db-old 'davidbarratt/custom-installer 1\.0\.0' "${I[@]}"

# --- johnpbloch/wordpress-core-installer 2.0.0 + composer/installers v1.12.0 -
JP='"config": {"allow-plugins": {"johnpbloch/wordpress-core-installer": true, "composer/installers": true}}'
JP_EXTRA='"extra": {"installer-paths": {"web/plugins/{$name}/": ["type:wordpress-plugin"]}, "wordpress-install-dir": {"acme/wpcore": "web/wp"}}'
lock jp-a "{\"require\": {\"johnpbloch/wordpress-core-installer\": \"2.0.0\", \"composer/installers\": \"v1.12.0\", \"acme/wpcore\": \"1.0.0\", \"acme/wp-plug\": \"1.0.0\"}, $JP_EXTRA, $JP}"
lock jp-b "{\"require\": {\"johnpbloch/wordpress-core-installer\": \"2.0.0\", \"composer/installers\": \"v1.12.0\", \"acme/wpcore\": \"1.0.1\", \"acme/wp-plug\": \"1.0.0\"}, $JP_EXTRA, $JP}"
lock jp-c "{\"require\": {\"johnpbloch/wordpress-core-installer\": \"2.0.0\", \"composer/installers\": \"v1.12.0\", \"acme/wpcore\": \"1.0.1\"}, $JP_EXTRA, $JP}"
run jp jp-a jp-b jp-c web/plugins/wp-plug/plug.php
[ -f "$WORK/jp-ref/web/wp/wp-load.php" ] || { echo "FAIL jp : wordpress-core pas sous web/wp chez Composer (cas mal construit)"; status=1; }
# Les autres commandes : la dépréciation au chargement des plugins.
step "$WORK/jp-ref" "$WORK/jp-viv" jp-dump dump-autoload
step "$WORK/jp-ref" "$WORK/jp-viv" jp-remove remove acme/wpcore --no-update
relock jp-a jp-old johnpbloch/wordpress-core-installer 1.0.2
handed jp-old jp-old 'johnpbloch/wordpress-core-installer 1\.0\.2' "${I[@]}"

# --- thelia/installer 1.6 + composer/installers v2.3.0 ----------------------
TH='"config": {"allow-plugins": {"thelia/installer": true, "composer/installers": true}}'
TH3='"extra": {"installer-paths": {"vendor/{$vendor}/modules/{$name}": ["type:thelia-module"], "templates/frontOffice/{$name}": ["type:thelia-frontoffice-template"], "templates/pdf/{$name}": ["type:thelia-pdf-template"], "local/": ["type:thelia-local"]}}'
TH_PKGS='"acme/front": "1.0.0", "acme/pdf": "1.0.0", "acme/config": "1.0.0", "acme/core": "1.0.0", "acme/lib": "1.0.0"'
lock th-a "{\"require\": {\"thelia/installer\": \"1.6\", \"composer/installers\": \"v2.3.0\", \"acme/cheque-module\": \"1.0.0\", $TH_PKGS}, $TH3, $TH}"
lock th-b "{\"require\": {\"thelia/installer\": \"1.6\", \"composer/installers\": \"v2.3.0\", \"acme/cheque-module\": \"1.0.1\", $TH_PKGS}, $TH3, $TH}"
lock th-c "{\"require\": {\"thelia/installer\": \"1.6\", \"composer/installers\": \"v2.3.0\", $TH_PKGS}, $TH3, $TH}"
run th th-a th-b th-c vendor/acme/modules/Cheque/Cheque.php
[ -f "$WORK/th-ref/local/config/config.yml" ] || { echo "FAIL th : thelia-local pas sous local/ (cas mal construit)"; status=1; }
grep -q '^Deleting .*/vendor/acme/modules/Cheque - deleted$' "$WORK/th-removal.composer.out" \
  || { echo "FAIL th-removal : pas de ligne « Deleting » chez Composer (cas mal construit)"; status=1; }
# Thelia 2 : sans `type:thelia-module` dans installer-paths, le plugin prend
# aussi les modules (local/modules/) et thelia-core (racine).
lock th2 "{\"require\": {\"thelia/installer\": \"1.6\", \"composer/installers\": \"v2.3.0\", \"acme/cheque-module\": \"1.0.0\", $TH_PKGS}, $TH}"
fresh th2 "$WORK/th2-ref" "$WORK/th2-viv"; step "$WORK/th2-ref" "$WORK/th2-viv" th2 "${I[@]}"
[ -f "$WORK/th2-ref/local/modules/Cheque/Cheque.php" ] || { echo "FAIL th2 : module pas sous local/modules (cas mal construit)"; status=1; }
# Retrait d'un template que thelia/installer et composer/installers
# prennent tous deux : seul le second écrit « Deleting ».
lock th-nofront "{\"require\": {\"thelia/installer\": \"1.6\", \"composer/installers\": \"v2.3.0\", \"acme/pdf\": \"1.0.0\", \"acme/config\": \"1.0.0\", \"acme/core\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $TH3, $TH}"
handed_on "$WORK/th-ref" th-nofront th-nofront 'removal of acme/front' "${I[@]}"
relock th-a th-old thelia/installer 1.5
handed th-old th-old 'thelia/installer 1\.5' "${I[@]}"

# --- contao-components/installer 1.4.1 ---------------------------------------
CC='"config": {"allow-plugins": {"contao-components/installer": true}}'
lock cc-a "{\"require\": {\"contao-components/installer\": \"1.4.1\", \"acme/jquery\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"contao-component-dir\": \"assets\"}, $CC}"
lock cc-b "{\"require\": {\"contao-components/installer\": \"1.4.1\", \"acme/jquery\": \"1.0.1\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"contao-component-dir\": \"assets\"}, $CC}"
lock cc-c "{\"require\": {\"contao-components/installer\": \"1.4.1\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"contao-component-dir\": \"assets\"}, $CC}"
run cc cc-a cc-b cc-c assets/jquery/jquery.js
lock cc-nodir "{\"require\": {\"contao-components/installer\": \"1.4.1\", \"acme/jquery\": \"1.0.0\"}, $CC}"
handed cc-nodir cc-nodir 'contao-components/installer' "${I[@]}"
# Sans installation (`remove --no-update`), l'activation du plugin lit
# config.component-dir : une ligne de dépréciation chez Composer.
lock cc-config "{\"require\": {\"contao-components/installer\": \"1.4.1\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"contao-component-dir\": \"assets\"}, \"config\": {\"component-dir\": \"assets\", \"allow-plugins\": {\"contao-components/installer\": true}}}"
handed_on "$WORK/cc-ref" cc-config cc-config 'component-dir' remove acme/lib --no-update

# --- oomphinc/composer-installers-extender 2.0.1 + composer/installers v2.3.0
OO='"config": {"allow-plugins": {"oomphinc/composer-installers-extender": true, "composer/installers": true}}'
OO_EXTRA='"extra": {"installer-types": ["npm-asset"], "installer-paths": {"public/libs/{$name}/": ["type:npm-asset"]}}'
lock oo-a "{\"require\": {\"oomphinc/composer-installers-extender\": \"2.0.1\", \"composer/installers\": \"v2.3.0\", \"acme/np\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $OO_EXTRA, $OO}"
lock oo-b "{\"require\": {\"oomphinc/composer-installers-extender\": \"2.0.1\", \"composer/installers\": \"v2.3.0\", \"acme/np\": \"1.0.1\", \"acme/lib\": \"1.0.0\"}, $OO_EXTRA, $OO}"
lock oo-c "{\"require\": {\"oomphinc/composer-installers-extender\": \"2.0.1\", \"composer/installers\": \"v2.3.0\", \"acme/lib\": \"1.0.0\"}, $OO_EXTRA, $OO}"
run oo oo-a oo-b oo-c public/libs/np/np.js
lock oo-nopath "{\"require\": {\"oomphinc/composer-installers-extender\": \"2.0.1\", \"composer/installers\": \"v2.3.0\", \"acme/np\": \"1.0.0\"}, \"extra\": {\"installer-types\": [\"npm-asset\"]}, $OO}"
handed oo-nopath oo-nopath 'oomphinc/composer-installers-extender' "${I[@]}"

# --- craftcms/plugin-installer 1.6.0 : inerte sans paquet craft-plugin -----
CR='"config": {"allow-plugins": {"craftcms/plugin-installer": true}}'
lock cr-a "{\"require\": {\"craftcms/plugin-installer\": \"1.6.0\", \"acme/lib\": \"1.0.0\"}, $CR}"
fresh cr-a "$WORK/cr-ref" "$WORK/cr-viv"; step "$WORK/cr-ref" "$WORK/cr-viv" cr-install "${I[@]}"
step "$WORK/cr-ref" "$WORK/cr-viv" cr-no-op "${I[@]}"
lock cr-plugin "{\"require\": {\"craftcms/plugin-installer\": \"1.6.0\", \"acme/craftplug\": \"1.0.0\"}, $CR}"
handed cr-plugin cr-plugin 'craftcms/plugin-installer' "${I[@]}"

exit $status
