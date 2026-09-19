#!/usr/bin/env bash
# Harness v0.15 (perf, P2) : le cache par fichier des scans hors store
# (sources du projet) ne change rien à ce que Composer produit. Sur la
# fixture scripts (`classmap` racine + psr-4), une suite d'installs où un
# fichier scanné est ajouté, modifié (nouvelle classe), remplacé par un
# fichier de même taille, puis supprimé, entre deux installs : à chaque
# étape vendor/ (et donc autoload_classmap.php / autoload_static.php) doit
# être identique à celui de Composer, cache chaud ou non. Le cas
# « remplacé dans la même seconde, même taille » est le risque résiduel
# documenté : le harness attend une seconde avant cette étape pour le
# tenir hors du cas testé.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="$ROOT/target/release/vivacity"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/root-scan"
SRC="$ROOT/fixtures/projects/scripts"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
# Un cache de classmap propre à ce run : le premier install le remplit, les
# suivants l'exercent.
export VIVACITY_CACHE_DIR="$WORK/cache"

ref="$WORK/ref"; viv="$WORK/viv"
for d in "$ref" "$viv"; do
  mkdir -p "$d"; (cd "$SRC" && tar -cf - .) | (cd "$d" && tar -xf -)
  jq '.autoload.classmap = ["lib/"] | .config["optimize-autoloader"] = true' "$SRC/composer.json" > "$d/composer.json"
  mkdir -p "$d/lib"
done
status=0

both() { # applique une commande shell dans les deux copies
  (cd "$ref" && eval "$1"); (cd "$viv" && eval "$1")
}
# Les avertissements du dump (« Ambiguous class resolution », violations
# PSR) : mêmes lignes, même ordre — les chemins absolus des deux copies
# ramenés à un préfixe commun.
warnings() { sed -e "s|$ref|<p>|g; s|$viv|<p>|g" "$1" | grep -E "Ambiguous class|does not comply|To resolve ambiguity" || true; }
step() { # nom
  local name="$1" c_code=0 v_code=0
  (cd "$ref" && composer install --no-plugins --no-scripts --no-interaction --no-ansi -o >/dev/null 2>"$WORK/$name.composer.err") || c_code=$?
  (cd "$viv" && "$VIVACITY" install --no-plugins --no-fallback -o >/dev/null 2>"$WORK/$name.vivacity.err") || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.err"; status=1; return; fi
  if ! diff <(warnings "$WORK/$name.composer.err") <(warnings "$WORK/$name.vivacity.err") >"$WORK/$name.warnings.diff"; then
    echo "FAIL $name : avertissements du dump différents"; head -6 "$WORK/$name.warnings.diff"; status=1; return
  fi
  if compare_vendor "$ref/vendor" "$viv/vendor" "$WORK/$name.diff" >/dev/null; then
    local n; n=$(grep -c "=>" "$viv/vendor/composer/autoload_classmap.php" || true)
    echo "OK   $name : vendor/ identique ($n entrées de classmap)"
  else
    echo "FAIL $name : vendor/ différent"; head -10 "$WORK/$name.diff"; status=1
  fi
}

step "initial (cache vide)"
step "no-op (cache chaud)"
both 'printf "<?php\nclass LibAdded {}\n" > lib/Added.php'
step "fichier ajouté"
both 'printf "<?php\nclass LibAdded {}\nclass LibAddedTwo {}\n" > lib/Added.php'
step "fichier modifié (classe en plus)"
sleep 1
both 'printf "<?php\nclass LibAddedX {}\nclass LibAddedTwo {}\n" > lib/Added.php'
step "fichier réécrit, même taille, seconde suivante"
both 'mkdir -p lib/sub && printf "<?php\nnamespace Lib\\\\Sub;\nclass Deep {}\n" > lib/sub/Deep.php'
step "sous-répertoire ajouté"
both 'rm lib/Added.php'
step "fichier supprimé"
both 'printf "<?php\nnamespace App;\nclass FromPsr4 {}\n" > src/FromPsr4.php'
step "classe psr-4 ajoutée (-o)"
both 'rm -rf lib/sub src/FromPsr4.php'
step "retour à l'état initial"
# Le gagnant d'une classe ambiguë suit l'ordre readdir du répertoire, comme
# le Finder de Composer (insensible à la casse sur APFS/NTFS : bar avant Foo).
both 'printf "<?php\nclass LibDup {}\n" > lib/Foo.php; printf "<?php\nclass LibDup {}\n" > lib/bar.php; printf "<?php\nclass LibDup {}\n" > lib/Zed.php'
step "classe ambiguë dans trois fichiers (Foo, bar, Zed)"
both 'mkdir -p src/Sub && printf "<?php\nnamespace App\\\\Wrong;\nclass Misplaced {}\n" > src/Sub/Misplaced.php && printf "<?php\nnamespace App\\\\Sub;\nclass Misplaced {}\n" > src/Sub/Other.php'
step "violation PSR-4 et doublon psr-4 (-o)"
both 'rm -rf lib/Foo.php lib/bar.php lib/Zed.php src/Sub'
step "nettoyage"
exit $status
