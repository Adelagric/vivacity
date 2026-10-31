#!/usr/bin/env bash
# Revalidation du cache de classmap (docs/plans/vendor-isolation.md, P1).
#
# Contrat : sur l'état COURANT du projet, vendor/ compris, la classmap de
# vivacity est celle de Composer. Jusqu'à 0.26.0, un enregistrement du cache
# de classmap était tenu par entrée du store et servi sans relire vendor/ :
# une classe renommée, un fichier ajouté ou supprimé dans vendor/ restaient
# invisibles ; un enregistrement écrit depuis la copie éditée d'un projet
# servait les autres projets ; les sources du projet n'étaient revalidées que
# par (mtime, taille), que `touch -r` restaure.
#
# Chaque cas compare `vendor/composer/autoload_classmap.php`, à l'octet, à
# celui de `VIVACITY_NO_CLASSMAP_CACHE=1 vivacity dump-autoload -o` (la
# référence de justesse) et à celui de `composer dump-autoload -o` sur le
# même état du projet ; il vérifie aussi la classe attendue (garde : l'édition
# a bien eu lieu, Composer la voit). Avant l'édition, le projet est laissé
# « reposé » (plus de deux secondes après la pose) puis redumpé : le cache
# servirait tout, l'édition seule doit le faire relire.
#
#   cm-same-size-mtime   classe renommée dans vendor/, même taille, `touch -r`
#   cm-edit              classe renommée, taille et mtime changés
#   cm-added             fichier de classe ajouté dans un répertoire PSR-4
#   cm-removed           fichier de classe supprimé
#   cm-root-same-size    source du projet renommée, même taille, `touch -r`
#   cm-revert            édition puis retour au contenu d'origine
#   cm-cross-project     A posé sans -o, édité, `dump-autoload -o` ; B posé
#                        ensuite depuis le même store : classmap d'origine
#   cm-served            sans changement, le dump est servi par le cache
#                        (compteurs de VIVACITY_TRACE=1) ; un projet neuf posé
#                        depuis un store chaud sert ses paquets depuis
#                        l'enregistrement du store
#
# Gardes : c'est vivacity qui installe et qui dumpe (`--no-fallback`, code 0,
# sa ligne de résumé, ses lignes `trace:`, aucune ligne `delegating`).
#
# Aucun réseau (dépôt `artifact`, install depuis un lock). Unix seulement.
# Usage : harness/classmap-revalidate.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
BASE="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/classmap-revalidate"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication du zip)"; exit 1; }
case "$(uname -s)" in Darwin|Linux) ;; *) echo "SKIP : harnais Unix (touch -r, écriture en place)"; exit 0 ;; esac
# Plus que la fenêtre « racy » de vivacity (2 s) : un fichier posé avant est
# servi sur son identité seule.
SETTLE="${CM_SETTLE:-2.2}"

fails=0; guards=0
ok() { echo "OK   $*"; }
fail() { echo "FAIL $*"; fails=$((fails + 1)); }
gfail() { echo "FAIL [garde] $*"; guards=$((guards + 1)); }
info() { echo "     $*"; }

rm -rf "${BASE:?}"; mkdir -p "$BASE"
WORK="$BASE"

# --- Le paquet, le projet témoin, le lock ------------------------------------
STR='<?php
namespace Acme\Cm;

class Str
{
}
'
OTHER='<?php
namespace Acme\Cm;

class Other
{
}
'
DEEP='<?php
namespace Acme\Cm\Sub;

class Deep
{
}
'
LEGACY='<?php
class Acme_Cm_Legacy
{
}
'
COMMENT='<?php
namespace App;

class Comment
{
}
'
mkdir -p "$WORK/artifacts" "$WORK/seed/src"
php -r '
$z = new ZipArchive();
$z->open($argv[1] . "/acme-cm-1.0.0.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
$z->addFromString("acme-cm/composer.json", json_encode(["name" => "acme/cm", "version" => "1.0.0", "autoload" => ["psr-4" => ["Acme\\Cm\\" => "src/"], "classmap" => ["lib/"]]], JSON_PRETTY_PRINT));
$z->addFromString("acme-cm/src/Str.php", $argv[2]);
$z->addFromString("acme-cm/src/Other.php", $argv[3]);
$z->addFromString("acme-cm/src/Sub/Deep.php", $argv[4]);
$z->addFromString("acme-cm/lib/Legacy.php", $argv[5]);
$z->close();
$z = new ZipArchive();
$z->open($argv[1] . "/acme-noref-1.0.0.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
$z->addFromString("acme-noref/composer.json", json_encode(["name" => "acme/noref", "version" => "1.0.0", "autoload" => ["psr-4" => ["Acme\\NoRef\\" => "src/"]]], JSON_PRETTY_PRINT));
$z->addFromString("acme-noref/src/NoRef.php", "<?php\nnamespace Acme\\NoRef;\n\nclass NoRef\n{\n}\n");
$z->close();
' "$WORK/artifacts" "$STR" "$OTHER" "$DEEP" "$LEGACY" || { echo "FAIL : fabrication des zips"; exit 1; }
cat > "$WORK/seed/composer.json" <<JSON
{
    "repositories": [{ "type": "artifact", "url": "$WORK/artifacts" }],
    "require": { "acme/cm": "^1.0", "acme/noref": "^1.0" },
    "autoload": { "psr-4": { "App\\\\": "src/" } }
}
JSON
printf '%s' "$COMMENT" > "$WORK/seed/src/Comment.php"
cenv() { env COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_NO_AUDIT=1 "$@"; }
if ! (cd "$WORK/seed" && cenv COMPOSER_CACHE_DIR="$WORK/seedcache" \
      composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >"$WORK/seed.log" 2>&1); then
  echo "FAIL : Composer n'a pas résolu le projet témoin"; sed -n '1,6p' "$WORK/seed.log"; exit 1
fi
grep -q '"acme/cm"' "$WORK/seed/composer.lock" || { echo "FAIL : acme/cm absent du lock"; exit 1; }
# Un dépôt `artifact` ne donne pas de référence de dist : acme/cm en reçoit
# une (comme un paquet de Packagist, le chemin de l'enregistrement de store),
# acme/noref garde la sienne vide (le store la nomme par le sha1 de l'URL).
jq '(.packages[] | select(.name == "acme/cm") | .dist.reference) = "0123456789abcdef0123456789abcdef01234567"' \
  "$WORK/seed/composer.lock" > "$WORK/seed/lock.tmp" && mv "$WORK/seed/lock.tmp" "$WORK/seed/composer.lock" \
  || { echo "FAIL : référence de dist"; exit 1; }

# --- Briques ------------------------------------------------------------------
newproj() { mkdir -p "$1"; (cd "$WORK/seed" && tar -cf - composer.json composer.lock src) | (cd "$1" && tar -xf -); }
venv() { # venv <cache> <cmd…> : l'environnement de vivacity, trace comprise
  local cache="$1"; shift
  env -u VIVACITY_NO_CLASSMAP_CACHE COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main \
    VIVACITY_CACHE_DIR="$cache" COMPOSER_CACHE_DIR="$cache.composer" VIVACITY_TRACE=1 "$@"
}
# viv_install <projet> <cache> [-o]
viv_install() {
  local dir="$1" cache="$2" code
  shift 2
  (cd "$dir" && venv "$cache" "$VIVACITY" install --no-scripts --no-fallback --no-interaction --no-ansi "$@" \
    >"$dir.install.out" 2>"$dir.install.err")
  code=$?
  if [ "$code" != 0 ] || ! grep -q '^vivacity: .* installed, ' "$dir.install.err" || grep -qi 'delegating' "$dir.install.err"; then
    gfail "$(basename "$(dirname "$dir")")/$(basename "$dir") : vivacity n'a pas installé lui-même (code $code)"; sed -n '1,6p' "$dir.install.err"
    return 1
  fi
}
# viv_dump <projet> <cache> : `dump-autoload -o`, cache actif ; la classmap
# produite est gardée dans <projet>.viv.php, la trace dans <projet>.dump.err.
viv_dump() {
  local dir="$1" cache="$2" code
  (cd "$dir" && venv "$cache" "$VIVACITY" dump-autoload -o --no-fallback --no-ansi >"$dir.dump.out" 2>"$dir.dump.err")
  code=$?
  if [ "$code" != 0 ] || ! grep -q '^trace:   dump scan' "$dir.dump.err" || grep -qi 'delegating' "$dir.dump.err"; then
    gfail "$(basename "$dir") : vivacity n'a pas dumpé lui-même (code $code)"; sed -n '1,6p' "$dir.dump.err"
    return 1
  fi
  cp "$dir/vendor/composer/autoload_classmap.php" "$dir.viv.php"
}
# Les compteurs du cache de classmap du dernier dump (ou install) : la ligne
# `trace:   classmap cache …`, vide si le binaire n'en imprime pas.
counters() { grep -o 'classmap cache .*' "$1" | tail -1; }
counter() { counters "$1" | grep -o "[0-9]* $2" | cut -d' ' -f1; }

# compare <projet> <libellé> <classe attendue> <classe absente> [dump|install]
# Après viv_dump (ou un install -o) : la classmap du cache est comparée à
# celle du cache coupé puis à celle de Composer, sur le même état, et la
# sortie du dump à celle de Composer ; les deux classes, chez Composer,
# gardent le cas honnête (l'édition a eu lieu, Composer la voit). Une classe
# renommée dans un fichier PSR-4 n'y est plus conforme : Composer la retire
# sans ajouter la nouvelle.
compare() {
  local dir="$1" tag="$2" want="$3" gone="$4" code
  (cd "$dir" && env VIVACITY_NO_CLASSMAP_CACHE=1 COMPOSER_HOME="$WORK/home" VIVACITY_CACHE_DIR="$dir.nocache" \
    "$VIVACITY" dump-autoload -o --no-fallback --no-ansi >/dev/null 2>"$dir.off.err") \
    || { gfail "$tag : dump sans cache (code $?)"; return; }
  cp "$dir/vendor/composer/autoload_classmap.php" "$dir.off.php"
  (cd "$dir" && cenv COMPOSER_CACHE_DIR="$dir.ccache" composer dump-autoload -o --no-scripts --no-interaction --no-ansi \
    >"$dir.composer.out" 2>"$dir.composer.err") || { gfail "$tag : dump de Composer (code $?)"; return; }
  cp "$dir/vendor/composer/autoload_classmap.php" "$dir.composer.php"
  code=0
  if ! grep -qF "'$want' =>" "$dir.composer.php" || grep -qF "'$gone' =>" "$dir.composer.php"; then
    gfail "$tag : Composer ne voit pas l'état attendu ($want présent, $gone absent)"; code=1
  fi
  if cmp -s "$dir.viv.php" "$dir.off.php"; then ok "$tag : classmap identique au cache coupé"
  else fail "$tag : classmap différente du cache coupé ($(diff "$dir.off.php" "$dir.viv.php" | grep '^[<>]' | head -3 | tr '\n' ' '))"; code=1; fi
  if cmp -s "$dir.viv.php" "$dir.composer.php"; then ok "$tag : classmap identique à Composer"
  else fail "$tag : classmap différente de Composer ($(diff "$dir.composer.php" "$dir.viv.php" | grep '^[<>]' | head -3 | tr '\n' ' '))"; code=1; fi
  # La sortie du dump, sauf après un install (la commande n'est pas la même).
  if [ "${5:-dump}" = dump ]; then
    if cmp -s "$dir.dump.out" "$dir.composer.out"; then ok "$tag : sortie identique à Composer"
    else fail "$tag : sortie différente de Composer ($(diff "$dir.composer.out" "$dir.dump.out" | grep '^[<>]' | head -2 | tr '\n' ' '))"; code=1; fi
  fi
  return $code
}

# Écriture en place (même inode), même taille : `1<>` ne tronque pas.
write_same_size() { printf '%s' "$2" 1<>"$1"; }

# setup <cas> : A posé avec -o, laissé reposer, redumpé (le cache est chaud
# et l'état « reposé » enregistré). Pose D, CACHE, A.
setup() {
  D="$WORK/$1"; CACHE="$D/cache"; A="$D/A"
  newproj "$A"
  viv_install "$A" "$CACHE" -o || return 1
  sleep "$SETTLE"
  viv_dump "$A" "$CACHE" || return 1
}

PKG=vendor/acme/cm

# --- cm-same-size-mtime ----------------------------------------------------------
if setup cm-same-size-mtime; then
  f="$A/$PKG/src/Str.php"
  cp -p "$f" "$D/Str.orig"
  write_same_size "$f" "${STR/class Str/class Stx}"
  touch -r "$D/Str.orig" "$f"
  [ "$(wc -c < "$f")" = "$(wc -c < "$D/Str.orig")" ] || gfail "cm-same-size-mtime : la taille a changé"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-same-size-mtime" 'Acme\\Cm\\Other' 'Acme\\Cm\\Str'
fi

# --- cm-edit ------------------------------------------------------------------------
if setup cm-edit; then
  # Dans lib/ (règle `classmap`, sans filtre PSR) : le nouveau nom entre.
  printf '%s' "${LEGACY/class Acme_Cm_Legacy/class Acme_Cm_LegacyRenamed}" > "$A/$PKG/lib/Legacy.php"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-edit" 'Acme_Cm_LegacyRenamed' 'Acme_Cm_Legacy'
  # Dans src/ (PSR-4) : la classe renommée sort de la classmap.
  printf '%s' "${STR/class Str/class StrRenamedLonger}" > "$A/$PKG/src/Str.php"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-edit (psr-4)" 'Acme\\Cm\\Other' 'Acme\\Cm\\Str'
fi

# --- cm-added ------------------------------------------------------------------------
if setup cm-added; then
  printf '%s' "${OTHER/class Other/class Added}" > "$A/$PKG/src/Added.php"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-added" 'Acme\\Cm\\Added' 'Acme\\Cm\\Nothing'
fi

# --- cm-removed ----------------------------------------------------------------------
if setup cm-removed; then
  rm "$A/$PKG/src/Other.php"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-removed" 'Acme\\Cm\\Str' 'Acme\\Cm\\Other'
fi

# --- cm-root-same-size ---------------------------------------------------------------
if setup cm-root-same-size; then
  f="$A/src/Comment.php"
  cp -p "$f" "$D/Comment.orig"
  write_same_size "$f" "${COMMENT/class Comment/class Commenx}"
  touch -r "$D/Comment.orig" "$f"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-root-same-size" 'Acme\\Cm\\Str' 'App\\Comment'
fi

# --- cm-revert -------------------------------------------------------------------------
if setup cm-revert; then
  f="$A/$PKG/src/Str.php"
  cp -p "$f" "$D/Str.orig"
  write_same_size "$f" "${STR/class Str/class Stx}"
  touch -r "$D/Str.orig" "$f"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-revert (édité)" 'Acme\\Cm\\Other' 'Acme\\Cm\\Str'
  write_same_size "$f" "$STR"
  touch -r "$D/Str.orig" "$f"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-revert (rétabli)" 'Acme\\Cm\\Str' 'Acme\\Cm\\Stx'
fi

# --- cm-cross-project --------------------------------------------------------------------
# P1-c : A posé SANS -o (seule la règle `classmap` est scannée), sa copie
# éditée, puis `dump-autoload -o` dans A ; B, posé ensuite depuis le même
# store et jamais édité, doit garder `Str`.
D="$WORK/cm-cross-project"; CACHE="$D/cache"; A="$D/A"; B="$D/B"
newproj "$A"; newproj "$B"
if viv_install "$A" "$CACHE"; then
  f="$A/$PKG/src/Str.php"
  cp -p "$f" "$D/Str.orig"
  write_same_size "$f" "${STR/class Str/class Stx}"
  touch -r "$D/Str.orig" "$f"
  viv_dump "$A" "$CACHE" && compare "$A" "cm-cross-project (A édité)" 'Acme\\Cm\\Other' 'Acme\\Cm\\Str'
  if viv_install "$B" "$CACHE" -o; then
    if grep -q '(2 from store, ' "$B.install.err"; then ok "cm-cross-project : B posé depuis le store de A"
    else gfail "cm-cross-project : B n'a pas été posé depuis le store ($(grep -o '([^)]*network' "$B.install.err"))"; fi
    cp "$B/vendor/composer/autoload_classmap.php" "$B.viv.php"
    compare "$B" "cm-cross-project (B, install -o)" 'Acme\\Cm\\Str' 'Acme\\Cm\\Stx' install
    viv_dump "$B" "$CACHE" && compare "$B" "cm-cross-project (B, dump -o)" 'Acme\\Cm\\Str' 'Acme\\Cm\\Stx'
  fi
fi

# --- cm-served ---------------------------------------------------------------------------
# Sans changement, le cache sert encore : tout fichier scanné est servi sur
# son identité, rien n'est relu ni rescanné. Un projet neuf posé depuis le
# store chaud prend ses paquets dans l'enregistrement du store, sans relire
# sa copie.
if setup cm-served; then
  viv_dump "$A" "$CACHE"
  c=$(counters "$A.dump.err")
  if [ -z "$c" ]; then fail "cm-served : pas de compteurs du cache de classmap dans la trace"
  elif [ "$(counter "$A.dump.err" served)" = 6 ] && [ "$(counter "$A.dump.err" read)" = 0 ] \
       && [ "$(counter "$A.dump.err" scanned)" = 0 ]; then
    ok "cm-served : dump sans changement servi par le cache ($c)"
  else fail "cm-served : le cache ne sert pas tout ($c)"; fi
  B="$D/B"; newproj "$B"
  if viv_install "$B" "$CACHE" -o; then
    c=$(counters "$B.install.err")
    # Les 5 fichiers des paquets depuis le store (acme/noref compris : sans
    # référence de dist, le store le nomme quand même) ; Comment.php (sources
    # du projet, jamais vues dans ce chemin) scanné.
    if [ "$(counter "$B.install.err" 'from store')" = 5 ] && [ "$(counter "$B.install.err" scanned)" = 1 ]; then
      ok "cm-served : projet neuf servi par l'enregistrement du store ($c)"
    else fail "cm-served : projet neuf, compteurs inattendus (${c:-aucun})"; fi
    cp "$B/vendor/composer/autoload_classmap.php" "$B.viv.php"
    compare "$B" "cm-served (B, install -o)" 'Acme\\Cm\\Str' 'Acme\\Cm\\Stx' install
  fi
fi

echo
if [ "$fails" = 0 ] && [ "$guards" = 0 ]; then echo "classmap-revalidate : tout vert"; exit 0; fi
echo "classmap-revalidate : $fails échec(s) sur le critère, $guards garde(s)"
exit 1
