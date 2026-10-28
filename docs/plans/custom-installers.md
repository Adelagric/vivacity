# Installeurs personnalisés qui bloquent le corpus — plan

> Suite de `docs/corpus/2026-10-21.md` : sur 106 projets réels, 30 partent
> à Composer en `--no-dev`, dont 11 à cause de plugins `composer-plugin` qui
> enregistrent un installeur décidant où une famille de types est posée.
> Niveau 2 : parité à l'octet de vendor/, du lock, de stdout, de stderr, du
> code de sortie et des fichiers que les plugins émulés écrivent ; ce qui
> n'est pas reproductible part à Composer, nommé, avant toute écriture.
> Méta-analyse et revue par sous-agents frais.

## Le problème

`InstallationManager::addInstaller` met l'installeur d'un plugin en tête de
liste (`array_unshift`) ; `getInstaller($type)` rend le premier qui
`supports()` le type — le dernier activé gagne. vivacity émule trois de ces
installeurs (composer/installers 2.0.0–2.3.0, roots/wordpress-core-installer,
mnsami/composer-custom-directory-installer) ; tout autre plugin est « not on
vivacity's known-plugin list » et la commande part à Composer. Le scan hors
ligne (`install --check-scope --no-dev`, binaire de `main` 8a48734) des
entrées concernées donne leurs bloqueurs **complets** — une entrée n'est
native que si tous le sont :

| entrée | bloqueurs |
|---|---|
| craftcms-craft | craftcms/plugin-installer |
| getkirby-plainkit | getkirby/composer-installer |
| neos-base-distribution | neos/composer-plugin |
| phplist-phplist3 | davidbarratt/custom-installer |
| themosis | johnpbloch/wordpress-core-installer ; composer/installers v1.12.0 (non porté) |
| thelia-project | thelia/installer ; 13 modules que composer/installers pose sous `vendor/thelia/modules/` (« inside vendor/ », refusé) |
| ilias-elearning-ilias | simplesamlphp/composer-module-installer ; simplesamlphp/composer-xmlprovider-installer |
| contao-managed-edition | contao-components/installer ; **contao/manager-plugin** |
| friendica | oomphinc/composer-installers-extender ; **friendica/json-ld sans dist** |
| omeka-s | omeka/composer-addon-installer (paquet `path` **absent de la fixture**) |
| silverstripe-installer | **silverstripe/recipe-plugin**, **silverstripe/vendor-plugin** ; composer/installers `silverstripe` (logique propre) |
| mautic, mautic-recommended-project | composer/installers `mautic` (logique propre) ; **cweagans/composer-patches**, **mautic/core-composer-scaffold**, **mautic/core-project-message**, paquets sans dist, package-versions 1.11.99.1 |

En gras : des bloqueurs qui ne sont pas des installeurs de chemin et que ce
chantier ne lève pas.
