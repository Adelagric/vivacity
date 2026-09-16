# La comparaison de vendor/ (ou du projet) entre la copie de Composer et
# celle de vivacity, partagée par harness/diff-vendor.sh et harness/corpus.sh.
#
# Écarts tolérés (documentés) :
#   - vendor/autoload_runtime.php : généré par vivacity (émulation
#     symfony/runtime), absent d'un install Composer sans plugins ;
#   - « No such file or directory » : `diff -r` ne sait pas suivre un
#     symlink pendant (identique des deux côtés, vérifié par l'inventaire) ;
#   - vendor/composer/include_paths.php (paquets PEAR à `include-path`) :
#     Composer l'écrit dans l'ordre d'achèvement des extractions asynchrones
#     (non déterministe, vérifié le 2026-09-11) ; comparé trié ;
#   - `$loader->setApcuPrefix('…')` dans autoload_real.php : préfixe aléatoire
#     chez Composer (`bin2hex(random_bytes(10))`) ; la ligne est ignorée ;
#   - le chemin absolu du projet dans un fichier généré : remplacé par
#     `<project>` de chaque côté avant comparaison ;
#   - vendor/pest-plugins.json : le plugin lit le dépôt local dans l'ordre
#     d'achèvement des installations asynchrones (constaté : le petit
#     pest-plugin-laravel finit avant pest) ; comparé trié.
# Les modes des fichiers et les cibles des liens sont comparés par un
# inventaire `stat` (`diff -r` ne voit ni les uns ni les autres) : une
# extraction par `unzip` (Composer) préserve les modes du zip, vivacity aussi.

# compare_vendor <scope de Composer> <scope de vivacity> [fichier de diff]
# Renvoie 0 quand identiques ; sinon 1 et imprime les premières lignes.
# Un fichier généré peut contenir le chemin absolu du projet (phpstan/
# extension-installer écrit `install_path` en absolu) : les deux copies
# vivent dans des répertoires différents, le contenu est comparé après
# remplacement du chemin physique de chaque côté par `<project>`.
compare_vendor() {
  local ref="$1" viv="$2" out="${3:-$(mktemp)}"
  local ip_ref="$ref/vendor/composer/include_paths.php" ip_viv="$viv/vendor/composer/include_paths.php"
  [ -d "$ref/vendor" ] || { ip_ref="$ref/composer/include_paths.php"; ip_viv="$viv/composer/include_paths.php"; }
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
  local ref_real viv_real ref_win="" viv_win="" ref_winf="" viv_winf=""
  ref_real="$(cd "$ref" && pwd -P)"; viv_real="$(cd "$viv" && pwd -P)"
  # Windows (Git Bash) : Composer écrit le chemin sous sa forme `D:\a\…`,
  # ou mixte `D:\a\…/vendor/x` ; les deux formes sont normalisées aussi.
  if command -v cygpath >/dev/null 2>&1; then
    ref_win="$(cygpath -w "$ref_real" | sed 's/\\/\\\\/g')"; viv_win="$(cygpath -w "$viv_real" | sed 's/\\/\\\\/g')"
    ref_winf="$(cygpath -m "$ref_real")"; viv_winf="$(cygpath -m "$viv_real")"
  fi
  norm_ref() { sed -e "s|$ref_real|<project>|g" -e "s|$ref|<project>|g" ${ref_win:+-e "s|$ref_win|<project>|g"} ${ref_winf:+-e "s|$ref_winf|<project>|g"} "$1"; }
  norm_viv() { sed -e "s|$viv_real|<project>|g" -e "s|$viv|<project>|g" ${viv_win:+-e "s|$viv_win|<project>|g"} ${viv_winf:+-e "s|$viv_winf|<project>|g"} "$1"; }
  : > "$out"
  # `diff -rq` : les fichiers présents d'un seul côté, et les paires qui
  # diffèrent — celles-ci sont recomparées normalisées.
  diff -rq --no-dereference --exclude=.git --exclude=include_paths.php --exclude=pest-plugins.json "$ref" "$viv" 2>&1 \
    | grep -v 'autoload_runtime.php' | grep -v 'No such file or directory' \
    | while IFS= read -r line; do
      case "$line" in
        "Files "*" and "*" differ")
          a="${line#Files }"; a="${a% and *}"; b="${line#* and }"; b="${b% differ}"
          if ! diff -I 'setApcuPrefix' <(norm_ref "$a") <(norm_viv "$b") >/dev/null 2>&1; then
            echo "$line"
            diff <(norm_ref "$a") <(norm_viv "$b") | head -6
          fi ;;
        *) echo "$line" ;;
      esac
    done >> "$out"
  diff <(vendor_inventory "$ref") <(vendor_inventory "$viv") >> "$out" || true
  if [ -s "$out" ]; then head -20 "$out"; return 1; fi
  return 0
}

# vendor_inventory <dir> : mode et, pour un lien, sa cible — un chemin par
# ligne, trié bytewise ; `.git` (le dépôt du harness) exclu, le stub
# autoload_runtime.php toléré comme dans le diff. Un seul `stat`/`find`
# pour tout l'arbre (un processus par fichier prenait des minutes).
vendor_inventory() {
  (cd "$1" && if [ "$(uname -s)" = Darwin ]; then
    find . -name .git -prune -o -print0 | xargs -0 stat -f '%Sp %N -> %Y' | sed -E 's/ -> $//'
  else
    find . -name .git -prune -o -printf '%M %p -> %l\n' | sed -E 's/ -> $//'
  fi) | grep -v 'autoload_runtime\.php$' | LC_ALL=C sort
}
