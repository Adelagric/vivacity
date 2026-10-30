# Trois écarts de parité avant 0.26, et l'inventaire de ceux qui restent — plan

> Niveau 2 : contrat de parité (stderr à l'octet, codes de sortie, rien
> d'écrit avant une délégation). Méta-analyse et revue par sous-agents frais.
> Branche `parity-gaps-026`, depuis c876c38 (candidat 0.26). Référence :
> Composer 2.10.3 installé (`/opt/homebrew/bin/composer`), sources lues dans
> le phar extrait hors dépôt et dans `docs/reference/`.

## Contraintes posées par le mainteneur

- Une décision de délégation ne lit que ce que vivacity a déjà en mémoire
  (manifeste, lock, configuration, URL de dists et de dépôts) : aucune
  requête et aucune lecture disque de plus sur le chemin commun. Le test
  `secure-http` est un test de chaîne sur des URL déjà lues.
- Corriger plutôt que déléguer quand la parité se prouve ; la délégation
  reste pour le rare ou l'irreproductible.
- Aucun chiffre mesuré dans les docs du dépôt (ni durée, ni compte de corpus
  nouveau) : ils vont au rapport.
- Corpus (`harness/corpus.sh`, deux modes) inchangé à la fin ; une
  délégation nouvelle qui toucherait un projet du corpus est signalée avant
  toute décision.

## Ce que Composer 2.10.3 fait (lu, puis mesuré)

### L'ordre de `doUpdate` jusqu'à « Updating dependencies »

`Installer::doUpdate` (Installer.php:512, :534, :536) : « Loading composer
repositories with package information », puis `createPool` →
`PoolBuilder::buildPool` (PoolBuilder.php:208-366), puis « Updating
dependencies ». Dans `buildPool`, tout est écrit au moment où ça arrive :

1. `warnAboutNonMatchingUpdateAllowList` (:216), avant tout chargement ;
2. le chargement des paquets : ce que les dépôts écrivent en chargeant (le
   dépôt dégradé « could not be fully loaded », l'avertissement de
   `Config::prohibitUrlByConfig` à la première requête vers un hôte) mêlé aux
   avertissements « Dependency … is also a root requirement » (:539, :569) ;
3. `PRE_POOL_CREATE` (:328) : Flex y lit son index (requête, donc
   l'avertissement http de son hôte) puis écrit « Restricting packages… » ;
4. `runSecurityAdvisoryFilter` (:358) puis `runFilterListFilter` (:359) :
   leurs requêtes, puis leurs avertissements « … could not be fetched … »
   (SecurityAdvisoryPoolFilter.php:82-85, FilterListPoolFilter.php:173-176) ;
5. l'optimiseur, puis « Updating dependencies ».

Mesuré sur 2.10.3 :

- `update vendor/zzz` sur un dépôt `http://` avec `secure-http: false` :
  la ligne de la liste d'update (1) PRÉCÈDE « Warning: Accessing 127.0.0.1
  over http… » (2) ;
- `update` de solver-policies avec un `security-advisories.api-url` en
  `http://` (secure-http par défaut) : « Security advisory data could not be
  fetched … » et sa ligne `  - Your configuration does not allow
  connections to http://…` AVANT « Updating dependencies ». vivacity
  (c876c38) l'écrit après, avec un autre message (le transport `file://`
  refuse le POST) ;
- les bancs Flex (flex-update, flex-recipes) : l'avertissement http de
  l'index tombe juste avant « Restricting packages… ».

vivacity (c876c38) : avertissements du constructeur ET des filtres après
« Updating dependencies » ; les lignes de dépôt (dégradé) au moment du
chargement.

### `secure-http`

`Config::prohibitUrlByConfig` (Config.php:692-738) : pour une URL valide (ou
`^https?://`) de schéma `http`, `git`, `ftp` ou `svn`, `secure-http` vrai →
`TransportException` « Your configuration does not allow connections to
<Url::sanitize>. See https://getcomposer.org/doc/06-config.md#secure-http
for details. » ; faux → « Warning: Accessing <hôte> over <schéma> which is
an insecure protocol. », une fois par hôte et par instance de `Config`.
`secure-http` (Config.php:415-425) : faux si `disable-tls` est vrai, sinon
`$val !== 'false' && (bool) $val` ; pas de variable `COMPOSER_*` à elle ;
couches config.json global, auth.json global, `config` du projet, auth.json
du projet, puis `COMPOSER_AUTH` (Factory.php:209-215, :334-344 — la méta l'a
relevé, mesuré : `COMPOSER_AUTH='{"secure-http": false}'` change le refus en
avertissement).

Appelants : `CurlDownloader::initDownload` (:181, toute requête http(s), sauf
`^http://(repo\.)?packagist\.org/p/` avec `$`), `RemoteFilesystem::get`
(:287), `Git::runCommand` (:109), hg/svn/fossil. `HttpDownloader::startJob`
répond AVANT curl sous `COMPOSER_DISABLE_NETWORK` (:328-338) : ni refus ni
avertissement hors ligne. Un dist servi par le cache de fichiers ne fait
aucune requête : ni refus ni avertissement.

Mesuré (dépôt `composer` local en `http://`, dists `http://`) :

| cas | Composer 2.10.3 |
|---|---|
| `update`, défaut | « Loading… » puis l'exception sur `packages.json`, code 100 |
| `update`, `false` | l'avertissement juste après « Loading… » |
| `install` du lock, dépôt http + dists http, défaut | « Verifying… », l'avertissement du filtre de listes (cause : le refus de `packages.json`), « Package operations », deux « Downloading », exception sur le premier dist, code 100 |
| `install`, sans dépôt http, dists http, défaut | même chose sans l'avertissement du filtre |
| `install`, dépôt http, `false` | l'avertissement après « Verifying… » (requête du filtre de listes) |
| `install`, sans dépôt http, dists http, `false` | l'avertissement APRÈS la première ligne « - Downloading » (la requête part dans la boucle synchrone de `FileDownloader`) |
| `update vendor/zzz`, `false` | ligne de liste d'update, puis l'avertissement |
| `require acme/a` (sans contrainte), `false` | l'avertissement deux fois : à la découverte de version, et après « Loading… » (seconde instance de `Composer`) |
| `remove acme/b`, `false` | aucune requête, aucun avertissement |

vivacity (c876c38) télécharge tout, n'écrit jamais l'avertissement ; les
harnais `auth-hosts.sh`, `flex-update.sh`, `flex-recipes.sh`,
`plugin-lines.sh` filtrent la ligne.

## Le design

### Points 1 et 2 : écrire au moment où Composer écrit

Plutôt que porter tel quel 53d5306 (qui imprime les avertissements du
constructeur en bloc après la construction du pool), le constructeur écrit
chaque avertissement au moment où il le produit (`vivacity_core::say!` dans
`pool.rs`, comme `writeError`), ce qui le met à sa place parmi les lignes que
les dépôts écrivent pendant le chargement (dépôt dégradé aujourd'hui,
avertissement http demain) — le port en bloc les remettrait toutes avant ou
toutes après. `Pool::warnings` disparaît. Le constructeur ne tourne qu'à deux
endroits (`UpdateSession::create_pool`, et `LockedPool::build` de la
vérification d'install, sans liste d'update ni racine à garder verrouillée :
il n'y écrit rien).

Puis, dans `create_filtered_pool` : la notice de Flex (`PRE_POOL_CREATE`)
écrite après la construction, précédée de l'avertissement http de son index
s'il est dû ; le filtre d'avis écrit ses avertissements sitôt fini, puis le
filtre de listes les siens ; `update` n'écrit plus que « Updating
dependencies » après.

### Point 3 : `secure-http`

Un module `vivacity_core::secure_http` :

- `insecure(url) -> Option<(schéma, hôte)>` : la condition de
  `prohibitUrlByConfig` (URL valide ou `^https?://`, schéma en minuscules
  dans http/git/ftp/svn), l'exception de `CurlDownloader` pour
  `http://(repo.)?packagist.org/p/…$` ;
- `secure_http_from(config du projet, config.json global)` : `disable-tls`
  vrai → faux ; sinon `$val !== 'false' && (bool) $val`, défaut vrai ;
- l'état d'une exécution : projet (pour lire la config à la demande, seulement
  quand une URL non sûre se présente), hors ligne ou non, hôtes déjà avertis ;
  `reset` à chaque `Composer` que Composer crée (début de commande ; avant
  « Loading composer repositories » d'une résolution) ;
- `request(url) -> Result<(), String>` : hors ligne → rien ; URL non sûre et
  `secure-http` vrai → `Err(message de Composer)` ; faux → l'avertissement,
  une fois par hôte, sur le canal de l'appelant.

Points de contrôle :

- **Délégation, avant toute écriture et toute requête, sur ce qui est en
  mémoire** (`secure-http` vrai seulement) :
  - `update`/`require`/`remove` (et `install` sans lock qui résout) : l'URL
    d'un dépôt `composer` du manifeste dans `resolution_fallback` (avant que
    `require`/`remove` n'éditent composer.json), celle d'un dépôt du
    config.json global à la création de la session (`MergedConfig`, déjà
    lue : erreur `Unsupported`, délégation nommée avant « Loading… »), les
    points d'entrée de Flex quand Flex lira son index (le contrôle des
    dépôts `package` prévu ici est sans objet : `update` refuse ce type,
    écart inventorié) ;
  - `install` du lock : un dist non sûr d'un paquet du lock dans le périmètre
    (`run.lock`, en mémoire) → délégation avant la première ligne.
- **Natif, message de Composer** (`secure-http` vrai, URL découverte en
  route) : les quatre points de requête de `ComposerRepository` passent par
  `request` ; le refus est une erreur de transport, que les filtres
  d'avis/de listes rattrapent sous `ignore-unreachable` exactement comme
  Composer (cas `api-url` ci-dessus, et le dépôt http d'un `install`) ;
  ailleurs elle sort en 100 comme toute erreur de transport (encadré de
  Composer contre `Error: …`, la convention acceptée de `transport-exit.sh`).
  Le récupérateur (`Fetcher`) refuse aussi en dernier recours, redirections
  comprises (politique de redirection de reqwest).
- **L'avertissement** (`secure-http` faux) : dépôts — au point de requête
  (écrit tout de suite, ou retenu avec les autres lignes du dépôt quand il
  retient sa sortie, comme la vérification d'install) ; dists — dans la passe
  synchrone des lignes « - Downloading », juste après la ligne du paquet ;
  index de Flex — retenu jusqu'à `PRE_POOL_CREATE`. Sources git : `http` et
  `git` sont déjà rendus à Composer par `git_source::plan`, rien ne change.

### Ce qui reste hors du design (écrit comme écart ou non vérifié)

- Une redirection d'`https` vers une URL non sûre (refus, message de
  Composer pour la cible) et la limite de 20 redirections : portées, non
  exercées (pas de serveur `https` local). La redirection `http` → `http`
  vers un nouvel hôte est mesurée (`dist-redirect`).
- Une recette de Flex servie par un autre hôte non sûr que son point
  d'entrée : rendue (place non reproduite).
- Un dépôt `file://` dont packages.json pointe vers des URL http(s) : le
  transport fichier de vivacity ne sait pas les lire (écart antérieur ;
  sous `secure-http` vrai, le refus de Composer est reproduit).

## Cas de harnais (chacun vu rouge sur c876c38, sauf les témoins nommés)

- `verbosity.sh` (comparaison entière sous normal, `-q`, `-v` ;
  `@registry-jq`, `@warns`, `VERBOSITY_ONLY`) : `laravel update
  vendor/not-locked`, `laravel update laravel/tinker -w` (exigence racine
  gardée, par les `replace` de laravel/framework), `solver-policies update`
  avec l'`api-url` d'avis en http (points 2 et 3). Rouges au niveau normal
  et sous `-v` ; `-q` passait (rien n'est écrit).
- `secure-http.sh` (nouveau, mandataire `http_proxy` local) : 32 cas (53
  après la revue du 2026-10-30, section suivante), 27
  rouges ; témoins verts avant comme après (cache chaud, `--dry-run`, hors
  ligne ×2, `remove` à cache chaud).
- `flex-update.sh` : `allow-list` (index lu à `POST_UPDATE_CMD`),
  `symfony-arg` et `remove-arg` (index lu à l'activation de Flex,
  `PackageResolver::resolve` sur les arguments — trouvé par `steps.sh`, dont
  le cas `symfony remove symfony/uid` portait l'avertissement en trop à
  `POST_UPDATE_CMD` : Composer l'avait écrit en tête, une fois par
  `Composer`), `secure-default` (rendu) ; rouges, comme tous ses cas une
  fois le filtre retiré.
- Filtres retirés : `auth-hosts.sh` (rouge sur c876c38 sans eux),
  `flex-update.sh`, `flex-recipes.sh`, `plugin-lines.sh`.

## Critère « vert »

Gates (build, clippy, fmt, test) ; les cas ci-dessus rouges sur c876c38,
verts après ; `steps.sh`, `update.sh`, `verbosity.sh`, `auth-hosts.sh`,
`plugin-lines.sh`, `allow-plugins.sh`, `flex-update.sh`, `flex-recipes.sh`,
`transport-exit.sh`, `secure-http.sh` verts ; corpus deux modes inchangé ;
hyperfine avant/après (rapport seulement).

## Méta-analyse (sous-agent frais, sur le code) et ce qui a changé

Seize points ; ceux qui ont changé le design :

- **Où et quand l'avertissement s'écrit.** vivacity n'envoie pas ses
  requêtes aux moments de Composer (l'index de Flex avant « Loading… », les
  recettes avant l'écriture du lock, la vérification d'`install` sur un
  fil) : la décision « dû » est prise à la requête, l'écriture aux points de
  Composer ; le `Fetcher` ne fait que refuser, jamais avertir.
- **Flex lit son index paresseusement** (`PackageFilter.php:91-93` :
  `$knownVersions ??= getVersions()` seulement pour un paquet hors
  `symfony/symfony` et pas à sa version verrouillée) : `remove_legacy_packages`
  dit s'il l'aurait lu, l'avertissement de `PRE_POOL_CREATE` n'est écrit
  qu'alors ; sinon c'est `POST_UPDATE_CMD` (`fetchRecipes` →
  `initialize`), mesuré par `plugin-lines.sh` (`ni-flex`) et le nouveau cas
  `allow-list` de `flex-update.sh`.
- **`secure-http` vient aussi d'auth.json et de `COMPOSER_AUTH`**
  (`Factory.php:209-215`, `:334-344`), mesuré : rendu à Composer.
- **Le plafond de `HttpDownloader`** (12 tâches, `COMPOSER_MAX_PARALLEL_HTTP`) :
  mesuré, le 13e téléchargement d'un nouvel hôte est averti après la 14e
  ligne ; porté.
- **`disable-tls`** fait plus que couper `secure-http` (pas de TLS, une
  ligne à `createHttpDownloader`) : rendu à Composer quand une URL non sûre
  se présente ; en général, écart inventorié ci-dessous.
- **Le contrôle `install` avant toute ligne** : après `read_manifests`,
  avant l'événement `COMMAND` de bamarni et `pre-install-cmd`.
- **Les dépôts `package`/`vcs` d'`update`** sortent en 1 (« not supported
  by vivacity update yet ») au lieu d'être rendus : hors de ce chantier,
  inventorié.
- Le banc : les cas d'ordre vont dans `verbosity.sh` (comparaison entière,
  trois niveaux) plutôt qu'un drapeau de plus dans `steps.sh`.

## Revue indépendante (sous-agent frais) et ce qui a changé

Dix-huit points ; corrigés :

- `COMPOSER_DISABLE_NETWORK` ne compte plus comme hors ligne pour
  `secure-http` : les transports de vivacity ne le lisent pas (écart
  existant, inventorié), le refus doit donc tenir.
- `COMPOSER_AUTH` l'emporte sur le projet (fusionné en dernier) : lu avant
  lui, rendu à Composer s'il pose la clé ; cas `dist-auth-env-wins`.
- Une redirection refusée donne le message de Composer pour la cible
  (`Error::Transport`, sortie 100), une redirection vers un nouvel hôte non
  sûr (`secure-http` faux) son avertissement après les lignes
  « Downloading » (cas `dist-redirect`) ou à la fin de la requête de
  métadonnées ; limite de 20 redirections (`CurlDownloader::$maxRedirects`).
- Le contrôle des dists ne compte pas un paquet déjà installé à sa version
  et sa référence, ni l'URL d'un miroir non préféré ; il ne lit installed.json
  qu'en présence d'une URL non sûre.
- Le plafond ne compte que les téléchargements `http(s)://` (un dist local
  passe par `RemoteFilesystem`, sans tâche curl).
- Flex sous `--dry-run` sans `extra.symfony.require` ne lit pas son index :
  pas de délégation.
- Des cas : `--offline`, auth.json du projet, `COMPOSER_AUTH` contre le
  projet, redirection, dépôt et dists sur le même hôte, Flex refusé.

Laissés, écrits : l'`svn` (jamais demandé par vivacity), le `realpath` de
composer.json pour trouver auth.json, l'avertissement `verify_peer` de la
même fonction (inventorié), une délégation tardive après des lignes déjà
écrites quand un réglage illisible (auth.json…) n'apparaît qu'au
chargement (la famille connue de HANDOVER).

## Revue de ebd53b1 : neuf défauts, et ce qui en est fait

Revue indépendante (coordinateur) de ebd53b1, six défauts reproduits contre
Composer 2.10.3. Corrigés dans des commits au-dessus de ebd53b1. Ordre :
le banc d'abord (point 9 : tel qu'écrit, il cachait les points 1 à 3), puis
chaque point vu rouge par un cas comparé à Composer, puis corrigé.

Règles inchangées : rendu avant toute ligne, requête ou écriture (cache
compris) quand c'est décidable ; porté exactement quand c'est prouvé ;
chemin tout-`https` sans lecture disque ni verrou de plus.

1. **`remove` et un dépôt `http://` du config.json global** : décidé dans
   `resolution_fallback`, avant l'édition, sur la liste fusionnée
   (`MergedConfig::load` : config.json global puis composer.json, avec les
   désactivations par nom). Le config.json global est déjà lu au démarrage
   (répertoire de cache) : il est gardé en mémoire pour le run
   (`fetch::global_files`, oublié au début de chaque run), et
   `MergedConfig::load`, le contrôle du répertoire de cache et le réglage
   `secure-http` le relisent de là — moins de lectures qu'avant.
2. **Dists `http://` du lock résolu** : choisi de **documenter la
   délégation tardive** pour un dépôt distant, pas de la rendre précoce.
   Ses dists ne sont connus que par ses métadonnées, donc après leurs
   requêtes et l'écriture de leur cache — aucune stratégie d'affichage ne
   rend vraie l'affirmation « avant toute requête ». Retenir toute la
   sortie de « Loading… » à la fin de la résolution sur chaque `update`
   pour cacher ce cas rare retarderait toutes les lignes du chemin commun.
   Pour un dépôt **local** (le cas du banc, `update-file-repo`), les dists
   se lisent sans requête (méta, point 1) : rendu avant tout, dans
   `resolution_fallback`, quand la commande installe. CHANGELOG, HANDOVER
   et DECISIONS corrigés ; le cas distant va dans la liste des délégations
   tardives de HANDOVER (non vérifiable ici : il faut un dépôt `https`).
3. **`secure-http` lu dans auth.json ou `COMPOSER_AUTH`** : **porté** au
   lieu d'être rendu. `Factory` fusionne sous `config` : config.json global,
   auth.json global, `COMPOSER_AUTH`, composer.json, auth.json du projet,
   `COMPOSER_AUTH` encore (Factory.php:209-215, :325-344) — la dernière
   couche qui pose la clé gagne, `Config::merge` remplace un scalaire,
   `null` compris. Plus de délégation sur cette raison, donc plus de
   délégation tardive au chargement. Reste rendu : un fichier ou
   `COMPOSER_AUTH` illisible (Composer s'arrête), `disable-tls` (point 7).
4. **Un refus trouvé au chargement que Composer ne rattrape pas**
   (`asyncFetchFile` : l'avertissement du dépôt dégradé, puis le cache s'il
   y en a un, sinon l'exception — l'encadré) : (a) dépôt `composer` local
   (`file://`, chemin) : son packages.json est lu dans
   `resolution_fallback` (une lecture locale, hors chemin tout-`https`) :
   toute URL qu'il nomme et qu'un run demanderait pour charger des paquets
   (`metadata-url`, URL v1, `providers-api`, `includes`, miroirs) refusée
   rend la commande avant tout — pas les URL des filtres de politique
   (avis, listes), dont Composer rattrape le refus sous
   `ignore-unreachable` (le défaut ; vu par `verbosity.sh`, cas
   `solver-policies`, quand la revue les avait ajoutées) ; (b) sinon (l'URL n'est connue
   qu'après une requête) : un refus qui remonte jusqu'à la session est un
   refus que Composer ne rattrape pas (`settle` a déjà servi le cache là
   où Composer le sert : asynchrone avec `Last-Modified`, synchrone avec
   une copie ; les filtres ont déjà rattrapé sous `ignore-unreachable`) →
   `Unsupported`, délégation tardive documentée.
5. **Redirection refusée sur un dist** : rendue (`Unsupported`) au lieu de
   `Error:` ; tardive par nature (connue à la réponse). Les dists que le run
   a mis dans le cache de fichiers sont retirés (second tour : Composer les
   y aurait trouvés et n'aurait pas écrit leur « Downloading »).
   Non vérifiable localement : vivacity ne fait confiance qu'aux racines
   webpki embarquées (pas de CA locale de test). Écrit non vérifié.
6. **L'exemption des paquets déjà installés** : `installed_as_locked`
   (`package_identity` : version, deux références, `abandoned`) sur
   installed.json purgé comme `local_repository_transaction` le purge
   (`purge_missing`, puis le chemin de l'installeur du run).
7. **`disable-tls` vrai sans URL non sûre** : rendu au démarrage, pour
   toutes les commandes (`config` comprise : mesuré, la ligne sort dès
   qu'un composer.json existe — `Application::getPluginCommands` crée le
   `Composer`), sur les couches déjà en mémoire (`COMPOSER_AUTH`,
   config.json et auth.json globaux gardés, composer.json lu par le
   contrôle du répertoire de cache ; `dump-autoload` à la lecture de son
   manifeste, avant le refus d'une racine invalide : le téléchargeur est
   créé avant le paquet racine, Factory.php:370, :394). Un `COMPOSER_AUTH`
   qui n'est pas un objet JSON est rendu au même endroit. Écart laissé,
   écrit : `disable-tls` posé dans le seul auth.json du projet sans URL non
   sûre (le lire au démarrage ouvrirait un fichier de plus à chaque run).
8. **Flex** : l'avertissement d'activation est retenu après les lignes de
   `Factory` et des plugins (`io::hold_after_factory`, qui ajoute désormais
   à la suite des lignes de plugins déjà retenues), donc avant celles de
   `COMPOSER_IGNORE_PLATFORM_REQ(S)` et `--no-suggest` ; plus de condition
   sur le téléchargeur désactivé (`Downloader::disable` ne vide que
   `$enabled`, `initialize` part quand même).
9. **Le banc** : `secure-http.sh` exige, pour un cas rendu, une stderr vide
   avant la raison (sauf `late`), et sous `--no-fallback` ni requête, ni
   écriture, ni cache (fichiers, store) ; `flex-update.sh` exige la ligne
   « Accessing » côté Composer (`allow-list`, `symfony-arg`, `remove-arg`)
   et compare toute la sortie de `secure-default`.

Petits points : `prefetch` sans allocation de plus quand tout est `https`
(test de chaîne d'abord) ; le miroir préféré essayé en premier est le
dernier (`array_unshift`, Package.php:700-702) ; l'hôte d'une redirection
pris par reqwest en minuscules au lieu du `Location` tel qu'écrit
(CurlDownloader.php:553-555) : la politique de redirection de reqwest ne
voit pas l'en-tête — écart inventorié, pas porté.

### Méta (sous-agent frais, sur le code) et revue indépendante : ce qui a changé

Méta, onze points ; retenus : les dists d'un dépôt local se lisent sans
requête (point 2 précoce pour lui) ; booléens JSON seulement (le schéma de
Composer signale ou refuse les autres) ; `COMPOSER_AUTH` « 0 » n'est pas
vide en 2.10.3 (mesuré : l'avertissement de schéma, plusieurs fois) ; un
point d'entrée de Flex refusé était perdu par l'avertissement d'activation
et `remove` le rendait après son édition ; `config` aussi écrit la ligne
de TLS ; Flex décidé avant `prepare_update` ; un second compteur dans
`io` (pas `FACTORY_HELD`, que `factory_held` rejoue) ; la redirection
refusée laissait un `vendor/` créé ; une seule fonction de purge ; plus
d'URL dans le contrôle local, la reprise asynchrone sur `Last-Modified` ;
le mémo par chemin et oublié au début du run ; le miroir de
`download_lines`. Laissé : l'auth.json du projet au démarrage (règle de
coût), le `realpath` de composer.json (inventorié).

Revue indépendante, huit points ; corrigés : la découverte de version de
`require` aplatissait la délégation en `Error:` ; `dump-autoload` jugeait
`disable-tls` après la racine ; le contrôle local ignorait les URL des
inclusions v1 (celles des filtres aussi, retirées ensuite : `verbosity.sh`
les a vues rendre un cas que Composer mène à bien), parcourait l'arbre sans installer, suivait
les liens ; le remède nommé pour toute raison de démarrage ; le mémo
oublié trop tard ; un config.json global illisible pris pour absent ; un
`vendor-dir` imbriqué laissait ses parents ; l'ordre exact de `getUrls`
(`in_array`). Laissé, écrit : l'auth.json du projet (même raison), le
chemin Windows du transport fichier (inventorié).

Critère « vert » de ce tour : chaque cas nouveau vu rouge sur ebd53b1 puis
vert ; gates ; `secure-http.sh`, `flex-update.sh`, `auth-hosts.sh`,
`transport-exit.sh`, `steps.sh`, `update.sh`, `verbosity.sh`,
`flex-recipes.sh`, `flex-install.sh`, `download-lines.sh`,
`fallback-args.sh`, `allow-plugins.sh`, `plugin-lines.sh` verts ;
hyperfine ebd53b1 contre le final (rapport seulement). Pas de corpus (le
coordinateur le passe).

## Second passage de revue (2026-10-30)

Sept points confirmés, le point 5 en partie, trois nouveaux :

- **N1, vitesse.** `local_insecure_url` parcourait et analysait tout
  l'arbre du `metadata-url` d'un dépôt local à chaque `update`/`require`/
  `remove` qui installe, même sous `secure-http` faux. Corrigé : rien lu si
  le réglage ne peut pas refuser ; sinon packages.json, puis les seuls
  fichiers des paquets que la résolution lit à coup sûr (exigences racine,
  verrouillés, arguments de `require`). Un dist `http://` qu'amène une
  dépendance nouvelle se voit une fois le lock résolu : délégation
  tardive, comparée par le cas `late-transitive`. Garde : le cas
  `update-file-repo-unrelated` (un dépôt qui sert d'autres paquets en
  `http`, que Composer ne lit pas : natif exigé), rouge sur a1b6684.
  Mesuré (chiffres au rapport).
- **N2, cache de fichiers.** Les archives qu'un run a mises dans le cache
  de Composer avant une délégation sont retirées (Composer, les trouvant,
  n'écrirait pas leur « Downloading »). Non exercé (redirection `https`).
- **D1.** README aligné (l'auth.json du projet).
- **Délégations tardives.** (2) l'`api-url` d'avis et les URL de listes
  d'un dépôt local sous `ignore-unreachable` faux : décidées avant tout
  (`meta-advisory-fatal`, rouge sur a1b6684). (1) dépôt distant (dists du
  lock résolu, métadonnées refusées) et (3) redirection pendant les
  téléchargements : **retenir la sortie n'est pas fait, le compromis est
  rapporté** — les octets et la durée totale seraient les mêmes, mais la
  première ligne attendrait la fin de ce qui décide (mesuré, chiffres au
  rapport) sur chaque
  `update` avec installation sous `secure-http` vrai ; pour (3), l'en-tête
  de l'installation attendrait la fin des téléchargements. À trancher par
  le mainteneur. Ces chemins restent tardifs et non comparés faute de
  dépôt `https` local (aucun cas `late` possible pour eux) ; le seul cas
  tardif exerçable, `late-transitive`, est comparé.

## Troisième passage de revue (2026-10-30)

- **F1, lecture trop large.** Le contrôle local lisait le fichier de chaque
  paquet verrouillé et jugeait chaque version : `update acme/l` (acme/x
  fixé par Composer, son fichier non lu) et `update` (acme/x 1.0.0, dist
  `http://`, hors de `^2.0`) étaient rendus à tort. Corrigé : le départ et
  les paquets fixés de `PoolBuilder::buildPool`, les versions qu'admettent
  la contrainte et les stabilités. Cas `partial-locked-http`,
  `full-outside-constraint`, rouges sur fb31ab6.
- **F4, fermeture.** Les `require` des versions chargées sont suivis sur
  les fichiers locaux : `late-transitive` devient `transitive-file-repo`,
  rendu avant tout (rouge sur fb31ab6 : deux lignes, cache et store).
  Reste tardif : une dépendance qu'un paquet d'un autre dépôt amène.
- **F2, `includes` locaux.** Relus (régression du second passage) : cas
  `satis-includes`, rouge sur fb31ab6.
- **F3, cache de fichiers.** Toute archive écrite par le run (installation,
  lecteur de Flex) est notée et retirée avant toute délégation ; un retrait
  raté arrête le run sur cette erreur. Non exerçable par un banc ; test
  unitaire. Le filtre « Downloading » de `flex-update.sh` reste (cache
  partagé entre les deux côtés).
- **F5, `allow_ssl_downgrade`.** Rendu avant tout (cas `ssl-downgrade`,
  rouge sur fb31ab6) plutôt que porté (le retour en `https` sur une
  empreinte `sha256` fausse serait à porter aussi).
- **Retenue de la sortie** : non faite, décision du mainteneur (le
  relecteur propose un déclencheur étroit : un dépôt `composer` distant
  autre que packagist.org, sous `secure-http` vrai).

## Quatrième passage de revue (2026-10-30)

Aucune rupture de contrat. Corrigés : G1 (drapeaux de stabilité implicites,
alias de branche et racine), G2 (paquet figé non installé, jugé sur le
lock), G3 (déverrouillage `-w`/`-W` au chargement), G4 (une version jugée
une fois, sans copie, et les fichiers lus par le contrôle gardés analysés
pour le dépôt qui les relit : banc du relecteur, `secure-http`
vrai au niveau de faux), G5 (retrait
raté : `vendor/` rendu, `Error:` et sortie 1), G6 (cas `late` vivant,
`late-path-dependency` ; cas Flex qui appelle Composer, `dist-then-recipe`).
Cas rouges sur f4de2f8 puis verts : `dev-implicit-flag`, `dev-branch-alias`,
`dev-root-alias`, `partial-fixed-missing`, `transitive-unlock` ;
`dist-then-recipe` rouge sur fb31ab6.

## Sortie retenue (2026-10-30, décision du mainteneur)

Déclencheur : `secure-http` vrai et un dépôt `composer` distant autre que
packagist.org (`fallback::hold_trigger`) ; pour `install`, le même seul
déclencheur (la garde par hôte, retirée à la cinquième revue). Retenue distincte de
`-v` (`io::hold_output`), abandonnée à toute délégation. `update` : de
« Loading… » au pool construit, ses dists jugés (`scan_pool_dists`) ;
`install` : de l'en-tête à la fin des téléchargements. Bancs :
`secure-http.sh` (traces du déclencheur, retenue imposée par
`VIVACITY_HOLD_OUTPUT=force` : natif sous normal, `-q`, `-v`, rendu sans
rien avant la raison, mode `loaded`), et `verbosity.sh`, `steps.sh`,
`flex-update.sh` passés avec la retenue imposée. Mesures dans DECISIONS.

## Cinquième revue (f4de2f8..bd879c9) : plan

- **F1, la retenue commence trop tard.** `update` : retenir avant
  l'événement `COMMAND` (bamarni) et la ligne de l'install sans lock.
  `require`/`remove` : décider le déclencheur avant l'édition et retenir dès
  là ; une délégation au pool n'écrit rien à l'écran, composer.json est
  rétabli à l'octet (le différer jusqu'au pool demanderait une résolution
  sur un manifeste en mémoire de bout en bout : non fait, dit).
- **F2.** Le balayage du pool seulement quand la commande installe
  (`--no-install`, `--dry-run` : non) ; la retenue relâchée au pool quand
  même.
- **F3.** Plus de garde par hôte : `install` retient sous le seul
  déclencheur du mainteneur. Le déclencheur devient un test bon marché sur
  le JSON déjà en mémoire (composer.json, config.json global gardé), sans
  `MergedConfig` ni passage sur le lock ; une seule évaluation par run.
- **F4.** `update` partiel : le lock que la session a lu sert à juger les
  paquets figés non installés, avant « Loading… » ; un test de chaîne par
  paquet verrouillé, rien de plus sans URL non sûre.
- **F5.** Une panique pendant la retenue : les lignes retenues écrites
  avant le message (ordre chronologique), sans attendre de verrou.
- **F6.** Cas : `require`/`remove`, install sans lock, install après
  update, Flex, bamarni, `-vv`/`-vvv`, `--no-install`/`--dry-run` (natifs),
  `install` Packagist seul à dist hors GitHub (pas de retenue).
- **F7.** Retirer du dépôt tout chiffre de banc ou de délai non approuvé ;
  les docs disent ce que fait le code.
- **Dépôts `package` et autres « not supported … yet »** : rendus avant
  toute sortie (types de dépôt autres que `composer`/`path`, filtres
  `only`/`exclude`/`canonical`, listes de politique personnalisées,
  `--minimal-changes`, `COMPOSER=` autre, `update lock|nothing|mirrors`) ;
  ce qui ne se voit qu'au chargement (protocole v1, `api-url` de liste) :
  délégation (tardive, retenue sous le déclencheur) au lieu de sortir en 1.

### Méta et revue indépendante de la cinquième revue

Méta (dix points) : retenir dès `resolution_fallback` passé pour
`require`/`remove` (pas seulement à l'édition : découverte de version,
lignes sur les paquets) ; fixation de `require` sur le lock lu avant
l'édition ; panique limitée au fil qui retient, ordre de `release_output`,
crochet chaîné ; `update --minimal-changes` et la clé fusionnée ; miroirs
de dist du dépôt ; les lignes de `Factory` gardées hors de la retenue
(sinon perdues sous `--no-fallback`) ; schéma d'URL et dépôt sans URL.
Laissés : le refus du second chargement de `remove` (18g), la fixation
après Flex (18h). Revue (six points) : vérité PHP de la clé
minimal-changes, `update -m`, `remove --no-update` sans contrôle de
dépôts, la liste de politique personnalisée laissée tardive (seul un dépôt
qui la déclare la rend fatale), chemin `https` avant `repo_urls`, ordre du
vidage sur panique.

## Sixième revue (bd879c9..02ef0a7) : plan

Portée réduite par le mainteneur à W, H1, H2, H4 et H5 ; H3 et S passent à
l'inventaire (32a, 32b), pour un lot séparé après la fusion.

- **W.** Sous la retenue, `require`/`remove` n'écrivent pas composer.json :
  le texte édité devient une écriture en attente (`io`), que lisent à sa
  place tous les lecteurs entre l'édition et le pool (`prepare_update` de
  la découverte et de la session, `resolve_and_lock`, la seconde lecture
  des plugins de `require`, `updated` de `remove`) ; `remove` édite par un
  `JsonConfigSource` qui passe par la même attente. L'écriture a lieu au
  relâchement de la sortie (pool jugé), ou à toute sortie qui n'est pas
  une délégation (Composer laisse le fichier édité sur ces sorties) ; une
  délégation l'abandonne. Un composer.json créé par `require` aussi. Un
  retour en arrière annule l'attente : il affiche sa ligne, supprime le
  lock d'un fichier créé, et n'écrit l'original que si l'édition a été
  écrite. La découverte rendue d'un fichier créé appelle `undo_edit` (pas
  `revert`, qui supprimait un lock existant et affichait une ligne avant la
  délégation). Un fichier non inscriptible est écrit tout de suite
  (l'erreur d'aujourd'hui, pas le « is not writable » de Composer, inchangé
  par cette revue) ; une écriture différée qui échoue arrête la résolution.
  Sans retenue : écrit où il l'est aujourd'hui. La raison `--no-fallback`
  dit ce qui a été écrit dans le projet.
- **H1.** `update-with-minimal-changes` lu sur les couches de `Factory`
  (config.json et auth.json globaux, `COMPOSER_AUTH`, composer.json,
  auth.json du projet, `COMPOSER_AUTH`) : la dernière couche gagne. Toute
  valeur non booléenne rend (méta et revue : composer.json échoue à la
  validation, les autres couches avertissent). Les fichiers par la copie
  gardée pour le run, que `fetch::Auth::load` relit aussi (pas de lecture
  de plus).
- **H2.** `require --no-update` dont chaque argument porte une contrainte :
  aucun contrôle de dépôts, pas de session de découverte (rien ne résout),
  seulement ce que `requireComposer()` refuse en instanciant les dépôts.
  Minimal-changes ignoré au démarrage pour `require`/`remove --no-update`.
- **H4.** `composer_home()` et le déclencheur gardés pour le run (une
  évaluation par commande) ; pas d'analyse du lock pour `require
  --no-update`.
- **H5.** Formulations : raison `--no-fallback`, « à chaque niveau » réduit
  à `-q`, normal, `-v` (`-vv`/`-vvv` partent à Composer au démarrage).

### Méta et revue indépendante de la sixième revue

La méta (sous-agent frais, sur le code) a corrigé le plan : l'édition en
attente est écrite à toute sortie qui n'est pas une délégation (Composer
laisse le fichier édité ou créé sur `--fixed`, une contrainte invalide, un
refus du second chargement de plugins) ; deux lecteurs du disque oubliés
(second chargement de `require`, manifeste relu de `remove`) ; `revert` doit
toujours supprimer le lock d'un fichier créé ; la délégation de la
découverte d'un fichier créé appelait `revert` (lock supprimé, ligne
écrite) ; `require --no-update` sans contrainte découvre encore une
version (les contrôles de dépôts restent) ; `Auth::load` ne passait pas
par la copie gardée ; une valeur non booléenne dans une couche d'auth est
signalée par Composer. La revue indépendante a trouvé : `undo_edit`
réécrivait composer.json quand rien n'avait été édité (`--dry-run`,
`remove` sans lien nommé) — corrigé par un témoin d'écriture ; la session
de découverte sautée perdait les refus de `requireComposer()` — test en
mémoire ajouté ; les valeurs non booléennes de composer.json et du
config.json global — rendues ; l'erreur d'une écriture différée avalée —
elle arrête la résolution ; l'état de l'édition en attente non remis à
zéro entre deux runs embarqués — remis ; un motif de délégation trop large
dans les cas W — resserré sur la raison du pool.

## Septième revue (02ef0a7..52ff54e) : plan

1. `Cli::minimal_changes` : `--no-update` n'exempte que le mode ; le
   contrôle de type des six couches tourne toujours (composer.json non
   booléen : erreur de Composer, sortie 1 ; config.json global, auth.json :
   avertissement) — rendu.
2. `unregistered_repository_issue` : un dépôt `package` dont `package`
   n'est ni un objet ni une liste d'objets (chacun avec `name` et `version`
   chaînes) — rendu, sans réécrire le schéma.
3. `flush_for_panic` : l'édition écrite compte (`MANIFEST_WRITTEN`).
4. `take_held_write_error` lu dès le retour de `session.update`, avant
   toute autre issue (Composer échoue à l'écriture, avant la résolution).

Cas : 1 et 2 au banc (rouges sur 52ff54e) ; 3 par un test unitaire de
`io` ; 4 non exercé (une écriture qui échoue entre le test d'inscriptibilité
et le pool), relu. Méta : sur ce lot, revue indépendante seulement (sous-
agent frais, sur le code) ; elle a relevé que le test de 3 partageait l'état
de sortie du processus avec l'autre test d'`io` (tests en fils parallèles) :
fondu dans ce test unique.

## Inventaire fermé avant 0.26 : plan, puis ce qui a été fait

- **32c/32d.** Au démarrage (`install`, `update`, `require`, `remove`,
  `config` ; `dump-autoload` une fois composer.json lu), sur le JSON brut en
  mémoire (`repositories` de composer.json et du config.json global gardé
  pour le run ; rien à faire sans cette section) : une entrée typée à
  `"packagist": false` (l'avertissement de `createRepository`, écrit à
  chaque création du `Composer` : rendre est sûr à l'octet) ; une définition
  que `createRepos` ou les constructeurs refusent — pas un objet, sans type,
  `only`/`exclude`/`canonical` mal formés, URL `composer` que `parse_url`
  refuse (pas d'hôte hors `file`, port qui n'est pas 1 à 5 chiffres jusqu'à
  65535 ; URL absente ou non chaîne), `filter` utilisateur qui n'est pas des
  booléens par nom de liste (une clé numérique est un entier en PHP). Un
  doute rend (un port `8a`, que PHP lit 8, est rendu). Plus large que la
  liste fusionnée (une définition désactivée par nom compte encore).
- **32a.** Dépôts locaux seulement (rien sur un chemin Packagist seul : ni
  fusion ni lecture) : leur packages.json, lu une fois et gardé pour la
  session (`read_local`, que le contrôle `secure-http` partage), juge avant
  tout — à la résolution (`insecure_repository_issue`) le protocole v1 et une
  liste personnalisée de la configuration que le dépôt annonce (après son
  `filter` utilisateur) ; pour `install` (`local_install_issue`, quand
  `malware` bloque `install`) la même liste, et une `api-url` de filtre
  derrière une liste `malware` annoncée (la portée `install` ne lit jamais
  les métadonnées : l'erreur est certaine). Reste tardif : l'`api-url` d'un
  dépôt local à la résolution (elle dépend de ce que le pool a lu dans ce
  dépôt, connu au chargement), et tout dépôt distant (son packages.json
  demande une requête).
- **32b.** `require` pose, après la lecture des sauvegardes (là où Composer
  crée son `SignalHandler`), des gestionnaires SIGINT/SIGTERM/SIGHUP
  (`sigaction` de libc, déjà une dépendance de vivacity-core sur macOS et
  Linux : aucune dépendance ajoutée ; signal-hook-registry, déjà dans
  l'arbre par tokio, laisse le signal ignoré au retrait). Le gestionnaire
  marque le run et écrit le numéro dans un tube ; un fil fait ce que fait
  `revertComposerFile` après ce que le run retenait (ligne, fichiers ; le
  lock relu au signal sous `--no-update`), puis mort par le signal. Les
  autres fils s'arrêtent à leur prochaine ligne, et chaque écriture de
  composer.json ou du lock passe une porte que le fil du signal prend pour
  de bon (une écriture en cours finit, aucune ne suit). Pendant les
  opérations sur les paquets, pas de retour en arrière (le gestionnaire
  d'`InstallationManager::execute` passe devant chez Composer) : mort par
  le signal. Retiré à la fin de `require` (dispositions précédentes
  rétablies) et avant qu'un Composer enfant prenne la main. Posé par le
  seul binaire : la bibliothèque embarquée ne touche pas aux signaux.
- **18g.** `remove` juge le second chargement des plugins sur le manifeste
  édité en mémoire (mêmes recherches que la boucle, section vidée retirée ;
  inchangé sous `--dry-run`) avant sa première ligne et l'édition, et
  réutilise ce chargement.
- **18h.** Rien à déplacer : `flex::activation_lines` ne fait aucune
  requête (avertissements ou délégation), l'index est lu par
  `flex::versions`, après le contrôle des paquets figés.

### Méta et revue indépendante de ce lot

Méta (sous-agent frais, sur le code) : pas de retour en arrière pendant les
opérations sur les paquets (`InstallationManager` passe devant) ; une
course avec le fil principal (porte sur les écritures) ; le lock de
`--no-update` relu au signal ; gestionnaires réservés au binaire et retirés
avant un Composer enfant ; `install` couvert pour les listes ; l'`api-url`
certaine en portée `install` ; `config` et `dump-autoload` couverts ;
règles de `parse_url` (clés numériques, `[]` accepté). Revue indépendante :
course vérification-puis-écriture (la porte tenue pendant l'écriture,
jamais pendant une ligne) ; marque posée dans le gestionnaire même ; une
panique du retour en arrière n'empêche pas la mort par le signal ; un
enfant entre `fork` et `exec` ne déclenche rien (pid vérifié) ; seul le
garde qui a posé les gestionnaires les retire.

Restent (inventaire) : Windows (Composer rétablit sur Ctrl-C par
`sapi_windows_set_ctrl_handler`, vivacity non) ; le nettoyage de
`runCleanup` pendant les opérations ; un Composer enfant attendu sans
relais des signaux (antérieur) ; l'avertissement « packagist key » des
dépôts de COMPOSER_HOME/composer.json (le `Composer` global) — lire ce
fichier au démarrage coûterait une lecture à chaque commande.

## Huitième revue (9f1117a..23ff273) : plan et fait

- **D.** DECISIONS justifie le `unsafe` de `signals.rs` (invariants, ce que
  signal-hook ferait ou non, preuve par `harness/require-signals.sh`).
- **1.** Délégation : `signals::spawn_forwarding` relaie SIGINT/SIGTERM/SIGHUP
  au Composer enfant dès avant son lancement (un signal venu pendant est
  relayé une fois le pid connu ; enfant attendu sans être récolté tant que
  le relais est posé), puis `exit_code_of` fait mourir vivacity du même
  signal (`128 + n` pour la bibliothèque et les signaux à core). Relais
  toujours (un signal de groupe arrive deux fois à l'enfant, sans effet
  observable).
- Revue indépendante de ce lot : signal pris juste avant le retrait (traité
  quand même), trou entre lancement et relais (fermé), `raise(3)` au lieu
  de `kill(getpid())`, `posix` aussi pendant les opérations, pid réutilisé
  (`waitid(WNOWAIT)`), crochets de `parse_url` (hôte tout entier entre
  crochets seulement), affirmations de DECISIONS corrigées ; inventoriés :
  `nohup` sans `pcntl`, `function_exists` contre extension chargée.
- **2.** Recettes de Flex : composer.json et le lock derrière `write_gate` ;
  doc du module exacte ; `runCleanup` inventorié (32b).
- **3, 4.** Un tube et un fil pour le processus, jamais fermés ; écriture
  non bloquante, `pipe2(O_CLOEXEC)` sous Linux, `FD_CLOEXEC` sous macOS ;
  `errno` sauvé ; porte attendue cinq secondes au plus ; mort par le signal
  si le tube ne se lit plus pendant un retour en arrière ; plus de
  `unsafe impl Send`.
- **5.** Ports lus comme `strtol`, crochets comme PHP (test unitaire contre
  les sorties de `parse_url`).
- **6.** 18g : clés exactes. `pcntl`/`posix` lus dans la sonde de plateforme
  du run quand elle a eu lieu, sinon supposés (inventaire 32b).
- Bancs : `COMPOSER_TEST_SUITE` dans secure-http.sh (`hold-vv` instable :
  ramasse-miettes tiré au sort à `-vv`).

## Résultats

Les trois points sont fermés. Bancs verts sur le binaire final :
`secure-http.sh` (32, puis 53 après la revue, 57 après le second passage, 61 après le troisième, 67 après le quatrième, 79 avec la sortie retenue, 103 après la cinquième revue, 116 après la sixième, 122 après la septième, 134 avec l'inventaire fermé), `auth-hosts.sh`,
`transport-exit.sh`, `flex-update.sh` (30, puis 34), `flex-recipes.sh`, `flex-install.sh`,
`download-lines.sh`, `fallback-args.sh`, `steps.sh` (279), `update.sh`,
`verbosity.sh` (frais et en ligne), `allow-plugins.sh`, `plugin-lines.sh`
(118) ; `cargo test`, clippy, fmt. Les durées (hyperfine avant/après) et le
corpus vont au rapport, pas ici.

## Inventaire des écarts connus restants

Écarts de sortie ou de comportement connus, ni corrigés ni rendus à
Composer, relevés dans HANDOVER, DECISIONS, README, CHANGELOG et les plans
(puis contrôlés dans le code là où c'était bon marché). Pour chacun : le
déclencheur et ce qui diffère ; **avant écriture** — se détecte-t-il sur ce
qui est déjà en mémoire avant toute écriture (une délégation bon marché) ;
**coût** (S/M/L) ; **corpus** — une délégation toucherait-elle un projet du
corpus (`harness/corpus.sh` ne joue que `install`) ; **sans coût** — un
correctif possible sans coût d'exécution sur le chemin commun. Non corrigés :
la décision revient au mainteneur.

### Réseau et authentification

1. **GitLab, Bitbucket, `custom-headers`, `client-certificate`, `forgejo-token`** :
   ces clés d'auth ne sont pas lues (requête sans identifiants) ; un
   `http-basic` que Composer traite à part (`oauth2`, `private-token`,
   `gitlab-ci-token`, `x-token-auth`) part en Basic là où Composer envoie
   Bearer ou `PRIVATE-TOKEN` ; `github-domains`/`gitlab-domains` ignorés.
   HANDOVER disait « rendus en Basic » : le code ne lit pas ces clés.
   Avant écriture : oui (auth.json, `COMPOSER_AUTH`). Coût : S pour rendre,
   M pour porter. Corpus : non. Sans coût : oui (lecture déjà faite).
2. **Clés d'auth dans `config`** (composer.json ou config.json global) :
   ignorées. Avant écriture : oui. S. Corpus : non. Sans coût : oui.
3. **« avoid overwriting already defined auth settings »** absent (deux
   types d'auth sur une origine ; même gagnant). Avant écriture : oui. S.
   Corpus : non. Sans coût : oui.
4. **Casse des hôtes et des clés** : vivacity met en minuscules, Composer
   compare tels qu'écrits. Avant écriture : oui. S. Corpus : non. Sans
   coût : oui.
5. **Redirection vers une origine qui a ses propres identifiants** :
   Composer les ajoute, reqwest retire l'en-tête. Avant écriture : non
   (réponse du serveur). M. Corpus : non. Sans coût : oui (politique de
   redirection).
6. **Magasin de CA** : webpki embarqué ; `SSL_CERT_FILE`, `cafile`,
   `capath`, `COMPOSER_CAFILE` ignorés. Avant écriture : oui (env, config).
   S pour rendre, M pour porter. Corpus : non. Sans coût : oui pour la
   délégation (variables d'environnement), la config demande la lecture
   déjà faite du config.json global.
7. **`disable-tls` vrai posé dans le seul auth.json du projet** (les autres
   couches sont rendues au démarrage depuis la revue du 2026-10-30) :
   Composer écrit « You are running Composer with SSL/TLS protection
   disabled. » et coupe TLS ; vivacity ne lit ce fichier qu'en présence
   d'une URL non sûre (rendu alors). Avant écriture : oui. S. Corpus : non.
   Sans coût : non — un fichier de plus ouvert à chaque run (l'auth.json du
   projet n'est lu qu'avec le transport, et pas du tout par un `install`
   sans opération ni par `dump-autoload`).
8. **`options.ssl.verify_peer` / `verify_peer_name` à faux** sur un dépôt :
   « Warning: Accessing <hôte> with verify_peer disabled. »
   (`Config.php:723-737`), même en https ; ni écrit ni rendu. Avant
   écriture : oui (manifeste). S. Corpus : non. Sans coût : oui.
9. **auth.json de projet invalide** : ignoré (Composer s'arrête), sauf
   quand une URL non sûre fait lire le réglage (rendu alors). Un
   `COMPOSER_AUTH` qui n'est pas un objet JSON est rendu au démarrage
   depuis le 2026-10-30. Avant écriture : oui. S. Corpus : non. Sans coût :
   non (même raison que le point 7). **Schéma** : une valeur de mauvais
   type dans le config.json global ou un auth.json (Composer l'écrit :
   « does not match the expected JSON schema »), ou dans composer.json
   (Composer s'arrête), n'est vue que pour `secure-http`/`disable-tls`
   (rendu) ; les autres clés, non.
10. **`COMPOSER_DISABLE_NETWORK` posé par l'utilisateur** : seul `--offline`
    coupe le réseau de vivacity (les sources git exceptées). Avant
    écriture : oui. S (rendre, ou le lire comme `--offline`). Corpus : non.
    Sans coût : oui.
11. **Hors ligne, dist absent du cache** : « missing cache for … in offline
    mode » au lieu de « Network disabled, request canceled: <url> ».
    Avant écriture : en partie. S. Corpus : non. Sans coût : oui.
12. **Texte des erreurs réseau** (curl contre reqwest, musl compris) et
    **encadré** des exceptions (`Error: …` contre la boîte de Symfony
    Console, normalisé par `transport-exit.sh` et d'autres) ; une
    installation qui échoue au téléchargement n'imprime pas les lignes
    « Downloading » (imprimées après la transaction). Avant écriture :
    non. M. Corpus : non. Sans coût : en partie.
13. **Dépôt injoignable sous `ignore-unreachable: false`** : sortie 1 notée
    contre 100 (HANDOVER:208) — peut-être périmé, non revérifié ici.
14. **Lignes « Downloading »** : archive en cache au sha1 périmé (Composer
    l'annonce), `cache-files-ttl: 0` et `cache-read-only` inconnus, cache
    non inscriptible (« Cannot create cache directory »), clé de cache sur
    l'URL brute. Avant écriture : la config oui, le sha1 non. S. Corpus :
    non (aucun ne pose ces clés). Sans coût : oui pour la config.
15. **Dist local lu une fois** (Composer : trois tentatives) et message
    d'un dist local absent. Avant écriture : non. S. Corpus : non.
16. **Miroirs de dist** (`dist.mirrors` du lock) : vivacity télécharge
    l'URL principale, Composer celle que `getUrls` met en tête (le dernier
    miroir préféré) — l'avertissement et la délégation de `secure-http`
    suivent l'URL de Composer depuis le 2026-10-30, pas le téléchargement ;
    les miroirs comparés tels qu'écrits (sans `ComposerMirror::processUrl`).
    Avant écriture : oui (lock). S pour rendre. Corpus : non (aucun lock
    n'en a). Sans coût : oui.
17. **Notifications d'installation** : Composer poste les paquets installés
    à leur `notification-url` (`InstallationManager::notifyInstalls`,
    erreurs avalées ; `config.notify-on-install`, vrai par défaut,
    Installer.php:333-341) ; vivacity n'envoie rien. Comportement réseau, pas de
    sortie (sauf l'avertissement `secure-http` d'une URL http, jamais vu).
    Avant écriture : oui, mais tous les locks du corpus en ont : une
    délégation les rendrait tous. Coût : M (une requête après
    l'installation). Sans coût : non (une requête de plus).
18. **Une redirection d'`https` vers une URL non sûre refusée** (rendue,
    tard), **la limite de 20 redirections** : portées mais non vérifiées
    (pas de serveur `https` local auquel vivacity fasse confiance). Avant
    écriture : non. **L'hôte averti d'une redirection** (`secure-http`
    faux) est celui de reqwest, en minuscules, pas le `Location` tel
    qu'écrit (CurlDownloader.php:553-555). Avant écriture : non. M (suivre
    les redirections à la main). Corpus : non.
18b. **auth.json du projet cherché à côté de composer.json, pas de son
    `realpath`** (Factory.php:335) : un composer.json lien symbolique fait
    lire un autre auth.json (identifiants et réglage). Avant écriture :
    oui. S. Corpus : non. Sans coût : non (un `realpath` par run).
18d. **Retenue de la sortie avant une délégation tardive** : décidée
    (2026-10-30, déclencheur étroit : `secure-http` vrai et un dépôt
    distant autre que packagist.org) et faite — retenue jusqu'au pool
    construit (pas la résolution), et pour `install` jusqu'à la fin des
    téléchargements (aussi quand un dist du lock est hors des hôtes sûrs).
    Restent tardifs : `require`/`remove` (avant la retenue), l'installation
    qui suit un `update` (lock écrit), un dist `http://` d'un projet
    Packagist seul, le miroir préféré d'un paquet du pool.
18e. **Faits établis par le relecteur pour la retenue** (troisième
    passage) : Composer 2.10.3 ne réécrit ni ne relève un dist `http://` ;
    les pilotes GitHub et Bitbucket écrivent `https` en dur, GitLab garde le
    schéma enregistré ; sur un échantillon local de 209 685 entrées de
    dists de Packagist, toutes en `https`.
18f. **Contrôle local : plus large que le téléchargement** par les versions
    que le pool charge et que le solveur ne prend pas, et par le mode
    transitif (`-w`/`-W`) élargi d'avance par les liens du lock en plus du
    déverrouillage que Composer fait au chargement ; une dépendance qu'un
    paquet d'un autre type de dépôt (`path`) amène n'est jugée qu'au lock
    résolu (tardif, cas `late-path-dependency`) ; un paquet figé non
    installé à dist `http://` hors dépôt local et hors `remove` aussi.
18g. **`remove` : refus du second chargement des plugins** — fermé le
    2026-10-30 : jugé avant toute ligne, sur le manifeste édité en mémoire
    (`harness/remove-plugin-load.sh`).
18h. **Fixation par `update` après une requête de Flex** — sans objet
    (vérifié le 2026-10-30) : `flex::activation_lines` ne fait pas de
    requête ; l'index est lu par `flex::versions`, après le contrôle.
18c. **Dépôt local (`file://`) sous Windows** : le chemin `\\?\` de
    `canonicalize` suivi de `/packages.json` (écart antérieur du
    transport fichier) ; le contrôle `secure-http` de ses métadonnées n'y
    lit rien. Avant écriture : oui. S. Corpus : non.
19. **`require` sans contrainte** : Composer relit `p2/<nom>.json` et
    `~dev.json` à la découverte de version, vivacity non (requêtes seulement,
    sortie identique, `secure-http.sh` `repo-require`).

### Résolution, `update`/`require`/`remove`

20. **Dépôts `vcs`, `git`, `github`, `package`, `artifact` sur une
    résolution** : « repository type … is not supported by vivacity update
    yet », sortie 1, au lieu d'être rendus (`session.rs`
    `open_repository_with`, `SessionError::new`). Avant écriture : oui
    (`repositories[].type`). S (renvoyer `Unsupported`). Corpus : non
    (`install` seulement ; 48 projets du corpus ont un dépôt `vcs`). Sans
    coût : oui.
21. **Changements minimaux** : `--minimal-changes` refusé ; `update` ignore
    en silence `COMPOSER_MINIMAL_CHANGES` et `config.update-with-minimal-changes`
    (`require`/`remove` les refusent) — le lock peut différer. Avant
    écriture : oui. S pour rendre. Corpus : non. Sans coût : oui.
22. **`bump-after-update`** (option ou config) jamais joué. Avant
    écriture : oui. S pour rendre, M pour porter. Corpus : non (`install`
    seulement ; symfony-skeleton pose la clé). Sans coût : oui.
23. **`update lock|nothing|mirrors`** refusé (sortie 1). Avant écriture :
    oui. S. Corpus : non. Sans coût : oui.
24. **Options de Composer refusées en erreur d'usage (code 2)** au lieu
    d'être rendues : globales `-d`, `--ansi`, `--no-cache`, `--profile` ;
    `install --audit`, `--audit-format`, `--download-only`, `--dev`,
    `--strict-psr-autoloader`, `--no-suggest` ; `update --lock`,
    `--patch-only`, `--root-reqs`, `--interactive`, `--bump-after-update`,
    `--minimal-changes`, `--prefer-*`, `--dev` ; `dump-autoload --dry-run`,
    `--dev`, `--strict-psr`, `--strict-ambiguous` ; `config` hors
    `--global cache-dir`. Avant écriture : oui (argv). S. Corpus : non.
    Sans coût : oui.
25. **Listes de politique à sources et `filter.api-url`** : refusées en
    erreur, pas rendues. Avant écriture : en partie (après packages.json).
    S. Corpus : non. Sans coût : oui.
26. **L'audit** ne tourne jamais (`update`/`require`/`remove` sans sortie
    d'audit). Avant écriture : oui. M. Corpus : non. Sans coût : non (les
    avis sont déjà lus pour les politiques, le rendu est du travail en plus).
27. **« Did you mean » / `getProviders`** absents de `require` d'un nom
    inconnu. Avant écriture : en partie. M. Corpus : non.
28. **Textes d'explication acceptés** : `Link::getPrettyString` d'un lien
    `self.version` d'alias, description coupée au milieu d'un caractère,
    `Url::sanitize` absent de `getRepoName`, échec de l'extraction non-dev
    formaté avec le jeu de dépôts de l'update. Avant écriture : non. S
    chacun. Corpus : non.
29. **`-v` « Dependency resolution completed »** d'`update` chronomètre
    tout le solve, règles comprises (valeur seulement). S. Sans coût : oui.
30. **Délégations tardives** : en-tête, « Verifying… », « Package
    operations », « Loading composer repositories » (et, depuis ce
    chantier, les avertissements du constructeur de pool) réimprimés par
    Composer quand la délégation suit des lignes déjà écrites (portée
    d'`install`, Flex et plugins décidés sur le lock résolu, réglage
    `secure-http` illisible trouvé au chargement) ; « Verifying… » de trop
    et suggestions manquantes pour une install rendue après un update.
    Avant écriture : oui, mais après des lignes. M (différer la sortie ou
    décider plus tôt). Corpus : oui pour la portée d'`install` (les 11
    projets rendus du corpus). Sans coût : non si l'on diffère la sortie
    (latence d'affichage).
31. **Mode interactif** jamais porté (questions de `require`/`remove`,
    `allow-plugins`, Docker de Flex, c3) : vivacity se comporte comme `-n`.
    Avant écriture : oui (TTY sans `-n`). S pour rendre. Corpus : non.
    Sans coût : oui.
32. **`remove --unused`** liste chaque paquet une fois (Composer répète les
    alias) ; **`require`** ne rétablit pas le mtime du lock après
    `updateHash` ; **`JsonManipulator`** : limite de retour arrière de PCRE,
    `1e999`, demi-substituts UTF-16, imbrication > 512. S chacun.
32a. **Données en mémoire rendues tard** — fermé le 2026-10-30 pour les
    dépôts locaux : protocole v1 et liste personnalisée rendus avant tout,
    `api-url` de filtre en portée `install`. Reste tardif : l'`api-url` d'un
    dépôt local à la résolution (dépend de ce que le pool y a lu) et les
    dépôts distants (packages.json par requête).
32b. **Signal pendant `require`** — fermé le 2026-10-30 (macOS, Linux ;
    binaire seulement, `harness/require-signals.sh`) ; le Composer enfant
    d'une délégation reçoit les signaux envoyés à vivacity, qui meurt du
    même signal (huitième revue). Restent : Windows (Ctrl-C) ; le
    `runCleanup` d'`InstallationManager` (téléchargements et extractions
    partiels retirés quand un signal arrive pendant les opérations) — non
    reproduit, ce que vivacity extrayait reste en place ; avant la sonde de
    plateforme du run, `pcntl` est supposé chargé (Composer sans `pcntl`
    n'a pas de gestionnaire : vivacity n'en tient compte qu'une fois la
    sonde faite, sans en lancer une pour cela) ; la sonde dit « extension
    chargée », seld teste `function_exists` (`disable_functions`) ; sans
    `pcntl` sous `nohup`, Composer survit à SIGHUP, vivacity non.
32c. **URL de dépôt `composer` que le constructeur refuse** — fermé le
    2026-10-30 (rendu au démarrage, avec les `filter`, `only`/`exclude`/
    `canonical` et définitions mal formés). Ports et hôtes entre crochets lus
    comme `parse_url` (huitième revue : `80a`, `+80`, ` 80`, `[::1`,
    `[::1]x` acceptés comme PHP les accepte).
32d. **Avertissement « Repository … has a packagist key »** — fermé le
    2026-10-30 pour composer.json et le config.json global (rendu au
    démarrage). Reste : les dépôts de COMPOSER_HOME/composer.json (le
    `Composer` global les crée aussi), non lus au démarrage.

### Installation, plugins, système de fichiers

33. **Ligne de résumé `vivacity: N installed …`** et **`vivacity:
    autoloader generated …`**, **`Note: plugin … installed as a plain
    library`**, **`Note: composer/installers … emulated natively`** sur
    stderr au niveau normal, filtrées par presque tous les harnais. Toute
    exécution native. S (les retirer ou les passer sous `-v`). Corpus : —
    (pas de délégation possible). Sans coût : oui.
34. **Préambule d'une délégation** (raisons, « delegating to … ») avant la
    sortie de Composer. Toute délégation. S. Sans coût : oui.
35. **« Skipped installation of bin … file not found in package »** jamais
    écrit (et `vendor/bin` créé) ; la variante « name conflicts » écrite
    après toutes les opérations. Avant écriture : oui. S–M. Corpus : à
    mesurer (dépend des paquets). Sans coût : oui.
36. **Lancé en root sans `COMPOSER_ALLOW_SUPERUSER`** (hors Docker) :
    Composer avertit et coupe les plugins, vivacity les émule. Avant
    écriture : oui (uid, env). S. Corpus : non (le banc ne tourne pas en
    root). Sans coût : oui.
37. **Plugin autorisé à l'API de plugin incompatible** : Composer le saute
    avec un avertissement. Avant écriture : oui. S. Sans coût : oui.
38. **Plugins émulés installés globalement** : ordre d'activation calculé,
    émulation non jouée ; **bamarni `forward-command: true` sur `update`**
    non transmis ; **dealerdirect quand phpcs ne tourne pas** (Composer
    écrit « Failed to set … », vivacity la configuration — accepté).
    Avant écriture : oui sauf dealerdirect. S (L pour dealerdirect).
39. **Restes de plugins** : dealerdirect 1.1.x sans install lit un
    installed.php non purgé ; la purge suit l'`install-path` enregistré ;
    la ligne « not writable » de package-versions ; `COMPOSER_DEBUG_EVENTS`
    et l'auto-install de php-http/discovery non portés ; lignes de
    dg-cleaner dans l'ordre de `readdir` ; un `vendor/bin` vide retiré ;
    drupol à contrainte `packages-dev` invalide sous `--no-dev` ; `PATH` du
    patcher. S chacun, avant écriture en partie.
40. **Archives hostiles, mtimes des zips, repli `unzip` en deux temps**
    (cinq lignes de Composer non imprimées) : acceptés et écrits (README,
    DECISIONS). Avant écriture : non. L/M/S.
41. **Store et identité** : un zip `artifact` remplacé à même nom et
    version encore servi par le store ; `target_dir` avec `..` ;
    `target-dir: "0"` ; `vendor-dir` par un répertoire absent. S chacun.
42. **Dépôts `path`** : pas de « Symlink failed, fallback to mirroring » ;
    accolades sous musl ; ordre des accolades ; `Url::sanitize` du nom ;
    fichier simple à la place d'un répertoire ; `is_readable` ; texte
    d'erreur JSON de serde ; FIFO ; entrées non UTF-8. S chacun, avant
    écriture pour la plupart. Corpus : non mesuré.
43. **`-v`, `--run-scripts`** : ordre des lignes du patcher et d'infection
    après une installation ; `composer run-script` rejoue des écouteurs de
    plugins, `pre-install-cmd` avant la décision de portée (deux fois si
    rendu ensuite) ; `-0` gardé en `-0` dans installed.json. S–M.
44. **Scripts** jamais joués par défaut (`install` = `--no-scripts`) :
    choix de contrat, écrit.
45. **Barre de progression** : aucune dans un terminal ; tranché comme
    irreproductible.

### Version racine et manifeste

46. **Avertissement de version racine** : doublon sur une délégation
    tardive, une ligne de plus par `composer run-script`, trois lignes chez
    Composer contre une pour un manifeste refusé après lui, composer.json
    global non lu, ce que Composer écrit avant (root/sudo, Xdebug,
    `disable-tls`, « dubious ownership »). S–M.
47. **Refus de manifeste formulés autrement** (schéma : nom racine en
    majuscules, contrainte non chaîne ; encadré absent) et **manifestes
    acceptés là où Composer refuse** (`install`/`dump-autoload` : stabilité
    inconnue, version invalide, dépôt sans `type`, alias mal formé,
    contrainte illisible ; le schéma `LAX` avant les éditions ; vu ici :
    `"secure-http": "false"` en chaîne, refusé par le schéma de Composer,
    installé par vivacity). Avant écriture : oui. L (validation de schéma).
    Corpus : non. Sans coût : non (une validation de schéma par exécution).
48. **`require Psr/Log:^3`** (une ligne de plus, note inversée) ;
    `{"require":{"0":"^1"}}` sort en 1 (Composer 255). S.
49. **`COMPOSER=<fichier>`** refusé (sortie 1) au lieu d'être suivi ou
    rendu. Avant écriture : oui. S. Sans coût : oui.

### Autoload

50. **`target-dir` racine avec `psr-0` racine** : la closure
    `targetDirLoader` jamais générée. Avant écriture : oui. S–M. Corpus :
    non (aucun manifeste du corpus n'a `target-dir`). Sans coût : oui.
51. **Le cache de classmap suppose vendor/ immuable** : contrat assumé.
52. **Chemin de projet avec `'`** (chemins absolus dans
    autoload_static.php), nom de paquet en majuscules comme clé
    d'installed.php, paquet aliasé du dépôt local qui remplace en
    `self.version`. S chacun.

### Windows

53. **Fins de ligne** `\n` au lieu de `\r\n` sur stdout ; un argument que
    Rust ne sait pas échapper pour `composer.bat` (sortie 1, rien lancé).
    S.

### Docs peut-être périmées (à revérifier, pas des écarts établis)

HANDOVER:213 (`acme/NOPE` 1 contre 2 : la validation des noms de liens est
portée), HANDOVER:208 (`--with` non porté : livré en 0.18 ; « le message
d'install d'un lock bloqué ne liste que les versions retirées »), HANDOVER:215
(`remove --dry-run` refusé, `vendor-dir`/`bin-dir` non lus, transport de
`remove` en 1), un `vendor-dir` posé seulement dans le config.json global.
