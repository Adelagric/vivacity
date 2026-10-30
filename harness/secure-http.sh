#!/usr/bin/env bash
# `secure-http` (docs/plans/parity-gaps-026.md), sans réseau.
#
# Composer vérifie chaque URL qu'il va demander (`Config::prohibitUrlByConfig`,
# appelé par `CurlDownloader::initDownload`) : pour une URL `http://`,
# `secure-http` vrai (le défaut) lève « Your configuration does not allow
# connections to … », faux écrit « Warning: Accessing <hôte> over http which
# is an insecure protocol. », une fois par hôte et par instance de `Config`.
#
# Un mandataire HTTP local (python3, `http_proxy` des deux côtés) sert, pour
# n'importe quel hôte, un dépôt `composer` (packages.json, p2) et des zips, et
# journalise chaque requête. Chaque cas part d'une copie neuve, d'un cache de
# fichiers vierge par côté (sauf `@warm`), et compare : code de sortie,
# stdout, stderr entière (moins les lignes `vivacity:` et `Note:` de
# vivacity, ou ce qui précède sa ligne `delegating to`), le projet (lock et
# vendor/), et l'ensemble des requêtes (natif seulement : une délégation
# fait celles de Composer ; un ensemble, pas un multiensemble — `require`
# sans contrainte : Composer relit `p2/acme/b.json` et `b~dev.json` à la
# découverte de version, vivacity non, sans effet sur la sortie). Un cas
# `delegates:<motif>` exige que vivacity rende la commande avec cette raison
# — et sous `--no-fallback`, qu'il sorte en 3 sans rien écrire ni rien
# demander ; un cas natif exige qu'il ne la rende pas. Un cas `expect:<motif>`
# exige le motif dans la stderr de Composer (un cas qui ne montre plus ce
# qu'il teste échoue).
#
# Usage : harness/secure-http.sh [cas...]
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/secure-http"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v python3 >/dev/null || { echo "python3 requis (mandataire)"; exit 1; }
composer --version --no-ansi 2>/dev/null | tail -1
rm -rf "$WORK"; mkdir -p "$WORK/srv/dist" "$WORK/srv/p2/acme" "$WORK/home"
status=0
fail() { echo "FAIL $*"; status=1; }
unset COMPOSER_AUTH HTTP_PROXY HTTPS_PROXY https_proxy NO_PROXY no_proxy CGI_HTTP_PROXY COMPOSER_DISABLE_NETWORK COMPOSER_MAX_PARALLEL_HTTP
# COLUMNS large : l'encadré d'une exception de Composer sur une ligne.
export COLUMNS=400 COMPOSER_HOME="$WORK/home" COMPOSER_NO_INTERACTION=1 COMPOSER_ROOT_VERSION=1.0.0 COMPOSER_NO_AUDIT=1 COMPOSER_TESTS_ARE_RUNNING=1
echo '{}' > "$WORK/home/config.json"

# Les zips : acme/<nom>, un fichier chacun.
NAMES="a b c r s p01 p02 p03 p04 p05 p06 p07 p08 p09 p10 p11 p12 p13 p14"
for n in $NAMES; do
  php -r '
    $z = new ZipArchive();
    $z->open($argv[1], ZipArchive::CREATE) || exit(1);
    $z->addFromString("acme-" . $argv[2] . "/composer.json", "{\"name\": \"acme/" . $argv[2] . "\"}\n");
    $z->addFromString("acme-" . $argv[2] . "/src/X.php", "<?php\n");
    $z->close() || exit(1);
  ' "$WORK/srv/dist/$n.zip" "$n" 2>/dev/null || { echo "zip impossible (ext-zip requise)"; exit 1; }
done
# Le dépôt `composer` (servi à tout hôte) : a et b, dists sur dist.test.
for n in a b c; do
  printf '{"packages": {"acme/%s": [{"name": "acme/%s", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "library", "dist": {"type": "zip", "url": "http://dist.test/dist/%s.zip"}}]}}\n' "$n" "$n" "$n" > "$WORK/srv/p2/acme/$n.json"
done
printf '{"packages": [], "metadata-url": "/p2/%%package%%.json", "available-package-patterns": ["acme/*"]}\n' > "$WORK/srv/packages.json"
# acme/s : servi par le même dépôt, son dist sur le même hôte que lui.
printf '{"packages": {"acme/s": [{"name": "acme/s", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "library", "dist": {"type": "zip", "url": "http://repo.test/dist/s.zip"}}]}}\n' > "$WORK/srv/p2/acme/s.json"
# acme/l : servi par le même dépôt, son dist est un chemin local (aucune
# requête pour lui).
printf '{"packages": {"acme/l": [{"name": "acme/l", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "library", "dist": {"type": "zip", "url": "%s"}}]}}\n' "$WORK/srv/dist/a.zip" > "$WORK/srv/p2/acme/l.json"
# Un dépôt `file://` dont les dists sont en `http://` : connus seulement une
# fois la résolution faite.
mkdir -p "$WORK/srvfile"
printf '{"packages": [], "metadata-url": "file://%s/p2/%%package%%.json", "available-package-patterns": ["acme/*"]}\n' "$WORK/srv" > "$WORK/srvfile/packages.json"

cat > "$WORK/proxy.py" <<'PY'
import http.server, os, re, sys
root, log = sys.argv[1], sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def serve(self):
        with open(log, "a") as f:
            f.write("%s %s\n" % (self.command, self.path))
        m = re.match(r"^http://redir\.test(/.*)$", self.path)
        if m:
            self.send_response(302)
            self.send_header("Location", "http://target.test" + m.group(1))
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        path = re.sub(r"^https?://[^/]+", "", self.path).split("?")[0]
        file = os.path.join(root, path.lstrip("/"))
        if not os.path.isfile(file):
            self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers(); return
        body = open(file, "rb").read()
        self.send_response(200)
        self.send_header("Content-Type", "application/zip" if file.endswith(".zip") else "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    do_GET = serve
    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0") or 0)
        self.rfile.read(n)
        self.serve()
    def log_message(self, *a):
        pass
srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
print(srv.server_address[1], flush=True)
srv.serve_forever()
PY
LOG="$WORK/proxy.log"; : > "$LOG"
python3 -I "$WORK/proxy.py" "$WORK/srv" "$LOG" > "$WORK/proxy.port" 2>"$WORK/proxy.err" &
PROXY_PID=$!
trap 'kill "$PROXY_PID" 2>/dev/null' EXIT
for _ in $(seq 1 50); do [ -s "$WORK/proxy.port" ] && break; sleep 0.1; done
PORT=$(head -1 "$WORK/proxy.port")
[ -n "$PORT" ] || { echo "le mandataire n'a pas démarré"; cat "$WORK/proxy.err"; exit 1; }
export http_proxy="http://127.0.0.1:$PORT"

# Les manifestes de départ.
# pkgrepo <noms…> : dépôts `package`, dist de acme/<nom> sur <hôte>/dist (nom:hôte).
pkgrepo() {
  local out="" spec n h
  for spec in "$@"; do
    n="${spec%%:*}"; h="${spec#*:}"
    out="$out{\"type\": \"package\", \"package\": {\"name\": \"acme/$n\", \"version\": \"1.0.0\", \"dist\": {\"type\": \"zip\", \"url\": \"http://$h/dist/$n.zip\"}}},"
  done
  printf '%s' "$out"
}
reqs() { local out="" n; for n in "$@"; do out="$out\"acme/${n%%:*}\": \"1.0.0\","; done; printf '%s' "${out%,}"; }
# seed <nom> <json> : un projet et son lock, écrit par Composer avec
# `secure-http: false` (le lock ne dépend pas de `config`, content-hash
# compris).
seed() {
  local d="$WORK/seed/$1"; mkdir -p "$d"
  printf '%s\n' "$2" | jq '.config["secure-http"] = false' > "$d/composer.json"
  (cd "$d" && COMPOSER_CACHE_DIR="$WORK/seedcache" composer update --no-install --no-scripts --no-ansi -q) \
    || { echo "le lock de $1 n'a pas pu être écrit"; exit 1; }
  printf '%s\n' "$2" > "$d/composer.json"
}
seed pkg "{\"name\": \"vivacity/secure-http\", \"type\": \"project\", \"repositories\": [$(pkgrepo a:dist.test b:dist.test) {\"packagist.org\": false}], \"require\": {$(reqs a b)}}"
CAP="p01:a.test p02:a.test p03:a.test p04:a.test p05:a.test p06:a.test p07:a.test p08:a.test p09:a.test p10:a.test p11:a.test p12:a.test p13:b.test p14:a.test"
# shellcheck disable=SC2086
seed cap "{\"name\": \"vivacity/secure-http-cap\", \"type\": \"project\", \"repositories\": [$(pkgrepo $CAP) {\"packagist.org\": false}], \"require\": {$(reqs $CAP)}}"
seed repolocal "{\"name\": \"vivacity/secure-http-repolocal\", \"type\": \"project\", \"repositories\": [{\"type\": \"composer\", \"url\": \"http://repo.test\"}, {\"packagist.org\": false}], \"require\": {\"acme/l\": \"1.0.0\"}}"
seed filerepo "{\"name\": \"vivacity/secure-http-filerepo\", \"type\": \"project\", \"repositories\": [{\"type\": \"composer\", \"url\": \"file://$WORK/srvfile\"}, {\"packagist.org\": false}], \"require\": {\"acme/a\": \"1.0.0\"}}"
seed redir "{\"name\": \"vivacity/secure-http-redir\", \"type\": \"project\", \"repositories\": [$(pkgrepo a:dist.test r:redir.test) {\"packagist.org\": false}], \"require\": {$(reqs a r)}}"
seed samehost "{\"name\": \"vivacity/secure-http-samehost\", \"type\": \"project\", \"repositories\": [{\"type\": \"composer\", \"url\": \"http://repo.test\"}, {\"packagist.org\": false}], \"require\": {\"acme/s\": \"1.0.0\"}}"
seed repo "{\"name\": \"vivacity/secure-http-repo\", \"type\": \"project\", \"repositories\": [{\"type\": \"composer\", \"url\": \"http://repo.test\"}, {\"packagist.org\": false}], \"require\": {\"acme/a\": \"1.0.0\", \"acme/b\": \"1.0.0\"}}"

# case <nom> <seed> <préparations> -- <arguments de la commande>
#   préparations : jq:<expr> (composer.json), nolock, warm (cache de fichiers
#   rempli par un install Composer jetable avec `secure-http: false`),
#   env:NOM=VALEUR, global:<json de config> (config.json global du cas),
#   delegates:<motif>, native, expect:<motif>, anyreq (requêtes non comparées),
#   authjson:<json> (auth.json du projet), offline (`--offline` contre
#   `COMPOSER_DISABLE_NETWORK=1`)
case_() {
  local name="$1" seedname="$2"; shift 2
  local preps=() args=()
  while [ "$1" != "--" ]; do preps+=("$1"); shift; done; shift; args=("$@")
  local d="$WORK/case/$name" delegates="" native=0 expect="" envs=() global='{}' warm=0 anyreq=0 offline=0 p
  rm -rf "$d"; mkdir -p "$d"
  for p in "${preps[@]}"; do
    case "$p" in
      delegates:*) delegates="${p#delegates:}" ;;
      native) native=1 ;;
      expect:*) expect="${p#expect:}" ;;
      env:*) envs+=("${p#env:}") ;;
      global:*) global="${p#global:}" ;;
      warm) warm=1 ;;
      anyreq) anyreq=1 ;;
      offline) offline=1 ;;
    esac
  done
  for side in ref viv nofb; do
    mkdir -p "$d/$side"; cp "$WORK/seed/$seedname/composer.json" "$WORK/seed/$seedname/composer.lock" "$d/$side/"
    for p in "${preps[@]}"; do
      case "$p" in
        jq:*) jq "${p#jq:}" "$d/$side/composer.json" > "$d/$side/c.tmp" && mv "$d/$side/c.tmp" "$d/$side/composer.json" ;;
        nolock) rm -f "$d/$side/composer.lock" ;;
        authjson:*) printf '%s\n' "${p#authjson:}" > "$d/$side/auth.json" ;;
      esac
    done
    mkdir -p "$d/cache-$side"
  done
  if [ "$warm" = 1 ]; then
    mkdir -p "$d/warm"; cp "$WORK/seed/$seedname/composer.json" "$WORK/seed/$seedname/composer.lock" "$d/warm/"
    jq '.config["secure-http"] = false' "$d/warm/composer.json" > "$d/warm/c.tmp" && mv "$d/warm/c.tmp" "$d/warm/composer.json"
    (cd "$d/warm" && COMPOSER_CACHE_DIR="$d/cache-warm" composer install --no-scripts --no-ansi -q) || { fail "$name : remplissage du cache"; return; }
    for side in ref viv nofb; do rm -rf "$d/cache-$side"; cp -R "$d/cache-warm" "$d/cache-$side"; done
  fi
  printf '%s\n' "$global" > "$d/config.json"
  run() { # <côté> <commande…>
    local side="$1"; shift
    : > "$LOG"
    (cd "$d/$side" && env "${envs[@]+"${envs[@]}"}" COMPOSER_HOME="$d" COMPOSER_CACHE_DIR="$d/cache-$side" VIVACITY_CACHE_DIR="$d/store-$side" \
      "$@" > "$d/$side.out" 2> "$d/$side.err"); echo $? > "$d/$side.code"
    sort -u "$LOG" > "$d/$side.requests"
  }
  cp "$d/ref/composer.json" "$d/start.json"
  # `offline` : `--offline` pour vivacity, `COMPOSER_DISABLE_NETWORK=1` pour
  # Composer (ce que le repli lui passe).
  local off=() offenv=()
  [ "$offline" = 0 ] || { off=(--offline); offenv=(COMPOSER_DISABLE_NETWORK=1); }
  run ref env "${offenv[@]+"${offenv[@]}"}" composer "${args[@]}" --no-scripts --no-ansi
  run viv "$VIVACITY" "${args[@]}" "${off[@]+"${off[@]}"}" --no-scripts --no-ansi
  run nofb "$VIVACITY" "${args[@]}" "${off[@]+"${off[@]}"}" --no-scripts --no-ansi --no-fallback
  # Une délégation : ce qui précède la ligne `delegating to` (la raison) est
  # de vivacity ; le reste, de Composer.
  if grep -q '^vivacity: delegating to ' "$d/viv.err"; then
    sed '1,/^vivacity: delegating to /d' "$d/viv.err" > "$d/viv.err.cmp"
  else
    grep -vE '^(vivacity|Note): ' "$d/viv.err" > "$d/viv.err.cmp"
  fi
  local bad=""
  [ -z "$expect" ] || grep -qE -- "$expect" "$d/ref.err" || bad="$bad Composer n'écrit pas « $expect » (cas mal choisi);"
  [ "$(cat "$d/ref.code")" = "$(cat "$d/viv.code")" ] || bad="$bad codes $(cat "$d/ref.code") vs $(cat "$d/viv.code");"
  cmp -s "$d/ref.out" "$d/viv.out" || bad="$bad stdout différente;"
  cmp -s "$d/ref.err" "$d/viv.err.cmp" || bad="$bad stderr différente;"
  compare_vendor "$d/ref" "$d/viv" "$d/vendor.diff" >/dev/null || bad="$bad projet différent;"
  if [ -n "$delegates" ]; then
    { grep -q 'delegating to' "$d/viv.err" && grep -qE -- "$delegates" "$d/viv.err"; } \
      || bad="$bad vivacity ne rend pas la commande (« $delegates »);"
    [ "$(cat "$d/nofb.code")" = 3 ] || bad="$bad --no-fallback sort en $(cat "$d/nofb.code"), pas 3;"
    grep -qE -- "$delegates" "$d/nofb.err" || bad="$bad --no-fallback ne nomme pas « $delegates »;"
    [ ! -s "$d/nofb.requests" ] || bad="$bad --no-fallback a fait des requêtes;"
    cmp -s "$d/nofb/composer.json" "$d/start.json" || bad="$bad --no-fallback a touché composer.json;"
    { [ ! -e "$WORK/seed/$seedname/composer.lock" ] || [ ! -e "$d/nofb/composer.lock" ] || cmp -s "$d/nofb/composer.lock" "$WORK/seed/$seedname/composer.lock"; } \
      || bad="$bad --no-fallback a touché composer.lock;"
    [ ! -e "$d/nofb/vendor" ] || bad="$bad --no-fallback a écrit vendor/;"
  fi
  if [ "$native" = 1 ]; then
    ! grep -q 'delegating to' "$d/viv.err" || bad="$bad vivacity a rendu la commande ($(grep -m1 'delegating' "$d/viv.err"));"
    [ "$anyreq" = 1 ] || cmp -s "$d/ref.requests" "$d/viv.requests" || bad="$bad requêtes différentes;"
  fi
  if [ -z "$bad" ]; then
    echo "OK   $name : même code ($(cat "$d/ref.code")), même sortie, même projet$([ -n "$delegates" ] && echo ', rendu à Composer')$([ "$native" = 1 ] && echo ', natif, mêmes requêtes')"
  else
    fail "$name :$bad"
    diff "$d/ref.err" "$d/viv.err.cmp" | head -10
    [ "$native" = 1 ] && diff "$d/ref.requests" "$d/viv.requests" | head -6
  fi
}

only() { [ $# -eq 0 ] && return 0; local c; for c in $SELECTED; do [ "$c" = "$1" ] && return 0; done; return 1; }
SELECTED="$*"
want() { [ -z "$SELECTED" ] || only "$1"; }

# 1. Dists `http://` installés depuis le lock.
want dist-default && case_ dist-default pkg 'delegates:secure-http' 'expect:does not allow connections to http://dist.test/dist/a.zip' -- install
want dist-false && case_ dist-false pkg native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing dist.test over http' -- install
want dist-global && case_ dist-global pkg native 'global:{"config": {"secure-http": false}}' 'expect:^Warning: Accessing dist.test over http' -- install
want dist-warm && case_ dist-warm pkg native warm 'jq:.config["secure-http"]=false' -- install
want dist-warm-default && case_ dist-warm-default pkg 'delegates:secure-http' warm -- install
want dist-dry-run && case_ dist-dry-run pkg native -- install --dry-run
want dist-auth-env && case_ dist-auth-env pkg 'delegates:COMPOSER_AUTH' 'env:COMPOSER_AUTH={"secure-http": false}' 'expect:^Warning: Accessing' -- install
# `secure-http` lu ailleurs que dans config : l'auth.json du projet, ou
# COMPOSER_AUTH, fusionné en dernier (il l'emporte sur le projet).
want dist-auth-json && case_ dist-auth-json pkg 'delegates:auth.json' 'authjson:{"secure-http": false}' 'expect:^Warning: Accessing' -- install
want dist-auth-env-wins && case_ dist-auth-env-wins pkg 'delegates:COMPOSER_AUTH' 'jq:.config["secure-http"]=false' 'env:COMPOSER_AUTH={"secure-http": true}' 'expect:does not allow connections' -- install
# Hors ligne, cache chaud : aucune requête, ni refus ni avertissement.
want dist-offline && case_ dist-offline pkg native offline warm -- install
want dist-offline-false && case_ dist-offline-false pkg native offline warm 'jq:.config["secure-http"]=false' -- install
# Une redirection vers un autre hôte http : averti quand la boucle d'attente
# relance la requête, après la dernière ligne « Downloading ».
want dist-redirect && case_ dist-redirect redir native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing target.test' -- install
want dist-disable-tls && case_ dist-disable-tls pkg 'delegates:disable-tls' 'jq:.config["disable-tls"]=true' -- install
# Douze téléchargements démarrent tout de suite, le treizième attend
# (`HttpDownloader::$maxJobs`) : son hôte est averti après la dernière ligne.
want dist-cap && case_ dist-cap cap native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing b.test' -- install
want dist-cap-1 && case_ dist-cap-1 cap native 'jq:.config["secure-http"]=false' 'env:COMPOSER_MAX_PARALLEL_HTTP=1' 'expect:^Warning: Accessing b.test' -- install
# 2. Un dépôt `composer` en `http://`.
want repo-default && case_ repo-default repo 'delegates:secure-http' 'expect:does not allow connections to http://repo.test/packages.json' -- update --no-install
want repo-false && case_ repo-false repo native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing repo.test over http' -- update
want repo-allowlist && case_ repo-allowlist repo native 'jq:.config["secure-http"]=false' 'expect:listed for update is not locked' -- update acme/zzz --no-install
# (`anyreq` : la découverte de version de Composer lit aussi
# `p2/acme/b~dev.json`, pas celle de vivacity — même hôte, même sortie.)
want repo-require && case_ repo-require repo native anyreq 'jq:.config["secure-http"]=false|del(.require["acme/b"])' 'expect:^Warning: Accessing repo.test' -- require acme/b --no-install
want repo-remove && case_ repo-remove repo native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing repo.test' -- remove acme/b --no-install
# Caches chauds (dépôt et fichiers) : la requête conditionnelle part quand même.
want repo-remove-warm && case_ repo-remove-warm repo native warm 'jq:.config["secure-http"]=false' -- remove acme/b --no-install
want repo-update-warm && case_ repo-update-warm repo native warm 'jq:.config["secure-http"]=false' -- update
# `remove` défaut : rendu avant d'éditer composer.json (aucune ligne de
# vivacity que Composer réimprimerait).
want repo-remove-default && case_ repo-remove-default repo 'delegates:secure-http' 'expect:does not allow connections to http://repo.test/packages.json' -- remove acme/b --no-install
# Le dépôt du config.json global : rendu à l'ouverture de la session.
want repo-global-default && case_ repo-global-default repo 'delegates:secure-http' 'jq:del(.repositories)' 'global:{"repositories": [{"type": "composer", "url": "http://repo.test"}, {"packagist.org": false}]}' 'expect:does not allow connections to http://repo.test/packages.json' -- update --no-install
want repo-require-default && case_ repo-require-default repo 'delegates:secure-http' 'jq:del(.require["acme/b"])' -- require acme/b --no-install
# Un dépôt et ses dists sur le même hôte : un avertissement pour les deux,
# de `update` à son install comme de la vérification du lock à l'install.
want samehost-update && case_ samehost-update samehost native nolock 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing repo.test' -- update
want samehost-install && case_ samehost-install samehost native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing repo.test' -- install
# 3. `install` avec ce dépôt : le filtre de listes lit packages.json.
want install-repo-false && case_ install-repo-false repo native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing repo.test' -- install
want install-repo-default && case_ install-repo-default repo 'delegates:secure-http' 'expect:Filter list data could not be fetched' -- install
# … le refus rattrapé par le filtre de listes (`ignore-unreachable`), le
# dist local installé : natif, l'avertissement de Composer mot pour mot.
want install-repo-default-local && case_ install-repo-default-local repolocal native 'expect:^  - Your configuration does not allow connections to http://repo.test/packages.json' -- install
# 4. Des dists `http://` connus une fois la résolution faite (dépôt
# `file://`) : Composer écrirait le lock, puis s'arrêterait au premier.
want update-file-repo && case_ update-file-repo filerepo 'delegates:secure-http' 'expect:does not allow connections to http://dist.test/dist/a.zip' -- update
want update-file-repo-false && case_ update-file-repo-false filerepo native 'jq:.config["secure-http"]=false' 'expect:^Warning: Accessing dist.test' -- update
exit $status
