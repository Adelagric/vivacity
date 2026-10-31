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
# archive en 500 (16 requêtes), JSON invalide (avec et sans cache, le
# résumé), ordre de deux dépôts dans le filtre, lot de fichiers p2 dont un
# échoue, include d'un dépôt complet que Composer ne demande pas.
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
# The router of section 4, plus FAIL_PATH (that path answers 500),
# BAD_JSON_PATH (that path answers 200 with a body that is not JSON) and
# REQ_LOG (every request's path, one line each).
cat > "$WORK/router7.php" <<'PHP'
<?php
$path = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
if (getenv('REQ_LOG') !== false) { file_put_contents(getenv('REQ_LOG'), "$path\n", FILE_APPEND); }
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
# A free port no other server of this run has drawn — nor the closed port of
# section 2, which 7d needs closed: in FRESH_PORT.
USED_PORTS=("$port" "$closed")
fresh_port() {
  local p
  while :; do
    p=$(free_port)
    case " ${USED_PORTS[*]} " in *" $p "*) continue ;; esac
    USED_PORTS+=("$p"); FRESH_PORT=$p; return
  done
}
S6_PIDS=()
trap 'kill $server ${S6_PIDS[*]:-} 2>/dev/null' EXIT
# `servers <dir>` : G and H for one side, ports in <dir>/g and <dir>/h, the
# registry in <dir>/reg (packages.json written by the case). Extra
# environment for G (FAIL_PATH, BAD_JSON_PATH, OFFLINE_LAST_MODIFIED) is
# taken from the caller's.
servers() {
  local d="$1" g h
  mkdir -p "$d/reg"; tar -C "$d/reg" -xzf "$ROOT/fixtures/registry/solver-policies.tar.gz"
  fresh_port; g=$FRESH_PORT; fresh_port; h=$FRESH_PORT
  : > "$d/fail.log"; : > "$d/req.log"
  OFFLINE_ROOT="$d/reg" FAIL_LOG="$d/fail.log" REQ_LOG="$d/req.log" php -S "127.0.0.1:$g" "$WORK/router7.php" >/dev/null 2>&1 &
  S6_PIDS+=($!)
  php "$WORK/hold.php" "$h" "$d/h.log" >/dev/null 2>&1 &
  S6_PIDS+=($!)
  for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$g/SNAPSHOT" && [ -f "$d/h.log" ] && break; sleep 0.1; done
  : > "$d/req.log"
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
# the background, each in its own directory; S6_REF_PREFIX and S6_VIV_PREFIX
# (arrays, `env VAR=value`) go before Composer's and vivacity's command.
launch() {
  local name="$1" limit="$2"; shift 2
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local d="$WORK/s6/$name"
  (cd "$d/ref/p" && COLUMNS=400 COMPOSER_HOME="$d/ref/home" COMPOSER_CACHE_DIR="$d/ref/cache" COMPOSER_ROOT_VERSION=dev-main COMPOSER_TESTS_ARE_RUNNING=1 \
    perl "$WORK/timed.pl" "$limit" "$d/ref.out" "$d/ref.err" "$d/ref.rc" ${S6_REF_PREFIX[@]+"${S6_REF_PREFIX[@]}"} composer "${c[@]}" --no-interaction --no-ansi) &
  S6_JOBS+=($!)
  (cd "$d/viv/p" && COLUMNS=400 COMPOSER_HOME="$d/viv/home" COMPOSER_CACHE_DIR="$d/viv/cache" COMPOSER_ROOT_VERSION=dev-main VIVACITY_CACHE_DIR="$d/viv/store" \
    perl "$WORK/timed.pl" "$limit" "$d/viv.out" "$d/viv.err" "$d/viv.rc" ${S6_VIV_PREFIX[@]+"${S6_VIV_PREFIX[@]}"} "$VIVACITY" "${v[@]}") &
  S6_JOBS+=($!)
}
# A side's output with its ports replaced, Composer's exception box turned
# into `Error: <message>` (the convention of section 4), and the text of a
# transport failure — curl's error or `CurlDownloader::failResponse`'s
# « The "<url>" file could not be downloaded (HTTP/…) » against reqwest's,
# point 54 of parity-gaps-026.md — replaced inside its line: the line
# itself, its place and its count stay compared.
normalize() { # <file> <side dir>
  # vivacity's own summary line (point 33) is not Composer's.
  ports "$2" < "$1" | grep -vE '^vivacity: [0-9]+ installed, ' > "$1.p"
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
# the names of the repository caches (`127.0.0.1-<port>`); and the sha1s of
# <dir>/sha by <SHA>.
ports() { # <side dir> (stdin → stdout)
  local d="$1" script="" n p
  for n in g h g2; do
    [ -f "$d/$n" ] || continue
    p=$(cat "$d/$n"); script="$script s#127\.0\.0\.1[:-]$p([^0-9]|\$)#<$(echo "$n" | tr a-z A-Z)>\1#g;"
  done
  # The files cache names an archive after the sha1 of its URL, which holds
  # the side's port: <dir>/sha lists them (section 8).
  if [ -f "$d/sha" ]; then
    while read -r p; do script="$script s#$p#<SHA>#g;"; done < "$d/sha"
  fi
  sed -E "$script"
}
# What a side left on disk: every file of the repository and archive caches
# (contents hashed), vendor/ (its directories too: an empty vendor/ left
# behind counts), the lock. Ports replaced.
effects() { # <side dir>
  local d="$1"
  # Sorted after the ports are replaced: two repository caches named after
  # random ports would otherwise come in either order.
  ( cd "$d" && find cache/repo cache/files p/vendor -type f 2>/dev/null | while read -r f; do
      printf '%s %s\n' "$(printf '%s' "$f" | ports "$d")" "$(ports "$d" < "$f" | shasum | cut -c1-12)"; done | LC_ALL=C sort
    find p/vendor -type d 2>/dev/null | ports "$d" | sed 's/^/dir /' | LC_ALL=C sort
    ports "$d" < p/composer.lock | shasum | cut -c1-12 )
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
  if [ "${S6_COUNT_REQ:-0}" = 1 ] && [ "$(wc -l < "$d/ref/req.log")" != "$(wc -l < "$d/viv/req.log")" ]; then
    echo "FAIL $name : requêtes à G $(wc -l < "$d/ref/req.log" | tr -d ' ') (composer) vs $(wc -l < "$d/viv/req.log" | tr -d ' ') (vivacity)"; status=1; return
  fi
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

# Section 8's tools, used by `tls-dist-store` (6d) too. Two archives of one
# package with the same files and different classes (a third for another
# package), a lock of zip dists
# with a fixed reference (the store keys an entry on it, not on the URL), a
# project per side, and the store of a side filled by an install of its own.
S8="$WORK/s8"; mkdir -p "$S8"
mkzip() { # <out> <class>
  python3 -I -c '
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as z:
    z.writestr("composer.json", "{\"name\": \"probe/desync\", \"autoload\": {\"classmap\": [\"src/\"]}}\n")
    z.writestr("src/A.php", "<?php\nnamespace Probe;\nclass %s {}\n" % sys.argv[2])
' "$1" "$2"
}
mkzip "$S8/a.zip" A; mkzip "$S8/b.zip" A2; mkzip "$S8/c.zip" C
REF40=0123456789abcdef0123456789abcdef01234567
# `s8_lock <name> <url> <shasum> [<name> <url> <shasum>]...` : the lock on stdout.
s8_lock() {
  local pkgs="[]"
  while [ $# -gt 0 ]; do
    pkgs=$(jq -c --arg n "$1" --arg u "$2" --arg s "$3" --arg r "$REF40" \
      '. + [{"name": $n, "version": "1.0.0", "dist": {"type": "zip", "url": $u, "reference": $r, "shasum": $s}, "autoload": {"classmap": ["src/"]}, "type": "library"}]' <<<"$pkgs")
    shift 3
  done
  jq -n --argjson p "$pkgs" '{"_readme": [], "content-hash": "0", "packages": $p, "packages-dev": [], "aliases": [], "minimum-stability": "stable", "stability-flags": {}, "prefer-stable": false, "prefer-lowest": false, "platform": {}, "platform-dev": {}, "plugin-api-version": "2.6.0"}'
}
# `s8_project <side dir> <name> <url> <shasum> [...]` : the project, its
# COMPOSER_HOME, and the sha1 of each URL for `ports`.
s8_project() {
  local d="$1"; shift
  mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
  printf '{"config": {"secure-http": false, "notify-on-install": false}}\n' > "$d/home/config.json"
  local req="{}" a=("$@")
  while [ $# -gt 0 ]; do
    req=$(jq -c --arg n "$1" '. + {($n): "1.0.0"}' <<<"$req")
    printf '%s' "$2" | shasum | cut -c1-40 >> "$d/sha"
    shift 3
  done
  jq -n --argjson r "$req" '{"repositories": [{"packagist.org": false}], "require": $r, "config": {"notify-on-install": false}}' > "$d/p/composer.json"
  s8_lock "${a[@]}" > "$d/p/composer.lock"
}
# `s8_cache_path <side dir> <name> <url>` : where the files cache keeps that dist.
s8_cache_path() { echo "$1/cache/files/$2/$(printf '%s' "$3" | shasum | cut -c1-40).zip"; }
# `viv_warm <side dir> [keep]` : vivacity installs the side's project (its
# store and its files cache filled), then vendor/ goes, and the files cache
# too unless `keep`; the request logs start again.
viv_warm() {
  local d="$1"
  (cd "$d/p" && COMPOSER_HOME="$d/home" COMPOSER_CACHE_DIR="$d/cache" COMPOSER_ROOT_VERSION=dev-main VIVACITY_CACHE_DIR="$d/store" \
    "$VIVACITY" install --no-scripts --no-fallback -o >"$d/warm.log" 2>&1) || { echo "FAIL : la préparation de $d a échoué"; tail -3 "$d/warm.log"; status=1; }
  rm -rf "${d:?}/p/vendor"
  [ "${2:-}" = keep ] || rm -rf "${d:?}/cache/files"
  : > "$d/req.log"; : > "$d/h.log"
}
# `ref_warm <side dir>` : the same for Composer (its files cache filled).
ref_warm() {
  local d="$1"
  (cd "$d/p" && COMPOSER_HOME="$d/home" COMPOSER_CACHE_DIR="$d/cache" COMPOSER_ROOT_VERSION=dev-main \
    composer install --no-plugins --no-scripts --no-interaction -o >"$d/warm.log" 2>&1) || { echo "FAIL : la préparation de $d a échoué"; tail -3 "$d/warm.log"; status=1; }
  rm -rf "${d:?}/p/vendor"
  : > "$d/req.log"; : > "$d/h.log"
}

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
# (cas `dist-500`, section 7). Depuis le point 55, vivacity imprime les
# lignes « Downloading » avant l'erreur d'un téléchargement, comme Composer :
# stderr comparée aussi.
s6_case tls-dist
for side in ref viv; do
  d="$WORK/s6/tls-dist/$side"
  mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
  printf '{"config": {"secure-http": false, "notify-on-install": false}}\n' > "$d/home/config.json"
  printf '{"repositories": [{"packagist.org": false}], "require": {"probe/stalled": "1.0.0"}, "config": {"notify-on-install": false}}\n' > "$d/p/composer.json"
  jq -n --arg url "https://127.0.0.1:$(cat "$d/h")/stalled.zip" '{"_readme": [], "content-hash": "0", "packages": [{"name": "probe/stalled", "version": "1.0.0", "dist": {"type": "zip", "url": $url, "shasum": ""}, "type": "library"}], "packages-dev": [], "aliases": [], "minimum-stability": "stable", "stability-flags": {}, "prefer-stable": false, "prefer-lowest": false, "platform": {}, "platform-dev": {}, "plugin-api-version": "2.6.0"}' > "$d/p/composer.lock"
done
launch tls-dist 70 install --no-plugins --no-scripts -- install --no-scripts --no-fallback; expect tls-dist 35 60 "  - Downloading probe/stalled"
# 6d'. Point 55 : la même archive bloquée, mais dans le store de vivacity
# (rempli depuis G, puis le lock repointé vers H) et absente du cache de
# fichiers de Composer : Composer la demande (FileDownloader.php l. 176-198),
# 4 tentatives, 100 ; vivacity aussi, au lieu de poser le paquet depuis son
# store et de sortir en 0. stderr comparée.
s6_case tls-dist-store
for side in ref viv; do
  d="$WORK/s6/tls-dist-store/$side"; g=$(cat "$d/g"); h=$(cat "$d/h")
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "http://127.0.0.1:$g/desync.zip" ""
  [ "$side" = viv ] && viv_warm "$d"
  s8_lock probe/desync "https://127.0.0.1:$h/desync.zip" "" > "$d/p/composer.lock"
  printf '%s' "https://127.0.0.1:$h/desync.zip" | shasum | cut -c1-40 >> "$d/sha"
done
launch tls-dist-store 70 install --no-plugins --no-scripts -- install --no-scripts --no-fallback; expect tls-dist-store 35 60 "  - Downloading probe/desync"
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
# stderr non comparée : chaque tour réimprime la ligne « Downloading »
# (l. 192-194), vivacity l'imprime une fois (point 12).
FAIL_PATH=/broken.zip s6_case dist-500
for side in ref viv; do
  d="$WORK/s6/dist-500/$side"
  mkdir -p "$d/home" "$d/p" "$d/cache" "$d/store"
  printf '{"config": {"secure-http": false, "notify-on-install": false}}\n' > "$d/home/config.json"
  printf '{"repositories": [{"packagist.org": false}], "require": {"probe/broken": "1.0.0"}, "config": {"notify-on-install": false}}\n' > "$d/p/composer.json"
  jq -n --arg url "http://127.0.0.1:$(cat "$d/g")/broken.zip" '{"_readme": [], "content-hash": "0", "packages": [{"name": "probe/broken", "version": "1.0.0", "dist": {"type": "zip", "url": $url, "shasum": ""}, "type": "library"}], "packages-dev": [], "aliases": [], "minimum-stability": "stable", "stability-flags": {}, "prefer-stable": false, "prefer-lowest": false, "platform": {}, "platform-dev": {}, "plugin-api-version": "2.6.0"}' > "$d/p/composer.lock"
done
launch dist-500 30 install --no-plugins --no-scripts -- install --no-scripts --no-fallback; expect dist-500 0 20 - no
# 7b. packages.json répond 200 avec autre chose que du JSON. Avec un cache
# (vieux de plus de 600 s, avec ou sans `last-modified`) : `fetchFile` et
# `fetchFileIfLastModified` rattrapent toute exception, avertissent et
# servent le cache (ComposerRepository.php l. 1801-1825, 1885-1900) ; le
# message est celui de `Response::decodeJson`, pas un message de transport.
# Sans cache : la `ParsingException` n'est pas une `TransportException`,
# `ignore-unreachable` n'y peut rien, sortie 1.
for variant in warm-lm warm-nolm cold; do
  name=json-pkg-$variant
  BAD_JSON_PATH=/packages.json s6_case "$name"
  for side in ref viv; do
    d="$WORK/s6/$name/$side"; g=$(cat "$d/g")
    packages_json "$d" "http://127.0.0.1:$g/summary.json" null true > "$d/reg/packages.json"
    project "$d" "http://127.0.0.1:$g"
    if [ "$variant" != cold ]; then
      mkdir -p "$d/cache/repo/http---127.0.0.1-$g"
      if [ "$variant" = warm-lm ]; then
        jq -c '. + {"last-modified": "Thu, 01 Jan 2026 00:00:00 GMT"}' "$d/reg/packages.json" > "$d/cache/repo/http---127.0.0.1-$g/packages.json"
      else
        cp "$d/reg/packages.json" "$d/cache/repo/http---127.0.0.1-$g/packages.json"
      fi
      touch -t 202601010000 "$d/cache/repo/http---127.0.0.1-$g/packages.json"
    fi
  done
  launch "$name" 30 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect "$name" 0 20 "does not contain valid JSON"
done
# 7c. Le résumé des listes en JSON invalide : l'exception part de `$accept`,
# que le `$reject` du même `then` ne rattrape pas (l. 1950-2010) : sortie 1,
# sous `ignore-unreachable` comme sans.
BAD_JSON_PATH=/summary.json s6_case json-summary
for side in ref viv; do
  d="$WORK/s6/json-summary/$side"; g=$(cat "$d/g")
  packages_json "$d" "http://127.0.0.1:$g/summary.json" null true > "$d/reg/packages.json"
  project "$d" "http://127.0.0.1:$g"
done
launch json-summary 30 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect json-summary 0 20 "does not contain valid JSON"
# 7d. Deux dépôts, dans l'ordre de Composer : `FilterListProviderSet` lit le
# packages.json de **tous** les dépôts (`hasFilter`) avant le premier
# `getFilter` (FilterListProviderSet.php l. 47-58, 150-158). A : packages.json
# servi, résumé sur un port fermé ; B : packages.json en 500, cache vieux.
# Chez Composer, l'avertissement de B (première passe) précède celui de A
# (seconde passe), puis la liste des sources ignorées.
s6_case order-a
FAIL_PATH=/packages.json s6_case order-b
for side in ref viv; do
  a="$WORK/s6/order-a/$side"; b="$WORK/s6/order-b/$side"; d="$WORK/s6/order-two-repos/$side"
  mkdir -p "$d"; cp "$a/g" "$a/h" "$d/"; : > "$d/fail.log"; : > "$d/h.log"
  packages_json "$a" "http://127.0.0.1:$closed/summary.json" null true > "$a/reg/packages.json"
  gb=$(cat "$b/g")
  packages_json "$b" "http://127.0.0.1:$gb/summary.json" null true > "$b/reg/packages.json"
  project "$d" "http://127.0.0.1:$(cat "$a/g")"
  jq --arg b "http://127.0.0.1:$gb" '.repositories = {"a": .repositories.snap, "b": {"type": "composer", "url": $b}, "packagist.org": false}' "$d/home/config.json" > "$d/home/c.json" && mv "$d/home/c.json" "$d/home/config.json"
  mkdir -p "$d/cache/repo/http---127.0.0.1-$gb"
  cp "$b/reg/packages.json" "$d/cache/repo/http---127.0.0.1-$gb/packages.json"
  touch -t 202601010000 "$d/cache/repo/http---127.0.0.1-$gb/packages.json"
  # The failures of B's server are this case's.
  ln -sf "$b/fail.log" "$d/fail.log"
  echo "$gb" > "$d/g2"
done
launch order-two-repos 30 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect order-two-repos 0 20 "Filter list data could not be fetched"
# 7e. Un lot de fichiers p2 dont un échoue (500) : Composer écrit en cache
# chaque réponse réussie (`$accept`) et attend tout le lot (`Loop::wait`)
# avant de lever ; le résumé désigne acme/bad et acme/lib.
FAIL_PATH=/p2/acme/bad.json s6_case p2-batch
for side in ref viv; do
  d="$WORK/s6/p2-batch/$side"; g=$(cat "$d/g")
  printf '{"filter": {"malware": {"acme/bad": "*", "acme/lib": "*"}}}\n' > "$d/reg/summary.json"
  packages_json "$d" "http://127.0.0.1:$g/summary.json" null true > "$d/reg/packages.json"
  project "$d" "http://127.0.0.1:$g"
done
launch p2-batch 30 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect p2-batch 0 20 "Filter list data could not be fetched"
# 7f. Un dépôt « complet » (sans `metadata-url`) dont l'include répond 500 :
# `hasFilter` ne lit que packages.json (`loadRootServerFile` ne charge pas
# les includes) et ce dépôt n'annonce pas de liste ; l'install depuis le lock
# ne charge aucun paquet de dépôt : l'include n'est jamais demandé.
FAIL_PATH=/include/all.json s6_case plain-includes
for side in ref viv; do
  d="$WORK/s6/plain-includes/$side"; g=$(cat "$d/g")
  printf '{"packages": {}, "includes": {"include/all.json": {"sha1": "0000000000000000000000000000000000000000"}}}\n' > "$d/reg/packages.json"
  project "$d" "http://127.0.0.1:$g"
done
launch plain-includes 30 "${VERIFY_REF[@]}" -- "${VERIFY_VIV[@]}"; expect plain-includes 0 20 -
wait "${S6_JOBS[@]}"
for i in "${!S6_NAME[@]}"; do
  judge "${S6_NAME[$i]}" "${S6_LO[$i]}" "${S6_HI[$i]}" "${S6_WANT[$i]}" "${S6_CMP[$i]}"
done

# 8. Point 55 (docs/plans/net-timeouts.md) : les deux caches désynchronisés.
# Une archive que le store de vivacity a et que le cache de fichiers de
# Composer n'a pas (ou pas avec la bonne somme) : Composer la télécharge
# (FileDownloader.php l. 176-198) et l'écrit dans son cache ; vivacity fait
# la même requête et en tire la même issue. Le témoin, deux caches chauds :
# aucune requête, ni d'un côté ni de l'autre. Comptés : les requêtes que G
# reçoit (REQ_LOG), de chaque côté.
S6_NAME=(); S6_LO=(); S6_HI=(); S6_WANT=(); S6_CMP=(); S6_JOBS=()
S6_IDLE_H=1; S6_COUNT_REQ=1
INSTALL_REF=(install --no-plugins --no-scripts -o)
INSTALL_VIV=(install --no-scripts --no-fallback -o)
# `requests <case> <expected count, both sides>` : after `judge`.
S8_REQ_NAME=(); S8_REQ_WANT=()
requests() { S8_REQ_NAME+=("$1"); S8_REQ_WANT+=("$2"); }
# 8a. Store chaud, cache froid, serveur qui répond.
s6_case desync-fetch
for side in ref viv; do
  d="$WORK/s6/desync-fetch/$side"
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "http://127.0.0.1:$(cat "$d/g")/desync.zip" ""
done
viv_warm "$WORK/s6/desync-fetch/viv"
launch desync-fetch 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect desync-fetch 0 20 "  - Downloading probe/desync"; requests desync-fetch 1
# 8b. Le store vient de l'archive A, le serveur sert B (mêmes fichiers,
# une autre classe) : le paquet posé est B, l'entrée du store reste A.
s6_case desync-other-bytes
for side in ref viv; do
  d="$WORK/s6/desync-other-bytes/$side"
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "http://127.0.0.1:$(cat "$d/g")/desync.zip" ""
  [ "$side" = viv ] && viv_warm "$d"
  cp "$S8/b.zip" "$d/reg/desync.zip"
done
launch desync-other-bytes 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect desync-other-bytes 0 20 "  - Downloading probe/desync"; requests desync-other-bytes 1
# 8c. Store chaud, cache froid, port fermé : 100 des deux côtés.
s6_case refused-dist-store
for side in ref viv; do
  d="$WORK/s6/refused-dist-store/$side"
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "http://127.0.0.1:$(cat "$d/g")/desync.zip" ""
  [ "$side" = viv ] && viv_warm "$d"
  s8_lock probe/desync "http://127.0.0.1:$closed/desync.zip" "" > "$d/p/composer.lock"
  printf '%s' "http://127.0.0.1:$closed/desync.zip" | shasum | cut -c1-40 >> "$d/sha"
done
launch refused-dist-store 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect refused-dist-store 0 20 "  - Downloading probe/desync"; requests refused-dist-store 0
# 8d. Le témoin : cache de Composer chaud, store chaud. Aucune requête.
s6_case desync-witness
for side in ref viv; do
  d="$WORK/s6/desync-witness/$side"
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "http://127.0.0.1:$(cat "$d/g")/desync.zip" ""
done
ref_warm "$WORK/s6/desync-witness/ref"; viv_warm "$WORK/s6/desync-witness/viv" keep
launch desync-witness 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect desync-witness 0 20 -; requests desync-witness 0
# 8e. Hors ligne (`--offline` contre COMPOSER_DISABLE_NETWORK=1), store
# chaud, cache froid : la ligne, puis « Network disabled, request
# canceled » (HttpDownloader.php l. 328-338), 100.
s6_case offline-dist-store
for side in ref viv; do
  d="$WORK/s6/offline-dist-store/$side"
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "http://127.0.0.1:$(cat "$d/g")/desync.zip" ""
done
viv_warm "$WORK/s6/offline-dist-store/viv"
S6_REF_PREFIX=(env COMPOSER_DISABLE_NETWORK=1)
launch offline-dist-store 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}" --offline; expect offline-dist-store 0 20 "Network disabled, request canceled"; requests offline-dist-store 0
S6_REF_PREFIX=()
# 8f, 8g. Une archive sur un chemin local (comme l'écrit un dépôt
# `artifact`), store chaud, cache froid : copiée par RemoteFilesystem, même
# hors ligne, et écrite dans le cache.
for variant in desync-local offline-local; do
  s6_case "$variant"
  for side in ref viv; do
    d="$WORK/s6/$variant/$side"
    s8_project "$d" probe/desync "$S8/a.zip" ""
  done
  viv_warm "$WORK/s6/$variant/viv"
  if [ "$variant" = offline-local ]; then
    S6_REF_PREFIX=(env COMPOSER_DISABLE_NETWORK=1)
    launch "$variant" 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}" --offline
    S6_REF_PREFIX=()
  else
    launch "$variant" 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"
  fi
  expect "$variant" 0 20 "  - Downloading probe/desync"; requests "$variant" 0
done
# 8h. `dist.shasum` posé, une autre archive en cache à la clé : Composer ne
# la sert pas (l. 180), télécharge et l'écrase ; store chaud chez vivacity.
s6_case desync-stale-sha
a_sha=$(shasum < "$S8/a.zip" | cut -c1-40)
for side in ref viv; do
  d="$WORK/s6/desync-stale-sha/$side"; url="http://127.0.0.1:$(cat "$d/g")/desync.zip"
  cp "$S8/a.zip" "$d/reg/desync.zip"
  s8_project "$d" probe/desync "$url" "$a_sha"
  [ "$side" = viv ] && viv_warm "$d"
  f=$(s8_cache_path "$d" probe/desync "$url"); mkdir -p "$(dirname "$f")"; cp "$S8/b.zip" "$f"
done
launch desync-stale-sha 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect desync-stale-sha 0 20 "  - Downloading probe/desync"; requests desync-stale-sha 1
# 8i. Un lot mixte : probe/cached dans les deux caches, probe/desync dans le
# store seul. Une requête de chaque côté, celle de probe/desync.
s6_case desync-mixed
for side in ref viv; do
  d="$WORK/s6/desync-mixed/$side"; g=$(cat "$d/g")
  cp "$S8/a.zip" "$d/reg/desync.zip"; cp "$S8/c.zip" "$d/reg/cached.zip"
  s8_project "$d" probe/cached "http://127.0.0.1:$g/cached.zip" "" probe/desync "http://127.0.0.1:$g/desync.zip" ""
  if [ "$side" = viv ]; then viv_warm "$d" keep; rm -f "$(s8_cache_path "$d" probe/desync "http://127.0.0.1:$g/desync.zip")"; fi
  f=$(s8_cache_path "$d" probe/cached "http://127.0.0.1:$g/cached.zip"); mkdir -p "$(dirname "$f")"; cp "$S8/c.zip" "$f"
done
launch desync-mixed 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect desync-mixed 0 20 "  - Downloading probe/desync"; requests desync-mixed 1
# 8k. Pas de cache de fichiers : `COMPOSER_CACHE_DIR=/dev/null` (`Cache::isUsable`
# refuse le chemin, sans un mot) et `cache-files-ttl: 0` (aucun cache,
# Factory.php l. 506-510). Composer télécharge tout et n'écrit rien ; l'archive
# déjà à la clé (ttl 0) n'est pas lue. Store chaud chez vivacity.
for variant in devnull-cache ttl-zero; do
  s6_case "$variant"
  for side in ref viv; do
    d="$WORK/s6/$variant/$side"; url="http://127.0.0.1:$(cat "$d/g")/desync.zip"
    cp "$S8/a.zip" "$d/reg/desync.zip"
    s8_project "$d" probe/desync "$url" ""
    [ "$side" = viv ] && viv_warm "$d"
    if [ "$variant" = ttl-zero ]; then
      jq '.config["cache-files-ttl"] = 0' "$d/p/composer.json" > "$d/p/c.json" && mv "$d/p/c.json" "$d/p/composer.json"
      f=$(s8_cache_path "$d" probe/desync "$url"); mkdir -p "$(dirname "$f")"; cp "$S8/a.zip" "$f"
    fi
  done
  if [ "$variant" = devnull-cache ]; then
    S6_REF_PREFIX=(env COMPOSER_CACHE_DIR=/dev/null); S6_VIV_PREFIX=(env COMPOSER_CACHE_DIR=/dev/null)
  fi
  launch "$variant" 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect "$variant" 0 20 "  - Downloading probe/desync"; requests "$variant" 1
  S6_REF_PREFIX=(); S6_VIV_PREFIX=()
done
# 8l. Un cache en lecture seule (`COMPOSER_CACHE_READ_ONLY`) : Composer lit
# ses caches sans jamais les écrire, vivacity écrit celui des dépôts —
# rendu à Composer avant toute sortie (sous --no-fallback : code 3, la
# raison seule, aucune requête).
d="$WORK/s8/ro"; mkdir -p "$d"; cp "$S8/a.zip" "$d/a.zip"
s8_project "$d" probe/desync "$d/a.zip" ""
ro_code=0
(cd "$d/p" && COMPOSER_HOME="$d/home" COMPOSER_CACHE_DIR="$d/cache" COMPOSER_ROOT_VERSION=dev-main VIVACITY_CACHE_DIR="$d/store" COMPOSER_CACHE_READ_ONLY=1 \
  "$VIVACITY" install --no-scripts --no-fallback >"$d/out" 2>"$d/err") || ro_code=$?
if [ "$ro_code" != 3 ] || ! grep -q "read-only" "$d/err" || [ -e "$d/p/vendor" ]; then
  echo "FAIL ro-cache : code $ro_code (3 attendu), vendor/ $([ -e "$d/p/vendor" ] && echo présent || echo absent)"; head -3 "$d/err"; status=1
else
  echo "OK   ro-cache : rendu à Composer avant toute sortie (code 3 sous --no-fallback)"
fi
judge_requests() {
  local i name want d rq vq
  for i in "${!S8_REQ_NAME[@]}"; do
    name="${S8_REQ_NAME[$i]}"; want="${S8_REQ_WANT[$i]}"; d="$WORK/s6/$name"
    rq=$(wc -l < "$d/ref/req.log" | tr -d ' '); vq=$(wc -l < "$d/viv/req.log" | tr -d ' ')
    if [ "$rq" != "$want" ] || [ "$vq" != "$want" ]; then
      echo "FAIL $name : $rq requête(s) chez Composer, $vq chez vivacity, $want attendue(s)"; status=1
    fi
  done
  S8_REQ_NAME=(); S8_REQ_WANT=()
}
wait "${S6_JOBS[@]}"
for i in "${!S6_NAME[@]}"; do
  judge "${S6_NAME[$i]}" "${S6_LO[$i]}" "${S6_HI[$i]}" "${S6_WANT[$i]}" "${S6_CMP[$i]}"
done
judge_requests
if grep -q '^  - Downloading ' "$WORK/s6/desync-witness/ref.err"; then
  echo "FAIL desync-witness : Composer télécharge (cas mal choisi)"; status=1
fi
# 8b, suite : l'entrée du store est toujours celle de l'archive A.
entry_file=$(find "$WORK/s6/desync-other-bytes/viv/store" -path '*/probe/desync/*' -name A.php -not -path '*/.staging/*' | head -1)
if [ -z "$entry_file" ] || ! grep -q 'class A {}' "$entry_file"; then
  echo "FAIL desync-other-bytes : l'entrée du store n'est plus celle de l'archive A (${entry_file:-absente})"; status=1
else
  echo "OK   desync-other-bytes : l'entrée du store est restée celle de l'archive A"
fi
# 8j. Le même projet, aussitôt réinstallé sans rien changer (vendor/ en
# place) : aucune requête, et la classmap reste celle de B.
S6_NAME=(); S6_LO=(); S6_HI=(); S6_WANT=(); S6_CMP=(); S6_JOBS=()
mkdir -p "$WORK/s6/desync-other-bytes-noop"
for side in ref viv; do
  ln -s "$WORK/s6/desync-other-bytes/$side" "$WORK/s6/desync-other-bytes-noop/$side"; : > "$WORK/s6/desync-other-bytes/$side/req.log"
done
launch desync-other-bytes-noop 30 "${INSTALL_REF[@]}" -- "${INSTALL_VIV[@]}"; expect desync-other-bytes-noop 0 20 -; requests desync-other-bytes-noop 0
wait "${S6_JOBS[@]}"
for i in "${!S6_NAME[@]}"; do
  judge "${S6_NAME[$i]}" "${S6_LO[$i]}" "${S6_HI[$i]}" "${S6_WANT[$i]}" "${S6_CMP[$i]}"
done
judge_requests
for f in "$WORK/s6/desync-other-bytes-noop/ref/p/vendor/composer/autoload_classmap.php" "$WORK/s6/desync-other-bytes-noop/viv/p/vendor/composer/autoload_classmap.php"; do
  grep -q "Probe\\\\\\\\A2'" "$f" || { echo "FAIL desync-other-bytes-noop : la classmap de $f n'a pas Probe\\A2"; status=1; }
done
# 8m. vivacity seul : vendor/ retiré, le paquet reposé depuis son entrée
# (cas normal, la copie n'est plus étrangère) — la marque s'efface. Pas une
# comparaison avec Composer : point 55a de parity-gaps-026.md.
d="$WORK/s6/desync-other-bytes/viv"; foreign_dir="$d/store/store/.foreign"
if [ -z "$(find "$foreign_dir" -type f 2>/dev/null)" ]; then
  echo "FAIL desync-other-bytes : aucune copie étrangère consignée"; status=1
else
  rm -rf "${d:?}/p/vendor"
  (cd "$d/p" && COMPOSER_HOME="$d/home" COMPOSER_CACHE_DIR="$d/cache" COMPOSER_ROOT_VERSION=dev-main VIVACITY_CACHE_DIR="$d/store" \
    "$VIVACITY" install --no-scripts --no-fallback -o >/dev/null 2>"$d/settle.err") || { echo "FAIL desync-settle : l'install a échoué"; status=1; }
  if [ -n "$(find "$foreign_dir" -type f 2>/dev/null)" ]; then
    echo "FAIL desync-settle : la copie reposée depuis l'entrée est encore consignée étrangère"; status=1
  else
    echo "OK   desync-settle : reposée depuis l'entrée, la copie n'est plus consignée étrangère"
  fi
fi

exit $status
