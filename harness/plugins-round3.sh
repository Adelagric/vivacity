#!/usr/bin/env bash
# Les replis restants du corpus (docs/plans/remaining-fallbacks.md) :
# infection/extension-installer 0.1.2, typo3/class-alias-loader v2.0.1 et
# typo3/cms-composer-installers v5.0.2 (inerte sous la racine `typo3/cms`).
# Par plugin, un projet qui exige le vrai plugin à sa version exacte
# (Packagist) et des paquets synthétiques servis par un dépôt `artifact`
# (zips fabriqués par ZipArchive) ; les locks sont écrits par Composer
# `--no-plugins`.
#
# Étapes contre Composer 2.10.3 (plugins actifs) : install sur vendor vierge,
# no-op, mise à jour d'un paquet, retrait d'un paquet, `--no-plugins` sur
# l'arbre posé et sur un vendor vierge ; selon le plugin `dump-autoload`,
# `update`, `--no-dev`, `-v`. Par étape : code de sortie, stdout, stderr (les
# lignes `Downloading` et les notes `vivacity:` mises à part) et le projet
# entier (harness/lib/compare.sh — le suffixe aléatoire de class-alias-loader
# y est masqué ; après chaque no-op, ce harnais vérifie qu'il vaut celui
# d'autoload.php des deux côtés). Les configurations que vivacity rend à
# Composer : code 3 sous --no-fallback, la raison, rien d'écrit.
#
# Usage : harness/plugins-round3.sh   (VIVACITY_BIN : un autre binaire)
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/plugins-round3"
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
# retirer.
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
        $z->addFromString("$root/$path", $content);
        $z->setExternalAttributesName("$root/$path", ZipArchive::OPSYS_UNIX, 0100644 << 16);
    }
    $z->close();
};
$psr4 = fn (string $ns, string $dir = "src/") => ["autoload" => ["psr-4" => [$ns => $dir]]];
$cls = fn (string $ns, string $c) => "<?php\nnamespace $ns;\nclass $c {}\n";
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lib", $v, "library", [], ["src/Lib.php" => $cls("Acme\\Lib", "Lib")], $psr4("Acme\\Lib\\"));
    // infection/extension-installer : une extension valide, une sans `class`.
    $make("acme/inf-ext", $v, "infection-extension", ["infection" => ["class" => "Acme\\InfExt\\Extension", "x" => [1, 2.5, "s"]]], ["src/Extension.php" => $cls("Acme\\InfExt", "Extension")], $psr4("Acme\\InfExt\\"));
}
$make("acme/lib2", "1.0.0", "library", [], ["src/Lib2.php" => $cls("Acme\\Lib2", "Lib2")], $psr4("Acme\\Lib2\\"));
$make("acme/inf-bad", "1.0.0", "infection-extension", ["infection" => ["note" => "no class"]], []);
$make("acme/inf-bad2", "1.0.0", "infection-extension", ["infection" => "Acme\\Bad2"], []);
$make("acme/inf-other", "1.0.0", "library", ["infection" => ["class" => "Acme\\NotAnExtension"]], []);
// phpcodesniffer-standard : dealerdirect imprime sa ligne au même événement.
$make("acme/cs-std", "1.0.0", "phpcodesniffer-standard", [], ["AcmeStd/ruleset.xml" => "<?xml version=\"1.0\"?>\n<ruleset name=\"AcmeStd\"><description>x</description></ruleset>\n"]);
// typo3/class-alias-loader : un paquet qui déclare des cartes d alias.
$make("acme/aliased", "1.0.0", "library", ["typo3/class-alias-loader" => ["class-alias-maps" => ["Migrations/ClassAliasMap.php"]]], ["Migrations/ClassAliasMap.php" => "<?php\nreturn [\"Old\\\\Name\" => \"New\\\\Name\"];\n"]);
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
    printf '%s\n' "$2" | jq ${INDENT:-} '.name //= "vivacity/plugins-round3"' > "$d/composer.json"
  else
    printf '%s\n' "$2" | jq ${INDENT:-} --arg art "$ART" '.repositories = [{"type": "artifact", "url": $art}] | .name //= "vivacity/plugins-round3"' > "$d/composer.json"
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
  # (Sur l'arbre posé lui-même, pas sur une copie : les fichiers générés
  # portent le chemin absolu du projet.)
  step "$ref" "$viv" "$p-installed-no-plugins" "${args[@]}" --no-plugins
  ref="$WORK/$p-noplug-ref"; viv="$WORK/$p-noplug-viv"; fresh "$a" "$ref" "$viv"
  step "$ref" "$viv" "$p-no-plugins" "${args[@]}" --no-plugins
}


# suffix_matches <projet> : le suffixe de class-alias-loader (ses deux
# fichiers) égale celui de vendor/autoload.php — le cas déterministe du
# plugin, que compare.sh masque.
suffix_matches() {
  local d="$1" s a b
  s="$(sed -n 's/^return ComposerAutoloaderInit\([0-9a-zA-Z_]*\)::getLoader();$/\1/p' "$d/vendor/autoload.php")"
  a="$(sed -n 's/^class ClassAliasLoaderStaticInit\(.*\)$/\1/p' "$d/vendor/composer/autoload_classaliasmap_static.php")"
  b="$(sed -n 's/^.*ClassAliasLoaderStaticInit\(.*\)::\$aliasMap);$/\1/p' "$d/vendor/typo3/alias-loader-include.php")"
  [ -n "$s" ] && [ "$s" = "$a" ] && [ "$s" = "$b" ]
}
check_suffix() { # nom, ref, viv
  if suffix_matches "$2" && suffix_matches "$3"; then echo "OK   $1 : suffixe d'autoload.php des deux côtés"
  else echo "FAIL $1 : suffixe différent de celui d'autoload.php"; status=1; fi
}

# --- infection/extension-installer 0.1.2 --------------------------------------
# Avec dealerdirect/phpcodesniffer-composer-installer v1.2.1 (une ligne au
# même post-install-cmd, l'ordre d'activation entre les deux) et une
# extension invalide. À l'install vierge, une seule invalide : au-delà,
# Composer les liste dans l'ordre d'achèvement des extractions.
INF_CFG='"config": {"allow-plugins": {"infection/extension-installer": true, "dealerdirect/phpcodesniffer-composer-installer": true}}'
INF_CS='"dealerdirect/phpcodesniffer-composer-installer": "v1.2.1", "squizlabs/php_codesniffer": "^3.13", "acme/cs-std": "1.0.0"'
lock inf-a "{\"require\": {\"infection/extension-installer\": \"0.1.2\", $INF_CS, \"acme/inf-ext\": \"1.0.0\", \"acme/inf-bad\": \"1.0.0\", \"acme/inf-other\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $INF_CFG}"
lock inf-b "{\"require\": {\"infection/extension-installer\": \"0.1.2\", $INF_CS, \"acme/inf-ext\": \"1.0.1\", \"acme/inf-bad\": \"1.0.0\", \"acme/inf-other\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $INF_CFG}"
lock inf-c "{\"require\": {\"infection/extension-installer\": \"0.1.2\", $INF_CS, \"acme/inf-ext\": \"1.0.1\", \"acme/lib\": \"1.0.0\"}, $INF_CFG}"
run inf inf-a inf-b inf-c vendor/infection/extension-installer/src/GeneratedExtensionsConfig.php
grep -q 'Invalid extensions:' "$WORK/inf-install.composer.out" && grep -q 'PHP CodeSniffer Config' "$WORK/inf-install.composer.out" \
  || { echo "FAIL inf : lignes attendues absentes chez Composer (cas mal construit)"; status=1; }
# Deux invalides, au no-op seulement (ordre d'installed.json).
lock inf-two "{\"require\": {\"infection/extension-installer\": \"0.1.2\", \"acme/inf-bad\": \"1.0.0\", \"acme/inf-bad2\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, $INF_CFG}"
fresh inf-two "$WORK/inf-two-ref" "$WORK/inf-two-viv"
for d in "$WORK/inf-two-ref" "$WORK/inf-two-viv"; do (cd "$d" && composer install --no-scripts --no-plugins --no-interaction --no-ansi -q); done
step "$WORK/inf-two-ref" "$WORK/inf-two-viv" inf-two-no-op "${I[@]}"
# Sans extension : « No extensions found » (l'entrée roave du corpus).
lock inf-none "{\"require\": {\"acme/lib\": \"1.0.0\"}, \"require-dev\": {\"infection/extension-installer\": \"0.1.2\"}, $INF_CFG}"
run inf-none inf-none inf-none inf-none vendor/infection/extension-installer/src/GeneratedExtensionsConfig.php
fresh inf-none "$WORK/inf-nd-ref" "$WORK/inf-nd-viv"; step "$WORK/inf-nd-ref" "$WORK/inf-nd-viv" inf-no-dev "${I[@]}" --no-dev
# (`-v` sans dealerdirect, dont vivacity rend le `-v`.)
lock inf-v "{\"require\": {\"infection/extension-installer\": \"0.1.2\", \"acme/inf-ext\": \"1.0.0\", \"acme/inf-bad\": \"1.0.0\"}, $INF_CFG}"
fresh inf-v "$WORK/inf-v-ref" "$WORK/inf-v-viv"; step "$WORK/inf-v-ref" "$WORK/inf-v-viv" inf-verbose "${I[@]}" -v
step "$WORK/inf-v-ref" "$WORK/inf-v-viv" inf-verbose-no-op "${I[@]}" -v
# `update` (post-update-cmd), paquets de Packagist seulement.
PACKAGIST_ONLY=1 lock inf-up "{\"require\": {\"infection/extension-installer\": \"0.1.2\", \"psr/log\": \"3.0.2\"}, $INF_CFG}"
fresh inf-up "$WORK/inf-up-ref" "$WORK/inf-up-viv"; step "$WORK/inf-up-ref" "$WORK/inf-up-viv" inf-composer-update "${U[@]}"
step "$WORK/inf-up-ref" "$WORK/inf-up-viv" inf-update-no-install "${U[@]}" --no-install
relock inf-a inf-old infection/extension-installer 0.1.1
handed inf-old inf-old 'infection/extension-installer 0\.1\.1' "${I[@]}"
lock inf-alias "{\"require\": {\"infection/extension-installer\": \"0.1.2\", \"acme/inf-ext\": \"1.0.0 as 1.0.9\"}, $INF_CFG}"
handed inf-alias inf-alias 'acme/inf-ext 1\.0\.0 is aliased' "${I[@]}"
# --run-scripts : `composer run-script post-install-cmd` rejouerait l'écouteur.
lock inf-scripts "{\"require\": {\"infection/extension-installer\": \"0.1.2\", \"acme/lib\": \"1.0.0\"}, \"scripts\": {\"post-install-cmd\": \"@php -r 'echo 1;'\"}, $INF_CFG}"
handed inf-scripts inf-scripts 'infection/extension-installer would run again' install --run-scripts

# --- typo3/class-alias-loader v2.0.1 + typo3/cms-composer-installers v5.0.2 ----
# La racine `typo3/cms` (le dépôt TYPO3 lui-même) : cms-composer-installers
# n'y écrit rien (observé) ; class-alias-loader, `always-add-alias-loader`,
# écrit ses deux fichiers et ajoute le sien à l'autoload `files` de la
# racine, après ceux de la racine.
T3_EXTRA='"extra": {"typo3/class-alias-loader": {"always-add-alias-loader": true}, "typo3/cms": {"web-dir": "public"}}'
T3_ROOT='"name": "typo3/cms", "autoload": {"psr-4": {"App\\": "src/"}, "files": ["src/functions.php"]}'
T3_CFG='"config": {"allow-plugins": {"typo3/class-alias-loader": true, "typo3/cms-composer-installers": true}}'
T3_PLUG='"typo3/class-alias-loader": "v2.0.1", "typo3/cms-composer-installers": "v5.0.2"'
t3_tree() { mkdir -p "$1/src"; printf '<?php\n' > "$1/src/functions.php"; }
TREE=t3_tree
lock t3-a "{$T3_ROOT, \"require\": {$T3_PLUG, \"acme/lib\": \"1.0.0\", \"acme/lib2\": \"1.0.0\"}, $T3_EXTRA, $T3_CFG}"
lock t3-b "{$T3_ROOT, \"require\": {$T3_PLUG, \"acme/lib\": \"1.0.1\", \"acme/lib2\": \"1.0.0\"}, $T3_EXTRA, $T3_CFG}"
lock t3-c "{$T3_ROOT, \"require\": {$T3_PLUG, \"acme/lib\": \"1.0.1\"}, $T3_EXTRA, $T3_CFG}"
run t3 t3-a t3-b t3-c vendor/typo3/alias-loader-include.php
check_suffix t3-no-op-suffix "$WORK/t3-ref" "$WORK/t3-viv"
step "$WORK/t3-ref" "$WORK/t3-viv" t3-dump dump-autoload
step "$WORK/t3-ref" "$WORK/t3-viv" t3-dump-o dump-autoload -o
check_suffix t3-dump-suffix "$WORK/t3-ref" "$WORK/t3-viv"
fresh t3-a "$WORK/t3-nd-ref" "$WORK/t3-nd-viv"; step "$WORK/t3-nd-ref" "$WORK/t3-nd-viv" t3-no-dev "${I[@]}" --no-dev
# Sans `always-add-alias-loader` ni carte : le plugin retourne aussitôt.
lock t3-off "{$T3_ROOT, \"require\": {$T3_PLUG, \"acme/lib\": \"1.0.0\"}, $T3_CFG}"
fresh t3-off "$WORK/t3-off-ref" "$WORK/t3-off-viv"; step "$WORK/t3-off-ref" "$WORK/t3-off-viv" t3-inert "${I[@]}"
[ ! -e "$WORK/t3-off-ref/vendor/typo3/alias-loader-include.php" ] || { echo "FAIL t3-inert : fichier écrit chez Composer (cas mal construit)"; status=1; }
# Le suffixe de la configuration (déterministe), `prepend-autoloader` faux,
# pas d'autoload à la racine.
lock t3-cfg "{\"name\": \"typo3/cms\", \"require\": {$T3_PLUG, \"acme/lib\": \"1.0.0\"}, $T3_EXTRA, \"config\": {\"autoloader-suffix\": \"Fixed1\", \"prepend-autoloader\": false, \"allow-plugins\": {\"typo3/class-alias-loader\": true, \"typo3/cms-composer-installers\": true}}}"
fresh t3-cfg "$WORK/t3-cfg-ref" "$WORK/t3-cfg-viv"; step "$WORK/t3-cfg-ref" "$WORK/t3-cfg-viv" t3-config "${I[@]}"
grep -q 'ClassAliasLoaderStaticInitFixed1' "$WORK/t3-cfg-viv/vendor/typo3/alias-loader-include.php" || { echo "FAIL t3-config : suffixe de la configuration absent"; status=1; }
# class-alias-loader seul, cms-composer-installers refusé : même chose.
lock t3-cal "{\"name\": \"acme/app\", \"require\": {\"typo3/class-alias-loader\": \"v2.0.1\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"typo3/class-alias-loader\": {\"always-add-alias-loader\": \"1\"}}, \"config\": {\"allow-plugins\": {\"typo3/class-alias-loader\": true}}}"
run t3-cal t3-cal t3-cal t3-cal vendor/composer/autoload_classaliasmap_static.php
check_suffix t3-cal-suffix "$WORK/t3-cal-ref" "$WORK/t3-cal-viv"
TREE=
# Rendus.
lock t3-other "{\"name\": \"acme/site\", \"require\": {$T3_PLUG, \"acme/lib\": \"1.0.0\"}, $T3_EXTRA, $T3_CFG}"
handed t3-other t3-other-root 'typo3/cms-composer-installers would write vendor/typo3/autoload-include.php' "${I[@]}"
lock t3-maps "{$T3_ROOT, \"require\": {$T3_PLUG, \"acme/aliased\": \"1.0.0\"}, $T3_EXTRA, $T3_CFG}"
TREE=t3_tree handed t3-maps t3-maps 'class alias maps of acme/aliased' "${I[@]}"
TREE=t3_tree handed t3-a t3-verbose 'typo3/class-alias-loader v2\.0\.1 is active \(its listeners. lines: it writes a line' "${I[@]}" -v
lock t3-vdir "{\"name\": \"typo3/cms\", \"require\": {$T3_PLUG}, $T3_EXTRA, \"config\": {\"vendor-dir\": \"lib\", \"allow-plugins\": {\"typo3/class-alias-loader\": true, \"typo3/cms-composer-installers\": true}}}"
handed t3-vdir t3-vendor-dir 'typo3/class-alias-loader with config.vendor-dir' "${I[@]}"
lock t3-scripts "{\"name\": \"typo3/cms\", \"require\": {$T3_PLUG}, $T3_EXTRA, \"scripts\": {\"pre-autoload-dump\": \"@php -r 'echo 1;'\"}, $T3_CFG}"
handed t3-scripts t3-scripts 'typo3/class-alias-loader would run again' install --run-scripts
relock t3-a t3-old typo3/class-alias-loader v2.0.0
TREE=t3_tree handed t3-old t3-old 'typo3/class-alias-loader v2\.0\.0' "${I[@]}"
relock t3-a t3-cms-old typo3/cms-composer-installers v5.0.1
TREE=t3_tree handed t3-cms-old t3-cms-old 'typo3/cms-composer-installers v5\.0\.1' "${I[@]}"

exit $status
