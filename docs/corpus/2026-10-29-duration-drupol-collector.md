# Corpus report — 2026-10-29

Baseline: Composer 2.10.3 `install --no-scripts` (plugins loaded and fully active — `--no-scripts` only skips the composer.json scripts) against `vivacity install --no-fallback`, `--ignore-platform-req=ext-*` on both sides (`php` too where this machine's PHP fails the lock, noted per entry). Buckets: **native** = 0 diff in `vendor/` (files, modes, link targets); **fallback** = vivacity stopped before any write, with the reason; **diff** = a parity bug; **unavailable** = Composer itself failed. See `fixtures/corpus/README.md` for the inputs and `harness/corpus.sh` for the method.

Entries: 106.

## Mode `dev`

| bucket | projects | share | distinct packages |
|---|---:|---:|---:|
| native | 91 | 86 % | 4251 |
| fallback | 15 | 14 % | 1512 |
| diff | 0 | 0 % | 0 |
| unavailable | 0 | — | 0 |

Distinct `name@version` packages across the counted entries: 4968; laid out natively at least once: 4251 (86 %).

### Fallback reasons, ranked

| reason | entries | note |
|---|---:|---|
| other `plugin drupal/core-project-message would print its post-install-cmd message (not emulated)` | 3 |  |
| plugin `drupal/core-composer-scaffold` | 3 | listens to post-install-cmd/post-update-cmd/post-package-* (active in the baseline) |
| installers `mautic` | 2 |  |
| layout-plugin `cweagans/composer-patches` | 2 | listens to pre-install-cmd/pre-update-cmd + post-package-install (active in the baseline) |
| plugin `drupal/core-vendor-hardening` | 2 |  |
| plugin `typo3/class-alias-loader` | 2 |  |
| plugin `typo3/cms-composer-installers` | 2 |  |
| installers `silverstripe` | 1 |  |
| other `package drupal/core-composer-scaffold is a path package whose directory composer/Plugin/Scaffold does not exist` | 1 |  |
| other `package drupal/core-project-message is a path package whose directory composer/Plugin/ProjectMessage does not exist` | 1 |  |
| other `package drupal/core-recipe-unpack is a path package whose directory composer/Plugin/RecipeUnpack does not exist` | 1 |  |
| other `package drupal/core-vendor-hardening is a path package whose directory composer/Plugin/VendorHardening does not exist` | 1 |  |
| other `package mautic/core-lib is a path package whose directory app does not exist` | 1 |  |
| other `package omeka/composer-addon-installer is a path package whose directory application/data/composer-addon-installer does not exist` | 1 |  |
| other `plugin composer/package-versions-deprecated 1.11.99.1 is not emulated in this version (versions read: 1.11.99.4, 1.11.99.5; earlier versions generate another Versions.php)` | 1 |  |
| other `plugin dealerdirect/phpcodesniffer-composer-installer v0.7.2 is not emulated in this version (versions read: 1.0.0, 1.1.1, 1.1.2, 1.2.0, 1.2.1; 0.x also searches the whole project for rulesets, v1.1.0 asks InstalledVersions where phpcs is)` | 1 |  |
| plugin `contao/manager-plugin` | 1 |  |
| plugin `drupal/site_template_helper` | 1 |  |
| plugin `infection/extension-installer` | 1 |  |
| plugin `mautic/core-composer-scaffold` | 1 | listens to post-install-cmd/post-update-cmd (active in the baseline) |
| plugin `mautic/core-project-message` | 1 | listens to post-create-project-cmd/post-install-cmd (active in the baseline) |
| plugin `mlocati/composer-patcher` | 1 | listens to pre-install-cmd/pre-update-cmd + post-package-* (active in the baseline) |
| plugin `omeka/composer-addon-installer` | 1 |  |
| plugin `ondrejmirtes/composer-attribute-collector` | 1 | listens to post-autoload-dump (active in the baseline) |
| plugin `silverstripe/recipe-plugin` | 1 | listens to post-package-install/update (active in the baseline) |
| plugin `silverstripe/vendor-plugin` | 1 | listens to installer + post-install-cmd (active in the baseline) |
| plugin `simplesamlphp/composer-module-installer` | 1 |  |
| plugin `simplesamlphp/composer-xmlprovider-installer` | 1 |  |
| plugin `tbachert/spi` | 1 |  |

### Entries

| entry | bucket | packages | composer s | vivacity s | ignored | detail |
|---|---|---:|---:|---:|---|---|
| aimeos | native | 142 | 7 | 2 | ext-* |  |
| akaunting | native | 223 | 9 | 4 | ext-* php |  |
| akeneo-pim-community-dev | native | 258 | 15 | 2 | ext-* php |  |
| alextselegidis-easyappointments | native | 52 | 22 | 7 | ext-* |  |
| ampache | native | 135 | 7 | 2 | ext-* |  |
| api-platform | native | 40 | 1 | 1 | ext-* |  |
| bagisto | native | 220 | 8 | 3 | ext-* php |  |
| bolt-project | native | 190 | 7 | 1 | ext-* |  |
| bookstackapp-bookstack | native | 151 | 7 | 2 | ext-* |  |
| cachethq-cachet | native | 180 | 7 | 3 | ext-* |  |
| cakephp-app | native | 91 | 4 | 1 | ext-* |  |
| chamilo-lms | fallback | 340 |  |  |  | plugin dealerdirect/phpcodesniffer-composer-installer v0.7.2 is not emulated in this version (versions read: 1.0.0, 1.1.1, 1.1.2, 1.2.0, 1.2.1; 0.x also searche |
| claroline | native | 174 | 6 | 1 | ext-* php |  |
| codeigniter4-appstarter | native | 33 | 2 | 0 | ext-* |  |
| composer | native | 36 | 2 | 0 | ext-* |  |
| composer-satis | native | 63 | 2 | 0 | ext-* |  |
| concretecms | fallback | 188 |  |  |  | plugin mlocati/composer-patcher is not on vivacity's known-plugin list |
| contao-managed-edition | fallback | 206 |  |  |  | plugin contao/manager-plugin is not on vivacity's known-plugin list |
| craftcms-craft | native | 119 | 7 | 2 | ext-* |  |
| crater-invoice-crater | native | 167 | 10 | 2 | ext-* php |  |
| cypht-org-cypht | native | 74 | 5 | 1 | ext-* |  |
| doppar | native | 74 | 3 | 0 | ext-* |  |
| drupal | fallback | 145 |  |  |  | plugin drupal/core-composer-scaffold is not on vivacity's known-plugin list; package drupal/core-composer-scaffold is a path package whose directory composer/Pl |
| drupal-cms | fallback | 219 |  |  |  | plugin drupal/core-composer-scaffold is not on vivacity's known-plugin list; plugin drupal/core-vendor-hardening is not on vivacity's known-plugin list; plugin  |
| drupal-recommended-project | fallback | 68 |  |  |  | plugin drupal/core-composer-scaffold is not on vivacity's known-plugin list; plugin drupal/core-project-message would print its post-install-cmd message (not em |
| elgg | native | 289 | 19 | 4 | ext-* |  |
| espocrm | native | 153 | 5 | 1 | ext-* |  |
| filamentphp-demo | native | 203 | 8 | 3 | ext-* |  |
| filegator | native | 53 | 3 | 0 | ext-* |  |
| firefly-iii | native | 186 | 6 | 1 | ext-* |  |
| forkcms | native | 90 | 8 | 2 | ext-* php |  |
| fossbilling | native | 154 | 5 | 1 | ext-* |  |
| freescout-help-desk-freescout | native | 112 | 5 | 1 | ext-* php |  |
| freshrss | native | 31 | 2 | 1 | ext-* |  |
| friendica | native | 172 | 8 | 2 | ext-* |  |
| getgrav-grav | native | 109 | 4 | 1 | ext-* |  |
| getkirby-plainkit | native | 19 | 1 | 1 | ext-* |  |
| glpi-project-glpi | native | 209 | 8 | 3 | ext-* |  |
| humhub | native | 257 | 9 | 3 | ext-* |  |
| hyperf-skeleton | native | 136 | 4 | 1 | ext-* |  |
| ibexa-oss-skeleton | native | 190 | 18 | 7 | ext-* |  |
| ilias-elearning-ilias | fallback | 153 |  |  |  | plugin simplesamlphp/composer-module-installer is not on vivacity's known-plugin list; plugin simplesamlphp/composer-xmlprovider-installer is not on vivacity's  |
| invoiceninja | native | 310 | 56 | 20 | ext-* |  |
| invoiceplane | native | 37 | 4 | 1 | ext-* |  |
| joomla-cms | native | 147 | 6 | 2 | ext-* |  |
| kimai | native | 197 | 6 | 2 | ext-* |  |
| koel | native | 201 | 7 | 3 | ext-* |  |
| kovah-linkace | native | 213 | 7 | 2 | ext-* |  |
| laminas-mvc-skeleton | native | 83 | 3 | 1 | ext-* php |  |
| laravel | native | 109 | 3 | 0 | ext-* |  |
| laravel-lumen | native | 104 | 4 | 0 | ext-* |  |
| leafs-mvc | native | 53 | 3 | 1 | ext-* |  |
| leantime | native | 226 | 10 | 2 | ext-* |  |
| librenms | native | 203 | 7 | 2 | ext-* |  |
| linuxserver-heimdall | native | 154 | 7 | 2 | ext-* |  |
| lycheeorg-lychee | native | 253 | 7 | 3 | ext-* |  |
| matomo-org-matomo | native | 87 | 4 | 1 | ext-* |  |
| mautic | fallback | 265 |  |  |  | plugin cweagans/composer-patches changes the install layout (not emulated); package mautic/core-lib is a path package whose directory app does not exist; plugin |
| mautic-recommended-project | fallback | 248 |  |  |  | plugin mautic/core-composer-scaffold is not on vivacity's known-plugin list; plugin mautic/core-project-message is not on vivacity's known-plugin list; installe |
| mezzio-skeleton | native | 147 | 5 | 0 | ext-* |  |
| monicahq-monica | native | 241 | 10 | 3 | ext-* php |  |
| neos-base-distribution | native | 155 | 7 | 2 | ext-* |  |
| nette-web-project | native | 24 | 3 | 0 | ext-* |  |
| nextcloud-server | native | 1 | 1 | 0 | ext-* |  |
| october | native | 140 | 3 | 1 | ext-* |  |
| omeka-s | fallback | 134 |  |  |  | plugin omeka/composer-addon-installer is not on vivacity's known-plugin list; package omeka/composer-addon-installer is a path package whose directory applicati |
| open-web-analytics | native | 47 | 2 | 1 | ext-* |  |
| opencart | native | 17 | 5 | 0 | ext-* |  |
| owncloud-core | native | 98 | 11 | 2 | ext-* |  |
| pelican-dev-panel | native | 219 | 8 | 3 | ext-* |  |
| phan | native | 50 | 2 | 1 | ext-* |  |
| phpdocumentor | native | 115 | 6 | 1 | ext-* |  |
| phplist-phplist3 | native | 69 | 7 | 1 | ext-* php |  |
| phpmyadmin | native | 132 | 4 | 0 | ext-* |  |
| phpro-grumphp | native | 110 | 2 | 0 | ext-* |  |
| phpstan-src | fallback | 101 |  |  |  | plugin ondrejmirtes/composer-attribute-collector is not on vivacity's known-plugin list; plugin cweagans/composer-patches changes the install layout (not emulat |
| pimcore-skeleton | native | 219 | 12 | 4 | ext-* |  |
| pixelfed | native | 198 | 6 | 2 | ext-* |  |
| prestashop | native | 281 | 10 | 2 | ext-* |  |
| pterodactyl-panel | native | 170 | 5 | 2 | ext-* |  |
| roave-backwardcompatibilitycheck | fallback | 144 |  |  |  | plugin infection/extension-installer is not on vivacity's known-plugin list |
| roots-bedrock | native | 73 | 45 | 1 | ext-* |  |
| rss-bridge | native | 29 | 0 | 1 | ext-* |  |
| salesagility-suitecrm-core | native | 303 | 17 | 10 | ext-* |  |
| sculpin | native | 90 | 3 | 0 | ext-* |  |
| sebastianbergmann-phpunit | native | 26 | 2 | 0 | ext-* |  |
| shaarli | native | 48 | 2 | 1 | ext-* |  |
| shopware-production | native | 174 | 13 | 3 | ext-* |  |
| silverstripe-installer | fallback | 113 |  |  |  | plugin silverstripe/recipe-plugin is not on vivacity's known-plugin list; plugin silverstripe/vendor-plugin is not on vivacity's known-plugin list; installers:  |
| slim-skeleton | native | 57 | 2 | 1 | ext-* |  |
| snipe-it | native | 243 | 8 | 3 | ext-* |  |
| spiral-app | native | 137 | 3 | 1 | ext-* |  |
| statamic | native | 161 | 6 | 2 | ext-* |  |
| sulu-skeleton | native | 195 | 8 | 2 | ext-* |  |
| symfony-demo | native | 153 | 9 | 0 | ext-* |  |
| symfony-skeleton | native | 1 | 0 | 0 | ext-* |  |
| thelia-project | native | 177 | 9 | 1 | ext-* |  |
| themosis | native | 140 | 7 | 1 | ext-* |  |
| typo3 | fallback | 129 |  |  |  | plugin typo3/class-alias-loader is not on vivacity's known-plugin list; plugin typo3/cms-composer-installers is not on vivacity's known-plugin list |
| typo3-cms-base-distribution | fallback | 105 |  |  |  | plugin typo3/class-alias-loader is not on vivacity's known-plugin list; plugin typo3/cms-composer-installers is not on vivacity's known-plugin list |
| unopim | native | 212 | 8 | 3 | ext-* php |  |
| uvdesk-community-skeleton | native | 1 | 1 | 0 | ext-* |  |
| wallabag | native | 271 | 10 | 1 | ext-* php |  |
| wordplate | native | 16 | 49 | 1 | ext-* |  |
| yiisoft-yii2-app-basic | native | 91 | 3 | 1 | ext-* |  |
| yourls | native | 14 | 1 | 1 | ext-* php |  |

## Mode `no-dev`

| bucket | projects | share | distinct packages |
|---|---:|---:|---:|
| native | 93 | 88 % | 3670 |
| fallback | 13 | 12 % | 1054 |
| diff | 0 | 0 % | 0 |
| unavailable | 0 | — | 0 |

Distinct `name@version` packages across the counted entries: 4205; laid out natively at least once: 3670 (87 %).

### Fallback reasons, ranked

| reason | entries | note |
|---|---:|---|
| other `plugin drupal/core-project-message would print its post-install-cmd message (not emulated)` | 3 |  |
| plugin `drupal/core-composer-scaffold` | 3 | listens to post-install-cmd/post-update-cmd/post-package-* (active in the baseline) |
| installers `mautic` | 2 |  |
| plugin `drupal/core-vendor-hardening` | 2 |  |
| plugin `typo3/class-alias-loader` | 2 |  |
| plugin `typo3/cms-composer-installers` | 2 |  |
| installers `silverstripe` | 1 |  |
| layout-plugin `cweagans/composer-patches` | 1 | listens to pre-install-cmd/pre-update-cmd + post-package-install (active in the baseline) |
| other `package drupal/core-composer-scaffold is a path package whose directory composer/Plugin/Scaffold does not exist` | 1 |  |
| other `package drupal/core-project-message is a path package whose directory composer/Plugin/ProjectMessage does not exist` | 1 |  |
| other `package drupal/core-recipe-unpack is a path package whose directory composer/Plugin/RecipeUnpack does not exist` | 1 |  |
| other `package drupal/core-vendor-hardening is a path package whose directory composer/Plugin/VendorHardening does not exist` | 1 |  |
| other `package mautic/core-lib is a path package whose directory app does not exist` | 1 |  |
| other `package omeka/composer-addon-installer is a path package whose directory application/data/composer-addon-installer does not exist` | 1 |  |
| other `plugin composer/package-versions-deprecated 1.11.99.1 is not emulated in this version (versions read: 1.11.99.4, 1.11.99.5; earlier versions generate another Versions.php)` | 1 |  |
| plugin `contao/manager-plugin` | 1 |  |
| plugin `drupal/site_template_helper` | 1 |  |
| plugin `mautic/core-composer-scaffold` | 1 | listens to post-install-cmd/post-update-cmd (active in the baseline) |
| plugin `mautic/core-project-message` | 1 | listens to post-create-project-cmd/post-install-cmd (active in the baseline) |
| plugin `mlocati/composer-patcher` | 1 | listens to pre-install-cmd/pre-update-cmd + post-package-* (active in the baseline) |
| plugin `omeka/composer-addon-installer` | 1 |  |
| plugin `ondrejmirtes/composer-attribute-collector` | 1 | listens to post-autoload-dump (active in the baseline) |
| plugin `silverstripe/recipe-plugin` | 1 | listens to post-package-install/update (active in the baseline) |
| plugin `silverstripe/vendor-plugin` | 1 | listens to installer + post-install-cmd (active in the baseline) |
| plugin `simplesamlphp/composer-module-installer` | 1 |  |
| plugin `simplesamlphp/composer-xmlprovider-installer` | 1 |  |

### Entries

| entry | bucket | packages | composer s | vivacity s | ignored | detail |
|---|---|---:|---:|---:|---|---|
| aimeos | native | 105 | 4 | 1 | ext-* |  |
| akaunting | native | 183 | 7 | 1 | ext-* php |  |
| akeneo-pim-community-dev | native | 159 | 6 | 0 | ext-* php |  |
| alextselegidis-easyappointments | native | 27 | 10 | 1 | ext-* |  |
| ampache | native | 74 | 3 | 0 | ext-* |  |
| api-platform | native | 13 | 1 | 0 | ext-* |  |
| bagisto | native | 167 | 7 | 1 | ext-* php |  |
| bolt-project | native | 184 | 8 | 1 | ext-* |  |
| bookstackapp-bookstack | native | 113 | 6 | 1 | ext-* |  |
| cachethq-cachet | native | 139 | 7 | 2 | ext-* |  |
| cakephp-app | native | 20 | 2 | 0 | ext-* |  |
| chamilo-lms | native | 271 | 11 | 6 | ext-* |  |
| claroline | native | 127 | 5 | 0 | ext-* php |  |
| codeigniter4-appstarter | native | 3 | 2 | 0 | ext-* |  |
| composer | native | 30 | 1 | 0 | ext-* |  |
| composer-satis | native | 35 | 1 | 1 | ext-* |  |
| concretecms | fallback | 158 |  |  |  | plugin mlocati/composer-patcher is not on vivacity's known-plugin list |
| contao-managed-edition | fallback | 206 |  |  |  | plugin contao/manager-plugin is not on vivacity's known-plugin list |
| craftcms-craft | native | 111 | 7 | 1 | ext-* |  |
| crater-invoice-crater | native | 143 | 9 | 1 | ext-* php |  |
| cypht-org-cypht | native | 45 | 9 | 1 | ext-* |  |
| doppar | native | 44 | 2 | 1 | ext-* |  |
| drupal | fallback | 64 |  |  |  | plugin drupal/core-composer-scaffold is not on vivacity's known-plugin list; package drupal/core-composer-scaffold is a path package whose directory composer/Pl |
| drupal-cms | fallback | 200 |  |  |  | plugin drupal/core-composer-scaffold is not on vivacity's known-plugin list; plugin drupal/core-vendor-hardening is not on vivacity's known-plugin list; plugin  |
| drupal-recommended-project | fallback | 68 |  |  |  | plugin drupal/core-composer-scaffold is not on vivacity's known-plugin list; plugin drupal/core-project-message would print its post-install-cmd message (not em |
| elgg | native | 257 | 17 | 2 | ext-* |  |
| espocrm | native | 125 | 7 | 1 | ext-* |  |
| filamentphp-demo | native | 150 | 7 | 2 | ext-* |  |
| filegator | native | 25 | 2 | 0 | ext-* |  |
| firefly-iii | native | 132 | 5 | 0 | ext-* |  |
| forkcms | native | 85 | 5 | 1 | ext-* php |  |
| fossbilling | native | 91 | 5 | 1 | ext-* |  |
| freescout-help-desk-freescout | native | 74 | 4 | 1 | ext-* php |  |
| freshrss | native | 0 | 0 | 0 | ext-* |  |
| friendica | native | 103 | 5 | 1 | ext-* |  |
| getgrav-grav | native | 61 | 3 | 1 | ext-* |  |
| getkirby-plainkit | native | 19 | 2 | 0 | ext-* |  |
| glpi-project-glpi | native | 130 | 5 | 1 | ext-* |  |
| humhub | native | 210 | 6 | 2 | ext-* |  |
| hyperf-skeleton | native | 81 | 3 | 1 | ext-* |  |
| ibexa-oss-skeleton | native | 190 | 18 | 3 | ext-* |  |
| ilias-elearning-ilias | fallback | 127 |  |  |  | plugin simplesamlphp/composer-module-installer is not on vivacity's known-plugin list; plugin simplesamlphp/composer-xmlprovider-installer is not on vivacity's  |
| invoiceninja | native | 245 | 39 | 4 | ext-* |  |
| invoiceplane | native | 31 | 3 | 0 | ext-* |  |
| joomla-cms | native | 93 | 5 | 0 | ext-* |  |
| kimai | native | 143 | 8 | 2 | ext-* |  |
| koel | native | 155 | 6 | 1 | ext-* |  |
| kovah-linkace | native | 156 | 5 | 1 | ext-* |  |
| laminas-mvc-skeleton | native | 22 | 2 | 0 | ext-* php |  |
| laravel | native | 76 | 2 | 1 | ext-* |  |
| laravel-lumen | native | 75 | 4 | 1 | ext-* |  |
| leafs-mvc | native | 53 | 3 | 1 | ext-* |  |
| leantime | native | 160 | 7 | 2 | ext-* |  |
| librenms | native | 138 | 5 | 1 | ext-* |  |
| linuxserver-heimdall | native | 121 | 6 | 1 | ext-* |  |
| lycheeorg-lychee | native | 174 | 8 | 1 | ext-* |  |
| matomo-org-matomo | native | 52 | 2 | 1 | ext-* |  |
| mautic | fallback | 193 |  |  |  | plugin cweagans/composer-patches changes the install layout (not emulated); package mautic/core-lib is a path package whose directory app does not exist; plugin |
| mautic-recommended-project | fallback | 248 |  |  |  | plugin mautic/core-composer-scaffold is not on vivacity's known-plugin list; plugin mautic/core-project-message is not on vivacity's known-plugin list; installe |
| mezzio-skeleton | native | 22 | 1 | 0 | ext-* |  |
| monicahq-monica | native | 165 | 7 | 1 | ext-* php |  |
| neos-base-distribution | native | 120 | 15 | 2 | ext-* |  |
| nette-web-project | native | 19 | 2 | 0 | ext-* |  |
| nextcloud-server | native | 0 | 0 | 1 | ext-* |  |
| october | native | 112 | 3 | 1 | ext-* |  |
| omeka-s | fallback | 81 |  |  |  | plugin omeka/composer-addon-installer is not on vivacity's known-plugin list; package omeka/composer-addon-installer is a path package whose directory applicati |
| open-web-analytics | native | 20 | 1 | 1 | ext-* |  |
| opencart | native | 17 | 3 | 0 | ext-* |  |
| owncloud-core | native | 68 | 11 | 2 | ext-* |  |
| pelican-dev-panel | native | 167 | 7 | 2 | ext-* |  |
| phan | native | 24 | 2 | 0 | ext-* |  |
| phpdocumentor | native | 74 | 5 | 0 | ext-* |  |
| phplist-phplist3 | native | 34 | 6 | 0 | ext-* php |  |
| phpmyadmin | native | 34 | 2 | 0 | ext-* |  |
| phpro-grumphp | native | 52 | 2 | 0 | ext-* |  |
| phpstan-src | fallback | 67 |  |  |  | plugin ondrejmirtes/composer-attribute-collector is not on vivacity's known-plugin list |
| pimcore-skeleton | native | 187 | 12 | 2 | ext-* |  |
| pixelfed | native | 147 | 6 | 1 | ext-* |  |
| prestashop | native | 228 | 10 | 1 | ext-* |  |
| pterodactyl-panel | native | 109 | 5 | 1 | ext-* |  |
| roave-backwardcompatibilitycheck | native | 65 | 3 | 1 | ext-* |  |
| roots-bedrock | native | 15 | 43 | 1 | ext-* |  |
| rss-bridge | native | 0 | 0 | 0 | ext-* |  |
| salesagility-suitecrm-core | native | 210 | 21 | 3 | ext-* |  |
| sculpin | native | 50 | 4 | 0 | ext-* |  |
| sebastianbergmann-phpunit | native | 26 | 2 | 0 | ext-* |  |
| shaarli | native | 16 | 1 | 1 | ext-* |  |
| shopware-production | native | 174 | 10 | 2 | ext-* |  |
| silverstripe-installer | fallback | 88 |  |  |  | plugin silverstripe/recipe-plugin is not on vivacity's known-plugin list; plugin silverstripe/vendor-plugin is not on vivacity's known-plugin list; installers:  |
| slim-skeleton | native | 16 | 1 | 0 | ext-* |  |
| snipe-it | native | 167 | 7 | 2 | ext-* |  |
| spiral-app | native | 60 | 3 | 0 | ext-* |  |
| statamic | native | 123 | 9 | 1 | ext-* |  |
| sulu-skeleton | native | 148 | 8 | 1 | ext-* |  |
| symfony-demo | native | 99 | 4 | 1 | ext-* |  |
| symfony-skeleton | native | 1 | 1 | 0 | ext-* |  |
| thelia-project | native | 177 | 9 | 1 | ext-* |  |
| themosis | native | 113 | 13 | 0 | ext-* |  |
| typo3 | fallback | 75 |  |  |  | plugin typo3/class-alias-loader is not on vivacity's known-plugin list; plugin typo3/cms-composer-installers is not on vivacity's known-plugin list |
| typo3-cms-base-distribution | fallback | 105 |  |  |  | plugin typo3/class-alias-loader is not on vivacity's known-plugin list; plugin typo3/cms-composer-installers is not on vivacity's known-plugin list |
| unopim | native | 146 | 5 | 1 | ext-* php |  |
| uvdesk-community-skeleton | native | 1 | 0 | 0 | ext-* |  |
| wallabag | native | 200 | 5 | 1 | ext-* php |  |
| wordplate | native | 15 | 8 | 0 | ext-* |  |
| yiisoft-yii2-app-basic | native | 25 | 2 | 1 | ext-* |  |
| yourls | native | 14 | 1 | 0 | ext-* php |  |

## Scripts declared (the next cadrage)

92 of 106 entries declare `scripts`. Events, ranked:

- `post-update-cmd`: 46
- `post-install-cmd`: 42
- `test`: 39
- `post-create-project-cmd`: 37
- `post-autoload-dump`: 29
- `post-root-package-install`: 24
- `phpstan`: 20
- `lint`: 15
- `auto-scripts`: 11
- `pre-install-cmd`: 11
- `pre-update-cmd`: 10
- `cs:fix`: 8
- `rector`: 7
- `cs-fix`: 6
- `cs:check`: 6
- `dev`: 6
- `check`: 5
- `phpcs`: 5
- `cs`: 4
- `cs-check`: 4
- `pre-autoload-dump`: 4
- `pre-package-uninstall`: 4
- `psalm`: 4
- `serve`: 4
- `setup`: 4
- `test-coverage`: 4
- `analyse`: 3
- `fix`: 3
- `phpcbf`: 3
- `pint`: 3
- `start`: 3
- `tests`: 3
- `build`: 2
- `clear-config-cache`: 2
- `coverage`: 2
- `development-disable`: 2
- `development-enable`: 2
- `development-status`: 2
- `format`: 2
- `generate-assets`: 2
- `lint:fix`: 2
- `npm-ci`: 2
- `npx-patch-package`: 2
- `php-cs-fixer`: 2
- `phpstan-baseline`: 2
- `phpstan:generate-baseline`: 2
- `phpunit`: 2
- `qa`: 2
- `rector:check`: 2
- `rector:fix`: 2
- `static-analysis`: 2
- `test:unit`: 2
- `addons:cs:check`: 1
- `addons:cs:fix`: 1
- `addons:phpstan`: 1
- `addons:qa`: 1
- `addons:rectify`: 1
- `addons:rector:check`: 1
- `addons:rector:run`: 1
- `addons:test`: 1
- `analyse:phpmd`: 1
- `analyse:phpmd:baseline`: 1
- `analyse:phpstan`: 1
- `analyse:phpstan:baseline`: 1
- `analyze`: 1
- `api`: 1
- `api-module-tests`: 1
- `api-module-tests-local`: 1
- `api:docs`: 1
- `api:docs:check`: 1
- `asset-map:clean`: 1
- `bootstrap-test-environment`: 1
- `build-licenses`: 1
- `bundles`: 1
- `check-static`: 1
- `check-test-db`: 1
- `cleanup-vendor`: 1
- `code-check`: 1
- `codestyle`: 1
- `codestyle-fix`: 1
- `compat`: 1
- `compile`: 1
- `compile-plurals`: 1
- `coverage:herd:clover`: 1
- `coverage:herd:html`: 1
- `create-release`: 1
- `create-test-db`: 1
- `create-test-table-dumps`: 1
- `db:update-structure`: 1
- `delete-cache`: 1
- `demo-database`: 1
- `disable-maintenance`: 1
- `docker:translate`: 1
- `drupal-phpunit-upgrade`: 1
- `drupal-phpunit-upgrade-check`: 1
- `e2e-test`: 1
- `enable-codestandard`: 1
- `enable-maintenance`: 1
- `fixcs`: 1
- `gerrit:setup`: 1
- `gerrit:setup:commitMessageHook:enable`: 1
- `gerrit:setup:preCommitHook:disable`: 1
- `gerrit:setup:preCommitHook:enable`: 1
- `git-hook-install`: 1
- `git-hook-uninstall`: 1
- `githooks`: 1
- `gjs-build`: 1
- `gjs-build-safe`: 1
- `header-stamp`: 1
- `header-stamp-fix`: 1
- `install:prod`: 1
- `install_files`: 1
- `integration-behaviour-tests`: 1
- `integration-test`: 1
- `integration-tests`: 1
- `koel:init`: 1
- `lang:recreate`: 1
- `lint-admin-build`: 1
- `lint-composer`: 1
- `lint-container`: 1
- `lint-doctrine`: 1
- `lint-php-cs`: 1
- `lint-rector`: 1
- `lint-translations`: 1
- `lint-twig`: 1
- `lint-twig-cs`: 1
- `lint-xliff`: 1
- `lint-yaml`: 1
- `lint:test`: 1
- `linting`: 1
- `mezzio`: 1
- `motd`: 1
- `openapi`: 1
- `patch`: 1
- `pcf`: 1
- `php-cs-fix`: 1
- `php-cs-fixer-fix`: 1
- `php-cs-fixer:dry`: 1
- `php-lint`: 1
- `phpcs-fix`: 1
- `phpmd`: 1
- `phpstan-framework`: 1
- `phpstan-next`: 1
- `phpstan-plugins`: 1
- `phpstan-src`: 1
- `phpstan-tests`: 1
- `phpstan:baseline`: 1
- `phpstan:migration`: 1
- `phpstan:templates`: 1
- `phtml-lint`: 1
- `pimcore-scripts`: 1
- `post-cmd`: 1
- `post-package-install`: 1
- `post-package-update`: 1
- `pre-commit`: 1
- `prod`: 1
- `psalm:baseline`: 1
- `psalm:fix`: 1
- `psalm:ncu`: 1
- `psalm:ocp`: 1
- `psalm:security`: 1
- `psalm:strict`: 1
- `psalm:update-baseline`: 1
- `rectify`: 1
- `rector-dry`: 1
- `rector:dry`: 1
- `rector:php-compat`: 1
- `rector:run`: 1
- `rector:strict`: 1
- `refresh-test-database`: 1
- `restore-test-db`: 1
- `restore-test-tables`: 1
- `rr:download`: 1
- `rr:download-protoc`: 1
- `run-api-module-tests-local`: 1
- `sass:build`: 1
- `sass:watch`: 1
- `setConfigParams`: 1
- `setup-geoip`: 1
- `spec`: 1
- `spec:refresh`: 1
- `stan`: 1
- `stan-baseline`: 1
- `static`: 1
- `static-analysis-update-baseline`: 1
- `syntax`: 1
- `t-reset`: 1
- `test-all`: 1
- `test-php`: 1
- `test-php-all`: 1
- `test-windows`: 1
- `test-with-coverage`: 1
- `test:configless`: 1
- `test:db`: 1
- `test:files_external`: 1
- `test:filter`: 1
- `test:functional`: 1
- `test:integration`: 1
- `test:isolation`: 1
- `test:legacy`: 1
- `test:lint`: 1
- `test:pest`: 1
- `test:phpstan`: 1
- `test:quick`: 1
- `test:scratch`: 1
- `test:templates`: 1
- `test:tia`: 1
- `test:tia:parallel`: 1
- `test:types`: 1
- `test:types-new`: 1
- `test:upgrade`: 1
- `tester`: 1
- `tests-integration`: 1
- `tests-unit`: 1
- `thelia-install`: 1
- `translate`: 1
- `translations`: 1
- `twig`: 1
- `twig-cs-fix`: 1
- `twig-lint`: 1
- `twig:fix`: 1
- `twigcs`: 1
- `twigcs-fix`: 1
- `unit-test`: 1
- `unit-tests`: 1
- `update-snapshots`: 1
- `update:baselines`: 1

## Platform requirements this machine failed

- `php ^5.6.20 || ^7.0.0 || ~8.0.0 || ~8.1.0 || ~8.2.0 || ~8.3.0 || ~8.4.0`: 4
- `php ~8.1.0 || ~8.2.0 || ~8.3.0 || ~8.4.0`: 4
- `php >=7.4.0 <8.5.0`: 3
- `php ~8.1.0 || ~8.2.0 || ~8.3.0`: 3
- `ext-imap *`: 2
- `php ^5.3.3 || ^7.0`: 2
- `php ^5.6 || ^7.0`: 2
- `php ^7.0`: 2
- `php ^7.4 || ~8.0.0 || ~8.1.0 || ~8.2.0 || ~8.3.0`: 2
- `php ~8.2.0 || ~8.3.0 || ~8.4.0`: 2
- `ext-amqp *`: 1
- `php 8.1 - 8.4`: 1
- `php >=7.1 <8.2`: 1
- `php >=7.2 <8.2`: 1
- `php >=7.4, <8.2`: 1
- `php ^7.2 || ~8.0, <8.2`: 1
- `php ^8.4`: 1
- `php ~5.5|~7.0`: 1
- `php ~5.6.0 || ~7.0.0 || ~7.1.0 || ~7.2.0 || ~7.3.0 || ~7.4.0 || ~8.0.0 || ~8.1.0 || ~8.2.0`: 1
- `php ~7.3.0 || ~7.4.0 || ~8.0.0 || ~8.1.0 || ~8.2.0 || ~8.3.0 || ~8.4.0`: 1
- `php ~8.0.0 || ~8.1.0 || ~8.2.0 || ~8.3.0`: 1

