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

# 12-19. Les `.git` des paquets installés depuis leur source
#    (docs/plans/git-sources.md) : à l'octet sauf l'index, l'horodatage des
#    reflogs et le rangement des objets. Un dépôt de deux commits, cloné de
#    chaque côté comme GitDownloader le fait (clone sans checkout, puis
#    checkout), à une seconde d'écart pour que les horodatages diffèrent.
if command -v git >/dev/null; then
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_CEILING_DIRECTORIES="$WORK"
  src="$WORK/git-origin"
  git init -q -b main "$src"
  git -C "$src" -c user.name=T -c user.email=t@e commit -q --allow-empty -m one
  printf 'x\n' > "$src/f"; git -C "$src" add f
  git -C "$src" -c user.name=T -c user.email=t@e commit -q -m two
  first="$(git -C "$src" rev-parse HEAD~1)"
  # git_seed <nom> : vendor/acme/pkg cloné des deux côtés, sur `main`.
  git_seed() {
    local name="$1" side
    seed "$name"
    for side in ref viv; do
      rm -rf "$WORK/$name/$side/vendor/acme/pkg"
      git clone -q --no-checkout -- "$src" "$WORK/$name/$side/vendor/acme/pkg"
      git -C "$WORK/$name/$side/vendor/acme/pkg" checkout -q main --
      [ "$side" = ref ] && sleep 1
    done
  }
  # 12. deux clones identiques à l'horodatage près : identiques.
  git_seed git-same
  expect git-same same
  # 13. HEAD sur un autre commit.
  git_seed git-head
  git -C "$WORK/git-head/viv/vendor/acme/pkg" checkout -q "$first" --
  expect git-head differ
  # 14. une ligne de configuration de plus (le `pushurl` de setPushUrl).
  git_seed git-config
  git -C "$WORK/git-config/viv/vendor/acme/pkg" remote set-url --push origin -- git@example.com:a/b.git
  expect git-config differ
  # 15. un reflog qui dit autre chose que l'heure.
  git_seed git-reflog
  printf '%s %s T <t@e> 1 +0000\tcheckout: moving from main to main\n' "$first" "$first" >> "$WORK/git-reflog/viv/vendor/acme/pkg/.git/logs/HEAD"
  expect git-reflog differ
  # 16. un objet de plus d'un côté.
  git_seed git-object
  printf 'extra\n' | git -C "$WORK/git-object/viv/vendor/acme/pkg" hash-object -w --stdin >/dev/null
  expect git-object differ
  # 17. les mêmes objets, empaquetés d'un côté, libres de l'autre : identiques.
  git_seed git-pack
  git -C "$WORK/git-pack/viv/vendor/acme/pkg" -c repack.updateServerInfo=false repack -q -a -d
  git -C "$WORK/git-pack/viv/vendor/acme/pkg" prune-packed
  expect git-pack same
  # 18. un fichier modifié dans l'arbre de travail : vu par le diff ET par
  #     `git status`, jamais toléré.
  git_seed git-dirty
  printf 'y\n' > "$WORK/git-dirty/viv/vendor/acme/pkg/f"
  expect git-dirty differ
  # 19. un `.git` hors du vendor (racine du projet) reste exclu.
  seed git-root
  for side in ref viv; do git init -q "$WORK/git-root/$side"; done
  printf 'x\n' > "$WORK/git-root/viv/.git/description"
  expect git-root same
else
  echo "FAIL : git requis pour les cas .git"; status=1
fi

exit $status
