#!/usr/bin/env bash
# Où part un jeton `github-oauth`, et ce qu'il change à la sortie
# (docs/plans/ci-github-token.md).
#
# Composer n'envoie le jeton de github.com qu'aux URL `^https?://api.github.com/`
# (`AuthHelper::addAuthenticationOptions`, origine `Url::getOrigin`) : ni à
# codeload.github.com (son origine propre), ni à github.com, ni aux autres
# `*.github.com` ; une redirection est une nouvelle requête dont l'origine
# est recalculée ; un domaine GitHub Enterprise reçoit le jeton en Basic
# (`jeton:x-oauth-basic`). COMPOSER_AUTH est fusionné après l'auth.json du
# projet (`Factory::loadComposerAuthEnv`, en dernier).
#
# Pour voir les en-têtes sans réseau ni TLS : des dists en `http://`
# (`secure-http: false`) et un mandataire HTTP local (python3, `http_proxy`
# des deux côtés) qui journalise chaque requête (URL absolue, en-tête
# `Authorization`), répond 302 vers codeload pour un zipball de l'API, et
# sert un zip pour le reste. Aucune requête ne quitte la machine.
#
# Comparés, par cas : le journal du mandataire (trié : les téléchargements
# sont parallèles), le code de sortie, stdout, stderr (moins « Warning:
# Accessing <hôte> over http… », que Composer écrit et vivacity non — un
# écart connu de `secure-http`, hors de ce banc — et les lignes `vivacity:`),
# le projet. Puis, outil par outil, le cas sans jeton contre les cas avec :
# le jeton ne change rien à la sortie. Le cas `env` affirme aussi le
# journal attendu ligne à ligne.
#
# Usage : harness/auth-hosts.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/auth-hosts"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication des zips)"; exit 1; }
command -v python3 >/dev/null || { echo "python3 requis (mandataire)"; exit 1; }
composer --version --no-ansi 2>/dev/null | tail -1
rm -rf "$WORK"; mkdir -p "$WORK/zips" "$WORK/home"
status=0
fail() { echo "FAIL $*"; status=1; }
# Rien de la machine ne doit s'en mêler : ni jeton, ni mandataire, ni
# exception de mandataire.
unset COMPOSER_AUTH HTTP_PROXY HTTPS_PROXY https_proxy NO_PROXY no_proxy CGI_HTTP_PROXY
export COMPOSER_HOME="$WORK/home" COMPOSER_NO_INTERACTION=1 COMPOSER_ROOT_VERSION=dev-main COMPOSER_NO_AUDIT=1

# Les paquets : nom court, URL du dist. Le zip servi est choisi par le nom
# court trouvé dans le chemin (`acme/<nom>`).
PKGS="api|http://api.github.com/repos/acme/api/zipball/1111111111111111111111111111111111111111
codeload|http://codeload.github.com/acme/codeload/legacy.zip/2222222222222222222222222222222222222222
web|http://github.com/acme/web/archive/3333333333333333333333333333333333333333.zip
objects|http://objects.github.com/acme/objects/4444.zip
ghe|http://ghe.example.test/api/v3/repos/acme/ghe/zipball/5555555555555555555555555555555555555555"

repos=""; reqs=""
while IFS='|' read -r name url; do
  php -r '
    $z = new ZipArchive();
    $z->open($argv[1], ZipArchive::CREATE) || exit(1);
    $z->addFromString("acme-" . $argv[2] . "-abc/composer.json", "{\"name\": \"acme/" . $argv[2] . "\"}\n");
    $z->addFromString("acme-" . $argv[2] . "-abc/src/" . ucfirst($argv[2]) . ".php", "<?php\n");
    $z->close() || exit(1);
  ' "$WORK/zips/$name.zip" "$name" 2>/dev/null || { echo "zip impossible (ext-zip requise)"; exit 1; }
  repos="$repos{\"type\": \"package\", \"package\": {\"name\": \"acme/$name\", \"version\": \"1.0.0\", \"dist\": {\"type\": \"zip\", \"url\": \"$url\"}}},"
  reqs="$reqs\"acme/$name\": \"1.0.0\","
done <<EOF
$PKGS
EOF
cat > "$WORK/seed.json" <<J
{"name": "acme/proj", "type": "project",
 "repositories": [${repos} {"packagist.org": false}],
 "require": {${reqs%,}},
 "config": {"secure-http": false}}
J

# Le mandataire : journal « URL<TAB>Authorization » (`-` sans en-tête).
cat > "$WORK/proxy.py" <<'PY'
import http.server, os, re, sys
zips, log = sys.argv[1], sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        with open(log, "a") as f:
            f.write("%s\t%s\n" % (self.path, self.headers.get("Authorization", "-")))
        m = re.match(r"^http://api\.github\.com/repos/([^/]+)/([^/]+)/zipball/(.+)$", self.path)
        if m:
            self.send_response(302)
            self.send_header("Location", "http://codeload.github.com/%s/%s/legacy.zip/%s" % m.groups())
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        m = re.search(r"/acme/([a-z]+)/", self.path)
        path = os.path.join(zips, (m.group(1) if m else "") + ".zip")
        if not m or not os.path.isfile(path):
            self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers(); return
        body = open(path, "rb").read()
        self.send_response(200)
        self.send_header("Content-Type", "application/zip")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass
srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
print(srv.server_address[1], flush=True)
srv.serve_forever()
PY
LOG="$WORK/proxy.log"; : > "$LOG"
python3 -I "$WORK/proxy.py" "$WORK/zips" "$LOG" > "$WORK/proxy.port" 2>"$WORK/proxy.err" &
PROXY_PID=$!
trap 'kill "$PROXY_PID" 2>/dev/null' EXIT
for _ in $(seq 1 50); do [ -s "$WORK/proxy.port" ] && break; sleep 0.1; done
PORT=$(head -1 "$WORK/proxy.port")
[ -n "$PORT" ] || { echo "le mandataire n'a pas démarré"; cat "$WORK/proxy.err"; exit 1; }
export http_proxy="http://127.0.0.1:$PORT"

# Le lock, écrit une fois par Composer sans jeton (aucun téléchargement).
mkdir -p "$WORK/seed"; cp "$WORK/seed.json" "$WORK/seed/composer.json"
(cd "$WORK/seed" && COMPOSER_CACHE_DIR="$WORK/seedcache" composer update --no-install --no-scripts --no-ansi -q) \
  || { echo "composer update a échoué"; exit 1; }

T_ENV='{"github-oauth": {"github.com": "T-ENV", "ghe.example.test": "T-GHE"}}'

# run_case <cas> <outil> : projet neuf, cache vierge par côté, journal à part.
run_case() {
  local c="$1" tool="$2" d="$WORK/$1/$2" env_auth=""
  mkdir -p "$d"; cp "$WORK/seed/composer.json" "$WORK/seed/composer.lock" "$d/"
  case "$c" in
    anon) ;;
    env) env_auth="$T_ENV" ;;
    order)
      env_auth="$T_ENV"
      echo '{"github-oauth": {"github.com": "T-PROJECT"}}' > "$d/auth.json" ;;
  esac
  : > "$LOG"
  if [ "$tool" = composer ]; then
    (cd "$d" && COMPOSER_AUTH="$env_auth" COMPOSER_CACHE_DIR="$WORK/$c/cache-$tool" \
      composer install --no-scripts --no-ansi > "$d.out" 2> "$d.err"); echo $? > "$d.code"
  else
    (cd "$d" && COMPOSER_AUTH="$env_auth" COMPOSER_CACHE_DIR="$WORK/$c/cache-$tool" VIVACITY_CACHE_DIR="$WORK/$c/store" \
      "$VIVACITY" install --no-scripts --no-fallback > "$d.out" 2> "$d.err"); echo $? > "$d.code"
  fi
  sort "$LOG" > "$d.requests"
  grep -vE '^Warning: Accessing [^ ]+ over http which is an insecure protocol\.$|^vivacity: ' "$d.err" > "$d.err.cmp"
  # Le projet seul : auth.json est une entrée du cas, pas un produit.
  rm -f "$d/auth.json"
}

for c in anon env order; do
  run_case "$c" composer
  run_case "$c" vivacity
  r="$WORK/$c/composer" v="$WORK/$c/vivacity"
  bad=""
  [ "$(cat "$r.code")" = "$(cat "$v.code")" ] || bad="$bad codes $(cat "$r.code") vs $(cat "$v.code");"
  [ "$(cat "$r.code")" = 0 ] || bad="$bad Composer sort en $(cat "$r.code");"
  cmp -s "$r.requests" "$v.requests" || bad="$bad requêtes différentes;"
  cmp -s "$r.out" "$v.out" || bad="$bad stdout différente;"
  cmp -s "$r.err.cmp" "$v.err.cmp" || bad="$bad stderr différente;"
  compare_vendor "$r" "$v" "$WORK/$c.vendor.diff" >/dev/null || bad="$bad projet différent;"
  if [ -z "$bad" ]; then
    echo "OK   $c : mêmes requêtes et mêmes en-têtes, même sortie, même projet"
  else
    fail "$c :$bad"
    diff "$r.requests" "$v.requests" | head -8
    diff "$r.err.cmp" "$v.err.cmp" | head -6
  fi
done

# Ce que Composer a fait, écrit en clair : le jeton à l'API seulement, la
# redirection vers codeload sans en-tête, le domaine GHE en Basic.
ghe_basic="Basic $(printf 'T-GHE:x-oauth-basic' | base64)"
sort > "$WORK/env.expected" <<EOF
http://api.github.com/repos/acme/api/zipball/1111111111111111111111111111111111111111	token T-ENV
http://codeload.github.com/acme/api/legacy.zip/1111111111111111111111111111111111111111	-
http://codeload.github.com/acme/codeload/legacy.zip/2222222222222222222222222222222222222222	-
http://github.com/acme/web/archive/3333333333333333333333333333333333333333.zip	-
http://objects.github.com/acme/objects/4444.zip	-
http://ghe.example.test/api/v3/repos/acme/ghe/zipball/5555555555555555555555555555555555555555	$ghe_basic
EOF
for tool in composer vivacity; do
  if cmp -s "$WORK/env.expected" "$WORK/env/$tool.requests"; then
    echo "OK   env ($tool) : le journal attendu, ligne à ligne"
  else
    fail "env ($tool) : journal inattendu"; diff "$WORK/env.expected" "$WORK/env/$tool.requests" | head -8
  fi
done
# L'ordre des sources : COMPOSER_AUTH l'emporte sur l'auth.json du projet.
if grep -q $'^http://api.github.com/.*\ttoken T-ENV$' "$WORK/order/composer.requests"; then
  echo "OK   order (composer) : COMPOSER_AUTH passe après l'auth.json du projet"
else
  fail "order (composer) : $(grep '^http://api' "$WORK/order/composer.requests")"
fi

# Le jeton ne change rien à la sortie, d'un côté comme de l'autre.
for tool in composer vivacity; do
  for c in env order; do
    if cmp -s "$WORK/anon/$tool.out" "$WORK/$c/$tool.out" && cmp -s "$WORK/anon/$tool.err.cmp" "$WORK/$c/$tool.err.cmp" \
       && compare_vendor "$WORK/anon/$tool" "$WORK/$c/$tool" "$WORK/$tool-$c.vendor.diff" >/dev/null; then
      echo "OK   $tool : sans jeton = $c (stdout, stderr, projet)"
    else
      fail "$tool : la sortie change avec le jeton ($c)"; diff "$WORK/anon/$tool.err.cmp" "$WORK/$c/$tool.err.cmp" | head -6
    fi
  done
done

exit $status
