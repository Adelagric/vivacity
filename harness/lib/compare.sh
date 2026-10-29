# La comparaison de vendor/ (ou du projet) entre la copie de Composer et
# celle de vivacity, partagée par harness/diff-vendor.sh et harness/corpus.sh.
#
# Écarts tolérés (documentés). Chacun est **ancré** : un paquet qui livre un
# fichier portant le nom d'une tolérance ne doit pas faire disparaître sa propre
# différence. Les tolérances étaient des `grep -v` en sous-chaîne jusqu'au
# 2026-10-01 ; `harness/compare-selftest.sh` tient la nuance.
#   - <vendor>/autoload_runtime.php, et lui seul : généré par vivacity
#     (émulation symfony/runtime), absent d'un install Composer sans plugins.
#     Un `src/autoload_runtime.php` d'un paquet est comparé comme le reste ;
#   - les lignes d'ERREUR de `diff` (`diff: <chemin>: No such file or
#     directory`) : `diff -r` ne sait pas suivre un symlink pendant (identique
#     des deux côtés, vérifié par l'inventaire). Un fichier qui s'appellerait
#     « No such file or directory » n'est pas toléré pour autant ;
#   - vendor/composer/include_paths.php (paquets PEAR à `include-path`) :
#     Composer l'écrit dans l'ordre d'achèvement des extractions asynchrones
#     (non déterministe, vérifié le 2026-09-11) ; comparé trié ;
#   - `$loader->setApcuPrefix('…')` **dans vendor/composer/autoload_real.php** :
#     préfixe aléatoire chez Composer (`bin2hex(random_bytes(10))`) ; la ligne
#     est ignorée là, et nulle part ailleurs ;
#   - `    public const GENERATED_AT = <entier>;` **dans les cinq classes de
#     <vendor>/drupol/composer-packages/build/** (Packages, Types,
#     Directories, Versions, Dependencies) : `time()` chez le plugin ; les
#     seuls chiffres de cette ligne exacte deviennent `<t>` des deux côtés,
#     puis la comparaison reste stricte (une ligne absente ou dédoublée
#     diffère toujours) ;
#   - le chemin absolu du projet dans un fichier généré : remplacé par
#     `<project>` de chaque côté avant comparaison ;
#   - vendor/pest-plugins.json : le plugin lit le dépôt local dans l'ordre
#     d'achèvement des installations asynchrones (constaté : le petit
#     pest-plugin-laravel finit avant pest) ; comparé trié ;
#   - vendor/yiisoft/extensions.php (yii2-composer) : même ordre
#     d'achèvement ; comparé clés triées via `php -r`.
# Les modes des fichiers et les cibles des liens sont comparés par un
# inventaire `stat` (`diff -r` ne voit ni les uns ni les autres) : une
# extraction par `unzip` (Composer) préserve les modes du zip, vivacity aussi.

# compare_vendor <scope de Composer> <scope de vivacity> [fichier de diff]
# Renvoie 0 quand identiques ; sinon 1 et imprime les premières lignes.
# Un fichier généré peut contenir le chemin absolu du projet (phpstan/
# extension-installer écrit `install_path` en absolu) : les deux copies
# vivent dans des répertoires différents, le contenu est comparé après
# remplacement du chemin physique de chaque côté par `<project>`.
# Le répertoire vendor du projet (`config.vendor-dir`) est `vendor` sauf
# `VENDOR_REL` : les fichiers tolérés sont cherchés là.
compare_vendor() {
  local ref="$1" viv="$2" out="${3:-$(mktemp)}" vrel="${VENDOR_REL:-vendor}"
  local ip_ref="$ref/$vrel/composer/include_paths.php" ip_viv="$viv/$vrel/composer/include_paths.php"
  [ -d "$ref/$vrel" ] || { ip_ref="$ref/composer/include_paths.php"; ip_viv="$viv/composer/include_paths.php"; }
  if [ -f "$ip_ref" ] && [ -f "$ip_viv" ] && ! diff -q <(sort "$ip_ref") <(sort "$ip_viv") >/dev/null; then
    echo "include_paths.php diffère même trié" > "$out"; diff <(sort "$ip_ref") <(sort "$ip_viv") | head >> "$out"
    head -20 "$out"; return 1
  fi
  # vendor/pest-plugins.json : même cause — le plugin lit le dépôt local
  # dans l'ordre d'achèvement des installations asynchrones ; comparé trié.
  local pp_ref="${ip_ref%composer/include_paths.php}pest-plugins.json" pp_viv="${ip_viv%composer/include_paths.php}pest-plugins.json"
  if [ -f "$pp_ref" ] && [ -f "$pp_viv" ] && ! diff -q <(sed 's/,$//' "$pp_ref" | sort) <(sed 's/,$//' "$pp_viv" | sort) >/dev/null; then
    echo "pest-plugins.json diffère même trié" > "$out"; diff <(sed 's/,$//' "$pp_ref" | sort) <(sed 's/,$//' "$pp_viv" | sort) | head >> "$out"
    head -20 "$out"; return 1
  fi
  # vendor/yiisoft/extensions.php (yii2-composer) : même cause encore —
  # l'installateur du plugin réécrit la carte à chaque opération, dans
  # l'ordre d'achèvement ; comparée clés triées, `$vendorDir` évalué puis
  # ramené à un marqueur.
  local ex_ref="${ip_ref%composer/include_paths.php}yiisoft/extensions.php" ex_viv="${ip_viv%composer/include_paths.php}yiisoft/extensions.php"
  if [ -f "$ex_ref" ] && [ -f "$ex_viv" ]; then
    # (Le préfixe absolu jusqu'à `vendor` ramené à un marqueur — sous
    # Windows il vient avec des antislashs, l'argument de bash avec des
    # slashes ou en forme `/c/…` : on ne compare que la suite.)
    local ex_norm='$a = require $argv[1]; ksort($a); array_walk_recursive($a, function (&$x) { if (is_string($x)) { $x = preg_replace("#^.*[\\\\/]vendor(?=[\\\\/])#", "<vendor>", $x); } }); echo var_export($a, true), "\n";'
    if ! diff -q <(php -r "$ex_norm" "$ex_ref") <(php -r "$ex_norm" "$ex_viv") >/dev/null; then
      echo "yiisoft/extensions.php diffère même trié" > "$out"; diff <(php -r "$ex_norm" "$ex_ref") <(php -r "$ex_norm" "$ex_viv") | head >> "$out"
      head -20 "$out"; return 1
    fi
  fi
  local ref_real viv_real ref_win="" viv_win="" ref_winf="" viv_winf=""
  ref_real="$(cd "$ref" && pwd -P)"; viv_real="$(cd "$viv" && pwd -P)"
  # Windows (Git Bash) : Composer écrit le chemin sous sa forme `D:\a\…`,
  # ou mixte `D:\a\…/vendor/x` ; les deux formes sont normalisées aussi.
  if command -v cygpath >/dev/null 2>&1; then
    # `var_export` double les antislashs : la forme doublée d'abord, puis la simple.
    ref_win="$(cygpath -w "$ref_real" | sed 's/\\/\\\\\\\\/g')|$(cygpath -w "$ref_real" | sed 's/\\/\\\\/g')"
    viv_win="$(cygpath -w "$viv_real" | sed 's/\\/\\\\\\\\/g')|$(cygpath -w "$viv_real" | sed 's/\\/\\\\/g')"
    ref_winf="$(cygpath -m "$ref_real")"; viv_winf="$(cygpath -m "$viv_real")"
  fi
  # (Arguments après le fichier : des expressions `sed` de plus.)
  norm_ref() { sed -E -e "s#$ref_real#<project>#g" -e "s#$ref#<project>#g" ${ref_win:+-e "s#$ref_win#<project>#g"} ${ref_winf:+-e "s#$ref_winf#<project>#g"} "${@:2}" "$1"; }
  norm_viv() { sed -E -e "s#$viv_real#<project>#g" -e "s#$viv#<project>#g" ${viv_win:+-e "s#$viv_win#<project>#g"} ${viv_winf:+-e "s#$viv_winf#<project>#g"} "${@:2}" "$1"; }
  : > "$out"
  # `diff -rq` : les fichiers présents d'un seul côté, et les paires qui
  # diffèrent — celles-ci sont recomparées normalisées.
  # Les deux répertoires où le stub de vivacity est toléré, en absolu : la
  # tolérance porte sur CE chemin, pas sur le nom.
  local runtime_dir_ref runtime_dir_viv
  if [ -d "$ref/$vrel" ]; then
    runtime_dir_ref="$ref/$vrel"; runtime_dir_viv="$viv/$vrel"
  else
    runtime_dir_ref="$ref"; runtime_dir_viv="$viv"
  fi
  diff -rq --no-dereference --exclude=.git --exclude=include_paths.php --exclude=pest-plugins.json --exclude=extensions.php "$ref" "$viv" 2>&1 \
    | while IFS= read -r line; do
      case "$line" in
        # Le stub d'émulation, à la racine du vendor et nulle part ailleurs.
        "Only in $runtime_dir_viv: autoload_runtime.php"|"Only in $runtime_dir_ref: autoload_runtime.php") continue ;;
        # Une ligne d'erreur de `diff` sur un lien pendant, pas un fichier qui
        # s'appellerait comme le message.
        "diff: "*": No such file or directory") continue ;;
        "Files "*" and "*" differ")
          a="${line#Files }"; a="${a% and *}"; b="${line#* and }"; b="${b% differ}"
          # Le préfixe APCu aléatoire n'est ignoré que dans le fichier qui le
          # porte : partout ailleurs, une ligne contenant `setApcuPrefix` est
          # une différence comme une autre.
          local ignore=() gen=()
          case "$a" in
            */composer/autoload_real.php) ignore=(-I 'setApcuPrefix') ;;
            # L'horodatage de drupol/composer-packages, dans ses cinq classes
            # du vendor et nulle part ailleurs.
            "$runtime_dir_ref/drupol/composer-packages/build/Packages.php"|"$runtime_dir_ref/drupol/composer-packages/build/Types.php"|"$runtime_dir_ref/drupol/composer-packages/build/Directories.php"|"$runtime_dir_ref/drupol/composer-packages/build/Versions.php"|"$runtime_dir_ref/drupol/composer-packages/build/Dependencies.php")
              gen=(-e 's/^(    public const GENERATED_AT = )[0-9]+;$/\1<t>;/') ;;
          esac
          # `${a[@]+"${a[@]}"}` : sous `set -u`, le bash 3.2 de macOS refuse
          # `"${a[@]}"` quand le tableau est vide.
          if ! diff ${ignore[@]+"${ignore[@]}"} <(norm_ref "$a" ${gen[@]+"${gen[@]}"}) <(norm_viv "$b" ${gen[@]+"${gen[@]}"}) >/dev/null 2>&1; then
            echo "$line"
            diff <(norm_ref "$a" ${gen[@]+"${gen[@]}"}) <(norm_viv "$b" ${gen[@]+"${gen[@]}"}) | head -6
          fi ;;
        *) echo "$line" ;;
      esac
    done >> "$out"
  diff <(vendor_inventory "$ref") <(vendor_inventory "$viv") >> "$out" || true
  compare_git_dirs "$ref" "$viv" >> "$out"
  if [ -s "$out" ]; then head -20 "$out"; return 1; fi
  return 0
}

# compare_git_dirs <scope de Composer> <scope de vivacity> : les `.git` des
# paquets installés depuis leur source (GitDownloader), c'est-à-dire ceux
# SOUS le vendor — ceux d'ailleurs (racine du projet, dépôts `path` d'un
# banc) restent exclus, comme le reste de compare_vendor les exclut.
# docs/plans/git-sources.md, « Décision de parité pour `.git` » : à l'octet,
# sauf trois familles que deux runs de Composer ne reproduisent pas —
#   - `index` (cache stat) : comparé par `git ls-files --stage` et
#     `git status --porcelain` ;
#   - `logs/**` (reflogs) : le seul champ `<epoch> <fuseau>` masqué ;
#   - `objects/` hors `objects/info/` (packs refaits par `repack` en
#     plusieurs threads) : comparé par l'ensemble des objets.
# Les modes de tout le `.git` hors `objects/` sont comparés aussi.
# N'écrit que les différences ; rien quand tout est identique.
compare_git_dirs() {
  local ref="$1" viv="$2" vrel="${VENDOR_REL:-vendor}" vref vviv
  if [ -d "$ref/$vrel" ]; then vref="$ref/$vrel"; vviv="$viv/$vrel"; else vref="$ref"; vviv="$viv"; fi
  [ -d "$vref" ] && [ -d "$vviv" ] || return 0
  git_dirs_of() { (cd "$1" && find . -name .git -type d -prune -print | LC_ALL=C sort); }
  local list_ref list_viv
  list_ref="$(git_dirs_of "$vref")"; list_viv="$(git_dirs_of "$vviv")"
  if [ "$list_ref" != "$list_viv" ]; then
    echo ".git présents d'un seul côté :"
    diff <(printf '%s\n' "$list_ref") <(printf '%s\n' "$list_viv") | sed -n '2,8p'
  fi
  local g a b name f
  while IFS= read -r g; do
    [ -n "$g" ] || continue
    a="$vref/$g"; b="$vviv/$g"
    [ -d "$b" ] || continue
    # Les entrées de tête, puis chacune à l'octet sauf les trois familles.
    if [ "$(ls -A "$a" | LC_ALL=C sort)" != "$(ls -A "$b" | LC_ALL=C sort)" ]; then
      echo "$g : entrées différentes"
      diff <(ls -A "$a" | LC_ALL=C sort) <(ls -A "$b" | LC_ALL=C sort) | sed -n '2,6p'
    fi
    for name in $(ls -A "$a"); do
      case "$name" in index|logs|objects) continue ;; esac
      [ -e "$b/$name" ] || continue
      if ! diff -r --no-dereference "$a/$name" "$b/$name" >/dev/null 2>&1; then
        echo "$g/$name diffère"
        diff -r --no-dereference "$a/$name" "$b/$name" 2>&1 | head -6
      fi
    done
    # `objects/info/` à l'octet (un `alternates` resté vers le cache serait
    # une vraie différence), sauf les NOMS de packs que `packs` liste.
    if [ -d "$a/objects/info" ] || [ -d "$b/objects/info" ]; then
      if ! diff -r --exclude=packs "$a/objects/info" "$b/objects/info" >/dev/null 2>&1; then
        echo "$g/objects/info diffère"; diff -r --exclude=packs "$a/objects/info" "$b/objects/info" 2>&1 | head -4
      fi
      if [ -f "$a/objects/info/packs" ] || [ -f "$b/objects/info/packs" ]; then
        if [ ! -f "$a/objects/info/packs" ] || [ ! -f "$b/objects/info/packs" ]; then
          echo "$g/objects/info/packs présent d'un seul côté"
        elif ! diff <(sed -E 's/pack-[0-9a-f]+\.pack/pack-N.pack/' "$a/objects/info/packs") \
            <(sed -E 's/pack-[0-9a-f]+\.pack/pack-N.pack/' "$b/objects/info/packs") >/dev/null; then
          echo "$g/objects/info/packs diffère (noms de packs masqués)"
        fi
      fi
    fi
    # L'ensemble des objets, quel que soit leur rangement.
    local objs_a objs_b
    objs_a="$(git --git-dir="$a" cat-file --batch-all-objects --batch-check='%(objectname) %(objecttype) %(objectsize)' 2>&1 | LC_ALL=C sort)"
    objs_b="$(git --git-dir="$b" cat-file --batch-all-objects --batch-check='%(objectname) %(objecttype) %(objectsize)' 2>&1 | LC_ALL=C sort)"
    if [ "$objs_a" != "$objs_b" ]; then
      echo "$g : objets différents"
      diff <(printf '%s\n' "$objs_a") <(printf '%s\n' "$objs_b") | sed -n '2,6p'
    fi
    # L'index par ce qu'il dit, sans le réécrire (`--no-optional-locks`).
    local ia ib
    ia="$(cd "${a%/.git}" && git --no-optional-locks ls-files --stage 2>&1; git --no-optional-locks status --porcelain 2>&1)"
    ib="$(cd "${b%/.git}" && git --no-optional-locks ls-files --stage 2>&1; git --no-optional-locks status --porcelain 2>&1)"
    if [ "$ia" != "$ib" ]; then
      echo "$g/index diffère (ls-files --stage, status)"
      diff <(printf '%s\n' "$ia") <(printf '%s\n' "$ib") | sed -n '2,6p'
    fi
    # Les reflogs, horodatage masqué : `<nom> <<courriel>> <epoch> <fuseau><TAB>`.
    if [ "$( (cd "$a" && find logs -type f 2>/dev/null) | LC_ALL=C sort)" != "$( (cd "$b" && find logs -type f 2>/dev/null) | LC_ALL=C sort)" ]; then
      echo "$g/logs : fichiers différents"
    fi
    while IFS= read -r f; do
      [ -n "$f" ] && [ -f "$b/$f" ] || continue
      if ! diff <(sed -E 's/> [0-9]+ [+-][0-9]{4}	/> <time>	/' "$a/$f") <(sed -E 's/> [0-9]+ [+-][0-9]{4}	/> <time>	/' "$b/$f") >/dev/null; then
        echo "$g/$f diffère (horodatage masqué)"
        diff <(sed -E 's/> [0-9]+ [+-][0-9]{4}	/> <time>	/' "$a/$f") <(sed -E 's/> [0-9]+ [+-][0-9]{4}	/> <time>	/' "$b/$f") | head -4
      fi
    done < <(cd "$a" && find logs -type f 2>/dev/null | LC_ALL=C sort)
    # Les modes, `objects/` excepté.
    if ! diff <(git_dir_modes "$a") <(git_dir_modes "$b") >/dev/null; then
      echo "$g : modes différents"; diff <(git_dir_modes "$a") <(git_dir_modes "$b") | sed -n '2,6p'
    fi
  done <<< "$list_ref"
}

# git_dir_modes <.git> : mode et chemin de chaque entrée hors objects/, trié.
git_dir_modes() {
  (cd "$1" && if [ "$(uname -s)" = Darwin ]; then
    find . -path ./objects -prune -o -print0 | xargs -0 stat -f '%Sp %N'
  else
    find . -path ./objects -prune -o -printf '%M %p\n'
  fi) | LC_ALL=C sort
}

# vendor_inventory <dir> : mode et, pour un lien, sa cible — un chemin par
# ligne, trié bytewise ; `.git` (le dépôt du harness) exclu, le stub
# autoload_runtime.php toléré comme dans le diff. Un seul `stat`/`find`
# pour tout l'arbre (un processus par fichier prenait des minutes).
vendor_inventory() {
  local dir="$1" rel="${VENDOR_REL:-vendor}" tolerated
  # Le stub est toléré à la racine du vendor seulement : `./vendor/…` quand le
  # périmètre est le projet, `./…` quand c'est déjà le vendor.
  if [ -d "$dir/$rel" ]; then tolerated="./$rel/autoload_runtime.php"; else tolerated="./autoload_runtime.php"; fi
  # Un nom contenant un saut de ligne casserait l'hypothèse « une ligne par
  # fichier » des deux côtés à la fois, donc sans faire échouer la comparaison :
  # il est refusé bruyamment plutôt que comparé de travers.
  local files lines
  files=$(cd "$dir" && find . -name .git -prune -o -print0 | tr -dc '\0' | wc -c | tr -d ' ')
  lines=$( (cd "$dir" && if [ "$(uname -s)" = Darwin ]; then
      find . -name .git -prune -o -print0 | xargs -0 stat -f '%Sp %N -> %Y'
    else
      find . -name .git -prune -o -printf '%M %p -> %l\n'
    fi) | wc -l | tr -d ' ')
  if [ "$files" != "$lines" ]; then
    echo "INVENTAIRE INUTILISABLE dans $dir : $files fichiers pour $lines lignes (un nom contient un saut de ligne ?)"
    return 0
  fi
  (cd "$dir" && if [ "$(uname -s)" = Darwin ]; then
    find . -name .git -prune -o -print0 | xargs -0 stat -f '%Sp %N -> %Y' | sed -E 's/ -> $//'
  else
    find . -name .git -prune -o -printf '%M %p -> %l\n' | sed -E 's/ -> $//'
  fi) | awk -v t=" $tolerated" '
      # Suffixe EXACT : `…/autoload_runtime.php.bak` ou un paquet qui livre
      # `src/autoload_runtime.php` ne sont pas la tolérance.
      substr($0, length($0) - length(t) + 1) == t { next }
      { print }
    ' | LC_ALL=C sort
}
