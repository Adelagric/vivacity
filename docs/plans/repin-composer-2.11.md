# Ré-épingler sur Composer 2.11 — la liste, mesurée sur 2.11-dev

> État au 2026-10-09, contre `2.11-dev+43f74517` (2026-10-08). 2.11 n'est pas
> publié : cette liste se rejoue au moment du ré-épinglage, elle ne s'exécute pas
> avant. Elle existe pour que ce jour-là soit un travail connu.

## Comment la mesure a été faite

Le canal `drift (composer snapshot)` était rouge chaque semaine depuis le
20 septembre, mais **son étape 2 s'arrêtait au premier test en échec** (`bash
-e`) : il rapportait une seule différence attendue pendant qu'aucun harnais ne
tournait derrière. Corrigé dans `drift.yml` (chaque commande tourne, `cargo test
--no-fail-fast`). Puis tout rejoué en local contre le snapshot, sans toucher au
Composer installé :

    cp …/composer.phar snap/ && php snap/composer.phar self-update --snapshot
    ln -s snap/composer.phar snap/bin/composer
    PATH=snap/bin:$PATH TMPDIR=snap/tmp VIVACITY_HARNESS_DIR=… VIVACITY_CACHE_DIR=… \
      cargo test --no-fail-fast ; harness/…

(`TMPDIR` à part n'est plus nécessaire depuis que la copie du phar des oracles est
nommée d'après l'identité de Composer — voir DECISIONS 2026-10-09.)

## Ce qui ne bouge pas

`diff-vendor` 12/12 et 12/12 avec l'autoloader, `removal` 3/3, `transitions`
13/13, `boot` 6/6, `root-manifest` 35/35 : **l'install et la validation du
manifeste racine sont inchangés** en 2.11-dev. 36 fichiers de référence ont
bougé en amont (`drift-reference.sh`), mais leur effet observable sur tout ce que
la suite et les harnais couvrent se réduit aux trois causes ci-dessous.

## Les trois causes, et ce qu'il faudra porter

1. **`PluginInterface::PLUGIN_API_VERSION` passe à `2.11.0`.** Visible dans le
   champ `plugin-api-version` de chaque lock et dans le paquet de plateforme
   `composer-plugin-api` (6 tests : `oracle_plain_repo`, `oracle_pool` ×3,
   `oracle_version_selector` ×2, et la plupart des cas de `steps` / `update` /
   `path-repos`). Port : la constante dans `vivacity-resolver/src/platform.rs`
   (`PLUGIN_API_VERSION`), avec `COMPOSER_VERSION` et `RUNTIME_API_VERSION` à
   relire au même moment.
2. **Un nouveau champ de lock, `published-time`.** `ArrayLoader.php:259-260` le lit
   dans les métadonnées (entier ou chaîne non vide), `ArrayDumper.php:98-99`
   l'écrit en RFC 3339 juste après `time`. vivacity ne l'écrit pas : c'est la
   différence de la majorité des 114 cas de `steps` et des 7 d'`update`. Port :
   le chargeur et le vidage de paquets, à l'octet près, ordre des clés compris —
   et un harnais qui le tient.
3. **semver #187** (déjà acquitté dans `DRIFT-ACK`) : plus de `-dev` sur une borne
   `>=` ou `<` portant un suffixe RC (`>=2.5.0-RC1` → `>= 2.5.0.0-RC1`). 4
   contraintes du corpus, 3 paires d'intervalles. Port : le `VersionParser` et
   `Intervals` ; semver #189 (acquitté aussi) au même moment.

À relire en plus, acquittés plus tôt et sans effet mesuré ici :
class-map-generator #48/#49/#50, `PlatformRepository` (MB_ONIGURUMA_VERSION).

## Comment on saura que c'est fini

La suite et les harnais verts **contre 2.11**, `drift-reference.sh` à 0 dérive
après re-vendorisation de `docs/reference/`, et le corpus rejoué.
