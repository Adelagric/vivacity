#!/usr/bin/env bash
# La famille « `unzip` échoue puis `ZipArchive` reprend ».
#
# `ZipDownloader::extractWithSystemUnzip` lance `unzip -qq <fichier> -d
# <chemin>` **sans `-o`** (ZipDownloader.php:51). Tout code de sortie non nul
# déclenche `$tryFallback` (lignes 146-179) : trois avertissements, puis
# `extractWithZipArchive` **par-dessus l'arbre déjà partiellement écrit**. La
# référence de ces archives n'est donc pas `unzip` seul, et c'est ce que la
# révision 1 du plan v0.20 avait manqué.
#
# Trois archives fabriquées, aucune ne vient du réseau :
#   dup      deux entrées du même nom (mode stocké 0600, contenus différents)
#            → `unzip` sort 1, invite `replace …?`, EOF = [N]one ; Composer
#              replie sur ZipArchive, qui écrase sans toucher aux modes : la
#              DERNIÈRE entrée gagne et le mode du zip survit. Install réussi.
#   collide  un fichier `a`, puis une entrée `a/b`
#            → `unzip` sort 2 (« checkdir error: … exists but is not
#              directory »), ZipArchive échoue aussi, l'install échoue.
#   reverse  l'entrée `a/b` d'abord, puis `a` comme fichier → même issue.
#
# Ce qui est comparé : le code de sortie, `vendor/` entier (modes et cibles de
# liens), et la stderr — celle de Composer porte deux jetons ALÉATOIRES
# (`vendor/composer/tmp-<32 hex>.zip` et `vendor/composer/<8 hex>`,
# `bin2hex(random_bytes(4))`, ArchiveDownloader.php:71), donc elle n'est pas
# reproductible d'une exécution à l'autre, même chez Composer. Le banc retire
# les sept lignes du repli en EXIGEANT de les trouver, puis compare le reste
# sans filtre : une ligne de repli qui disparaîtrait amont fait échouer le banc
# au lieu de passer inaperçue.
#
# Usage : harness/two-stage.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/two-stage"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication des archives)"; exit 1; }
command -v unzip >/dev/null || { echo "unzip requis : c'est l'extracteur de Composer"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK/art"
status=0

# Les trois archives, écrites en octets : `ZipArchive` refuse deux entrées du
# même nom, donc le doublon est posé à la main (le même enregistrement, un
# contenu différent, le même mode stocké).
php -r '
$dir = $argv[1];
$put = function (ZipArchive $z, string $name, string $data, int $mode) {
    $z->addFromString($name, $data);
    $z->setExternalAttributesName($name, ZipArchive::OPSYS_UNIX, ($mode | 0100000) << 16);
};
foreach ([
    "collide" => [["composer.json", null, 0644], ["a", "je suis un fichier\n", 0600], ["a/b", "et moi dessous\n", 0644]],
    "reverse" => [["composer.json", null, 0644], ["a/b", "dessous dabord\n", 0644], ["a", "puis le fichier\n", 0600]],
    "dup"     => [["composer.json", null, 0644], ["same.txt", "PREMIER\n", 0600]],
] as $name => $entries) {
    $z = new ZipArchive();
    $z->open("$dir/acme-$name-1.0.0.zip", ZipArchive::CREATE | ZipArchive::OVERWRITE);
    foreach ($entries as [$path, $data, $mode]) {
        $data ??= json_encode(["name" => "acme/$name", "version" => "1.0.0"]);
        $put($z, "acme-$name/$path", $data, $mode);
    }
    $z->close();
}
' "$WORK/art" || { echo "FAIL : fabrication des archives"; exit 1; }

# Le doublon : second enregistrement du même nom, ajouté en recopiant l'archive.
php -r '
$src = $argv[1];
$in = new ZipArchive(); $in->open($src);
$entries = [];
for ($i = 0; $i < $in->numFiles; $i++) {
    $st = $in->statIndex($i);
    $in->getExternalAttributesIndex($i, $opsys, $attr);
    $entries[] = [$st["name"], $in->getFromIndex($i), $attr];
}
$in->close();
// Réécriture à la main : en-tête local + annuaire central, store (0), pour
// pouvoir répéter un nom — ce que ZipArchive interdit.
$local = ""; $central = ""; $offset = 0;
$add = function (string $name, string $data, int $attr) use (&$local, &$central, &$offset) {
    $crc = crc32($data); $len = strlen($data);
    $h = pack("VvvvvvVVVvv", 0x04034b50, 10, 0, 0, 0, 0, $crc, $len, $len, strlen($name), 0) . $name;
    $local .= $h . $data;
    $central .= pack("VvvvvvvVVVvvvvvVV", 0x02014b50, 0x031e, 10, 0, 0, 0, 0, $crc, $len, $len,
        strlen($name), 0, 0, 0, 0, $attr, $offset) . $name;
    $offset += strlen($h) + $len;
};
foreach ($entries as [$name, $data, $attr]) { $add($name, $data, $attr); }
// Le doublon, avec le mode de son jumeau.
[$name, , $attr] = $entries[count($entries) - 1];
$add($name, "SECOND\n", $attr);
$count = count($entries) + 1;
$end = pack("VvvvvVVv", 0x06054b50, 0, 0, $count, $count, strlen($central), $offset, 0);
file_put_contents($src, $local . $central . $end);
' "$WORK/art/acme-dup-1.0.0.zip" || { echo "FAIL : fabrication du doublon"; exit 1; }
# Le doublon doit exister : sans lui le cas ne prouve rien.
dupes=$(unzip -l "$WORK/art/acme-dup-1.0.0.zip" | grep -c 'acme-dup/same.txt')
if [ "$dupes" != 2 ]; then echo "FAIL : l'archive du doublon n'a pas deux entrées same.txt ($dupes)"; exit 1; fi

seed() { # nom
  local name="$1" d
  for side in ref viv; do
    d="$WORK/$name/$side"; mkdir -p "$d"
  done
  cat > "$WORK/$name/ref/composer.json" <<JSON
{
    "repositories": [ { "type": "artifact", "url": "$WORK/art" } ],
    "require": { "acme/$name": "^1.0" }
}
JSON
  (cd "$WORK/$name/ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/seedcache" \
    composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >/dev/null 2>&1) || return 1
  cp "$WORK/$name/ref/composer.json" "$WORK/$name/ref/composer.lock" "$WORK/$name/viv/"
}

install_both() { # nom
  local name="$1" c=0 v=0
  (cd "$WORK/$name/ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-ref-$name" \
    composer install --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.ref.err") || c=$?
  (cd "$WORK/$name/viv" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-viv-$name" \
    VIVACITY_CACHE_DIR="$WORK/store-$name" "$VIVACITY" install --no-scripts --no-fallback >/dev/null 2>"$WORK/$name.viv.err") || v=$?
  REF_CODE=$c; VIV_CODE=$v
}

for name in dup collide reverse; do
  seed "$name" || { echo "FAIL $name : Composer n'a pas écrit le lock"; status=1; continue; }
  install_both "$name"
  if [ "$REF_CODE" != "$VIV_CODE" ]; then
    echo "FAIL $name : codes $REF_CODE (composer) vs $VIV_CODE (vivacity)"
    tail -2 "$WORK/$name.viv.err"; status=1; continue
  fi
  echo "OK   $name : les deux sortent $REF_CODE"
done

# --- dup : l'install réussit des deux côtés, l'arbre doit être identique.
if [ "$REF_CODE" = 0 ] || true; then
  if compare_vendor "$WORK/dup/ref" "$WORK/dup/viv" "$WORK/dup.diff" >/dev/null; then
    echo "OK   dup : vendor/ identique, modes compris (la dernière entrée gagne des deux côtés)"
  else
    echo "FAIL dup : vendor/ différent"; head -10 "$WORK/dup.diff"; status=1
  fi
  # Le mode stocké (0600) survit au repli : ZipArchive écrase le contenu sans
  # toucher au mode qu'`unzip` avait posé. C'est ce qui rend l'arbre
  # reproductible malgré les deux étages.
  if stat -c '%a' . >/dev/null 2>&1; then mode_of() { stat -c '%a' "$1"; }; else mode_of() { stat -f '%Lp' "$1"; }; fi
  for side in ref viv; do
    m=$(mode_of "$WORK/dup/$side/vendor/acme/dup/same.txt")
    c=$(cat "$WORK/dup/$side/vendor/acme/dup/same.txt")
    if [ "$m" != 600 ] || [ "$c" != "SECOND" ]; then
      echo "FAIL dup ($side) : mode $m, contenu \"$c\" (attendu 600 / SECOND)"; status=1
    else
      echo "OK   dup ($side) : mode 600 gardé, contenu de la dernière entrée"
    fi
  done
  # La stderr : les sept lignes du repli sont retirées en EXIGEANT de les voir.
  fallback_lines=0
  while IFS= read -r line; do
    case "$line" in
      "    Failed to extract acme/dup: (1) "*) fallback_lines=$((fallback_lines + 1)) ;;
      "replace "*"? [y]es, [n]o, [A]ll, [N]one, [r]ename:"*) fallback_lines=$((fallback_lines + 1)) ;;
      '(EOF or read error, treating as "[N]one" ...)') fallback_lines=$((fallback_lines + 1)) ;;
      "    The archive may contain identical file names"*) fallback_lines=$((fallback_lines + 1)) ;;
      "    Unzip with unzip command failed, falling back to ZipArchive class") fallback_lines=$((fallback_lines + 1)) ;;
      "") ;;
      *) echo "$line" ;;
    esac
  done < "$WORK/dup.ref.err" > "$WORK/dup.ref.filtered"
  if [ "$fallback_lines" != 5 ]; then
    echo "FAIL dup : $fallback_lines lignes de repli trouvées sur 5 — la référence a changé de bavardage"
    status=1
  else
    echo "OK   dup : les cinq lignes du repli sont bien là (deux jetons aléatoires, non reproductibles)"
  fi
  grep -v '^vivacity: ' "$WORK/dup.viv.err" | grep -v '^$' > "$WORK/dup.viv.filtered"
  if diff "$WORK/dup.ref.filtered" "$WORK/dup.viv.filtered" > "$WORK/dup.err.diff"; then
    echo "OK   dup : le reste de la stderr est identique"
  else
    echo "FAIL dup : stderr différente au-delà du repli"; head -8 "$WORK/dup.err.diff"; status=1
  fi
fi

# --- collide / reverse : rien d'installé, et notre message nomme le conflit.
for name in collide reverse; do
  if [ -e "$WORK/$name/ref/vendor/acme/$name" ] || [ -e "$WORK/$name/viv/vendor/acme/$name" ]; then
    echo "FAIL $name : un côté a laissé le paquet installé"; status=1
  else
    echo "OK   $name : aucun des deux n'installe le paquet"
  fi
done
if grep -q "entry a/b would be written under a, which the archive wrote as a file" "$WORK/collide.viv.err"; then
  echo "OK   collide : le refus nomme l'entrée et le fichier qui la bloque"
else
  echo "FAIL collide : message inattendu — $(tail -1 "$WORK/collide.viv.err")"; status=1
fi
if grep -q "entry a is a file where the archive already made a directory" "$WORK/reverse.viv.err"; then
  echo "OK   reverse : le refus nomme l'entrée et le répertoire déjà fait"
else
  echo "FAIL reverse : message inattendu — $(tail -1 "$WORK/reverse.viv.err")"; status=1
fi
exit $status
