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
  sed -n '/^In [A-Za-z]*\.php line [0-9]*:$/,/^$/p' "$1" \
    | sed 's/^ *//; s/ *$//' | grep -v '^$' | grep -vE '^In [A-Za-z]*\.php line [0-9]*:$'
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
  (cd "$d-ref" && COLUMNS=400 COMPOSER_NO_INTERACTION=1 composer "$@" --no-ansi >/dev/null 2>"$WORK/$name.composer.err") || rc=$?
  (cd "$d-viv" && "$VIVACITY" "$@" >/dev/null 2>"$WORK/$name.vivacity.err") || vc=$?
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
  local written
  for side in ref viv; do
    written=$(cd "$d-$side" && find . -newer "$WORK/$name.marker" -type f | head -5)
    if [ -n "$written" ]; then
      echo "FAIL $name : $side a écrit malgré le refus : $(echo "$written" | tr '\n' ' ')"; status=1; return
    fi
  done
  echo "OK   $name : code $rc, message identique, rien d'écrit des deux côtés"
}

for cmd_label in "dump-autoload:dump-autoload" "install:install" "update:update --no-install"; do
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
