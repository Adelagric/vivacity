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
# `COMPOSER_DISABLE_NETWORK=1` (stderr entière, trois états du cache) ;
# section 5, une connexion refusée sous `policy.ignore-unreachable` (code et
# sortie hors du message de transport, dont le texte diffère, inventorié).
#
# Section 6, les délais (docs/plans/net-timeouts.md) : un serveur qui accepte
# TCP et n'écrit jamais rien. Une URL `https://` y bloque la négociation TLS
# (le délai de connexion de curl, 10 s, la couvre ; 4 tentatives pour un
# GET), une URL `http://` y bloque après la connexion (le délai total, 300 s
# par tentative). `packages.json` d'un dépôt à liste malware, son résumé, une
# archive, l'API des avis (POST, 10 s, sans nouvelle tentative). Les deux
# côtés tournent en même temps, chacun avec ses serveurs ; comparés : code,
# stdout, stderr entière où seul le message de transport est remplacé (à
# l'intérieur de la ligne), caches des dépôts et des archives, vendor/, lock,
# durée bornée, connexions reçues. Les cas à 300 s ne tournent que sous
# TRANSPORT_EXIT_SLOW=1.
#
# Section 7, ce que Composer décide après un échec qui n'est pas un délai :
# archive en 500 (16 requêtes).
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

# 5. Connexion refusée (pas `--offline`) : le dépôt de la section 4, servi le
# temps de remplir le cache, puis arrêté. Le code de sortie et toute la
# sortie hors du message de transport sont comparés ; ce message, lui,
# diffère (le texte de curl chez Composer, celui de reqwest chez vivacity :
# inventaire de docs/plans/parity-gaps-026.md, point 54), et le cas vérifie
# que chacun écrit bien le sien.
refused_case() { # <nom> <chaud|froid> <ignore-unreachable true|false>
  local name="$1" warmth="$2" iu="$3" reg="$WORK/ref-$1" p rc=0 vc=0
  mkdir -p "$reg"; tar -C "$reg" -xzf "$ROOT/fixtures/registry/solver-policies.tar.gz"
  p=$(php -r '$s = stream_socket_server("tcp://127.0.0.1:0", $e, $m); $n = stream_socket_get_name($s, false); fclose($s); echo substr($n, strrpos($n, ":") + 1);')
  printf '{"packages": [], "metadata-url": "http://127.0.0.1:%s/p2/%%package%%.json", "available-package-patterns": ["*"], "security-advisories": {"metadata": true, "api-url": null}, "filter": {"metadata": true, "lists": {"malware": {"enabled": true}}, "summary-url": "http://127.0.0.1:%s/summary.json"}}\n' "$p" "$p" > "$reg/packages.json"
  [ -f "$reg/summary.json" ] || printf '{"filter": {"malware": {}}}\n' > "$reg/summary.json"
  mkdir -p "$WORK/ref-home-$name" "$WORK/$name/ref" "$WORK/$name/viv" "$WORK/ref-cache-$name"
  printf '{"repositories": {"snap": {"type": "composer", "url": "http://127.0.0.1:%s"}, "packagist.org": false}, "config": {"secure-http": false}}\n' "$p" > "$WORK/ref-home-$name/config.json"
  for side in ref viv; do
    cp "$ROOT/fixtures/projects/solver-policies/composer.lock" "$WORK/$name/$side/"
    jq ".config.policy[\"ignore-unreachable\"]=$iu" "$ROOT/fixtures/projects/solver-policies/composer.json" > "$WORK/$name/$side/composer.json"
  done
  if [ "$warmth" = chaud ]; then
    OFFLINE_ROOT="$reg" php -S "127.0.0.1:$p" "$WORK/router.php" >/dev/null 2>&1 &
    local srv=$!
    for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$p/packages.json" && break; sleep 0.1; done
    (cd "$WORK/$name/ref" && COMPOSER_HOME="$WORK/ref-home-$name" COMPOSER_CACHE_DIR="$WORK/ref-cache-$name" COMPOSER_ROOT_VERSION=dev-main \
      composer install --dry-run --no-plugins --no-scripts --no-interaction -q) || { echo "FAIL $name : le remplissage du cache a échoué"; status=1; kill $srv; return; }
    kill $srv 2>/dev/null; wait $srv 2>/dev/null
  fi
  cp -R "$WORK/ref-cache-$name" "$WORK/ref-cache-$name-ref"; cp -R "$WORK/ref-cache-$name" "$WORK/ref-cache-$name-viv"
  (cd "$WORK/$name/ref" && COLUMNS=400 COMPOSER_HOME="$WORK/ref-home-$name" COMPOSER_CACHE_DIR="$WORK/ref-cache-$name-ref" COMPOSER_ROOT_VERSION=dev-main COMPOSER_TESTS_ARE_RUNNING=1 \
    composer install --dry-run --no-plugins --no-scripts --no-interaction --no-ansi >/dev/null 2>"$WORK/$name.ref.err") || rc=$?
  (cd "$WORK/$name/viv" && COMPOSER_HOME="$WORK/ref-home-$name" COMPOSER_CACHE_DIR="$WORK/ref-cache-$name-viv" COMPOSER_ROOT_VERSION=dev-main \
    "$VIVACITY" install --dry-run --no-fallback >/dev/null 2>"$WORK/$name.viv.err") || vc=$?
  if [ "$rc" != "$vc" ]; then echo "FAIL $name : codes $rc (composer) vs $vc (vivacity)"; tail -3 "$WORK/$name.viv.err"; status=1; return; fi
  if [ "$iu" = false ]; then
    grep -q "Failed to connect to" "$WORK/$name.ref.err" || { echo "FAIL $name : Composer n'écrit pas l'échec de connexion (cas mal choisi)"; status=1; return; }
    grep -q "error sending request for url" "$WORK/$name.viv.err" || { echo "FAIL $name : vivacity n'écrit pas l'échec de connexion"; status=1; return; }
  fi
  # Toute la sortie sauf les lignes qui portent le message de transport :
  # l'avertissement du dépôt dégradé, l'encadré de Composer (titre, message,
  # synopsis, lignes vides), la ligne `Error:` de vivacity, et la ligne
  # `  - <message>` de la liste des sources ignorées.
  local drop='could not be fully loaded|^In [A-Za-z0-9_]*\.php line [0-9]*:$|curl error [0-9]+ while|HTTP failure for |^install \[|^Error: '
  grep -vE "$drop" "$WORK/$name.ref.err" | grep -v '^ *$' > "$WORK/$name.ref.rest"
  grep -vE "$drop" "$WORK/$name.viv.err" | grep -v '^ *$' > "$WORK/$name.viv.rest"
  if ! diff -q "$WORK/$name.ref.rest" "$WORK/$name.viv.rest" >/dev/null; then
    echo "FAIL $name : sortie différente hors du message de transport"; diff "$WORK/$name.ref.rest" "$WORK/$name.viv.rest" | head -8; status=1; return
  fi
  echo "OK   $name : code $rc, $(wc -l < "$WORK/$name.ref.rest" | tr -d ' ') lignes identiques hors du message de transport"
}
refused_case refused-warm-fatal chaud false
refused_case refused-cold-fatal froid false
refused_case refused-warm-ignored chaud true

# 6. Les délais. Chaque côté a ses serveurs : G sert l'instantané
# solver-policies en HTTP (le routeur de la section 4), H accepte toute
# connexion et n'écrit jamais un octet (journal des connexions acceptées).
# Les ports diffèrent d'un côté à l'autre : ils sont remplacés par <G> et
# <H> dans tout ce qui est comparé, noms des répertoires de cache compris.
cat > "$WORK/hold.php" <<'PHP'
<?php
// Accepts every connection and never writes a byte: an `https://` URL waits
// for a ServerHello that never comes, an `http://` one for a response.
$s = stream_socket_server('tcp://127.0.0.1:' . $argv[1], $errno, $errstr);
if ($s === false) { exit(1); }
file_put_contents($argv[2], '');
$held = [];
while (true) {
    $c = @stream_socket_accept($s, 3600);
    if ($c !== false) { $held[] = $c; file_put_contents($argv[2], "accept\n", FILE_APPEND); }
}
PHP
# Runs a command with a limit: kills its process group past the limit, then
# writes "<code|timeout> <ms>" — a hang shows as a timeout, not as a stuck run.
cat > "$WORK/timed.pl" <<'PERL'
use strict; use warnings; use Time::HiRes qw(time);
my ($limit, $out, $err, $rcfile, @cmd) = @ARGV;
my $t0 = time;
my $pid = fork();
die "fork: $!" unless defined $pid;
if ($pid == 0) {
    setpgrp(0, 0);
    open STDOUT, '>', $out or die; open STDERR, '>', $err or die;
    exec @cmd or exit 127;
}
my $timed_out = 0;
local $SIG{ALRM} = sub { $timed_out = 1; kill 'KILL', -$pid; };
alarm $limit;
waitpid($pid, 0);
my $st = $?;
alarm 0;
my $ms = int((time - $t0) * 1000);
open my $f, '>', $rcfile or die;
print $f ($timed_out ? 'timeout' : ($st >> 8)), " $ms\n";
PERL
# The router of section 4, plus FAIL_PATH (that path answers 500) and
# BAD_JSON_PATH (that path answers 200 with a body that is not JSON).
cat > "$WORK/router7.php" <<'PHP'
<?php
$path = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
if (getenv('FAIL_PATH') !== false && $path === getenv('FAIL_PATH')) {
    file_put_contents(getenv('FAIL_LOG'), "500\n", FILE_APPEND); http_response_code(500); return true;
}
if (getenv('BAD_JSON_PATH') !== false && $path === getenv('BAD_JSON_PATH')) {
    header('Content-Type: text/html'); echo "<html>not json</html>\n"; return true;
}
$file = getenv('OFFLINE_ROOT') . $path;
if (!is_file($file)) { http_response_code(404); return true; }
if (getenv('OFFLINE_LAST_MODIFIED') === '1') {
    $lm = 'Thu, 01 Jan 2026 00:00:00 GMT';
    header('Last-Modified: ' . $lm);
    if (($_SERVER['HTTP_IF_MODIFIED_SINCE'] ?? '') === $lm) { http_response_code(304); return true; }
}
header('Content-Type: application/json');
readfile($file);
return true;
PHP
free_port() { php -r '$s = stream_socket_server("tcp://127.0.0.1:0", $e, $m); $n = stream_socket_get_name($s, false); fclose($s); echo substr($n, strrpos($n, ":") + 1);'; }
S6_PIDS=()
trap 'kill $server ${S6_PIDS[*]:-} 2>/dev/null' EXIT
# `servers <dir>` : G and H for one side, ports in <dir>/g and <dir>/h, the
# registry in <dir>/reg (packages.json written by the case). Extra
# environment for G (FAIL_PATH, BAD_JSON_PATH, OFFLINE_LAST_MODIFIED) is
# taken from the caller's.
servers() {
  local d="$1" g h
  mkdir -p "$d/reg"; tar -C "$d/reg" -xzf "$ROOT/fixtures/registry/solver-policies.tar.gz"
  g=$(free_port); h=$(free_port)
  : > "$d/fail.log"
  OFFLINE_ROOT="$d/reg" FAIL_LOG="$d/fail.log" php -S "127.0.0.1:$g" "$WORK/router7.php" >/dev/null 2>&1 &
  S6_PIDS+=($!)
  php "$WORK/hold.php" "$h" "$d/h.log" >/dev/null 2>&1 &
  S6_PIDS+=($!)
  for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$g/SNAPSHOT" && [ -f "$d/h.log" ] && break; sleep 0.1; done
  echo "$g" > "$d/g"; echo "$h" > "$d/h"
}
# The packages.json of the snapshot: metadata on G; the summary and the
# advisories API where the case says (`G` or `H`, with a scheme).
packages_json() { # <dir> <summary-url> <advisories-api-url or "null"> <advisories metadata true|false>
  local d="$1" g; g=$(cat "$d/g")
  printf '{"packages": [], "metadata-url": "http://127.0.0.1:%s/p2/%%package%%.json", "available-package-patterns": ["*"], "security-advisories": {"metadata": %s, "api-url": %s}, "filter": {"metadata": true, "lists": {"malware": {"enabled": true}}, "summary-url": "%s"}}\n' \
    "$g" "$4" "$3" "$2"
}
# The project of a side: solver-policies' lock and manifest (jq edit), its
# COMPOSER_HOME with the repository at <repo-url>, notifications off.
project() { # <dir> <repo-url> [jq edit]
  local d="$1"
  mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
  printf '{"repositories": {"snap": {"type": "composer", "url": "%s"}, "packagist.org": false}, "config": {"secure-http": false, "notify-on-install": false}}\n' "$2" > "$d/home/config.json"
  cp "$ROOT/fixtures/projects/solver-policies/composer.lock" "$d/p/"
  jq "${3:-.}" "$ROOT/fixtures/projects/solver-policies/composer.json" > "$d/p/composer.json"
}
# `launch <case> <limit s> <composer args> -- <vivacity args>` : both sides in
# the background, each in its own directory.
launch() {
  local name="$1" limit="$2"; shift 2
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local d="$WORK/s6/$name"
  (cd "$d/ref/p" && COLUMNS=400 COMPOSER_HOME="$d/ref/home" COMPOSER_CACHE_DIR="$d/ref/cache" COMPOSER_ROOT_VERSION=dev-main COMPOSER_TESTS_ARE_RUNNING=1 \
    perl "$WORK/timed.pl" "$limit" "$d/ref.out" "$d/ref.err" "$d/ref.rc" composer "${c[@]}" --no-interaction --no-ansi) &
  S6_JOBS+=($!)
  (cd "$d/viv/p" && COLUMNS=400 COMPOSER_HOME="$d/viv/home" COMPOSER_CACHE_DIR="$d/viv/cache" COMPOSER_ROOT_VERSION=dev-main VIVACITY_CACHE_DIR="$d/viv/store" \
    perl "$WORK/timed.pl" "$limit" "$d/viv.out" "$d/viv.err" "$d/viv.rc" "$VIVACITY" "${v[@]}") &
  S6_JOBS+=($!)
}
# A side's output with its ports replaced, Composer's exception box turned
# into `Error: <message>` (the convention of section 4), and the text of a
# transport failure — curl's error or `CurlDownloader::failResponse`'s
# « The "<url>" file could not be downloaded (HTTP/…) » against reqwest's,
# point 54 of parity-gaps-026.md — replaced inside its line: the line
# itself, its place and its count stay compared.
normalize() { # <file> <side dir>
  ports "$2" < "$1" > "$1.p"
  if grep -q '^In [A-Za-z0-9_]*\.php line [0-9]*:$' "$1.p"; then
    { sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/q;p' "$1.p" | grep -v '^$'
      sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/,/^$/p' "$1.p" | sed 's/^ *//; s/ *$//' | grep -v '^$' | grep -vE '^In [A-Za-z0-9_]*\.php line [0-9]*:$' | sed 's/^/Error: /'
    } > "$1.b"; mv "$1.b" "$1.p"
  fi
  local t='(curl error [0-9]+ while downloading |HTTP failure for |The "[^"]+" file could not be downloaded \(HTTP/)'
  sed -E "s#could not be fully loaded \\($t.*\\), package information#could not be fully loaded (<transport>), package information#; s#^  - $t.*#  - <transport>#; s#^Error: $t.*#Error: <transport>#" "$1.p" > "$1.n"
  rm -f "$1.p"
}
# A side's ports replaced by their names, in URLs (`127.0.0.1:<port>`) and in
# the names of the repository caches (`127.0.0.1-<port>`).
ports() { # <side dir> (stdin → stdout)
  local d="$1" script="" n p
  for n in g h g2; do
    [ -f "$d/$n" ] || continue
    p=$(cat "$d/$n"); script="$script s#127\.0\.0\.1[:-]$p([^0-9]|\$)#<$(echo "$n" | tr a-z A-Z)>\1#g;"
  done
  sed -E "$script"
}
# What a side left on disk: every file of the repository and archive caches
# (contents hashed), vendor/, the lock. Ports replaced.
effects() { # <side dir>
  local d="$1"
  ( cd "$d" && find cache/repo cache/files p/vendor -type f 2>/dev/null | LC_ALL=C sort | while read -r f; do
      printf '%s %s\n' "$f" "$(ports "$d" < "$f" | shasum | cut -c1-12)"; done
    ports "$d" < p/composer.lock | shasum | cut -c1-12 ) | ports "$d"
}
# `judge <case> <min s> <max s> <motif Composer must print, or -> [compare stderr: yes|no]`
judge() {
  local name="$1" lo="$2" hi="$3" want="$4" cmp="${5:-yes}" d="$WORK/s6/$1" rc vc rms vms
  read -r rc rms < "$d/ref.rc"; read -r vc vms < "$d/viv.rc"
  if [ "$rc" = timeout ]; then echo "FAIL $name : Composer dépasse ${hi} s (cas mal choisi)"; status=1; return; fi
  if [ "$want" != - ] && ! grep -q -- "$want" "$d/ref.err"; then echo "FAIL $name : Composer n'écrit pas « $want » (cas mal choisi)"; status=1; return; fi
  if [ "${S6_IDLE_H:-0}" = 0 ] && [ ! -s "$d/ref/h.log" ]; then echo "FAIL $name : H n'a reçu aucune connexion de Composer (cas mal choisi)"; status=1; return; fi
  if [ "$vc" = timeout ]; then echo "FAIL $name : vivacity tourne encore après ${hi} s (tué ; Composer : code $rc en $((rms / 1000)) s)"; status=1; return; fi
  if [ "$rc" != "$vc" ]; then echo "FAIL $name : codes $rc (composer) vs $vc (vivacity)"; tail -3 "$d/viv.err"; status=1; return; fi
  normalize "$d/ref.err" "$d/ref"; normalize "$d/viv.err" "$d/viv"
  normalize "$d/ref.out" "$d/ref"; normalize "$d/viv.out" "$d/viv"
  if [ "$cmp" = yes ] && ! diff -q "$d/ref.err.n" "$d/viv.err.n" >/dev/null; then
    echo "FAIL $name : stderr diffère (message de transport remplacé)"; diff "$d/ref.err.n" "$d/viv.err.n" | head -10; status=1; return
  fi
  if ! diff -q "$d/ref.out.n" "$d/viv.out.n" >/dev/null; then echo "FAIL $name : stdout diffère"; diff "$d/ref.out.n" "$d/viv.out.n" | head -6; status=1; return; fi
  effects "$d/ref" > "$d/ref.fx"; effects "$d/viv" > "$d/viv.fx"
  if ! diff -q "$d/ref.fx" "$d/viv.fx" >/dev/null; then echo "FAIL $name : effets sur le disque différents"; diff "$d/ref.fx" "$d/viv.fx" | head -8; status=1; return; fi
  if [ "$(wc -l < "$d/ref/h.log")" != "$(wc -l < "$d/viv/h.log")" ] || [ "$(wc -l < "$d/ref/fail.log")" != "$(wc -l < "$d/viv/fail.log")" ]; then
    echo "FAIL $name : tentatives $(wc -l < "$d/ref/h.log" | tr -d ' ')+$(wc -l < "$d/ref/fail.log" | tr -d ' ') (composer) vs $(wc -l < "$d/viv/h.log" | tr -d ' ')+$(wc -l < "$d/viv/fail.log" | tr -d ' ') (vivacity) — connexions à H + réponses 500 de G"
    status=1; return
  fi
  if [ "$vms" -lt $((lo * 1000)) ] || [ "$vms" -gt $((hi * 1000)) ]; then
    echo "FAIL $name : vivacity en $vms ms, hors de [$lo s, $hi s] (Composer : $rms ms)"; status=1; return
  fi
  echo "OK   $name : code $rc, $( [ "$cmp" = yes ] && echo "stderr identique hors du texte de transport ($(wc -l < "$d/ref.err.n" | tr -d ' ') lignes), " )effets identiques, $((rms / 100))/10 s Composer, $((vms / 100))/10 s vivacity, $(( $(wc -l < "$d/ref/h.log") + $(wc -l < "$d/ref/fail.log") )) tentatives de chaque côté"
}
s6_case() { # <case> : the two sides' servers
  for side in ref viv; do servers "$WORK/s6/$1/$side"; done
}
mkdir -p "$WORK/s6"
VERIFY_REF=(install --dry-run --no-plugins --no-scripts)
VERIFY_VIV=(install --dry-run --no-fallback)
S6_NAME=(); S6_LO=(); S6_HI=(); S6_WANT=(); S6_CMP=(); S6_JOBS=()
# `expect <case> <min s> <max s> <motif Composer must print, or -> [yes|no]`
expect() { S6_NAME+=("$1"); S6_LO+=("$2"); S6_HI+=("$3"); S6_WANT+=("$4"); S6_CMP+=("${5:-yes}"); }

# 6a. packages.json bloqué en TLS, cache froid : `hasFilter` lève, la source
# est ignorée sous `ignore-unreachable` (défaut), ou fatale (100).
for iu in true false; do
  name=tls-pkg-cold; [ "$iu" = false ] && name=tls-pkg-cold-fatal
  s6_case "$name"
  for side in ref viv; do
    d="$WORK/s6/$name/$side"
    packages_json "$d" "http://127.0.0.1:$(cat "$d/g")/summary.json" null true > "$d/reg/packages.json"
    project "$d" "https://127.0.0.1:$(cat "$d/h")" ".config.policy[\"ignore-unreachable\"]=$iu"
  done
  launch "$name" 70 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect $name 35 60 -
done
# 6b. Le cas cypht : packages.json bloqué en TLS, un cache de plus de 600 s
# (forgé dans le répertoire de l'URL https, avec ou sans `last-modified` :
# `fetchFileIfLastModified` ou `fetchFile`) : l'avertissement du dépôt
# dégradé, le cache, puis le résumé servi par G.
for lm in lm nolm; do
  name=tls-pkg-warm-$lm
  s6_case "$name"
  for side in ref viv; do
    d="$WORK/s6/$name/$side"; h=$(cat "$d/h"); g=$(cat "$d/g")
    packages_json "$d" "http://127.0.0.1:$g/summary.json" null true > "$d/reg/packages.json"
    project "$d" "https://127.0.0.1:$h"
    mkdir -p "$d/cache/repo/https---127.0.0.1-$h"
    if [ "$lm" = lm ]; then
      jq -c '. + {"last-modified": "Thu, 01 Jan 2026 00:00:00 GMT"}' "$d/reg/packages.json" > "$d/cache/repo/https---127.0.0.1-$h/packages.json"
    else
      cp "$d/reg/packages.json" "$d/cache/repo/https---127.0.0.1-$h/packages.json"
    fi
    touch -t 202601010000 "$d/cache/repo/https---127.0.0.1-$h/packages.json"
  done
  launch "$name" 70 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect $name 35 60 "could not be fully loaded"
done
# 6c. Le résumé des listes bloqué en TLS (packages.json servi) :
# `asyncFetchFile` avertit, sans cache rien ne le remplace, la source est
# ignorée.
s6_case tls-summary
for side in ref viv; do
  d="$WORK/s6/tls-summary/$side"
  packages_json "$d" "https://127.0.0.1:$(cat "$d/h")/summary.json" null true > "$d/reg/packages.json"
  project "$d" "http://127.0.0.1:$(cat "$d/g")"
done
launch tls-summary 70 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect tls-summary 35 60 "Filter list data could not be fetched"
# 6d. Une archive bloquée en TLS : 4 tentatives curl, puis l'exception —
# `FileDownloader` ne rappelle pas `$download()` pour une erreur curl, dont
# la `TransportException` a le code 400 par défaut (TransportException.php,
# FileDownloader.php l. 262-266) ; il le fait pour un 500/502/503/504
# (cas `dist-500`, section 7). Le texte d'un téléchargement qui échoue
# diffère déjà (point 12 : vivacity n'imprime pas les lignes
# « Downloading ») : code, stdout, effets, tentatives et durée comparés,
# pas stderr.
s6_case tls-dist
for side in ref viv; do
  d="$WORK/s6/tls-dist/$side"
  mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
  printf '{"config": {"secure-http": false, "notify-on-install": false}}\n' > "$d/home/config.json"
  printf '{"repositories": [{"packagist.org": false}], "require": {"probe/stalled": "1.0.0"}, "config": {"notify-on-install": false}}\n' > "$d/p/composer.json"
  jq -n --arg url "https://127.0.0.1:$(cat "$d/h")/stalled.zip" '{"_readme": [], "content-hash": "0", "packages": [{"name": "probe/stalled", "version": "1.0.0", "dist": {"type": "zip", "url": $url, "shasum": ""}, "type": "library"}], "packages-dev": [], "aliases": [], "minimum-stability": "stable", "stability-flags": {}, "prefer-stable": false, "prefer-lowest": false, "platform": {}, "platform-dev": {}, "plugin-api-version": "2.6.0"}' > "$d/p/composer.lock"
done
launch tls-dist 70 install --no-plugins --no-scripts -- install --no-scripts --no-fallback; expect tls-dist 35 60 - no
# 6e. L'API des avis bloquée (TLS, puis après connexion), sur `update` :
# 10 s au total, un POST n'est pas retenté. `metadata: false` pour que le
# POST parte, `--no-audit` (Composer audite `update`, vivacity non : point
# 26). Témoins : la requête des avis a ses 10 s depuis longtemps.
for scheme in https http; do
  name=$scheme-advisory
  s6_case "$name"
  for side in ref viv; do
    d="$WORK/s6/$name/$side"
    packages_json "$d" "http://127.0.0.1:$(cat "$d/g")/summary.json" "\"$scheme://127.0.0.1:$(cat "$d/h")/api/security-advisories/\"" false > "$d/reg/packages.json"
    project "$d" "http://127.0.0.1:$(cat "$d/g")"
  done
  launch "$name" 40 update --no-install --no-plugins --no-scripts --no-audit -- update --no-install --no-scripts --no-audit --no-fallback
  expect $name 9 25 "Security advisory data could not be fetched"
done
# 6f. Hors CI (TRANSPORT_EXIT_SLOW=1) : bloqué après la connexion, le délai
# total : 4 × 300 s pour packages.json comme pour une archive.
if [ "${TRANSPORT_EXIT_SLOW:-0}" = 1 ]; then
  s6_case http-pkg-cold
  for side in ref viv; do
    d="$WORK/s6/http-pkg-cold/$side"
    packages_json "$d" "http://127.0.0.1:$(cat "$d/g")/summary.json" null true > "$d/reg/packages.json"
    project "$d" "http://127.0.0.1:$(cat "$d/h")"
  done
  launch http-pkg-cold 1500 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect http-pkg-cold 1190 1400 "Filter list data could not be fetched"
  s6_case http-dist
  for side in ref viv; do
    d="$WORK/s6/http-dist/$side"
    mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
    printf '{"config": {"secure-http": false, "notify-on-install": false}}\n' > "$d/home/config.json"
    printf '{"repositories": [{"packagist.org": false}], "require": {"probe/stalled": "1.0.0"}, "config": {"notify-on-install": false}}\n' > "$d/p/composer.json"
    jq -n --arg url "http://127.0.0.1:$(cat "$d/h")/stalled.zip" '{"_readme": [], "content-hash": "0", "packages": [{"name": "probe/stalled", "version": "1.0.0", "dist": {"type": "zip", "url": $url, "shasum": ""}, "type": "library"}], "packages-dev": [], "aliases": [], "minimum-stability": "stable", "stability-flags": {}, "prefer-stable": false, "prefer-lowest": false, "platform": {}, "platform-dev": {}, "plugin-api-version": "2.6.0"}' > "$d/p/composer.lock"
  done
  launch http-dist 1500 install --no-plugins --no-scripts -- install --no-scripts --no-fallback; expect http-dist 1190 1400 - no
fi
# Every case runs at once: the section lasts as long as its longest case.
wait "${S6_JOBS[@]}"
for i in "${!S6_NAME[@]}"; do
  judge "${S6_NAME[$i]}" "${S6_LO[$i]}" "${S6_HI[$i]}" "${S6_WANT[$i]}" "${S6_CMP[$i]}"
done

# 7. Ce que Composer décide après un échec qui n'est pas un délai : rapides,
# mêmes outils que la section 6 (H y est démarré et ne reçoit rien).
S6_NAME=(); S6_LO=(); S6_HI=(); S6_WANT=(); S6_CMP=(); S6_JOBS=()
S6_IDLE_H=1
# 7a. Une archive en 500 : `FileDownloader` rappelle `$download()` trois
# fois après 500 ms (la `TransportException` porte le code 500, l. 262-276),
# et chaque appel refait les 4 tentatives de `CurlDownloader` : 16 requêtes.
FAIL_PATH=/broken.zip s6_case dist-500
for side in ref viv; do
  d="$WORK/s6/dist-500/$side"
  mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
  printf '{"config": {"secure-http": false, "notify-on-install": false}}\n' > "$d/home/config.json"
  printf '{"repositories": [{"packagist.org": false}], "require": {"probe/broken": "1.0.0"}, "config": {"notify-on-install": false}}\n' > "$d/p/composer.json"
  jq -n --arg url "http://127.0.0.1:$(cat "$d/g")/broken.zip" '{"_readme": [], "content-hash": "0", "packages": [{"name": "probe/broken", "version": "1.0.0", "dist": {"type": "zip", "url": $url, "shasum": ""}, "type": "library"}], "packages-dev": [], "aliases": [], "minimum-stability": "stable", "stability-flags": {}, "prefer-stable": false, "prefer-lowest": false, "platform": {}, "platform-dev": {}, "plugin-api-version": "2.6.0"}' > "$d/p/composer.lock"
done
launch dist-500 30 install --no-plugins --no-scripts -- install --no-scripts --no-fallback; expect dist-500 0 20 - no
wait "${S6_JOBS[@]}"
for i in "${!S6_NAME[@]}"; do
  judge "${S6_NAME[$i]}" "${S6_LO[$i]}" "${S6_HI[$i]}" "${S6_WANT[$i]}" "${S6_CMP[$i]}"
done

exit $status
