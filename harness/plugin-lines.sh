#!/usr/bin/env bash
# Harness des lignes que les installeurs de plugins écrivent à
# `POST_INSTALL_CMD` / `POST_UPDATE_CMD` (et à `POST_AUTOLOAD_DUMP` pour
# composer/package-versions-deprecated), et de leur ordre
# (docs/plans/plugin-lines.md). Projets résolus sur Packagist en ligne,
# versions exactes (le lock est écrit une fois par Composer sous
# `--no-plugins`, puis copié des deux côtés) ; Composer (plugins actifs :
# `--no-scripts` ne coupe que les scripts de composer.json) contre
# vivacity (émulations). À chaque étape : code de sortie, projet entier
# (harness/lib/compare.sh) et sortie standard + sortie d'erreur
# fusionnées, dans l'ordre (le résumé `vivacity: …` et les `  - Downloading`
# hors comparaison, comme yii2-composer.sh), plus une assertion sur la
# sortie de Composer — un cas qui ne montre plus ce qu'il teste échoue au
# lieu de passer.
#
# - phpstan/extension-installer 1.4.3 seul (`Plugin::process`, l. 198-214) :
#   « Extensions installed » à chaque install, les lignes `> nom: installed`,
#   `not supported`, `ignored` quand GeneratedConfig.php change ;
# - rector/extension-installer 0.11.2 seul (`PluginInstaller::install`,
#   l. 95-104) : rien quand le fichier ne change pas ;
# - dealerdirect/phpcodesniffer-composer-installer v1.2.1 seul
#   (`Plugin::saveInstalledPaths`, l. 263-315) : « set to », « delete »,
#   rien sans changement de configuration ;
#   phpstan avec `require` puis `remove` de codeception/c3 ;
# - les trois avec codeception/c3 : l'ordre d'activation des plugins
#   (`PluginManager::loadRepository` → `PackageSorter::sortPackages` sur
#   installed.json, puis ceux que la commande installe ou met à jour, à ce
#   moment-là ; les poids vus quand dealerdirect, c3 et phpstan écrivent
#   ensemble), -q, -v (rendu à Composer), --no-autoloader, --no-dev,
#   --no-plugins, `update` ;
# - dealerdirect v1.0.0 : un chemin enregistré relatif au projet gardé ;
# - les versions dont le code n'a pas été lu (dealerdirect v0.7.1, phpstan
#   1.3.1, rector 0.10.2, package-versions 1.11.99.1), une extension aliasée
#   et drupal/core-project-message avec un message : rendus à Composer (code
#   3 sous --no-fallback, la raison, rien d'écrit) ;
# - `remove` d'un plugin encore requis : toujours chargé, il écrit ;
# - composer/package-versions-deprecated (`Installer::dumpVersionsClass`) :
#   deux lignes à chaque dump, dump-autoload compris ; avec bamarni, l'ordre
#   des deux écouteurs de `POST_AUTOLOAD_DUMP`.
# - `update`/`require`/`remove --no-install` (docs/plans/no-install-listeners.md) :
#   `POST_UPDATE_CMD` part sans installation, vers les plugins chargés
#   d'installed.json, qui lisent le dépôt local (l'ancien état) — phpstan,
#   dealerdirect, c3, rector, les quatre ensemble (ordre), Flex à son rang ;
#   vendor vierge, plugin ajouté ou retiré par l'update, -q, --no-dev,
#   --no-plugins, --dry-run, `config.lock: false`, vendor-dir, ensemble
#   insoluble, yii2-composer (`activate`) ; replis nommés : versions non
#   lues, plugin global, php-http/discovery qui surveille une abstraction,
#   composer-patches.
#
# Usage : harness/plugin-lines.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/fixture.sh
. "$ROOT/harness/lib/fixture.sh"
# shellcheck source=lib/compare.sh
. "$ROOT/harness/lib/compare.sh"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) if [ -x "${VIVACITY}.exe" ]; then VIVACITY="${VIVACITY}.exe"; fi ;;
esac
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/plugin-lines"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
command -v php >/dev/null || { echo "php requis"; exit 1; }
rm -rf "$WORK"; mkdir -p "$WORK"; harness_git_env "$WORK"
# Isolé de la configuration globale de la machine (plugins globaux,
# `allow-plugins` global) : un COMPOSER_HOME vide pour les deux outils, qui
# garde le cache de fichiers habituel (téléchargements partagés) et les
# jetons d'auth.json (limite de l'API GitHub sur cache froid).
CACHE_DIR="${COMPOSER_CACHE_DIR:-$(composer config --global cache-dir 2>/dev/null)}"
HOME_DIR="$(composer config --global home 2>/dev/null || true)"
mkdir -p "$WORK/home"
[ -n "$HOME_DIR" ] && [ -f "$HOME_DIR/auth.json" ] && cp "$HOME_DIR/auth.json" "$WORK/home/"
export COMPOSER_HOME="$WORK/home" COMPOSER_CACHE_DIR="$CACHE_DIR"
status=0

# lock <nom> <composer.json> : le lock écrit par Composer (sans plugins,
# aucun ne touche à la résolution ici), gardé sous $WORK/locks/<nom>.
lock() {
  local d="$WORK/locks/$1"
  rm -rf "$d"; mkdir -p "$d"; printf '%s\n' "$2" > "$d/composer.json"
  (cd "$d" && composer update --no-install --no-plugins --no-scripts --no-audit --no-security-blocking --no-interaction --no-ansi -q) \
    || { echo "FAIL lock $1 : Composer n'a pas résolu le projet"; exit 1; }
}
# stage <nom du lock> <répertoires…> : composer.json et composer.lock copiés
# (vendor/ et le reste gardés : une étape enchaîne sur la précédente).
stage() { local src="$WORK/locks/$1"; shift; for d in "$@"; do mkdir -p "$d"; cp "$src/composer.json" "$src/composer.lock" "$d/"; done; }
fresh() { local src="$1"; shift; for d in "$@"; do rm -rf "$d"; done; stage "$src" "$@"; }

step() { # ref, viv, nom, args composer..., "--", args vivacity...
  local ref="$1" viv="$2" name="$3"; shift 3
  local c=() v=(); while [ "$1" != "--" ]; do c+=("$1"); shift; done; shift; v=("$@")
  local c_code=0 v_code=0
  (cd "$ref" && composer "${c[@]}" --no-interaction --no-ansi >"$WORK/$name.composer.all" 2>&1) || c_code=$?
  (cd "$viv" && "$VIVACITY" "${v[@]}" >"$WORK/$name.vivacity.all" 2>&1) || v_code=$?
  if [ "$c_code" != "$v_code" ]; then echo "FAIL $name : codes $c_code vs $v_code"; tail -3 "$WORK/$name.vivacity.all"; status=1; return; fi
  # `Note: plugin … installed as a plain library` : la note de vivacity sur
  # un plugin bénin (drupal/core-project-message), tolérée comme dans
  # flex-install.sh.
  # `Warning: Accessing 127.0.0.1 over http…` (l'index de Flex servi en
  # http local, `ni-flex`) est comparé : Flex le lit à `POST_UPDATE_CMD`.
  if ! diff <(grep -vE '^  - Downloading ' "$WORK/$name.composer.all") <(grep -v '^vivacity: \|^Note: plugin ' "$WORK/$name.vivacity.all") >"$WORK/$name.order.diff"; then
    echo "FAIL $name : sortie fusionnée différente"; head -12 "$WORK/$name.order.diff"; status=1; return
  fi
  if compare_vendor "$ref" "$viv" "$WORK/$name.diff" >/dev/null; then
    echo "OK   $name : projet et sortie fusionnée identiques ($(grep -cE '^(> |phpstan/|rector/|PHP CodeSniffer|\[codeception/c3\]|composer/package-versions)' "$WORK/$name.composer.all" || true) ligne(s) de plugins)"
  else
    echo "FAIL $name : projet différent"; head -12 "$WORK/$name.diff"; status=1
  fi
}
# expect <nom> <motif grep -E> : la sortie de Composer contient le motif
# (sinon le cas est mal construit) ; `expect_not` : ne le contient pas.
expect() { grep -qE -- "$2" "$WORK/$1.composer.all" || { echo "FAIL $1 : « $2 » absent chez Composer (cas mal construit)"; status=1; }; }
expect_not() { if grep -qE -- "$2" "$WORK/$1.composer.all"; then echo "FAIL $1 : « $2 » présent chez Composer (cas mal construit)"; status=1; fi; }
# order <nom> <motif>… : les motifs apparaissent dans cet ordre chez Composer
# (chacun cherché après la ligne du précédent).
order() {
  local name="$1"; shift; local prev=0 n p
  for p in "$@"; do
    n=$(grep -nE -- "$p" "$WORK/$name.composer.all" | awk -F: -v p="$prev" '$1 > p { print $1; exit }')
    if [ -z "$n" ]; then echo "FAIL $name : « $p » pas à sa place chez Composer (cas mal construit)"; status=1; return; fi
    prev=$n
  done
}
I=(install --no-scripts)
V=(install --no-scripts --no-fallback)
R=(--no-scripts --no-audit --no-security-blocking)
PHPSTAN='^phpstan/extension-installer: Extensions installed$'
RECTOR='^rector/rector-installer: Extensions installed$'
PHPCS='^PHP CodeSniffer Config installed_paths '
C3='^\[codeception/c3\] '

# --- phpstan/extension-installer seul -------------------------------------
lock phpstan '{"name": "vivacity/plugin-lines-phpstan",
  "require": {"phpstan/extension-installer": "1.4.3", "phpstan/phpstan": "2.3.1"},
  "require-dev": {"phpstan/phpstan-strict-rules": "2.0.4", "phpstan/phpstan-deprecation-rules": "2.0.5", "phpstan/php-8-stubs": "0.4.43"},
  "extra": {"phpstan/extension-installer": {"ignore": ["phpstan/phpstan-deprecation-rules"]}},
  "config": {"allow-plugins": {"phpstan/extension-installer": true}}}'
ref="$WORK/phpstan-ref"; viv="$WORK/phpstan-viv"; fresh phpstan "$ref" "$viv"
step "$ref" "$viv" phpstan-install "${I[@]}" -- "${V[@]}"
expect phpstan-install '^> phpstan/phpstan-strict-rules: installed$'
order phpstan-install 'looking for funding|^Generating autoload files$' "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$' '^> phpstan/php-8-stubs: not supported$' '^> phpstan/phpstan-deprecation-rules: ignored$'
step "$ref" "$viv" phpstan-no-op "${I[@]}" -- "${V[@]}"
expect phpstan-no-op "$PHPSTAN"; expect_not phpstan-no-op '^> '
step "$ref" "$viv" phpstan-quiet "${I[@]}" -q -- "${V[@]}" -q
expect_not phpstan-quiet '.'
step "$ref" "$viv" phpstan-no-dev "${I[@]}" --no-dev -- "${V[@]}" --no-dev
expect phpstan-no-dev "$PHPSTAN"; expect_not phpstan-no-dev '^> '
step "$ref" "$viv" phpstan-re-dev "${I[@]}" -- "${V[@]}"
expect phpstan-re-dev '^> phpstan/phpstan-strict-rules: installed$'
# `require` d'un plugin : activé à son installation, donc après phpstan
# (chargé au démarrage) — l'ordre des noms dirait l'inverse ; `remove` :
# désinstallé (« Deleting c3.php »), plus d'écouteur — avec la règle
# `codeception/*: false` APRÈS celle de c3 : `remove` retire l'entrée de c3
# une fois les plugins chargés, la règle lue la première reste celle de c3.
for d in "$ref" "$viv"; do jq '.config["allow-plugins"]["codeception/c3"] = true | .config["allow-plugins"]["codeception/*"] = false' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json"; done
step "$ref" "$viv" phpstan-require require codeception/c3:2.9.0 "${R[@]}" -- require codeception/c3:2.9.0 "${R[@]}" --no-fallback
order phpstan-require 'looking for funding' "$PHPSTAN" "$C3"'Copying'
step "$ref" "$viv" phpstan-remove remove codeception/c3 "${R[@]}" -- remove codeception/c3 "${R[@]}" --no-fallback
order phpstan-remove "$C3"'Deleting' 'looking for funding' "$PHPSTAN"; expect_not phpstan-remove "$C3"'(Copying|c3.php is)'
# `allow-plugins` à false (hors content-hash) : le plugin n'est pas chargé.
for d in "$ref" "$viv"; do jq '.config["allow-plugins"]["phpstan/extension-installer"] = false' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json"; done
step "$ref" "$viv" phpstan-not-allowed "${I[@]}" -- "${V[@]}"
expect_not phpstan-not-allowed '^phpstan/'

# --- rector/extension-installer seul --------------------------------------
lock rector '{"name": "vivacity/plugin-lines-rector",
  "require": {"rector/extension-installer": "0.11.2"},
  "require-dev": {"rector/rector-symfony": "1.1.0"},
  "config": {"allow-plugins": {"rector/extension-installer": true}}}'
ref="$WORK/rector-ref"; viv="$WORK/rector-viv"; fresh rector "$ref" "$viv"
step "$ref" "$viv" rector-install "${I[@]}" -- "${V[@]}"
order rector-install "$RECTOR" '^> rector/rector-symfony: installed$'
step "$ref" "$viv" rector-no-op "${I[@]}" -- "${V[@]}"
expect_not rector-no-op '^rector/|^> '
step "$ref" "$viv" rector-no-dev "${I[@]}" --no-dev -- "${V[@]}" --no-dev
expect rector-no-dev "$RECTOR"; expect_not rector-no-dev '^> '
step "$ref" "$viv" rector-re-dev "${I[@]}" -- "${V[@]}"
expect rector-re-dev '^> rector/rector-symfony: installed$'

# --- dealerdirect/phpcodesniffer-composer-installer seul -------------------
lock phpcs '{"name": "vivacity/plugin-lines-phpcs",
  "require": {"dealerdirect/phpcodesniffer-composer-installer": "v1.2.1", "squizlabs/php_codesniffer": "4.0.4"},
  "require-dev": {"phpcsstandards/phpcsextra": "1.5.1", "phpcsstandards/phpcsutils": "1.2.3"},
  "config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true}}}'
ref="$WORK/phpcs-ref"; viv="$WORK/phpcs-viv"; fresh phpcs "$ref" "$viv"
step "$ref" "$viv" phpcs-install "${I[@]}" -- "${V[@]}"
expect phpcs-install '^PHP CodeSniffer Config installed_paths set to \.\./\.\./phpcsstandards/phpcsextra,\.\./\.\./phpcsstandards/phpcsutils$'
step "$ref" "$viv" phpcs-no-op "${I[@]}" -- "${V[@]}"
expect_not phpcs-no-op "$PHPCS"
# v1.2.1 : un chemin enregistré relatif au projet (pas à phpcs) est retiré.
for d in "$ref" "$viv"; do
  printf "<?php\n \$phpCodeSnifferConfig = array (\n  'installed_paths' => '../../phpcsstandards/phpcsextra,../../phpcsstandards/phpcsutils,vendor/phpcsstandards/phpcsutils',\n);\n?>" > "$d/vendor/squizlabs/php_codesniffer/CodeSniffer.conf"
done
step "$ref" "$viv" phpcs-cwd-relative-dropped "${I[@]}" -- "${V[@]}"
expect phpcs-cwd-relative-dropped "$PHPCS"'set to \.\./\.\./phpcsstandards/phpcsextra,\.\./\.\./phpcsstandards/phpcsutils$'
# La configuration de phpcs perdue : le plugin la réécrit, et le dit.
rm -f "$ref/vendor/squizlabs/php_codesniffer/CodeSniffer.conf" "$viv/vendor/squizlabs/php_codesniffer/CodeSniffer.conf"
step "$ref" "$viv" phpcs-conf-lost "${I[@]}" -- "${V[@]}"
expect phpcs-conf-lost "$PHPCS"'set to '
step "$ref" "$viv" phpcs-no-dev "${I[@]}" --no-dev -- "${V[@]}" --no-dev
expect phpcs-no-dev '^PHP CodeSniffer Config installed_paths delete$'
step "$ref" "$viv" phpcs-re-dev "${I[@]}" -- "${V[@]}"
expect phpcs-re-dev "$PHPCS"'set to '

# --- les trois et codeception/c3 : l'ordre d'activation --------------------
ALL='"require": {"phpstan/extension-installer": "1.4.3", "phpstan/phpstan": "2.3.1",
    "dealerdirect/phpcodesniffer-composer-installer": "v1.2.1", "squizlabs/php_codesniffer": "4.0.4",
    "rector/extension-installer": "0.11.2"},
  "config": {"allow-plugins": {"phpstan/extension-installer": true, "rector/extension-installer": true,
    "dealerdirect/phpcodesniffer-composer-installer": true, "codeception/c3": true}}'
DEV='"phpstan/phpstan-strict-rules": "2.0.4", "phpcsstandards/phpcsextra": "1.5.1", "phpcsstandards/phpcsutils": "1.2.3",
    "slevomat/coding-standard": "8.31.1", "phpstan/phpdoc-parser": "2.3.6", "rector/rector-symfony": "1.1.0"'
lock all "{\"name\": \"vivacity/plugin-lines-all\", $ALL, \"require-dev\": {\"codeception/c3\": \"2.9.0\", $DEV}}"
lock all-c3-old "{\"name\": \"vivacity/plugin-lines-all\", $ALL, \"require-dev\": {\"codeception/c3\": \"2.8.1\", $DEV}}"
ref="$WORK/all-ref"; viv="$WORK/all-viv"; fresh all "$ref" "$viv"
# Vendor vierge : chaque plugin est activé quand la transaction l'installe.
step "$ref" "$viv" all-install "${I[@]}" -- "${V[@]}"
order all-install 'looking for funding' "$C3" "$RECTOR" "$PHPCS" "$PHPSTAN"
# Tout est dans installed.json : l'ordre de `PackageSorter::sortPackages`
# (dealerdirect, requis par trois paquets, d'abord ; puis l'ordre des noms).
step "$ref" "$viv" all-no-op "${I[@]}" -- "${V[@]}"
order all-no-op 'looking for funding' "$C3" "$PHPSTAN"
# Les poids de `PackageSorter` vus : c3.php et la configuration de phpcs
# perdus, trois plugins écrivent — dealerdirect (requis par trois paquets)
# avant c3, avant phpstan.
rm -f "$ref/c3.php" "$viv/c3.php" "$ref/vendor/squizlabs/php_codesniffer/CodeSniffer.conf" "$viv/vendor/squizlabs/php_codesniffer/CodeSniffer.conf"
step "$ref" "$viv" all-weights "${I[@]}" -- "${V[@]}"
order all-weights 'looking for funding' "$PHPCS"'set to' "$C3"'Copying' "$PHPSTAN"
# --no-autoloader : `POST_INSTALL_CMD` sans dump.
step "$ref" "$viv" all-no-autoloader "${I[@]}" --no-autoloader -- "${V[@]}" --no-autoloader
order all-no-autoloader 'looking for funding' "$C3" "$PHPSTAN"; expect_not all-no-autoloader '^Generating autoload files$'
step "$ref" "$viv" all-quiet "${I[@]}" -q -- "${V[@]}" -q
expect_not all-quiet '.'
# -v : Composer imprime une ligne par écouteur (`> post-install-cmd: …`),
# vivacity rend la commande (code 3 sous --no-fallback, rien d'écrit).
c_code=0; v_code=0
touch "$WORK/all-verbose.mark"; sleep 1
(cd "$ref" && composer "${I[@]}" -v --no-interaction --no-ansi >"$WORK/all-verbose.composer.all" 2>&1) || c_code=$?
(cd "$viv" && "$VIVACITY" "${V[@]}" -v >"$WORK/all-verbose.vivacity.all" 2>&1) || v_code=$?
if [ "$c_code" = 0 ] && [ "$v_code" = 3 ] && grep -q 'while .* is active (its listeners' "$WORK/all-verbose.vivacity.all" \
   && grep -q '^> post-install-cmd: PHPStan' "$WORK/all-verbose.composer.all" \
   && [ -z "$(find "$viv" -newer "$WORK/all-verbose.mark" -print -quit)" ]; then
  echo "OK   all-verbose : -v rendu à Composer (une ligne par écouteur chez lui)"
else
  echo "FAIL all-verbose : codes $c_code vs $v_code"; tail -3 "$WORK/all-verbose.vivacity.all"; status=1
fi
# --no-dev : c3 part (désinstallé, plus d'écouteur), les extensions et les
# standards aussi ; dealerdirect, phpstan, rector restent et écrivent.
step "$ref" "$viv" all-no-dev "${I[@]}" --no-dev -- "${V[@]}" --no-dev
order all-no-dev "$C3"'Deleting' 'looking for funding' "$PHPCS"'delete' "$PHPSTAN" "$RECTOR"
# Retour en dev : les trois déjà chargés (ordre des noms), c3 activé à son
# installation, donc après eux.
step "$ref" "$viv" all-re-dev "${I[@]}" -- "${V[@]}"
order all-re-dev 'looking for funding' "$PHPCS" "$PHPSTAN" "$RECTOR" "$C3"'Copying'
# Une mise à jour de plugin le désactive puis le réactive
# (`PluginInstaller::update`) : c3 passe après les autres.
ref="$WORK/all-upd-ref"; viv="$WORK/all-upd-viv"; fresh all-c3-old "$ref" "$viv"
step "$ref" "$viv" all-c3-old "${I[@]}" -- "${V[@]}"
order all-c3-old 'looking for funding' "$C3"'Copying' "$RECTOR" "$PHPCS" "$PHPSTAN"
stage all "$ref" "$viv"; rm -f "$ref/c3.php" "$viv/c3.php"
step "$ref" "$viv" all-c3-update "${I[@]}" -- "${V[@]}"
expect all-c3-update 'Upgrading codeception/c3'
order all-c3-update 'looking for funding' "$PHPSTAN" "$C3"'Copying'
# --no-plugins : aucune ligne.
ref="$WORK/all-noplug-ref"; viv="$WORK/all-noplug-viv"; fresh all "$ref" "$viv"
step "$ref" "$viv" all-no-plugins "${I[@]}" --no-plugins -- "${V[@]}" --no-plugins
expect_not all-no-plugins '^(phpstan|rector)/|^PHP CodeSniffer|^\[codeception'
# `update` avec installation (vendor vierge), puis `update` sans changement
# (installed.json complet : ordre des noms).
ref="$WORK/all-update-ref"; viv="$WORK/all-update-viv"; fresh all "$ref" "$viv"
step "$ref" "$viv" all-update update "${R[@]}" -- update "${R[@]}" --no-fallback
order all-update 'looking for funding' "$C3"'Copying' "$RECTOR" "$PHPCS" "$PHPSTAN"
# Sans changement : c3 (`askForUpdateV2`) ne dit rien quand c3.php est à
# jour ; c3.php et la configuration de phpcs perdus, les trois écrivent dans
# l'ordre de `PackageSorter`.
rm -f "$ref/c3.php" "$viv/c3.php" "$ref/vendor/squizlabs/php_codesniffer/CodeSniffer.conf" "$viv/vendor/squizlabs/php_codesniffer/CodeSniffer.conf"
step "$ref" "$viv" all-update-again update "${R[@]}" -- update "${R[@]}" --no-fallback
order all-update-again 'Nothing to modify in lock file' 'looking for funding' "$PHPCS"'set to' "$C3"'Copying' "$PHPSTAN"

# --- dealerdirect v1.0.0 : un chemin relatif au projet reste enregistré ----
# (`cleanInstalledPaths` et `$alternativePath` jusqu'à v1.2.0 ; v1.2.1 ne
# lit plus que relativement à phpcs).
lock phpcs-v100 '{"name": "vivacity/plugin-lines-phpcs-v100",
  "require": {"dealerdirect/phpcodesniffer-composer-installer": "v1.0.0", "squizlabs/php_codesniffer": "3.13.6",
    "phpcsstandards/phpcsutils": "1.2.3"},
  "config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true}}}'
ref="$WORK/phpcs100-ref"; viv="$WORK/phpcs100-viv"; fresh phpcs-v100 "$ref" "$viv"
step "$ref" "$viv" phpcs-v100-install "${I[@]}" -- "${V[@]}"
expect phpcs-v100-install "$PHPCS"'set to \.\./\.\./phpcsstandards/phpcsutils$'
for d in "$ref" "$viv"; do
  printf "<?php\n \$phpCodeSnifferConfig = array (\n  'installed_paths' => '../../phpcsstandards/phpcsutils,gone/std,vendor/phpcsstandards/phpcsutils',\n);\n?>" > "$d/vendor/squizlabs/php_codesniffer/CodeSniffer.conf"
done
step "$ref" "$viv" phpcs-v100-cwd-relative "${I[@]}" -- "${V[@]}"
expect phpcs-v100-cwd-relative "$PHPCS"'set to \.\./\.\./phpcsstandards/phpcsutils,vendor/phpcsstandards/phpcsutils$'

# --- les versions non lues : rendues à Composer, nommées -------------------
lock old-versions '{"name": "vivacity/plugin-lines-old-versions",
  "require": {"dealerdirect/phpcodesniffer-composer-installer": "v0.7.1", "squizlabs/php_codesniffer": "3.13.6",
    "phpstan/extension-installer": "1.3.1", "phpstan/phpstan": "1.12.33", "rector/extension-installer": "0.10.2",
    "composer/package-versions-deprecated": "1.11.99.1"},
  "config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true,
    "phpstan/extension-installer": true, "rector/extension-installer": true,
    "composer/package-versions-deprecated": true}}}'
viv="$WORK/old-versions-viv"; fresh old-versions "$viv"
v_code=0; (cd "$viv" && "$VIVACITY" "${V[@]}" >"$WORK/old-versions.vivacity.all" 2>&1) || v_code=$?
n=$(grep -cE '^  - plugin (dealerdirect/phpcodesniffer-composer-installer v0\.7\.1|phpstan/extension-installer 1\.3\.1|rector/extension-installer 0\.10\.2|composer/package-versions-deprecated 1\.11\.99\.1) is not emulated in this version' "$WORK/old-versions.vivacity.all" || true)
if [ "$v_code" = 3 ] && [ "$n" = 4 ] && [ ! -e "$viv/vendor" ]; then
  echo "OK   old-versions : quatre versions non lues rendues à Composer, nommées, rien d'écrit"
else
  echo "FAIL old-versions : code $v_code, $n raison(s)"; tail -6 "$WORK/old-versions.vivacity.all"; status=1
fi

# Une extension aliasée en ligne : la version écrite dépend de l'ordre du
# dépôt local de Composer (le paquet sur vendor vierge, l'alias ensuite).
lock aliased '{"name": "vivacity/plugin-lines-aliased",
  "require": {"phpstan/extension-installer": "1.4.3", "phpstan/phpstan": "2.3.1",
    "phpstan/phpstan-strict-rules": "2.0.4 as 2.0.99"},
  "config": {"allow-plugins": {"phpstan/extension-installer": true}}}'
viv="$WORK/aliased-viv"; fresh aliased "$viv"
v_code=0; (cd "$viv" && "$VIVACITY" "${V[@]}" >"$WORK/aliased.vivacity.all" 2>&1) || v_code=$?
if [ "$v_code" = 3 ] && grep -q '^  - package phpstan/phpstan-strict-rules 2\.0\.4 is aliased' "$WORK/aliased.vivacity.all" && [ ! -e "$viv/vendor" ]; then
  echo "OK   aliased : extension aliasée rendue à Composer, nommée, rien d'écrit"
else
  echo "FAIL aliased : code $v_code"; tail -4 "$WORK/aliased.vivacity.all"; status=1
fi

# `remove` d'un plugin qu'un autre paquet requiert encore : il reste installé
# et chargé (les règles `allow-plugins` lues à la création du `Composer` de
# la commande, avant que `remove` retire son entrée — ici la seule, donc la
# clé entière), écrit à `POST_UPDATE_CMD`, puis « Removal failed », code 2.
# Une ligne « Dependency … is also a root requirement », quand elle sort,
# vient avant « Updating dependencies » des deux côtés depuis le 2026-10-29
# (docs/plans/parity-gaps-026.md ; harness/verbosity.sh la compare).
mkdir -p "$WORK/pkg/needs-installer"
printf '{"name": "vivacity/needs-installer", "version": "1.0.0", "require": {"phpstan/extension-installer": "1.4.3"}}\n' > "$WORK/pkg/needs-installer/composer.json"
lock remove-required "{\"name\": \"vivacity/plugin-lines-remove-required\",
  \"repositories\": [{\"type\": \"path\", \"url\": \"$WORK/pkg/needs-installer\"}],
  \"require\": {\"phpstan/extension-installer\": \"1.4.3\",
    \"vivacity/needs-installer\": \"1.0.0\", \"phpstan/phpstan-strict-rules\": \"2.0.4\"},
  \"config\": {\"allow-plugins\": {\"phpstan/extension-installer\": true}}}"
ref="$WORK/remove-required-ref"; viv="$WORK/remove-required-viv"; fresh remove-required "$ref" "$viv"
step "$ref" "$viv" remove-required-install "${I[@]}" -- "${V[@]}"
step "$ref" "$viv" remove-required remove phpstan/extension-installer "${R[@]}" -- remove phpstan/extension-installer "${R[@]}" --no-fallback
order remove-required "$PHPSTAN" 'Removal failed'

# --- drupal/core-project-message : le message de post-install-cmd ----------
lock message '{"name": "vivacity/plugin-lines-message",
  "require": {"drupal/core-project-message": "11.4.6"},
  "extra": {"drupal-core-project-message": {"post-install-cmd-message": ["<info>hello</info> from post-install-cmd"]}},
  "config": {"allow-plugins": {"drupal/core-project-message": true}}}'
viv="$WORK/message-viv"; fresh message "$viv"
v_code=0; (cd "$viv" && "$VIVACITY" "${V[@]}" >"$WORK/message.vivacity.all" 2>&1) || v_code=$?
if [ "$v_code" = 3 ] && grep -q 'drupal/core-project-message would print its post-install-cmd message' "$WORK/message.vivacity.all" && [ ! -e "$viv/vendor" ]; then
  echo "OK   message : drupal/core-project-message qui écrirait, rendu à Composer"
else
  echo "FAIL message : code $v_code"; tail -4 "$WORK/message.vivacity.all"; status=1
fi
# Sans message configuré : rien chez Composer, natif chez vivacity.
lock message-none '{"name": "vivacity/plugin-lines-message-none",
  "require": {"drupal/core-project-message": "11.4.6"},
  "config": {"allow-plugins": {"drupal/core-project-message": true}}}'
ref="$WORK/message-none-ref"; viv="$WORK/message-none-viv"; fresh message-none "$ref" "$viv"
step "$ref" "$viv" message-none "${I[@]}" -- "${V[@]}"

# --- composer/package-versions-deprecated : deux lignes à chaque dump -------
SRC_PV="$ROOT/fixtures/projects/package-versions"
ref="$WORK/pv-ref"; viv="$WORK/pv-viv"
for d in "$ref" "$viv"; do rm -rf "$d"; mkdir -p "$d"; cp "$SRC_PV/composer.json" "$SRC_PV/composer.lock" "$d/"; done
step "$ref" "$viv" pv-install "${I[@]}" -- "${V[@]}"
order pv-install '^Generating autoload files$' '^composer/package-versions-deprecated: Generating version class\.\.\.$' '^composer/package-versions-deprecated: \.\.\.done generating version class$' 'looking for funding'
step "$ref" "$viv" pv-no-op "${I[@]}" -- "${V[@]}"
expect pv-no-op '^composer/package-versions-deprecated: Generating'
step "$ref" "$viv" pv-dump dump-autoload -- dump-autoload
expect pv-dump '^composer/package-versions-deprecated: Generating'
step "$ref" "$viv" pv-quiet "${I[@]}" -q -- "${V[@]}" -q
expect_not pv-quiet '.'
# Avec bamarni/composer-bin-plugin, l'autre écouteur de `POST_AUTOLOAD_DUMP`
# qui écrit : package-versions-deprecated, requis par
# jean85/pretty-package-versions, passe devant (poids de `PackageSorter`) ;
# vendor vierge : ordre de la transaction.
lock pv-bamarni '{"name": "vivacity/plugin-lines-pv-bamarni",
  "require": {"composer/package-versions-deprecated": "1.11.99.5", "jean85/pretty-package-versions": "1.6.0"},
  "require-dev": {"bamarni/composer-bin-plugin": "1.9.1"},
  "extra": {"bamarni-bin": {"bin-links": false}},
  "config": {"allow-plugins": {"composer/package-versions-deprecated": true, "bamarni/composer-bin-plugin": true}}}'
ref="$WORK/pvb-ref"; viv="$WORK/pvb-viv"; fresh pv-bamarni "$ref" "$viv"
step "$ref" "$viv" pvb-install "${I[@]}" -- "${V[@]}"
order pvb-install '^Generating autoload files$' '^\[bamarni-bin\]' '^composer/package-versions-deprecated: \.\.\.done'
step "$ref" "$viv" pvb-no-op "${I[@]}" -- "${V[@]}"
order pvb-no-op '^Generating autoload files$' '^composer/package-versions-deprecated: \.\.\.done' '^\[bamarni-bin\]'
step "$ref" "$viv" pvb-dump dump-autoload -- dump-autoload
order pvb-dump '^composer/package-versions-deprecated: \.\.\.done' '^\[bamarni-bin\]'

# --- update --no-install : les écouteurs de POST_UPDATE_CMD -------------------
# (docs/plans/no-install-listeners.md) `Installer::run` sans installation
# saute le dump mais dispatche `POST_UPDATE_CMD` (Installer.php:438-441) :
# les plugins chargés d'installed.json lisent le dépôt LOCAL — l'ancien état,
# purgé, dev compris —, jamais le lock. Chaque projet est installé par son
# outil, l'état gardé au même chemin (les chemins absolus de
# GeneratedConfig.php ne bougent pas), puis chaque cas repart de cet état.
NI=(update --no-install "${R[@]}")
GC=vendor/phpstan/extension-installer/src/GeneratedConfig.php
RGC=vendor/rector/extension-installer/src/GeneratedConfig.php
CONF=vendor/squizlabs/php_codesniffer/CodeSniffer.conf
# ni_base <nom> <lock> : les deux copies installées, puis gardées.
ni_base() {
  ref="$WORK/ni-$1-ref"; viv="$WORK/ni-$1-viv"; fresh "$2" "$ref" "$viv"
  step "$ref" "$viv" "ni-$1-install" "${I[@]}" -- "${V[@]}"
  rm -rf "$ref.base" "$viv.base"; cp -a "$ref" "$ref.base"; cp -a "$viv" "$viv.base"
}
# ni_base_composer <nom> <lock> : Composer installe les deux copies (un état
# que vivacity rend à Composer à l'install : versions non lues, alias).
ni_base_composer() {
  ref="$WORK/ni-$1-ref"; viv="$WORK/ni-$1-viv"; fresh "$2" "$ref" "$viv"
  for d in "$ref" "$viv"; do (cd "$d" && composer "${I[@]}" --no-interaction -q) || { echo "FAIL ni-$1 : install de Composer"; status=1; }; done
  rm -rf "$ref.base" "$viv.base"; cp -a "$ref" "$ref.base"; cp -a "$viv" "$viv.base"
}
ni_reset() { rm -rf "$ref" "$viv"; cp -a "$ref.base" "$ref"; cp -a "$viv.base" "$viv"; }
ni_rm() { for p in "$@"; do rm -rf "${ref:?}/$p" "${viv:?}/$p"; done; }
ni_json() { printf '%s\n' "$1" > "$ref/composer.json"; printf '%s\n' "$1" > "$viv/composer.json"; }
# ni_refused <nom> <motif de raison> <nombre> <args vivacity…> : rendu à
# Composer sous --no-fallback (code 3), raisons comptées, rien d'écrit.
ni_refused() {
  local name="$1" pat="$2" want="$3"; shift 3
  local v_code=0 n
  touch "$WORK/$name.mark"; sleep 1
  (cd "$viv" && "$VIVACITY" "$@" >"$WORK/$name.vivacity.all" 2>&1) || v_code=$?
  n=$(grep -cE -- "$pat" "$WORK/$name.vivacity.all" || true)
  if [ "$v_code" = 3 ] && [ "$n" = "$want" ] && [ -z "$(find "$viv" -newer "$WORK/$name.mark" -print -quit)" ]; then
    echo "OK   $name : rendu à Composer, $n raison(s), rien d'écrit"
  else
    echo "FAIL $name : code $v_code, $n raison(s) sur $want"; tail -6 "$WORK/$name.vivacity.all"; status=1
  fi
}
PS_BASE='"phpstan/extension-installer": "1.4.3", "phpstan/phpstan": "2.3.1"'
PS_DEV='"phpstan/phpstan-strict-rules": "2.0.4", "phpstan/phpstan-deprecation-rules": "2.0.5", "phpstan/php-8-stubs": "0.4.43"'
PS_IGNORE='"extra": {"phpstan/extension-installer": {"ignore": ["phpstan/phpstan-deprecation-rules"]}}'
PS_ALLOW='"config": {"allow-plugins": {"phpstan/extension-installer": true}}'
ps_json() { ni_json "{\"name\": \"vivacity/plugin-lines-phpstan\", \"require\": {$1}, \"require-dev\": {$2}, ${3:+$3, }$PS_ALLOW}"; }

# phpstan : la ligne à chaque fois ; les lignes de paquets quand le fichier
# change — calculées sur le dépôt local, quel que soit --no-dev.
ni_base ps phpstan
step "$ref" "$viv" ni-ps-noop "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-ps-noop 'looking for funding' "$PHPSTAN"; expect_not ni-ps-noop '^> '
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-ps-lost 'looking for funding' "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$' '^> phpstan/php-8-stubs: not supported$' '^> phpstan/phpstan-deprecation-rules: ignored$'
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-quiet "${NI[@]}" -q -- "${NI[@]}" -q --no-fallback
expect_not ni-ps-quiet '.'
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-no-dev "${NI[@]}" --no-dev -- "${NI[@]}" --no-dev --no-fallback
expect ni-ps-no-dev '^> phpstan/phpstan-strict-rules: installed$'
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-no-plugins "${NI[@]}" --no-plugins -- "${NI[@]}" --no-plugins --no-fallback
expect_not ni-ps-no-plugins '^phpstan/'
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-dry-run "${NI[@]}" --dry-run -- "${NI[@]}" --dry-run --no-fallback
expect_not ni-ps-dry-run '^phpstan/'
# Une extension ajoutée par l'update n'est pas dans le dépôt local ; une
# extension retirée y est encore.
ni_reset; ni_rm "$GC"; ps_json "$PS_BASE" "$PS_DEV, \"phpstan/phpstan-phpunit\": \"2.0.7\"" "$PS_IGNORE"
step "$ref" "$viv" ni-ps-ext-added "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-ps-ext-added 'Locking phpstan/phpstan-phpunit' "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$'; expect_not ni-ps-ext-added 'phpstan-phpunit: installed'
ni_reset; ni_rm "$GC"; ps_json "$PS_BASE" '"phpstan/phpstan-deprecation-rules": "2.0.5", "phpstan/php-8-stubs": "0.4.43"' "$PS_IGNORE"
step "$ref" "$viv" ni-ps-ext-removed "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-ps-ext-removed 'Removing phpstan/phpstan-strict-rules' "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$'
# Le plugin retiré par l'update : encore chargé.
ni_reset; ps_json '"phpstan/phpstan": "2.3.1"' "$PS_DEV" "$PS_IGNORE"
step "$ref" "$viv" ni-ps-plugin-removed "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-ps-plugin-removed 'Removing phpstan/extension-installer' "$PHPSTAN"
# L'`extra` de la racine est relu : `ignore` retiré, le fichier change.
ni_reset; ps_json "$PS_BASE" "$PS_DEV"
step "$ref" "$viv" ni-ps-ignore-dropped "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-ps-ignore-dropped '^> phpstan/phpstan-deprecation-rules: installed$'
ni_reset; ps_json "$PS_BASE" "$PS_DEV, \"phpstan/phpstan-phpunit\": \"2.0.7\"" "$PS_IGNORE"
step "$ref" "$viv" ni-ps-partial update phpstan/phpstan-phpunit --no-install "${R[@]}" -- update phpstan/phpstan-phpunit --no-install "${R[@]}" --no-fallback
order ni-ps-partial 'Locking phpstan/phpstan-phpunit' "$PHPSTAN"; expect_not ni-ps-partial '^> '
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-require require phpstan/phpstan-phpunit:2.0.7 --dev --no-install "${R[@]}" -- require phpstan/phpstan-phpunit:2.0.7 --dev --no-install "${R[@]}" --no-fallback
order ni-ps-require 'Writing lock file' "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$'; expect_not ni-ps-require 'phpstan-phpunit: installed'
# `remove` sans installation : l'écouteur lit le dépôt local (le paquet y est
# encore), puis « Removal failed » (le dépôt local, toujours), code 2.
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-remove remove phpstan/phpstan-strict-rules --dev --no-install "${R[@]}" -- remove phpstan/phpstan-strict-rules --dev --no-install "${R[@]}" --no-fallback
order ni-ps-remove "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$' 'Removal failed'
ni_reset; ni_rm "$GC"
step "$ref" "$viv" ni-ps-remove-plugin remove phpstan/extension-installer --no-install "${R[@]}" -- remove phpstan/extension-installer --no-install "${R[@]}" --no-fallback
order ni-ps-remove-plugin "$PHPSTAN" '^> phpstan/phpstan-strict-rules: installed$' 'Removal failed'
# Un répertoire d'extension supprimé : purgé du dépôt local.
ni_reset; ni_rm "$GC" vendor/phpstan/phpstan-strict-rules
step "$ref" "$viv" ni-ps-ext-dir-gone "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-ps-ext-dir-gone '^> phpstan/php-8-stubs: not supported$'; expect_not ni-ps-ext-dir-gone 'strict-rules: installed'
# Vendor vierge : aucun plugin chargé, ni funding.
ni_reset; ni_rm vendor
step "$ref" "$viv" ni-ps-bare "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-ps-bare '^phpstan/|funding'
# Le plugin ajouté par l'update : pas chargé.
lock ni-ps-noplugin "{\"name\": \"vivacity/plugin-lines-phpstan\", \"require\": {\"phpstan/phpstan\": \"2.3.1\"}, \"require-dev\": {$PS_DEV}, $PS_IGNORE, $PS_ALLOW}"
ni_base ps-add ni-ps-noplugin; ps_json "$PS_BASE" "$PS_DEV" "$PS_IGNORE"
step "$ref" "$viv" ni-ps-plugin-added "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-ps-plugin-added 'Locking phpstan/extension-installer'; expect_not ni-ps-plugin-added '^phpstan/'
# Un alias en ligne : absent du dépôt local sans installation — la version
# du paquet, une seule entrée (installé par Composer : vivacity rend
# l'install d'une extension aliasée).
ni_base_composer ps-alias aliased; ni_rm "$GC"
step "$ref" "$viv" ni-ps-alias "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-ps-alias '^> phpstan/phpstan-strict-rules: installed$'

# dealerdirect : la configuration de phpcs réécrite quand elle manque, sur
# les standards du dépôt local.
ni_base pc phpcs
step "$ref" "$viv" ni-pc-noop "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-pc-noop "$PHPCS"
ni_reset; ni_rm "$CONF"
step "$ref" "$viv" ni-pc-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-pc-lost 'looking for funding' "$PHPCS"'set to \.\./\.\./phpcsstandards/phpcsextra,\.\./\.\./phpcsstandards/phpcsutils$'
ni_reset; ni_rm "$CONF"
step "$ref" "$viv" ni-pc-no-dev "${NI[@]}" --no-dev -- "${NI[@]}" --no-dev --no-fallback
expect ni-pc-no-dev "$PHPCS"'set to \.\./\.\./phpcsstandards/phpcsextra'
# Le plugin retiré par l'update (avec les standards qui l'exigent) : encore
# chargé, il réenregistre les standards du dépôt local.
ni_reset; ni_rm "$CONF"; ni_json '{"name": "vivacity/plugin-lines-phpcs", "require": {"squizlabs/php_codesniffer": "4.0.4"}, "config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true}}}'
step "$ref" "$viv" ni-pc-plugin-removed "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-pc-plugin-removed 'Removing dealerdirect/phpcodesniffer-composer-installer' "$PHPCS"'set to \.\./\.\./phpcsstandards/phpcsextra,\.\./\.\./phpcsstandards/phpcsutils$'
ni_reset; ni_json '{"name": "vivacity/plugin-lines-phpcs", "require": {"dealerdirect/phpcodesniffer-composer-installer": "v1.2.1", "squizlabs/php_codesniffer": "4.0.4"}, "require-dev": {"phpcsstandards/phpcsextra": "1.5.1", "phpcsstandards/phpcsutils": "1.2.3", "slevomat/coding-standard": "8.31.1"}, "config": {"allow-plugins": {"dealerdirect/phpcodesniffer-composer-installer": true}}}'
step "$ref" "$viv" ni-pc-std-added "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-pc-std-added 'Locking slevomat/coding-standard'; expect_not ni-pc-std-added "$PHPCS"

# codeception/c3 : `askForUpdateV2` copie le c3.php du c3 INSTALLÉ quand il
# manque ; muet s'il est à jour ou modifié.
C3_ALLOW='"config": {"allow-plugins": {"codeception/c3": true}}'
lock ni-c3 "{\"name\": \"vivacity/plugin-lines-c3\", \"require\": {\"psr/log\": \"3.0.2\"}, \"require-dev\": {\"codeception/c3\": \"2.9.0\"}, $C3_ALLOW}"
lock ni-c3-old "{\"name\": \"vivacity/plugin-lines-c3\", \"require\": {\"psr/log\": \"3.0.2\"}, \"require-dev\": {\"codeception/c3\": \"2.8.1\"}, $C3_ALLOW}"
lock ni-c3-none "{\"name\": \"vivacity/plugin-lines-c3\", \"require\": {\"psr/log\": \"3.0.2\"}, $C3_ALLOW}"
ni_base c3 ni-c3
step "$ref" "$viv" ni-c3-noop "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-c3-noop "$C3"
ni_reset; ni_rm c3.php
step "$ref" "$viv" ni-c3-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-c3-lost 'Writing lock file' "$C3"'Copying' "$C3"'Include'
ni_reset; for d in "$ref" "$viv"; do echo '<?php // edited' > "$d/c3.php"; done
step "$ref" "$viv" ni-c3-edited "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-c3-edited "$C3"
ni_reset; ni_rm c3.php
step "$ref" "$viv" ni-c3-remove remove codeception/c3 --dev --no-install "${R[@]}" -- remove codeception/c3 --dev --no-install "${R[@]}" --no-fallback
order ni-c3-remove 'Removing codeception/c3' "$C3"'Copying' 'Removal failed'
ni_base c3-old ni-c3-old; ni_rm c3.php; ni_json "{\"name\": \"vivacity/plugin-lines-c3\", \"require\": {\"psr/log\": \"3.0.2\"}, \"require-dev\": {\"codeception/c3\": \"2.9.0\"}, $C3_ALLOW}"
step "$ref" "$viv" ni-c3-updated "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-c3-updated 'Upgrading codeception/c3 \(2\.8\.1 => 2\.9\.0\)' "$C3"'Copying'
ni_base c3-add ni-c3-none; ni_json "{\"name\": \"vivacity/plugin-lines-c3\", \"require\": {\"psr/log\": \"3.0.2\"}, \"require-dev\": {\"codeception/c3\": \"2.9.0\"}, $C3_ALLOW}"
step "$ref" "$viv" ni-c3-plugin-added "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-c3-plugin-added 'Locking codeception/c3'; expect_not ni-c3-plugin-added "$C3"

# rector : seulement quand le fichier change ; une branche par défaut
# (alias chargé avant son paquet : la version du paquet).
ni_base rc rector; ni_rm "$RGC"
step "$ref" "$viv" ni-rc-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-rc-lost 'Writing lock file' "$RECTOR" '^> rector/rector-symfony: installed$'
lock ni-rc-main '{"name": "vivacity/plugin-lines-rector-main", "minimum-stability": "dev", "prefer-stable": true,
  "require": {"rector/extension-installer": "0.11.2"}, "require-dev": {"rector/rector-symfony": "dev-main"},
  "config": {"allow-plugins": {"rector/extension-installer": true}}}'
ni_base rc-main ni-rc-main
step "$ref" "$viv" ni-rc-main-noop "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-rc-main-noop "$RECTOR"
ni_reset; ni_rm "$RGC"
step "$ref" "$viv" ni-rc-main-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-rc-main-lost '^> rector/rector-symfony: installed$'

# Les quatre : l'ordre d'activation d'installed.json (dealerdirect d'abord,
# puis les noms), sans transaction.
ni_base all all
step "$ref" "$viv" ni-all-noop "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-all-noop 'looking for funding' "$PHPSTAN"; expect_not ni-all-noop "$PHPCS|$C3|$RECTOR"
ni_reset; ni_rm "$GC" "$RGC" "$CONF" c3.php
step "$ref" "$viv" ni-all-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-all-lost 'looking for funding' "$PHPCS"'set to' "$C3"'Copying' "$PHPSTAN" "$RECTOR"
ni_reset; ni_rm "$GC" "$RGC" "$CONF" c3.php
step "$ref" "$viv" ni-all-quiet "${NI[@]}" -q -- "${NI[@]}" -q --no-fallback
expect_not ni-all-quiet '.'

# Replis : les versions non lues des trois écouteurs (package-versions,
# muet sans dump, n'est pas nommé).
ni_base_composer old old-versions
ni_refused ni-old-versions '^  - plugin (dealerdirect/phpcodesniffer-composer-installer v0\.7\.1|phpstan/extension-installer 1\.3\.1|rector/extension-installer 0\.10\.2) is not emulated in this version' 3 "${NI[@]}" --no-fallback
if grep -q '^  - plugin composer/package-versions-deprecated' "$WORK/ni-old-versions.vivacity.all"; then echo "FAIL ni-old-versions : package-versions nommé"; status=1; fi
# php-http/discovery : son `postUpdate` interroge le dépôt local, pas le
# lock, et peut éditer composer.json puis relancer un `Installer` (mesuré :
# sans fin sous --no-install) ; quand il agirait (`getMissingRequires`
# porté, 1.19.0–1.20.0), rendu. Composer n'est pas lancé sur les cas rendus
# (il boucle sur `ni-discovery-dev`, mesuré par la méta-analyse).
DISC='"php-http/discovery": "1.20.0", "psr/http-client-implementation": "*", "guzzlehttp/guzzle": "7.10.0"'
DISC_ALLOW='"config": {"allow-plugins": {"php-http/discovery": true}}'
DISC_PAT='^  - plugin php-http/discovery (looks for PSR implementations|would add or move a PSR implementation)'
lock ni-discovery "{\"name\": \"vivacity/plugin-lines-discovery\", \"require\": {$DISC}, $DISC_ALLOW}"
ni_base discovery ni-discovery
# Depuis le 2026-10-27, sa décision est portée (docs/plans/plugin-allow-discovery.md) :
# l'implémentation installée en `require`, il ne fait rien — natif, comparé.
step "$ref" "$viv" ni-discovery-same "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-discovery-same 'Nothing to modify in lock file'
ni_json "{\"name\": \"vivacity/plugin-lines-discovery\", \"require\": {$DISC, \"psr/log\": \"3.0.2\"}, $DISC_ALLOW}"
step "$ref" "$viv" ni-discovery "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-discovery 'Locking psr/log'
lock ni-discovery-dev "{\"name\": \"vivacity/plugin-lines-discovery-dev\", \"type\": \"project\",
  \"require\": {\"php-http/discovery\": \"1.20.0\", \"psr/http-client-implementation\": \"*\"},
  \"require-dev\": {\"guzzlehttp/guzzle\": \"7.10.0\"}, $DISC_ALLOW}"
ni_base discovery-dev ni-discovery-dev
ni_refused ni-discovery-dev "$DISC_PAT" 1 "${NI[@]}" --no-fallback
# Rien de surveillé (aucune abstraction exigée) : natif, comparé.
lock ni-discovery-idle "{\"name\": \"vivacity/plugin-lines-discovery-idle\",
  \"require\": {\"php-http/discovery\": \"1.20.0\", \"psr/http-client\": \"1.0.3\"}, $DISC_ALLOW}"
ni_base discovery-idle ni-discovery-idle
ni_json "{\"name\": \"vivacity/plugin-lines-discovery-idle\", \"require\": {\"php-http/discovery\": \"1.20.0\", \"psr/http-client\": \"1.0.3\", \"psr/log\": \"3.0.2\"}, $DISC_ALLOW}"
step "$ref" "$viv" ni-discovery-idle "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-discovery-idle 'Locking psr/log'

# Bloqué (`allow-plugins` à false) : pas chargé.
ni_base ps-blocked phpstan; ni_rm "$GC"
for d in "$ref" "$viv"; do jq '.config["allow-plugins"]["phpstan/extension-installer"] = false' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json"; done
step "$ref" "$viv" ni-ps-blocked "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-ps-blocked '^phpstan/'
# Sans --no-scripts (aucun script déclaré) : les écouteurs partent pareil.
ni_reset; ni_rm "$GC"; for d in "$ref" "$viv"; do jq '.config["allow-plugins"]["phpstan/extension-installer"] = true' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json"; done
step "$ref" "$viv" ni-ps-scripts update --no-install --no-audit --no-security-blocking -- update --no-install --no-audit --no-security-blocking --no-fallback
expect ni-ps-scripts '^> phpstan/phpstan-strict-rules: installed$'
# Le répertoire du plugin supprimé : sa classe introuvable, pas chargé.
ni_reset; ni_rm "$GC" vendor/phpstan/extension-installer
step "$ref" "$viv" ni-ps-plugin-dir-gone "${NI[@]}" -- "${NI[@]}" --no-fallback
expect_not ni-ps-plugin-dir-gone '^phpstan/'
# `config.lock: false` : aucun lock écrit, l'écouteur part quand même.
ni_reset; ni_rm "$GC"
for d in "$ref" "$viv"; do jq '.config.lock = false' "$d/composer.json" > "$d/c.tmp" && mv "$d/c.tmp" "$d/composer.json"; done
step "$ref" "$viv" ni-ps-lock-false "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-ps-lock-false '^> phpstan/phpstan-strict-rules: installed$'
# Un ensemble insoluble : code 2, aucun écouteur.
ni_reset; ni_rm "$GC"; ps_json "$PS_BASE, \"phpstan/phpstan-strict-rules\": \"9.9.9\"" '"phpstan/php-8-stubs": "0.4.43"' "$PS_IGNORE"
step "$ref" "$viv" ni-ps-unsolvable "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-ps-unsolvable 'Your requirements could not be resolved'; expect_not ni-ps-unsolvable '^phpstan/'
# --working-dir depuis le répertoire parent : `getcwd()` est le projet.
ni_reset; ni_rm "$GC"
c_code=0; v_code=0
(cd "$WORK" && composer "${NI[@]}" --working-dir="$ref" --no-interaction --no-ansi >"$WORK/ni-ps-working-dir.composer.all" 2>&1) || c_code=$?
(cd "$WORK" && "$VIVACITY" "${NI[@]}" --working-dir="$viv" --no-fallback >"$WORK/ni-ps-working-dir.vivacity.all" 2>&1) || v_code=$?
if [ "$c_code" = "$v_code" ] && diff <(grep -v '^  - Downloading ' "$WORK/ni-ps-working-dir.composer.all") <(grep -v '^vivacity: ' "$WORK/ni-ps-working-dir.vivacity.all") >/dev/null \
   && grep -q '^> phpstan/phpstan-strict-rules: installed$' "$WORK/ni-ps-working-dir.composer.all" \
   && compare_vendor "$ref" "$viv" "$WORK/ni-ps-working-dir.diff" >/dev/null; then
  echo "OK   ni-ps-working-dir : projet et sortie fusionnée identiques"
else
  echo "FAIL ni-ps-working-dir : codes $c_code vs $v_code"; { diff "$WORK/ni-ps-working-dir.composer.all" "$WORK/ni-ps-working-dir.vivacity.all" || true; } | head -8; status=1
fi
# Un vendor-dir configuré : les chemins écrits suivent.
lock ni-ps-vendor-dir "{\"name\": \"vivacity/plugin-lines-phpstan-vendor-dir\", \"require\": {$PS_BASE}, \"require-dev\": {$PS_DEV}, $PS_IGNORE,
  \"config\": {\"vendor-dir\": \"lib\", \"allow-plugins\": {\"phpstan/extension-installer\": true}}}"
ni_base ps-vendor-dir ni-ps-vendor-dir; ni_rm lib/phpstan/extension-installer/src/GeneratedConfig.php
step "$ref" "$viv" ni-ps-vendor-dir "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-ps-vendor-dir '^> phpstan/phpstan-strict-rules: installed$'
# Un écouteur émulé installé GLOBALEMENT : chargé aussi (le dépôt global),
# vivacity ne joue que ceux du projet — rendu.
ni_base ps-global phpstan
# (`Factory::createGlobalComposer` : seulement avec COMPOSER_HOME/composer.json)
mkdir -p "$WORK/home/vendor/composer" "$WORK/home/vendor/phpstan/extension-installer"
# (autorisé par la configuration globale, son API exigée : chargé)
printf '{"config": {"allow-plugins": {"phpstan/extension-installer": true}}}\n' > "$WORK/home/composer.json"
printf '{"packages": [{"name": "phpstan/extension-installer", "version": "1.4.3", "type": "composer-plugin", "require": {"composer-plugin-api": "^2.0"}, "install-path": "../phpstan/extension-installer"}], "dev": true, "dev-package-names": []}\n' > "$WORK/home/vendor/composer/installed.json"
ni_refused ni-global '^  - plugin phpstan/extension-installer is installed globally' 1 "${NI[@]}" --no-fallback
rm -rf "$WORK/home/vendor" "$WORK/home/composer.json"
# yiisoft/yii2-composer : `activate` recrée extensions.php, sans
# installation et en dry run aussi.
lock ni-yii2 '{"name": "vivacity/plugin-lines-yii2", "require": {"yiisoft/yii2-composer": "2.0.11", "psr/log": "3.0.2"},
  "config": {"allow-plugins": {"yiisoft/yii2-composer": true}}}'
ni_base yii2 ni-yii2; ni_rm vendor/yiisoft/extensions.php
step "$ref" "$viv" ni-yii2-lost "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-yii2-lost 'Nothing to modify in lock file'
ni_reset; ni_rm vendor/yiisoft/extensions.php
step "$ref" "$viv" ni-yii2-dry-run update --dry-run "${R[@]}" -- update --dry-run "${R[@]}" --no-fallback
expect ni-yii2-dry-run 'Nothing to modify in lock file'
# Un ensemble insoluble : le plugin a été activé au chargement quand même.
ni_reset; ni_rm vendor/yiisoft/extensions.php
ni_json '{"name": "vivacity/plugin-lines-yii2", "require": {"yiisoft/yii2-composer": "2.0.11", "psr/log": "9.9.9"}, "config": {"allow-plugins": {"yiisoft/yii2-composer": true}}}'
step "$ref" "$viv" ni-yii2-unsolvable "${NI[@]}" -- "${NI[@]}" --no-fallback
expect ni-yii2-unsolvable 'Your requirements could not be resolved'
# cweagans/composer-patches (installé par Composer : vivacity rend son
# install) : `PRE_UPDATE_CMD` part aussi sans installation — rendu.
lock ni-patches '{"name": "vivacity/plugin-lines-patches", "require": {"cweagans/composer-patches": "1.7.3", "psr/log": "3.0.2"},
  "config": {"allow-plugins": {"cweagans/composer-patches": true}}}'
ni_base_composer patches ni-patches
ni_refused ni-patches '^  - plugin cweagans/composer-patches runs listeners' 1 "${NI[@]}" --no-fallback
# symfony/flex avec phpstan : la sortie de Flex à son rang d'activation
# (phpstan avant symfony/flex, ordre des noms), l'index servi en local.
flex_index_serve
lock ni-flex "{\"name\": \"vivacity/plugin-lines-flex\", \"require\": {\"symfony/flex\": \"v2.11.0\", $PS_BASE},
  \"extra\": {\"symfony\": {\"endpoint\": [\"$FLEX_INDEX_URL\"]}},
  \"config\": {\"secure-http\": false, \"allow-plugins\": {\"symfony/flex\": true, \"phpstan/extension-installer\": true}}}"
ni_base flex ni-flex; ni_rm "$GC"
step "$ref" "$viv" ni-flex "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-flex 'looking for funding' "$PHPSTAN" '^Run composer recipes at any time'
# Flex requis par un paquet installé (`PackageSorter` : poids -1) : activé
# avant phpstan, sa sortie passe devant — l'ancien emplacement (Flex en
# dernier) ne le donnerait pas.
mkdir -p "$WORK/pkg/needs-flex"
printf '{"name": "vivacity/needs-flex", "version": "1.0.0", "require": {"symfony/flex": "v2.11.0"}}\n' > "$WORK/pkg/needs-flex/composer.json"
lock ni-flex-weighted "{\"name\": \"vivacity/plugin-lines-flex-weighted\",
  \"repositories\": [{\"type\": \"path\", \"url\": \"$WORK/pkg/needs-flex\"}],
  \"require\": {\"vivacity/needs-flex\": \"1.0.0\", \"symfony/flex\": \"v2.11.0\", $PS_BASE},
  \"extra\": {\"symfony\": {\"endpoint\": [\"$FLEX_INDEX_URL\"]}},
  \"config\": {\"secure-http\": false, \"allow-plugins\": {\"symfony/flex\": true, \"phpstan/extension-installer\": true}}}"
ni_base flex-weighted ni-flex-weighted; ni_rm "$GC"
step "$ref" "$viv" ni-flex-weighted "${NI[@]}" -- "${NI[@]}" --no-fallback
order ni-flex-weighted 'looking for funding' '^Run composer recipes at any time' "$PHPSTAN"
flex_index_stop
exit $status
