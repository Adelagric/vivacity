#!/usr/bin/env bash
# Le comparateur se compare lui-même : `harness/lib/compare.sh` porte des
# tolérances (le stub `autoload_runtime.php`, les lignes d'erreur de `diff` sur
# un lien pendant, le préfixe APCu aléatoire), et une tolérance mal ancrée est
# pire que pas de tolérance — elle fait disparaître la différence qu'elle
# touche. Jusqu'au 2026-10-01 c'étaient des `grep -v` en sous-chaîne : un paquet
# livrant `src/autoload_runtime.php` effaçait sa propre ligne.
#
# Chaque cas fabrique deux arbres et exige un verdict. Aucun réseau, pas de
# Composer, pas de php sauf pour les cas qui en ont besoin.
#
# Usage : harness/compare-selftest.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/compare-selftest"
rm -rf "$WORK"; mkdir -p "$WORK"
status=0

# case <nom> <verdict attendu : same|differ> ; les deux arbres sont déjà en place
expect() {
  local name="$1" want="$2" code=0
  compare_vendor "$WORK/$name/ref" "$WORK/$name/viv" "$WORK/$name.diff" >/dev/null || code=$?
  local got=same; [ "$code" = 0 ] || got=differ
  if [ "$got" = "$want" ]; then
    echo "OK   $name : $want"
  else
    echo "FAIL $name : attendu $want, obtenu $got"
    head -6 "$WORK/$name.diff" 2>/dev/null
    status=1
  fi
}

# Un projet minimal, identique des deux côtés.
seed() {
  local name="$1" d
  for side in ref viv; do
    d="$WORK/$name/$side/vendor"
    mkdir -p "$d/composer" "$d/acme/pkg/src"
    printf '<?php // autoload\n' > "$d/autoload.php"
    printf '<?php return array();\n' > "$d/composer/installed.php"
    printf '<?php class A {}\n' > "$d/acme/pkg/src/A.php"
  done
}

# 1. deux arbres identiques : identiques.
seed identical
expect identical same

# 2. le stub de vivacity à la racine du vendor : toléré.
seed stub
printf '<?php // runtime stub\n' > "$WORK/stub/viv/vendor/autoload_runtime.php"
expect stub same

# 3. LE CAS QUI MANQUAIT : un paquet qui livre son propre
#    `src/autoload_runtime.php`, présent d'un seul côté. La tolérance ne doit
#    pas s'appliquer.
seed pkgruntime
printf '<?php // le fichier du paquet\n' > "$WORK/pkgruntime/viv/vendor/acme/pkg/src/autoload_runtime.php"
expect pkgruntime differ

# 4. le même fichier présent des deux côtés mais DIFFÉRENT.
seed pkgruntime2
printf 'A\n' > "$WORK/pkgruntime2/ref/vendor/acme/pkg/src/autoload_runtime.php"
printf 'B\n' > "$WORK/pkgruntime2/viv/vendor/acme/pkg/src/autoload_runtime.php"
expect pkgruntime2 differ

# 5. un fichier qui porte le nom du message d'erreur de `diff`.
seed message
printf 'A\n' > "$WORK/message/ref/vendor/acme/pkg/No such file or directory"
printf 'B\n' > "$WORK/message/viv/vendor/acme/pkg/No such file or directory"
expect message differ

# 6. un lien pendant, identique des deux côtés : `diff -r` s'en plaint, et cette
#    plainte-là est tolérée.
seed dangling
for side in ref viv; do ln -s nowhere "$WORK/dangling/$side/vendor/acme/pkg/link"; done
expect dangling same

# 7. le préfixe APCu : ignoré dans autoload_real.php…
seed apcu
printf "<?php \$loader->setApcuPrefix('aaaaaaaaaa');\n" > "$WORK/apcu/ref/vendor/composer/autoload_real.php"
printf "<?php \$loader->setApcuPrefix('bbbbbbbbbb');\n" > "$WORK/apcu/viv/vendor/composer/autoload_real.php"
expect apcu same

# 8. …et nulle part ailleurs.
seed apcu2
printf "<?php \$loader->setApcuPrefix('aaaaaaaaaa');\n" > "$WORK/apcu2/ref/vendor/acme/pkg/src/B.php"
printf "<?php \$loader->setApcuPrefix('bbbbbbbbbb');\n" > "$WORK/apcu2/viv/vendor/acme/pkg/src/B.php"
expect apcu2 differ

# 9. un mode qui diffère, contenu identique : `diff -r` ne le voit pas,
#    l'inventaire si.
seed mode
chmod 0600 "$WORK/mode/viv/vendor/acme/pkg/src/A.php"
expect mode differ

# 10. une cible de lien qui diffère, contenu identique de part et d'autre.
seed linktarget
ln -s A.php "$WORK/linktarget/ref/vendor/acme/pkg/src/alias.php"
ln -s ../src/A.php "$WORK/linktarget/viv/vendor/acme/pkg/src/alias.php"
expect linktarget differ

# 11. le stub toléré ne couvre pas un nom qui le prolonge.
seed stubbak
printf 'x\n' > "$WORK/stubbak/viv/vendor/autoload_runtime.php.bak"
expect stubbak differ

exit $status
