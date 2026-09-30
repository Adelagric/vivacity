#!/usr/bin/env bash
# Drift, premier étage : chaque fichier de docs/reference/ est la copie d'un
# fichier du phar Composer 2.10.3 (ou d'une dépendance embarquée). On extrait
# les mêmes fichiers du Composer installé et on diffe : un fichier qui a bougé
# désigne exactement le port à revoir, avant même de lancer le harness.
#
# Usage : harness/drift-reference.sh [composer.phar]   (défaut : `which composer`)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PHAR="${1:-$(command -v composer)}"
[ -f "$PHAR" ] || { echo "composer introuvable"; exit 1; }
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
cp "$PHAR" "$TMP/composer.phar"

# docs/reference/<fichier> → chemin dans le phar
twin() {
  case "$1" in
    ArrayDumper.php)          echo "src/Composer/Package/Dumper/ArrayDumper.php" ;;
    AutoloadGenerator.php)    echo "src/Composer/Autoload/AutoloadGenerator.php" ;;
    BinaryInstaller.php)      echo "src/Composer/Installer/BinaryInstaller.php" ;;
    ClassLoader.php)          echo "src/Composer/Autoload/ClassLoader.php" ;;
    Factory.php)              echo "src/Composer/Factory.php" ;;
    FilesystemRepository.php) echo "src/Composer/Repository/FilesystemRepository.php" ;;
    InstalledVersions.php)    echo "src/Composer/InstalledVersions.php" ;;
    JsonFile.php)             echo "src/Composer/Json/JsonFile.php" ;;
    JsonManipulator.php)      echo "src/Composer/Json/JsonManipulator.php" ;;
    JsonConfigSource.php)     echo "src/Composer/Config/JsonConfigSource.php" ;;
    RemoveCommand.php)        echo "src/Composer/Command/RemoveCommand.php" ;;
    VersionSelector.php)      echo "src/Composer/Package/Version/VersionSelector.php" ;;
    RequireCommand.php)       echo "src/Composer/Command/RequireCommand.php" ;;
    PackageDiscoveryTrait.php) echo "src/Composer/Command/PackageDiscoveryTrait.php" ;;
    Locker.php)               echo "src/Composer/Package/Locker.php" ;;
    PackageSorter.php)        echo "src/Composer/Util/PackageSorter.php" ;;
    RootPackageLoader.php)    echo "src/Composer/Package/Loader/RootPackageLoader.php" ;;
    VersionGuesser.php)       echo "src/Composer/Package/Version/VersionGuesser.php" ;;
    Filesystem.php)           echo "src/Composer/Util/Filesystem.php" ;;
    PluginManager.php)        echo "src/Composer/Plugin/PluginManager.php" ;;
    ArchiveDownloader.php)    echo "src/Composer/Downloader/ArchiveDownloader.php" ;;
    ZipDownloader.php)        echo "src/Composer/Downloader/ZipDownloader.php" ;;
    TarDownloader.php)        echo "src/Composer/Downloader/TarDownloader.php" ;;
    LibraryInstaller.php)     echo "src/Composer/Installer/LibraryInstaller.php" ;;
    cmg-ClassMap.php)         echo "vendor/composer/class-map-generator/src/ClassMap.php" ;;
    cmg-ClassMapGenerator.php) echo "vendor/composer/class-map-generator/src/ClassMapGenerator.php" ;;
    cmg-FileList.php)         echo "vendor/composer/class-map-generator/src/FileList.php" ;;
    cmg-PhpFileCleaner.php)   echo "vendor/composer/class-map-generator/src/PhpFileCleaner.php" ;;
    cmg-PhpFileParser.php)    echo "vendor/composer/class-map-generator/src/PhpFileParser.php" ;;
    Installer.php)            echo "src/Composer/Installer.php" ;;
    SuggestedPackagesReporter.php) echo "src/Composer/Installer/SuggestedPackagesReporter.php" ;;
    UpdateCommand.php)        echo "src/Composer/Command/UpdateCommand.php" ;;
    PathDownloader.php)       echo "src/Composer/Downloader/PathDownloader.php" ;;
    FileDownloader.php)       echo "src/Composer/Downloader/FileDownloader.php" ;;
    ArchivableFilesFinder.php) echo "src/Composer/Package/Archiver/ArchivableFilesFinder.php" ;;
    ArchivableFilesFilter.php) echo "src/Composer/Package/Archiver/ArchivableFilesFilter.php" ;;
    BaseExcludeFilter.php)    echo "src/Composer/Package/Archiver/BaseExcludeFilter.php" ;;
    GitExcludeFilter.php)     echo "src/Composer/Package/Archiver/GitExcludeFilter.php" ;;
    symfony-Filesystem.php)   echo "vendor/symfony/filesystem/Filesystem.php" ;;
    symfony-finder-Glob.php)  echo "vendor/symfony/finder/Glob.php" ;;
    *) echo "" ;;
  esac
}

# docs/reference/resolver/<fichier> → chemin dans le phar : le fichier est
# cherché sous les espaces de noms que le résolveur touche ; `semver-X.php`
# est composer/semver (src/ ou src/Constraint/), `Operation/X.php` les
# opérations du solveur.
resolver_twin() {
  local name="$1" base cands c
  case "$name" in
    Operation/*) echo "src/Composer/DependencyResolver/$name"; return ;;
    semver-*)
      base="${name#semver-}"
      cands="vendor/composer/semver/src/$base vendor/composer/semver/src/Constraint/$base" ;;
    xdebug-handler-*)
      cands="vendor/composer/xdebug-handler/src/${name#xdebug-handler-}" ;;
    *)
      cands="src/Composer/DependencyResolver/$name src/Composer/Repository/$name src/Composer/Package/$name src/Composer/Package/Loader/$name src/Composer/Package/Version/$name src/Composer/Filter/PlatformRequirementFilter/$name vendor/composer/metadata-minifier/src/$name src/Composer/Policy/$name src/Composer/Advisory/$name src/Composer/FilterList/$name src/Composer/FilterList/FilterListProvider/$name src/Composer/Util/$name" ;;
  esac
  for c in $cands; do
    if php -r 'exit(@file_get_contents("phar://'"$TMP"'/composer.phar/'"$c"'") === false ? 1 : 0);' 2>/dev/null; then
      echo "$c"; return
    fi
  done
  echo ""
}

# Dérives déjà lues et tranchées : une ligne `<fichier> <empreinte> <raison>`
# dans docs/reference/DRIFT-ACK. L'empreinte est celle du diff lui-même, donc
# un NOUVEAU mouvement amont réalerte — un acquittement ne peut pas endormir
# la surveillance, seulement taire un écart dont la décision est écrite.
ACK="$ROOT/docs/reference/DRIFT-ACK"
ack_reason() { # fichier, empreinte
  [ -f "$ACK" ] || return 1
  awk -v f="$1" -v h="$2" '
    /^[[:space:]]*(#|$)/ { next }
    $1 == f && $2 == h { $1 = ""; $2 = ""; sub(/^[[:space:]]+/, ""); print; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$ACK"
}

version=$(php "$TMP/composer.phar" --version --no-ansi 2>/dev/null | sed -n 's/^Composer version \([^ ]*\).*/\1/p')
echo "Composer $version vs docs/reference (2.10.3)"
status=0; checked=0; acked=0
for f in "$ROOT"/docs/reference/*.php "$ROOT"/docs/reference/resolver/*.php "$ROOT"/docs/reference/resolver/Operation/*.php "$ROOT"/docs/reference/policy/*.php; do
  name=$(basename "$f")
  [ -s "$f" ] || continue
  case "$f" in
    */docs/reference/resolver/Operation/*) inner=$(resolver_twin "Operation/$name") ;;
    */docs/reference/resolver/*) inner=$(resolver_twin "$name") ;;
    */docs/reference/policy/*) inner=$(resolver_twin "$name") ;;
    *) inner=$(twin "$name") ;;
  esac
  [ -n "$inner" ] || { echo "??   $name : pas de jumeau connu (à ajouter dans twin())"; status=1; continue; }
  if ! php -r 'echo file_get_contents("phar://'"$TMP"'/composer.phar/'"$inner"'");' > "$TMP/twin.php" 2>/dev/null; then
    echo "MISSING $name : $inner absent du phar"; status=1; continue
  fi
  if diff -q "$f" "$TMP/twin.php" >/dev/null; then
    checked=$((checked + 1))
  else
    # `diff` sort 1 quand les fichiers diffèrent, et `set -e -o pipefail`
    # tuerait l'affectation : le diff est écrit une fois, puis lu.
    diff "$f" "$TMP/twin.php" > "$TMP/drift.diff" || true
    lines=$(grep -c '^[<>]' "$TMP/drift.diff" || true)
    fingerprint=$(shasum -a 256 < "$TMP/drift.diff" | cut -d' ' -f1)
    if reason=$(ack_reason "$name" "$fingerprint"); then
      echo "ACK   $name ($inner) : $lines lignes, acquittées — $reason"
      acked=$((acked + 1))
    else
      echo "DRIFT $name ($inner) : $lines lignes"
      echo "      pour acquitter, après avoir lu le diff et écrit la décision :"
      echo "      $name $fingerprint <raison en une ligne>"
      status=1
    fi
  fi
done
if [ "$acked" -gt 0 ]; then
  echo "$checked fichiers identiques, $acked dérives acquittées (docs/reference/DRIFT-ACK)"
else
  echo "$checked fichiers identiques"
fi
exit $status
