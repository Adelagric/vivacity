#!/usr/bin/env bash
# Les refus de `RootPackageLoader::load`, que Composer lève pour TOUTE commande
# parce que `Factory::createComposer` charge le paquet racine : nom racine
# invalide, paquet qui s'exige lui-même, nom de lien invalide.
#
# Ce banc vérifie les trois choses que `steps.sh` ne peut pas vérifier :
#   1. le TEXTE du message. Composer répond par un encadré de Symfony Console,
#      qu'aucune ancre de steps.sh ne décrit ; ici on extrait le contenu de
#      l'encadré (COLUMNS large, pour qu'il ne soit pas replié) et on le compare
#      aux lignes que vivacity imprime — sans encadré ni synopsis, déviation
#      déjà assumée pour les dépôts `path`.
#   2. qu'aucun fichier n'est écrit. `compare_vendor` ne compare pas les mtimes :
#      `dump-autoload` laissait l'empreinte de l'arbre inchangée tout en
#      écrivant l'autoloader. Ici on cherche tout fichier plus récent qu'un
#      témoin posé juste avant la commande.
#   3. `dump-autoload`, que steps.sh ne couvre pas.
#
# Usage : harness/root-manifest.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/root-manifest"
FX="${FX:-laravel}"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v jq >/dev/null || { echo "jq requis"; exit 1; }
composer --version --no-ansi | tail -1
rm -rf "$WORK"; mkdir -p "$WORK/base"
cp "$ROOT/fixtures/projects/$FX/composer.json" "$ROOT/fixtures/projects/$FX/composer.lock" "$WORK/base/"
status=0

# Le contenu de l'encadré d'erreur de Symfony Console, sans son en-tête
# (`In X.php line N:`) ni le synopsis, marges rognées.
box_text() {
  sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/,/^$/p' "$1" \
    | sed 's/^ *//; s/ *$//' | grep -v '^$' | grep -vE '^In [A-Za-z0-9_]*\.php line [0-9]*:$'
}

run() { # nom, expression jq, commande...
  local name="$1" expr="$2"; shift 2
  local d="$WORK/$name"
  for side in ref viv; do
    rm -rf "$d-$side"; cp -R "$WORK/base" "$d-$side"
    jq --indent 4 "$expr" "$d-$side/composer.json" > "$d-$side/c.tmp" && mv "$d-$side/c.tmp" "$d-$side/composer.json"
  done
  local rc=0 vc=0
  touch "$WORK/$name.marker"
  (cd "$d-ref" && COLUMNS=400 COMPOSER_NO_INTERACTION=1 composer "$@" --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || rc=$?
  (cd "$d-viv" && "$VIVACITY" "$@" >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || vc=$?
  if [ "$rc" != "$vc" ]; then
    echo "FAIL $name : codes $rc vs $vc"; head -4 "$WORK/$name.vivacity.err"; status=1; return
  fi
  if [ "$rc" = 0 ]; then
    echo "FAIL $name : Composer n'a rien refusé (cas mal choisi)"; status=1; return
  fi
  box_text "$WORK/$name.composer.err" > "$WORK/$name.composer.msg"
  grep -v '^$' "$WORK/$name.vivacity.err" > "$WORK/$name.vivacity.msg"
  if ! [ -s "$WORK/$name.composer.msg" ]; then
    echo "FAIL $name : pas d'encadré dans la sortie de Composer (cas mal choisi)"; status=1; return
  fi
  if ! diff -q "$WORK/$name.composer.msg" "$WORK/$name.vivacity.msg" >/dev/null; then
    echo "FAIL $name : message différent"; diff "$WORK/$name.composer.msg" "$WORK/$name.vivacity.msg" | head -6; status=1; return
  fi
  # stdout : rien, ou l'annotation de GitHub Actions (`Application::doRun`)
  # quand GITHUB_ACTIONS est posé sans COMPOSER_TESTS_ARE_RUNNING.
  if ! diff -q "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >/dev/null; then
    echo "FAIL $name : stdout différente"; diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" | head -4; status=1; return
  fi
  local written
  for side in ref viv; do
    written=$(cd "$d-$side" && find . -newer "$WORK/$name.marker" ! -name . | head -5)
    if [ -n "$written" ]; then
      echo "FAIL $name : $side a écrit malgré le refus : $(echo "$written" | tr '\n' ' ')"; status=1; return
    fi
  done
  echo "OK   $name : code $rc, message identique, rien d'écrit des deux côtés"
}

# Les lignes que Composer écrit avant l'encadré, puis l'encadré : ce que
# vivacity doit écrire, dans cet ordre.
lines_then_box() {
  sed -n '/^In [A-Za-z0-9_]*\.php line [0-9]*:$/q;p' "$1" | grep -v '^$'
  box_text "$1"
}

# run_lock <nom> <expression jq sur composer.lock> <commande...> : une
# exception levée après des lignes ordinaires (`Locker::getLockedRepository`
# après l'en-tête d'`install`), même code, mêmes lignes, même message.
run_lock() {
  local name="$1" expr="$2"; shift 2
  local d="$WORK/$name"
  for side in ref viv; do
    rm -rf "$d-$side"; cp -R "$WORK/base" "$d-$side"
    jq --indent 4 "$expr" "$d-$side/composer.lock" > "$d-$side/l.tmp" && mv "$d-$side/l.tmp" "$d-$side/composer.lock"
  done
  local rc=0 vc=0
  touch "$WORK/$name.marker"
  (cd "$d-ref" && COLUMNS=400 COMPOSER_NO_INTERACTION=1 composer "$@" --no-ansi >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || rc=$?
  (cd "$d-viv" && "$VIVACITY" "$@" >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || vc=$?
  if [ "$rc" != "$vc" ]; then
    echo "FAIL $name : codes $rc vs $vc"; head -4 "$WORK/$name.vivacity.err"; status=1; return
  fi
  if ! grep -q '^In [A-Za-z0-9_]*\.php line [0-9]*:$' "$WORK/$name.composer.err"; then
    echo "FAIL $name : pas d'encadré dans la sortie de Composer (cas mal choisi)"; status=1; return
  fi
  lines_then_box "$WORK/$name.composer.err" > "$WORK/$name.composer.msg"
  grep -v '^$' "$WORK/$name.vivacity.err" > "$WORK/$name.vivacity.msg"
  if ! diff -q "$WORK/$name.composer.msg" "$WORK/$name.vivacity.msg" >/dev/null; then
    echo "FAIL $name : sortie différente"; diff "$WORK/$name.composer.msg" "$WORK/$name.vivacity.msg" | head -6; status=1; return
  fi
  # stdout : rien, ou l'annotation de GitHub Actions (`Application::doRun`)
  # quand GITHUB_ACTIONS est posé sans COMPOSER_TESTS_ARE_RUNNING.
  if ! diff -q "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >/dev/null; then
    echo "FAIL $name : stdout différente"; diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" | head -4; status=1; return
  fi
  local written
  if [ "${LOCK_WRITES:-}" = 1 ]; then
    # La commande édite composer.json avant l'exception (`remove`), ou le
    # rétablit (`require`) : les deux côtés écrivent, les fichiers doivent
    # finir identiques.
    for f in composer.json composer.lock; do
      if ! cmp -s "$d-ref/$f" "$d-viv/$f"; then
        echo "FAIL $name : $f diffère"; status=1; return
      fi
    done
  else
  for side in ref viv; do
    written=$(cd "$d-$side" && find . -newer "$WORK/$name.marker" ! -name . | head -5)
    if [ -n "$written" ]; then
      echo "FAIL $name : $side a écrit malgré le refus : $(echo "$written" | tr '\n' ' ')"; status=1; return
    fi
  done
  fi
  echo "OK   $name : code $rc, lignes et message identiques, rien d'écrit des deux côtés"
}

# Un lock sans `packages-dev` en mode dev : `Installer::doInstall` écrit son
# en-tête, puis `Locker::getLockedRepository(true)` lève (avant « Verifying… »).
run_lock lock-no-packages-dev 'del(.["packages-dev"])' install --dry-run --no-plugins --no-scripts
run_lock lock-null-packages-dev '.["packages-dev"]=null' install --dry-run --no-plugins --no-scripts
# Sans dev, l'install passe : c'est la liste des abandonnés
# (`Installer::run`, l. 356, `getLockedRepository(true)`) qui lève, après les
# opérations. Les commandes de résolution lèvent dans `doUpdate` (l. 496),
# avant « Loading composer repositories » ; `require` rétablit ses fichiers
# d'abord (« Installation failed, reverting… »), `remove` garde son édition.
run_lock lock-no-dev-no-packages-dev 'del(.["packages-dev"])' install --no-dev --dry-run --no-plugins --no-scripts
run_lock update-no-packages-dev 'del(.["packages-dev"])' update --no-install --no-plugins --no-scripts
LOCK_WRITES=1 run_lock require-no-packages-dev 'del(.["packages-dev"])' require psr/log --no-install --no-plugins --no-scripts
LOCK_WRITES=1 run_lock remove-no-packages-dev 'del(.["packages-dev"])' remove laravel/tinker --no-install --no-plugins --no-scripts
# Sous GitHub Actions, le message de l'exception annoté sur stdout.
(unset COMPOSER_TESTS_ARE_RUNNING; export GITHUB_ACTIONS=true
 run_lock gha-lock-no-packages-dev 'del(.["packages-dev"])' install --dry-run --no-plugins --no-scripts
 run gha-self-require '.name="laravel/framework"' install
 exit $status) || status=1
[ -s "$WORK/gha-self-require.composer.out" ] || { echo "FAIL gha-self-require : pas d'annotation chez Composer (cas mal choisi)"; status=1; }

for cmd_label in "dump-autoload:dump-autoload" "install:install" "update:update --no-install" \
                 "remove:remove psr/log" "require:require psr/log"; do
  label="${cmd_label%%:*}"; cmdline="${cmd_label#*:}"
  # shellcheck disable=SC2086
  # Un nom racine en majuscules ou de forme invalide est refusé par le SCHÉMA,
  # dont le motif `name` est sensible à la casse — message que vivacity ne
  # reproduit pas (famille reportée). Seuls le suffixe `.json` et les noms
  # réservés franchissent le schéma et atteignent le chargeur.
  run "name-reserved-$label" '.name="con/thing"'             $cmdline
  run "name-json-$label"    '.name="acme/thing.json"'        $cmdline
  run "self-require-$label" '.name="laravel/framework"'      $cmdline
  run "link-upper-$label"   '.require["Acme/Thing"]="^1"'    $cmdline
  run "link-reserved-$label" '.require["con/x"]="*"'         $cmdline
  run "link-json-$label"    '.require["acme/x.json"]="*"'    $cmdline
  run "provide-upper-$label" '.provide["BAR/Foo"]="1.0"'     $cmdline
done
exit $status
