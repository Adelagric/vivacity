#!/usr/bin/env bash
# Isolation de vendor/ entre projets (docs/plans/vendor-isolation.md, P0).
#
# Composer ne partage jamais un fichier entre deux projets : une écriture en
# place dans vendor/ d'un projet ne change que ce projet. Jusqu'à 0.26.0,
# vivacity posait vendor/ par liens durs depuis son store hors de la copie
# sur écriture (ext4, overlayfs, HFS+, NTFS) : l'écriture passait dans le
# store, dans les autres projets, et dans tout projet installé ensuite.
#
# Critère (propriétés observables seulement, aucune technique affirmée) :
#   - A accepte l'écriture en place : nouveaux octets, même inode, le mode
#     que pose Composer ;
#   - B et l'entrée du store gardent les octets d'origine (sha256 contre une
#     extraction À PART de l'archive, pas contre le store, qui est testé).
# Ordres : `iso-before` (A et B installés, puis l'écriture dans A) et
# `iso-after` (A, l'écriture, puis B depuis le même store) ; chacun cache de
# classmap actif puis coupé (`VIVACITY_NO_CLASSMAP_CACHE=1` ne protège pas
# le store : les deux bras doivent se comporter pareil).
# `iso-upgrade` : A et B posés avant le correctif (liens durs vers une
# ancienne entrée), install sans opération du nouveau binaire, puis
# l'écriture dans A ; `iso-upgrade-edited` : l'ancienne entrée éditée avant
# la mise à jour — le contenu courant des projets est préservé (pas de
# « réparation »), le nouveau store porte les octets de l'archive, un projet
# installé ensuite aussi.
#
# Gardes :
#   - c'est vivacity qui installe : `--no-fallback`, code 0, sa ligne de
#     résumé, aucune ligne `delegating` (le paquet vient d'un dépôt
#     `artifact`, qu'`update` confie à Composer : on installe depuis un lock) ;
#   - A et B partagent un store : même VIVACITY_CACHE_DIR, l'entrée existe
#     avant l'install de B, qui la lit (« 1 from store ») ;
#   - le FS est celui qu'on croit : ISO_FS=link (défaut) exige qu'un lien dur
#     y soit possible — image HFS+ sous macOS, le répertoire de travail sous
#     Linux (ext4 sur les runners) ; ISO_FS=cow exige la copie sur écriture
#     (APFS, btrfs, xfs) et joue les mêmes contrôles d'octets.
#   - ISO_PHASE=red (binaire d'avant le correctif dans VIVACITY_BIN) :
#     reproduit le défaut historique. Exige en plus qu'un clone échoue sur
#     ce FS et qu'au départ le fichier de A partage son inode avec le store ;
#     les cas doivent alors échouer SUR LE CRITÈRE, gardes tenues.
#
# `iso-upgrade` pose l'état d'avant le correctif avec VIVACITY_OLD_BIN s'il
# est donné (un binaire ≤ 0.26.0), sinon le simule : install par le binaire
# testé dans un cache à part, puis chaque fichier du paquet remplacé par un
# lien dur vers une copie de l'entrée placée en `store/v3/` du vrai cache.
#
# Aucun réseau. Unix seulement. Usage : harness/vendor-isolation.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
OLD_BIN="${VIVACITY_OLD_BIN:-}"
PHASE="${ISO_PHASE:-green}"
FS="${ISO_FS:-link}"
BASE="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/vendor-isolation"
case "$PHASE" in red|green) ;; *) echo "ISO_PHASE=red|green"; exit 2 ;; esac
case "$FS" in link|cow) ;; *) echo "ISO_FS=link|cow"; exit 2 ;; esac
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
[ -z "$OLD_BIN" ] || [ -x "$OLD_BIN" ] || { echo "VIVACITY_OLD_BIN n'est pas exécutable"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication du zip)"; exit 1; }
OS="$(uname -s)"
case "$OS" in Darwin|Linux) ;; *) echo "SKIP : harnais Unix (liens durs et inodes)"; exit 0 ;; esac

# stat et sha256, GNU ou BSD. `stat -c` d'abord : sous GNU, `stat -f` existe
# aussi (infos du FS) et sort 0.
if stat -c '%a' . >/dev/null 2>&1; then
  mode_of() { stat -c '%a' "$1"; }
  ino_of() { stat -c '%d:%i' "$1"; }
  nlink_of() { stat -c '%h' "$1"; }
else
  mode_of() { stat -f '%Lp' "$1"; }
  ino_of() { stat -f '%d:%i' "$1"; }
  nlink_of() { stat -f '%l' "$1"; }
fi
if command -v sha256sum >/dev/null; then
  sha() { sha256sum | cut -d' ' -f1; }
else
  sha() { shasum -a 256 | cut -d' ' -f1; }
fi
sha_of() { sha < "$1"; }

fails=0; guards=0
ok() { echo "OK   $*"; }
fail() { echo "FAIL $*"; fails=$((fails + 1)); }
gfail() { echo "FAIL [garde] $*"; guards=$((guards + 1)); }
info() { echo "     $*"; }

# --- Le système de fichiers -------------------------------------------------
# Une image restée montée d'un run interrompu : détachée avant d'effacer.
[ "$OS" = Darwin ] && [ -d "$BASE/mnt" ] && hdiutil detach -force -quiet "$BASE/mnt" 2>/dev/null
rm -rf "$BASE"; mkdir -p "$BASE"
MNT=""
cleanup() {
  if [ -n "$MNT" ]; then
    hdiutil detach -quiet "$MNT" 2>/dev/null || hdiutil detach -force -quiet "$MNT" 2>/dev/null
    rm -f "$BASE/iso.dmg"
  fi
}
trap cleanup EXIT
if [ "$OS" = Darwin ] && [ "$FS" = link ]; then
  # HFS+ : ni clonefile ni reflink, des liens durs. `-owners on` : les modes
  # et propriétaires y sont tenus comme sur le disque système.
  made=""
  for _ in 1 2 3; do
    rm -f "$BASE/iso.dmg"
    if hdiutil create -quiet -fs HFS+ -size 64m -volname vivacity-iso "$BASE/iso.dmg" \
       && mkdir -p "$BASE/mnt" \
       && hdiutil attach -quiet -nobrowse -owners on -mountpoint "$BASE/mnt" "$BASE/iso.dmg"; then
      made=1; break
    fi
    sleep 2
  done
  [ -n "$made" ] || { rm -f "$BASE/iso.dmg"; echo "FAIL : image HFS+ impossible (hdiutil)"; exit 1; }
  MNT="$BASE/mnt"
  WORK="$MNT/w"
else
  WORK="$BASE/w"
fi
mkdir -p "$WORK/probe"
printf 'probe\n' > "$WORK/probe/a"
if ln "$WORK/probe/a" "$WORK/probe/b" 2>/dev/null; then
  ok "sonde : un lien dur est possible sur ce FS ($FS)"
else
  gfail "un lien dur est impossible sur ce FS : le cas ne prouverait rien"
fi
# Un clone (copie sur écriture) sur ce FS ?
if [ "$OS" = Darwin ]; then
  # `cp -c` se replie en copie ordinaire hors APFS : clonefile(2) directement.
  python3 -c 'import ctypes, sys; sys.exit(ctypes.CDLL(None, use_errno=True).clonefile(sys.argv[1].encode(), sys.argv[2].encode(), 0) != 0)' \
    "$WORK/probe/a" "$WORK/probe/c" 2>/dev/null && clones=1 || clones=0
else
  cp --reflink=always "$WORK/probe/a" "$WORK/probe/c" 2>/dev/null && clones=1 || clones=0
fi
if [ "$FS" = cow ]; then
  if [ "$clones" = 1 ]; then ok "sonde : ce FS copie sur écriture (ISO_FS=cow)"
  else gfail "ISO_FS=cow mais ce FS ne clone pas (APFS, btrfs ou xfs attendu)"; fi
elif [ "$PHASE" = red ]; then
  # Seulement pour reproduire le défaut : le binaire d'avant clone d'abord.
  if [ "$clones" = 0 ]; then ok "sonde (phase rouge) : un clone échoue sur ce FS, le binaire d'avant y lie"
  else gfail "phase rouge sur un FS qui clone : le binaire d'avant y isolerait déjà"; fi
else
  # Pas un critère : dit seulement quel chemin de vivacity le run exerce.
  if [ "$clones" = 1 ]; then info "ce FS clone : le run exerce le clone (copie sur écriture), pas la copie"
  else info "ce FS ne clone pas : le run exerce la copie"; fi
fi

# --- Le paquet, le lock, l'archive extraite à part, la référence Composer ----
REL="src/Iso.php"
ORIG='<?php
namespace Acme\Iso;

class Iso
{
}
'
NEW="${ORIG/class Iso/class Isx}"
# Plus long : écrit par `>` (troncature, même inode) dans `iso-after`.
NEW_LONG="${ORIG/class Iso/class IsoRewrittenInPlace}"
EDIT="${ORIG/class Iso/class Ise}"
mkdir -p "$WORK/artifacts" "$WORK/seed" "$WORK/extract"
php -r '
$z = new ZipArchive();
$z->open($argv[1] . "/acme-iso-1.0.0.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
$add = function (string $p, string $c, int $m) use ($z) {
    $z->addFromString("acme-iso/$p", $c);
    $z->setExternalAttributesName("acme-iso/$p", ZipArchive::OPSYS_UNIX, ($m | 0100000) << 16);
};
$add("composer.json", json_encode(["name" => "acme/iso", "version" => "1.0.0", "autoload" => ["psr-4" => ["Acme\\Iso\\" => "src/"]], "bin" => ["bin/iso"]], JSON_PRETTY_PRINT), 0644);
$add("src/Iso.php", $argv[2], 0640);
$add("bin/iso", "#!/bin/sh\necho iso\n", 0755);
$add("data/notes.txt", "notes\n", 0640);
$z->close();
' "$WORK/artifacts" "$ORIG" || { echo "FAIL : fabrication du zip"; exit 1; }
php -r '$z = new ZipArchive(); $z->open($argv[1]) === true && $z->extractTo($argv[2]) || exit(1);' \
  "$WORK/artifacts/acme-iso-1.0.0.zip" "$WORK/extract" || { echo "FAIL : extraction à part"; exit 1; }
SHA_ARCHIVE=$(sha_of "$WORK/extract/acme-iso/$REL")
SHA_NEW=$(printf '%s' "$NEW" | sha)
SHA_NEW_LONG=$(printf '%s' "$NEW_LONG" | sha)
SHA_EDIT=$(printf '%s' "$EDIT" | sha)
[ "$SHA_ARCHIVE" = "$(printf '%s' "$ORIG" | sha)" ] || { echo "FAIL : l'extraction ne rend pas le contenu écrit"; exit 1; }

cat > "$WORK/seed/composer.json" <<JSON
{
    "repositories": [{ "type": "artifact", "url": "$WORK/artifacts" }],
    "require": { "acme/iso": "^1.0" }
}
JSON
cenv() { env COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_NO_AUDIT=1 "$@"; }
if ! (cd "$WORK/seed" && cenv COMPOSER_CACHE_DIR="$WORK/seedcache" \
      composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >"$WORK/seed.log" 2>&1); then
  echo "FAIL : Composer n'a pas résolu le projet témoin"; sed -n '1,6p' "$WORK/seed.log"; exit 1
fi
grep -q '"acme/iso"' "$WORK/seed/composer.lock" || { echo "FAIL : acme/iso absent du lock"; exit 1; }
mkdir -p "$WORK/ref"; cp "$WORK/seed/composer.json" "$WORK/seed/composer.lock" "$WORK/ref/"
(cd "$WORK/ref" && cenv COMPOSER_CACHE_DIR="$WORK/refcache" \
  composer install --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/ref.err") \
  || { echo "FAIL : install de Composer"; exit 1; }
MODE_COMPOSER=$(mode_of "$WORK/ref/vendor/acme/iso/$REL")
# Les modes de tous les fichiers du paquet chez Composer (trois des quatre diffèrent
# du défaut d'un fichier neuf : une copie qui le reprendrait se voit).
modes_of() { (cd "$1" && find . -type f | sort | while read -r f; do echo "$f $(mode_of "$f")"; done); }
MODES_COMPOSER=$(modes_of "$WORK/ref/vendor/acme/iso")
info "archive : $SHA_ARCHIVE ; modes de Composer : $(echo "$MODES_COMPOSER" | tr '\n' ' ')"
[ "$MODE_COMPOSER" = 640 ] || { echo "FAIL : Composer pose $MODE_COMPOSER sur $REL, le zip stocke 640"; exit 1; }

# --- Briques ----------------------------------------------------------------
# viv <bin> <projet> <cache vivacity> <cache composer> <bras> : install depuis le lock.
viv() {
  local bin="$1" dir="$2" cache="$3" ccache="$4" arm="$5" code
  local nocache=(-u VIVACITY_NO_CLASSMAP_CACHE)
  [ "$arm" = off ] && nocache=(VIVACITY_NO_CLASSMAP_CACHE=1)
  (cd "$dir" && env "${nocache[@]}" COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main VIVACITY_CACHE_DIR="$cache" COMPOSER_CACHE_DIR="$ccache" \
    "$bin" install --no-scripts --no-fallback --no-interaction --no-ansi --optimize-autoloader \
    >"$dir.out" 2>"$dir.err")
  code=$?
  if [ "$code" != 0 ] || ! grep -q '^vivacity: .* installed, ' "$dir.err" || grep -qi 'delegating' "$dir.err"; then
    gfail "$(basename "$dir") : vivacity n'a pas installé lui-même (code $code)"; sed -n '1,6p' "$dir.err"
    return 1
  fi
  return 0
}
newproj() { mkdir -p "$1"; cp "$WORK/seed/composer.json" "$WORK/seed/composer.lock" "$1/"; }
# Les fichiers $REL des entrées de store (un par ligne).
store_files() { find "$1/store" -path "$1/store/.*" -prune -o -path "*/acme/iso/*/$REL" -type f -print 2>/dev/null | sort; }
# Écriture en place, hors de vivacity : même inode, pas de rename (`sed -i`
# remplacerait le fichier et ne prouverait rien). Même taille : `1<>` (ni
# troncature ni création) ; autre taille : `>` (troncature du même inode).
write_in_place() { printf '%s' "$2" 1<>"$1"; }
rewrite_in_place() { printf '%s' "$2" >"$1"; }
# check_modes <projet> <libellé> : modes de tous les fichiers du paquet.
check_modes() {
  if [ "$(modes_of "$1/vendor/acme/iso")" = "$MODES_COMPOSER" ]; then ok "$2 : modes de Composer"
  else fail "$2 : modes différents de ceux de Composer ($(modes_of "$1/vendor/acme/iso" | tr '\n' ' '))"; fi
}

# check_a <fichier de A> <inode avant l'écriture> <libellé> [sha écrit]
check_a() {
  local f="$1" ino="$2" tag="$3" want="${4:-$SHA_NEW}"
  if [ "$(sha_of "$f")" != "$want" ]; then fail "$tag : A ne porte pas les octets écrits"
  elif [ "$(ino_of "$f")" != "$ino" ]; then gfail "$tag : l'écriture dans A n'a pas eu lieu en place"
  elif [ "$(mode_of "$f")" != "$MODE_COMPOSER" ]; then fail "$tag : mode de A $(mode_of "$f"), Composer pose $MODE_COMPOSER"
  else ok "$tag : A accepte l'écriture en place (octets écrits, même inode, mode $MODE_COMPOSER)"; fi
}
# check_bytes <fichier> <sha attendu> <libellé>
check_bytes() {
  if [ ! -f "$1" ]; then fail "$3 : absent ($1)"
  elif [ "$(sha_of "$1")" = "$2" ]; then ok "$3 : octets attendus"
  else fail "$3 : octets changés"; fi
}

# --- iso-before / iso-after ---------------------------------------------------
run_order() {
  local order="$1" arm="$2" tag="iso-$1 (cache $2)"
  local d="$WORK/$order-$arm" cache ccache A B fa fb sf ino
  cache="$d/viv-cache"; ccache="$d/composer-cache"; A="$d/A"; B="$d/B"
  newproj "$A"; newproj "$B"
  viv "$VIVACITY" "$A" "$cache" "$ccache" "$arm" || return
  fa="$A/vendor/acme/iso/$REL"; fb="$B/vendor/acme/iso/$REL"
  sf=$(store_files "$cache")
  if [ "$(printf '%s\n' "$sf" | grep -c .)" != 1 ]; then gfail "$tag : une entrée de store attendue, trouvé : ${sf:-aucune}"; return; fi
  if [ "$PHASE" = red ] && [ "$FS" = link ]; then
    if [ "$(ino_of "$fa")" = "$(ino_of "$sf")" ] && [ "$(nlink_of "$fa")" -ge 2 ]; then
      ok "$tag : (phase rouge) A partage son inode avec le store"
    else gfail "$tag : (phase rouge) A ne partage pas son inode avec le store : montage déjà isolé"; fi
  fi
  ino=$(ino_of "$fa")
  if [ "$order" = before ]; then
    viv "$VIVACITY" "$B" "$cache" "$ccache" "$arm" || return
    write_in_place "$fa" "$NEW"
  else
    rewrite_in_place "$fa" "$NEW_LONG"
    [ -f "$sf" ] || { gfail "$tag : l'entrée de store a disparu avant B"; return; }
    viv "$VIVACITY" "$B" "$cache" "$ccache" "$arm" || return
  fi
  if grep -q '(1 from store, 0 from cache, 0 from network' "$B.err"; then ok "$tag : B lu depuis le store partagé"
  else gfail "$tag : B n'a pas été posé depuis le store ($(grep -o '([^)]*network' "$B.err"))"; fi
  if [ "$order" = before ]; then check_a "$fa" "$ino" "$tag"; else check_a "$fa" "$ino" "$tag" "$SHA_NEW_LONG"; fi
  check_bytes "$fb" "$SHA_ARCHIVE" "$tag : B"
  check_bytes "$sf" "$SHA_ARCHIVE" "$tag : store"
  check_modes "$A" "$tag : A"
  check_modes "$B" "$tag : B"
  info "$tag : nlink A=$(nlink_of "$fa") B=$(nlink_of "$fb") store=$(nlink_of "$sf")"
}

# --- iso-upgrade --------------------------------------------------------------
# Les projets A et B posés comme avant le correctif : liens durs vers une
# entrée d'ancien format, dans le cache que le nouveau binaire va lire.
old_layout() {
  local d="$1" cache="$2" ccache="$3" arm="$4" A="$1/A" B="$1/B" sim entry rel p
  if [ -n "$OLD_BIN" ]; then
    viv "$OLD_BIN" "$A" "$cache" "$ccache" "$arm" && viv "$OLD_BIN" "$B" "$cache" "$ccache" "$arm" || return 1
    return 0
  fi
  sim="$d/sim-cache"
  viv "$VIVACITY" "$A" "$sim" "$ccache" "$arm" && viv "$VIVACITY" "$B" "$sim" "$ccache" "$arm" || return 1
  entry=$(store_files "$sim"); entry="${entry%/"$REL"}"
  [ -d "$entry" ] || { gfail "simulation : entrée introuvable"; return 1; }
  local old
  old="$cache/store/v3/acme/iso/$(basename "$entry")"
  mkdir -p "$(dirname "$old")"; cp -Rp "$entry" "$old"
  (cd "$old" && find . -type f) | while read -r rel; do
    for p in "$A" "$B"; do ln -f "$old/$rel" "$p/vendor/acme/iso/$rel"; done
  done
}

run_upgrade() {
  local edited="$1" arm="$2" tag d cache ccache A B C fa fb old new before ino
  tag="iso-upgrade$([ "$edited" = 1 ] && echo -edited) (cache $arm)"
  d="$WORK/upgrade$edited-$arm"; cache="$d/viv-cache"; ccache="$d/composer-cache"
  A="$d/A"; B="$d/B"; C="$d/C"
  newproj "$A"; newproj "$B"; newproj "$C"
  old_layout "$d" "$cache" "$ccache" "$arm" || return
  fa="$A/vendor/acme/iso/$REL"; fb="$B/vendor/acme/iso/$REL"
  old=$(store_files "$cache")
  if [ "$(printf '%s\n' "$old" | grep -c .)" != 1 ]; then gfail "$tag : une ancienne entrée attendue, trouvé : ${old:-aucune}"; return; fi
  if [ "$FS" = link ] || [ -z "$OLD_BIN" ]; then
    if [ "$(ino_of "$fa")" = "$(ino_of "$old")" ] && [ "$(ino_of "$fb")" = "$(ino_of "$old")" ]; then
      ok "$tag : état d'avant le correctif posé (A, B et l'ancienne entrée liés)"
    else gfail "$tag : A, B et l'ancienne entrée ne sont pas liés : l'état d'avant n'est pas reproduit"; return; fi
  fi
  before="$SHA_ARCHIVE"
  if [ "$edited" = 1 ]; then
    write_in_place "$old" "$EDIT"; before="$SHA_EDIT"
    [ "$(sha_of "$fa")" = "$SHA_EDIT" ] && [ "$(sha_of "$fb")" = "$SHA_EDIT" ] \
      || { gfail "$tag : l'édition de l'ancienne entrée n'a pas atteint A et B"; return; }
  fi
  # Le nouveau binaire : un install sans opération dans A et dans B.
  viv "$VIVACITY" "$A" "$cache" "$ccache" "$arm" || return
  viv "$VIVACITY" "$B" "$cache" "$ccache" "$arm" || return
  grep -q '^vivacity: 0 installed, 1 unchanged' "$A.err" || gfail "$tag : l'install de A n'était pas sans opération"
  grep -q '^vivacity: 0 installed, 1 unchanged' "$B.err" || gfail "$tag : l'install de B n'était pas sans opération"
  check_bytes "$fa" "$before" "$tag : A garde son contenu courant après la mise à jour"
  check_bytes "$fb" "$before" "$tag : B garde son contenu courant après la mise à jour"
  new=$(store_files "$cache" | grep -vxF "$old")
  if [ "$(printf '%s\n' "$new" | grep -c .)" != 1 ]; then fail "$tag : une nouvelle entrée de store attendue, trouvé : ${new:-aucune}"; new=""
  else check_bytes "$new" "$SHA_ARCHIVE" "$tag : nouvelle entrée (depuis l'archive)"; fi
  ino=$(ino_of "$fa")
  write_in_place "$fa" "$NEW"
  check_a "$fa" "$ino" "$tag"
  check_bytes "$fb" "$before" "$tag : B après l'écriture dans A"
  check_bytes "$old" "$before" "$tag : ancienne entrée après l'écriture dans A"
  [ -z "$new" ] || check_bytes "$new" "$SHA_ARCHIVE" "$tag : nouvelle entrée après l'écriture dans A"
  check_modes "$A" "$tag : A"
  check_modes "$B" "$tag : B"
  # Un projet installé ensuite reçoit l'archive, jamais l'ancienne entrée.
  viv "$VIVACITY" "$C" "$cache" "$ccache" "$arm" || return
  check_bytes "$C/vendor/acme/iso/$REL" "$SHA_ARCHIVE" "$tag : C installé après"
  check_modes "$C" "$tag : C"
}

for arm in on off; do
  run_order before "$arm"
  run_order after "$arm"
  run_upgrade 0 "$arm"
  run_upgrade 1 "$arm"
done

echo "---"
echo "phase $PHASE, FS $FS ($OS) : $fails échec(s) sur le critère, $guards garde(s) non tenue(s)"
if [ "$PHASE" = red ]; then
  [ "$guards" = 0 ] || echo "phase rouge INVALIDE : une garde ne tient pas, l'échec ne prouve rien"
  [ "$guards" = 0 ] && [ "$fails" -gt 0 ] && echo "phase rouge reproduite : le défaut est vu sur le critère"
fi
[ "$fails" = 0 ] && [ "$guards" = 0 ]
