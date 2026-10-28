#!/usr/bin/env bash
# Plugins à effets de bord, second tour (docs/plans/plugins-round2.md) :
# spiral/composer-publish-plugin, sculpin/sculpin-theme-composer-plugin,
# laminas/laminas-component-installer, laminas/laminas-skeleton-installer,
# dg/composer-cleaner, liborm85/composer-vendor-cleaner,
# cakephp/plugin-installer. Par plugin, un projet qui exige le vrai plugin à
# sa version exacte (Packagist) et des paquets synthétiques servis par un
# dépôt `artifact` (zips fabriqués par ZipArchive) ; les locks sont écrits
# par Composer `--no-plugins`.
#
# Étapes, chacune contre Composer 2.10.3 (plugins actifs) : install sur
# vendor vierge, no-op, mise à jour d'un paquet (second lock), retrait d'un
# paquet (troisième lock), `--no-plugins` sur l'arbre posé et sur un vendor
# vierge ; selon le plugin, `dump-autoload`, `update`, le retrait du plugin
# lui-même, `--no-dev`, `-v`. Par étape : code de sortie, stdout, stderr (les
# lignes `Downloading` et les notes `vivacity:` mises à part) et le projet
# entier (fichiers, modes, liens, installed.json, autoloader —
# harness/lib/compare.sh). Les configurations que vivacity rend à Composer :
# code 3 sous --no-fallback, la raison, rien d'écrit.
#
# Usage : harness/plugins-round2.sh
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/plugins-round2"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication des zips)"; exit 1; }
command -v jq >/dev/null || { echo "jq requis"; exit 1; }
composer --version --no-ansi | tail -1
php -r 'echo "PHP ", PHP_VERSION, "\n";'
rm -rf "$WORK"; mkdir -p "$WORK/artifacts" "$WORK/home"; harness_git_env "$WORK"
CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
HOME_DIR="$(composer config --global home 2>/dev/null || true)"
[ -n "$HOME_DIR" ] && [ -f "$HOME_DIR/auth.json" ] && cp "$HOME_DIR/auth.json" "$WORK/home/"
# Le store de vivacity propre au passage : les zips sont refabriqués à
# chaque fois, à nom et version égaux.
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION=1.0.0 VIVACITY_CACHE_DIR="$WORK/vivacity-cache"
unset COMPOSER_TESTS_ARE_RUNNING GITHUB_ACTIONS COMPOSER_NO_DEV
status=0
ART="$WORK/artifacts"

# Les paquets synthétiques : <vendor>-<name>-<version>.zip, une racine à
# retirer. Un contenu `null` est un répertoire vide, `["link" => cible]` un
# lien symbolique.
php -r '
$dir = $argv[1];
$make = function (string $name, string $version, string $type, array $extra, array $files, array $more = []) use ($dir) {
    $root = str_replace("/", "-", $name);
    $json = ["name" => $name, "version" => $version, "type" => $type] + $more;
    if ($extra) { $json["extra"] = $extra; }
    $z = new ZipArchive();
    $z->open("$dir/$root-$version.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
    $z->addFromString("$root/composer.json", json_encode($json, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES));
    $z->setExternalAttributesName("$root/composer.json", ZipArchive::OPSYS_UNIX, 0100644 << 16);
    foreach ($files + ["README.md" => "# $name $version\n"] as $path => $content) {
        if ($content === null) {
            $z->addEmptyDir("$root/$path");
            $z->setExternalAttributesName("$root/$path/", ZipArchive::OPSYS_UNIX, 040755 << 16);
        } elseif (is_array($content)) {
            $z->addFromString("$root/$path", $content["link"]);
            $z->setExternalAttributesName("$root/$path", ZipArchive::OPSYS_UNIX, 0120777 << 16);
        } else {
            $z->addFromString("$root/$path", $content);
            $mode = str_starts_with($path, "bin/") ? 0100755 : 0100644;
            $z->setExternalAttributesName("$root/$path", ZipArchive::OPSYS_UNIX, $mode << 16);
        }
    }
    $z->close();
};
$psr4 = fn (string $ns, string $dir = "src/") => ["autoload" => ["psr-4" => [$ns => $dir]]];
$cls = fn (string $ns, string $c) => "<?php\nnamespace $ns;\nclass $c {}\n";
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lib", $v, "library", [], ["src/Lib.php" => $cls("Acme\\Lib", "Lib")], $psr4("Acme\\Lib\\"));
}
$make("acme/lib2", "1.0.0", "library", [], ["src/Lib2.php" => $cls("Acme\\Lib2", "Lib2")], $psr4("Acme\\Lib2\\"));
// spiral/composer-publish-plugin
$make("acme/sp-pub", "1.0.0", "library", ["publish" => ["public/sp.css" => "replace"]], ["public/sp.css" => "body{}\n"]);
// sculpin/sculpin-theme-composer-plugin
$make("acme/sc-theme", "1.0.0", "sculpin-theme", [], ["_views/default.html" => "<html></html>\n"]);
$make("acme/sc-extra", "1.0.0", "library", ["sculpin-theme" => ["name" => "acme/extra"]], ["_views/x.html" => "<p></p>\n"]);
// laminas/laminas-component-installer
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lc-mod", $v, "library", ["laminas" => ["module" => "Acme\\LcMod", "config-provider" => "Acme\\LcMod\\ConfigProvider"]], ["src/Module.php" => $cls("Acme\\LcMod", "Module")], $psr4("Acme\\LcMod\\"));
}
// dg/composer-cleaner
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/dg-lib", $v, "library", [], [
        "src/A.php" => $cls("Acme\\DgLib", "A"), "src/Legacy/L.php" => $cls("Acme\\DgLib\\Legacy", "L"),
        "Tests/T.php" => "<?php\n", "tests/x.php" => "<?php\n", "docs/a.md" => "# a $v\n", "LICENSE.txt" => "MIT\n",
        "license" => "MIT\n", "License.md" => "MIT\n", ".gitattributes" => "/tests export-ignore\n", ".phpstorm.meta.php" => "<?php\n",
        "bin/dg-tool" => "#!/bin/sh\necho dg\n", "CHANGELOG.md" => "- $v\n", "docs-link" => ["link" => "docs"], "dangling" => ["link" => "nowhere"],
    ], ["autoload" => ["psr-4" => ["Acme\\DgLib\\" => "src/"], "exclude-from-classmap" => ["/Tests/", "src/Legacy/"]], "bin" => ["bin/dg-tool"]]);
}
$make("acme/dg-psr0", "1.0.0", "library", [], ["lib/Acme/Psr0.php" => "<?php\nclass Acme_Psr0 {}\n", "extra.txt" => "x\n"], ["autoload" => ["psr-0" => ["Acme_Psr0" => "lib/"]]]);
$make("acme/dg-root", "1.0.0", "library", [], ["Root.php" => $cls("Acme\\Root", "Root"), "junk.txt" => "x\n"], $psr4("Acme\\Root\\", ""));
$make("acme/dg-noauto", "1.0.0", "library", [], ["junk.txt" => "x\n"]);
$make("acme/dg-type", "1.0.0", "acme-thing", [], ["src/T.php" => $cls("Acme\\T", "T"), "junk.txt" => "x\n"], $psr4("Acme\\T\\"));
$make("acme/dg-ign", "1.0.0", "library", [], ["src/I.php" => $cls("Acme\\I", "I"), "junk.txt" => "x\n"], $psr4("Acme\\I\\"));
$make("acme/dg-part", "1.0.0", "library", [], ["src/P.php" => $cls("Acme\\P", "P"), "docs/p.md" => "p\n", "junk.txt" => "x\n"], $psr4("Acme\\P\\"));
$make("acme/dg-dot", "1.0.0", "library", [], [".src/D.php" => $cls("Acme\\D", "D"), "src/S.php" => "<?php\n", "junk.txt" => "x\n"], $psr4("Acme\\D\\", ".src/"));
$make("acme/dg-unknown", "1.0.0", "library", [], ["src/U.php" => $cls("Acme\\U", "U"), "junk.txt" => "x\n"], ["autoload" => ["psr-4" => ["Acme\\U\\" => "src/"], "psr-5" => ["Acme\\U\\" => "src/"]]]);
// liborm85/composer-vendor-cleaner
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lv-lib", $v, "library", [], [
        "src/A.php" => $cls("Acme\\LvLib", "A"), "src/tests/TestA.php" => $cls("Acme\\LvLib\\Tests", "TestA"),
        "tests/T.php" => $cls("Acme\\LvLib\\T", "T"), "docs/d.md" => "d $v\n", ".travis.yml" => "x\n",
        "src/.travis.yml" => "x\n", "empty" => null, "keep/k.txt" => "k\n", "deep/a/b" => null,
    ], ["autoload" => ["classmap" => ["src/"]]]);
}
$make("acme/lv-other", "1.0.0", "library", [], ["Docs/D.md" => "D\n", "res/a.txt" => "a\n", "res/keep.txt" => "k\n", "res/sub/b.txt" => "b\n", "src/O.php" => $cls("Acme\\O", "O")], $psr4("Acme\\O\\"));
$make("acme/lv-meta", "1.0.0", "metapackage", [], [], ["require" => ["acme/lib" => "*"]]);
$make("acme/lv-dev", "1.0.0", "library", [], ["tests/x.txt" => "x\n", "src/Dv.php" => $cls("Acme\\Dv", "Dv")], $psr4("Acme\\Dv\\"));
$make("acme/lv-bin", "1.0.0", "library", [], ["bin/lv" => "#!/bin/sh\necho lv\n", "tests/y.txt" => "y\n"], ["bin" => ["bin/lv"]]);
// cakephp/plugin-installer
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/ck-plug", $v, "cakephp-plugin", [], ["src/Plugin.php" => $cls("Acme\\Plug", "Plugin")], $psr4("Acme\\Plug\\"));
}
$make("acme/ck-multi", "1.0.0", "cakephp-plugin", [], ["src/Plugin.php" => $cls("Acme\\Multi", "Plugin"), "tests/T.php" => "<?php\n"], ["autoload" => ["psr-4" => ["Acme\\Multi\\Test\\" => "tests/", "Acme\\Multi\\" => "src/"]]]);
$make("acme/ck-dev", "1.0.0", "cakephp-plugin", [], ["src/Plugin.php" => $cls("Acme\\Dev", "Plugin")], $psr4("Acme\\Dev\\", "src"));
$make("acme/ck-nons", "1.0.0", "cakephp-plugin", [], ["Plugin.php" => "<?php\n"], ["autoload" => ["classmap" => ["Plugin.php"]]]);
' "$ART" || { echo "FAIL : fabrication des zips"; exit 1; }

# lock <nom> <composer.json> : écrit par Composer sans plugins (le dépôt
# `artifact` d'abord, Packagist pour les plugins). `INDENT` : l'indentation
# du composer.json écrit (jq : 2 espaces par défaut). `PACKAGIST_ONLY` :
# sans le dépôt `artifact`, pour les étapes `update` (vivacity ne résout pas
# encore un dépôt `artifact` : il s'arrête en erreur, défaut noté au plan).
lock() {
  local d="$WORK/locks/$1"
  rm -rf "$d"; mkdir -p "$d"
  # shellcheck disable=SC2086
  if [ -n "${PACKAGIST_ONLY:-}" ]; then
    printf '%s\n' "$2" | jq ${INDENT:-} '.name = "vivacity/plugins-round2"' > "$d/composer.json"
  else
    printf '%s\n' "$2" | jq ${INDENT:-} --arg art "$ART" '.repositories = [{"type": "artifact", "url": $art}] | .name = "vivacity/plugins-round2"' > "$d/composer.json"
  fi
  (cd "$d" && composer update --no-install --no-plugins --no-scripts --no-audit --no-security-blocking --no-interaction --no-ansi -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; exit 1; }
}
# relock <lock source> <nom> <paquet> <version> : le lock source, la version
# d'un paquet réécrite — une version non lue (vivacity s'arrête avant tout
# téléchargement).
relock() {
  local d="$WORK/locks/$2"
  rm -rf "$d"; mkdir -p "$d"
  jq --arg n "$3" --arg v "$4" '.require[$n] = $v' "$WORK/locks/$1/composer.json" > "$d/composer.json"
  jq --arg n "$3" --arg v "$4" '.packages |= map(if .name == $n then .version = $v else . end)' "$WORK/locks/$1/composer.lock" > "$d/composer.lock"
}
# `TREE` : une fonction qui pose l'arbre du projet (plugins applicatifs de
# cakephp, fichiers de configuration de laminas) dans un répertoire.
stage() { local src="$WORK/locks/$1"; shift; local d; for d in "$@"; do mkdir -p "$d"; cp "$src/composer.json" "$src/composer.lock" "$d/"; if [ -n "${TREE:-}" ]; then "$TREE" "$d"; fi; done; }
fresh() { local src="$1"; shift; local d; for d in "$@"; do rm -rf "$d"; done; stage "$src" "$@"; }

step() { # ref, viv, nom, args… (les mêmes des deux côtés)
  local ref="$1" viv="$2" name="$3"; shift 3
  local c_code=0 v_code=0
  (cd "$ref" && composer "$@" --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -4 "$WORK/$name.vivacity.err"; status=1; return; fi
  # Le chemin du projet (dg/composer-cleaner écrit des chemins absolus en
  # cas d'erreur) ramené à un marqueur, de chaque côté.
  local rr vr; rr="$(cd "$ref" && pwd -P)"; vr="$(cd "$viv" && pwd -P)"
  if ! diff <(sed -e "s#$rr#<project>#g" -e "s#$ref#<project>#g" "$WORK/$name.composer.out") <(sed -e "s#$vr#<project>#g" -e "s#$viv#<project>#g" "$WORK/$name.vivacity.out") >"$WORK/$name.out.diff"; then
    echo "FAIL $name : stdout différente"; head -8 "$WORK/$name.out.diff"; status=1; return
  fi
  # (`-v` : la durée de la résolution ramenée à un marqueur.)
  local t='s/completed in [0-9.]* seconds/completed in <t> seconds/'
  if ! diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.err" | sed -e "s#$rr#<project>#g" -e "s#$ref#<project>#g" -e "$t") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.err" | sed -e "s#$vr#<project>#g" -e "s#$viv#<project>#g" -e "$t") >"$WORK/$name.err.diff"; then
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
# handed_on <projet installé> <lock|-> <nom> <motif> <args…> : comme handed,
# sur une copie d'un projet que Composer a installé (son installed.json) ;
# `-` garde le composer.json et le lock du projet.
handed_on() {
  local from="$1" src="$2" name="$3" pat="$4"; shift 4
  local d="$WORK/$name-viv" v=0
  rm -rf "$d"; cp -a "$from" "$d"; [ "$src" = - ] || stage "$src" "$d"; touch "$WORK/$name.marker"; sleep 1
  (cd "$d" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.all" 2>&1) || v=$?
  if [ "$v" = 3 ] && grep -qE -- "$pat" "$WORK/$name.vivacity.all" && [ -z "$(find "$d" -newer "$WORK/$name.marker" -print -quit)" ]; then
    echo "OK   $name : rendu à Composer, la raison, rien d'écrit"
  else
    echo "FAIL $name : code $v"; tail -4 "$WORK/$name.vivacity.all"; status=1
  fi
}
I=(install --no-scripts)
U=(update --no-scripts --no-audit --no-security-blocking)

# run <préfixe> <lock A> <lock B> <lock C> <fichier attendu> [args d'install…] :
# install, no-op, mise à jour (B), retrait (C), --no-plugins sur l'arbre
# posé, --no-plugins sur un vendor vierge (A) ; le fichier (relatif au
# projet) doit exister chez Composer après l'install.
run() {
  local p="$1" a="$2" b="$3" c="$4" want="$5"; shift 5
  local args=("${I[@]}" "$@") ref="$WORK/$p-ref" viv="$WORK/$p-viv"
  fresh "$a" "$ref" "$viv"
  step "$ref" "$viv" "$p-install" "${args[@]}"
  [ -e "$ref/$want" ] || { echo "FAIL $p : $want absent chez Composer (cas mal construit)"; status=1; }
  step "$ref" "$viv" "$p-no-op" "${args[@]}"
  stage "$b" "$ref" "$viv"; step "$ref" "$viv" "$p-update" "${args[@]}"
  stage "$c" "$ref" "$viv"; step "$ref" "$viv" "$p-removal" "${args[@]}"
  rm -rf "$ref-np" "$viv-np"; cp -a "$ref" "$ref-np"; cp -a "$viv" "$viv-np"
  step "$ref-np" "$viv-np" "$p-installed-no-plugins" "${args[@]}" --no-plugins
  ref="$WORK/$p-noplug-ref"; viv="$WORK/$p-noplug-viv"; fresh "$a" "$ref" "$viv"
  step "$ref" "$viv" "$p-no-plugins" "${args[@]}" --no-plugins
}

# --- spiral/composer-publish-plugin v1.1.1–v1.1.2 : inerte sans paquet `publish`
# v1.1.1 exige php ^7.1 : plateforme figée à 7.4.
SP='"extra": {"publish-cmd": "php app.php publish"}, "config": {"platform": {"php": "7.4.0"}, "allow-plugins": {"spiral/composer-publish-plugin": true}}'
lock sp-a "{\"require\": {\"spiral/composer-publish-plugin\": \"v1.1.1\", \"acme/lib\": \"1.0.0\", \"acme/lib2\": \"1.0.0\"}, $SP}"
# (spiral/code-style, que v1.1.1 exige, gardé en B et C : son retrait
# viderait vendor/bin, un écart de vivacity indépendant de ce plugin —
# noté au plan.)
lock sp-b "{\"require\": {\"spiral/composer-publish-plugin\": \"v1.1.2\", \"spiral/code-style\": \"^1.0\", \"acme/lib\": \"1.0.1\", \"acme/lib2\": \"1.0.0\"}, $SP}"
lock sp-c "{\"require\": {\"spiral/composer-publish-plugin\": \"v1.1.2\", \"spiral/code-style\": \"^1.0\", \"acme/lib\": \"1.0.1\"}, $SP}"
run sp sp-a sp-b sp-c vendor/spiral/composer-publish-plugin/src/PublishPlugin.php
step "$WORK/sp-ref" "$WORK/sp-viv" sp-dump dump-autoload
lock sp-pub "{\"require\": {\"spiral/composer-publish-plugin\": \"v1.1.2\", \"acme/sp-pub\": \"1.0.0\"}, $SP}"
handed sp-pub sp-pub 'spiral/composer-publish-plugin' "${I[@]}"
fresh sp-pub "$WORK/sp-pub-ref"; (cd "$WORK/sp-pub-ref" && composer install --no-scripts --no-plugins --no-interaction --no-ansi -q)
handed_on "$WORK/sp-pub-ref" - sp-pub-dump 'spiral/composer-publish-plugin' dump-autoload
relock sp-a sp-old spiral/composer-publish-plugin v1.1.0
handed sp-old sp-old 'spiral/composer-publish-plugin v1\.1\.0' "${I[@]}"

# --- sculpin/sculpin-theme-composer-plugin 1.0.3 : inerte sans thème -------
SC='"config": {"allow-plugins": {"sculpin/sculpin-theme-composer-plugin": true}}'
lock sc-a "{\"require\": {\"sculpin/sculpin-theme-composer-plugin\": \"1.0.3\", \"acme/lib\": \"1.0.0\", \"acme/lib2\": \"1.0.0\"}, $SC}"
lock sc-b "{\"require\": {\"sculpin/sculpin-theme-composer-plugin\": \"1.0.3\", \"acme/lib\": \"1.0.1\", \"acme/lib2\": \"1.0.0\"}, $SC}"
lock sc-c "{\"require\": {\"sculpin/sculpin-theme-composer-plugin\": \"1.0.3\", \"acme/lib\": \"1.0.1\"}, $SC}"
run sc sc-a sc-b sc-c vendor/acme/lib/src/Lib.php
lock sc-theme "{\"require\": {\"sculpin/sculpin-theme-composer-plugin\": \"1.0.3\", \"acme/sc-theme\": \"1.0.0\"}, $SC}"
handed sc-theme sc-theme 'sculpin' "${I[@]}"
lock sc-extra "{\"require\": {\"sculpin/sculpin-theme-composer-plugin\": \"1.0.3\", \"acme/sc-extra\": \"1.0.0\"}, $SC}"
handed sc-extra sc-extra 'sculpin' "${I[@]}"
relock sc-a sc-old sculpin/sculpin-theme-composer-plugin 1.0.2
handed sc-old sc-old 'sculpin/sculpin-theme-composer-plugin 1\.0\.2' "${I[@]}"

# --- laminas/laminas-component-installer 3.5.0–3.8.0 -----------------------
# Inerte en --no-dev, et en dev sans fichier de configuration à découvrir.
LC='"config": {"allow-plugins": {"laminas/laminas-component-installer": true}}'
lock lc-a "{\"require\": {\"laminas/laminas-component-installer\": \"3.8.0\", \"acme/lc-mod\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"require-dev\": {\"acme/lib2\": \"1.0.0\"}, $LC}"
lock lc-b "{\"require\": {\"laminas/laminas-component-installer\": \"3.8.0\", \"acme/lc-mod\": \"1.0.1\", \"acme/lib\": \"1.0.0\"}, \"require-dev\": {\"acme/lib2\": \"1.0.0\"}, $LC}"
lock lc-c "{\"require\": {\"laminas/laminas-component-installer\": \"3.8.0\", \"acme/lib\": \"1.0.0\"}, \"require-dev\": {\"acme/lib2\": \"1.0.0\"}, $LC}"
run lc lc-a lc-b lc-c vendor/acme/lc-mod/src/Module.php
# Un fichier de configuration : inerte en --no-dev quand même.
lc_config() { mkdir -p "$1/config"; printf '<?php\nreturn [\n    '"'"'modules'"'"' => [\n    ],\n];\n' > "$1/config/application.config.php"; }
TREE=lc_config
run lcp lc-a lc-b lc-c config/application.config.php --no-dev
handed lc-a lc-config 'laminas/laminas-component-installer' "${I[@]}"
TREE=
relock lc-a lc-old laminas/laminas-component-installer 3.4.0
handed lc-old lc-old 'laminas/laminas-component-installer 3\.4\.0' "${I[@]}" --no-dev

# --- laminas/laminas-skeleton-installer 1.2.0–1.3.0 ------------------------
# Avec codeception/c3 2.9.0, qui imprime à post-install-cmd à la priorité 0 :
# skeleton-installer (priorité 1000) passe avant lui quel que soit l'ordre
# d'activation. composer.json écrit par jq en 2 espaces (indentation
# détectée), puis en 4, puis en tabulations.
SK_PLUG='"laminas/laminas-skeleton-installer": "1.3.0", "codeception/c3": "2.9.0"'
SK_CFG='"config": {"platform": {"php": "8.3.0"}, "allow-plugins": {"laminas/laminas-skeleton-installer": true, "laminas/laminas-component-installer": true, "codeception/c3": true}}'
SK_EXTRA='"extra": {"laminas-skeleton-installer": [{"name": "acme/lib2", "constraint": "^1.0", "prompt": "Install lib2?", "module": true}, {"name": "acme/lc-mod", "constraint": "^1.0", "prompt": "Install lc?", "dev": true}], "other": {"empty": {}, "list": [], "slash": "a/b", "uni": "été"}}'
lock sk-a "{\"require\": {$SK_PLUG, \"acme/lib\": \"1.0.0\", \"acme/lib2\": \"1.0.0\"}, $SK_EXTRA, $SK_CFG}"
lock sk-b "{\"require\": {$SK_PLUG, \"acme/lib\": \"1.0.1\", \"acme/lib2\": \"1.0.0\"}, $SK_EXTRA, $SK_CFG}"
lock sk-c "{\"require\": {$SK_PLUG, \"acme/lib\": \"1.0.1\"}, $SK_EXTRA, $SK_CFG}"
run sk sk-a sk-b sk-c c3.php
grep -q 'Removing optional packages from composer.json' "$WORK/sk-install.composer.out" \
  || { echo "FAIL sk : pas de ligne du plugin chez Composer (cas mal construit)"; status=1; }
INDENT='--indent 4' lock sk-4 "{\"require\": {$SK_PLUG, \"acme/lib\": \"1.0.0\"}, \"extra\": {\"zend-skeleton-installer\": [{\"name\": \"acme/lib2\", \"constraint\": \"^1.0\", \"prompt\": \"Install?\"}]}, $SK_CFG}"
fresh sk-4 "$WORK/sk-4-ref" "$WORK/sk-4-viv"; step "$WORK/sk-4-ref" "$WORK/sk-4-viv" sk-4 "${I[@]}"
INDENT='--tab' lock sk-tab "{\"require\": {$SK_PLUG, \"acme/lib\": \"1.0.0\"}, $SK_EXTRA, $SK_CFG}"
fresh sk-tab "$WORK/sk-tab-ref" "$WORK/sk-tab-viv"; step "$WORK/sk-tab-ref" "$WORK/sk-tab-viv" sk-tab "${I[@]}"
# `update` : post-update-cmd (paquets de Packagist seulement).
PACKAGIST_ONLY=1 lock sk-up "{\"require\": {$SK_PLUG, \"psr/log\": \"3.0.2\"}, $SK_EXTRA, $SK_CFG}"
fresh sk-up "$WORK/sk-up-ref" "$WORK/sk-up-viv"; step "$WORK/sk-up-ref" "$WORK/sk-up-viv" sk-update "${U[@]}"
# Le retrait du plugin boucle chez Composer (Plugin::uninstall → Uninstaller).
lock sk-gone "{\"require\": {\"codeception/c3\": \"2.9.0\", \"acme/lib\": \"1.0.0\"}, $SK_EXTRA, $SK_CFG}"
handed_on "$WORK/sk-ref" sk-gone sk-gone 'laminas/laminas-skeleton-installer' "${I[@]}"
# `require` et `--no-install` sur un projet installé dont composer.json
# porte encore la configuration (paquets de Packagist : vivacity résout).
fresh sk-up "$WORK/sk-rq-ref"; (cd "$WORK/sk-rq-ref" && composer install --no-scripts --no-plugins --no-interaction --no-ansi -q)
handed_on "$WORK/sk-rq-ref" - sk-require 'laminas/laminas-skeleton-installer' require psr/container:2.0.2
handed_on "$WORK/sk-rq-ref" - sk-no-install 'laminas/laminas-skeleton-installer' "${U[@]}" --no-install
lock sk-bad "{\"require\": {$SK_PLUG, \"acme/lib\": \"1.0.0\"}, \"extra\": {\"laminas-skeleton-installer\": [\"acme/lib2\"]}, $SK_CFG}"
handed sk-bad sk-bad 'laminas/laminas-skeleton-installer' "${I[@]}"
relock sk-a sk-old laminas/laminas-skeleton-installer 1.1.0
handed sk-old sk-old 'laminas/laminas-skeleton-installer 1\.1\.0' "${I[@]}"

# --- dg/composer-cleaner v2.2.2 ----------------------------------------------
DG_CFG='"config": {"allow-plugins": {"dg/composer-cleaner": true}}'
DG_EXTRA='"extra": {"cleaner-ignore": {"acme/dg-ign": true, "acme/dg-part": ["doc*"]}}'
DG_PKGS='"acme/dg-psr0": "1.0.0", "acme/dg-root": "1.0.0", "acme/dg-noauto": "1.0.0", "acme/dg-type": "1.0.0", "acme/dg-ign": "1.0.0", "acme/dg-part": "1.0.0", "acme/dg-dot": "1.0.0", "acme/dg-unknown": "1.0.0"'
lock dg-a "{\"require\": {\"dg/composer-cleaner\": \"v2.2.2\", \"acme/dg-lib\": \"1.0.0\", $DG_PKGS}, $DG_EXTRA, $DG_CFG}"
lock dg-b "{\"require\": {\"dg/composer-cleaner\": \"v2.2.2\", \"acme/dg-lib\": \"1.0.1\", $DG_PKGS}, $DG_EXTRA, $DG_CFG}"
lock dg-c "{\"require\": {\"dg/composer-cleaner\": \"v2.2.2\", \"acme/dg-lib\": \"1.0.1\", \"acme/dg-root\": \"1.0.0\", \"acme/dg-part\": \"1.0.0\"}, $DG_EXTRA, $DG_CFG}"
run dg dg-a dg-b dg-c vendor/acme/dg-lib/src/A.php
grep -q '^Composer cleaner: Removed ' "$WORK/dg-install.composer.out" \
  || { echo "FAIL dg : pas de bilan chez Composer (cas mal construit)"; status=1; }
# Le plugin retiré : plus d'écouteur.
lock dg-gone "{\"require\": {\"acme/dg-lib\": \"1.0.1\", \"acme/dg-root\": \"1.0.0\"}, $DG_EXTRA, $DG_CFG}"
stage dg-gone "$WORK/dg-ref" "$WORK/dg-viv"; step "$WORK/dg-ref" "$WORK/dg-viv" dg-plugin-removal "${I[@]}"
# `update` (post-update-cmd, paquets de Packagist seulement), et
# cleaner-ignore dans config.
PACKAGIST_ONLY=1 lock dg-up "{\"require\": {\"dg/composer-cleaner\": \"v2.2.2\", \"psr/log\": \"3.0.2\", \"psr/container\": \"2.0.2\"}, \"extra\": {\"cleaner-ignore\": {\"psr/container\": [\"*.md\"]}}, $DG_CFG}"
fresh dg-up "$WORK/dg-up-ref" "$WORK/dg-up-viv"; step "$WORK/dg-up-ref" "$WORK/dg-up-viv" dg-update "${U[@]}"
lock dg-config "{\"require\": {\"dg/composer-cleaner\": \"v2.2.2\", \"acme/dg-lib\": \"1.0.0\", \"acme/dg-ign\": \"1.0.0\"}, \"config\": {\"cleaner-ignore\": {\"acme/dg-ign\": true}, \"allow-plugins\": {\"dg/composer-cleaner\": true}}}"
fresh dg-config "$WORK/dg-config-ref" "$WORK/dg-config-viv"; step "$WORK/dg-config-ref" "$WORK/dg-config-viv" dg-config "${I[@]}"
handed dg-a dg-verbose 'dg/composer-cleaner' "${I[@]}" -v
fresh dg-a "$WORK/dg-ni-ref"; (cd "$WORK/dg-ni-ref" && composer install --no-scripts --no-interaction --no-ansi -q)
handed_on "$WORK/dg-ni-ref" - dg-no-install 'dg/composer-cleaner' "${U[@]}" --no-install
lock dg-bad "{\"require\": {\"dg/composer-cleaner\": \"v2.2.2\", \"acme/dg-lib\": \"1.0.0\"}, \"extra\": {\"cleaner-ignore\": {\"acme/dg-lib\": false}}, $DG_CFG}"
handed dg-bad dg-bad 'dg/composer-cleaner' "${I[@]}"
relock dg-a dg-old dg/composer-cleaner v2.2.1
handed dg-old dg-old 'dg/composer-cleaner v2\.2\.1' "${I[@]}"

# --- liborm85/composer-vendor-cleaner 1.7.1 ---------------------------------
# Avec codeception/c3 (lignes à post-install-cmd) et l'autoloader optimisé :
# la classmap suit l'arbre nettoyé, aussi après le retrait du plugin.
LV_CFG='"config": {"optimize-autoloader": true, "allow-plugins": {"liborm85/composer-vendor-cleaner": true, "codeception/c3": true}}'
LV_FILES='"dev-files": {"/": ["tests/", "docs/", ".travis.yml"], "acme/lv-other": ["res/", "!res/keep.txt"], "bin": ["*.bat"]}'
LV_PKGS='"acme/lv-other": "1.0.0", "acme/lv-meta": "1.0.0", "acme/lv-bin": "1.0.0", "acme/lib": "1.0.0"'
lock lv-a "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"codeception/c3\": \"2.9.0\", \"acme/lv-lib\": \"1.0.0\", $LV_PKGS}, \"require-dev\": {\"acme/lv-dev\": \"1.0.0\"}, \"extra\": {$LV_FILES}, $LV_CFG}"
lock lv-b "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"codeception/c3\": \"2.9.0\", \"acme/lv-lib\": \"1.0.1\", $LV_PKGS}, \"require-dev\": {\"acme/lv-dev\": \"1.0.0\"}, \"extra\": {$LV_FILES}, $LV_CFG}"
lock lv-c "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"codeception/c3\": \"2.9.0\", \"acme/lv-lib\": \"1.0.1\", \"acme/lv-meta\": \"1.0.0\", \"acme/lv-bin\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"require-dev\": {\"acme/lv-dev\": \"1.0.0\"}, \"extra\": {$LV_FILES}, $LV_CFG}"
run lv lv-a lv-b lv-c vendor/acme/lv-lib/src/A.php
[ ! -e "$WORK/lv-ref/vendor/acme/lv-lib/src/tests" ] || { echo "FAIL lv : src/tests gardé chez Composer (cas mal construit)"; status=1; }
step "$WORK/lv-ref" "$WORK/lv-viv" lv-dump dump-autoload
step "$WORK/lv-ref" "$WORK/lv-viv" lv-dump-no-dev dump-autoload --no-dev
# Le plugin retiré : l'arbre reste nettoyé, la classmap le suit.
lock lv-gone "{\"require\": {\"codeception/c3\": \"2.9.0\", \"acme/lv-lib\": \"1.0.1\", \"acme/lv-bin\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"require-dev\": {\"acme/lv-dev\": \"1.0.0\"}, \"extra\": {$LV_FILES}, $LV_CFG}"
stage lv-gone "$WORK/lv-ref" "$WORK/lv-viv"; step "$WORK/lv-ref" "$WORK/lv-viv" lv-plugin-removal "${I[@]}"
step "$WORK/lv-ref" "$WORK/lv-viv" lv-gone-no-op "${I[@]}"
fresh lv-a "$WORK/lv-nd-ref" "$WORK/lv-nd-viv"; step "$WORK/lv-nd-ref" "$WORK/lv-nd-viv" lv-no-dev "${I[@]}" --no-dev
PACKAGIST_ONLY=1 lock lv-up "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"psr/log\": \"3.0.2\", \"psr/container\": \"2.0.2\"}, \"extra\": {\"dev-files\": {\"/\": [\"*.md\"], \"psr/log\": [\"src/\", \"!src/LoggerInterface.php\"]}}, \"config\": {\"allow-plugins\": {\"liborm85/composer-vendor-cleaner\": true}}}"
fresh lv-up "$WORK/lv-up-ref" "$WORK/lv-up-viv"; step "$WORK/lv-up-ref" "$WORK/lv-up-viv" lv-update "${U[@]}"
for v in 'match-case:false' 'remove-empty-dirs:false' 'no-dev-only:true'; do
  k="${v%%:*}"; val="${v#*:}"
  lock "lv-$k" "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"acme/lv-lib\": \"1.0.0\", $LV_PKGS}, \"require-dev\": {\"acme/lv-dev\": \"1.0.0\"}, \"extra\": {$LV_FILES}, \"config\": {\"dev-files\": {\"$k\": $val}, \"allow-plugins\": {\"liborm85/composer-vendor-cleaner\": true}}}"
  fresh "lv-$k" "$WORK/lv-$k-ref" "$WORK/lv-$k-viv"; step "$WORK/lv-$k-ref" "$WORK/lv-$k-viv" "lv-$k" "${I[@]}"
done
fresh lv-no-dev-only "$WORK/lv-ndo-ref" "$WORK/lv-ndo-viv"; step "$WORK/lv-ndo-ref" "$WORK/lv-ndo-viv" lv-no-dev-only-prod "${I[@]}" --no-dev
lock lv-ext "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"acme/lv-lib\": \"1.0.0\"}, \"extra\": {\"dev-files\": \"dev-files.json\"}, \"config\": {\"allow-plugins\": {\"liborm85/composer-vendor-cleaner\": true}}}"
handed lv-ext lv-ext 'liborm85/composer-vendor-cleaner' "${I[@]}"
lock lv-list "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"acme/lv-lib\": \"1.0.0\"}, \"extra\": {\"dev-files\": [\"tests/\"]}, \"config\": {\"allow-plugins\": {\"liborm85/composer-vendor-cleaner\": true}}}"
handed lv-list lv-list 'liborm85/composer-vendor-cleaner' "${I[@]}"
lock lv-binpat "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"acme/lv-bin\": \"1.0.0\"}, \"extra\": {\"dev-files\": {\"/\": [\"bin/\"]}}, \"config\": {\"allow-plugins\": {\"liborm85/composer-vendor-cleaner\": true}}}"
handed lv-binpat lv-binpat 'liborm85/composer-vendor-cleaner' "${I[@]}"
lock lv-binproxy "{\"require\": {\"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"acme/lv-bin\": \"1.0.0\"}, \"extra\": {\"dev-files\": {\"bin\": [\"lv\"]}}, \"config\": {\"allow-plugins\": {\"liborm85/composer-vendor-cleaner\": true}}}"
handed lv-binproxy lv-binproxy 'liborm85/composer-vendor-cleaner' "${I[@]}"
handed lv-a lv-verbose 'liborm85/composer-vendor-cleaner' "${I[@]}" -v
handed lv-a lv-no-autoloader 'liborm85/composer-vendor-cleaner' "${I[@]}" --no-autoloader
handed_on "$WORK/lv-up-ref" - lv-no-install 'liborm85/composer-vendor-cleaner' "${U[@]}" --no-install
relock lv-a lv-old liborm85/composer-vendor-cleaner 1.7.0
handed lv-old lv-old 'liborm85/composer-vendor-cleaner 1\.7\.0' "${I[@]}"

# --- cakephp/plugin-installer 2.0.2 -----------------------------------------
# Des plugins applicatifs sous plugins/ : à src (autoload racine ajouté),
# à tests (autoload-dev), un vendeur à deux niveaux, un répertoire sans src
# (gardé par post-autoload-dump seulement), un caché, un fichier, et un nom
# qu'un paquet porte aussi (le paquet l'emporte). Un nom numérique devient
# une clé entière : TypeError chez le plugin (`strict_types`), rendu.
ck_tree() {
  mkdir -p "$1/src" "$1/plugins/Foo/src" "$1/plugins/Foo/tests" "$1/plugins/Vendor/Bar/src" "$1/plugins/Legacy" "$1/plugins/.hidden/src" "$1/plugins/Acme/Plug/src"
  printf 'x\n' > "$1/plugins/Legacy/readme.txt"; printf 'x\n' > "$1/plugins/file.txt"
  printf '<?php\nnamespace Foo;\nclass Plugin {}\n' > "$1/plugins/Foo/src/Plugin.php"
}
CK='"autoload": {"psr-4": {"App\\": "src/"}}, "config": {"allow-plugins": {"cakephp/plugin-installer": true}}'
lock ck-a "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-plug\": \"1.0.0\", \"acme/ck-multi\": \"1.0.0\"}, \"require-dev\": {\"acme/ck-dev\": \"1.0.0\"}, $CK}"
lock ck-b "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-plug\": \"1.0.1\", \"acme/ck-multi\": \"1.0.0\"}, \"require-dev\": {\"acme/ck-dev\": \"1.0.0\"}, $CK}"
lock ck-c "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-plug\": \"1.0.1\"}, \"require-dev\": {\"acme/ck-dev\": \"1.0.0\"}, $CK}"
TREE=ck_tree
run ck ck-a ck-b ck-c vendor/cakephp-plugins.php
step "$WORK/ck-ref" "$WORK/ck-viv" ck-dump dump-autoload
step "$WORK/ck-ref" "$WORK/ck-viv" ck-dump-no-dev dump-autoload --no-dev
step "$WORK/ck-ref" "$WORK/ck-viv" ck-dump-o dump-autoload -o
fresh ck-a "$WORK/ck-nd-ref" "$WORK/ck-nd-viv"; step "$WORK/ck-nd-ref" "$WORK/ck-nd-viv" ck-no-dev "${I[@]}" --no-dev
fresh ck-a "$WORK/ck-v-ref" "$WORK/ck-v-viv"; step "$WORK/ck-v-ref" "$WORK/ck-v-viv" ck-verbose "${I[@]}" -v
lock ck-paths "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-plug\": \"1.0.0\"}, \"extra\": {\"plugin-paths\": [\"plugins/Vendor\", \"./plugins\"]}, $CK}"
fresh ck-paths "$WORK/ck-paths-ref" "$WORK/ck-paths-viv"; step "$WORK/ck-paths-ref" "$WORK/ck-paths-viv" ck-paths "${I[@]}"
lock ck-vdir "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-plug\": \"1.0.0\"}, \"autoload\": {\"psr-4\": {\"App\\\\\": \"src/\"}}, \"config\": {\"vendor-dir\": \"lib/vendor\", \"allow-plugins\": {\"cakephp/plugin-installer\": true}}}"
fresh ck-vdir "$WORK/ck-vdir-ref" "$WORK/ck-vdir-viv"; VENDOR_REL=lib/vendor step "$WORK/ck-vdir-ref" "$WORK/ck-vdir-viv" ck-vdir "${I[@]}"
ck_num() { ck_tree "$1"; mkdir -p "$1/plugins/7/src"; }
TREE=ck_num handed ck-a ck-num 'cakephp/plugin-installer' "${I[@]}"
lock ck-nons "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-nons\": \"1.0.0\"}, $CK}"
handed ck-nons ck-nons 'cakephp/plugin-installer' "${I[@]}"
lock ck-abs "{\"require\": {\"cakephp/plugin-installer\": \"2.0.2\", \"acme/ck-plug\": \"1.0.0\"}, \"extra\": {\"plugin-paths\": [\"/opt/plugins\"]}, $CK}"
handed ck-abs ck-abs 'cakephp/plugin-installer' "${I[@]}"
relock ck-a ck-old cakephp/plugin-installer 2.0.1
handed ck-old ck-old 'cakephp/plugin-installer 2\.0\.1' "${I[@]}"
TREE=

exit $status
