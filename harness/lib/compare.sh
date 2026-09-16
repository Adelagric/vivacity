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
#     chez Composer (`bin2hex(random_bytes(10))`) ; la ligne est ignorée.
# Les modes des fichiers et les cibles des liens sont comparés par un
# inventaire `stat` (`diff -r` ne voit ni les uns ni les autres) : une
# extraction par `unzip` (Composer) préserve les modes du zip, vivacity aussi.

# compare_vendor <scope de Composer> <scope de vivacity> [fichier de diff]
# Renvoie 0 quand identiques ; sinon 1 et imprime les premières lignes.
compare_vendor() {
  local ref="$1" viv="$2" out="${3:-$(mktemp)}"
  local ip_ref="$ref/vendor/composer/include_paths.php" ip_viv="$viv/vendor/composer/include_paths.php"
  [ -d "$ref/vendor" ] || { ip_ref="$ref/composer/include_paths.php"; ip_viv="$viv/composer/include_paths.php"; }
  if [ -f "$ip_ref" ] && [ -f "$ip_viv" ] && ! diff -q <(sort "$ip_ref") <(sort "$ip_viv") >/dev/null; then
    echo "include_paths.php diffère même trié" > "$out"; diff <(sort "$ip_ref") <(sort "$ip_viv") | head >> "$out"
    head -20 "$out"; return 1
  fi
  # `setApcuPrefix('…')` : Composer tire un préfixe aléatoire (bin2hex de
  # 10 octets) quand `apcu-autoloader` est actif sans préfixe donné — les
  # deux côtés en ont un, jamais le même.
  diff -r --no-dereference -I 'setApcuPrefix' --exclude=.git --exclude=include_paths.php "$ref" "$viv" 2>&1 \
    | grep -v 'vendor/autoload_runtime.php\|vendor: autoload_runtime.php' \
    | grep -v 'No such file or directory' > "$out" || true   # grep -v renvoie 1 sur diff vide : succès
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
  fi) | grep -v ' \./autoload_runtime\.php$' | LC_ALL=C sort
}
