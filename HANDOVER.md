# HANDOVER

État par jalon, commandes de validation, risques connus, pièges. Mis à jour en continu.

## Validation (toutes plateformes de dev)

```bash
fixtures/make.sh                         # une fois : crée + qualifie les 6 fixtures (php+composer requis)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                               # inclut les tests oracle (php + composer dans le PATH)
cargo build --release
harness/diff-vendor.sh [--with-autoloader]   # parité vs Composer sur les 6 fixtures (projet entier pour wordpress et drupal ; second passage --no-plugins des deux côtés, natif exigé) — harness/lib/compare.sh : diff -r + inventaire des modes et des liens
harness/net-retry.sh harness/<nom>.sh     # ce que la CI lance pour chaque harnais : en échec avec une erreur réseau dans la sortie (DNS perdu par un runner, curl 6/7/28, erreur d'envoi du client de vivacity), un second passage une minute plus tard ; une vraie différence échoue deux fois ; un refus de GitHub (« Could not authenticate against github.com », 403/429 sur api/codeload, d'un côté ou de l'autre) n'est pas relancé : la cause est nommée (libellé de Composer ou de vivacity seul, COMPOSER_AUTH posé ou non), le harnais reste rouge
harness/net-retry-selftest.sh            # net-retry.sh sur des harnais scriptés et sur l'extrait capturé du run macOS du 2026-10-29 (fixtures/net-retry/) : refus de GitHub non relancé et nommé, réseau relancé une fois, différence rouge du premier coup
harness/auth-hosts.sh                    # où part un jeton github-oauth (2026-10-29, docs/plans/ci-github-token.md), sans réseau : mandataire HTTP local qui journalise les en-têtes, dists http:// sur api.github.com (302 vers codeload), codeload, github.com, objects.github.com, un domaine GHE ; sans jeton, COMPOSER_AUTH, auth.json du projet + COMPOSER_AUTH — mêmes requêtes et mêmes en-têtes que Composer, journal attendu ligne à ligne, et le jeton ne change ni stdout, ni stderr, ni le projet
harness/removal.sh                       # paquets retirés du lock : même projet que Composer après
harness/root-scan.sh                     # scans hors store : 9 éditions de sources racine + classe ambiguë + violation PSR vs Composer (vendor/ et avertissements)
bench/ci-bench.sh && bench/gate.py <dir> --baseline bench/results/baseline-ratio.json   # ratio vivacity/Composer par scénario (bench.yml en CI ; baseline = médiane de plusieurs runs, --merge)
harness/transitions.sh                   # montée de version d'un plugin émulé → main rendue à Composer, disque intact
harness/update.sh                        # `composer update` vs `vivacity update` (complet + 10 cas partiels, dont 3 sur `solver-held-branch` : branches dev tenues via leur branch-alias) : locks identiques sur l'instantané Packagist figé (drupal avec `--no-security-blocking` des deux côtés : les avis de packages.drupal.org ne sont pas figés)
harness/steps.sh                         # `composer update|require|remove|install` vs vivacity (279 cas, 239 avec stderr identique) : composer.json, lock, stderr et code retour identiques
harness/scripts.sh                       # --run-scripts, 6 cas : journal des 4 événements identique à Composer (dev, --no-dev, dump -o), échec propagé, --no-autoloader, rien sans le drapeau ; ordre des lignes stdout+stderr fusionnées : `COMMAND` de bamarni avant pre-install-cmd, « Generating autoload files » avant pre-autoload-dump, funding avant post-install-cmd
harness/merge-plugin.sh [variante...]    # wikimedia/composer-merge-plugin (+ `install-repos-filter` : la liste malware d'un dépôt déclaré par un fichier inclus bloque le lock, code 2, lignes identiques ; ignorée sur vendor vierge, plugin pas encore actif) : 5 variantes × install, dump -o, dump -a, --no-dev (référence = plugin pré-amorcé, lock inchangé vérifié) + exigence absente (vendor chaud : code 4 identique ; vierge : repli avec la raison) + `update` sur vendor vierge refusé + résolution plugin actif (update, --no-dev, replace, flag @dev inclus, require, remove, update avec install : lock identique ; include avec `repositories` → repli)
harness/tar-dist.sh                      # dists `tar` (asset-packagist) : install, no-op sans téléchargement (cache Composer `.tar` partagé), --no-dev, dump -o — projet identique modes compris
harness/yii2-composer.sh                 # yiisoft/yii2-composer + codeception/c3, 11 cas : extensions.php et c3.php (install, no-op, --no-dev, retour dev, dump -o, vendor vierge --no-dev, --no-plugins) — projet identique, carte comparée clés triées, ordre stdout+stderr fusionnées (copie de c3 après le funding, « Deleting c3.php » avant `- Removing codeception/c3`) ; un projet c3 sans yii2 (Packagist en ligne) : `update` avec installation, copie et suppression sous `--no-autoloader`
harness/bin-plugin.sh                    # bamarni/composer-bin-plugin, 12 cas : lignes de dépréciation aux mêmes points (install, no-op, dump, --no-dev ; puis update, update --no-install, update --dry-run, require, remove, update partiel sans lock, Packagist en ligne ; stderr complète identique), forward-command → repli
harness/wp-core.sh                       # roots/wordpress-core-installer : wordpress-core à wordpress-install-dir (chaîne, carte, défaut), --no-plugins, `.` refusé — projet identique
harness/custom-dirs.sh                   # mnsami/composer-custom-directory-installer : bibliothèques nommées dans installer-paths (install, no-op, carte vidée, --no-plugins) — projet identique
harness/custom-installers.sh             # kirby, neos, davidbarratt, johnpbloch + installers 1.12, thelia, contao-components, oomphinc, craftcms : install, no-op, mise à jour, retrait, --no-plugins (vierge et sur l'arbre posé), dump-autoload et remove --no-update (dépréciations en tête), les rendus (versions non lues, priorité, déplacement, target-dir) — projet, stdout, stderr identiques
harness/plugins-round2.sh                # dg/composer-cleaner, liborm85/composer-vendor-cleaner, cakephp/plugin-installer, laminas-skeleton-installer, et spiral, sculpin, laminas-component-installer inertes : install, no-op, mise à jour, retrait, --no-plugins, dump-autoload, update, --no-dev, -v (cakephp), le plugin retiré ; les rendus (versions non lues — au lock et chargées —, thème, publication, config laminas en dev, retrait du squelette, require, --no-install, alias, binaire touché…) — projet, stdout, stderr identiques, sous PHP 8.4 et 8.5
harness/composer-packages.sh             # drupol/composer-packages 2.0.0 (docs/plans/duration-drupol-collector.md), 29 cas : Composer et vivacity AU MÊME CHEMIN ABSOLU (renommés entre les passages : Directories.php porte les chemins d'installation sous `var_export` et sous l'échappement HTML de Twig), install, no-op (les classes générées dans la classmap), mise à jour, retrait, dump-autoload (×3), --no-plugins, --no-dev, -v, sans nom racine, plugin refusé, `update`, `require -v`/`remove -v` (`_composer_tmpN`), `config.lock: false` ; les rendus (1.1.2, Twig 2, retrait du plugin, dump sans lock ou lock désactivé, `--run-scripts` avant `pre-install-cmd`, composer/installers, liborm85, type numérique, zéro négatif) — les cinq classes à l'octet hors les chiffres de `GENERATED_AT` (compare.sh), dont la valeur doit tomber dans la fenêtre du passage de vivacity ; sous PHP 8.4 et 8.5
harness/plugins-round3.sh                # infection/extension-installer 0.1.2 (avec dealerdirect 1.2.1 au même post-install-cmd), typo3/class-alias-loader v2.0.1 et typo3/cms-composer-installers v5.0.2 (inerte sous la racine `typo3/cms`) — docs/plans/remaining-fallbacks.md : install, no-op, mise à jour, retrait, --no-plugins, dump-autoload, --no-dev, -v, update ; le suffixe aléatoire de class-alias-loader masqué par compare.sh, vérifié égal à celui d'autoload.php après un no-op ; les rendus (versions, alias, cartes d'alias, autre racine, vendor-dir, --run-scripts)
harness/composer-patcher.sh              # mlocati/composer-patcher 1.2.6 : correctifs appliqués par le même `patch` (et `git apply` pour un dist à `.git`), patches_applied dans installed.json, no-op (déjà appliqué / échec du patch de macOS), mise à jour (ordre du dépôt local), retrait, --no-plugins, --no-dev, la racine fournisseur (nommée, `__root__`), une classe ajoutée par correctif (no-op -o, dump-autoload -o), un correctif qui échoue, le store intact ; stdout : le répertoire `PCH…` masqué, les lignes « Applying » triées par paquet à l'install vierge seulement ; les rendus (patches-file, distant, racine visée, alias, hash changé, update, -v, --run-scripts, 1.2.5)
harness/flex-update.sh                   # `update` AVEC install sur un projet Flex (tranches B et C), 26 cas : natif quand aucun paquet enregistré n'a de recette dans l'index servi (`index-recipes.json`, sans fichiers de recette) — la classe de bundle lue dans le dist pour un paquet que l'install pose (`dist-bundle`, `dist-no-bundle`), dans vendor/ sinon, et depuis le chantier des configurateurs la recette auto-générée appliquée et comparée (`bundle-class`, `dist-bundle`, `real-lock-gap`) ; le rappel `symfony/thanks` sur une opération d'update (`thanks`) ; repli pour une recette dont le fichier n'est pas servi (`recipe-in-index`), une suppression (`remove-package`), symfony/flex installé par le run (`flex-installed`) ; le vrai `symfony.lock` de la fixture (30 entrées, 123 paquets enregistrés) natif, et le même moins une entrée à bundle (recette auto-générée appliquée) — la paire prouve que c'est ce fichier qui décide de l'ensemble enregistré, et les six cas `sync-*` du garde de synchronisation (importmap.php ou package.json sans paquet `symfony-ux` : natif ; paquet `symfony-ux`, assets/controllers.json, lien `file:` mort, fichier non normalisé : repli). ATTENTION : toute retouche de l'émulation de Flex doit aussi faire tourner flex-recipes.sh, transitions.sh et vendor-dir.sh, qui affirment ses motifs de repli
harness/flex-recipes.sh                  # les recettes de Flex APPLIQUÉES par `update` (docs/plans/flex-configurators.md), 26 cas : les vrais index et fichiers de recette de symfony/recipes et recipes-contrib servis hors ligne (fixtures/flex/configurators/, PROVENANCE) ; bundles (existant, absent, auto-généré), copy-from-recipe (exécutable, fichier existant non écrasé, échange phpunit.xml.dist), gitignore (créé, ajouté), env (commentaires `#n`, guillemets, XML PHPUnit), dotenv, composer-scripts + réécriture du lock par `updateComposerLock`, add-lines (`after_target`, `requires`, `warn_if_missing` au niveau normal), docker à false, contrib, paquet posé par le run avec composer.lock réécrit (`install-recipe`), ordre de plusieurs recettes, `synchronize_package_json` à faux, -q ; sans recette : tout verrouillé, --no-dev, --no-plugins ; replis : docker non décidé, `%generate(secret)%`, contrib refusée, importmap.php créé avec paquets symfony-ux, bin-dir `bin`. Chaque cas affirme que l'oracle applique (ou n'applique pas) des recettes avant de comparer projet, composer.lock, symfony.lock, stdout et stderr (session normalisée) ; un repli exige l'arbre du projet et une empreinte de vendor/ inchangés
harness/root-manifest.sh                 # les refus de `RootPackageLoader::load` (nom racine, auto-exigence, nom de lien) sur les cinq commandes, 35 cas (+1 : `config.lock: false` ne fait rien écrire avant le refus), plus 6 cas `run_lock` (lock sans `packages-dev` : l'en-tête puis le message, avant « Verifying… » ; `--no-dev`, après les opérations ; `update`, `require` — fichiers rétablis —, `remove`) et 2 sous `GITHUB_ACTIONS=true` (l'annotation `::error ::` comparée sur stdout, comme pour tous les cas quand le banc tourne sur le runner) : même code, même TEXTE de message (l'encadré de Symfony Console extrait côté Composer, COLUMNS large) et rien d'écrit — vérifié par `find -newer` sur un témoin, parce que `compare_vendor` ne compare pas les mtimes. Les codes de sortie et les fichiers sont couverts en plus par 9 lignes de steps.sh (`@nostderr` : aucune ancre de steps.sh ne décrit un encadré d'erreur)
harness/transport-exit.sh                # code 100 d'un échec de transport (dist 404, dépôt injoignable sur `update` et sur `require` sans contrainte, dist local absent) ; et `--offline` contre `COMPOSER_DISABLE_NETWORK=1` sur un dépôt servi en HTTP local (cache chaud avec et sans `Last-Modified`, cache froid, et `ignore-unreachable` faux : l'avertissement du dépôt dégradé avant l'exception) : stderr entière identique (l'encadré de Composer contre `Error: <message>`)
harness/download-lines.sh                # les lignes `  - Downloading` de FileDownloader, 2 cas : cache de fichiers VIERGE par côté (stderr comparée sans aucun filtre) et cache chaud des deux côtés (personne n'imprime). Fichier à part parce que les harnais qui comparent la stderr partagent un cache : Composer, qui passe en premier, le réchauffe — c'est cette asymétrie que leurs filtres `^  - Downloading ` compensent, pas un manque de notre part. Réseau requis (deux petites archives)
harness/flex-install.sh                  # symfony/flex à l'install : .env depuis .env.dist (5 cas), symfony-pack sans suffixe, flex hors require, --no-plugins — projet ET stderr complète identiques
harness/plugin-lines.sh                  # lignes des installeurs de plugins et leur ordre, 118 cas (Packagist en ligne, COMPOSER_HOME isolé) : phpstan, rector, dealerdirect seuls (install, no-op, --no-dev, retour dev, configuration perdue, chemin relatif au projet v1.2.1/v1.0.0, require/remove d'un plugin) ; les trois et c3 (vendor vierge, no-op, poids de PackageSorter, --no-autoloader, -q, -v rendu, --no-dev, mise à jour de plugin, --no-plugins, update) ; replis nommés (versions non lues, extension aliasée, core-project-message) ; remove d'un plugin encore requis ; package-versions et bamarni (ordre à POST_AUTOLOAD_DUMP) ; depuis le 2026-10-22, `update`/`require`/`remove --no-install` (72 lignes dont 18 installs de départ, docs/plans/no-install-listeners.md) : les écouteurs `POST_UPDATE_CMD` sur le dépôt local — phpstan, dealerdirect, c3, rector, les quatre (ordre), Flex à son rang, yii2-composer (`activate`), vendor vierge, plugin ajouté/retiré, -q, --no-dev, --no-plugins, --dry-run, `config.lock: false`, vendor-dir, --working-dir, insoluble ; replis (versions non lues, plugin global, php-http/discovery, composer-patches) — code, projet entier et stdout+stderr fusionnées
harness/dealerdirect-07.sh               # dealerdirect v0.7.2 (docs/plans/chamilo-dealerdirect.md), 35 cas : la recherche dans tout le projet (vendor/ compris : une bibliothèque à ruleset.xml), racine (`../../../`), imbriqué à la borne, profondeur 4, dossiers à point (relatifs à chaque racine de recherche : projet sous un dossier à point, `vendor-dir: .vendor`), liens de répertoire non suivis (paquet `path` lié : lu comme racine), lien pendant, VCS, ruleset/répertoire retirés, conf perdue, --no-dev, update, update --no-install, --no-plugins, retrait d'un paquet et du plugin, `delete`, search-depth 4, phpcs 4, passage 0.7.2 ↔ 1.2.1 (install et --no-install), 1.2.1 à racine standard (même Finder) ; rendus : -v, v0.7.1, search-depth en chaîne, phpcs 2 (0.7.2, 1.1.2), bin-dir déjà dans le PATH après un autre phpcs — code, projet entier et stdout+stderr fusionnées, sous PHP 8.4 et 8.5
harness/allow-plugins.sh                 # `allow-plugins` au chargement des plugins et php-http/discovery après un update (2026-10-27, docs/plans/plugin-allow-discovery.md), 58 cas (Packagist en ligne, COMPOSER_HOME isolé) : refus (install, update, --no-install, --dry-run, require, remove édité d'abord, dump, -q, GitHub Actions, lock d'avant l'API 2.2.0, transitif, global), répétitions de Factory, avertissement « not loaded » (une ou deux fois), garde-fous (--no-plugins, false, plugin-optional, package-versions, sans composer.json global), rendus (vendor vierge, require qui rend requis, discovery qui agirait) — texte de l'encadré contre vivacity, sinon projet et sorties entières
harness/package-versions.sh              # composer/package-versions-deprecated : Versions.php régénéré depuis le lock (install, no-op, --no-dev, retour dev, dump, allow-plugins false, --no-plugins) — projet identique, mode 0664 compris
harness/git-sources.sh [cas...]          # paquets sans dist servis par git (2026-10-28, docs/plans/git-sources.md), 25 cas : dépôt nu fabriqué par le banc, en chemin local (absolu, relatif, `file://`) et en `https://` réécrit vers lui par `insteadOf` (cache VCS sans réseau, domaine GitHub et son `pushurl`) ; tag, sha, référence nommée ; cache froid PAR CÔTÉ au même chemin (miroirs comparés), chaud, rassis (objets libres réempaquetés), resynchronisation ; -v, -q, no-op, mixte zip + sources (ordre Downloading/Syncing), --no-dev, dump -o, métapaquet, deux paquets sur un miroir ; replis sans rien écrire (réseau coupé, référence absente locale et distante, paquet source retiré) ; un cas réel github.com (krixon, shaarli ; `GIT_SOURCES_OFFLINE=1` le saute) — projet entier, `.git` compris, stdout/stderr entières, code
harness/vendor-dir.sh [variante...]      # config.vendor-dir / bin-dir : 6 variantes (symfony ×5, wordpress ×1) — install, dump -o, no-op ; projet entier comparé, lignes « Skipped installation of bin » comparées
harness/path-repos.sh                    # dépôts `path` (14 étapes, dont 4 sur la ligne « Generating [optimized] autoload files ») : install (liens), install à vide, update après édition, remove, require, changement d'options (liens → miroirs → liens absolus), lien supprimé à la main, install en miroir — stderr, lock, vendor/ (diff + inventaire modes/liens + mtime des miroirs)
harness/verbosity.sh                     # stdout et stderr entiers sous normal, -q et -v (296 cas : 212 sur instantanés figés, dont 49 sans COMPOSER_ROOT_VERSION : l'avertissement de version racine ; `@files:` compare des fichiers écrits sans être dits, `platform_check.php` ; puis 84 `online`, plugins actifs, Packagist en ligne et fixtures/projects/laravel-pest : lignes `> événement: classe->méthode` sous -v, sortie fusionnée et projet comparés, `@delegates:` pour les plugins rendus avec leur raison ; `VERBOSITY_ONLINE_ONLY=<motif>` pour un sous-ensemble, `VIVACITY_BIN` pour un autre binaire)
harness/fallback-args.sh                 # ce que le fallback transmet à Composer (50 cas, faux composer, faux `hg`) + les lignes retenues (version racine, variables `COMPOSER_IGNORE_PLATFORM_REQ(S)`, `--no-suggest`) comptées devant chaque délégation
harness/composer-cache-dir.sh            # le répertoire de cache de Composer (2026-10-29, docs/plans/action-cache.md), sans réseau : `config --global cache-dir` à l'octet (stdout, stderr, code) et l'endroit où atterrit le zip d'un dépôt `artifact`, 20 environnements (`COMPOSER_CACHE_DIR` absolu, barre finale, `~`, `~\` (valeur seulement), `$HOME`, `%HOME%`, relatif, vide, `0` ; `COMPOSER_HOME` à barre finale ou `0` ; `XDG_*` ; `~/.composer/cache` existant ; `/etc/xdg` sous Linux root ; `COMPOSER_CACHE_FILES_DIR` ; `cache-files-dir` du projet ; `COMPOSER_AUTH` à jetons) + un relatif sous `--working-dir` ; les cas rendus vérifient la délégation
harness/no-php.sh [cas...]               # `update`/`require`/`remove`/`install` sans lock SANS PHP sous `--ignore-platform-reqs` (2026-10-29, docs/plans/update-without-php.md), 42 cas : vivacity avec `VIVACITY_PHP=/nonexistent` contre Composer avec son PHP, registre `file://` synthétique dont les paquets exigent ce qu'aucun PHP ne satisfait (php >=99, ext-nope, lib-nope, conflit sur ext-json chargée ; l'oracle vérifie que Composer refuse sans le drapeau) — update avec et sans lock, partiels (-w, -W, --with), --no-install, --dry-run, --no-dev, require (avec, sans contrainte, --dry-run, --no-update), remove (et --dev), install sans lock, config.platform, les suggestions de noms plateforme (extension qu'aucun PHP ne charge, rendue connue, désactivée par `false` : identiques ; php-64bit et ext-json, que le PHP de Composer fournit : seule la ligne du compte diffère, de exactement 2, le nombre vérifié par `composer show --platform`), COMPOSER_IGNORE_PLATFORM_REQS, insoluble, les avertissements de PoolBuilder avant « Updating dependencies » (`update acme/nothing`, `require composer-plugin-api`, `require ext-foo:^1`), `remove --no-update` sans drapeau, ni VIVACITY_PHP ni php dans le PATH : code, stdout, stderr, projet entier ; refus nommés (code 1 sans le drapeau, liste partielle, config.platform.php seul, remove sans édition ; code 3 pour require ext-foo/php sans contrainte, replace d'extension, --with sur ext-json, -v, -vv, -v --no-fallback), un faux `composer` en tête du PATH jamais lancé ; composer/installers 1.12.0 en ligne (`NO_PHP_OFFLINE=1` le saute) : sortie de Composer moins ses deux lignes de dépréciation. Sous PHP 8.4 : `PATH=/opt/homebrew/opt/php@8.4/bin:$PATH`
tools/snapshot-packagist.sh <fixture>    # (re)capture un instantané Packagist + lock de référence
harness/boot.sh                          # les 6 fixtures démarrent sur un vendor 100 % vivacity
harness/drift-reference.sh [phar]        # docs/reference/ == fichiers du phar (2.10.3 ou autre) ; une dérive lue et tranchée s'acquitte dans docs/reference/DRIFT-ACK (clé = empreinte du diff, donc un nouveau mouvement amont réalerte), le script imprimant la ligne à coller
php tools/gen-installers-table.php /tmp/composer.phar [src] [tag]   # régénère assets/installers/<tag>.json
harness/linux.sh                         # toute la chaîne dans un conteneur Linux (Docker)
tools/linux-release.sh build <triple>    # binaire Linux de release : glibc 2.17 (`<triple>.2.17`) ou musl statique, par cargo-zigbuild (tools/zigbuild-requirements.txt, épinglé par hachage, dans un venv) — marche depuis macOS (`rustup target add <triple>`)
tools/linux-release.sh check <triple> [bin]  # plancher : aucune GLIBC_ > 2.17 dans les références de version (faibles comprises), NEEDED de base / statique ; puis dans centos:7 (gnu) ou alpine (musl) : `--version` à stderr vide, `-vv` rendu à un faux composer (processus fils, code 7 relu), vrai `install` de fixtures/projects/scripts (réseau). Sous macOS : `OBJDUMP=/usr/bin/objdump`, Docker requis
bench/profile.sh ; bench/spike-vs-composer.sh   # M0, longs
```

Les tests intégration `tests/oracle_*.rs` et `tests/fixtures_*.rs` copient le
binaire `composer` du PATH dans `$TMPDIR` (`tools/oracle_phar.rs`, une copie par
identité de Composer : une mise à jour est prise en compte) et
appellent ses classes via `php -r`. Pour tout rejouer contre un autre Composer
(un snapshot, une version candidate) sans toucher à celui installé : un lien
`composer` vers son phar en tête du `PATH`, plus `VIVACITY_HARNESS_DIR` et
`VIVACITY_CACHE_DIR` à part — recette complète dans
`docs/plans/repin-composer-2.11.md`. Sans php/composer ils ÉCHOUENT avec un
message explicite (jamais de skip silencieux).

`tests/oracle_zip.rs` est l'exception : son oracle n'est pas le phar mais
`unzip -qq`, la commande que `ZipDownloader::extractWithSystemUnzip` construit
(sans `-o`). Il couvre les archives qu'`unzip` extrait avec un code 0 ; une
archive qui le fait sortir non nul envoie Composer dans une SECONDE extraction
par `ZipArchive` par-dessus l'arbre partiel, comportement à deux étages tranché
par la mesure dans docs/plans/v0.20-hostile-archives.md.

Pour reproduire la sortie du runner en local : `GITHUB_ACTIONS=true
harness/<banc>.sh`. Composer imprime six lignes de débogage en plus dans son
repli `unzip`→`ZipArchive` quand cette variable est posée, et
`GithubActionError` ajoute ses annotations — c'est ce qui a fait tomber
`two-stage.sh` sur les deux runners alors qu'il passait ici. vivacity les
écrit aussi depuis le 2026-10-16 (`vivacity_core::io::github_action_error`,
même condition : `GITHUB_ACTIONS` vrai, `COMPOSER_TESTS_ARE_RUNNING` faux)
aux trois points du solveur et pour les exceptions dont il reproduit le
message (refus de `RootPackageLoader`, lock sans `packages-dev`) ; ses
propres `Error: …` n'en ont pas. Les bancs qui
comparent la stderr d'un repli posent `COMPOSER_TESTS_ARE_RUNNING=1` côté
référence, l'interrupteur que Composer fournit pour ça.

⚠️ Les runners suivent **stable**, donc une nouvelle version de Rust peut rendre
`-D warnings` rouge sur du code qui n'a pas bougé : `std::usize::MAX`, déprécié
en 1.99, a cassé le job `parity` le 2026-10-03 sur un commit qui ne touchait pas
à `vivacity-pcre2-sys`. Garder la chaîne locale à jour (`rustup update stable`)
pour que les gates d'ici soient ceux de là-bas.

Le code Windows se **typecheck localement** sans attendre le runner (une heure
par aller-retour) : `brew install mingw-w64` puis
`rustup target add x86_64-pc-windows-gnu`, et ensuite
`cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings`
(~15 s à chaud). Ça ne remplace pas l'exécution — un test qui mesure le vrai
7-Zip du runner ne peut tourner que là-bas — mais ça attrape les erreurs de
compilation et les lints propres à `#[cfg(windows)]`, qui sont la cause de
presque tous les rouges de ce job.

⚠️ Le store porte un segment de disposition (`<store>/v3/<vendor>/<pkg>/<version>-<ref12>`,
`store.rs`, `LAYOUT`) : un changement de sémantique d'extraction le fait
monter (v3, 2026-10-27 : la racine d'une entrée prend le mode du répertoire
de tête de l'archive, et le clone recopie les modes des répertoires) ;
sinon une entrée extraite avant continue d'être servie.

## Publication sur crates.io (dernière : 0.25.0 le 2026-10-28)

Six crates (`vivacity-pcre2-sys`, `vivacity-pcre2`, `vivacity-core`,
`vivacity-autoload`, `vivacity-resolver`, `vivacity`) s'empaquettent
(`cargo package --workspace --no-verify` ; `pcre2-link-test` est
`publish = false`). Procédure, dans l'ordre des dépendances, gérée par
cargo :

```bash
cargo login                       # une fois, jeton du compte crates.io du mainteneur
# bumper la version : [workspace.package].version, les dépendances internes
# `path = "../vivacity-*", version = "…"` (crates/vivacity*/Cargo.toml, la
# dépendance pcre2 de Cargo.toml et de crates/vivacity-pcre2), le pin de
# README.md (`uses: Adelagric/vivacity@vX`, `vivacity = { version = "X" …`),
# action.yml, l'en-tête CHANGELOG — release.yml vérifie la cohérence.
cargo publish --workspace         # pcre2-sys → pcre2 → core → autoload → resolver → vivacity
```

Vérifié après la 0.25.0 (comme après chaque version depuis la 0.13.0) : binaire GitHub (checksum, `--version`, aucune
PCRE2 dynamique, symboles `vivacity_pcre2_*`), `cargo install vivacity
--version 0.25.0` depuis crates.io, install réel avec ce binaire (Laravel, 109
paquets, `--no-plugins`). Le job publish de release.yml
(`softprops/action-gh-release`) a échoué une fois à mi-téléversement
(« Error creating asset temp dir », aléa de l'action) : `gh run rerun
<id> --failed` relance le seul job publish, les artefacts de build sont
conservés et `overwrite_files: true` complète la release. Un job de
publication crates.io dans release.yml demanderait un secret
`CARGO_REGISTRY_TOKEN` (décision du mainteneur). Une fois publiés,
`cargo install vivacity` et l'embarquement (`vivacity = "0.13"` →
`vivacity::run(args)`) ne passent plus par git.

## Jalons

| jalon | état | preuve |
|---|---|---|
| M0 fixtures + profil + spike | terminé | bench/M0-profil.md |
| M1 manifestes, content-hash, scope, platform | terminé | oracle golden + différentiels + proptest, 0 divergence |
| M2 fetch + store + clone + état + proxies + CLI | terminé (`--no-autoloader`) | harness/diff-vendor.sh : 0 diff × 3 fixtures ; bench/M2-install.md |
| M3 autoload (normal, -o, -a, --no-dev) | terminé | harness --with-autoloader : 0 diff × 3 fixtures ; oracle classmap ~50k fichiers ; bench/M3-autoload.md |
| M4 harness (parité + boot), Linux en conteneur, CI GitHub Actions | terminé : chaîne verte dans le conteneur Linux arm64 (copie ET hardlinks) ET sur GitHub Actions ubuntu-latest x86_64 + macos-latest (run #2, 2026-09-10) ; réseau réel exercé | bench/M4-linux.md, harness/*.sh, .github/workflows/ci.yml |
| M5 perf classmap | terminé (détection parallèle + cache par entrée de store) ; benchmarks publiables à consolider en M6 | bench/M5-perf.md ; harness 0 diff cache froid/chaud |
| M7 coût d'un artefact de cache CI (zips vs store) | terminé 2026-09-16 : le store en artefact ne paie pas (+2,4 s pour −1,1 s), le cache CI reste les zips ; install CI 5,4 s → 2,1 s | bench/M7-ci-cache.md, bench/ci-cache.sh, .github/workflows/ci-cache.yml |
| M6 sortie publique | terminé : v0.1.0/v0.1.1 publiées, annonce r/PHP | .github/workflows/release.yml |
| v0.2 composer/installers natif, drift, action | publié (v0.2.0, 2026-09-11) | tests/oracle_installers.rs (665 cas), fixture wordpress (projet entier 0 diff), harness/removal.sh, drift.yml, action.yml + action-test.yml |
| v0.3 drupal/core-composer-scaffold natif | publié (v0.3.0, 2026-09-11), **retiré en 0.6.0** (port d'un code GPL-2.0-or-later, voir Licences) ; la fixture drupal passe désormais par le fallback `composer install` | fixture drupal (projet entier 0 diff via le fallback, boot `vendor/bin/dr`), harness/transitions.sh (scaffold présent → refus sans toucher au disque) |

| v0.4 résolveur (option A : port du solveur) | publié (v0.4.0, 2026-09-12) : `vivacity update` écrit le lock de Composer à l'octet (pool, séquence de décisions du solveur, opérations, lock) ; cache de métadonnées au format de Composer ; mises à jour partielles | docs/plans/v0.4-resolver.md, tests/oracle_pool.rs, harness/update.sh |
| v0.17 (plan) `tar` + six plugins — sur main | **corpus rejoué le 2026-09-22 : 70/105 dev, 72/105 no-dev, 0 diff, 1 unavailable (chamilo, réseau) — docs/corpus/2026-09-22.md** ; doppar ajouté après ce run, puis **corpus rejoué le 2026-09-23 : 73/106 dev, 75/106 no-dev, 0 diff — docs/corpus/2026-09-23.md** (package-versions-deprecated émulé, ibexa/post-install et license-check classés, Flex à l'install) : dists `tar` (PharData reproduit), symfony/thanks + composer-normalize bénins, yii2-composer, codeception/c3, bamarni/composer-bin-plugin, roots/wordpress-core-installer, mnsami/composer-custom-directory-installer ; deux bugs trouvés par le corpus : re-contrôle `dump-autoload` du harnais sans `--no-scripts`, et un bin `<x>.bat` déclaré purgé comme proxy périmé (0.11.0 → 0.16.0) | docs/plans/v0.17-tar-dist.md, DECISIONS 2026-09-22 |
| v0.15 perf — publié (v0.15.0, 2026-09-19) | P1 requête recouverte, P2 dump 2×, P3c dump planifié pendant l'attente, P4 un parse JSON/process ; **corpus rejoué le 2026-09-19 : 58/105 dev, 66/105 no-dev, 0 diff, 1 unavailable (chamilo, phpcs sous PHP 8.5) — docs/corpus/2026-09-19.md** ; gate CPU-bound (`--no-blocking` des deux côtés, baseline = médiane de 4 runs) | docs/plans/v0.15-perf-install.md, DECISIONS 2026-09-18/19 |
| 0.14.0 `--no-plugins` natif + sonde de plateforme en cache + gate de perf en ratio | publié (v0.14.0, 2026-09-18) : sous `--no-plugins` tout plugin est une bibliothèque (Composer n'en charge aucun) — `scope::classify_package` respecte le régime, `diff-vendor.sh` rejoue chaque fixture à plugins sous `--no-plugins` (natif exigé) ; `platform::probe` en cache (binaire php + ini chargés/scannés + répertoire de scan + PHPRC/PHP_INI_SCAN_DIR/XDEBUG_*), no-op laravel 163 → 84 ms ; `bench/gate.py` (ratio vivacity/Composer, tolérance 15 %, marge 5 ms) dans bench.yml, baseline à produire par `--merge` de plusieurs runs | CHANGELOG 0.14.0, DECISIONS 2026-09-18, bench/gate.py |
| v0.14 (plan) `--run-scripts` | publié (v0.13.0, 2026-09-17) : `crates/vivacity/src/scripts.rs` — `composer run-script [--no-dev] <event>` pour chaque événement déclaré, aux points d'`Installer::run` / `AutoloadGenerator::dump` ; opt-in, contrat inchangé par défaut ; non porté : événements par paquet, drapeau `optimize` | docs/plans/v0.14-run-scripts.md |
| v0.13 (plan) `wikimedia/composer-merge-plugin` — livré en **0.12.0** | publié (v0.12.0, 2026-09-17) — corpus 58/105 natifs en dev, 66/105 en --no-dev, 0 diff (docs/corpus/2026-09-17-v0.12.md) : `vivacity-resolver::merge_plugin` (port d'`ExtraPackage`/`PluginState` : globs PHP sans option, fusion require/autoload/links/extra, doublons structurés, `self.version`, récursion), manifeste fusionné avant l'en-tête d'`install`/`dump`, contrôle du lock sur les liens structurés (racine candidate avec ses replace/provide), **jamais la mise à jour implicite** (repli avec raison sur vendor vierge ; `update`/`require`/`remove` refusés jusqu'à la 0.16, émulés depuis — plan v0.16 C) ; harness 5 variantes 0 diff ; corpus-add conserve les fichiers inclus | docs/plans/v0.13-merge-plugin.md |
| v0.12 PCRE2 préfixé (vivacity-pcre2-sys, vivacity-pcre2) | publié dans 0.11.0 (2026-09-17) : symboles `vivacity_*` (137/137, tools/check-pcre2-symbols.sh), toujours statique, test de lien contre une PCRE2 étrangère (crates/pcre2-link-test) sur 3 OS, binaires release sans libpcre2-8 dynamique ; déclencheur ephpm/ephpm#523 — **mergé, ePHPm v0.11.0 (2026-09-18) embarque vivacity 0.11.1 (`ephpm composer`)** | docs/plans/v0.12-pcre2-prefix.md |
| v0.11 `config.vendor-dir` / `bin-dir` | publié (v0.11.0, 2026-09-17) — corpus 55/105 natifs en dev, 63/105 en --no-dev, 0 diff (docs/corpus/2026-09-16-v0.11.md) : `dirs.rs` (résolution `Config::get` : env > config projet > config globale, `{$vendor-dir}`, formes refusées = issues de layout), `Layout` porte les répertoires (chemins, `install_path` racine, proxies, autoloader, plugins émulés) ; règle de conflit de `BinaryInstaller` (fichier existant conservé + message), `removeBinaries` (proxies du paquet retiré ou mis à jour, rmdir si vide) ; harness `vendor-dir.sh` 6 variantes 0 diff ; deux parités trouvées par le corpus (alias inline du lock dans installed.php ; ordre de `files` non stable chez Composer sur un cycle — annoté) | docs/plans/v0.11-vendor-dir.md, crates/vivacity-core/src/dirs.rs, harness/vendor-dir.sh |
| v0.10 le corpus (106 projets réels, 0 diff, 56/105 natifs en --no-dev, 50/105 en dev) + 4 plugins émulés (pest-plugin, dealerdirect, phpstan/rector extension-installer) + référence plugins-on | publié (v0.10.0, 2026-09-16) : docs/corpus/2026-09-16.md, harness/corpus.sh, ScopeIssue::Config, compare.sh avec inventaire des modes | docs/plans/v0.10-corpus.md |
| v0.9 dépôts `path` + PR #4 perf + PR #5 TLS sélectionnable | publié (v0.9.0, 2026-09-16) : `path_repo.rs` (port de `PathRepository::initialize` : glob à accolades, référence `sha1(json . serialize(options))` ou HEAD du dépôt imbriqué, précédence des versions, branche de fonctionnalité + parente), `path_install.rs` (port de `PathDownloader` : lien relatif/absolu, miroir `ArchivableFilesFinder` + Symfony `copy`), `glob.rs`/`phpserialize.rs` (oracles PHP) ; fixture `path-repos` matérialisée par `fixtures/path-repos.sh` ; `harness/path-repos.sh` 10 étapes + 20 cas `steps.sh` + `update.sh` ; lignes d'opérations d'une installation réelle (`: Extracting archive`, `: Symlinking from …`, `: Source already present`) et `Generating autoload files` | docs/plans/v0.9-path-repositories.md |
| v0.8 `--dry-run` + lignes d'opérations + rapport post-update | publié (v0.8.0, 2026-09-15) : `steps.sh` compare stderr par défaut (202 cas, 180 avec stderr identique), `@dry-install` ; `RootPatch` pour `require`/`remove --dry-run` ; lock virtuel pour la phase d'installation d'un dry run ; dépôts `path` → v0.9 | docs/plans/v0.8-dry-run-path.md |
| v0.7 Windows livré + explications d'un ensemble insoluble | publié (v0.7.0, 2026-09-15) : binaire Windows dans la release, `install.sh` Git Bash + `install.ps1`, action sur windows-latest, `bin-compat` global ; `problem.rs` (Problem/Rule/SolverProblemsException), 51 cas d'explications identiques à l'octet dans `harness/steps.sh` | docs/plans/v0.7-windows-problems.md, fixtures/make-solver-problems.py, tools/package-release.sh |
| v0.5 require/remove/politiques | publié (v0.5.0, 2026-09-14, 4 crates sur crates.io) : partielles, `JsonManipulator` (12 102 + 725 scénarios vs phar), `remove`, `VersionSelector` (902 noms), `require`, politiques de blocage (54 cas) ; `harness/steps.sh` 128 cas | docs/plans/v0.5-require-remove.md, tests/oracle_json_manipulator.rs, harness/steps.sh |

## Licences (2026-09-14)

NOTICE.md liste les origines des ports et leurs licences ; les textes MIT de
Composer, composer/semver, class-map-generator, metadata-minifier et
composer/installers sont à côté des sources vendorées. L'émulation de
`drupal/core-composer-scaffold` (port d'un code GPL-2.0-or-later, œuvre
dérivée incompatible avec la distribution MIT/Apache-2.0) a été retirée en
0.6.0 avec sa source vendorée ; les versions 0.3.0 à 0.5.0 qui la
contenaient ne sont plus distribuées sur crates.io (crates `vivace*`
supprimés le 2026-09-15, ce qui libère aussi le nom pour svandragt/vivace).
Rien n'est porté d'un logiciel GPL. roots/wordpress-core-installer est
GPL-2.0-or-later (NOTICE le disait MIT jusqu'au 2026-10-28) : sa copie
vendorée a été retirée et `layout::wp_core_dir` récrite depuis le
comportement mesuré seul (`harness/wp-core.sh`, `custom-installers.sh`) ;
johnpbloch, GPL lui aussi, réutilise cette fonction. thelia/installer et
contao-components/installer (LGPL-3.0-or-later) : réimplémentés depuis le
comportement, non vendorés.

## Ce qui N'EST PAS couvert / testé (honnêtement)

- **Sans PHP sous `--ignore-platform-reqs`** (2026-10-29, docs/plans/update-without-php.md) : vérifié sous macOS seulement (harnais et test d'intégration) ; Linux par la CI, Windows jamais (le test d'intégration est `#[cfg(unix)]`, le harnais n'est pas dans le job Windows). Écart choisi (option 2, DECISIONS 2026-10-29) : le compte « N package suggestions were added… » sans PHP est un majorant de celui de Composer (laravel depuis zéro : 95 contre 49 ; symfony : 55 contre 21 ; lock et autres lignes identiques), revenu à celui de Composer quand chaque nom plateforme suggéré que le PHP fournit (php-64bit compris) est déclaré avec une version dans `config.platform`. La règle « même nom » est vérifiée sur tout le pool filtré (plus large que les paquets que le solveur atteint : un refus de trop possible, jamais un de moins).

- **Binaires Linux de release** (2026-10-29, docs/plans/linux-glibc-floor.md) : plancher glibc 2.17 (gnu) et musl statique, construits par `tools/linux-release.sh` (cargo-zigbuild) et vérifiés par lui à chaque release (quatre triples, image du plancher, binaire lancé en natif) et à chaque push (x86_64 seulement : job `installers` d'action-test.yml, plus le témoin négatif qui exige que le contrôle REFUSE le binaire v0.25.0). Le job `action` d'action-test.yml teste un binaire `cargo build` du runner (gcc, glibc 2.39), pas celui qu'on publie. Les binaires zig n'ont plus de table des symboles (zig traduit le `strip = "debuginfo"` du profil en strip total) : backtraces sans noms de fonction. Sous musl, les messages d'erreur système (`strerror`, `gai_strerror`) diffèrent de la glibc (« Operation timed out », « Name does not resolve ») — seulement dans des chemins d'erreur, où vivacity ne reproduit déjà pas le texte de curl ; aarch64-musl compile PCRE2 sans JIT, que vivacity n'active jamais. Non mesuré : l'allocateur de musl sur un runner à beaucoup de cœurs (banc fait à 4 vCPU). L'action dans un `container:` Alpine exige `bash` et `curl` dans l'image (étapes `shell: bash`, `install.sh`) ; elle n'y a jamais tourné en CI.

- **Parallélisme d'E/S** : `platform::parallel_io()` (Linux vrai, ailleurs faux, `VIVACITY_PARALLEL_IO=0|1`) gouverne les répertoires en parallèle du scan et le fan-out de la matérialisation ; mesuré sur APFS et ext4 seulement (DECISIONS 2026-09-15) — btrfs/xfs (reflink), NFS, disques lents : non mesurés. Cache de classmap en binaire v2 (`CACHE_FORMAT`), les anciens `*.json` restent orphelins dans `~/.cache/vivacity/classmap/` (jamais lus).
- **Cache de classmap** : par entrée de store (suppose vendor/ immuable entre deux installs — un fichier édité à la main dans vendor/ n'est pas rescanné) et, depuis v0.15, par fichier pour tout chemin hors store (`classmap-files/`, clé (mtime ns, taille) ; risque résiduel : un fichier réécrit dans le même tick d'horloge avec la même taille) ; `VIVACITY_NO_CLASSMAP_CACHE=1` pour désactiver. Sur un vendor/ posé par Composer, le premier `vivacity install` chauffe le store depuis le cache zip (≈1 s sur Laravel) ; les suivants profitent du cache (65 ms). `VIVACITY_TRACE=1` affiche les phases (temps cumulés ; dans le dump : inputs, jobs, scan, merge, classmap file, files, static). **Sonde de plateforme** (`vivacity_resolver::platform::probe`) : mise en cache dans `platform-probe.json` (répertoire de cache de vivacity) **seulement pour un php dont on voit ce qu'il charge** (depuis 0.21, DECISIONS 2026-10-09) : la sonde doit rapporter qu'elle est le binaire localisé, et sous unix ses images doivent avoir été listées (`/proc/self/maps` sous Linux, `DYLD_PRINT_LIBRARIES` sous macOS ; sous Windows les DLL de son répertoire et de `extension_dir`). Sinon — wrapper `#!/bin/sh`, shim de gestionnaire de versions, binaire qui ignore `DYLD_*` — la sonde tourne à chaque fois (~40 ms). Clé : un numéro de format comparé, le texte de la sonde, le binaire, chaque ini, chaque image sur disque (mtime, taille, ctime, inode), la cible du lien `<prefix>/opt/<formule>` de chaque keg Homebrew chargé, les marqueurs système (`/etc/ld.so.cache`, bases dpkg/rpm/apk/pacman, cache partagé de dyld) et dix variables d'environnement dont celles du chargeur dynamique. Chemin chaud mesuré : ~0,4 ms de plus que l'ancienne clé (54 images, 39 liens, 24 marqueurs sur ce Mac). `VIVACITY_NO_PLATFORM_CACHE=1` pour désactiver. Reste aveugle : un fichier remplacé en conservant mtime, taille, ctime et inode ; une DLL Windows chargée depuis le `PATH` hors des deux répertoires. Le no-op laravel : 163 ms → 84 ms (M4 Max).
- **Autoload, cas non exercés par les fixtures** : `target-dir` avec psr-0 racine (targetDirLoader non porté), `include-path`, `exclude-from-classmap` avec globs `**` (porté, non vérifié par diff), chemins `.phar`. `apcu-autoloader` : porté (préfixe aléatoire comme Composer, ligne ignorée par `compare.sh`), exercé par le corpus (grav, easyappointments).
- **Le corpus** (v0.10) : `fixtures/corpus/` (106 projets réels, json + lock + arbre minimal de l'autoload racine + PROVENANCE ; `tools/corpus-add.sh`, `excluded.txt`), `harness/corpus.sh` (`--scan` hors ligne via `install --check-scope --ignore-platform-reqs`, puis double install Composer `--no-scripts` — plugins actifs : `--no-scripts` ne coupe que les scripts de composer.json — contre `vivacity install --no-fallback`, `--ignore-platform-req=ext-*` des deux côtés et `php` quand cette machine échoue le lock — repéré depuis le 2026-10-21 dans le texte de Composer que `install --check-scope` imprime, « requires php … -> your php version … » ; l'ancien message de vérification de vivacity, remplacé le 2026-10-16, laissait douze projets `unavailable`), `tools/corpus-report.py` → `docs/corpus/<date>.md`. Ce que le corpus ne prouve pas : les scripts (jamais exécutés par vivacity), `update`, Windows, les plugins hors liste (fallback). Dernier passage : `docs/corpus/2026-10-29-chamilo-dealerdirect.md` (`docs/plans/chamilo-dealerdirect.md` : 95/106 natifs en `--no-dev`, 95/106 en dev, 0 diff, 0 indisponible — chamilo-lms natif dans les deux modes, sa fixture tirant ses fichiers legacy au commit épinglé ; les treize entrées à dealerdirect rejouées, les 93 autres reprises du passage précédent) ; avant lui `docs/corpus/2026-10-29-remaining-fallbacks.md` (`docs/plans/remaining-fallbacks.md` : 95/106 natifs en `--no-dev`, 94/106 en dev, 0 diff, 0 indisponible — roave-backwardcompatibilitycheck natif en dev, typo3 et concretecms dans les deux modes) ; avant lui `docs/corpus/2026-10-29-duration-drupol-collector.md` (drupol/composer-packages, `docs/plans/duration-drupol-collector.md` : 93/106 natifs en `--no-dev`, 91/106 en dev, 0 diff, 0 indisponible — bolt-project natif dans les deux modes) ; puis `docs/corpus/2026-10-28-plugins-round2.md` (après 0.25.0, les sept plugins de `docs/plans/plugins-round2.md` : 92/106 natifs en `--no-dev`, 90/106 en dev, 0 diff) ; puis `docs/corpus/2026-10-28.md` (0.25.0 : 85/106 natifs en `--no-dev`, 83/106 en dev, 0 diff ; le 2026-10-21 : 76/106 et 74/105 — installeurs personnalisés et paquets git seuls). Politique de rafraîchissement : ré-épingler les entrées une fois par an (date dans PROVENANCE et dans le nom du rapport). La liste `BENIGN_PLUGINS` avait été qualifiée contre un Composer `--no-plugins` : sous la référence du corpus (plugins actifs) `phpstan/extension-installer` et `rector/extension-installer` (GeneratedConfig.php), `pestphp/pest-plugin` (vendor/pest-plugins.json) et `dealerdirect/phpcodesniffer-composer-installer` (CodeSniffer.conf) écrivent des fichiers — **retirés de la liste** (fallback avec la raison) jusqu'à leur émulation, dans l'ordre pest-plugin, dealerdirect et les extension-installers (**tous émulés** — `crates/vivacity/src/extension_installers.rs` pour phpstan/rector : `GeneratedConfig.php`, chemins absolus (cwd physique + chemin vendor) et relatifs (`findShortestPath`), `var_export`, `Intervals::compactConstraint` ; les deux écoutent `post-install-cmd`, jamais `dump-autoload` — `phpcs_installer.rs` : recherche des `ruleset.xml` à la profondeur de Finder (0 dès phpcs 3, `extra.phpcodesniffer-search-depth`), `installed_paths` triés dans `CodeSniffer.conf` au format var_export de phpcs ; la référence *échoue* quand phpcs ne peut pas charger l'autoload du projet (« Failed to set PHP CodeSniffer installed_paths Config ») : le corpus classe ce cas `unavailable`, pas `diff` — sur chamilo, c'étaient les `autoload.files` de la fixture qui manquaient, pas phpcs sous PHP 8.5 ; la fixture les tire depuis le 2026-10-29, `UPSTREAM`) ; pest-plugin : `pest_plugin.rs`, `vendor/pest-plugins.json` au dump de l'autoloader, ordre d'opérations — celui de Composer est l'ordre d'achèvement de ses extractions parallèles, non reproductible, comparé trié par les harnais) → dealerdirect → extension-installer. Depuis, la référence des fixtures est elle aussi à plugins actifs dès que le manifeste en autorise un (`diff-vendor.sh`/`removal.sh` : laravel, symfony, wordpress natifs ; sylius, rector, drupal rendus à Composer, le harness l'affiche) ; le fallback transmet la ligne de commande reçue (moins `--no-fallback`, `--check-scope`, `--offline` devenu `COMPOSER_DISABLE_NETWORK=1`, `--run-scripts`), avec `--no-scripts` sauf `--run-scripts` (contrat de vivacity), en processus fils dans le répertoire de vivacity — jamais `exec` — ; la phase d'installation d'un `update`/`require`/`remove` transmet une ligne `composer install` construite depuis ses options (`after_update_line`, `harness/fallback-args.sh`) ; `autoload_runtime.php` est écrit au dump de l'autoloader (jamais avec `--no-autoloader`), comme le plugin.

- **Recettes de symfony/flex** (2026-10-16, `docs/plans/flex-configurators.md`) : `update` avec install applique nativement les recettes (`bundles` et recette auto-générée, `copy-from-recipe`, `env`, `dotenv`, `gitignore`, `add-lines`, `composer-scripts` avec la réécriture du lock par `updateComposerLock`, Docker seulement à `extra.symfony.docker: false`), décidées sur un système de fichiers virtuel avant l'écriture du lock puis appliquées à `POST_UPDATE_CMD` (états vérifiés). Replis nommés : `container`, `makefile`, `composer-commands`, `copy-from-package`, Docker activé ou non décidé, contrib sans `allow-contrib`, `%generate(secret)%` écrit, conflit de recette qui correspond, `root-dir` ≠ `.`, chemin regardé dans une zone que l'install écrit (bin-dir, paquets hors vendor/, `c3.php`), endpoint `api.github.com` (donc toutes les recettes de Sylius) ou `file://`, `--offline` avec une recette à télécharger, désinstallation d'un paquet que l'index liste, `SYMFONY_LOCKFILE`, version de `symfony.lock` non normalisable, `config/bundles.php` qui n'est pas dans la forme de `buildContents`, message `add-lines` de niveau normal contenant `<` ou `\`. `require` et `remove` avec install restent rendus à Composer. Non exercés par un harnais : le téléchargeur désactivé avec Flex actif (un Flex global ; un `update` qui retire symfony/flex des exigences le désinstalle), le séparateur `\` de `getLocalFilePath` sous Windows (porté), deux chemins de casse différente dans un même lot sur un système de fichiers insensible à la casse. L'index servi s'arrête à 7.4 pour symfony/framework-bundle (PROVENANCE).
- **Linux** : exercé en conteneur arm64 (php:8.4) et sur runner GitHub x86_64 (ubuntu-latest) — gates, tests, parité, boot ; copie et hardlinks exercés en conteneur. Perf Linux mesurée en runs uniques seulement (pas d'hyperfine sur runner).
- **Réseau réel** : exercé une fois (109 zips GitHub via rustls, caches vides, 3,46 s, app boote) ; retries/backoff jamais exercés en conditions réelles. Auth : les en-têtes comparés à ceux de Composer par `harness/auth-hosts.sh` (mandataire local, dists `http://`) ; en CI, chaque job `parity` passe le jeton du job par `COMPOSER_AUTH` aux deux côtés (2026-10-29). Non portés : `secure-http` (vivacity télécharge un dist `http://` que Composer refuse — avec ses identifiants s'il y en a pour cette origine ; et n'écrit pas « Warning: Accessing <hôte> over http », que les harnais filtrent), l'avertissement « avoid overwriting already defined auth settings » (deux types d'auth sur une origine : le gagnant est le même que chez Composer, la ligne manque), les clés d'auth posées dans `config` de composer.json ou du config.json global ; les jetons GitLab, `x-token-auth` de Bitbucket, `custom-headers`, `client-certificate` (rendus en Basic) ; une redirection vers une origine qui a ses propres identifiants (Composer les ajoute, reqwest ne fait que retirer l'en-tête entre hôtes) ; la casse (Composer compare hôtes et clés tels qu'écrits, vivacity les met en minuscules) ; un COMPOSER_AUTH mal formé est rendu à Composer (qui s'arrête) par la résolution du répertoire de cache. `gitlab-token`/`gitlab-oauth` non implémentées (github-oauth, http-basic, bearer seulement). rustls n'utilise pas le magasin de CA système (`SSL_CERT_FILE` ignoré).
- **Version du root package** : portée (VersionGuesser git : branche, HEAD détaché, tag exact, branche de feature → parente, branch-alias ; COMPOSER_ROOT_VERSION). Le harness commite un dépôt git identique des deux côtés : parité vérifiée sur `main` et, via la fixture path-repos, sur `develop` et une branche de fonctionnalité d'un dépôt imbriqué. Depuis 0.9, git n'est plus épinglé sur `<projet>/.git` : il remonte les répertoires comme chez Composer (un projet dans un dépôt parent prend la branche de ce dépôt — les harnais fixent `GIT_CEILING_DIRECTORIES` au parent du projet pour que le checkout de vivacity ne fuie jamais). `non-feature-branches` est une liste de motifs regex (`release-.*`), comme chez Composer. Non exercés par le harness : HEAD détaché (dont `(HEAD detached from X)`, qui ne donne aucune version — test unitaire), alias de la racine (tests unitaires seulement) ; hg/fossil/svn non portés : là où l'un d'eux répondrait (une copie de travail `.hg`, `_FOSSIL_`/`.fslckout` ou `.svn` au-dessus du projet, git sans version, et `hg branch`, `fossil branch|tag list` ou `svn info --xml` qui sort en 0 — rien n'est lancé sans copie de travail), la commande entière va à Composer avant toute lecture (racine sans `version` ni `COMPOSER_ROOT_VERSION`, tout type, tout niveau), ou la résolution rend la main (paquet `path` sans version) ; binaire absent ou en échec : natif, comme Composer qui retombe alors sur `1.0.0` (2026-10-16, `harness/fallback-args.sh` section h). **L'avertissement** « Composer could not detect the root package (…) version » (2026-10-16, `docs/plans/root-version-warning.md`) : écrit comme `RootPackageLoader::load` — racine nommée, pas `project`, ni version ni variable ni version devinée — en tête de la commande, au niveau normal, deux fois pour `require`/`remove` (seconde création du Composer) ; retenu par `vivacity_core::io` jusqu'à la première ligne de vivacity, jeté si la commande est rendue à Composer avant (Composer l'écrit). Comparé par `harness/verbosity.sh` (`@unset:COMPOSER_ROOT_VERSION`, 17 cas) et compté devant chaque sorte de délégation par `harness/fallback-args.sh` (section g) ; partout ailleurs les harnais gardent `COMPOSER_ROOT_VERSION`. Les avertissements des variables `COMPOSER_IGNORE_PLATFORM_REQ(S)` (`BaseCommand::initialize`, cinq commandes, `dump-autoload` compris depuis le 2026-10-16) et la dépréciation `--no-suggest` sont retenus avec elle, dans l'ordre de Composer (`docs/plans/install-defects.md`, point 3). Écarts : doublon sur une délégation tardive (portée d'`install`, Flex après résolution, phase d'installation rendue à `composer install`) ; une ligne de plus par `composer run-script` sous `--run-scripts` ; trois lignes chez Composer pour un manifeste refusé après elle, une chez vivacity ; composer.json global non lu. Depuis le 2026-10-16, git ne répond qu'une fois par question et par exécution (`root_version::git`, cache vidé par `vivacity::run`) : le `git branch` de l'avertissement sert aussi à la version et à installed.php.
- **Paquets sans dist servis par git** (2026-10-28, `docs/plans/git-sources.md`) : `crates/vivacity-core/src/git_source.rs` lance le binaire `git` avec les commandes de `GitDownloader` (miroir du cache VCS, clone `--dissociate --reference`, `updateToCommit`, `pushurl` GitHub), tout ce qui peut échouer avant la première écriture (répertoire de travail `vendor/.vivacity-git-*`, puis `rename`). `harness/lib/compare.sh` compare désormais les `.git` SOUS le vendor : à l'octet sauf `index` (`ls-files --stage` + `status`), l'horodatage des reflogs et le rangement des objets (ensemble des objets ; les packs de `--dissociate` ne sont pas reproductibles) — un banc qui compare un projet où vivacity et Composer clonent doit donc donner aux deux le même chemin de cache VCS. Non portés (rendus, nommés) : autres VCS, `http`/`git://`/ssh/identifiants, cache ou protocoles GitHub configurés, réseau coupé, mise à jour ou suppression d'un paquet installé depuis la source, chemin d'installation hors du vendor, Windows, `--prefer-source`/`preferred-install: source` pour un paquet qui a une dist. Laissés : un dépassement de `process-timeout` ne tue que `git` (pas `git-remote-https`) ; un `vendor/.vivacity-git-*` survit à un arrêt brutal. Corpus (six entrées rejouées le 2026-10-28) : ampache et kovah-linkace natives, 0 diff.
- **Dépôts `path`** (v0.9) : portés et comparés à l'octet (lock, stderr, vendor/ avec modes et cibles des liens) sur Linux/macOS. Sous Windows, un lock avec un paquet `path` part en fallback `composer install` (jonctions non portées ; `update`/`require`/`remove` fonctionnent, le lock est le même). Non portés : le repli « Symlink failed, fallback to use mirroring! » (un `symlink()` qui échoue est une erreur), les guessers hg/fossil/svn d'un paquet sans dépôt git (depuis le 2026-10-16, la résolution rend la main quand l'un d'eux répondrait), `GLOB_BRACE` absent sous musl (Composer y refuse les accolades, vivacity les développe), l'ordre des correspondances *entre* alternatives d'accolades dépend de la build de PHP (le PHP 8.4 du runner macOS trie globalement, PHP 8.5 sur macOS et la glibc par alternative — vivacity suit la glibc ; seul l'ordre d'`addPackage` entre alternatives en dépend, l'oracle unitaire compare ces motifs comme des ensembles), `Url::sanitize` dans le nom du dépôt, le message « source is still present » n'est atteint que si le chemin d'installation *est* la source. Règles du miroir vérifiées par la fixture : dossiers VCS exclus à toute profondeur (dossiers seulement, un fichier `.git` est copié), `.gitattributes` de la racine (`export-ignore`/`-export-ignore`, lignes à exactement deux champs), lien vers un fichier ou un dossier vide recréé avec sa cible brute, lien vers un dossier non vide / pendant / sortant abandonné, dossier vide gardé, un dossier exclu par `export-ignore` est quand même parcouru (un enfant `-export-ignore` est gardé, `docs/keep.md`), fichier copié en `0666 & ~umask | bits x de la source` avec le mtime de la source à la seconde (invariant vérifié par `harness/path-repos.sh` de chaque côté : le mtime d'un fichier en miroir = celui de sa source). Une mise à jour ou une suppression crée `vendor/bin` même sans binaire (`BinaryInstaller::removeBinaries`), comme Composer — pas une réinstallation après purge (`Factory::purgePackages` : un chemin d'installation disparu = paquet absent d'installed.json). **Dépôt local** (revue v0.9) : installed.json, installed.php et l'autoloader sont produits depuis le dépôt local — l'entrée d'installed.json pour un paquet inchangé (même version, même référence, répertoire présent), l'entrée du lock pour un paquet installé ou mis à jour — comme `InstalledFilesystemRepository::write` et `AutoloadGenerator::dump($localRepo)` : un lock qui change les métadonnées d'un paquet sans changer son identité (édition non commitée d'un paquet `path` à référence HEAD ou `none`, options `symlink`/`relative` d'un tel paquet) laisse installed.json et l'autoloader tels que Composer les laisse. Non portés : un fichier ordinaire au chemin d'installation (Composer le laisse, `symlink()` échoue, repli miroir qui échoue ; vivacity le remplace), `is_readable` (Composer) vs « répertoire » (vivacity) pour décider qu'un paquet est installé, la forme encadrée des erreurs (`In PathRepository.php line 163:` + synopsis — vivacity imprime le texte seul, code identique), le nom du miroir dans `Failed loading the package` est le chemin réel (`/private/tmp/…` sous macOS, comme Composer) mais le message d'un JSON invalide est celui de serde, pas de jsonlint ; une FIFO dans une source bloque la copie ; les entrées non UTF-8 d'un glob sont ignorées (PHP les matche).
- **Alias des paquets verrouillés** : port de `ArrayLoader::getBranchAlias` (`branch-alias` + `default-branch`), oracle de 31 cas contre le phar ; exercé par le harness via la fixture rector (`dev-main` + `default-branch`). Non exercé par diff : `extra.branch-alias` sur un paquet verrouillé en dev (oracle seulement).
- **`symfony/runtime`** : `autoload_runtime.php` est généré depuis le gabarit du paquet installé (`Internal/autoload_runtime.template`, ou `extra.runtime.autoload_template`) avec `extra.runtime` substitué (`var_export` porté, `project_dir` en `dirname(__DIR__, n)`), comme `ComposerPlugin::updateAutoloadFile` ; `extra.runtime: false` n'écrit rien. Un `symfony-pack` n'est installé nulle part quand Flex est verrouillé et autorisé (`SymfonyPackInstaller`).
- **Politiques de blocage** : portées (avis, liste malware, abandonnés) ; non portés : les listes personnalisées avec sources (refusées quand un dépôt les annonce) et un dépôt avec `filter.api-url` (refusé) ; le message d'échec d'`install` sur un lock signalé ne liste que les versions retirées, pas les problèmes des paquets qui en dépendent (Composer rejoue le solveur ; code 2 identique) ; le code retour d'une source injoignable avec `ignore-unreachable: false` est 1 (Composer 100) ; `vivacity install` fait désormais un aller-retour réseau (packages.json des dépôts, résumé des listes) comme Composer, ignoré avec avertissement hors ligne ; `bump-after-update` (config ou option) n'est pas exécuté par `vivacity update` (Sylius l'a dans sa config : Composer réécrit composer.json après l'update, vivacity non) ; `--with`, `update lock/nothing/mirrors`, `--minimal-changes`, les dépôts `vcs` : non portés (les dépôts `path` le sont depuis 0.9).
- **`config.lock: false` et `install` sans lock** (2026-10-16, `docs/plans/install-defects.md`, point 1) : la ligne de `Factory` (« ./composer.lock is present but ignored as the "lock" config option is disabled. ») retenue après celle de version racine pour les cinq commandes, deux fois pour `require`/`remove` ; le lock ignoré par la résolution (`UpdateOptions::ignore_lock` : ni dépôt verrouillé, ni mise à jour partielle pour `require`/`remove`, `remove --unused` refusé, `update a/b` en 3) ; l'installation qui suit lit le lock résolu en mémoire (`Resolved::virtual_lock`), jamais celui du disque ; `install` sans lock utilisable (désactivé, absent, sans `packages`) écrit la ligne d'`Installer::run` et passe par `resolve_and_lock` avec ses seules options (`--check-scope` et `--run-scripts` : 3 ou Composer). `dump-autoload` sans lock à lire (désactivé ou absent) prend installed.json comme source de sa disposition, le dépôt local de Composer (revue). Reste : une délégation tardive rend un `composer install` qui résout à nouveau.
- **Sortie des commandes** (v0.8) : lignes d'opérations, suggestions, abandonnés, funding, `install` (`Installing dependencies…`, `Verifying…`, `Package operations`) et `--dry-run` portés et comparés à l'octet ; non couverts : les lignes `  - Downloading …` d'une installation réelle (vivacity télécharge sans les imprimer ; les lignes `  - Installing … : Extracting archive` et `Generating autoload files` sont imprimées depuis 0.9, après la transaction — les placements sont parallèles — et vivacity garde sa ligne de résumé `vivacity: …`, tolérée par les harnais), l'audit (`--no-audit` est le comportement comparé), le filtre des suggestions utilise la plateforme *avec* `config.platform` (Composer lit `platform-overrides` du lock = la même chose). Portés après revue : `getMissingRequirementInfo` (exigence racine absente du lock → code 4, texte identique), la vérification du lock de `doInstall` (2026-10-16 : une résolution sur le pool [racine figée, plateforme sous les `platform-overrides` du lock, lock], exigences de plateforme de la racine puis du lock, filtre de liste, `--ignore-platform-req(s)` et `COMPOSER_IGNORE_PLATFORM_REQ(S)` ; « Your lock file does not contain a compatible set… » et les problèmes de Composer, code 2, ou « cannot be installed on this system without changes », code 4 — atteint par un paquet verrouillé que la stabilité du lock refuse ; sur le fil de l'attente réseau, rien après un `update` ; sans php, avertissement gardé et résolution sans php/extensions/bibliothèques — `docs/plans/install-verify.md`), la purge d'installed.json (`Factory::purgePackages` : un chemin d'installation disparu = paquet absent, pour la transaction et le funding), la ligne funding en dry run aussi (`COMPOSER_FUND=0` la coupe), la restauration de composer.json par `require` quand la phase d'installation échoue (`revertComposerFile` sur tout statut non nul). Un dry run reste natif même sur un lock hors portée (la liste d'opérations vient du lock, identique à Composer) ; les fallbacks atteints avant son retour (plugin de fusion) transmettent `--dry-run`, jamais une installation réelle (2026-10-16).
- **Ordre et lignes des écouteurs de plugins** (2026-10-16, `docs/plans/install-order.md` ; 2026-10-21, `docs/plans/plugin-lines.md`) : `COMMAND`, funding puis `POST_*_CMD` puis scripts suivent `Installer::run` sur `install`/`update`/`require`/`remove` ; **imprimées depuis le 2026-10-21** : les lignes stdout de phpstan/extension-installer (« Extensions installed » à chaque install, `installed`/`not supported`/`ignored` quand GeneratedConfig.php change), rector/extension-installer (quand le fichier change), dealerdirect (« set to » / « delete » quand il sauve) et package-versions-deprecated (deux lignes à chaque dump) ; les émulations jouent dans l'**ordre d'activation de Composer** (`crates/vivacity/src/plugin_order.rs` : installed.json tel qu'écrit trié par `PackageSorter` avec les poids de `loadRepository`, global ensuite, puis la transaction — installé ou mis à jour en fin, désinstallé retiré), à `POST_*_CMD` (dealerdirect, phpstan, rector, c3, Flex) et à `POST_AUTOLOAD_DUMP` (bamarni, package-versions) ; versions liées à celles lues (`scope::EMULATED_VERSIONS`, repli nommé sinon, dès la résolution pour `update`/`require`/`remove`), paquet listé et aliasé → repli, drupal/core-project-message avec message → repli ; `remove` garde les `allow-plugins` lus au chargement (`allow_plugins_at_load`, ordre des règles compris). **Sans installation** (2026-10-22, `docs/plans/no-install-listeners.md`) : `update`/`require`/`remove --no-install` jouent les écouteurs `POST_UPDATE_CMD` que Composer dispatche encore (dealerdirect, phpstan, rector, c3, Flex à son rang) sur le dépôt local — installed.json purgé, dev compris, ordre d'activation sans transaction (`crates/vivacity/src/no_install.rs`) — et `activate` de yii2-composer (extensions.php, dry run compris) ; rendus, nommés, avant toute écriture : version non lue des trois installeurs, l'un d'eux installé globalement, plugin actif ni émulé ni bénin (composer-patches, installers-extender), disposition non calculable, php-http/discovery dont `postUpdate` agirait (son `getMissingRequires` porté, 1.19.0–1.20.0, `crates/vivacity/src/discovery.rs` ; 1.15.0–1.18.1 : dès qu'il surveille une abstraction PSR ; il édite composer.json et relance l'update, sans fin mesurée). Restent : `-v` rendu à Composer pour les plugins dont les lignes verbeuses ne sont pas reproduites (Flex, dealerdirect, composer-merge-plugin, yii2-composer, symfony/thanks, license-check, tout plugin hors table, une version non lue, un plugin global) — les autres écrivent leurs lignes `> <événement>: <classe>-><méthode>` depuis le 2026-10-22 (voir « `-v` et les plugins ») ; sous `--run-scripts`, `composer run-script` rejoue les écouteurs des plugins (c3 « already up-to-date » en plus, mesuré ; bamarni à `post-autoload-dump`) et `pre-install-cmd` tourne avant la décision de portée ; dealerdirect quand phpcs ne tourne pas sous le PHP de la machine (« Failed to set … » chez Composer) ; plugins émulés installés globalement (ordre calculé, émulation absente ; rendus sans installation) ; `forward-command: true` sur `update` non renvoyé ; dealerdirect v1.1.x sans installation (installed.php non purgé).
- **`-v` et les plugins** (2026-10-22, `docs/plans/verbose-plugins.md`) : `crates/vivacity/src/verbose_listeners.rs` porte la table des plugins dont vivacity écrit la ligne `>` de chaque écouteur (EventDispatcher.php:230) aux événements qu'il place — `command` (bamarni, même sous `--dry-run`), `pre-autoload-dump`, `post-autoload-dump`, `post-install-cmd`/`post-update-cmd` (aussi sous `--no-install`, jamais sous `--dry-run`) — dans l'ordre d'activation, avec le suffixe `_composer_tmpN` de `PluginManager::registerPackage` (rechargement de `require`/`remove`, mise à jour d'un plugin ; ensemble des classes déclarées, remis à zéro par `run`). Liés aux tags lus un par un, branche par branche (`>=7.4.0 <=7.4.21 || …`) : une nouvelle version d'un de ces plugins, même un correctif, rend `-v` à Composer jusqu'à sa lecture (rebalayer ses tags : classe, `getSubscribedEvents`, écritures VERBOSE, puis étendre la borne de la branche). Un plugin qu'amène la résolution est jugé sur le lock résolu, avant son écriture (`resolved_issue`) ; sous `-v`, un `update`/`require`/`remove` qui installe garde ses lignes jusque-là (`vivacity_core::io::defer` : relâchées, ou jetées si la run part chez Composer) — rien ne s'affiche pendant la résolution. De même pour l'`install` sans lock, résolu comme un `update` (2026-10-27). Non portés : `COMPOSER_DEBUG_EVENTS`, l'auto-installation de php-http/discovery, `post-package-*` (yii2, thanks, merge-plugin, Flex : rendus). package-versions-deprecated en `require-dev` sous `--no-dev` : chargé (sa ligne `>` sous -v), il ne se trouve pas dans le lock sans `packages-dev` et n'écrit rien — mesuré, identique (`verbosity.sh`, `pv-dev`). `dump-autoload` purge un paquet dont le répertoire a disparu, comme `Factory::purgePackages` (2026-10-27).
- **Explications d'un ensemble insoluble** (v0.7) : `problem.rs` = port de `Problem`, `Rule::getPrettyString`, `SolverProblemsException` (+ `RepositorySet::findPackages`/`getProviders`, `IniHelper::getAll`, le suivi des versions retirées par l'optimiseur et par les politiques dans `Pool`). Oracle : `harness/steps.sh @stderr` (Composer sans `--quiet`, stderr séparé, `COMPOSER_TESTS_ARE_RUNNING`) sur 51 cas, texte identique à l'octet de la ligne d'ancrage à la fin (dont l'API `providers-api` rejouée en `file://`). Portés mais sans oracle (inatteignables avec la CLI supportée) : les contraintes temporaires (`--with`), un dépôt de priorité inférieure (`computeCheckForLowerPrioRepo`, le harness n'injecte qu'un dépôt), les deux indices `composer-plugin-api[2.0.0]` (texte mort en 2.10), la règle `LEARNED` dans un problème (« Conclusion: … ») — le diamant du corpus ne l'atteint pas, l'échec de l'extraction non-dev (« Unable to find a compatible set… », aucun cas construit ne l'atteint : les paquets résolus restent candidats ; de plus ce chemin formate avec le jeu de dépôts de l'update, pas le jeu [racine, plateforme, résultat] de `extractDevPackages` — `getRepoName` et `findPackages` y diffèreraient). La forme `--verbose` des explications est câblée depuis 0.21 (`is_verbose` suit `-v`) et comparée par `harness/verbosity.sh` (`acme/many` : texte détaillé identique). Écarts connus et acceptés : `Link::getPrettyString` d'un lien `self.version` d'un alias (Composer imprime `== x.y.z.z`), `substr` d'une description coupée au milieu d'un caractère multi-octets (remplacé par U+FFFD), `Url::sanitize` absent de `getRepoName`. Hypothèse d'oracle : la liste des `.ini` vient de `php_ini_loaded_file()`/`php_ini_scanned_files()` du PHP sondé (`COMPOSER_ORIGINAL_INIS` respecté) — identique sur la machine du harness par construction. Trouvé en chemin, non corrigé : un nom en majuscules dans `require` (`acme/NOPE`) fait sortir Composer en 1 (`RootPackageLoader` : « should not contain uppercase characters ») là où vivacity résout et répond 2.
- **`require`** : `--minimal-changes` et `COMPOSER=autre.json` refusés ; une erreur de transport sort en 100 comme chez Composer, y compris pendant le choix de la version (2026-10-27, `harness/transport-exit.sh`, `repo-require`) ; le mtime du lock n'est pas restauré après `updateHash` ; toujours non interactif (pas de proposition de `--dev` d'après les mots-clés, pas de déplacement de clé demandé, pas de confirmation des branches de fonctionnalité) ; `getProviders` (API Packagist des fournisseurs) et `findSimilar` (« Did you mean ») non portés : un nom introuvable donne le message final de Composer sans suggestion ; la validation `LAX_SCHEMA` idem `remove`.
- **`remove`** : `--dry-run`, `--minimal-changes` et `COMPOSER=autre.json` refusés ; toujours non interactif (Composer, sur un TTY sans `-n`, propose de retirer un paquet trouvé dans l'autre section — vivacity avertit seulement, comme `-n`) ; la validation `LAX_SCHEMA` après chaque édition n'est pas portée (Composer refuse d'éditer un manifeste déjà invalide, vivacity l'édite) ; le code retour d'une erreur de transport diffère (Composer 100 ou statut HTTP, vivacity 1) ; `--unused` liste chaque paquet une fois là où Composer répète les alias (message seulement) ; `config.vendor-dir`, `bin-dir` et `preferred-install: source` ne sont pas lus (vendor/ partout dans vivacity) — depuis 0.10 ils sont des `ScopeIssue::Config` : `install` part en fallback avec la raison au lieu d'écrire une disposition différente ; `JsonManipulator` : seuil de backtracking PCRE, `1e999`, substituts UTF-16 isolés, > 512 niveaux (en-tête du module).
- **Windows** : livré depuis 0.7.0 (binaire `x86_64-pc-windows-msvc` dans chaque release, `install.ps1`, action sur windows-latest ; proxies `.bat` générés et exécutés en CI, `bin-compat` global). Couvert par `windows.yml` : build, lints, tests unitaires, puis `harness/diff-vendor.sh` (avec et sans autoloader) contre un Composer natif Windows, 0 diff exigé. Non couvert sous Windows : `steps.sh`/`update.sh`/`path-repos.sh`/`corpus.sh` (Linux/macOS seulement) ; un lock avec un paquet `path` part en fallback `composer install` (jonctions non portées, voir Dépôts `path`).
- **Concurrence** : deux installs simultanés sur le même vendor/ ne sont pas
  protégés (comme Composer) ; le store, lui, est sûr (temp+rename).
- **Drift** : `ci.yml` épinglé sur Composer 2.10.3 ; `drift.yml` (hebdo +
  manuel) teste `composer:v2` et `snapshot` en deux étages (jumeaux de
  docs/reference/, puis tests + harness) et ouvre une issue `drift`. Le
  template symfony/runtime n'a pas de jumeau vendoré (pas dans le phar) :
  son drift n'est vu que par le boot de la fixture symfony. Drift connu
  du snapshot 2.11 (2026-09-21) : `ComposerRepository.php` (requête
  security-advisories par lots de 500 — **porté**, la référence reste
  2.10.3, donc le jumeau restera signalé jusqu'au passage en 2.11) ;
  `PlatformRepository.php` (`lib-mbstring-oniguruma` absent dès PHP 8.6
  — à porter avec la référence 2.11, sans effet avant).
- **composer/installers** : émulé pour les tags 2.0.0…2.3.0 et les 58
  frameworks « table seule » ; les 38 à logique custom (agl, akaunting,
  asgard, bitrix, cakephp, cockpit, croogo, dokuwiki, ee2, ee3, fork, grav,
  hurad, lms, majima, mantisbt, matomo, mautic, maya, mediawiki, microweber,
  october, ontowiki, oxid, piwik, plentymarkets, processwire, pxcms, radphp,
  roundcube, shopware, silverstripe, sitedirect, sydes, tao, tastyigniter,
  winter, yawik) → fallback nominatif. Exercé par diff : WordPress (plugins,
  mu-plugin, thème, `installer-paths` par type et par nom, `bin` hors
  vendor/, suppression, `--no-plugins`, `--working-dir` relatif à la main) ;
  par oracle seulement : les autres frameworks,
  `installer-disable`, `installer-name`, `vendor:`. Non couvert : un
  `installer-paths` ciblant vendor/ (refusé), `allow-plugins` global lu mais
  jamais exercé en CI, un lock 1.x (refusé), un projet dont le vendor a été
  posé par un installers de version différente (le plan de suppression
  compare l'ancien install-path au chemin recalculé : désaccord → fallback) ;
  plugin présent d'un seul côté entre installed.json et le lock → fallback
  (test unitaire, pas de fixture). Non porté : `realpath()` de BinaryInstaller
  sur un vendor/ symlinké avec un `bin` hors vendor/ (Composer écrirait un
  chemin absolu) ; `installer-name` contenant `{` refusé plutôt qu'imité.
- **drupal/core-composer-scaffold** : plus émulé depuis 0.6.0 (licence).
  Plugin inconnu → fallback `composer install` avant toute écriture ;
  `dump-autoload` refuse (code 3) tant que le plugin est verrouillé et
  autorisé, puisque Composer exécuterait son `pre-autoload-dump`.
- **Extraction** : strip du dossier racine seulement s'il est unique
  (règle ArchiveDownloader) ; un zip avec un `.DS_Store` de premier niveau
  et rien d'autre à côté du dossier est traité comme mono-dossier, comme Composer.
- **La sortie d'une run rendue à Composer** (2026-10-16, `harness/fallback-args.sh`
  prouve ce que Composer *reçoit*, pas ce qu'il imprime) : le fallback de portée
  d'`install` est décidé après que vivacity a imprimé l'en-tête, « Verifying… » et
  « Package operations », que Composer réimprime ; une délégation décidée sur le
  lock résolu (Flex) suit « has been updated » / « Running composer update » /
  « Loading composer repositories », que Composer réimprime aussi ; l'install
  rendue après un update imprime « Verifying… » qu'un update n'imprime pas, et
  pas les suggestions ; sous `--run-scripts` hors portée, `pre-install-cmd`
  tourne avant la décision puis dans Composer. Sous Windows, `composer.bat` est
  lancé par `cmd.exe` : Rust refuse un argument qu'il ne sait pas échapper
  (sortie 1, rien de lancé) — non exercé, le harnais tourne sur ubuntu et macos.
  L'avertissement de version racine suit la même règle (2026-10-16) : jeté
  quand la délégation précède toute ligne de vivacity, réimprimé par
  Composer sinon ; de même les avertissements des variables
  `COMPOSER_IGNORE_PLATFORM_REQ(S)` et la dépréciation `--no-suggest`,
  retenus avec lui.

- **Installeurs personnalisés (2026-10-28, `docs/plans/custom-installers.md`)** :
  l'ordre d'activation n'est pas rejoué — deux installeurs sur un type dont
  les effets diffèrent partent à Composer ; un plugin présent dans
  installed.json mais dont le répertoire manque (chargé par la transaction,
  pas au démarrage) n'est pas distingué ; les gardes de version ne lisent
  que la chaîne de version (un fork au même tag passerait, comme pour les
  émulations existantes) ; les lignes de dépréciation de composer/installers
  1.x ne sont exercées que sur PHP 8.5 dans cette session (CI : 8.4 ; sous
  8.4 elles sont attendues identiques, non vérifié ici) ; un binaire
  symlink dans une archive d'un paquet posé à un chemin relatif
  (`realpath`) n'est pas vérifié ; simplesamlphp, contao/manager-plugin,
  silverstripe, mautic, omeka, `vendor/craftcms/plugins.php` restent rendus.
  Vus en construisant le banc, laissés : « Skipped installation of bin …
  file not found in package » non imprimé ; le store sert un zip `artifact`
  remplacé à nom et version égaux (dist sans `reference`, `shasum` hors
  identité) ; `lock.rs::target_dir` ne réécrit pas les segments `..` comme
  `Package::getTargetDir`.

- **Plugins à effets de bord (2026-10-28, `docs/plans/plugins-round2.md`)** :
  portés dg/composer-cleaner v2.2.2, liborm85/composer-vendor-cleaner 1.7.1,
  cakephp/plugin-installer 2.0.2, laminas-skeleton-installer 1.2.0–1.3.0 ;
  inertes seulement spiral, sculpin, laminas-component-installer (une
  publication, un thème, une configuration à injecter en dev → Composer).
  Non couverts : `-v` des six autres que cakephp ; Windows pour les quatre à
  effets de fichiers ; l'ordre `readdir` des lignes stderr de dg (« unknown
  autoload type », « Invalid … ») quand plusieurs paquets en écrivent ; un
  composer.json de dist qui diffère de l'entrée du lock (les formes sont
  vérifiées sur le lock) ; un vendor nettoyé par liborm85 dont le plugin et
  la configuration ont disparu, paquets non réinstallés depuis : le cache de
  classmap du store peut encore y servir les classes de fichiers supprimés
  (DECISIONS 2026-10-28). Rendus : drupol/composer-packages, attribute-
  collector, class-alias-loader + cms-composer-installers (typo3),
  composer-patcher, contao/manager-plugin. Vus, laissés : `vendor/bin` vide
  que Composer recrée quand une mise à jour sans binaire suit le retrait
  qui l'a vidé (vivacity le supprime) ; `update`/`require` sur un dépôt
  `artifact` s'arrêtent en erreur au lieu de partir à Composer.
- **drupol/composer-packages, la durée de la résolution, l'attribute-
  collector (2026-10-29, `docs/plans/duration-drupol-collector.md`)** :
  drupol 2.0.0 porté (bolt-project natif) ; rendus : les cas nommés au plan
  (installeur ou nettoyeur à côté, `symfony-pack`, Windows, son retrait,
  pas de lock lisible, `--run-scripts` avec un script `post-autoload-dump`,
  Twig 2, données de lock que `var_export` ne relirait pas). Non couverts :
  une contrainte invalide dans `packages-dev` d'un lock retouché à la main
  sous `--no-dev` (Composer lèverait dans le plugin, vivacity génère) ; le
  rendu sous Twig 2 (rendu à Composer). La ligne `Dependency resolution
  completed` d'`install -v` ne compte que le SAT ; celle d'`update`
  compte encore les règles (laissé). L'attribute-collector reste rendu :
  la réflexion de PHP évalue les arguments des attributs (DECISIONS
  2026-10-29). Vus, laissés : un chemin de projet à `'` (autoload_static.php
  en chemins absolus chez vivacity), un nom de paquet à majuscules (clé
  d'installed.php), `-0` dans un lock (installed.json), un repli de portée
  sous `--run-scripts` qui arrive après `pre-install-cmd`.
- **Les replis restants (2026-10-29, `docs/plans/remaining-fallbacks.md`)** :
  portés infection/extension-installer 0.1.2, typo3/class-alias-loader
  v2.0.1 (avec typo3/cms-composer-installers v5.0.2, inerte sous la racine
  `typo3/cms` seulement — règle observée, source GPL non ouverte) et
  mlocati/composer-patcher 1.2.6 ; négatifs mesurés :
  typo3-cms-base-distribution (PackageArtifact.php change d'ordre d'un run
  de Composer à l'autre), contao/manager-plugin (code des paquets chargé) ;
  dealerdirect 0.7.2 porté ensuite (`docs/plans/chamilo-dealerdirect.md`,
  ci-dessous). Non couverts : la voie
  GNU patch du patcher (« Skipping patch. », correctif déjà appliqué)
  n'est exercée qu'en CI (Linux) — le `patch` de macOS ne l'emprunte
  jamais ; après un install, l'ordre des lignes du patcher et des
  extensions invalides d'infection suit les opérations chez vivacity,
  l'achèvement des extractions chez Composer (harnais : comparé trié à
  l'install vierge, strict au no-op et à la mise à jour) ; `-v` de
  class-alias-loader, cms-composer-installers et du patcher (rendus) ; un
  `bin-dir` déjà dans le `PATH` sous une autre graphie (barre finale) : le
  `PATH` des commandes du patcher peut différer de celui de Composer.
  Vus, laissés : installed.php d'un paquet *aliasé* du dépôt local qui
  remplace à `self.version` (Composer listerait aussi la version de
  l'alias ; relevé par la revue en lisant `generateInstalledVersions`, non
  reproduit).
- **chamilo-lms et dealerdirect v0.7.2 (2026-10-29,
  `docs/plans/chamilo-dealerdirect.md`)** : v0.7.2 porté (recherche dans
  tout le projet, `phpcs_installer::variant`) ; `find_rulesets` suit le
  Finder de Composer 2.10.3 pour toutes les versions (dossiers à point,
  liens de répertoire, lien pendant) ; gardes nommées : phpcs 2 avant
  v1.2.0, phpcs non stable ou aliasé, search-depth non entier, bin-dir déjà
  dans le `PATH` après un autre `phpcs` ou `PATH` sans `php` (0.7.2), aussi
  sur le lock résolu d'un `update`. La fixture chamilo tire ses quinze
  fichiers legacy (GPL-3.0-or-later, 903 Ko) au commit épinglé
  (`fixtures/corpus/chamilo-lms/UPSTREAM`, `harness/corpus.sh`,
  `tools/corpus-upstream.sh`) ; plus rien de GPL commité pour elle. Non
  couverts : Linux et Windows (le harnais crée des liens symboliques ; sous
  MSYS il casse, sans faux vert) ; stdout et stderr comparés fusionnés
  seulement ; `isRunningGlobally` (projet lancé depuis `COMPOSER_HOME`,
  chemins absolus chez le plugin) non géré, préexistant ; `phpcs` exécute
  le PHP du projet (`autoload.files`) : quand il plante, Composer imprime
  « Failed to set … » et n'écrit rien, vivacity écrit la conf —
  imprévisible sans exécuter phpcs, vrai pour toutes les versions ; le même
  trou qu'`emulated_version_issue` pour une version de plugin introduite
  par un `update` reste ouvert ailleurs que pour ces gardes.
- **Le cache de l'action et le répertoire de cache de Composer (2026-10-29,
  `docs/plans/action-cache.md`)** : l'action ne met plus en cache que le
  répertoire que `vivacity config --global cache-dir` imprime ; le cœur suit
  `Config::get('cache-dir')` (`harness/composer-cache-dir.sh`, 20
  environnements + `--working-dir`, macOS et conteneur Linux). Le workflow
  `action` (trois OS, `action-test.yml`) n'est exercé qu'en CI : la
  branche Windows (`cygpath -m` d'une valeur en `/c/…`, `%LOCALAPPDATA%`)
  n'a tourné que dans les tests unitaires (hôte injecté). Points ouverts :
  - les **enregistrements de classmap** hors du cache (2026-10-29,
    `docs/plans/classmap-cache.md`, `bench/M8-classmap-cache.md`) : leur clé
    (chemin de l'entrée de store) survit à la reconstruction du store, ils
    servent sur un runner neuf ; gain mesuré sur M4 Max avec `-o` : 1,4 s
    (Laravel), 2,8 s (Sylius), ~0,2 s sans `-o`. Laissés dehors pour une
    raison de correction : la clé ne porte ni l'URL de la dist ni les règles
    d'extraction, un enregistrement restauré peut décrire d'autres fichiers
    que le store reconstruit. Reste ouvert : les chiffres Linux et Windows
    (lancer le workflow `classmap-cache`, `workflow_dispatch`), puis, s'ils
    le justifient, une clé qui porte l'identité de l'archive (cœur) et une
    clé de cache liée au binaire (action) — conditions au bench M8 ;
  - **générateur et installeur ne nomment pas l'entrée de store pareil** :
    `dist_reference()` côté `generator.rs`, `store_reference()` (repli sur
    le sha1 de l'URL) côté installeur ; une dist sans référence (tar
    d'asset-packagist) n'a jamais d'enregistrement de classmap de store,
    rescannée à chaque install (relevé par la méta du 2026-10-29, vérifié
    dans le code, non mesuré, non corrigé) ;
  - **le ménage du cache de Composer** : vivacity n'en fait jamais,
    Composer si (fichiers de plus de 6 mois, `cache-files-maxsize` de 300
    Mio) ; un cache restauré par restore-key grossit donc d'un run à
    l'autre tant qu'aucun Composer ne passe (une action composite n'a pas
    d'étape `post:` pour élaguer) ;
  - **les anciens `store/vN`** ne sont jamais supprimés (`LAYOUT` bumpé,
    l'arbre précédent reste sur disque jusqu'au vidage du cache) ;
  - `hashFiles('**/composer.lock')` **hache tous les locks du dépôt**
    (fixtures, sous-projets), pas celui qu'on installe ;
  - les autres **`actions/*@v4`** des workflows du dépôt (`checkout`,
    `cache`, `upload-artifact`, `download-artifact` dans action-test, ci,
    bench, ci-cache, drift, release, windows, corpus) : non épinglés par
    SHA, avertissement node20 ; hors de ce changement ;
  - le store se dit « content-addressed » (`store.rs`, README) : sa clé est
    la version plus les 12 premiers caractères de la référence, testée par
    `is_dir()` — pas un condensat du contenu ;
  - « reconstruit sans réseau » ne vaut que pour les dists : une source git
    est resynchronisée pour une référence sans branche ni tag, Windows rend
    les sources git à Composer, Flex fait un GET conditionnel ;
  - `config.cache-dir` / `cache-files-dir` / `cache-repo-dir` du
    `composer.json` **du projet** : rendus à Composer par les commandes,
    mais pas suivis par l'action (elle ne connaît pas le projet ; un
    `working-directory:` de l'install non plus) ;
  - `vivacity config` n'accepte que `--global`/`-g` et `cache-dir` : `-q`,
    `-n`, `--no-ansi`, `-d` sortent en 2 là où Composer répond ; un
    `COMPOSER_BIN_COMPAT` ou un `./composer.json` invalides font sortir
    Composer en 1, vivacity répond ; une délégation de `config` ajoute la
    ligne `vivacity: delegating to …` sur stderr ;
  - l'action : `cache: 'false'` le dit dans le journal, sans notice ; une entrée
    `version:` antérieure à cette version n'a pas `config` et coupe le
    cache (notice) ; la valeur résolue l'est dans bash — MSYS convertit
    `/c/…` pour le binaire natif, si bien que `cygpath -m` ne sert presque
    jamais, et une étape `pwsh` qui lit un `COMPOSER_CACHE_DIR` en `/…`
    brut peut écrire ailleurs ;
  - **sous Windows, la fin de ligne de stdout** : Composer écrit `PHP_EOL`
    (`\r\n`), vivacity `\n`, pour toutes les commandes (constaté par la
    revue en lisant `StreamOutput::doWrite`, non mesuré sur un runner) ;
    `action-test.yml` retire le `\r` avant de comparer ;
  - `htaccess-protect` : Composer crée `.htaccess` dans son home, son cache
    et son répertoire de données à chaque commande, vivacity jamais ; sous
    macOS il déplace aussi un ancien `<home>/cache` vers
    `~/Library/Caches/composer`, vivacity non.

## Pièges

- `serde_json` DOIT garder `preserve_order` + `float_roundtrip` (content-hash).
- Un `conflict` ou `replace` de la racine visant un paquet du lock : Composer
  refuse le lock à l'install (règles du solveur, « Your lock file does not
  contain a compatible set of packages ») ; vivacity aussi depuis le
  2026-10-16 (la racine figée garde ses liens dans la résolution de
  `install`). Visible avec composer-merge-plugin (un include peut apporter
  ces liens) ; la fixture les tient hors du lock.
- Le pattern classmap de Composer exige pcre2 (possessifs, lookbehind,
  octets non-UTF-8) — décision plan r1/F4. Les noms de classes sont des `Vec<u8>`.
- PCRE2 est compilé depuis `crates/vivacity-pcre2-sys/upstream/` avec tous
  ses symboles préfixés `vivacity_` (jamais la lib système : un embarqueur
  qui lie `libphp.a` — ePHPm — aurait des symboles en double). Pour passer à
  un pcre2-sys plus récent : recopier `upstream/`, `src/bindings.rs`,
  `build.rs` ; réappliquer le patch de `pcre2.h` / `pcre2_internal.h`
  (README du crate) ; `tools/prefix-pcre2-bindings.py` ;
  `tools/check-pcre2-symbols.sh` doit compter 0 symbole non préfixé.
- `harness/diff-vendor.sh` copie le projet complet (les règles d'autoload de
  la racine — `src/Kernel.php` chez Sylius — doivent exister).
- Sylius boot : `php -d memory_limit=1G` (128 Mo brew insuffisants) ; une résolution FRAÎCHE de sylius-standard ne boote pas sur PHP 8.5 (Doctrine ORM / lazy objects) — CI épinglée en PHP 8.4.
- `symfony/demo` n'existe pas sur Packagist : `symfony/symfony-demo`.
- Les fixtures (`fixtures/work/`) sont gitignorées : `fixtures/make.sh` d'abord — il copie les squelettes FIGÉS de `fixtures/projects/` (locks committés) ; ne pas re-résoudre.
- Cache Composer : chemins canoniques portés de `Factory` (Linux sans XDG → `~/.composer/cache`) ; `harness/linux.sh` persiste `/root/.composer` dans un volume.
