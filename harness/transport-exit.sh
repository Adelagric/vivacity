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
# Et, section 4, ce que `--offline` écrit : la même chose que Composer sous
# `COMPOSER_DISABLE_NETWORK=1` (stderr entière, trois états du cache).
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
# … et sur `require` sans contrainte : la version à écrire est cherchée dans
# ce dépôt avant toute résolution (`PackageDiscoveryTrait`), une
# `TransportException` là aussi (vivacity sortait en 1).
for d in ref viv; do mkdir -p "$WORK/repo-require/$d"; cp "$WORK/repo/ref/composer.json" "$WORK/repo-require/$d/"; done
compare repo-require require acme/nothing --no-scripts --no-fallback -- require acme/nothing --no-scripts

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

# 4. Hors ligne : `--offline` de vivacity contre `COMPOSER_DISABLE_NETWORK=1`
# (la variable que le fallback lui donne). `HttpDownloader` (l. 328-338)
# répond 304 à une requête conditionnelle et rejette toute autre par
# « Network disabled, request canceled: <url> ». Un dépôt `composer` servi en
# HTTP local (l'instantané solver-policies : liste malware, `summary.json`),
# avec ou sans `Last-Modified` ; cache rempli par un `install --dry-run` de
# Composer en ligne (chaud), ou vide (froid). stderr comparée en entier.
cat > "$WORK/router.php" <<'PHP'
<?php
// Static files of OFFLINE_ROOT, with a fixed Last-Modified (and a 304 on a
// matching If-Modified-Since) when OFFLINE_LAST_MODIFIED is 1.
$path = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
$file = getenv('OFFLINE_ROOT') . $path;
if (!is_file($file)) {
    http_response_code(404);
    return true;
}
if (getenv('OFFLINE_LAST_MODIFIED') === '1') {
    $lm = 'Thu, 01 Jan 2026 00:00:00 GMT';
    header('Last-Modified: ' . $lm);
    if (($_SERVER['HTTP_IF_MODIFIED_SINCE'] ?? '') === $lm) {
        http_response_code(304);
        return true;
    }
}
header('Content-Type: application/json');
readfile($file);
return true;
PHP
offline_case() { # <nom> <Last-Modified 0|1> <chaud|froid> <motif attendu chez Composer, ou "-" pour aucun « could not »> [jq sur composer.json]
  local name="$1" lm="$2" warmth="$3" want="$4" edit="${5:-.}" reg="$WORK/off-$1" p rc=0 vc=0
  mkdir -p "$reg"; tar -C "$reg" -xzf "$ROOT/fixtures/registry/solver-policies.tar.gz"
  p=$(php -r '$s = stream_socket_server("tcp://127.0.0.1:0", $e, $m); $n = stream_socket_get_name($s, false); fclose($s); echo substr($n, strrpos($n, ":") + 1);')
  printf '{"packages": [], "metadata-url": "http://127.0.0.1:%s/p2/%%package%%.json", "available-package-patterns": ["*"], "security-advisories": {"metadata": true, "api-url": null}, "filter": {"metadata": true, "lists": {"malware": {"enabled": true}}, "summary-url": "http://127.0.0.1:%s/summary.json"}}\n' "$p" "$p" > "$reg/packages.json"
  [ -f "$reg/summary.json" ] || printf '{"filter": {"malware": {}}}\n' > "$reg/summary.json"
  OFFLINE_ROOT="$reg" OFFLINE_LAST_MODIFIED="$lm" php -S "127.0.0.1:$p" "$WORK/router.php" >/dev/null 2>&1 &
  local srv=$!
  for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$p/packages.json" && break; sleep 0.1; done
  mkdir -p "$WORK/off-home-$name" "$WORK/$name/ref" "$WORK/$name/viv"
  printf '{"repositories": {"snap": {"type": "composer", "url": "http://127.0.0.1:%s"}, "packagist.org": false}, "config": {"secure-http": false}}\n' "$p" > "$WORK/off-home-$name/config.json"
  for side in ref viv; do
    cp "$ROOT/fixtures/projects/solver-policies/composer.lock" "$WORK/$name/$side/"
    jq "$edit" "$ROOT/fixtures/projects/solver-policies/composer.json" > "$WORK/$name/$side/composer.json"
  done
  if [ "$warmth" = chaud ]; then
    (cd "$WORK/$name/ref" && COMPOSER_HOME="$WORK/off-home-$name" COMPOSER_CACHE_DIR="$WORK/off-cache-$name" COMPOSER_ROOT_VERSION=dev-main \
      composer install --dry-run --no-plugins --no-scripts --no-interaction -q) || { echo "FAIL $name : le remplissage du cache a échoué"; status=1; kill $srv; return; }
  fi
  mkdir -p "$WORK/off-cache-$name"
  cp -R "$WORK/off-cache-$name" "$WORK/off-cache-$name-ref"; cp -R "$WORK/off-cache-$name" "$WORK/off-cache-$name-viv"
  (cd "$WORK/$name/ref" && COLUMNS=400 COMPOSER_HOME="$WORK/off-home-$name" COMPOSER_CACHE_DIR="$WORK/off-cache-$name-ref" COMPOSER_ROOT_VERSION=dev-main COMPOSER_DISABLE_NETWORK=1 COMPOSER_TESTS_ARE_RUNNING=1 \
    composer install --dry-run --no-plugins --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.ref.err") || rc=$?
  (cd "$WORK/$name/viv" && COMPOSER_HOME="$WORK/off-home-$name" COMPOSER_CACHE_DIR="$WORK/off-cache-$name-viv" COMPOSER_ROOT_VERSION=dev-main \
    "$VIVACITY" install --dry-run --offline >/dev/null 2>"$WORK/$name.viv.err") || vc=$?
  kill $srv 2>/dev/null; wait $srv 2>/dev/null
  if [ "$want" = - ]; then
    if grep -q "could not be" "$WORK/$name.ref.err"; then echo "FAIL $name : Composer n'est pas muet (cas mal choisi)"; status=1; return; fi
  elif ! grep -q -- "$want" "$WORK/$name.ref.err"; then
    echo "FAIL $name : Composer n'écrit pas « $want » (cas mal choisi)"; status=1; return
  fi
  if [ "$rc" != "$vc" ]; then echo "FAIL $name : codes $rc (composer) vs $vc (vivacity)"; tail -3 "$WORK/$name.viv.err"; status=1; return; fi
  sed "s#127.0.0.1:$p#<server>#g" "$WORK/$name.ref.err" > "$WORK/$name.ref.n"
  sed "s#127.0.0.1:$p#<server>#g" "$WORK/$name.viv.err" > "$WORK/$name.viv.n"
  # Une exception : chez Composer les lignes d'avant puis l'encadré (sans le
  # synopsis), chez vivacity les mêmes lignes puis `Error: <message>`.
  if grep -q '^In [A-Za-z0-9_]*\.php line [0-9]*:$' "$WORK/$name.ref.n"; then
    { sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/q;p' "$WORK/$name.ref.n" | grep -v '^$'
      sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/,/^$/p' "$WORK/$name.ref.n" | sed 's/^ *//; s/ *$//' | grep -v '^$' | grep -vE '^In [A-Za-z0-9_]*\.php line [0-9]*:$' | sed 's/^/Error: /'
    } > "$WORK/$name.ref.box"; mv "$WORK/$name.ref.box" "$WORK/$name.ref.n"
  fi
  if ! diff -q "$WORK/$name.ref.n" "$WORK/$name.viv.n" >/dev/null; then
    echo "FAIL $name : stderr diffère"; diff "$WORK/$name.ref.n" "$WORK/$name.viv.n" | head -8; status=1; return
  fi
  echo "OK   $name : code $rc, stderr identique ($(wc -l < "$WORK/$name.ref.err" | tr -d ' ') lignes)"
}
offline_case offline-warm-lm 1 chaud -
offline_case offline-warm-nolm 0 chaud "could not be fully loaded (Network disabled, request canceled"
offline_case offline-cold 1 froid "  - Network disabled, request canceled: http://"
# `policy.ignore-unreachable` faux : le résumé refusé devient l'exception, après
# l'avertissement du dépôt dégradé, que Composer a déjà écrit.
offline_case offline-unreachable-fatal 0 chaud "could not be fully loaded (Network disabled, request canceled" '.config.policy["ignore-unreachable"]=false'

exit $status
