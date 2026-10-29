#!/usr/bin/env bash
# Harness : les recettes de symfony/flex APPLIQUÉES par un `update` avec
# install (docs/plans/flex-configurators.md). Flex enregistre une opération
# d'install par paquet du lock résolu absent de `symfony.lock`
# (`recordOperations`) et, à POST_UPDATE_CMD, applique la recette que
# l'index lui donne — ou la recette auto-générée d'un bundle.
#
# L'oracle : l'index réel de `symfony/recipes` et de `recipes-contrib`
# (réduit aux paquets des cas) et leurs vrais fichiers de recette, servis en
# local par `flex_index_serve` depuis fixtures/flex/configurators/
# (PROVENANCE). Composer + Flex 2.11.0 d'abord, vivacity ensuite
# (`--no-fallback`), sur deux copies du même état convergé. Comparés : le
# projet entier (contenu, modes, liens), composer.lock, symfony.lock, la
# stdout et la stderr — la session aléatoire de « Symfony operations: … (<32 hex>) »
# normalisée après avoir vérifié sa forme des deux côtés.
#
# Chaque cas affirme ce que l'ORACLE fait, pour qu'un cas ne passe jamais à
# vide : un cas à recettes exige « Symfony operations » dans la sortie de
# Composer et un fichier attendu créé ou modifié chez lui ; un cas sans
# recette exige l'absence de cette ligne. Un cas de repli exige le code 3 de
# vivacity avec la raison, AUCUN changement dans son projet (arbre entier
# hors vendor/), et que Composer, lui, applique bien des recettes.
#
# Usage : harness/flex-recipes.sh   (FLEX_RECIPES_ONLY="cas…" pour un sous-ensemble)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
# shellcheck source=lib/registry.sh
. "$ROOT/harness/lib/registry.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/flex-recipes"
FX=symfony
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
composer --version --no-ansi | head -1
command -v php >/dev/null || { echo "php requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
archive="$ROOT/fixtures/registry/$FX.tar.gz"
[ -f "$archive" ] || { echo "SKIP : pas d'instantané pour $FX"; exit 0; }
reg="$WORK/registry"; mkdir -p "$reg"; tar -C "$reg" -xzf "$archive"
write_snapshot_packages_json "$reg"
home="$WORK/home"; mkdir -p "$home"
# Le cache de fichiers habituel, partagé par les deux côtés (comme
# plugin-lines.sh) : les dists de la fixture y sont déjà (fixtures/make.sh,
# restauré par actions/cache en CI). Un cache vierge par run téléchargeait
# ~150 zipballs de api.github.com à chaque passage, assez pour qu'un runner
# macOS à IP partagée se fasse refuser par GitHub (2026-10-29). Les lignes
# `  - Downloading` restent hors comparaison.
CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
[ -n "$CACHE_DIR" ] || { echo "cache de fichiers de Composer introuvable"; exit 1; }
printf '{"repositories": %s}\n' "$(snapshot_repositories_json "$reg")" > "$home/config.json"
status=0
flex_index_serve configurators/recipes/index.json
trap 'flex_index_stop' EXIT
RECIPES_URL="$FLEX_INDEX_URL"
CONTRIB_URL="${FLEX_INDEX_URL%/recipes/index.json}/contrib/index.json"
curl -fs -o /dev/null "$CONTRIB_URL" || { echo "FAIL : l'index contrib n'est pas servi ($CONTRIB_URL)"; exit 1; }
root_version="${COMPOSER_ROOT_VERSION:-dev-main}"
SRC="$ROOT/fixtures/projects/$FX"

# Un `symfony.lock` sans recette couvrant tous les paquets sauf ceux passés
# en argument.
write_symfony_lock() { # projet, paquets à omettre...
  local d="$1"; shift
  php -r '
    $lock = json_decode(file_get_contents($argv[1] . "/composer.lock"), true);
    $omit = array_slice($argv, 2);
    $out = [];
    foreach (array_merge($lock["packages"], $lock["packages-dev"] ?? []) as $p) {
      if (in_array($p["name"], $omit, true)) { continue; }
      $out[$p["name"]] = ["version" => $p["version"]];
    }
    ksort($out);
    file_put_contents($argv[1] . "/symfony.lock", json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
  ' "$d" "$@"
}

jq_manifest() { # projet, filtre jq
  jq --indent 4 "$2" "$1/composer.json" > "$1/c.tmp" && mv "$1/c.tmp" "$1/composer.json"
}
# config/bundles.php de la fixture, moins les lignes d'un motif.
bundles_without() { # projet, motif grep -F
  mkdir -p "$1/config"; grep -vF "$2" "$SRC/config/bundles.php" > "$1/config/bundles.php"
}
# Les paquets `symfony-ux` retirés des exigences racine : sans eux
# `synchronizePackageJson` n'a rien à inscrire dans importmap.php.
drop_ux() { jq_manifest "$1" 'del(.require["symfony/stimulus-bundle"], .require["symfony/ux-icons"], .require["symfony/ux-live-component"])'; }

# Les formes : `pre` avant la convergence (composer.json, donc la
# résolution), `post` après (les fichiers que Flex lit). Les fichiers ne
# peuvent pas être posés avant : l'update de préparation les traiterait.
shape_bundles_twig()  { case "$1" in post) bundles_without "$2" 'TwigBundle\TwigBundle::class';; esac; }
shape_bundles_extra() { case "$1" in post) bundles_without "$2" 'TwigExtraBundle::class';; esac; }
shape_gitignore()     { case "$1" in post) printf '/var/\n/vendor/\n' > "$2/.gitignore";; esac; }
# .env et phpunit.dist.xml de la fixture, sans le bloc qu'y avait posé la
# recette du paquet (sinon `isFileMarked` les saute et rien n'est prouvé).
env_files_without() { # projet, paquet
  php -r '
    [$_, $d, $src, $p] = $argv;
    $q = preg_quote($p, "{");
    $env = preg_replace("{\n*###> $q ###.*###< $q ###\n}s", "\n", file_get_contents("$src/.env"));
    $xml = preg_replace("{\n*\s*<!-- ###\+ $q ### -->.*<!-- ###- $q ### -->\n}s", "\n", file_get_contents("$src/phpunit.dist.xml"));
    file_put_contents("$d/.env", $env);
    file_put_contents("$d/phpunit.dist.xml", $xml);
  ' "$1" "$SRC" "$2"
  ! grep -qF "###> $2 ###" "$1/.env" && ! grep -qF "###+ $2 ###" "$1/phpunit.dist.xml"
}
shape_env_phpunit()   { case "$1" in post) env_files_without "$2" symfony/routing;; esac; }
shape_doctrine()      { case "$1" in
                          pre) jq_manifest "$2" '.extra.symfony.docker = false';;
                          post) env_files_without "$2" doctrine/doctrine-bundle
                                bundles_without "$2" 'DoctrineBundle\DoctrineBundle::class';;
                        esac; }
# framework-bundle : composer.json sans `auto-scripts` (composer-scripts
# l'écrit, `updateComposerLock` réécrit alors composer.lock), un
# services.yaml déjà là (pas écrasé), bundles.php sans FrameworkBundle
# (fusionné en tête), un .env (env), pas de .env.dev (dotenv : rien).
shape_framework()     { case "$1" in
                          pre) jq_manifest "$2" 'del(.scripts["auto-scripts"])';;
                          post) bundles_without "$2" 'FrameworkBundle\FrameworkBundle::class'
                                mkdir -p "$2/config"; cp "$SRC/config/services.yaml" "$2/config/services.yaml"
                                cp "$SRC/.env" "$2/.env";;
                        esac; }
shape_framework_secret() { case "$1" in
                          post) shape_framework post "$2"; printf 'APP_SECRET=\n' > "$2/.env.dev";;
                        esac; }
# phpunit.xml.dist présent : la cible `phpunit.dist.xml` de la recette
# devient `phpunit.xml.dist` (`expandTargetDir`), qui existe — non copiée,
# mais patchée par add-lines.
# (sans les deux lignes qu'add-lines y remettrait, sinon il n'a rien à faire)
shape_phpunit_swap()  { case "$1" in post) grep -vF 'Deprecations\Deprecation::' "$SRC/phpunit.dist.xml" > "$2/phpunit.xml.dist";; esac; }
shape_docker_off()    { case "$1" in pre) jq_manifest "$2" '.extra.symfony.docker = false';; esac; }
shape_no_contrib()    { case "$1" in pre) jq_manifest "$2" '.extra.symfony["allow-contrib"] = false';; esac; }
shape_no_sync()       { case "$1" in pre) jq_manifest "$2" '.extra["symfony/flex"].synchronize_package_json = false';; esac; }
shape_several()       { case "$1" in post) bundles_without "$2" 'TwigBundle\TwigBundle::class'; cp "$SRC/.env" "$2/.env";; esac; }
shape_asset_mapper()  { case "$1" in pre) drop_ux "$2";; esac; }
shape_bin_dir()       { case "$1" in pre) jq_manifest "$2" '.config["bin-dir"] = "bin"';; esac; }
# Le paquet est posé par CE run (retiré de vendor/ et de installed.json) et
# composer.lock est réécrit par l'update lui-même (content-hash faussé) :
# la recette s'applique à un paquet que l'install pose, et le plan vérifie
# le lock que l'update vient d'écrire. (Une exigence ajoutée à
# composer.json rend la main plus tôt : `unpack()` chercherait le paquet.)
forget_package() { # projet, paquet
  local f="$1/vendor/composer/installed.json"
  jq --indent 4 --arg n "$2" '.packages |= map(select(.name != $n)) | ."dev-package-names" |= map(select(. != $n))' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  rm -rf "$1/vendor/$2"
}
shape_install_recipe() { case "$1" in
                          post) forget_package "$2" friendsofphp/php-cs-fixer
                                jq --indent 4 '."content-hash" = "00000000000000000000000000000000"' "$2/composer.lock" > "$2/l.tmp" && mv "$2/l.tmp" "$2/composer.lock";;
                        esac; }

# `FLEX_RECIPES_ONLY="cas1 cas2"` : ne prépare et ne lance que ces cas.
ONLY="${FLEX_RECIPES_ONLY:-}"
prepare() { # nom, forme (`-` : aucune), paquets à omettre de symfony.lock
  local name="$1" shape="$2"; shift 2
  if [ -n "$ONLY" ] && ! printf ' %s ' "$ONLY" | grep -qF " $name "; then return 1; fi
  for side in ref viv; do
    local d="$WORK/$side-$name"
    stage_project "$FX" "$d"
    [ "$shape" = - ] || "$shape" pre "$d"
    seed_flex "$d" "$FX" "$RECIPES_URL" || return 1
    jq_manifest "$d" ".extra.symfony.endpoint = [\"$RECIPES_URL\", \"$CONTRIB_URL\"]"
    cp "$ROOT/fixtures/projects/$FX/composer.lock" "$d/composer.lock"
    # Un symfony.lock complet AVANT la convergence : rien n'est enregistré,
    # donc aucune recette ne s'applique pendant la préparation.
    write_symfony_lock "$d"
    (cd "$d" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION="$root_version" \
      composer update --no-scripts --no-interaction --no-audit --no-ansi --no-blocking --quiet >"$WORK/$name.$side.prepare.log" 2>&1) || {
        echo "FAIL $name : la préparation a échoué ($side)"; sed -n '1,8p' "$WORK/$name.$side.prepare.log"; status=1; return 1; }
    write_symfony_lock "$d" "$@"
    [ "$shape" = - ] || "$shape" post "$d"
  done
}

# L'arbre du projet hors vendor/ : chemin, mode, empreinte.
mode_of() { if [ "$(uname -s)" = Darwin ]; then stat -f '%Sp' "$1"; else stat -c '%A' "$1"; fi; }
tree_state() { (cd "$1" && find . -path ./vendor -prune -o -path ./.git -prune -o -type f -print | LC_ALL=C sort | while IFS= read -r f; do printf '%s %s %s\n' "$f" "$(mode_of "$f")" "$(cksum < "$f" | tr -s ' ' '-')"; done); }

# vendor/ en une empreinte (chemins, tailles, contenus) : un repli ne doit
# rien y avoir écrit non plus.
vendor_state() { (cd "$1" && find vendor -type f -exec cksum {} + 2>/dev/null | LC_ALL=C sort | cksum); }

SESSION_RE='^Symfony operations: [0-9]+ recipes? \([0-9a-f]{32}\)$'
normalize_err() { sed -E 's/^(Symfony operations: [0-9]+ recipes?) \([0-9a-f]{32}\)$/\1 (<session>)/' "$1"; }

# run <nom> <attendu> <fichier|-> [raison] [options]
#   attendu : recipes (natif, l'oracle applique des recettes, <fichier> créé
#   ou modifié chez lui), none (natif, aucune recette), fallback (repli
#   avec <raison>, rien d'écrit ; l'oracle applique des recettes).
run() {
  local name="$1" expect="$2" file="$3" reason="${4:-}" extra="${5:-}"
  local ref="$WORK/ref-$name" viv="$WORK/viv-$name" c=0 v=0
  local before_viv before_vendor
  before_viv="$(tree_state "$viv")"; before_vendor="$(vendor_state "$viv")"
  cp "$ref/composer.lock" "$WORK/$name.lock.converged"
  # L'empreinte de chaque fichier attendu chez la référence, avant.
  local f sums="$WORK/$name.expected-before"; mkdir -p "$sums"
  for f in $(printf '%s' "$file" | tr ',' ' '); do
    [ "$f" = - ] && continue
    if [ -f "$ref/$f" ]; then cksum < "$ref/$f"; else echo absent; fi > "$sums/$(printf '%s' "$f" | tr '/' '_')"
  done
  (cd "$ref" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION="$root_version" \
    composer update --no-scripts $extra --no-interaction --no-audit --no-ansi --no-blocking >"$WORK/$name.composer.out" 2>"$WORK/$name.composer.err") || c=$?
  (cd "$viv" && COMPOSER_HOME="$home" COMPOSER_CACHE_DIR="$CACHE_DIR" COMPOSER_ROOT_VERSION="$root_version" \
    "$VIVACITY" update $extra --no-fallback --no-blocking >"$WORK/$name.vivacity.out" 2>"$WORK/$name.vivacity.err") || v=$?
  # L'oracle d'abord : ce que Composer + Flex ont vraiment fait.
  local applied=0
  grep -qE '^Symfony operations: ' "$WORK/$name.composer.err" && applied=1
  # Sous -q, la ligne n'est pas imprimée : c'est le fichier qui témoigne.
  case "$expect" in
    recipes|fallback)
      if [ $applied = 0 ] && ! printf '%s' "$extra" | grep -q -- '-q'; then
        echo "FAIL $name : l'oracle n'a appliqué aucune recette (pas de « Symfony operations »)"; status=1; return
      fi
      for f in $(printf '%s' "$file" | tr ',' ' '); do
        [ "$f" = - ] && continue
        if [ ! -f "$ref/$f" ] || [ "$(cksum < "$ref/$f")" = "$(cat "$sums/$(printf '%s' "$f" | tr '/' '_')")" ]; then
          echo "FAIL $name : l'oracle n'a ni créé ni modifié $f"; status=1; return
        fi
      done ;;
    none)
      if [ $applied = 1 ]; then echo "FAIL $name : l'oracle a appliqué des recettes"; status=1; return; fi ;;
  esac
  if [ $applied = 1 ] && ! grep -qE "$SESSION_RE" "$WORK/$name.composer.err"; then
    echo "FAIL $name : session de Composer hors format"; grep '^Symfony operations' "$WORK/$name.composer.err"; status=1; return
  fi
  if [ "$expect" = fallback ]; then
    if [ "$(tree_state "$viv")" != "$before_viv" ]; then
      echo "FAIL $name : vivacity a écrit avant le repli"; diff <(echo "$before_viv") <(tree_state "$viv") | head -5; status=1; return
    fi
    if [ "$(vendor_state "$viv")" != "$before_vendor" ]; then
      echo "FAIL $name : vivacity a écrit dans vendor/ avant le repli"; status=1; return
    fi
    if [ "$v" = 3 ] && grep -qF -- "$reason" "$WORK/$name.vivacity.err"; then
      echo "OK   $name : rendu à Composer ($reason)"
    else
      echo "FAIL $name : attendu un repli sur « $reason », code $v"; tail -4 "$WORK/$name.vivacity.err"; status=1
    fi
    return
  fi
  if [ "$c" != "$v" ]; then echo "FAIL $name : codes $c vs $v"; tail -4 "$WORK/$name.vivacity.err"; status=1; return; fi
  if grep -qE '^Symfony operations: ' "$WORK/$name.vivacity.err" && ! grep -qE "$SESSION_RE" "$WORK/$name.vivacity.err"; then
    echo "FAIL $name : session de vivacity hors format"; status=1; return
  fi
  for f in composer.json composer.lock symfony.lock; do
    if ! cmp -s "$ref/$f" "$viv/$f"; then echo "FAIL $name : $f diffère"; diff "$ref/$f" "$viv/$f" | head -6; status=1; return; fi
  done
  # `Warning: Accessing 127.0.0.1 over http…` : artefact du banc (index et
  # recettes servis en http local, `secure-http: false`). `  - Downloading
  # …` : cache froid.
  if ! diff <(normalize_err "$WORK/$name.composer.err" | grep -vE '^  - Downloading |^Warning: Accessing 127\.0\.0\.1 over http ') \
            <(normalize_err "$WORK/$name.vivacity.err" | grep -v '^vivacity: \|^Note: plugin ') >"$WORK/$name.err.diff"; then
    echo "FAIL $name : stderr différente"; head -10 "$WORK/$name.err.diff"; status=1; return
  fi
  if ! diff "$WORK/$name.composer.out" "$WORK/$name.vivacity.out" >"$WORK/$name.out.diff"; then
    echo "FAIL $name : stdout différente"; head -6 "$WORK/$name.out.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    local n; n=$(grep -cE '^  - Configuring ' "$WORK/$name.composer.err" || true)
    echo "OK   $name : projet, composer.lock, symfony.lock et stderr identiques ($n recette(s))"
  else
    echo "FAIL $name : projet différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}

# copy-from-recipe : `bin/console`, exécutable (`chmod | 0111`).
prepare copy-exec        -                    symfony/console          && run copy-exec        recipes bin/console
# bundles : la ligne ajoutée à un bundles.php existant ; conflit de
# recette (framework-bundle <5.3) et `recipe-conflicts` évalués, sans effet.
prepare bundles-existing shape_bundles_twig   symfony/twig-bundle      && run bundles-existing recipes config/bundles.php
prepare bundles-absent   -                    symfony/twig-bundle      && run bundles-absent   recipes config/bundles.php
# La recette auto-générée : pas de recette dans l'index, une classe de bundle.
prepare auto-bundle      shape_bundles_extra  twig/extra-bundle        && run auto-bundle      recipes config/bundles.php
# gitignore : créé, puis ajouté à un fichier existant.
prepare gitignore-new    -                    friendsofphp/php-cs-fixer && run gitignore-new   recipes .gitignore
prepare gitignore-append shape_gitignore      friendsofphp/php-cs-fixer && run gitignore-append recipes .gitignore
# env : `.env` (commentaires `#n`) et phpunit.dist.xml (`<env …/>`, sans .env.test).
prepare env-phpunit      shape_env_phpunit    symfony/routing          && run env-phpunit      recipes .env,phpunit.dist.xml
# env : guillemets (`!`, `&`) dans .env, échappement XML dans phpunit.dist.xml,
# commentaires XML ; docker à false, donc dockerfile et docker-compose muets.
prepare env-quoting      shape_doctrine       doctrine/doctrine-bundle && run env-quoting      recipes .env,phpunit.dist.xml
# framework-bundle : bundles (fusion de FrameworkBundle), copy (fichier déjà
# là non écrasé), env, dotenv sans .env.dev, composer-scripts (composer.json
# puis composer.lock réécrit par `updateComposerLock`), gitignore.
prepare framework        shape_framework      symfony/framework-bundle && run framework        recipes composer.json
if [ -f "$WORK/ref-framework/composer.lock" ] && cmp -s "$WORK/ref-framework/composer.lock" "$WORK/framework.lock.converged" 2>/dev/null; then
  echo "FAIL framework : composer.lock de la référence inchangé"; status=1
fi
# add-lines (après TOUTES les recettes) : phpunit.dist.xml copié puis
# patché `after_target`, `requires` doctrine/deprecations installé ; deux
# exécutables (bin/phpunit) ; .env.test.
prepare add-lines        -                    phpunit/phpunit          && run add-lines        recipes phpunit.dist.xml
prepare install-recipe   shape_install_recipe friendsofphp/php-cs-fixer && run install-recipe recipes .php-cs-fixer.dist.php
prepare phpunit-swap     shape_phpunit_swap   phpunit/phpunit          && run phpunit-swap     recipes phpunit.xml.dist
# add-lines `warn_if_missing` au niveau normal (templates/base.html.twig
# absent), composer-scripts, importmap.php créé par la recette, sans paquet
# symfony-ux : la synchronisation qui suit les recettes n'écrit rien.
prepare asset-mapper     shape_asset_mapper   symfony/asset-mapper     && run asset-mapper     recipes importmap.php
prepare docker-off       shape_docker_off     symfony/mailer           && run docker-off       recipes config/packages/mailer.yaml
for f in compose.yaml compose.override.yaml docker-compose.yml docker-compose.override.yml; do
  [ -e "$WORK/ref-docker-off/$f" ] && { echo "FAIL docker-off : la référence a écrit $f"; status=1; }
done
prepare contrib          -                    phpstan/phpstan          && run contrib          recipes phpstan.dist.neon
# L'ordre : framework-bundle en tête, puis l'ordre des opérations, contrib comprise.
prepare several          shape_several        symfony/console symfony/twig-bundle friendsofphp/php-cs-fixer symfony/routing symfony/framework-bundle phpstan/phpstan \
                                                                       && run several          recipes bin/console
prepare no-sync          shape_no_sync        symfony/console          && run no-sync          recipes bin/console
prepare quiet            -                    symfony/console          && run quiet            recipes bin/console "" -q
# Pas de recette : tout est dans symfony.lock ; phpunit (dev) sous --no-dev ;
# Flex non chargé sous --no-plugins.
prepare already-locked   -                                             && run already-locked   none -
prepare no-dev           -                    phpunit/phpunit          && run no-dev           none - "" --no-dev
prepare no-plugins       -                    symfony/console          && run no-plugins       none - "" --no-plugins
[ -e "$WORK/viv-no-plugins/bin/console" ] && { echo "FAIL no-plugins : bin/console écrit"; status=1; }
# Replis : ce qui n'est pas reproduit, nommé.
prepare docker-unset     -                    symfony/mailer           && run docker-unset     fallback - "extra.symfony.docker is not set"
prepare dotenv-secret    shape_framework_secret symfony/framework-bundle && run dotenv-secret  fallback - "%generate(secret)%"
prepare contrib-denied   shape_no_contrib     phpstan/phpstan          && run contrib-denied   fallback - "allow-contrib is off"
# importmap.php créé par la recette, paquets symfony-ux présents :
# `synchronizePackageJson`, après les recettes, l'alimenterait.
prepare asset-mapper-ux  -                    symfony/asset-mapper     && run asset-mapper-ux  fallback - "symfony-ux"
# bin-dir à `bin` : l'install y pose ses proxys avant que Flex n'y copie.
prepare bin-dir          shape_bin_dir        symfony/console          && run bin-dir          fallback - "which the install itself writes"
exit $status
