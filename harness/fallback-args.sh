#!/usr/bin/env bash
# Ce que le fallback transmet à Composer : argv, répertoire, environnement.
#
# Un faux `composer` en tête du PATH enregistre chaque appel — son répertoire
# courant, `SHELL_VERBOSITY`, `COMPOSER_DISABLE_NETWORK`, puis ses arguments, un
# par ligne — et sort avec le code demandé. Chaque cas compare l'enregistrement
# entier à celui attendu : la ligne de commande d'origine moins les options
# propres à vivacity, `--no-scripts` sauf `--run-scripts`, le répertoire de
# vivacity (un `--working-dir` relatif garde son sens parce que Composer
# l'applique lui-même), le niveau de la run, `--offline` devenu
# `COMPOSER_DISABLE_NETWORK=1`, et un processus fils dont le code de sortie
# est rendu (jamais `exec`, qui remplacerait le processus d'un embarqueur).
# Plan : docs/plans/fallback-argv.md.
#
# Scénarios :
#   a. `install` hors portée (laravel, `preferred-install: source`) lancé du
#      répertoire parent avec `--working-dir` relatif, sous -q, -v,
#      `--prefer-source`, `--no-progress`, `--offline`, `--run-scripts` ; son
#      dry run reste natif ; le dry run hors portée du plugin de fusion
#      transmet `--dry-run` ;
#   b. `update`/`require`/`remove` rendus à Composer (un plugin installé et
#      autorisé hors des listes de vivacity) avec `--working-dir` relatif ;
#   c. la phase d'installation d'un `update`/`require` (`preferred-install: source`) :
#      lock écrit par vivacity, puis la ligne `composer install` construite ;
#   d. `--no-fallback` / `--check-scope` : sortie 3, Composer jamais lancé ;
#   e. `-vv` : la délégation de verbosité, même modèle ;
#   f. la commande entière rendue après la résolution (Flex verrouillé, pas
#      installé) : Composer repart du composer.json d'origine, vivacity
#      n'ajoute rien après lui ;
#   g. l'avertissement de version racine (racine nommée, pas de type
#      `project`, ni version ni dépôt git ni `COMPOSER_ROOT_VERSION`) : rendue
#      avant que vivacity écrive quoi que ce soit, la commande laisse la ligne
#      à Composer — aucune chez vivacity ; une délégation tardive l'a déjà
#      vue sortir.
#
# Usage : harness/fallback-args.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/fallback-args"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v jq >/dev/null || { echo "jq requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK/bin" "$WORK/home" "$WORK/cache"
WORK="$(cd "$WORK" && pwd -P)"
# Rien de l'environnement appelant ne doit colorer l'enregistrement.
unset SHELL_VERBOSITY COMPOSER_DISABLE_NETWORK COMPOSER COMPOSER_NO_DEV COMPOSER_VENDOR_DIR COMPOSER_BIN_DIR
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$WORK/cache" VIVACITY_CACHE_DIR="$WORK/store"
export COMPOSER_ROOT_VERSION=dev-main
REC="$WORK/record"

cat > "$WORK/bin/composer" <<'SH'
#!/usr/bin/env bash
{
  echo "--- call"
  echo "cwd=$(pwd -P)"
  echo "SHELL_VERBOSITY=${SHELL_VERBOSITY-<unset>}"
  echo "COMPOSER_DISABLE_NETWORK=${COMPOSER_DISABLE_NETWORK-<unset>}"
  # The manifest Composer starts from, when the case watches one.
  if [ -n "${FAKE_COMPOSER_MANIFEST:-}" ]; then echo "manifest=$(cksum < "$FAKE_COMPOSER_MANIFEST")"; fi
  for a in "$@"; do echo "arg=$a"; done
} >> "$FAKE_COMPOSER_RECORD"
# A marker on stderr, when the case watches where its output lands.
if [ -n "${FAKE_COMPOSER_STDERR:-}" ]; then echo "$FAKE_COMPOSER_STDERR" >&2; fi
exit "${FAKE_COMPOSER_EXIT:-0}"
SH
chmod +x "$WORK/bin/composer"
export PATH="$WORK/bin:$PATH" FAKE_COMPOSER_RECORD="$REC"

status=0
fail() { echo "FAIL $*"; status=1; }

# `expect_call <dir> <SHELL_VERBOSITY> <COMPOSER_DISABLE_NETWORK> args...` :
# ajoute un appel attendu à $WORK/expected.
expect_call() {
  local dir="$1" sv="$2" net="$3"; shift 3
  {
    echo "--- call"
    echo "cwd=$(cd "$dir" && pwd -P)"
    echo "SHELL_VERBOSITY=$sv"
    echo "COMPOSER_DISABLE_NETWORK=$net"
    if [ -n "${FAKE_COMPOSER_MANIFEST:-}" ]; then echo "manifest=$EXPECTED_MANIFEST"; fi
    for a in "$@"; do echo "arg=$a"; done
  } >> "$WORK/expected"
}

# `check <nom> <dir> <code attendu> <code du faux composer> -- vivacity args...` :
# lance vivacity depuis <dir>, compare son code de sortie et l'enregistrement à
# $WORK/expected (absent = Composer ne doit pas avoir été lancé), puis remet les
# deux à zéro. stderr est gardé dans $WORK/<nom>.err.
check() {
  local name="$1" dir="$2" want="$3" fake_exit="$4"; shift 5
  local code=0
  (cd "$dir" && FAKE_COMPOSER_EXIT="$fake_exit" "$VIVACITY" "$@" >"$WORK/$name.out" 2>"$WORK/$name.err") || code=$?
  if [ "$code" != "$want" ]; then
    fail "$name : sortie $code, attendu $want"; tail -5 "$WORK/$name.err"
  elif [ -f "$WORK/expected" ] && [ ! -f "$REC" ]; then
    fail "$name : Composer n'a pas été lancé"; tail -5 "$WORK/$name.err"
  elif [ ! -f "$WORK/expected" ] && [ -f "$REC" ]; then
    fail "$name : Composer a été lancé, il ne devait pas l'être"; cat "$REC"
  elif [ -f "$WORK/expected" ] && ! diff -u "$WORK/expected" "$REC" >"$WORK/$name.diff"; then
    fail "$name : appel différent de celui attendu"; cat "$WORK/$name.diff"
  else
    echo "OK   $name"
  fi
  rm -f "$WORK/expected" "$REC"
}

# --- a. install hors portée -------------------------------------------------
A="$WORK/a"; mkdir -p "$A/proj"
cp "$ROOT/fixtures/projects/laravel/composer.json" "$ROOT/fixtures/projects/laravel/composer.lock" "$A/proj/"
jq '.config["preferred-install"] = "source"' "$A/proj/composer.json" > "$A/x.json" && mv "$A/x.json" "$A/proj/composer.json"
# Le cas est bien hors portée pour une vraie install (ScopeIssue::Config).
code=0; (cd "$A" && "$VIVACITY" install --check-scope --working-dir proj --ignore-platform-reqs --no-plugins --offline >/dev/null 2>"$WORK/a-scope.err") || code=$?
if [ "$code" != 3 ] || ! grep -q 'preferred-install "source"' "$WORK/a-scope.err"; then
  fail "a : le lock devait être hors portée (preferred-install source), sortie $code"; cat "$WORK/a-scope.err"
fi

expect_call "$A" 0 1 install --working-dir proj --prefer-source --no-progress --ignore-platform-reqs --no-scripts
check a-install-normal "$A" 7 7 -- install --working-dir proj --prefer-source --no-progress --offline --ignore-platform-reqs
if ! grep -qF 'vivacity: delegating to `composer install --working-dir proj --prefer-source --no-progress --ignore-platform-reqs --no-scripts`' "$WORK/a-install-normal.err"; then
  fail "a-install-normal : la ligne « delegating to » ne montre pas la ligne transmise"; grep delegating "$WORK/a-install-normal.err" || true
fi

expect_call "$A" -1 1 install -q --working-dir proj --no-plugins --ignore-platform-reqs --no-scripts
check a-install-quiet "$A" 0 0 -- install -q --working-dir proj --no-plugins --offline --ignore-platform-reqs
if [ -s "$WORK/a-install-quiet.err" ] || [ -s "$WORK/a-install-quiet.out" ]; then
  fail "a-install-quiet : vivacity a écrit sous -q"; cat "$WORK/a-install-quiet.err"
fi

expect_call "$A" 1 1 install -v --no-plugins --working-dir=proj --ignore-platform-reqs --no-dev --no-scripts
check a-install-verbose "$A" 0 0 -- install -v --no-plugins --working-dir=proj --offline --ignore-platform-reqs --no-dev

# --run-scripts : Composer exécute les scripts lui-même (pas de --no-scripts).
expect_call "$A" 0 1 install --working-dir proj --no-plugins --ignore-platform-reqs --apcu-autoloader-prefix pfx --no-blocking
check a-install-run-scripts "$A" 0 0 -- install --run-scripts --working-dir proj --no-plugins --offline --ignore-platform-reqs --apcu-autoloader-prefix pfx --no-blocking

# Le dry run du même lock : natif, identique à Composer (décision 5 du plan).
check a-dry-run-native "$A" 0 0 -- install --dry-run --working-dir proj --no-plugins --offline --ignore-platform-reqs

# Un dry run hors portée avant le retour du dry run (plugin de fusion, fichier
# requis absent) : Composer reçoit --dry-run, jamais une installation réelle.
M="$WORK/merge"; mkdir -p "$M"
cp -R "$ROOT/fixtures/projects/merge-plugin" "$M/proj"
rm "$M/proj/ext/composer.ext.json"
expect_call "$M" 0 1 install --dry-run --working-dir proj --no-scripts
check a-merge-dry-run "$M" 0 0 -- install --dry-run --working-dir proj --offline

# --- b. update / require / remove rendus à Composer --------------------------
B="$WORK/b"; mkdir -p "$B/proj/vendor/composer"
cat > "$B/proj/composer.json" <<'JSON'
{
    "require": { "psr/log": "^3.0" },
    "config": { "allow-plugins": { "acme/probe-plugin": true } }
}
JSON
cat > "$B/proj/vendor/composer/installed.json" <<'JSON'
{ "packages": [ { "name": "acme/probe-plugin", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "composer-plugin" } ], "dev": true, "dev-package-names": [] }
JSON
b_before=$(cat "$B/proj/composer.json")

expect_call "$B" 0 '<unset>' update --working-dir proj -n --no-progress --no-scripts
check b-update "$B" 5 5 -- update --working-dir proj -n --no-progress

expect_call "$B" 0 '<unset>' r psr/log:^3.0 --working-dir proj --no-scripts
check b-require-alias "$B" 0 0 -- r psr/log:^3.0 --no-autoloader --working-dir proj

expect_call "$B" 0 '<unset>' require --working-dir proj --no-scripts -- psr/log:^3.0
check b-require-dashes "$B" 0 0 -- require --working-dir proj -- psr/log:^3.0

expect_call "$B" -1 1 remove psr/log -q --working-dir proj --no-scripts
check b-remove "$B" 0 0 -- remove psr/log --no-autoloader -q --offline --working-dir proj
[ "$(cat "$B/proj/composer.json")" = "$b_before" ] || fail "b : composer.json modifié avant la délégation"

# --- c. la phase d'installation d'un update / require ------------------------
C="$WORK/c"; mkdir -p "$C/proj/pkgs/local" "$C/proj/pkgs/other"
# Deux paquets `path` (résolus nativement) et `preferred-install: source`, que
# la résolution ignore et que l'install refuse (ScopeIssue::Config).
cat > "$C/proj/composer.json" <<'JSON'
{
    "repositories": [
        { "type": "path", "url": "pkgs/*" },
        { "packagist.org": false }
    ],
    "require": { "acme/local": "1.0.0" },
    "config": { "preferred-install": "source" }
}
JSON
for p in local other; do
  printf '{ "name": "acme/%s", "version": "1.0.0" }\n' "$p" > "$C/proj/pkgs/$p/composer.json"
done
expect_call "$C" 1 1 install --no-dev --no-progress --optimize-autoloader --classmap-authoritative \
  --no-plugins --no-blocking --no-interaction --no-ansi --apcu-autoloader-prefix=pfx \
  --ignore-platform-req=ext-foo --working-dir=proj --no-scripts
check c-update "$C" 0 0 -- update --working-dir proj --no-dev -o -a --apcu-autoloader-prefix=pfx --no-progress \
  --ignore-platform-req=ext-foo --no-plugins -n --no-ansi --no-blocking --offline -v
[ -f "$C/proj/composer.lock" ] || fail "c-update : vivacity devait écrire composer.lock avant de rendre l'installation"
grep -q '"name": "acme/local"' "$C/proj/composer.lock" 2>/dev/null || fail "c-update : le lock écrit ne contient pas acme/local"

# `require` : -n et --update-no-dev arrivent à l'install ; un échec de Composer
# restaure composer.json (revertComposerFile), un succès le garde.
c_before=$(cat "$C/proj/composer.json")
expect_call "$C" 0 1 install --no-dev --no-interaction --working-dir=proj --no-scripts
c_lock_before=$(cat "$C/proj/composer.lock")
check c-require-fails "$C" 6 6 -- require acme/other:1.0.0 --working-dir proj -n --update-no-dev --offline
[ "$(cat "$C/proj/composer.json")" = "$c_before" ] || fail "c-require-fails : composer.json non restauré après l'échec de Composer"
[ "$(cat "$C/proj/composer.lock")" = "$c_lock_before" ] || fail "c-require-fails : composer.lock non restauré après l'échec de Composer"
expect_call "$C" 0 1 install --no-interaction --working-dir=proj --no-scripts
check c-require "$C" 0 0 -- require acme/other:1.0.0 --working-dir proj -n --offline
jq -e '.require["acme/other"] == "1.0.0"' "$C/proj/composer.json" >/dev/null || fail "c-require : composer.json ne garde pas acme/other"

# `require --prefer-source` : l'install qui suit le lit (RequireCommand), et
# le rend à Composer avec l'option.
C2="$WORK/c2"; mkdir -p "$C2"; cp -R "$C/proj" "$C2/proj"
jq 'del(.config) | del(.require["acme/other"])' "$C/proj/composer.json" > "$C2/proj/composer.json"
rm -f "$C2/proj/composer.lock"
expect_call "$C2" 0 1 install --prefer-source --working-dir=proj --no-scripts
check c-require-prefer-source "$C2" 0 0 -- require acme/other:1.0.0 --prefer-source --working-dir proj --offline

# Une préférence impossible : refusée avant la résolution (`doUpdate`,
# `getPreferredInstallOptions`), les fichiers restaurés, Composer jamais lancé.
c2_json=$(cat "$C2/proj/composer.json"); c2_lock=$(cat "$C2/proj/composer.lock")
check c-require-bad-prefer "$C2" 1 0 -- require acme/local:1.0.0 --prefer-source --prefer-install dist --working-dir proj --offline
grep -q -- '--prefer-source can not be used together with --prefer-install' "$WORK/c-require-bad-prefer.err" || fail "c-require-bad-prefer : message de Composer absent"
[ "$(cat "$C2/proj/composer.json")" = "$c2_json" ] && [ "$(cat "$C2/proj/composer.lock")" = "$c2_lock" ] || fail "c-require-bad-prefer : fichiers modifiés"

# --- f. la commande entière rendue à Composer après la résolution -------------
# symfony/flex verrouillé mais pas encore installé (`flex_reinstall_reason`) :
# décidé sur le lock résolu, après l'édition de composer.json par
# `require`/`remove`. Composer doit repartir du manifeste d'origine, et
# vivacity ne rien faire après lui (ni install, ni lock, ni restauration).
F="$WORK/f"; mkdir -p "$F/proj/pkgs/flex" "$F/proj/pkgs/other" "$F/proj/pkgs/third"
cat > "$F/proj/composer.json" <<'JSON'
{
    "repositories": [
        { "type": "path", "url": "pkgs/*" },
        { "packagist.org": false }
    ],
    "require": { "symfony/flex": "2.0.0", "acme/other": "1.0.0" },
    "config": { "allow-plugins": { "symfony/flex": true } }
}
JSON
printf '{ "name": "symfony/flex", "version": "2.0.0", "type": "composer-plugin" }\n' > "$F/proj/pkgs/flex/composer.json"
for p in other third; do
  printf '{ "name": "acme/%s", "version": "1.0.0" }\n' "$p" > "$F/proj/pkgs/$p/composer.json"
done
f_before=$(cat "$F/proj/composer.json")
export FAKE_COMPOSER_MANIFEST="$F/proj/composer.json"
EXPECTED_MANIFEST=$(cksum < "$F/proj/composer.json")
# Composer qui réussit (0) et qui échoue (4) : dans les deux cas son code, et
# rien de vivacity après lui.
for cmd in "update" "remove acme/other" "require acme/third:1.0.0"; do
  for fake_exit in 0 4; do
    name="f-${cmd%% *}-$fake_exit"
    # shellcheck disable=SC2086
    expect_call "$F" 0 1 $cmd --working-dir proj --no-scripts
    # shellcheck disable=SC2086
    check "$name" "$F" "$fake_exit" "$fake_exit" -- $cmd --working-dir proj --offline
    [ "$(cat "$F/proj/composer.json")" = "$f_before" ] || fail "$name : composer.json n'est pas celui d'origine"
    [ ! -e "$F/proj/composer.lock" ] || fail "$name : vivacity a écrit composer.lock avant de rendre la commande"
    # La raison vient du lock résolu (`flex_reinstall_reason`), pas du
    # contrôle d'avant l'édition : c'est bien ce chemin qui est exercé.
    grep -q 'would install itself' "$WORK/$name.err" || fail "$name : pas la raison d'après la résolution"
    if grep -q 'Installation failed\|Removal failed\|Installing dependencies\|Nothing to install' "$WORK/$name.err"; then
      fail "$name : vivacity a continué après Composer"; cat "$WORK/$name.err"
    fi
  done
done
# --no-fallback : rien de lancé, l'édition défaite, sortie 3.
check f-remove-no-fallback "$F" 3 0 -- remove acme/other --no-fallback --working-dir proj --offline
[ "$(cat "$F/proj/composer.json")" = "$f_before" ] || fail "f-remove-no-fallback : composer.json n'est pas celui d'origine"
unset FAKE_COMPOSER_MANIFEST

# --- d. --no-fallback / --check-scope : sortie 3, Composer jamais lancé ------
check d-install-no-fallback "$A" 3 0 -- install --no-fallback --working-dir proj --no-plugins --offline --ignore-platform-reqs
check d-install-check-scope "$A" 3 0 -- install --check-scope --working-dir proj --no-plugins --offline --ignore-platform-reqs
check d-update-no-fallback "$B" 3 0 -- update --no-fallback --working-dir proj
check d-after-update-no-fallback "$C" 3 0 -- update --no-fallback --working-dir proj --offline
check d-verbosity-no-fallback "$A" 3 0 -- install -vv --no-fallback --working-dir proj

# --- e. -vv : délégation de verbosité, même modèle ---------------------------
expect_call "$A" 1 1 install -vv --working-dir proj --no-scripts
check e-install-vv "$A" 0 0 -- install -vv --working-dir proj --offline
expect_call "$B" 1 '<unset>' rm psr/log -vv --working-dir proj --no-scripts
check e-remove-alias-vv "$B" 0 0 -- rm psr/log --no-autoloader -vv --working-dir proj

# --- g. l'avertissement de version racine et les délégations ----------------
# `RootPackageLoader::load` sans version devinable écrit « Composer could not
# detect the root package (acme/lib) version… » en tête de la commande. Le
# faux composer n'écrit rien : on compte les lignes de vivacity seule. 0 avant
# une délégation précoce (Composer écrit la sienne) ; 1 avant une délégation
# tardive (écart connu : Composer en écrit une seconde), sous --no-fallback
# (Composer jamais lancé) et en natif.
unset COMPOSER_ROOT_VERSION
export GIT_CEILING_DIRECTORIES="$WORK"
expect_rootwarn() { # <nom du cas> <nombre attendu>
  local got
  got=$(grep -c "^Composer could not detect the root package (acme/lib) version, defaulting to '1.0.0'. See https://getcomposer.org/root-version$" "$WORK/$1.err" || true)
  [ "$got" = "$2" ] || { fail "$1 : vivacity écrit $got fois l'avertissement de version racine, attendu $2"; cat "$WORK/$1.err"; }
}
# Précoces : un plugin inconnu installé (`resolution_fallback`), avant toute
# ligne et avant l'édition de composer.json.
GB="$WORK/gb"; mkdir -p "$GB"; cp -R "$B/proj" "$GB/proj"
jq '.name = "acme/lib"' "$B/proj/composer.json" > "$GB/proj/composer.json"
expect_call "$GB" 0 '<unset>' update --working-dir proj --no-scripts
check g-update "$GB" 0 0 -- update --working-dir proj
expect_rootwarn g-update 0
expect_call "$GB" 0 '<unset>' require psr/log:^3.0 --working-dir proj --no-scripts
check g-require "$GB" 0 0 -- require psr/log:^3.0 --working-dir proj
expect_rootwarn g-require 0
expect_call "$GB" 0 '<unset>' remove psr/log --working-dir proj --no-scripts
check g-remove "$GB" 0 0 -- remove psr/log --working-dir proj
expect_rootwarn g-remove 0
check g-update-no-fallback "$GB" 3 0 -- update --no-fallback --working-dir proj
expect_rootwarn g-update-no-fallback 1
# Précoce : le plugin de fusion (fichier inclus absent), avant l'en-tête.
GM="$WORK/gm"; mkdir -p "$GM"; cp -R "$M/proj" "$GM/proj"
jq '.name = "acme/lib" | .type = "library" | del(.version)' "$M/proj/composer.json" > "$GM/proj/composer.json"
expect_call "$GM" 0 1 install --dry-run --working-dir proj --no-scripts
check g-merge-dry-run "$GM" 0 0 -- install --dry-run --working-dir proj --offline
expect_rootwarn g-merge-dry-run 0
# Précoce : -vv, rendu avant la lecture du projet.
GA="$WORK/ga"; mkdir -p "$GA"; cp -R "$A/proj" "$GA/proj"
jq '.name = "acme/lib" | .type = "library"' "$A/proj/composer.json" > "$GA/proj/composer.json"
expect_call "$GA" 1 1 install -vv --working-dir proj --no-scripts
check g-install-vv "$GA" 0 0 -- install -vv --working-dir proj --offline
expect_rootwarn g-install-vv 0
# Tardive : la portée d'`install` (`preferred-install: source`), décidée après
# l'en-tête — la ligne de vivacity est déjà sortie.
expect_call "$GA" 0 1 install --working-dir proj --no-plugins --ignore-platform-reqs --no-scripts
check g-install-late "$GA" 0 0 -- install --working-dir proj --no-plugins --offline --ignore-platform-reqs
expect_rootwarn g-install-late 1
check g-install-no-fallback "$GA" 3 0 -- install --no-fallback --working-dir proj --no-plugins --offline --ignore-platform-reqs
expect_rootwarn g-install-no-fallback 1
# Natif : le dry run du même lock.
check g-install-native "$GA" 0 0 -- install --dry-run --working-dir proj --no-plugins --offline --ignore-platform-reqs
expect_rootwarn g-install-native 1
# Ce que Composer écrit lui aussi avant la commande — l'avertissement d'une
# variable d'environnement (`BaseCommand::initialize`), `--no-suggest`
# (`RequireCommand::execute`) — est retenu avec la ligne de version : une
# délégation précoce les laisse tous au Composer lancé (le plugin de fusion,
# le plugin inconnu).
expect_line() { # <nom du cas> <motif> <nombre attendu>
  local got
  got=$(grep -c -- "$2" "$WORK/$1.err" || true)
  [ "$got" = "$3" ] || { fail "$1 : vivacity écrit $got fois « $2 », attendu $3"; cat "$WORK/$1.err"; }
}
expect_call "$GM" 0 1 install --dry-run --working-dir proj --no-scripts
COMPOSER_IGNORE_PLATFORM_REQS=1 check g-merge-env "$GM" 0 0 -- install --dry-run --working-dir proj --offline
expect_rootwarn g-merge-env 0
expect_line g-merge-env '^COMPOSER_IGNORE_PLATFORM_REQS is set' 0
expect_call "$GB" 0 '<unset>' require psr/log:^3.0 --no-suggest --working-dir proj --no-scripts
check g-require-no-suggest "$GB" 0 0 -- require psr/log:^3.0 --no-suggest --working-dir proj
expect_rootwarn g-require-no-suggest 0
expect_line g-require-no-suggest 'deprecated option "--no-suggest"' 0
# Sous --no-fallback (Composer jamais lancé), c'est vivacity qui parle : les
# lignes retenues sortent, dans l'ordre de Composer, avant ses raisons.
COMPOSER_IGNORE_PLATFORM_REQS=1 check g-update-env-no-fallback "$GB" 3 0 -- update --no-fallback --working-dir proj
expect_rootwarn g-update-env-no-fallback 1
expect_line g-update-env-no-fallback '^COMPOSER_IGNORE_PLATFORM_REQS is set' 1
if [ "$(grep -e '^Composer could not detect' -e '^COMPOSER_IGNORE_PLATFORM_REQS is set' -e '^vivacity: ' "$WORK/g-update-env-no-fallback.err" | head -3 | awk '{printf "%s ", $1}')" != "Composer COMPOSER_IGNORE_PLATFORM_REQS vivacity: " ]; then
  fail "g-update-env-no-fallback : ordre version racine → variable → raisons non tenu"; cat "$WORK/g-update-env-no-fallback.err"
fi
# `--run-scripts` : la ligne avant ce que `composer run-script` écrit
# (`pre-install-cmd`, qui échoue ici : la commande s'arrête sur son code).
GS="$WORK/gs"; mkdir -p "$GS"; cp -R "$A/proj" "$GS/proj"
jq '.name = "acme/lib" | .type = "library" | del(.config["preferred-install"]) | .scripts = {"pre-install-cmd": "echo pre"}' "$A/proj/composer.json" > "$GS/proj/composer.json"
expect_call "$GS" '<unset>' '<unset>' run-script --no-interaction -d "$GS/proj" pre-install-cmd
FAKE_COMPOSER_STDERR="marker: run-script" check g-run-scripts "$GS" 9 9 -- install --run-scripts --working-dir proj --no-plugins --offline --ignore-platform-reqs
expect_rootwarn g-run-scripts 1
if [ "$(grep -e '^Composer could not detect' -e '^marker: run-script$' "$WORK/g-run-scripts.err" | awk '{printf "%s ", $1}')" != "Composer marker: " ]; then
  fail "g-run-scripts : la ligne de version ne précède pas la sortie du script"; cat "$WORK/g-run-scripts.err"
fi
export COMPOSER_ROOT_VERSION=dev-main
unset GIT_CEILING_DIRECTORIES

exit $status
