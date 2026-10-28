#!/usr/bin/env bash
# Paquets sans dist, servis par un dépôt git (docs/plans/git-sources.md) :
# Composer 2.10.3 les clone (GitDownloader) et laisse leur `.git` dans
# vendor/ ; vivacity lance les mêmes commandes git.
#
# Un dépôt nu fabriqué ici (deux commits, tag 1.0.0, un fichier exécutable,
# un `bin`) sert les paquets de dépôts `package` :
#   - en chemin local absolu : pas de cache, `Cloning <ref>` ;
#   - en `https://git.example.invalid/…` et `https://github.com/acme/…`,
#     réécrits vers ce dépôt par `url.<base>.insteadOf` dans la
#     configuration git du banc : Composer les traite en URL distantes
#     (miroir dans le cache VCS, `Syncing … into cache`, `pushurl` GitHub),
#     git les lit sur le disque — aucun réseau.
# Plus un cas réel (github.com) : krixon/xbmc-php-rpc et
# shaarli/netscape-bookmark-parser, les paquets d'ampache et de
# kovah-linkace (`GIT_SOURCES_OFFLINE=1` le saute).
#
# Chaque côté a SON cache VCS, au MÊME chemin (le reflog du clone porte ce
# chemin) : celui de Composer est mis de côté avant le passage de vivacity,
# qui part donc lui aussi d'un cache froid ; les deux miroirs sont comparés
# ensuite (config, refs, objets). Le cas « chaud » donne aux deux une copie
# du même cache.
#
# Comparés : code de sortie, projet entier (`compare_vendor`, les `.git` du
# vendor compris selon la décision du plan), stdout et stderr entières (la
# ligne `vivacity:` exceptée). Les replis : `--no-fallback` sort en 3 sans
# rien écrire, et le repli par défaut laisse le projet de Composer.
#
# Usage : harness/git-sources.sh [cas…]
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/git-sources"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v git >/dev/null || { echo "git requis"; exit 1; }
command -v php >/dev/null || { echo "php requis (fabrication du zip)"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd -P)"
status=0
only="${*:-}"

harness_git_env "$WORK"
export COMPOSER_HOME="$WORK/home" COMPOSER_NO_INTERACTION=1 COMPOSER_ROOT_VERSION=dev-main
unset COMPOSER_CACHE_VCS_DIR COMPOSER_DISABLE_NETWORK COMPOSER_PROCESS_TIMEOUT
CACHE="$WORK/cache"

# Le dépôt nu, ses deux commits datés (identifiants stables d'un run à
# l'autre), et la réécriture des deux URL distantes vers lui.
REPO="$WORK/repos/lib.git"
git init -q --bare -b master "$REPO"
git clone -q "$REPO" "$WORK/repos/work" 2>/dev/null
(
  cd "$WORK/repos/work" || exit 1
  mkdir -p src bin
  printf '<?php\nnamespace Acme;\nclass A {}\n' > src/A.php
  printf '#!/bin/sh\necho tool\n' > bin/tool; chmod 0755 bin/tool
  printf '{"name": "acme/lib", "autoload": {"psr-4": {"Acme\\\\": "src/"}}}\n' > composer.json
  git add -A
  GIT_AUTHOR_DATE='2020-01-01T00:00:00Z' GIT_COMMITTER_DATE='2020-01-01T00:00:00Z' \
    git -c user.name=T -c user.email=t@e commit -qm one
  git tag 1.0.0
  printf '<?php\nnamespace Acme;\nclass B {}\n' > src/B.php
  git add -A
  GIT_AUTHOR_DATE='2020-01-02T00:00:00Z' GIT_COMMITTER_DATE='2020-01-02T00:00:00Z' \
    git -c user.name=T -c user.email=t@e commit -qm two
  git push -q origin master --tags
) || { echo "FAIL : fabrication du dépôt"; exit 1; }
TAG_REF="$(git --git-dir="$REPO" rev-parse 1.0.0^{commit})"
HEAD_REF="$(git --git-dir="$REPO" rev-parse master)"
REMOTE_URL="https://git.example.invalid/acme/lib.git"
GITHUB_URL="https://github.com/acme/lib.git"
cat > "$WORK/gitconfig" <<EOF
[url "file://$REPO"]
	insteadOf = $REMOTE_URL
	insteadOf = $GITHUB_URL
EOF
export GIT_CONFIG_GLOBAL="$WORK/gitconfig"
# L'état ancien du dépôt, pour le cache rassis : master au premier commit.
git clone -q --bare "$REPO" "$WORK/repos/old.git"
git --git-dir="$WORK/repos/old.git" update-ref refs/heads/master "$TAG_REF"
sed "s#file://$REPO#file://$WORK/repos/old.git#" "$WORK/gitconfig" > "$WORK/gitconfig-old"

# Un zip local pour le cas mixte (dist + sources).
mkdir -p "$WORK/zips"
php -r '$z = new ZipArchive(); $z->open($argv[1], ZipArchive::CREATE); $z->addFromString("zz/composer.json", "{\"name\": \"acme/zz\"}\n"); $z->addFromString("zz/Z.php", "<?php class Z {}\n"); $z->close();' "$WORK/zips/zz.zip" \
  || { echo "FAIL : fabrication du zip"; exit 1; }

# pkg <nom> <version> <url> <référence> [bin] : une entrée de dépôt
# `package` à source git seule ; le `bin` (celui du dépôt du banc) sur
# demande — deux paquets qui le déclarent tous deux feraient écrire à
# Composer « name conflicts » au milieu des opérations, une ligne que
# vivacity écrit après elles (écart antérieur, sans rapport avec git).
pkg() {
  local bin=""
  [ -z "${5:-}" ] || bin=', "bin": ["bin/tool"]'
  printf '{"type": "package", "package": {"name": "%s", "version": "%s", "source": {"type": "git", "url": "%s", "reference": "%s"}, "autoload": {"psr-4": {"Acme\\\\": "src/"}}%s}}' "$1" "$2" "$3" "$4" "$bin"
}

# seed <nom> <dépôts JSON> <require JSON> [require-dev JSON] : le projet et
# son lock, résolu par Composer (cache à part, jamais celui des cas).
seed() {
  local name="$1" repos="$2" req="$3" dev="${4:-}" d="$WORK/seeds/$1"
  [ -n "$dev" ] || dev='{}' 
  mkdir -p "$d"
  printf '{"repositories": [%s, {"packagist.org": false}], "require": %s, "require-dev": %s, "config": {"allow-plugins": false}}\n' \
    "$repos" "$req" "$dev" > "$d/composer.json"
  if ! (cd "$d" && COMPOSER_CACHE_DIR="$WORK/seedcache" composer update --no-install --no-scripts --no-ansi >"$d/seed.log" 2>&1); then
    echo "FAIL : Composer n'a pas résolu le projet $name"; sed -n '1,6p' "$d/seed.log"; status=1; return 1
  fi
}

seed local-tag "$(pkg acme/lib 1.0.0 "$REPO" "$TAG_REF" bin)" '{"acme/lib": "1.0.0"}'
seed local-dev "$(pkg acme/lib dev-master "$REPO" "$HEAD_REF" bin)" '{"acme/lib": "dev-master"}'
seed local-branch "$(pkg acme/lib dev-master "$REPO" master)" '{"acme/lib": "dev-master"}'
seed cache-tag "$(pkg acme/lib 1.0.0 "$REMOTE_URL" "$TAG_REF" bin)" '{"acme/lib": "1.0.0"}'
seed cache-dev "$(pkg acme/lib dev-master "$REMOTE_URL" "$HEAD_REF" bin)" '{"acme/lib": "dev-master"}'
seed cache-branch "$(pkg acme/lib dev-master "$REMOTE_URL" master)" '{"acme/lib": "dev-master"}'
seed github "$(pkg acme/lib 1.0.0 "$GITHUB_URL" "$TAG_REF")" '{"acme/lib": "1.0.0"}'
seed mixed "{\"type\": \"package\", \"package\": {\"name\": \"acme/aa\", \"version\": \"1.0.0\", \"dist\": {\"type\": \"zip\", \"url\": \"$WORK/zips/zz.zip\"}}}, $(pkg acme/lib 1.0.0 "$REMOTE_URL" "$TAG_REF" bin), {\"type\": \"package\", \"package\": {\"name\": \"acme/zz\", \"version\": \"1.0.0\", \"dist\": {\"type\": \"zip\", \"url\": \"$WORK/zips/zz.zip\"}}}, $(pkg acme/loc 1.0.0 "$REPO" "$TAG_REF")" \
  '{"acme/aa": "1.0.0", "acme/lib": "1.0.0", "acme/zz": "1.0.0", "acme/loc": "1.0.0"}'
seed no-dev "$(pkg acme/lib 1.0.0 "$REPO" "$TAG_REF"), $(pkg acme/dev dev-master "$REMOTE_URL" "$HEAD_REF")" \
  '{"acme/lib": "1.0.0"}' '{"acme/dev": "dev-master"}'
seed empty "{\"type\": \"package\", \"package\": {\"name\": \"acme/aa\", \"version\": \"1.0.0\", \"dist\": {\"type\": \"zip\", \"url\": \"$WORK/zips/zz.zip\"}}}" '{"acme/aa": "1.0.0"}'
seed gone "$(pkg acme/lib 1.0.0 "$REMOTE_URL" 0123456789abcdef0123456789abcdef01234567)" '{"acme/lib": "1.0.0"}'
if [ -z "${GIT_SOURCES_OFFLINE:-}" ]; then
  seed real "$(pkg krixon/xbmc-php-rpc dev-master https://github.com/krixon/xbmc-php-rpc.git master), $(pkg shaarli/netscape-bookmark-parser dev-master https://github.com/Kovah/netscape-bookmark-parser 26d391ce73d54ee7fe279d6a72138d122d5f4f32)" \
    '{"krixon/xbmc-php-rpc": "dev-master", "shaarli/netscape-bookmark-parser": "dev-master"}'
fi

wanted() { [ -z "$only" ] || case " $only " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# compare_mirrors <cache A> <cache B> : les miroirs du cache VCS, par ce qui
# les définit (les packs d'un `clone --mirror` sont ceux du serveur, mais un
# `remote update` peut en ajouter d'autres).
compare_mirrors() {
  local a="$1/vcs" b="$2/vcs" m out=""
  [ "$(ls -A "$a" 2>/dev/null)" = "$(ls -A "$b" 2>/dev/null)" ] || { echo "miroirs différents : $(ls -A "$a" 2>/dev/null | tr '\n' ' ')/ $(ls -A "$b" 2>/dev/null | tr '\n' ' ')"; return 1; }
  for m in $(ls -A "$a" 2>/dev/null); do
    for probe in "config --file config --list" "for-each-ref" "symbolic-ref HEAD" "cat-file --batch-all-objects --batch-check"; do
      # shellcheck disable=SC2086
      if [ "$(cd "$a/$m" && git $probe 2>&1)" != "$(cd "$b/$m" && git $probe 2>&1)" ]; then
        out="$out $m:$probe"
      fi
    done
  done
  [ -z "$out" ] || { echo "miroirs : différences ($out)"; return 1; }
}

# side <ref|viv> <répertoire> <commande…> : un install (ou la commande
# donnée) d'un côté, ses sorties dans <répertoire>/../<côté>.{out,err,code}.
side() {
  local who="$1" dir="$2"; shift 2
  local base; base="$(dirname "$dir")/$who"
  if [ "$who" = ref ]; then
    (cd "$dir" && COMPOSER_CACHE_DIR="$CACHE" COMPOSER_TESTS_ARE_RUNNING=1 composer "$@" --no-ansi >"$base.out" 2>"$base.err")
  else
    (cd "$dir" && COMPOSER_CACHE_DIR="$CACHE" VIVACITY_CACHE_DIR="$WORK/store" "$VIVACITY" "$@" >"$base.out" 2>"$base.err")
  fi
  echo $? > "$base.code"
}

# run_case <nom> <seed> <froid|chaud> <arguments d'install…> ; SECOND="<commande>"
# pour une seconde commande des deux côtés (seules ses sorties sont
# comparées), VIV_ARGS pour ce que vivacity reçoit en plus (`--no-fallback`
# par défaut).
run_case() {
  local name="$1" seedname="$2" cache="$3"; shift 3
  wanted "$name" || return 0
  local c="$WORK/cases/$name" ok=1
  mkdir -p "$c/ref" "$c/viv"
  cp "$WORK/seeds/$seedname/composer.json" "$WORK/seeds/$seedname/composer.lock" "$c/ref/"
  cp "$WORK/seeds/$seedname/composer.json" "$WORK/seeds/$seedname/composer.lock" "$c/viv/"
  # Le cache chaud : réchauffé par un install de Composer jeté ensuite ;
  # « rassis » : réchauffé sur un état plus ancien du dépôt (master au
  # premier commit), que la synchronisation doit compléter — des objets
  # libres dans le miroir, que `clone --dissociate` réempaquette.
  local warm="$WORK/warm/$cache-$seedname" warmcfg="$GIT_CONFIG_GLOBAL"
  [ "$cache" != rassis ] || warmcfg="$WORK/gitconfig-old"
  if [ "$cache" != froid ] && [ ! -d "$warm" ]; then
    mkdir -p "$WORK/warmrun/$cache-$seedname"
    cp "$WORK/seeds/$seedname/composer.json" "$WORK/seeds/$seedname/composer.lock" "$WORK/warmrun/$cache-$seedname/"
    rm -rf "$CACHE"
    (cd "$WORK/warmrun/$cache-$seedname" && GIT_CONFIG_GLOBAL="$warmcfg" COMPOSER_CACHE_DIR="$CACHE" composer install --no-ansi >/dev/null 2>&1)
    mkdir -p "$WORK/warm"; mv "$CACHE" "$warm" || { echo "FAIL : cache chaud non construit"; status=1; }
  fi
  local who
  for who in ref viv; do
    rm -rf "$CACHE"
    if [ "$cache" != froid ]; then cp -pR "$warm" "$CACHE"; fi
    if [ "$who" = ref ]; then
      side ref "$c/ref" install --no-scripts "$@"
      [ -z "${SECOND:-}" ] || { cp "$c/ref.err" "$c/ref.first.err"; side ref "$c/ref" $SECOND; }
    else
      # shellcheck disable=SC2086
      side viv "$c/viv" install --no-scripts ${VIV_ARGS:---no-fallback} "$@"
      # shellcheck disable=SC2086
      [ -z "${SECOND:-}" ] || { cp "$c/viv.err" "$c/viv.first.err"; side viv "$c/viv" $SECOND ${VIV_ARGS:---no-fallback}; }
    fi
    mv "$CACHE" "$c/cache-$who" 2>/dev/null || mkdir -p "$c/cache-$who"
  done
  local rc vc
  rc="$(cat "$c/ref.code")"; vc="$(cat "$c/viv.code")"
  if [ "$rc" != "$vc" ]; then
    echo "FAIL $name : codes $rc (composer) / $vc (vivacity)"; tail -5 "$c/viv.err"; tail -3 "$c/ref.err"; ok=0
  fi
  if ! compare_vendor "$c/ref" "$c/viv" "$c/project.diff" >/dev/null; then
    echo "FAIL $name : projet différent"; head -12 "$c/project.diff"; ok=0
  fi
  if ! diff "$c/ref.out" "$c/viv.out" >"$c/out.diff"; then
    echo "FAIL $name : stdout différente"; head -8 "$c/out.diff"; ok=0
  fi
  # Seule la durée de résolution (`-v`) est masquée : mesurée, jamais égale.
  if ! diff <(sed -E 's/ in [0-9]+\.[0-9]{3} seconds/ in N seconds/' "$c/ref.err") \
      <(grep -v '^vivacity: ' "$c/viv.err" | sed -E 's/ in [0-9]+\.[0-9]{3} seconds/ in N seconds/') >"$c/err.diff"; then
    echo "FAIL $name : stderr différente"; head -10 "$c/err.diff"; ok=0
  fi
  if ! compare_mirrors "$c/cache-ref" "$c/cache-viv"; then
    echo "FAIL $name : cache VCS différent"; ok=0
  fi
  if [ "$ok" = 1 ]; then echo "OK   $name"; else status=1; fi
}

# expect_line <cas> <côté> <motif> : la sortie d'erreur porte la ligne
# (preuve que le cas exerce bien ce qu'il dit).
expect_line() {
  wanted "$1" || return 0
  if ! grep -qF -- "$3" "$WORK/cases/$1/$2.err"; then
    echo "FAIL $1 : « $3 » absent de la sortie ($2)"; status=1
  fi
}

run_case local-tag local-tag froid
expect_line local-tag ref ": Cloning ${TAG_REF:0:10}"
run_case local-dev local-dev froid
run_case local-branch local-branch froid
expect_line local-branch ref ": Cloning master"
run_case cache-tag cache-tag froid
expect_line cache-tag ref "  - Syncing acme/lib (1.0.0) into cache"
expect_line cache-tag ref ": Cloning ${TAG_REF:0:10} from cache"
run_case cache-dev cache-dev froid
run_case cache-branch cache-branch froid
run_case cache-warm cache-tag chaud
if wanted cache-warm && [ ! -d "$WORK/cases/cache-warm/cache-viv/vcs" ]; then
  echo "FAIL cache-warm : aucun miroir dans le cache (cas mal construit)"; status=1
fi
run_case cache-stale cache-dev rassis
if wanted cache-stale && [ -z "$(cd "$WORK/cases/cache-stale/cache-viv/vcs/"* 2>/dev/null && find objects -path objects/pack -prune -o -path objects/info -prune -o -type f -print | head -1)" ]; then
  echo "FAIL cache-stale : la synchronisation n'a laissé aucun objet libre dans le miroir (cas mal construit)"; status=1
fi
run_case github github froid
if wanted github && ! grep -q 'pushurl = git@github.com:acme/lib.git' "$WORK/cases/github/ref/vendor/acme/lib/.git/config"; then
  echo "FAIL github : Composer n'a pas posé le pushurl GitHub (cas mal construit)"; status=1
fi
run_case verbose cache-dev froid -v
expect_line verbose ref ": Cloning $HEAD_REF from cache"
run_case quiet cache-dev froid -q
SECOND="install --no-scripts" run_case no-op cache-tag froid
expect_line no-op ref "Nothing to install, update or remove"
run_case mixed mixed froid
expect_line mixed ref "  - Syncing acme/lib (1.0.0) into cache"
run_case no-dev no-dev froid --no-dev
SECOND="dump-autoload -o" run_case dump cache-dev froid

# Repli : un paquet installé depuis la source que le lock retire. Le
# projet de départ est celui de Composer (des deux côtés) ; vivacity rend
# la main avant toute écriture.
if wanted remove; then
  c="$WORK/cases/remove"; mkdir -p "$c/ref" "$c/viv"
  for who in ref viv; do
    cp "$WORK/seeds/local-tag/composer.json" "$WORK/seeds/local-tag/composer.lock" "$c/$who/"
    (cd "$c/$who" && COMPOSER_CACHE_DIR="$CACHE" composer install --no-scripts --no-ansi >/dev/null 2>&1)
    cp "$WORK/seeds/empty/composer.json" "$WORK/seeds/empty/composer.lock" "$c/$who/"
  done
  ( cd "$c/viv" && COMPOSER_CACHE_DIR="$CACHE" "$VIVACITY" install --no-scripts --no-fallback >"$c/nofb.out" 2>"$c/nofb.err" ); code=$?
  if [ "$code" != 3 ] || ! grep -q "acme/lib is installed from source" "$c/nofb.err" || [ ! -d "$c/viv/vendor/acme/lib/.git" ]; then
    echo "FAIL remove : --no-fallback attendu en 3, motif nommé, clone intact (code $code)"; tail -4 "$c/nofb.err"; status=1
  else
    side ref "$c/ref" install --no-scripts
    side viv "$c/viv" install --no-scripts
    if [ "$(cat "$c/ref.code")" = "$(cat "$c/viv.code")" ] && compare_vendor "$c/ref" "$c/viv" "$c/project.diff" >/dev/null; then
      echo "OK   remove (repli nommé, projet de Composer)"
    else
      echo "FAIL remove : après repli"; head -8 "$c/project.diff"; status=1
    fi
  fi
fi

# Repli : une référence absente du dépôt. Rien n'est écrit dans le projet,
# Composer rejoue et échoue de lui-même.
if wanted gone; then
  c="$WORK/cases/gone"; mkdir -p "$c/ref" "$c/viv"
  for who in ref viv; do cp "$WORK/seeds/gone/composer.json" "$WORK/seeds/gone/composer.lock" "$c/$who/"; done
  rm -rf "$CACHE"
  ( cd "$c/viv" && COMPOSER_CACHE_DIR="$CACHE" "$VIVACITY" install --no-scripts --no-fallback >"$c/nofb.out" 2>"$c/nofb.err" ); code=$?
  leftovers="$(cd "$c/viv" && find vendor -mindepth 1 2>/dev/null | head -3)"
  if [ "$code" != 3 ] || ! grep -q "acme/lib: git source not installed natively" "$c/nofb.err" || [ -n "$leftovers" ]; then
    echo "FAIL gone : --no-fallback attendu en 3, motif nommé, vendor vide (code $code ; reste : $leftovers)"; tail -4 "$c/nofb.err"; status=1
  else
    rm -rf "$CACHE"; side ref "$c/ref" install --no-scripts
    rm -rf "$CACHE"; side viv "$c/viv" install --no-scripts
    if [ "$(cat "$c/ref.code")" = "$(cat "$c/viv.code")" ] && [ "$(cat "$c/ref.code")" != 0 ]; then
      echo "OK   gone (repli nommé ; Composer échoue en $(cat "$c/ref.code") des deux côtés)"
    else
      echo "FAIL gone : codes $(cat "$c/ref.code") / $(cat "$c/viv.code")"; tail -3 "$c/viv.err"; status=1
    fi
  fi
fi

if [ -z "${GIT_SOURCES_OFFLINE:-}" ]; then
  run_case real real froid
  expect_line real ref "  - Syncing shaarli/netscape-bookmark-parser (dev-master 26d391c) into cache"
fi

exit $status
