#!/usr/bin/env bash
# Le code de sortie d'un échec de transport : **100**, pas 1.
#
# `Application::doRun` réécrit le code d'une `TransportException` en
# `Installer::ERROR_TRANSPORT_EXCEPTION` (= 100) avant que Symfony Console n'en
# fasse un code de sortie (ZipDownloader/Application.php:502-506,
# Installer.php:92). Donc tout ce que Composer n'a pas réussi à RÉCUPÉRER sort
# en 100, là où vivacity sortait 1 — un script qui branche sur le code voyait
# une erreur générique au lieu d'un problème de réseau.
#
# Trois formes du même échec, toutes sans réseau : un dist qui répond 404 (servi
# par `php -S`), un dépôt `composer` injoignable (port fermé), et un dist local
# absent. Le banc compare le code de sortie des deux côtés, et vérifie au
# passage que le 404 n'est PAS réessayé — `CurlDownloader` ne réessaie que
# 423/425/500/502/503/504/507/510 (et un 400 de codeload.github.com).
#
# Usage : harness/transport-exit.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/transport-exit"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (serveur local)"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK/serve"
status=0

# Un serveur qui ne sert rien : toute requête répond 404.
port=$(php -r '$s = stream_socket_server("tcp://127.0.0.1:0", $e, $m); $n = stream_socket_get_name($s, false); fclose($s); echo substr($n, strrpos($n, ":") + 1);')
php -S "127.0.0.1:$port" -t "$WORK/serve" >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT
for _ in $(seq 1 50); do
  curl -s -o /dev/null "http://127.0.0.1:$port/ping" && break
  sleep 0.1
done
# Un port fermé pour le dépôt injoignable : celui du serveur plus un, libéré.
closed=$(php -r '$s = stream_socket_server("tcp://127.0.0.1:0", $e, $m); $n = stream_socket_get_name($s, false); fclose($s); echo substr($n, strrpos($n, ":") + 1);')

# `case <nom> <attendu>` : chaque cas est un projet déjà préparé dans $WORK/<nom>,
# avec son lock quand il en faut un.
VIV_MS=-1  # durée du dernier run vivacity, -1 quand le cas a échoué avant de
           # la mesurer : les contrôles qui la lisent se taisent plutôt que de
           # dire OK sur une valeur qui n'a pas été prise.
compare() { # nom, commande vivacity..., "--", commande composer...
  local name="$1"; shift
  local v=() c=(); while [ "$1" != "--" ]; do v+=("$1"); shift; done; shift; c=("$@")
  local vd="$WORK/$name/viv" cd_="$WORK/$name/ref"
  local v_code=0 c_code=0 v_start v_ms
  (cd "$cd_" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-ref" \
     composer "${c[@]}" --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.ref.err") || c_code=$?
  v_start=$(php -r 'echo (int) (microtime(true) * 1000);')
  (cd "$vd" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-viv" \
     VIVACITY_CACHE_DIR="$WORK/store-$name" "$VIVACITY" "${v[@]}" >/dev/null 2>"$WORK/$name.viv.err") || v_code=$?
  v_ms=$(( $(php -r 'echo (int) (microtime(true) * 1000);') - v_start ))
  if [ "$c_code" = 0 ] || [ "$v_code" = 0 ]; then
    echo "FAIL $name : un côté a réussi ($c_code composer, $v_code vivacity) — le cas ne prouve rien"
    status=1; VIV_MS=-1; return
  fi
  if [ "$c_code" != "$v_code" ]; then
    echo "FAIL $name : codes $c_code (composer) vs $v_code (vivacity)"
    tail -2 "$WORK/$name.viv.err"; status=1; VIV_MS=-1; return
  fi
  echo "OK   $name : les deux sortent $c_code (${v_ms} ms côté vivacity)"
  VIV_MS=$v_ms
}

# 1. un dist en 404 : le lock est écrit par Composer, les deux installent.
mkdir -p "$WORK/d404/ref" "$WORK/d404/viv"
cat > "$WORK/d404/ref/composer.json" <<JSON
{
    "repositories": [
        { "type": "package", "package": {
            "name": "probe/missing", "version": "1.0.0",
            "dist": { "type": "zip", "url": "http://127.0.0.1:$port/nope.zip", "shasum": "" } } }
    ],
    "require": { "probe/missing": "1.0.0" },
    "config": { "secure-http": false }
}
JSON
(cd "$WORK/d404/ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-seed" \
  composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >/dev/null 2>&1) || {
  echo "FAIL : Composer n'a pas écrit le lock du cas 404"; exit 1; }
cp "$WORK/d404/ref/composer.json" "$WORK/d404/ref/composer.lock" "$WORK/d404/viv/"
compare d404 install --no-scripts --no-fallback -- install --no-scripts
# Un 404 n'est pas réessayé : trois tentatives de plus se verraient (délais
# 0/100/500 ms) et surtout vivacity l'annonçait. Seuil large, la mesure sert
# seulement à attraper un retour en arrière.
if [ "$VIV_MS" -lt 0 ]; then
  : # le cas a déjà échoué : la durée ne veut rien dire
elif [ "$VIV_MS" -gt 3000 ]; then
  echo "FAIL d404 : ${VIV_MS} ms — un 404 a l'air réessayé"; status=1
else
  echo "OK   d404 : pas de réessai (${VIV_MS} ms)"
fi
if grep -q "attempts" "$WORK/d404.viv.err"; then
  echo "FAIL d404 : le message parle de plusieurs tentatives"; status=1
else
  echo "OK   d404 : le message ne prétend pas avoir réessayé"
fi

# 2. un dépôt composer injoignable, sur `update`.
mkdir -p "$WORK/repo/ref" "$WORK/repo/viv"
cat > "$WORK/repo/ref/composer.json" <<JSON
{
    "repositories": [
        { "type": "composer", "url": "http://127.0.0.1:$closed/repo" },
        { "packagist.org": false }
    ],
    "require": { "acme/nothing": "^1.0" },
    "config": { "secure-http": false }
}
JSON
cp "$WORK/repo/ref/composer.json" "$WORK/repo/viv/"
compare repo update --no-install --no-scripts --no-fallback -- update --no-install --no-scripts

# 3. un dist local absent (dépôt `package` pointant sur un fichier qui n'existe pas).
mkdir -p "$WORK/local/ref" "$WORK/local/viv"
cat > "$WORK/local/ref/composer.json" <<JSON
{
    "repositories": [
        { "type": "package", "package": {
            "name": "probe/gone", "version": "1.0.0",
            "dist": { "type": "zip", "url": "$WORK/serve/absent.zip", "shasum": "" } } }
    ],
    "require": { "probe/gone": "1.0.0" },
    "config": { "secure-http": false }
}
JSON
(cd "$WORK/local/ref" && COMPOSER_HOME="$WORK/home" COMPOSER_ROOT_VERSION=dev-main COMPOSER_CACHE_DIR="$WORK/cache-seed" \
  composer update --no-install --no-scripts --no-interaction --no-audit --no-ansi >/dev/null 2>&1) || {
  echo "FAIL : Composer n'a pas écrit le lock du cas local"; exit 1; }
cp "$WORK/local/ref/composer.json" "$WORK/local/ref/composer.lock" "$WORK/local/viv/"
compare local install --no-scripts --no-fallback -- install --no-scripts

exit $status
