#!/usr/bin/env bash
# Écrit fixtures/corpus/<nom>/UPSTREAM : les fichiers d'un projet du corpus
# (entrée `git`) qu'il faut sur le disque pour que la référence s'exécute
# comme dans le vrai projet, mais qui ne sont pas commités ici — code sous
# la licence du projet (GPL pour chamilo-lms), tiré au commit de PROVENANCE
# par harness/corpus.sh au moment de la mesure, vérifié par sa somme.
#
# Format : `# <owner/repo>@<sha>` puis une ligne `<sha256>  <chemin>` par
# fichier (chemins relatifs à la racine du projet).
#
# Usage : tools/corpus-upstream.sh <nom> [<chemin>…]
#         (sans chemin : ceux de l'UPSTREAM existant, au commit de
#         PROVENANCE — après un rafraîchissement par tools/corpus-add.sh)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
name="${1:?nom d’entrée}"; shift
dest="$ROOT/fixtures/corpus/$name"
read -r kind url sha _ < "$dest/PROVENANCE"
[ "$kind" = git ] || { echo "$name : entrée $kind, pas git"; exit 1; }
repo="${url#https://github.com/}"
paths=("$@")
if [ ${#paths[@]} -eq 0 ]; then
  [ -f "$dest/UPSTREAM" ] || { echo "$name : ni chemin ni UPSTREAM"; exit 1; }
  while read -r sum p; do case "$sum" in ''|'#'*) continue ;; esac; paths+=("$p"); done < "$dest/UPSTREAM"
fi
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d' ' -f1; else shasum -a 256 | cut -d' ' -f1; fi; }
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
{
  echo "# $repo@$sha"
  for p in "${paths[@]}"; do
    curl -fsSL --retry 3 -o "$work/f" "https://raw.githubusercontent.com/$repo/$sha/$p" || { echo "$p : introuvable à $sha" >&2; exit 1; }
    echo "$(sha256 < "$work/f")  $p"
  done
} > "$work/UPSTREAM"
mv "$work/UPSTREAM" "$dest/UPSTREAM"
echo "OK   $name : ${#paths[@]} fichier(s) à $repo@$sha"
