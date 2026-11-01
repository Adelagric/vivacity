# Délais réseau et erreurs masquées pendant la vérification du lock — plan

> Niveau 2 : contrat de sortie (stdout, stderr, code, projet identiques à
> Composer 2.10.3, ou passation avant toute sortie) et vérification de
> sécurité (liste de blocage). Branche `net-timeouts`, tirée de
> `vendor-isolation`, rebasée sur main. Sources de Composer : le phar
> `/opt/homebrew/Cellar/composer/2.10.3/bin/composer`, extrait ; les numéros
> de ligne sont ceux des fichiers du phar (commentaires retirés par la
> compilation du phar, donc différents de ceux du dépôt git).

## Le problème (observé par le mainteneur le 2026-10-31)

`fixtures/corpus/cypht-org-cypht`, `install --no-dev` : le projet déclare
`https://composer.tiki.org` (`only: henrique-borba/php-sieve-manager`).
tiki acceptait TCP mais ne terminait jamais la négociation TLS (une heure
plus tard, TCP lui-même échouait au bout de 75 s : banc inutilisable).

- Composer : « https://composer.tiki.org could not be fully loaded (curl
  error 28 …), package information was loaded from the local cache and may
  be out of date », puis échec sur l'archive de sieve-manager (curl 28,
  « Source fallback is disabled »), sortie non nulle.
- vivacity (0.26.0 comme main) : 424 s muets dans la vérification du
  lock (`install::join_verification` → `session::verify_install` →
  `pool_filters::filter_list_filter` → `ComposerRepository::root_data_max_age`
  → `fetch_cached` → `HttpTransport::fetch`), puis « filter lists
  ["malware"] → 0 entries », installation de sieve-manager depuis le store,
  sortie 0, aucun avertissement.

## Cadrage

**Problème.** Une requête de vivacity n'a aucun délai : une négociation TLS
qui ne se termine pas, ou un serveur qui ne répond plus, bloque la commande
indéfiniment là où Composer abandonne, avertit, et décide (repli sur le
cache, `policy.ignore-unreachable`, échec). Question de fond : une opération
de sécurité accessoire (liste de blocage, avis) peut-elle échouer chez
vivacity sans que rien ne le dise, alors que chez Composer l'échec change
l'issue ?

**Contraintes.** Parité de Composer cas par cas : ni politique plus
permissive, ni plus stricte. Ne rien toucher de P0 (isolation de vendor/,
détachement, rapport) ni du cache de classmap. Pas de réseau externe dans
les cas. Harnais aux délais raisonnables.

**Non-objectifs.** Le texte des messages de transport (curl contre reqwest,
point 54 de `parity-gaps-026.md`) : inventorié, pas réécrit. La pile TLS
(magasin de certificats). La notification d'installation (`notify-batch`).

**Critères de succès observables.**
1. Chaque cas neuf de `harness/transport-exit.sh` rouge sur main, vert
   après le correctif.
2. Pour chaque cas : même code de retour, même sortie hors du message de
   transport, mêmes effets (vendor/, lock, cache des dépôts, cache des
   archives), et une durée du même ordre que celle de Composer.
3. `connect_timeout` de reqwest mesuré sur la négociation TLS, pas supposé.
4. L'audit (tableau plus bas) dit, pour chaque requête et chaque erreur,
   l'échec lui-même et la décision de Composer après l'échec, puis ce que
   fait vivacity ; chaque écart est corrigé, ou listé avec sa reproduction.

## Ce que fait Composer 2.10.3 (lu dans le phar)

**Délais** (`src/Composer/Util/Http/CurlDownloader.php`) :
- l. 221 : `CURLOPT_CONNECTTIMEOUT, 10` — curl y compte la résolution, la
  connexion TCP **et la négociation TLS** (observé : « Connection timed out
  after 10001 milliseconds » sur un serveur qui accepte TCP sans jamais
  répondre au ClientHello) ;
- l. 222 : `CURLOPT_TIMEOUT, max((int) ini_get("default_socket_timeout"), 300)`,
  délai **total** de la tentative (connexion, en-têtes et corps compris) ;
- l. 76 et l. 279-289 : l'option `http.timeout` d'un appelant remplace ce
  total : 10 s pour l'API des avis (`ComposerRepository.php` l. 777) et
  pour l'API des listes (`FilterListApiClient.php` l. 62), 3 s et 6 s pour
  les notifications d'installation (`InstallationManager.php` l. 594, 626).

**Nouvelles tentatives** (`CurlDownloader.php` l. 384-398, 465-478,
660-669) : seulement pour une requête GET ; erreur curl 6, 7, 16, 28, 92,
ou 35/56 avec « Connection reset by peer » ; statut 423, 425, 500, 502,
503, 504, 507, 510 (et 400 de codeload.github.com) ; trois au plus, après
0, 100 puis 500 ms. Un délai dépassé (28) est donc retenté : **4 × 10 s**
pour une négociation TLS bloquée, **4 × 300 s** pour un serveur muet après
connexion. Un POST (avis) n'est jamais retenté : 10 s en tout.

**`default_socket_timeout`** : lu par le php qui exécute Composer. Les
harnais lancent `composer` (le phar, `#!/usr/bin/env php`) sans `-d`, comme
le fallback de vivacity (`fallback.rs`, `which_composer`) : la valeur est
celle du php.ini du `php` du PATH (60 ici), donc 300 s. Une valeur > 300
allonge le total.

**Décisions des appelants** : voir l'audit.

## Ce que fait vivacity sur main (avant cette branche)

- `Fetcher::new` (`fetch.rs` l. 969-990) : ni `connect_timeout` ni
  `timeout` ; seule la requête des avis (`post_form`, l. 1123) a 10 s.
- Règle de nouvelle tentative (`MAX_RETRIES`, `transport_is_retryable`,
  `status_is_retryable`) : celle de `CurlDownloader`, à ceci près que toute
  erreur de connexion est retentée (curl ne retente pas 35 sans
  « Connection reset by peer », ni 60).
- Erreurs : `ComposerRepository::settle` reproduit `fetchFile`,
  `fetchFileIfLastModified` et `asyncFetchFile` (avertissement « could not
  be fully loaded », repli sur le cache) ; `filter_list_filter` reproduit
  `FilterListProviderSet` (`ignore-unreachable`, avertissement « Filter
  list data could not be fetched… »).

## Plan

### Étape 0 — expérience `connect_timeout` (aucun changement de produit)

Test jetable : un écouteur TCP qui accepte et ne répond jamais ; client
reqwest du `Cargo.lock` (0.12.28, hyper-rustls 0.27.9, rustls 0.23.44,
ring, racines webpki-roots — `cargo tree -e features -i rustls`) ; quatre
combinaisons https/http × connect/total. Le résultat devient un test de
non-régression dans `fetch.rs`.

### Étape 1 — cas rouges (`harness/transport-exit.sh`, section 6)

Deux serveurs locaux en php : `G` sert l'instantané `solver-policies` en
HTTP (le routeur de la section 4) ; `H` accepte toute connexion et
n'écrit jamais un octet. Une URL `https://H` bloque la négociation TLS
(cas A, délai de connexion) ; une URL `http://H` bloque après connexion
(cas B, délai total).

Pour chaque cas : Composer et vivacity **en parallèle** (le cas dure
une fois le délai, pas deux), vivacity sous chien de garde (3 × la durée
attendue) ; comparés : code, sortie entière hors des lignes qui portent
le message de transport (convention de la section 5), arborescence et
contenu du cache des dépôts et des archives, vendor/ et lock, durée
(vivacity entre 0,7 × et 1,5 × Composer).

Cas par défaut (CI), tous de la famille A (≈ 41 s) ou à 10 s :
- `tls-pkg-cold` : `packages.json` sur `https://H`, cache froid,
  `ignore-unreachable` par défaut — install `--dry-run` ;
- `tls-pkg-cold-fatal` : idem, `ignore-unreachable` faux ;
- `tls-pkg-warm` : idem, cache vieux de plus de 600 s (le cas cypht :
  « could not be fully loaded », puis le résumé servi par G) ;
- `tls-summary` : `packages.json` servi par G, `summary-url` sur `https://H` ;
- `tls-dist` : install réelle, archive sur `https://H` ;
- `tls-advisory` et `http-advisory` : `update`, `security-advisories.api-url`
  sur `https://H` / `http://H` (10 s, sans nouvelle tentative).

Cas longs (`TRANSPORT_EXIT_SLOW=1`, hors CI : 4 × 300 s par côté) :
`http-pkg-cold`, `http-dist`. Justification : le délai total de Composer
est de 300 s par tentative et ne se réduit pas (`max(…, 300)`) ; une
passation à Composer est impossible ici, la panne ne se voit qu'en
attendant, après la ligne d'en-tête déjà écrite. Le comportement de
vivacity sous délai total est couvert en CI par un test unitaire à délai
court (étape 2).

### Étape 2 — correctif des délais (`crates/vivacity-core/src/fetch.rs`)

- `connect_timeout(10 s)` sur le client (si l'étape 0 confirme qu'il
  couvre TLS ; sinon un délai de négociation explicite) ;
- délai total par tentative, appliqué à toute la tentative (envoi, en-têtes,
  corps) des GET de métadonnées et d'archives : 300 s, puis, s'il est
  atteint, `default_socket_timeout` du php (`VIVACITY_PHP` ou `php`, comme
  la sonde de plateforme) lu à ce moment-là seulement — pas de processus
  de plus sur le chemin chaud — et l'attente prolongée jusqu'à cette
  valeur si elle est plus grande ; la tentative dépassée est retentée
  comme curl 28 ;
- `post_form` garde 10 s au total, une tentative (`http.timeout` de
  Composer remplace le total ; un POST n'est pas retenté).
- Tests : délai de connexion sur TLS bloqué (étape 0), délai total sur un
  corps qui s'arrête, prolongation par `default_socket_timeout` (délai de
  base injecté court).

### Étape 3 — audit, puis corrections des écarts de décision

Audit écrit ci-dessous, une ligne par requête × erreur ; chaque cellule
« vivacity » vérifiée par lecture du code, et par un cas quand l'écart
est corrigé. Écarts de décision trouvés à la lecture (à confirmer par un
cas rouge) :
- **D1** `packages.json` en JSON invalide avec un cache : Composer
  (`fetchFile` l. 1801-1825, `fetchFileIfLastModified` l. 1885-1900)
  avertit et sert le cache ; vivacity échoue (`settle` rend une erreur de
  données avant de regarder le cache) — **plus strict** ;
- **D2** ordre des requêtes et des avertissements entre dépôts :
  `FilterListProviderSet::__construct` (l. 47-58) appelle `hasFilter()`
  sur **tous** les dépôts avant le premier `getFilter()`, et liste les
  dépôts injoignables de cette première passe avant ceux de la seconde
  (l. 150-158) ; vivacity alterne dépôt par dépôt.

### Étape 4 — documentation

HANDOVER (ligne `transport-exit.sh`, inventaire des écarts), DECISIONS,
CHANGELOG, ce plan (exécution).

## Critères de vert

- Section 6 de `transport-exit.sh` : rouge sur main (chaque cas
  nommé), vert après ; sections 1-5 toujours vertes.
- `secure-http.sh`, `auth-hosts.sh`, `net-retry-selftest.sh` verts (les
  harnais réseau directement touchés), `allow-plugins.sh` si les avis le
  sont.
- `cargo test`, `cargo clippy --all-targets -D warnings`, la même chose
  pour `x86_64-pc-windows-gnu`.

## Risques

- Un délai total de 300 s sur une archive très lente mais vivante : c'est
  le comportement de Composer (curl 28 au bout de 300 s), reproduit.
- La lecture paresseuse de `default_socket_timeout` suppose que Composer
  tournerait sous le même php : faux pour un `php -d … composer.phar`
  lancé à la main (que vivacity ne voit pas).

## Méta-analyse (sous-agent frais, plan et code, 2026-10-31)

22 failles (3 bloquantes, 10 majeures, 9 mineures). Vérifiées dans le
code avant d'être retenues ; ce qu'elles changent :

- **M1 (bloquante, retenue en partie après mesure)** — `FileDownloader.php`
  l. 135 (`$retries = 3`) et l. 249-290 rappellent `$download()` après
  500 ms, et chaque appel refait les 4 tentatives de `CurlDownloader`.
  La méta y mettait toute erreur curl (« code 0 ») : **faux**, mesuré — une
  archive bloquée en TLS donne 4 connexions, une ligne « Downloading » et
  40 s, parce qu'une `TransportException` a le code **400** par défaut
  (`TransportException.php`, constructeur), qui met `$retries` à 0. Seuls
  500/502/503/504 (le code est le statut, `failResponse`) font les
  nouveaux tours : 16 requêtes, mesurées (cas `dist-500`). **Porté** pour
  ceux-là dans `dist_bytes`.
- **M2 (bloquante, retenue)** — supprimer les lignes qui portent un message
  de transport (convention de la section 5) cache justement celles qu'un
  cas doit prouver (présence, nombre, ordre des avertissements).
  **Changé** : la section 6 remplace le message à l'intérieur de la ligne
  (`could not be fully loaded (<transport>)`, `  - <transport>`) et compare
  tout le reste, ordre compris.
- **M3 (bloquante, retenue)** — un cache chaud contre `https://H` ne peut pas
  se remplir depuis G (la clé de répertoire contient schéma et port), et
  `cp -R` rajeunit les fichiers (moins de 600 s : aucune requête).
  **Changé** : `packages.json` forgé dans `repo/https---127.0.0.1-<H>/`,
  avec et sans `last-modified` (les deux chemins de Composer), vieilli par
  `touch` après la copie ; chaque cas vérifie que H a reçu des connexions.
- **M4 (retenue, non corrigée)** — après le correctif, cypht diverge
  encore : vivacity installe sieve-manager depuis son store, Composer, au
  cache d'archives vide, le télécharge et échoue. Écart d'issue, pas de
  politique de sécurité ; inventorié, décision au mainteneur.
- **M5 (retenue)** — les cas d'avis : `post_form` a déjà 10 s au total, qui
  couvrent la négociation (témoins verts sur main, pas des cas rouges) ;
  il faut `metadata: false` pour que le POST parte, `--no-audit` des deux
  côtés (Composer audite `update`, vivacity non, point 26).
- **M6 (retenue)** — `transport-exit.sh` tourne sous `net-retry.sh` en CI,
  qui relance sur « curl error 28 » / « error sending request » : la
  section 6 en produit à chaque fois. **Changé** : le harnais, sans réseau,
  sort de `net-retry.sh` (`ci.yml`).
- **M7 (retenue, corrigée)** — `prefetch` s'arrête au premier échec du lot
  et n'écrit pas en cache les réponses réussies suivantes ; Composer
  (`$accept`, `Loop::wait`) les écrit toutes. Tous les résultats sont
  désormais réglés, la première erreur rendue.
- **M8 (retenue, non corrigée)** — une archive en échec interrompt les
  autres téléchargements (`installer.rs`, `JoinSet`) ; Composer les mène
  à terme et les met en cache. Effet sur le cache d'archives ; inventorié.
- **M9 (retenue)** — D2 complété : sous `ignore-unreachable` faux, Composer
  interroge le `packages.json` de **tous** les dépôts avant de lever la
  première exception ; avec une liste personnalisée, vivacity interrogeait
  deux fois un dépôt muet (`get_filter_lists` non mémorisé en erreur).
  Une seule passe `get_filter_lists`, réutilisée.
- **M10 (retenue, non corrigée)** — avant une passation (plugin de fusion),
  `abandon_and_wait` attend la fin de la vérification : jusqu'au délai
  entier, puis Composer refait la même attente. Durée seulement ;
  inventorié.
- **M11 (retenue)** — D1 : le message n'est pas un message de transport,
  c'est `"<url>" does not contain valid JSON` (`Response.php` l. 89-99),
  à reproduire à l'octet ; la forme asynchrone reste fatale (exception
  levée dans `$accept`) ; sans cache, fatal même sous `ignore-unreachable`.
- **M12 (retenue, non corrigée)** — Flex (index, recettes, `DistReader`)
  passe par `Fetcher` : il hérite des délais, mais ses décisions après
  échec ne sont pas auditées ici.
- **M13 (partiellement retenue)** — `default_socket_timeout` : un délai
  reqwest ne se prolonge pas ; douze requêtes l'atteignent ensemble ; le
  php du fallback est celui du shebang de `composer`. **Choix** : pas de
  délai reqwest, un `tokio::time::timeout` sur la tentative épinglée,
  prolongé une fois ; valeur lue une seule fois par processus
  (`OnceLock`), par le php de la sonde (`VIVACITY_PHP`, sinon le `php` du
  PATH, celui que `#!/usr/bin/env php` choisit), 300 s si php ne répond
  pas. Rejeté : l'ajouter à la sonde de plateforme (la sonde tourne après
  le filtre, et lit un autre état) ; le coût n'existe qu'après 300 s
  d'attente. Signature de `Fetcher::new` inchangée.
- **Mineures retenues, inventoriées** : une redirection partage le délai de
  la tentative (curl : un job par saut) ; `CURLOPT_FRESH_CONNECT` à chaque
  nouvelle tentative contre une connexion h2 réutilisable par reqwest (non
  mesuré) ; 8 connexions par hôte chez curl, 12 requêtes de métadonnées
  sans limite par hôte chez vivacity ; Composer sans ext-curl
  (`RemoteFilesystem`) n'a aucun de ces délais ; curl 55 retenté sans
  limite pour toute méthode. **Appliquées aux cas** : un H par côté et par
  cas, journal des connexions ; store neuf par côté ; bornes de durée
  absolues autour du plancher réseau ; chien de garde des deux côtés ;
  `notify-on-install: false` ; test https conditionné à `rustls-tls-ring`.

## Audit : chaque requête d'un `install` depuis un lock (vérification comprise)

Sources : le phar (lignes des fichiers extraits). « L'échec » : ce que
Composer lève ; « la décision » : ce qu'il en fait ensuite. « Texte » :
le message de transport diffère (point 54 de `parity-gaps-026.md`), tout le
reste de la sortie est comparé. Les requêtes d'un `install` depuis un lock,
chez Composer : le filtre des listes (`Installer.php` l. 808 :
`createFilterListPoolFilter(BLOCK_SCOPE_INSTALL)`, pas de filtre d'avis —
`null` en septième argument), les archives, les notifications. Les avis ne
sont demandés qu'à `update` (`SecurityAdvisoryPoolFilter`) ou sous
`install --audit` (rendu à Composer par vivacity) ; l'abandon ne fait
aucune requête (donnée du lock).

| Requête | Erreur | L'échec chez Composer | Décision de Composer après l'échec | vivacity sur main | vivacity après | Cas |
|---|---|---|---|---|---|---|
| `packages.json` (`hasFilter` → `loadRootServerFile(600)` ; aucune requête si le cache a ≤ 600 s) | négociation TLS bloquée | curl 28 après 4 × 10 s (GET retenté) | cache présent : « could not be fully loaded », le cache ; absent : `FilterListProviderSet` garde l'exception → `ignore-unreachable` vrai : « Filter list data could not be fetched… », sortie 0 ; faux : exception, 100, indice `COMPOSER_DISABLE_NETWORK=1` | attend indéfiniment | identique (texte) | `tls-pkg-cold`, `tls-pkg-cold-fatal`, `tls-pkg-warm-lm`, `tls-pkg-warm-nolm` |
| idem | muet après connexion | curl 28 après 4 × 300 s | idem | attend indéfiniment | identique (texte) | `http-pkg-cold` (lent) |
| idem | connexion refusée | curl 7, 4 tentatives | idem (sans indice) | identique (texte) | inchangé | `refused-*` (section 5) |
| idem | 404 | `TransportException` 404, pas retentée | relevée même avec un cache (`fetchFile` l. 1809, `fetchFileIfLastModified` l. 1889) → liste des sources ignorées ou 100 | même décision, texte « <url> not found » | inchangé | lu, non mesuré |
| idem | 500/502/503/504 | retentée 3 fois, puis `TransportException` | comme une erreur de transport | même décision (texte) | inchangé, et l'ordre des dépôts | `order-two-repos` |
| idem | JSON invalide | `ParsingException` « "<url>" does not contain valid JSON » | cache : avertissement et cache, sortie 0 ; sans cache : sortie 1, `ignore-unreachable` n'y peut rien | sortie 1 dans les deux cas, autre message (**plus strict**) | identique à l'octet | `json-pkg-warm-lm`, `json-pkg-warm-nolm`, `json-pkg-cold` |
| includes d'un dépôt sans `metadata-url` | toute erreur | **aucune requête** (`loadRootServerFile` ne lit pas les includes) | — | demandés dans les données racine : avertissement ou échec | non demandés par la vérification | `plain-includes` |
| résumé des listes (`loadFilterSummary`, `asyncFetchFile`, conditionnel si le cache a `last-modified`) | transport, délai, 5xx | `TransportException` | avertissement du dépôt dégradé ; conditionnel : le cache ; sinon relevée → liste ou 100 | identique pour un refus ; attente infinie sur un délai | identique (texte) | `tls-summary` |
| idem | 404 | rejet 404 | « Filter summary URL … returned 404 for … » (`TransportException` 404) → liste ou 100 | même message, même décision | inchangé | lu, non mesuré |
| idem | JSON invalide | exception levée dans `$accept`, que `$reject` ne rattrape pas | sortie 1, sans avertissement, quel que soit `ignore-unreachable` | sortie 1, autre message | identique à l'octet | `json-summary` |
| fichiers p2 des candidats (`startCachedAsyncDownload`, en lot) | transport, 5xx sur l'un | relevée après tout le lot (`Loop::wait`) | chaque réponse réussie écrite en cache, puis comme le résumé | arrêt au premier échec, réussites suivantes non écrites | lot réglé en entier | `p2-batch` |
| idem | 404 | paquet absent | aucune entrée, silence | identique | inchangé | lu |
| idem | JSON invalide | levée dans `$accept` | sortie 1 | autre message | identique à l'octet | test unitaire `a_package_file_that_is_not_json_is_fatal_without_a_warning` |
| `filter.api-url` (POST, 10 s) | — | — | — | rendu à Composer avant tout | inchangé | `secure-http.sh` |
| archive (`FileDownloader`, `CurlDownloader`) | TLS bloquée / muet | curl 28, 4 tentatives (code 400 : pas de nouveau tour) | pas de source : exception, 100, indice `COMPOSER_DISABLE_NETWORK=1` | attend indéfiniment | 100, indice ; lignes « Downloading » absentes (point 12) | `tls-dist`, `http-dist` (lent) |
| idem | 404 | une tentative | 100 | identique | inchangé | `d404` (section 1) |
| idem | 500/502/503/504 | 4 tours de 4 tentatives, 500 ms entre les tours | 100 | 4 tentatives | 16 | `dist-500` |
| idem | archive dans le store de vivacity, absente du cache de Composer | Composer la télécharge | échec du téléchargement → 100 | installée depuis le store, 0 (**plus permissif**) | téléchargée comme chez Composer, 100 (point 55, plus bas) | `tls-dist-store`, `refused-dist-store`, section 8 |
| notifications (`notifyInstalls`, POST 6 s / 3 s) | toute erreur | avalée (`catch` vide) | rien | aucune requête (point 17) | inchangé | — |
| API des avis (`update`, POST, 10 s, une tentative) | TLS bloquée / muet | curl 28 après 10 s | `ignore-unreachable.update` vrai : « Security advisory data could not be fetched… », sinon 100 | identique (10 s depuis longtemps) | inchangé | `https-advisory`, `http-advisory` (témoins) |
| idem | JSON invalide | `ParsingException` | sortie 1 | autre message | identique à l'octet | lu ; message par `invalid_json` |

Écarts ouverts (repros dans `parity-gaps-026.md`, points 54-60) : texte des
messages de transport ; archive servie par le store (cypht, issue plus
permissive — fermé ensuite, option (b) du mainteneur : section « Point 55 »
plus bas) ; autres téléchargements interrompus au
premier échec ; magasin de certificats ; attente avant une passation
tardive ; Flex non audité ; repli sur la source non comparé.

## Exécution (2026-10-31)

**Cas rouges sur main** (binaire de la tête de départ, mêmes cas) :
`tls-pkg-cold`, `tls-pkg-cold-fatal`, `tls-pkg-warm-lm`,
`tls-pkg-warm-nolm`, `tls-summary`, `tls-dist` (vivacity tué après 60 s,
Composer fini en 40 s), `dist-500` (4 tentatives contre 16),
`json-pkg-warm-lm`, `json-pkg-warm-nolm` (1 contre 0), `json-pkg-cold`,
`json-summary` (message), `order-two-repos` (ordre des avertissements),
`p2-batch` (cache), `plain-includes` (avertissement de trop). Témoins verts
sur main : `https-advisory`, `http-advisory`. Après : tous verts, et les
sections 1-5 aussi (28 cas).

**Cas lents** (`TRANSPORT_EXIT_SLOW=1`, une exécution sur ce Mac) :
`http-pkg-cold` et `http-dist`, Composer 1200,8 s, 4 tentatives (l'archive
aussi : curl 28, pas de nouveau tour) ; vivacity après : 1200,7 s,
4 tentatives, sortie, code et effets identiques ; vivacity sur main : une
seule connexion, toujours en attente à 1500 s (tué).

**L'erreur était-elle avalée ?** Non, sur ce chemin. Sur main, une
négociation bloquée qui finit en erreur (serveur qui ferme au bout de 5 s,
cache présent) donne « could not be fully loaded (…) » puis la liste des
sources ignorées : rien n'est perdu. Le silence de cypht après 424 s
signifie que tiki a fini par répondre à vivacity (qui attendait encore),
là où Composer avait abandonné à 40 s et averti. Déduit, pas observé sur
tiki (qui a changé d'état depuis).

**Pourquoi le cas https « TLS terminé puis plus rien » n'existe pas.**
vivacity ne fait confiance qu'aux racines compilées (webpki-roots) et ne lit
ni `cafile` ni `SSL_CERT_FILE` (point 57) : aucun certificat local ne passe
sa négociation. Le cas est exercé en http (`http-pkg-cold`, `http-dist`) :
après la connexion, le chemin est le même (seul le délai total compte, chez
curl comme chez reqwest), et le test unitaire
`an_attempt_is_bounded_by_its_total_time_body_included` couvre un corps qui
s'arrête en route.

**`default_socket_timeout`** : le chemin de prolongation n'est exercé que
par le test unitaire (valeur injectée) ; pas de bout en bout avec un php.ini
à plus de 300 s (il faudrait 4 × 600 s de chaque côté).

## Revue indépendante (sous-agent frais, diff seul) et ce qui a changé

- **Majeur, corrigé** : `effects()` triait les chemins avant de remplacer
  les ports ; deux caches de dépôts nommés d'après des ports aléatoires
  (`order-two-repos`) sortaient dans un ordre ou l'autre — un échec une fois
  sur deux sans écart de comportement. Tri après le remplacement.
- **Corrigé** : la lecture de `default_socket_timeout` attendait php sans
  borne ; un shim qui attend aurait défait le délai qu'on lui demande.
  Bornée à 5 s (le processus est tué, le plancher reste), valeur lue après
  un marqueur (un avertissement de démarrage sur stdout n'est plus pris
  pour elle).
- **Corrigé** : une erreur non transport de `hasFilter` avec une liste
  personnalisée configurée était ignorée quand aucune liste malware n'est
  active, et l'arrêt à cette erreur cachait la liste qu'un dépôt suivant
  annonce. Composer lève dans le constructeur ; la liste des listes
  actives n'est pas connue ici : la commande est rendue à Composer.
- **Corrigé** : deux serveurs pouvaient tirer le même port, ou le port
  fermé de la section 2 (`fresh_port`).
- **Consigné** (point 54) : deux échecs dans un lot, Composer cite le
  premier dans le temps, vivacity le premier du lot.
- Fichiers de documentation non commités au moment de la revue : commités
  ensuite.

## Point 55 — parité stricte des téléchargements (option b du mainteneur)

### Cadrage

**Problème.** Caches désynchronisés : une archive absente du cache de
fichiers de Composer, présente dans le store de vivacity. Les deux
impriment `  - Downloading …` ; Composer télécharge (succès : l'archive dans
son cache ; échec réseau : 100), vivacity pose le paquet depuis son store,
sans requête, et sort en 0 (cypht, `composer.tiki.org` injoignable).

**Décision du mainteneur.** Quand Composer téléchargerait une archive,
vivacity fait la même requête (mêmes tentatives et délais), même si son
store a le paquet, et en tire la même issue : code, sortie, effets sur le
cache de fichiers de Composer. **Restriction** : seulement quand Composer ne
servirait pas l'archive depuis son cache ; le cas normal (cache de Composer
chaud) reste sans requête, le store sert comme aujourd'hui.

**Non-objectifs.** Le cas normal où le cache de Composer et le store
divergent en contenu (pas de hachage de plus sur le chemin chaud). Les
miroirs (point 16), `updateDistReference` (point 14, clé sur l'URL brute),
le repli sur la source (point 60), les autres téléchargements interrompus
au premier échec (point 56), le texte des messages de transport (point 54).

**Critères de succès observables.** Les cas de la section 8 de
`harness/transport-exit.sh` (plus `tls-dist-store` en section 6) rouges sur
la branche avant le point 55, verts après ; le témoin `desync-witness` (deux
caches chauds, aucune requête de part et d'autre) vert avant et après ;
sections 1-7, `download-lines.sh`, `diff-vendor.sh`, `vendor-isolation.sh`
(link, cow), `secure-http.sh` verts.

### Le prédicat de Composer (lu dans le phar)

`FileDownloader::download` (`src/Composer/Downloader/FileDownloader.php`)
l. 176-198 : l'archive est copiée du cache **ssi**
`$this->cache !== null && ($checksum === null || $checksum === '' ||
$checksum === $this->cache->sha1($cacheKey)) && $this->cache->copyTo($cacheKey, $fileName)` ;
sinon la ligne `Downloading` (l. 192-194) et `addCopy` (l. 196). Clé
(l. 129-133, 148) : `<getName()>/sha1(<URL traitée>).<type>`, URL traitée
= `processUrl` (l. 481-492 : `updateDistReference`) de la première URL de
`getDistUrls` (miroirs préférés en tête).

- **Archive absente** : `copyTo` rend faux (`Cache.php` l. 213-233) →
  téléchargement.
- **Cache nul** : `Factory::createDownloadManager` (`Factory.php`
  l. 506-510) ne crée le cache que si `cache-files-ttl > 0`
  (`Config.php` l. 458-463 : à défaut, `cache-ttl`) → téléchargement de
  tout, rien d'écrit.
- **Cache désactivé** : `Cache::isUsable` (`Cache.php` l. 82-85 : un
  segment `$null`, `nul`, `NUL`, `/dev/null`) → désactivé sans message ;
  répertoire non créable ou non inscriptible (l. 90-108) → l'avertissement
  « Cannot create cache directory … Proceeding without cache », désactivé →
  téléchargement de tout.
- **Lecture seule** (`cache-read-only`, `COMPOSER_CACHE_READ_ONLY`,
  `Config.php` l. 400-412 ; `Factory.php` l. 509) : lu comme d'habitude
  (`isEnabled` ne teste pas l'écriture, l. 95-101), jamais écrit
  (`$accept`, l. 239-242 ; `copyFrom`, l. 192).
- **Ramasse-miettes** : au constructeur (`FileDownloader.php` l. 106-109),
  `gcIsNecessary` (`Cache.php` l. 238-254) tire `!random_int(0, 50)` : une
  fois sur 51, `gc` (l. 304-332) efface les archives dont le **mtime** a
  plus de `cache-files-ttl` (défaut `cache-ttl`, 6 mois) — `copyTo` ne
  touche que l'atime (l. 219) — puis les moins récemment lues au-delà de
  `cache-files-maxsize`. Tirage aléatoire : non reproductible.
- **`COMPOSER_DISABLE_NETWORK`** (`HttpDownloader.php` l. 73, 328-338) :
  le prédicat ne change pas ; la ligne `Downloading` s'imprime, puis
  `startJob` rejette la requête (`TransportException` « Network disabled,
  request canceled: <url> », statut 499 → `FileDownloader` l. 272-275 : ni
  nouvelle tentative ni autre URL) → 100. Seuls les jobs curl passent par
  `startJob` : un chemin local ou `file://` (`RemoteFilesystem`, l. 236-264,
  résolu dans le constructeur de la promesse) est copié même hors ligne.
- **Dist `file://` ou chemin local** : même `FileDownloader`, même cache,
  même prédicat ; la copie passe par `RemoteFilesystem` puis `$accept`
  l'écrit dans le cache. **Dist `path`** : `PathDownloader::download`
  (l. 42-…) ne lit ni n'écrit le cache, aucune ligne.
- **Somme de contrôle** : avec un `dist.shasum` non vide, une archive en
  cache dont le sha1 diffère n'est pas servie → téléchargement ; le fichier
  périmé n'est **pas** effacé avant (écrasé par `copyFrom` au succès,
  laissé en place à l'échec : `$reject` ne nettoie que `lastCacheWrites`).
  L'archive téléchargée est écrite en cache **avant** sa vérification
  (l. 239-242 puis 213-215).

### Plan

1. **Cas rouges** (`harness/transport-exit.sh`) : section 8 neuve (rapide)
   et `tls-dist-store` en section 6 (dans le lot parallèle, sans durée de
   plus). Chaque côté son serveur ; journal des requêtes de l'archive
   (`router7.php` : `REQ_LOG`) ; le sha1 de l'URL de chaque côté remplacé
   par `<SHA>` dans les effets (les ports diffèrent), comme les ports. Cas :
   `desync-fetch` (store chaud, cache froid, serveur qui répond : une
   requête de chaque côté, archive en cache, projet identique),
   `desync-other-bytes` (le store vient de l'archive A, le serveur sert B :
   vendor/ de B des deux côtés), `refused-dist-store` (port fermé) et
   `tls-dist-store` (muet en TLS) : 100, même sortie hors texte de
   transport, mêmes effets (vendor/ compris, répertoires vides inclus) ;
   `desync-witness` (deux caches chauds : zéro requête, vert avant et
   après) ; `offline-dist-store` (`--offline` contre
   `COMPOSER_DISABLE_NETWORK=1`, store chaud, cache froid) ;
   `desync-local` et `offline-local` (dist sur un chemin local, cache froid,
   store chaud : l'archive écrite en cache) ; `desync-stale-sha`
   (`dist.shasum` posé, une autre archive en cache à la clé : Composer
   télécharge et l'écrase).
2. **Le prédicat, un seul** (`vivacity-core/src/fetch.rs`) :
   `composer_downloads(cache_root, name, url, type, shasum)` = archive
   absente du cache, ou `shasum` non vide différent du sha1 du fichier.
   `download_lines` (install.rs) l'évalue une fois par paquet de la
   transaction, imprime la ligne et rend l'ensemble des noms ;
   `InstallOptions::downloads` le passe à l'installeur (à défaut,
   l'installeur l'évalue lui-même avec la même fonction) ; le
   téléchargement suit cet ensemble, pas un second test du cache.
3. **Les cas du cache que vivacity ne reproduit pas, rendus avant toute
   sortie** (`cache_dir_hand_over_with`, `project_cache_config`, et
   l'`auth.json` du projet) : `cache-read-only`, `cache-files-ttl`,
   `cache-ttl` posés dans une couche, `COMPOSER_CACHE_READ_ONLY` posé, un
   répertoire `files/` inutilisable (`Cache::isUsable`) ou ni inscriptible
   ni créable. Présence suffit (comme `cache-files-dir` aujourd'hui) :
   prudent, aucune conversion PHP à reproduire.
4. **L'installeur** (`installer.rs`) : un paquet de l'ensemble est
   téléchargé même s'il est dans le store ; hors ligne, `Error::Transport`
   « Network disabled, request canceled: <url assainie> » sauf chemin local
   (lu même hors ligne) ; le fichier périmé n'est plus effacé avant le
   téléchargement. **Sort de l'archive** : l'entrée du store est réutilisée
   seulement si un fichier de provenance (`<store>/.archives/<layout>/…`,
   le sha1 de l'archive dont l'entrée a été extraite, écrit par le seul
   processus qui a renommé l'entrée) égale le sha1 de l'archive
   téléchargée. Sinon : extraction de l'archive téléchargée en zone de
   préparation, vendor/<paquet> posé depuis elle, **l'entrée du store
   inchangée** (une entrée est immuable : le cache de classmap la clé par
   son chemin, d'autres projets en ont des copies), et une marque « copie
   étrangère » (`<store>/.foreign/<layout>/<clé de vendor>/<nom>`) que le
   dump lit (`mutable_copies` : pas d'enregistrement de store pour cette
   copie) et que la prochaine pose depuis le store efface.
5. **Échec du téléchargement** : les lignes `Downloading` (et leurs
   avertissements `secure-http`) imprimées avant l'erreur, comme Composer ;
   un vendor/ créé par le run retiré (Composer : `cleanup`,
   `FileDownloader.php` l. 316-343).
6. Docs : point 55 fermé, point 14 mis à jour (ce qui reste), DECISIONS,
   CHANGELOG, HANDOVER, ce plan.

### Critères de vert

Cas neufs rouges sur la branche avant le point 55 (nommés), verts après ;
`desync-witness` vert des deux côtés ; `transport-exit.sh` (sans les cas
lents), `download-lines.sh`, `diff-vendor.sh`, `vendor-isolation.sh` (link,
cow), `secure-http.sh` verts ; `cargo test` des crates touchées ; clippy `-D
warnings` hôte et `x86_64-pc-windows-gnu`.

### Risques

- Un projet dont le cache de Composer est vidé (CI sans cache Composer mais
  avec le store de vivacity) télécharge de nouveau toutes ses archives :
  c'est le coût voulu, le comportement de Composer.
- Hors ligne, store chaud et cache froid : échec là où vivacity
  réussissait (voulu).
- La marque de copie étrangère est un état de plus ; perdue (store effacé),
  le dump retombe sur l'hypothèse d'aujourd'hui (copie = entrée), P1.

### Méta-analyse (sous-agent frais, plan et code, 2026-10-31)

14 failles (1 bloquante, 7 majeures, 6 mineures), vérifiées dans le code
avant d'être retenues :

- **1 (bloquante, retenue)** — la ligne vient des opérations, le
  téléchargement de `to_install` (filtré par le chemin d'installation) : un
  paquet peut avoir sa ligne sans requête. **Changé** : l'ensemble calculé
  par `download_lines` est la seule vérité ; l'installeur télécharge chacun
  de ses membres, posé ou non (une archive sans chemin d'installation est
  téléchargée et mise en cache, comme chez Composer). Un miroir préféré en
  tête change la clé et l'URL de Composer (point 16) : consigné, pas rendu
  (voir « Exécution »).
- **2 (majeure, retenue en partie)** — le hachage d'une archive en cache
  dont le lock porte un `shasum`, sur une install à vendor/ vide : c'est le
  prédicat de Composer (`hash_file`, l. 180), qui fait ce travail lui
  aussi ; gardé, limité aux paquets de la transaction qui ont un `shasum`
  (aucun sur Packagist), et l'archive vérifiée n'est pas relue ni rehachée
  ensuite.
- **3 (majeure, retenue)** — `COMPOSER_CACHE_DIR=/dev/null` (idiome Docker)
  marche en natif aujourd'hui ; un rendu à la présence le casserait.
  **Changé** : cache nul (`cache-files-ttl`/`cache-ttl` entiers ≤ 0) et
  cache inutilisable (`isUsable`) en natif : tout se télécharge, rien ne
  s'écrit. Rendus : lecture seule effective (sémantique de `Config::get`,
  l'env `!== 'false' && (bool)`), une valeur non entière des durées, une
  clé dans `auth.json`/`COMPOSER_AUTH`, un répertoire `files/` ni
  inscriptible ni créable (`access(2)` sur l'ancêtre existant).
- **4 (majeure, rejetée ici)** — `COMPOSER_DISABLE_NETWORK` posé par
  l'utilisateur sans `--offline` : écart connu et laissé le 2026-10-16
  (DECISIONS), hors de ce point ; consigné de nouveau. `sanitize_url`
  passe dans vivacity-core (le message hors ligne l'assainit).
- **5 (majeure, retenue en partie)** — la marque de copie étrangère se perd
  si vendor/ est déplacé : écrite avant la pose (un arrêt la laisse
  prudente) ; la perte retombe sur l'hypothèse d'aujourd'hui (P1).
- **6 (majeure, retenue)** — le fichier de provenance ne sert à rien pour
  les entrées d'avant (aucune n'en a) et ne guérit jamais. **Changé** : pas
  de fichier de provenance ; dans le seul cas désynchronisé, l'archive
  téléchargée est extraite et comparée à l'entrée (chemins, types,
  contenus, cibles des liens, modes) : identique, l'entrée sert ;
  différente, copie étrangère. L'install suivante à vendor/ vidé sert
  l'entrée (cas normal, cache de Composer chaud : la restriction) —
  consigné avec sa repro, pas un cas du harnais.
- **7 (majeure, consignée)** — `ArchiveDownloader` l. 86-100 retire
  l'archive du cache quand l'extraction échoue : écart existant (déjà vrai
  sur un store froid), consigné, non corrigé.
- **8 (majeure, retenue en partie)** — les deux lignes « Failed to
  download … from dist » / « Source fallback is disabled » d'un paquet qui a
  une source : point 60, hors 0.26.1 par décision du mainteneur ; les
  lignes `Downloading` seulement pour une erreur de téléchargement ; le
  retrait sur l'échec suit `cleanup` (vendor/ s'il est vide, pas ses
  parents) ; les lots de plugins installés avant l'échec : consigné.
- **9 (mineure, consignée)** — Composer crée `files/` vide même si le
  téléchargement échoue.
- **10 (mineure, retenue)** — l'archive téléchargée est écrite en cache
  avant la vérification de sa somme, comme `$accept` puis l. 213-215.
- **11, 12 (mineures, retenues)** — cas `-o` avec deux archives aux mêmes
  fichiers et à classes différentes, entrée du store vérifiée inchangée,
  lot mixte, comptes de requêtes exacts (≥ 1 chez Composer dans les cas
  rouges, 0 des deux côtés dans le témoin), journal remis à zéro après la
  préparation, ligne `vivacity:` retirée, compteurs du résumé. `update` /
  `require` non ajoutés (même installeur) : non vérifiés.
- **13 (mineure, retenue)** — deux changements de plus, listés dans
  DECISIONS : le message hors ligne et les lignes `Downloading` avant une
  erreur.
- **14 (mineure, rejetée)** — sondes paresseuses : un cache en lecture
  seule change aussi le cache des dépôts (vivacity l'écrit) ; le rendu au
  démarrage est le seul sûr.

### Exécution (2026-10-31)

**Rouges sur la branche avant le point 55** (binaire de la tête de départ,
mêmes cas) : `tls-dist-store` (vivacity 0 contre 100), `desync-fetch`
(aucune requête, archive absente du cache de Composer), `desync-other-bytes`
(vendor/ de l'archive A contre B) et `desync-other-bytes-noop` (la classmap
de A), `refused-dist-store` et `offline-dist-store` (0 contre 100),
`desync-local` et `offline-local` (archive non écrite dans le cache),
`desync-stale-sha` (ni ligne ni requête), `desync-mixed`, `devnull-cache`
(aucune requête), `ttl-zero` (ni ligne ni requête), `ro-cache` (natif au
lieu de rendu), `desync-settle` (ajouté après la revue : aucune marque) ;
`tls-dist` et `dist-500` aussi, par le vendor/ vide que vivacity laissait
(les effets listent désormais les répertoires de vendor/). Le témoin
`desync-witness` (deux caches chauds : aucune requête) est vert sur la
branche avant le point 55. Après : tous verts, et les sections 1 à 7 ;
`tls-dist` compare désormais stderr.

**Vu rouge en chemin** : `vendor-isolation.sh` (`iso-upgrade-nocache`) a
attrapé une régression du premier jet — un paquet inchangé dont l'entrée
manque lisait son archive locale (le contrôle hors ligne déplacé après la
lecture locale) et l'écrivait dans le cache de Composer. Le réchauffement ne
prend plus que la copie du cache (`Fetcher::cached_dist_bytes`).

**Vérifié que la marque de copie étrangère porte** : dans le projet de
`desync-other-bytes`, la marque retirée, `dump-autoload -o` sert
l'enregistrement de l'entrée (`Probe\A`) ; remise, `Probe\A2`.

**Écarté en route** : rendre à Composer une dist à miroir préféré (méta,
faille 1) — `secure-http.sh` (`dist-mirrors`) attend ce cas natif, décision
du 2026-10-30 ; la clé du miroir reste le point 16.

### Revue indépendante (sous-agent frais, diff seul) et ce qui a changé

- **Rejeté ici, déjà consigné** : un téléchargement qui échoue interrompt
  les autres, là où `Loop::wait` les mène à terme et les met en cache —
  point 56, hors 0.26.1 par décision du mainteneur ; le comportement est
  celui d'avant (le `JoinSet` abandonné au retour).
- **Corrigé** : un paquet que l'installeur juge inchangé mais que Composer
  télécharge (transaction et identité en désaccord) était lancé deux fois,
  en réchauffement : il est téléchargé, plus réchauffé.
- **Corrigé** : le lecteur d'archives de Flex (`DistReader`) suit l'état du
  cache de fichiers (sans cache : rien lu, rien écrit).
- **Corrigé** : la marque de copie étrangère qui ne peut pas s'écrire fait
  échouer la pose (une copie non consignée servirait au dump les
  enregistrements de l'entrée) ; écrite juste avant la pose, après les
  clones git (une passation de leur part ne la laisse plus derrière).
- **Corrigé** : un fichier (ou un lien mort) à la place de `files/` rend la
  commande à Composer (`mkdir` y échoue) ; `sanitize_username` ne coupe plus
  un caractère multioctet (panique possible, code déplacé de
  vivacity-resolver).
- **Corrigé (harnais)** : seule la ligne de résumé `vivacity: N installed,`
  est retirée par `normalize`, plus toute ligne `vivacity:` ; un cas
  `desync-settle` vérifie que la marque s'efface quand la copie est reposée
  depuis son entrée.
- **Consigné** : le hachage des archives à `dist.shasum` se fait en série
  dans `download_lines` (comme Composer), avant le téléchargement parallèle.

### Archive refusée par sa somme (2026-11-01)

Le cas qui manquait à la section 8 : `dist.shasum` posé, cache de Composer
froid, store chaud chez vivacity, le serveur rend une autre archive.
Composer (FileDownloader.php l. 211-213, phar 2.10.3) imprime la ligne
`Downloading`, fait une requête, écrit l'archive dans son cache (`$accept`),
puis lève `UnexpectedValueException` « The checksum verification of the
file failed (downloaded from <`Url::sanitize($url['base'])`>) » — dans le
`then` qui suit `$reject` : ni nouvelle tentative ni URL suivante —, code 1,
vendor/ retiré par `cleanup`. vivacity faisait déjà tout cela, sauf le
message (le sien : `dist checksum mismatch for <nom>: expected sha1 …, got
…`). **Aligné** : `Error::ShasumMismatch` porte l'URL passée par
`sanitize_url` et rend le message de Composer, pour une archive téléchargée
comme pour une dist sur un chemin local (même code chez Composer).

`sanitize_url` relu contre `Url::sanitize` du phar 2.10.3 (la source 2.10.1
du store en diffère) : chaque jeu d'identifiants est masqué (le premier
seulement avant), la liste `NON_SECRET_CREDENTIALS` est celle de Composer
(`private-token` gardé tel quel, il était coupé), `GITHUB_TOKEN_REGEX` porte
sur le nom entier. Tous les appelants en héritent (messages, nom du
répertoire du cache d'un dépôt à identifiants) : vers Composer.

Cas `desync-sha-bad` et `desync-sha-bad-creds` (`http://user:<mot de
passe>@…`) : code, stdout, stderr comparée (l'encadré ramené à `Error:`, la
ligne d'usage qui le suit retirée par `normalize`, comme ailleurs), effets
(l'archive refusée dans le cache des deux côtés, pas de vendor/), une
requête de chaque côté, entrée du store restée A, mot de passe absent des
sorties des deux outils. Rouges avant ce correctif (le message), verts après.

**Reste, consigné** : `Url::sanitize` ne masque ni un autre paramètre
d'URL (`private_token=`, une signature) ni un nom court sans mot de passe
(`http://<jeton>@hôte`) ; `sanitize_url` non plus, par parité, dans les
sorties soumises au contrat Composer. **Corrigé le 2026-11-01** : l'échec de
transport d'une archive (`Error::Http`, « HTTP failure for <url> », point
54) imprimait l'URL sans l'assainir, mot de passe compris ; ce diagnostic
propre à vivacity, comme le rapport de déliaison, passe désormais par
`fetch::mask_url_secrets`, qui masque aussi ce que Composer laisse (règle,
formes couvertes et limites : DECISIONS.md, 2026-11-01, « Identifiants dans
les diagnostics de vivacity »). Corrigés ensuite, même entrée : la raison
du filet de `secure-http` (`refuse_insecure`, « … would be fetched over an
insecure protocol »), qui gardait `user:mot de passe@`, par
`mask_url_secrets` ; et le refus d'une redirection (`redirect_verdict`),
qui reprenait la cible brute là où Composer la passe par `Url::sanitize`,
par `sanitize_url` (cas `redirect-refused-*` de `harness/secure-http.sh`).
