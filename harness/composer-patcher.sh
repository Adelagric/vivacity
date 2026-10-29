#!/usr/bin/env bash
# mlocati/composer-patcher 1.2.6 (docs/plans/remaining-fallbacks.md) : les
# correctifs d'un paquet fournisseur appliqués par `patch` aux paquets
# installés, `extra.patches_applied` dans installed.json. Le vrai plugin
# (Packagist) et des paquets synthétiques servis par un dépôt `artifact` ;
# les locks écrits par Composer `--no-plugins`.
#
# Étapes contre Composer 2.10.3 : install vierge, no-op, mise à jour d'une
# cible (le correctif ne la vise plus), retrait d'une cible, `--no-plugins`
# (posé, vierge), `--no-dev`, un correctif qui ajoute une classe (no-op
# `-o`, `dump-autoload -o` : la classmap suit les fichiers corrigés), un
# correctif qui échoue. Par étape : code, stdout, stderr, projet entier.
# Tolérances de stdout, propres à ce plugin : le répertoire temporaire du
# plugin (`PCH…`, dans les messages d'échec de `patch`) ramené à un
# marqueur ; à l'install vierge seulement, les lignes « Applying patch »
# triées par paquet visé (l'ordre interne au paquet gardé) — Composer
# parcourt alors son dépôt local dans l'ordre d'achèvement des extractions.
# Après chaque install, l'entrée du store de vivacity reste sans correctif.
# Rendus : code 3 sous --no-fallback, la raison, rien d'écrit.
#
# Usage : harness/composer-patcher.sh   (VIVACITY_BIN : un autre binaire)
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
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/composer-patcher"
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



command -v patch >/dev/null || { echo "patch requis"; exit 1; }
patch --version 2>&1 | head -1
# Les paquets synthétiques.
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
$lib = fn (string $v) => "<?php\nnamespace Acme\\Lib;\nclass Lib\n{\n    public function a() { return 1; }\n    public function v() { return \"$v\"; }\n}\n";
foreach (["1.0.0", "1.0.1"] as $v) {
    $make("acme/lib", $v, "library", [], ["src/Lib.php" => $lib($v)], ["autoload" => ["psr-4" => ["Acme\\Lib\\" => "src/"]]]);
}
$make("acme/lib2", "1.0.0", "library", [], ["lib2.txt" => "one\ntwo\nthree\n"]);
$make("acme/cm", "1.0.0", "library", [], ["lib/Base.php" => "<?php\nclass AcmeCmBase {}\n"], ["autoload" => ["classmap" => ["lib/"]]]);
$pLib = "--- a/src/Lib.php\n+++ b/src/Lib.php\n@@ -2,5 +2,5 @@\n namespace Acme\\Lib;\n class Lib\n {\n-    public function a() { return 1; }\n+    public function a() { return 2; }\n     public function v() { return \"1.0.0\"; }\n";
$pLib2 = "--- lib2.txt\n+++ lib2.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
$pLib2b = "--- lib2.txt\n+++ lib2.txt\n@@ -1,3 +1,3 @@\n-one\n+ONE\n TWO\n three\n";
$pCm = "--- /dev/null\n+++ b/lib/Added.php\n@@ -0,0 +1,2 @@\n+<?php\n+class AcmeCmAdded {}\n";
$bad = "--- a/src/Lib.php\n+++ b/src/Lib.php\n@@ -3,3 +3,3 @@\n class Lib\n {\n-    public function zz() { return 1; }\n+    public function zz() { return 3; }\n";
$patches = [
    "acme/lib:1.0.0" => ["Make a return 2" => "p/lib.patch"],
    "acme/lib2" => ["Shout two" => ["path" => "p/lib2.patch", "levels" => ["-p1", "-p0"]], "Shout one" => "p/lib2b.patch"],
    "acme/lib:^2" => ["Never" => "p/lib.patch"],
    "acme/cm" => ["Add a class" => "p/cm.patch"],
];
$files = ["p/lib.patch" => $pLib, "p/lib2.patch" => $pLib2, "p/lib2b.patch" => $pLib2b, "p/cm.patch" => $pCm];
$make("acme/patches", "1.0.0", "library", ["patches" => $patches], $files);
$make("acme/patches-bad", "1.0.0", "library", ["patches" => ["acme/lib" => ["Broken" => "p/bad.patch"]]], ["p/bad.patch" => $bad]);
$make("acme/patches-remote", "1.0.0", "library", ["patches" => ["acme/lib" => ["Far" => "https://example.invalid/x.patch"]]], []);
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
    printf '%s\n' "$2" | jq ${INDENT:-} '.name //= "vivacity/patcher-project"' > "$d/composer.json"
  else
    printf '%s\n' "$2" | jq ${INDENT:-} --arg art "$ART" '.repositories = [{"type": "artifact", "url": $art}] | .name //= "vivacity/patcher-project"' > "$d/composer.json"
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



# Les tolérances de stdout propres au plugin (voir l'en-tête) : `SORTED=1`
# trie les lignes « Applying patch » par paquet visé.
norm_out() {
  local s='s#PCH[A-Za-z0-9]+#PCH<tmp>#g'
  if [ -n "${SORTED:-}" ]; then
    sed -E "$s" "$1" | awk '/^Applying patch /{ i = index($0, " to "); r = substr($0, i + 4); k = substr(r, 1, index(r, "...") - 1); printf "%s\t%08d\t%s\n", k, NR, $0; next } { printf "~\t%08d\t%s\n", NR, $0 }' | sort -t "$(printf '\t')" -k1,1 -k2,2n | cut -f3-
  else
    sed -E "$s" "$1"
  fi
}
pstep() { # ref, viv, nom, args…
  local ref="$1" viv="$2" name="$3"; shift 3
  local c_code=0 v_code=0
  (cd "$ref" && composer "$@" --no-interaction --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" "$@" --no-fallback >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -4 "$WORK/$name.vivacity.err"; status=1; return; fi
  if ! diff <(norm_out "$WORK/$name.composer.out") <(norm_out "$WORK/$name.vivacity.out") >"$WORK/$name.out.diff"; then
    echo "FAIL $name : stdout différente"; head -8 "$WORK/$name.out.diff"; status=1; return
  fi
  local rr vr; rr="$(cd "$ref" && pwd -P)"; vr="$(cd "$viv" && pwd -P)"
  if ! diff <(grep -v '^  - Downloading ' "$WORK/$name.composer.err" | sed -e "s#$rr#<project>#g" -e "s#$ref#<project>#g") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.err" | sed -e "s#$vr#<project>#g" -e "s#$viv#<project>#g") >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -10 "$WORK/$name.err.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : code $c_code, stdout, stderr et projet identiques"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}
# L'entrée du store de vivacity n'a reçu aucun correctif : aucune copie de
# Lib.php, lib2.txt ou Added.php corrigée sous le store.
store_clean() { # nom
  local hits
  hits="$(grep -rlE 'return 2;|^TWO$|^ONE$|AcmeCmAdded' "$VIVACITY_CACHE_DIR/store" 2>/dev/null | grep -v '\.patch$' || true)"
  if [ -z "$hits" ]; then echo "OK   $1 : store intact"; else echo "FAIL $1 : store corrigé"; echo "$hits" | head -3; status=1; fi
}

PT_CFG='"config": {"allow-plugins": {"mlocati/composer-patcher": true}}'
PT_EXTRA='"extra": {"allow-subpatches": ["acme/patches"]}'
lock pt-a "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches\": \"1.0.0\", \"acme/lib\": \"1.0.0\", \"acme/lib2\": \"1.0.0\"}, \"require-dev\": {\"acme/cm\": \"1.0.0\"}, $PT_EXTRA, $PT_CFG}"
lock pt-b "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches\": \"1.0.0\", \"acme/lib\": \"1.0.1\", \"acme/lib2\": \"1.0.0\"}, \"require-dev\": {\"acme/cm\": \"1.0.0\"}, $PT_EXTRA, $PT_CFG}"
lock pt-c "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches\": \"1.0.0\", \"acme/lib\": \"1.0.1\"}, \"require-dev\": {\"acme/cm\": \"1.0.0\"}, $PT_EXTRA, $PT_CFG}"
ref="$WORK/pt-ref"; viv="$WORK/pt-viv"
fresh pt-a "$ref" "$viv"
SORTED=1 pstep "$ref" "$viv" pt-install "${I[@]}"
grep -q 'Applying patch acme/patches/Add a class to acme/cm... done.' "$WORK/pt-install.composer.out" && [ -f "$ref/vendor/acme/cm/lib/Added.php" ] \
  || { echo "FAIL pt : correctifs non appliqués chez Composer (cas mal construit)"; status=1; }
store_clean pt-install-store
pstep "$ref" "$viv" pt-no-op "${I[@]}"
pstep "$ref" "$viv" pt-no-op-o "${I[@]}" -o
grep -q AcmeCmAdded "$viv/vendor/composer/autoload_classmap.php" || { echo "FAIL pt-no-op-o : classe ajoutée absente de la classmap"; status=1; }
pstep "$ref" "$viv" pt-dump-o dump-autoload -o
stage pt-b "$ref" "$viv"; pstep "$ref" "$viv" pt-update "${I[@]}"
stage pt-c "$ref" "$viv"; pstep "$ref" "$viv" pt-removal "${I[@]}"
pstep "$ref" "$viv" pt-installed-no-plugins "${I[@]}" --no-plugins
fresh pt-a "$WORK/pt-np-ref" "$WORK/pt-np-viv"; pstep "$WORK/pt-np-ref" "$WORK/pt-np-viv" pt-no-plugins "${I[@]}" --no-plugins
fresh pt-a "$WORK/pt-nd-ref" "$WORK/pt-nd-viv"; SORTED=1 pstep "$WORK/pt-nd-ref" "$WORK/pt-nd-viv" pt-no-dev "${I[@]}" --no-dev
store_clean pt-no-dev-store
# Le plugin activé sur un projet déjà installé sans lui : pre (rien à
# collecter, il n'est pas encore chargé), puis les correctifs.
fresh pt-a "$WORK/pt-late-ref" "$WORK/pt-late-viv"
for d in "$WORK/pt-late-ref" "$WORK/pt-late-viv"; do (cd "$d" && composer install --no-scripts --no-plugins --no-interaction --no-ansi -q); done
pstep "$WORK/pt-late-ref" "$WORK/pt-late-viv" pt-late "${I[@]}"
# Un correctif qui échoue : la ligne d'erreur, pas de patches_applied.
lock pt-bad "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches-bad\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"allow-subpatches\": true}, $PT_CFG}"
fresh pt-bad "$WORK/pt-bad-ref" "$WORK/pt-bad-viv"; pstep "$WORK/pt-bad-ref" "$WORK/pt-bad-viv" pt-failing "${I[@]}"
grep -q 'Unable to apply the patch "Broken"' "$WORK/pt-failing.composer.out" || { echo "FAIL pt-failing : pas d'échec chez Composer (cas mal construit)"; status=1; }
# Aucun correctif à collecter.
lock pt-none "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/lib\": \"1.0.0\"}, $PT_CFG}"
fresh pt-none "$WORK/pt-none-ref" "$WORK/pt-none-viv"; pstep "$WORK/pt-none-ref" "$WORK/pt-none-viv" pt-none "${I[@]}"
pstep "$WORK/pt-none-ref" "$WORK/pt-none-viv" pt-none-no-op "${I[@]}"

# Rendus.
handed pt-a pt-verbose 'mlocati/composer-patcher 1\.2\.6 is active \(its listeners. lines: it writes a line per package' "${I[@]}" -v
lock pt-file "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"patches-file\": \"patches.json\"}, $PT_CFG}"
handed pt-file pt-patches-file 'with extra.patches-file' "${I[@]}"
lock pt-remote "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches-remote\": \"1.0.0\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"allow-subpatches\": true}, $PT_CFG}"
handed pt-remote pt-remote 'is downloaded from https://' "${I[@]}"
lock pt-root "{\"name\": \"acme/site\", \"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/lib\": \"1.0.0\"}, \"extra\": {\"patches\": {\"acme/site\": {\"Self\": \"self.patch\"}}}, $PT_CFG}"
handed pt-root pt-root-target 'would patch the root package acme/site' "${I[@]}"
lock pt-alias "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches\": \"1.0.0\", \"acme/lib\": \"1.0.0 as 1.0.5\", \"acme/lib2\": \"1.0.0\"}, $PT_EXTRA, $PT_CFG}"
handed pt-alias pt-alias 'aliased in the local repository' "${I[@]}"
lock pt-scripts "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/lib\": \"1.0.0\"}, \"scripts\": {\"post-install-cmd\": \"@php -r 'echo 1;'\"}, $PT_CFG}"
handed pt-scripts pt-scripts 'mlocati/composer-patcher would run again' install --run-scripts
relock pt-a pt-old mlocati/composer-patcher 1.2.5
handed pt-old pt-old 'mlocati/composer-patcher 1\.2\.5' "${I[@]}"
# Les correctifs d'un paquet changés après coup (un correctif de plus à la
# racine) : le pre-install-cmd désinstallerait la cible.
pt_root_patch() { printf -- '--- lib2.txt\n+++ lib2.txt\n@@ -1,3 +1,3 @@\n one\n TWO\n-three\n+THREE\n' > "$1/root.patch"; }
lock pt-more "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"acme/patches\": \"1.0.0\", \"acme/lib\": \"1.0.0\", \"acme/lib2\": \"1.0.0\"}, \"require-dev\": {\"acme/cm\": \"1.0.0\"}, \"extra\": {\"allow-subpatches\": [\"acme/patches\"], \"patches\": {\"acme/lib2\": {\"Root three\": \"root.patch\"}}}, $PT_CFG}"
fresh pt-a "$WORK/pt-more-ref"; (cd "$WORK/pt-more-ref" && composer install --no-scripts --no-interaction --no-ansi -q)
TREE=pt_root_patch handed_on "$WORK/pt-more-ref" pt-more pt-changed 'would uninstall acme/lib2' "${I[@]}"
# `update` (pre-update-cmd et post-update-cmd), paquets de Packagist seulement.
PACKAGIST_ONLY=1 lock pt-up "{\"require\": {\"mlocati/composer-patcher\": \"1.2.6\", \"psr/log\": \"3.0.2\"}, $PT_CFG}"
fresh pt-up "$WORK/pt-up-ref"; (cd "$WORK/pt-up-ref" && composer install --no-scripts --no-interaction --no-ansi -q)
handed_on "$WORK/pt-up-ref" - pt-update-cmd 'mlocati/composer-patcher listens to pre-update-cmd' "${U[@]}"

exit $status
