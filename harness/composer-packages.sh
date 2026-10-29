#!/usr/bin/env bash
# drupol/composer-packages 2.0.0 (docs/plans/duration-drupol-collector.md) :
# à post-autoload-dump, cinq classes générées sous
# vendor/drupol/composer-packages/build/ depuis le lock (dev compris) et
# deux lignes sur stdout. Un projet qui exige le vrai plugin (Packagist),
# composer/package-versions-deprecated (ses deux lignes au même événement)
# et des paquets synthétiques servis par un dépôt `artifact` (métapaquet,
# `target-dir`, cible de `require` à majuscules, type à caractères HTML,
# flottants et clés numériques dans `extra`, descriptions qui ressemblent
# à du PHP), avec `optimize-autoloader` : au no-op, les classes générées
# entrent dans la classmap.
#
# Composer et vivacity tournent AU MÊME CHEMIN ABSOLU, l'un après l'autre
# (les répertoires sont renommés entre les passages) : Directories.php
# porte les chemins d'installation absolus, sous `var_export` et sous
# l'échappement HTML de Twig (`&amp;`), et ce chemin contient `&` exprès.
# (Pas de `'` : vivacity écrit alors autoload_static.php en chemins absolus
# là où Composer écrit `__DIR__ . '/..'` — un écart de l'autoloader sans
# lien avec le plugin, noté au plan. Le `'` est exercé ailleurs : dans une
# description pour `var_export`, dans un type pour `&#039;`.) Par étape : code de sortie, stdout (brute), stderr (les
# lignes `Downloading`, les notes `vivacity:` et la durée de la résolution
# mises à part), et le projet entier — les cinq classes à l'octet près hors
# les chiffres de `GENERATED_AT` (harness/lib/compare.sh). Les cas que
# vivacity rend à Composer : code 3 sous --no-fallback, la raison, rien
# d'écrit.
#
# Usage : harness/composer-packages.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) echo "SKIP : vivacity rend drupol/composer-packages à Composer sous Windows"; exit 0 ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/composer-packages"
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
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION=1.0.0 VIVACITY_CACHE_DIR="$WORK/vivacity-cache"
unset COMPOSER_TESTS_ARE_RUNNING GITHUB_ACTIONS COMPOSER_NO_DEV
status=0
ART="$WORK/artifacts"
# Le chemin commun aux deux passages.
P="$WORK/a&b"

php -r '
$dir = $argv[1];
$make = function (string $name, string $version, string $type, array $files, array $more = []) use ($dir) {
    $root = str_replace("/", "-", strtolower($name));
    $json = ["name" => $name, "version" => $version, "type" => $type] + $more;
    $z = new ZipArchive();
    $z->open("$dir/$root-$version.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
    $z->addFromString("$root/composer.json", json_encode($json, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_PRESERVE_ZERO_FRACTION));
    $z->setExternalAttributesName("$root/composer.json", ZipArchive::OPSYS_UNIX, 0100644 << 16);
    foreach ($files + ["README.md" => "# $name $version\n"] as $path => $content) {
        $z->addFromString("$root/$path", $content);
        $z->setExternalAttributesName("$root/$path", ZipArchive::OPSYS_UNIX, 0100644 << 16);
    }
    $z->close();
};
$cls = fn (string $ns, string $c) => "<?php\nnamespace $ns;\nclass $c {}\n";
// Une description et un `extra` que la classmap relira dans Packages.php et
// Types.php : du texte qui ressemble à du PHP, des flottants aux formes de
// `var_export`, des clés que PHP rend entières.
$tricky = "class Fake {} ?> <?php /* not closed # comment <<<EOT \\\\ back\\\\slash '"'"'q'"'"' \"dq\" & <b>";
$extra = ["f" => 1.5, "big" => 1e20, "small" => 1.5e-7, "huge" => 18446744073709551615, "int" => 3,
    "keys" => ["0" => "zero", "12" => "twelve", "-3" => "minus", "07" => "lead", "1.5" => "dot"], "empty" => new stdClass(), "list" => [],
    "nested" => ["a" => ["b" => ["c" => null, "t" => true, "f" => false]]], "uni" => "été"];
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lib", $v, "library", ["src/Lib.php" => $cls("Acme\\Lib", "Lib")], [
        "description" => $tricky, "extra" => $extra, "autoload" => ["psr-4" => ["Acme\\Lib\\" => "src/"]],
        "require" => ["php" => ">=7.4", "ext-json" => "*", "composer-runtime-api" => "^2"],
    ]);
}
$make("acme/meta", "1.0.0", "metapackage", [], ["require" => ["acme/lib" => "*"]]);
$make("acme/target", "1.0.0", "library", ["Acme/Target/T.php" => "<?php\nclass Acme_Target_T {}\n"], ["target-dir" => "Acme/Target", "autoload" => ["psr-0" => ["Acme_Target_" => ""]]]);
// Une cible de `require` à majuscules (Dependencies la met en minuscules).
// (Un nom de paquet à majuscules : vivacity écrit installed.php sous le nom
// tel quel là où Composer le met en minuscules — un écart sans lien avec le
// plugin, noté au plan ; le cas est couvert par un test unitaire.)
$make("acme/caps", "1.0.0", "library", ["src/U.php" => $cls("Acme\\Caps", "U")], ["autoload" => ["psr-4" => ["Acme\\Caps\\" => "src/"]], "require" => ["ACME/Lib" => "^1.0"]]);
$make("acme/typed", "1.0.0", "o'"'"'type&<x>", ["src/T.php" => $cls("Acme\\Typed", "T")], ["autoload" => ["psr-4" => ["Acme\\Typed\\" => "src/"]]]);
$make("acme/dev-lib", "1.0.0", "library", ["src/D.php" => $cls("Acme\\Dev", "D")], ["autoload" => ["psr-4" => ["Acme\\Dev\\" => "src/"]], "require" => ["acme/lib" => "^1.0"]]);
$make("acme/numtype", "1.0.0", "123", [], []);
// `-0` (entier 0 pour PHP) et `-0.0` : indiscernables une fois lus.
$make("acme/negzero", "1.0.0", "library", [], ["extra" => ["z" => -0.0]]);
' "$ART" || { echo "FAIL : fabrication des zips"; exit 1; }

# lock <nom> <composer.json> : écrit par Composer sans plugins, nom racine
# `vivacity/composer-packages` (Composer refuse un nom racine à
# majuscules ; `NONAME=1` : sans nom). `PACKAGIST_ONLY` : sans le dépôt `artifact`
# (étapes `update` : vivacity ne résout pas un dépôt `artifact`).
lock() {
  local d="$WORK/locks/$1"
  rm -rf "$d"; mkdir -p "$d"
  local name='.name = "vivacity/composer-packages"'
  [ -z "${NONAME:-}" ] || name='del(.name)'
  if [ -n "${PACKAGIST_ONLY:-}" ]; then
    printf '%s\n' "$2" | jq "$name" > "$d/composer.json"
  else
    printf '%s\n' "$2" | jq --arg art "$ART" ".repositories = [{\"type\": \"artifact\", \"url\": \$art}] | $name" > "$d/composer.json"
  fi
  (cd "$d" && composer update --no-install --no-plugins --no-scripts --no-audit --no-security-blocking --no-interaction --no-ansi -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; exit 1; }
}
relock() { # source, nom, paquet, version
  local d="$WORK/locks/$2"
  rm -rf "$d"; mkdir -p "$d"
  jq --arg n "$3" --arg v "$4" '.require[$n] = $v' "$WORK/locks/$1/composer.json" > "$d/composer.json"
  jq --arg n "$3" --arg v "$4" '.packages |= map(if .name == $n then .version = $v else . end)' "$WORK/locks/$1/composer.lock" > "$d/composer.lock"
}
stage() { local src="$WORK/locks/$1"; shift; local d; for d in "$@"; do mkdir -p "$d"; cp "$src/composer.json" "$src/composer.lock" "$d/"; done; }
fresh() { local src="$1"; shift; local d; for d in "$@"; do rm -rf "$d"; done; stage "$src" "$@"; }

# step <ref> <viv> <nom> <args…> : Composer dans <ref>, vivacity dans <viv>,
# chacun déplacé en $P le temps de son passage.
step() {
  local ref="$1" viv="$2" name="$3"; shift 3
  local c_code=0 v_code=0
  rm -rf "$P"
  mv "$ref" "$P"
  (cd "$P" && composer "$@" --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  mv "$P" "$ref"; mv "$viv" "$P"
  local t0 t1; t0="$(date +%s)"
  (cd "$P" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
  t1="$(date +%s)"
  mv "$P" "$viv"
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -4 "$WORK/$name.vivacity.err"; status=1; return; fi
  # La comparaison masque les chiffres de GENERATED_AT : ceux de vivacity
  # doivent être l'heure de son passage, quand le plugin a tourné.
  if grep -q '^drupol/composer-packages: Done\.$' "$WORK/$name.vivacity.out"; then
    local c ts
    for c in Packages Types Directories Versions Dependencies; do
      ts="$(sed -n 's/^    public const GENERATED_AT = \([0-9]*\);$/\1/p' "$viv/vendor/drupol/composer-packages/build/$c.php")"
      if [ -z "$ts" ] || [ "$ts" -lt "$t0" ] || [ "$ts" -gt "$t1" ]; then
        echo "FAIL $name : GENERATED_AT de $c.php ($ts) hors du passage de vivacity ($t0–$t1)"; status=1; return
      fi
    done
  fi
  if ! diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >"$WORK/$name.out.diff"; then
    echo "FAIL $name : stdout différente"; head -8 "$WORK/$name.out.diff"; status=1; return
  fi
  local t='s/completed in [0-9.]* seconds/completed in <t> seconds/'
  if ! diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.err" | sed -e "$t") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.err" | sed -e "$t") >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -10 "$WORK/$name.err.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : code $c_code, stdout, stderr et projet identiques"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}
# handed <lock> <nom> <motif> <args…> : vivacity seul, --no-fallback, en $P.
handed() {
  local src="$1" name="$2" pat="$3"; shift 3
  rm -rf "$P"; stage "$src" "$P"
  handed_here "$name" "$pat" "$@"
}
# handed_on <projet installé> <lock|-> <nom> <motif> <args…> : sur une copie
# d'un projet que Composer a installé ; `-` garde son composer.json et son
# lock. `PREP` : une fonction appliquée à la copie avant le passage.
handed_on() {
  local from="$1" src="$2" name="$3" pat="$4"; shift 4
  rm -rf "$P"; cp -a "$from" "$P"; [ "$src" = - ] || stage "$src" "$P"
  if [ -n "${PREP:-}" ]; then "$PREP" "$P"; fi
  handed_here "$name" "$pat" "$@"
}
handed_here() {
  local name="$1" pat="$2" v=0; shift 2
  touch "$WORK/$name.marker"; sleep 1
  (cd "$P" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.all" 2>&1) || v=$?
  if [ "$v" = 3 ] && grep -qE -- "$pat" "$WORK/$name.vivacity.all" && [ -z "$(find "$P" -newer "$WORK/$name.marker" -print -quit)" ]; then
    echo "OK   $name : rendu à Composer, la raison, rien d'écrit"
  else
    echo "FAIL $name : code $v"; tail -4 "$WORK/$name.vivacity.all"; status=1
  fi
  rm -rf "$P"
}
# Composer seul (projet de départ d'un cas rendu), en $P puis déplacé.
composer_in() { # répertoire, args…
  local d="$1"; shift
  rm -rf "$P"; mv "$d" "$P"
  (cd "$P" && composer "$@" --no-interaction --no-ansi -q) || { echo "FAIL : Composer a échoué ($*)"; status=1; }
  mv "$P" "$d"
}
built() { # répertoire : les cinq classes chez Composer
  local c
  for c in Packages Types Directories Versions Dependencies; do
    [ -f "$1/vendor/drupol/composer-packages/build/$c.php" ] || { echo "FAIL : $c.php absent chez Composer (cas mal construit)"; status=1; return; }
  done
}

I=(install --no-scripts)
U=(update --no-scripts --no-audit --no-security-blocking)
CFG='"config": {"optimize-autoloader": true, "allow-plugins": {"drupol/composer-packages": true, "composer/package-versions-deprecated": true}}'
PLUG='"drupol/composer-packages": "2.0.0", "composer/package-versions-deprecated": "1.11.99.5"'
DEV='"require-dev": {"acme/dev-lib": "1.0.0"}'
lock dp-a "{\"require\": {$PLUG, \"acme/lib\": \"1.0.0\", \"acme/meta\": \"1.0.0\", \"acme/target\": \"1.0.0\", \"acme/caps\": \"1.0.0\", \"acme/typed\": \"1.0.0\"}, $DEV, $CFG}"
lock dp-b "{\"require\": {$PLUG, \"acme/lib\": \"1.0.1\", \"acme/meta\": \"1.0.0\", \"acme/target\": \"1.0.0\", \"acme/caps\": \"1.0.0\", \"acme/typed\": \"1.0.0\"}, $DEV, $CFG}"
lock dp-c "{\"require\": {$PLUG, \"acme/lib\": \"1.0.1\", \"acme/meta\": \"1.0.0\", \"acme/caps\": \"1.0.0\", \"acme/typed\": \"1.0.0\"}, $DEV, $CFG}"

ref="$WORK/dp-ref"; viv="$WORK/dp-viv"
fresh dp-a "$ref" "$viv"
step "$ref" "$viv" dp-install "${I[@]}"; built "$ref"
grep -q '^drupol/composer-packages: Done\.$' "$WORK/dp-install.composer.out" || { echo "FAIL : pas de ligne du plugin chez Composer (cas mal construit)"; status=1; }
step "$ref" "$viv" dp-no-op "${I[@]}"
grep -q 'ComposerPackages\\\\Packages' "$ref/vendor/composer/autoload_classmap.php" || { echo "FAIL : classes générées absentes de la classmap au no-op (cas mal construit)"; status=1; }
stage dp-b "$ref" "$viv"; step "$ref" "$viv" dp-update "${I[@]}"
stage dp-c "$ref" "$viv"; step "$ref" "$viv" dp-removal "${I[@]}"
step "$ref" "$viv" dp-dump dump-autoload
step "$ref" "$viv" dp-dump-o dump-autoload -o
step "$ref" "$viv" dp-dump-no-dev dump-autoload --no-dev
step "$ref" "$viv" dp-installed-no-plugins "${I[@]}" --no-plugins
step "$ref" "$viv" dp-installed-no-dev "${I[@]}" --no-dev
fresh dp-a "$ref" "$viv"; step "$ref" "$viv" dp-no-plugins "${I[@]}" --no-plugins
fresh dp-a "$ref" "$viv"; step "$ref" "$viv" dp-no-dev "${I[@]}" --no-dev; built "$ref"
fresh dp-a "$ref" "$viv"; step "$ref" "$viv" dp-verbose "${I[@]}" -v
# Sans nom racine : `__root__`.
NONAME=1 lock dp-noname "{\"require\": {$PLUG, \"acme/lib\": \"1.0.0\"}, $CFG}"
fresh dp-noname "$ref" "$viv"; step "$ref" "$viv" dp-noname "${I[@]}"
# Le plugin refusé : une bibliothèque.
lock dp-refused "{\"require\": {$PLUG, \"acme/lib\": \"1.0.0\"}, \"config\": {\"allow-plugins\": {\"drupol/composer-packages\": false, \"composer/package-versions-deprecated\": true}}}"
fresh dp-refused "$ref" "$viv"; step "$ref" "$viv" dp-refused "${I[@]}"
# `update` avec installation, `config.lock: false` (lock virtuel), puis
# `require -v` et `remove -v` (classes renommées `_composer_tmpN`) :
# paquets de Packagist seulement.
PACKAGIST_ONLY=1 lock dp-up "{\"require\": {$PLUG, \"psr/log\": \"3.0.2\"}, $CFG}"
fresh dp-up "$ref" "$viv"; step "$ref" "$viv" dp-update-cmd "${U[@]}"
step "$ref" "$viv" dp-require-v require psr/link:2.0.1 --no-scripts --no-audit --no-security-blocking -v
step "$ref" "$viv" dp-remove-v remove psr/link --no-scripts --no-audit --no-security-blocking -v
fresh dp-up "$ref" "$viv"
for d in "$ref" "$viv"; do rm -f "$d/composer.lock"; jq '.config.lock = false' "$d/composer.json" > "$d/c.json" && mv "$d/c.json" "$d/composer.json"; done
step "$ref" "$viv" dp-lock-false "${U[@]}"

# --- rendus -----------------------------------------------------------------
relock dp-a dp-old drupol/composer-packages 1.1.2
handed dp-old dp-old 'drupol/composer-packages 1\.1\.2' "${I[@]}"
relock dp-a dp-twig2 twig/twig v2.16.1
handed dp-twig2 dp-twig2 'drupol/composer-packages with twig/twig v2\.16\.1' "${I[@]}"
fresh dp-a "$WORK/dp-base"; composer_in "$WORK/dp-base" "${I[@]}"
# Le plugin chargé, retiré par le lock : son `uninstall`.
lock dp-gone "{\"require\": {\"composer/package-versions-deprecated\": \"1.11.99.5\", \"acme/lib\": \"1.0.0\"}, $CFG}"
handed_on "$WORK/dp-base" dp-gone dp-plugin-removal 'drupol/composer-packages is removed while loaded' "${I[@]}"
# dump-autoload sans lock, et avec le lock désactivé.
no_lock() { rm -f "$1/composer.lock"; }
PREP=no_lock handed_on "$WORK/dp-base" - dp-dump-no-lock 'drupol/composer-packages reads composer\.lock' dump-autoload
lock_off() { jq '.config.lock = false' "$1/composer.json" > "$1/c.json" && mv "$1/c.json" "$1/composer.json"; }
PREP=lock_off handed_on "$WORK/dp-base" - dp-dump-lock-false 'drupol/composer-packages reads composer\.lock' dump-autoload
# --run-scripts avec un script racine post-autoload-dump : rendu avant le
# script pre-install-cmd (qui écrirait un fichier, vu par `handed_here`).
with_script() { jq '.scripts["post-autoload-dump"] = ["@php -r \"echo 1;\""] | .scripts["pre-install-cmd"] = ["@php -r \"file_put_contents(\u0027marker\u0027, \u0027x\u0027);\""]' "$1/composer.json" > "$1/c.json" && mv "$1/c.json" "$1/composer.json"; }
PREP=with_script handed_on "$WORK/dp-base" - dp-run-scripts 'drupol/composer-packages would run again' install --run-scripts
PREP=with_script handed_on "$WORK/dp-base" - dp-dump-run-scripts 'drupol/composer-packages would run again' dump-autoload --run-scripts
# Un installeur (composer/installers actif) : `getInstallPath` d'autres installeurs.
lock dp-installers "{\"require\": {$PLUG, \"composer/installers\": \"2.3.0\", \"acme/lib\": \"1.0.0\"}, \"config\": {\"allow-plugins\": {\"drupol/composer-packages\": true, \"composer/package-versions-deprecated\": true, \"composer/installers\": true}}}"
handed dp-installers dp-installers 'drupol/composer-packages with the installer plugin composer/installers' "${I[@]}"
# Avec un nettoyeur.
lock dp-cleaner "{\"require\": {$PLUG, \"liborm85/composer-vendor-cleaner\": \"1.7.1\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"dev-files\": {\"/\": [\"tests/\"]}}, \"config\": {\"allow-plugins\": {\"drupol/composer-packages\": true, \"composer/package-versions-deprecated\": true, \"liborm85/composer-vendor-cleaner\": true}}}"
handed dp-cleaner dp-cleaner 'drupol/composer-packages with liborm85/composer-vendor-cleaner' "${I[@]}"
# Un type numérique (une clé entière de Types).
lock dp-numtype "{\"require\": {$PLUG, \"acme/numtype\": \"1.0.0\"}, $CFG}"
handed dp-numtype dp-numtype 'drupol/composer-packages .*numeric' "${I[@]}"
lock dp-negzero "{\"require\": {$PLUG, \"acme/negzero\": \"1.0.0\"}, $CFG}"
handed dp-negzero dp-negzero 'drupol/composer-packages .*negative zero' "${I[@]}"

exit $status
