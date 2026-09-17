#!/usr/bin/env bash
# Ajoute (ou rafraîchit) une entrée du corpus (fixtures/corpus/<nom>/) : un
# projet PHP réel réduit à ce qu'un `install` lit — composer.json,
# composer.lock, les fichiers `files` de l'autoload racine (copiés tels
# quels) et les répertoires psr-4/psr-0/classmap de la racine (vides, avec
# un .gitkeep : Composer refuse de scanner un répertoire absent), plus une
# ligne PROVENANCE (origine, commit ou version, date, Composer utilisé).
#
# Deux sortes d'entrées :
#   git <owner/repo> [ref]      dépôt GitHub qui commite son lock ; `ref`
#                               (branche, tag, sha ; défaut : HEAD) est
#                               résolu en sha, json et lock lus à ce sha,
#                               l'arbre racine via un tarball codeload
#   template <vendor/name> [v]  gabarit `create-project` sans lock : le lock
#                               est résolu UNE fois, ici, par Composer
#                               (`update --no-install`, plugins chargés, pas
#                               de scripts), à la date notée
#
# Usage : tools/corpus-add.sh git composer/composer
#         tools/corpus-add.sh template laravel/laravel
#         tools/corpus-add.sh template symfony/skeleton "7.3.*"
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CORPUS="$ROOT/fixtures/corpus"
kind="${1:?git|template}"; source="${2:?owner/repo ou vendor/name}"; ref="${3:-}"
# Nom de l'entrée : `<vendor>-<nom>` sans répéter le vendor (laravel/laravel
# → laravel, slim/slim-skeleton → slim-skeleton, cakephp/app → cakephp-app).
vendor="$(echo "${source%%/*}" | tr '[:upper:]' '[:lower:]')"; short="$(echo "${source#*/}" | tr '[:upper:]' '[:lower:]')"
short="${short#"$vendor"-}"; if [ "$short" = "$vendor" ]; then short=""; fi
name="$vendor${short:+-$short}"
if [ -n "${CORPUS_NAME:-}" ]; then name="$CORPUS_NAME"; fi
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
tree="$work/tree"; mkdir -p "$tree"
today="$(date -u +%Y-%m-%d)"
composer_version="$(composer --version --no-ansi 2>/dev/null | sed -n 's/^Composer version \([^ ]*\).*/\1/p')"

case "$kind" in
  git)
    sha="$(gh api "repos/$source/commits/${ref:-HEAD}" --jq .sha)"
    curl -fsSL "https://codeload.github.com/$source/tar.gz/$sha" | tar -xzf - -C "$tree" --strip-components=1
    [ -f "$tree/composer.lock" ] || { echo "$source@$sha ne commite pas de composer.lock : entrée template ?"; exit 1; }
    provenance="git https://github.com/$source $sha fetched $today"
    ;;
  template)
    # Plugins chargés (un gabarit Symfony n'existe qu'à travers Flex), jamais
    # de scripts. Le PHP local peut être trop récent pour le gabarit
    # (laminas exige ~8.3) : second essai en ignorant l'exigence `php`, noté
    # dans PROVENANCE pour que le harnais l'ignore aussi.
    php_flag=""; log="$work/create.log"
    if ! (cd "$work" && composer create-project --no-install --no-scripts --no-interaction --no-ansi "$source" tree ${ref:+"$ref"} >"$log" 2>&1); then
      rm -rf "$tree"; php_flag="--ignore-platform-req=php"
      (cd "$work" && composer create-project --no-install --no-scripts --no-interaction --no-ansi $php_flag "$source" tree ${ref:+"$ref"} >"$log" 2>&1) || { grep -v '^ *$' "$log" | head -5; exit 1; }
    fi
    # La version du gabarit est dans la ligne « Installing vendor/name (vX) »
    # (le squelette n'a pas de VCS, `show -s` ne sait rien).
    version="$(sed -n "s/^.*Installing $(echo "$source" | sed 's/[\/&]/\\&/g') (\([^)]*\)).*$/\1/p" "$log" | head -1)"
    rm -f "$tree/composer.lock"
    (cd "$tree" && composer update --no-install --no-scripts --no-interaction --no-audit --ignore-platform-req='ext-*' $php_flag --quiet) || { echo "update a échoué pour $source"; exit 1; }
    provenance="template $source ${version:-?} lock resolved $today with Composer $composer_version (plugins on, no scripts${php_flag:+, php-ignored})"
    ;;
  *) echo "kind inconnu : $kind"; exit 1 ;;
esac

dest="$CORPUS/$name"; rm -rf "$dest"; mkdir -p "$dest"
cp "$tree/composer.json" "$tree/composer.lock" "$dest/"
# Ce que l'autoload de la racine exige sur le disque.
jq -r '[.autoload, .["autoload-dev"]] | map(select(. != null)) | .[] |
        ((.files // [])[] | "file\t" + .),
        ((.classmap // [])[] | "path\t" + .),
        (((.["psr-4"] // {}) + (.["psr-0"] // {})) | to_entries[] | .value | if type == "array" then .[] else . end | "dir\t" + .)' \
  "$tree/composer.json" | sort -u | while IFS=$'\t' read -r what p; do
  p="${p#./}"; p="${p%/}"
  case "$what" in
    file) if [ -f "$tree/$p" ]; then mkdir -p "$dest/$(dirname "$p")"; cp "$tree/$p" "$dest/$p"; fi ;;
    path) if [ -f "$tree/$p" ]; then mkdir -p "$dest/$(dirname "$p")"; cp "$tree/$p" "$dest/$p"
          else mkdir -p "$dest/$p" && touch "$dest/$p/.gitkeep"; fi ;;
    dir)  if [ -n "$p" ]; then mkdir -p "$dest/$p" && touch "$dest/$p/.gitkeep"; fi ;;
  esac
done
# Les manifestes que wikimedia/composer-merge-plugin fusionne (`extra.
# merge-plugin.include` / `require`, globs PHP sans option, récursifs), avec
# ce que leur autoload exige — sans eux le plugin serait un no-op muet sur la
# fixture, et la référence ne représenterait pas le projet.
merge_autoload_paths() { # manifeste, préfixe de répertoire
  jq -r --arg base "$2" '[.autoload, .["autoload-dev"]] | map(select(. != null)) | .[] |
        ((.files // [])[] | "file\t" + $base + .),
        ((.classmap // [])[] | "path\t" + $base + .),
        (((.["psr-4"] // {}) + (.["psr-0"] // {})) | to_entries[] | .value | if type == "array" then .[] else . end | "dir\t" + $base + .)' "$1"
}
copy_autoload_paths() { # lignes "what<TAB>path"
  sort -u | while IFS=$'\t' read -r what p; do
    p="${p#./}"; p="${p%/}"
    case "$what" in
      file) if [ -f "$tree/$p" ]; then mkdir -p "$dest/$(dirname "$p")"; cp "$tree/$p" "$dest/$p"; fi ;;
      path) if [ -f "$tree/$p" ]; then mkdir -p "$dest/$(dirname "$p")"; cp "$tree/$p" "$dest/$p"
            else mkdir -p "$dest/$p" && touch "$dest/$p/.gitkeep"; fi ;;
      dir)  if [ -n "$p" ]; then mkdir -p "$dest/$p" && touch "$dest/$p/.gitkeep"; fi ;;
    esac
  done
}
merge_includes() { # manifeste (chemin relatif à $tree)
  local m="$1" base pattern f
  base="$(dirname "$m")"; [ "$base" = "." ] && base="" || base="$base/"
  jq -r '.extra["merge-plugin"] // {} | [(.include // []), (.require // [])] | flatten | .[]' "$tree/$m" 2>/dev/null | while read -r pattern; do
    [ -n "$pattern" ] || continue
    (cd "$tree" && php -r 'foreach (glob($argv[1]) as $f) echo "$f\n";' "$base$pattern") | while read -r f; do
      [ -f "$tree/$f" ] || continue
      mkdir -p "$dest/$(dirname "$f")"; cp "$tree/$f" "$dest/$f"
      merge_autoload_paths "$tree/$f" "$(dirname "$f")/" | copy_autoload_paths
      merge_includes "$f"
    done
  done
}
if jq -e '.extra["merge-plugin"] != null' "$tree/composer.json" >/dev/null 2>&1; then
  merge_includes composer.json
fi
# Le gabarit d'autoload_runtime.php propre au projet (symfony/runtime,
# `extra.runtime.autoload_template`) : sans lui, Composer lui-même échoue.
t="$(jq -r '.extra.runtime.autoload_template // empty' "$tree/composer.json")"
if [ -n "$t" ] && [ -f "$tree/$t" ]; then mkdir -p "$dest/$(dirname "$t")"; cp "$tree/$t" "$dest/$t"; fi
echo "$provenance" > "$dest/PROVENANCE"
n="$(jq '(.packages | length) + (.["packages-dev"] | length)' "$dest/composer.lock")"
echo "OK   $name : $n paquets, $(find "$dest" -type f | wc -l | tr -d ' ') fichiers — $provenance"
