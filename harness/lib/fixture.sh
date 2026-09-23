# Mise en place d'une copie de fixture pour les harnais : `composer.json` et
# `composer.lock` pour les projets ordinaires ; pour `path-repos`, l'arbre
# entier matérialisé par fixtures/path-repos.sh (dépôts git imbriqués, liens,
# modes, dépôt du projet sur `develop`).
#
# Environnement git des deux côtés : le plafond de recherche est le PARENT du
# répertoire du projet (git n'examine jamais le plafond lui-même : le dépôt
# du projet doit rester visible depuis packages/*, celui du checkout de
# vivacity ne doit jamais l'être), configuration globale et système ignorées
# (signature, tri des branches…), locale C pour l'ordre d'un glob.

# stage_project <fixture> <destination>
stage_project() {
  local fx="$1" dest="$2"
  rm -rf "$dest"; mkdir -p "$dest"
  if [ "$fx" = "path-repos" ]; then
    "$ROOT/fixtures/path-repos.sh" "$dest"
  else
    cp "$ROOT/fixtures/projects/$fx/composer.json" "$dest/"
    [ -f "$ROOT/fixtures/projects/$fx/composer.lock" ] && cp "$ROOT/fixtures/projects/$fx/composer.lock" "$dest/"
  fi
  return 0
}

# flex_index_serve: the captured Flex index (fixtures/flex/index.json)
# served over localhost by `php -S` for the run — Flex ignores a `file://`
# endpoint (no HTTP status line, so no 200), and Composer refuses plain
# http except with `secure-http: false`, which seed_flex sets on the
# staged copies. Sets FLEX_INDEX_URL and FLEX_INDEX_PID (no command
# substitution: the pid must survive); `flex_index_stop` ends it.
# `flex_index_serve [nom]` : `index.json` par défaut, `index-recipes.json`
# (les mêmes `versions` plus de vraies entrées `recipes`) sur demande.
flex_index_serve() {
  local file="${1:-index.json}" port
  port=$(php -r '$s = stream_socket_server("tcp://127.0.0.1:0", $e, $m); $n = stream_socket_get_name($s, false); fclose($s); echo substr($n, strrpos($n, ":") + 1);')
  php -S "127.0.0.1:$port" -t "$ROOT/fixtures/flex" >/dev/null 2>&1 &
  FLEX_INDEX_PID=$!
  FLEX_INDEX_URL="http://127.0.0.1:$port/$file"
  for _ in $(seq 1 50); do
    curl -fs -o /dev/null "$FLEX_INDEX_URL" 2>/dev/null && break
    sleep 0.1
  done
}
flex_index_stop() { [ -n "${FLEX_INDEX_PID:-}" ] && kill "$FLEX_INDEX_PID" 2>/dev/null; FLEX_INDEX_PID=""; FLEX_INDEX_URL=""; return 0; }

# seed_flex <project dir> <fixture> <index url>: symfony/flex installed for
# real in the staged project (its lock entry in installed.json, its files
# from the fixture's vendor/), and the served index as the only endpoint
# (`extra.symfony.endpoint` as a list: Flex appends no default to a list,
# so no network). Composer then runs with Flex active; vivacity emulates
# its pool filter (plan v0.16 B).
seed_flex() {
  local dest="$1" fx="$2" url="$3"
  mkdir -p "$dest/vendor/composer" "$dest/vendor/symfony/flex"
  # L'entrée est celle que Composer écrirait : `version_normalized` et
  # `installation-source` compris (sans eux, un paquet inchangé garde
  # l'entrée telle quelle chez Composer et la comparaison verrait un écart
  # qui vient de l'amorçage, pas du produit). `--indent 4` pour la même
  # raison : `JsonFile::write` reprend l'indentation du fichier existant,
  # et Composer écrit installed.json en 4 espaces.
  jq --indent 4 '{packages: [(.packages[] | select(.name == "symfony/flex") | . + {"version_normalized": (.version | ltrimstr("v") + ".0"), "installation-source": "dist", "install-path": "../symfony/flex"})], dev: true, "dev-package-names": []}' "$dest/composer.lock" > "$dest/vendor/composer/installed.json"
  [ "$(jq '.packages | length' "$dest/vendor/composer/installed.json")" = 1 ] || { echo "seed_flex: symfony/flex not in $fx's lock" >&2; return 1; }
  [ -d "$ROOT/fixtures/work/$fx/vendor/symfony/flex" ] || { echo "seed_flex: fixtures/work/$fx/vendor/symfony/flex missing (fixtures/make.sh)" >&2; return 1; }
  (cd "$ROOT/fixtures/work/$fx/vendor/symfony/flex" && tar -cf - .) | (cd "$dest/vendor/symfony/flex" && tar -xf -)
  jq --arg u "$url" '.extra.symfony.endpoint = [$u] | .config["secure-http"] = false' "$dest/composer.json" > "$dest/c.tmp" && mv "$dest/c.tmp" "$dest/composer.json"
}

# harness_git_env <parent of the project directories>
harness_git_env() {
  export GIT_CEILING_DIRECTORIES="$1" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 LC_ALL=C
}
