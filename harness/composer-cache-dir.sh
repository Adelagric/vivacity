#!/usr/bin/env bash
# Le répertoire de cache de Composer, tel que vivacity le résout, contre
# Composer lui-même : `Config::get('cache-dir')` par-dessus
# `Factory::getCacheDir` (barres finales retirées, `~`/`$VAR`/`%VAR%`
# développés, relatif résolu contre le répertoire de travail, `/etc/xdg`).
#
# Pour chaque environnement de la matrice, deux mesures, chacune dans un bac
# à sable remis à neuf au même chemin pour les deux outils :
#   1. `config --global cache-dir` : stdout, stderr et code de sortie, à
#      l'octet (ce que l'action lit pour savoir quoi mettre en cache) ;
#   2. `install` d'un projet à dépôt `artifact` (aucun réseau), store de
#      vivacity vierge : l'endroit où le zip du paquet atterrit vraiment.
#      Les cas que vivacity rend à Composer (valeur vide, `0`,
#      COMPOSER_CACHE_FILES_DIR, `cache-files-dir` du composer.json du projet,
#      relatif sous --working-dir) passent par la
#      délégation : même endroit, par construction, et la délégation est
#      vérifiée au passage.
#
# Le cas `/etc/xdg` n'existe que sous Linux, en root (conteneur) ou là où le
# répertoire existe déjà (runners ubuntu) ; ailleurs il est sauté et dit.
# Plan : docs/plans/action-cache.md.
#
# Aucun réseau. Usage : harness/composer-cache-dir.sh
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/composer-cache-dir"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
COMPOSER="$(command -v composer)" || { echo "composer requis"; exit 1; }
PHP="$(command -v php)" || { echo "php requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd -P)"
# Rien de l'environnement appelant : chaque outil ne voit que PATH, HOME et le cas.
BASE_PATH="$(dirname "$COMPOSER"):$(dirname "$PHP"):/usr/bin:/bin"
SB="$WORK/sb"
status=0; passed=0; failed=0; skipped=0
fail() { echo "FAIL $*"; status=1; failed=$((failed + 1)); }

# Un paquet, un zip, un dépôt artifact, un lock fait une fois par Composer.
mkdir -p "$WORK/artifacts" "$WORK/seed/foo/src" "$WORK/proj"
cat > "$WORK/seed/foo/composer.json" <<'J'
{"name":"acme/foo","version":"1.0.0","type":"library","autoload":{"psr-4":{"Acme\\Foo\\":"src/"}}}
J
echo '<?php namespace Acme\Foo; class Foo {}' > "$WORK/seed/foo/src/Foo.php"
"$PHP" -r '
$z = new ZipArchive();
$z->open($argv[1], ZipArchive::CREATE) || exit(1);
foreach (["foo/composer.json", "foo/src/Foo.php"] as $f) { $z->addFile($argv[2] . "/" . $f, $f); }
$z->close() || exit(1);
' "$WORK/artifacts/acme-foo-1.0.0.zip" "$WORK/seed" || { echo "zip impossible (ext-zip requise)"; exit 1; }
cat > "$WORK/proj/composer.json" <<J
{"name":"acme/proj","type":"project","repositories":[{"type":"artifact","url":"$WORK/artifacts"},{"packagist.org":false}],"require":{"acme/foo":"1.0.0"}}
J
(cd "$WORK/proj" && env -i PATH="$BASE_PATH" HOME="$WORK/lockhome" COMPOSER_CACHE_DIR="$WORK/lockcache" \
  "$COMPOSER" update -n -q --no-install) || { echo "composer update a échoué"; exit 1; }
rm -rf "$WORK/lockhome" "$WORK/lockcache"

# Le bac à sable, remis à neuf : un HOME vide, le projet, et ce que le cas
# prépare (`legacy` : ~/.composer/cache existe déjà).
reset() {
  rm -rf "$SB"; mkdir -p "$SB/home"
  cp -R "$WORK/proj" "$SB/proj"
  case "$1" in
    legacy*) mkdir -p "$SB/home/.composer/cache" ;;
    project-config)
      "$PHP" -r '$f = $argv[1]; $j = json_decode(file_get_contents($f), true);
        $j["config"]["cache-files-dir"] = $argv[2];
        file_put_contents($f, json_encode($j, JSON_UNESCAPED_SLASHES));' "$SB/proj/composer.json" "$SB/pf" ;;
  esac
}

# Les variables d'un cas, une par ligne (`NOM=valeur`, `@` pour le bac à sable).
case_env() {
  case "$1" in
    no-xdg)        ;;
    absolute)      echo "COMPOSER_CACHE_DIR=@/cc" ;;
    trailing)      echo "COMPOSER_CACHE_DIR=@/cc/" ;;
    tilde)         echo "COMPOSER_CACHE_DIR=~/cc" ;;
    tilde-backslash) echo 'COMPOSER_CACHE_DIR=~\cc' ;;
    dollar-home)   echo 'COMPOSER_CACHE_DIR=$HOME/cc' ;;
    percent-home)  echo 'COMPOSER_CACHE_DIR=%HOME%/cc' ;;
    relative)      echo "COMPOSER_CACHE_DIR=rel/cc" ;;
    home-slash)    echo "COMPOSER_HOME=@/ch/" ;;
    home-zero)     echo "COMPOSER_HOME=0" ;;
    xdg-cache)     echo "XDG_CACHE_HOME=@/xdg" ;;
    xdg-other)     echo "XDG_RUNTIME_DIR=@/run" ;;
    legacy)        ;;
    legacy-xdg)    echo "XDG_CACHE_HOME=@/xdg" ;;
    etc-xdg)       ;;
    empty)         echo "COMPOSER_CACHE_DIR=" ;;
    zero)          echo "COMPOSER_CACHE_DIR=0" ;;
    files-dir)     echo "COMPOSER_CACHE_FILES_DIR=@/ff" ;;
    composer-auth) echo 'COMPOSER_AUTH={"github-oauth":{"github.com":"x"}}' ;;
    project-config) ;;
  esac
}

# `run <case> <dir> <cmd...>` : la commande, dans <dir>, sous l'environnement du cas.
run() {
  local c="$1" dir="$2"; shift 2
  local -a vars=(PATH="$BASE_PATH" HOME="$SB/home" VIVACITY_CACHE_DIR="$SB/store")
  local line
  while IFS= read -r line; do
    [ -n "$line" ] && vars+=("${line//@/$SB}")
  done < <(case_env "$c")
  (cd "$dir" && env -i "${vars[@]}" "$@")
}

# Où le zip du paquet a atterri, chemins relatifs au bac à sable.
zips() {
  (cd "$SB" && find . -name '*.zip' -path '*acme/foo/*' | sed 's|^\./||' | sort)
}

etc_xdg_made=0
cleanup() { [ "$etc_xdg_made" = 1 ] && rmdir /etc/xdg 2>/dev/null; true; }
trap cleanup EXIT

if [ -d /etc/xdg ]; then
  echo "note : /etc/xdg existe ici, chaque cas Linux passe par la branche XDG"
fi

CASES="no-xdg absolute trailing tilde tilde-backslash dollar-home percent-home relative home-slash home-zero xdg-cache xdg-other legacy legacy-xdg etc-xdg empty zero files-dir composer-auth project-config"
for c in $CASES; do
  if [ "$c" = etc-xdg ]; then
    if [ "$(uname -s)" != Linux ]; then
      echo "skip $c : Linux seulement (testé en unitaire, sonde injectée)"; skipped=$((skipped + 1)); continue
    fi
    if [ ! -d /etc/xdg ]; then
      if mkdir /etc/xdg 2>/dev/null; then etc_xdg_made=1
      else echo "skip $c : /etc/xdg absent et non créable"; skipped=$((skipped + 1)); continue; fi
    fi
  fi

  # 1. config --global cache-dir, à l'octet.
  reset "$c"; run "$c" "$SB/proj" "$COMPOSER" config --global cache-dir >"$WORK/c.out" 2>"$WORK/c.err"; c_code=$?
  reset "$c"; run "$c" "$SB/proj" "$VIVACITY" config --global cache-dir >"$WORK/v.out" 2>"$WORK/v.err"; v_code=$?
  case_ok=1
  if ! cmp -s "$WORK/c.out" "$WORK/v.out"; then
    fail "$c config stdout : composer [$(cat "$WORK/c.out")] vivacity [$(cat "$WORK/v.out")]"; case_ok=0
  fi
  if ! cmp -s "$WORK/c.err" "$WORK/v.err"; then
    fail "$c config stderr : composer [$(cat "$WORK/c.err")] vivacity [$(cat "$WORK/v.err")]"; case_ok=0
  fi
  [ "$c_code" = "$v_code" ] || { fail "$c config exit : composer $c_code vivacity $v_code"; case_ok=0; }
  printed="$(cat "$WORK/c.out")"

  # `~\cc` n'est pas développé (la classe de `expandPath` est `[\/]` : une
  # barre oblique seulement) ; hors Windows, Composer échoue ensuite à écrire
  # son cache à travers la contre-oblique (Filesystem::copy la normalise) :
  # seule la valeur imprimée se compare.
  if [ "$c" = tilde-backslash ]; then
    if [ "$case_ok" = 1 ]; then echo "ok   $c : [$printed] (config seulement)"; passed=$((passed + 1)); fi
    continue
  fi

  # 2. Où install écrit le zip.
  reset "$c"; run "$c" "$SB/proj" "$COMPOSER" install -n >"$WORK/ci.log" 2>&1 || { fail "$c composer install"; cat "$WORK/ci.log"; case_ok=0; }
  c_zips="$(zips)"
  reset "$c"; run "$c" "$SB/proj" "$VIVACITY" install -n >"$WORK/vi.log" 2>&1 || { fail "$c vivacity install"; cat "$WORK/vi.log"; case_ok=0; }
  v_zips="$(zips)"
  if [ "$c_zips" != "$v_zips" ] || [ -z "$c_zips" ]; then
    fail "$c install : composer [${c_zips//$'\n'/ }] vivacity [${v_zips//$'\n'/ }]"; case_ok=0
  fi
  delegated=no; grep -q '^vivacity: delegating to `composer install' "$WORK/vi.log" && delegated=yes
  case "$c" in
    empty|zero|files-dir|project-config) [ "$delegated" = yes ] || { fail "$c : vivacity aurait dû déléguer"; case_ok=0; } ;;
    *) [ "$delegated" = no ] || { fail "$c : délégation inattendue"; case_ok=0; } ;;
  esac
  if [ "$case_ok" = 1 ]; then
    echo "ok   $c : [$printed] -> ${c_zips%/acme/foo/*}$([ "$delegated" = yes ] && echo ' (délégué)')"
    passed=$((passed + 1))
  fi
  if [ "$c" = etc-xdg ] && [ "$etc_xdg_made" = 1 ]; then rmdir /etc/xdg; etc_xdg_made=0; fi
done

# Un relatif sous --working-dir : Composer le résout depuis le projet, après
# son chdir ; vivacity, qui ne change jamais de répertoire, le rend.
c=relative
reset "$c"; run "$c" "$SB" "$COMPOSER" install -n --working-dir proj >"$WORK/ci.log" 2>&1 || fail "relative --working-dir : composer install"
c_zips="$(zips)"
reset "$c"; run "$c" "$SB" "$VIVACITY" install -n --working-dir proj >"$WORK/vi.log" 2>&1 || fail "relative --working-dir : vivacity install"
v_zips="$(zips)"
if [ "$c_zips" = "$v_zips" ] && [ -n "$c_zips" ] && grep -q '^vivacity: delegating to `composer install' "$WORK/vi.log"; then
  echo "ok   relative --working-dir : ${c_zips%/acme/foo/*} (délégué)"; passed=$((passed + 1))
else
  fail "relative --working-dir : composer [${c_zips//$'\n'/ }] vivacity [${v_zips//$'\n'/ }]"
fi

echo "$passed ok, $failed échec(s), $skipped sauté(s)"
exit $status
