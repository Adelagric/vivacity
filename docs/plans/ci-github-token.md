# Le jeton du job pour GitHub, des deux côtés — plan

> Niveau 2 : touche la CI qui garde le contrat de parité, l'exposition d'un
> jeton à du code tiers (les plugins Composer que les harnais exécutent), et
> le comportement réseau de vivacity (en-tête `Authorization`). Méta-analyse
> et revue par sous-agents frais.

## Le problème

Run 40894179601 (`ci`, job `parity (macos-latest)`, main 5768dc9) : l'étape
`harness/net-retry.sh harness/flex-update.sh` est rouge. Les 25 cas échouent
dans la préparation de la référence, avant que vivacity ne tourne :

```
FAIL all-locked : la préparation a échoué (ref)
In AuthHelper.php line 152:
  Could not authenticate against github.com
```

`net-retry.sh` n'a rien relancé : son motif `NETWORK_ERROR` ne couvre pas ce
message.

## Ce qui a été lu et mesuré (avant le plan)

- **AuthHelper.php:152** (phar 2.10.3) : `promptAuthIfNeeded`, branche
  `github-domains`, après un 401/403 de `api.github.com`. En mode non
  interactif, rate limit ou non, le message est le même ; le texte qui nomme
  la limite (« GitHub API limit (%d calls/hr) is exhausted… ») n'est passé
  qu'à `authorizeOAuthInteractively`, jamais écrit. Le journal ne dit donc
  pas pourquoi GitHub a refusé.
- **Un zipball anonyme ne consomme pas le compteur `core`** : quatre
  `GET api.github.com/repos/symfony/console/zipball/…` depuis cette machine
  répondent 302 avec `x-ratelimit-used: 0`, et `/rate_limit` reste à
  `remaining: 58` avant comme après. Le refus du runner vient donc d'une
  autre limite (secondaire, par IP — les runners macOS partagent leurs IP),
  pas des « 60 requêtes/heure » au sens strict. Hypothèse, pas preuve : le
  journal ne porte pas les en-têtes.
- **setup-php écrit déjà le jeton du job dans `~/.composer/auth.json`**
  (SHA eb7c497 utilisé par le run, `add_tools.sh:set_composer_auth_helper` :
  `github-oauth.github.com = ${COMPOSER_TOKEN:-$GITHUB_TOKEN}`, l'entrée
  `github-token` valant `github.token` par défaut ; 2.10.3 hors des plages
  exclues de `composer-gh-auth-no-op`). Les harnais qui gardent le
  COMPOSER_HOME global, ou recopient son `auth.json` (plugin-lines,
  allow-plugins, custom-installers, plugins-round2/3, composer-packages,
  composer-patcher, verbosity en ligne), sont donc déjà authentifiés en CI.
- **flex-update.sh ne l'est pas** : `COMPOSER_HOME="$WORK/home"` sans copie
  d'`auth.json`, et `COMPOSER_CACHE_DIR="$home/cache"` vierge à chaque run :
  la préparation installe le lock de la fixture symfony, 153 dists
  `api.github.com`. Même forme (home privé, pas de copie) : flex-recipes,
  update, steps, path-repos, git-sources, download-lines, two-stage,
  transport-exit, artifact-repo, fallback-args (la plupart hors GitHub :
  dépôts locaux, `127.0.0.1`).
- **Permissions du jeton** : le run affiche `Contents: read, Metadata: read,
  Packages: read` (réglage du dépôt, aucun bloc `permissions:` dans ci.yml).
  `drift.yml` déclare `issues: write` au niveau du workflow : ce jeton-là
  est écrit par setup-php dans `auth.json` et lisible par les plugins tiers
  que ses harnais exécutent. `actions/checkout` persiste aussi le jeton dans
  `.git/config` (`http.https://github.com/.extraheader`).
- **Composer et le jeton** (`AuthHelper::addAuthenticationOptions`,
  `Url::getOrigin`, `CurlDownloader::restartJob`) : l'origine d'une URL est
  son hôte (port compris), `*.github.com` sauf `codeload.github.com` → 
  `github.com`, `repo.packagist.org` → `packagist.org`. Un jeton
  `github-oauth.github.com` n'est envoyé (`Authorization: token T`) qu'aux
  URL `^https?://api\.github\.com/` ; ni à `codeload.github.com`, ni à
  `github.com`, ni aux autres `*.github.com`. Sur une redirection, l'origine
  est recalculée sur la nouvelle URL avec les options d'origine : rien vers
  codeload. Un domaine GitHub Enterprise reçoit `Basic base64(T:x-oauth-basic)`.
  Une seule authentification par origine, la dernière chargée gagne
  (`BaseIO::loadConfiguration` : github-oauth, …, http-basic, bearer).
- **vivacity** (`fetch.rs:Auth::authorization_for(host)`) : `token T` vers
  `github.com` ET tout `*.github.com` (codeload compris — le test
  `auth_header_selection` l'affirme : « github dists go through codeload »),
  `token T` vers un domaine GHE, préséance github > bearer > basic, clés par
  hôte (pas d'origine : `repo.packagist.org` ne lit pas `packagist.org`).
  reqwest retire `Authorization` sur une redirection inter-hôte
  (`redirect.rs:239`), donc le 302 api → codeload est sûr ; mais une URL de
  dist directement sur codeload ou github.com reçoit le jeton, que Composer
  n'envoie pas. Avec un jeton valide la sortie est la même ; avec un jeton
  invalide ou révoqué, codeload répondrait 401 à vivacity et 200 à Composer.
- **Le jeton ne change pas la sortie de Composer aux niveaux comparés** :
  « Loading auth config from COMPOSER_AUTH » et « Using GitHub token
  authentication » sont écrits en `DEBUG` (-vvv), que vivacity rend toujours
  à Composer (`lib.rs`, `requested >= 3`) ; `Git::runCommands` n'utilise
  l'authentification qu'après l'échec d'un clone ; deux sources du même
  jeton (auth.json recopié + COMPOSER_AUTH) se fusionnent sans
  l'avertissement d'écrasement (`Config::merge` par domaine, valeurs égales).
- `composer-cache-dir.sh` lance tout sous `env -i` (dépôt `artifact` local,
  aucun accès GitHub) : le jeton n'y entre pas, et son cas `composer-auth`
  pose sa propre variable.

## Cadrage

- **Contraintes** : rien poussé ; diffs de workflow minimaux et locaux (un
  autre chantier réépingle composer 2.10.3 → 2.11.0 dans les mêmes
  fichiers) ; un PR depuis un fork reçoit un jeton en lecture seule (ou rien)
  et la CI doit marcher sans.
- **Non-objectifs** : réchauffer le cache de flex-update (piste suivante,
  notée) ; porter gitlab/bitbucket/custom-headers ; l'avertissement
  « avoid overwriting already defined auth settings » quand deux types
  d'authentification visent une origine (écart préexistant, noté).
- **Alternatives** : (a) recopier `auth.json` dans chaque harnais à home
  privé, comme le font déjà sept harnais — couvre ce qui est écrit
  aujourd'hui, pas le prochain harnais, et dépend d'un détail d'implémentation
  de setup-php ; (b) `COMPOSER_AUTH` au niveau du job — chaque harnais, chaque
  côté, sauf `env -i` (qui n'en a pas besoin) ; (c) réchauffer les caches pour
  ne plus télécharger — réduit le volume, ne supprime pas la dépendance à
  l'IP partagée. Retenu : (b), (c) en piste suivante.
- **Critères de succès observables** :
  1. vivacity envoie le jeton `github-oauth` exactement là où Composer
     l'envoie, prouvé par un harnais différentiel qui voit les en-têtes,
     vu rouge sur main ;
  2. le jeton ne change ni stdout, ni stderr, ni le projet, d'un côté comme
     de l'autre (même harnais : avec et sans jeton) ;
  3. `net-retry.sh` nomme un refus de GitHub au lieu de le laisser passer
     pour une régression, sans relancer, et garde « une vraie régression
     échoue deux fois » ; prouvé sur l'extrait du journal capturé, vu rouge
     sur l'ancien script ;
  4. chaque workflow qui exécute du code tiers tourne avec
     `permissions: contents: read` ; `issues: write` n'existe plus que dans
     un job qui n'exécute aucun plugin ;
  5. gates Rust verts, actionlint propre.

## Plan

1. **`fetch.rs`** — `Auth::authorization_for(url)` à la place de
   `(host)`, décidé par origine comme Composer : `Url::getOrigin`
   (port compris, `*.github.com` sauf codeload → github.com,
   repo.packagist.org → packagist.org), `findAuthOrigin`
   (`api.github.com`/`api.bitbucket.org` → domaine canonique), une
   authentification par origine dans l'ordre de chargement de Composer
   (bearer > http-basic > github-oauth), puis l'en-tête :
   bearer → `Bearer`, http-basic → `Basic`, github-oauth → `token T`
   seulement pour `^https?://api\.github\.com/` quand l'origine est
   github.com, `Basic T:x-oauth-basic` ailleurs. Les trois appelants passent
   l'URL. Test d'abord (rouge sur l'ancien code : codeload, github.com,
   GHE, préséance, packagist.org).
2. **`harness/auth-hosts.sh`** — un mandataire HTTP local (python3) qui
   journalise `(méthode, URL absolue, Authorization)` et sert un zip ;
   un lock à dists `http://` (api.github.com, codeload.github.com,
   github.com, objects.github.com, un domaine GHE déclaré dans
   `github-oauth`) ; `secure-http: false`, `http_proxy` des deux côtés.
   Comparés : le journal du mandataire (Composer contre vivacity), et
   stdout/stderr/projet entre les deux outils et entre « avec jeton » et
   « sans jeton ». Vu rouge avec le binaire de main.
3. **`net-retry.sh`** — motif `GITHUB_REFUSAL` (Composer : « Could not
   authenticate against github.com », « GitHub API limit (…) is
   exhausted », « API rate limit exceeded » ; vivacity : `(401
   Unauthorized|403 Forbidden) for url (https?://api.github.com/`) :
   aucune relance, une ligne qui nomme la cause et dit si un jeton était
   configuré, `::error::` sous GitHub Actions, code de sortie du harnais
   inchangé. Préséance sur la relance réseau (une relance consomme encore
   la limite). **`harness/net-retry-selftest.sh`** : faux harnais
   (rejeu de l'extrait capturé `fixtures/net-retry/…`, erreur réseau puis
   vert, réseau deux fois, différence sans réseau, vert), vu rouge sur
   l'ancien script.
4. **Workflows** — `permissions: contents: read` en tête de ci, windows,
   action-test, bench, ci-cache, corpus ; `COMPOSER_AUTH` gardé
   (`github.token != ''`) au niveau des jobs `parity` de ci.yml et
   windows.yml ; corpus.yml retombe sur `github.token` sans secret ;
   drift.yml : `contents: read` pour le job qui exécute les harnais, le
   rapport (gh issue) déplacé dans un job `report` (`issues: write`,
   `needs: drift`, aucun checkout, aucun plugin) qui lit le résultat de
   chaque canal dans un artefact. Deux étapes ajoutées à ci.yml
   (auth-hosts, net-retry-selftest).
5. **Docs** : ce plan (méta, exécution, revue), DECISIONS (2026-10-29),
   HANDOVER (lignes de validation, risque), CHANGELOG (Unreleased).

Gates : `cargo build --release`, `cargo clippy --all-targets -- -D
warnings`, `cargo fmt --check`, `cargo test` (aussi sous un
`COMPOSER_AUTH` factice, pour voir qu'aucun test ne dépend de son
absence), `actionlint`, les deux nouveaux harnais, flex-update.sh.

## Méta-analyse (sous-agent frais, cadrage + plan seuls, code ouvert)

Huit failles remontées ; chacune vérifiée dans le code avant d'être prise
ou écartée.

| # | faille | gravité | suite |
|---|---|---|---|
| 1 | vivacity ignore `secure-http` (aucune occurrence dans `crates/`) : un dist `http://api.github.com/…` que Composer refuse (`Config::prohibitUrlByConfig`, Config.php:702) recevrait le jeton en clair | critique (préexistant) | **écart préexistant, hors jeton** : vivacity télécharge déjà les dists `http://` que Composer refuse, et y envoie déjà `http-basic`/`bearer`. Le plan réduit les hôtes qui reçoivent le jeton GitHub (de tout `*.github.com` à `api.github.com`) ; il ne l'aggrave pas. Signalé comme écart de parité à part (HANDOVER), pas corrigé ici : le corriger change l'issue d'installs `http://` existants |
| 2 | ordre des sources : Composer fusionne auth.json global < `config` de composer.json < auth.json du projet < COMPOSER_AUTH (Factory.php:209, 328-345 — `loadComposerAuthEnv` en dernier) ; vivacity : global < COMPOSER_AUTH < projet | majeure | **pris** : `Auth::load` dans l'ordre de Composer, test d'abord. Les clés d'auth posées dans `config` (composer.json, config.json global) restent ignorées : noté |
| 3 | deux types d'auth pour une origine : Composer écrit « avoid overwriting already defined auth settings » ; reproduire le gagnant sans la ligne diverge en silence | majeure | **en partie** : le gagnant est aligné (une auth par origine, ordre de chargement), la ligne reste un écart noté. Rendre la main à Composer demanderait de faire remonter l'état d'`Auth` jusqu'à la décision de repli de chaque commande — hors de ce chantier |
| 4 | rien ne prouve que le jeton guérit le run macOS ; avec un jeton, un second 401/403 donne le même message (AuthHelper.php:89, 152) ; un zipball authentifié compte peut-être dans les 1 000/h du dépôt | critique | **pris** : (a) une étape de diagnostic dans le job `parity` qui lit `/rate_limit` (gratuit) au début et à la fin, avec et sans jeton, pour mesurer en CI ce qu'on ne peut pas mesurer ici ; (b) flex-update.sh et flex-recipes.sh prennent le cache de fichiers global (celui qu'`actions/cache` restaure, qui contient déjà les dists de la fixture symfony via fixtures/make.sh) au lieu d'un cache vierge par run — le volume tombe de ~150 téléchargements à ~0 ; (c) critère ajouté : flex-update vert sur macos-latest |
| 5 | `COMPOSER_AUTH` au niveau du job touche les fichiers de l'autre chantier ; la garde `github.token != ''` est du code mort (un PR de fork reçoit un jeton en lecture) | majeure | **garde retirée** (`github.token` est toujours émis). `COMPOSER_AUTH` gardé : hunk à part (en tête de job), indépendant de setup-php, couvre aussi le côté vivacity et le prochain harnais à home privé |
| 6 | job `report` de drift.yml avec `needs: drift` : sauté quand `drift` échoue, c'est-à-dire quand il sert ; l'artefact vient d'un job qui exécute du code tiers | majeure | **pris** : `if: ${{ !cancelled() && needs.drift.result == 'failure' }}`, l'artefact est écrit et envoyé dès qu'une étape de dérive a échoué (pas `always()` : une erreur de build n'ouvrait pas d'issue avant, elle n'en ouvre pas maintenant), champs lus par variables et validés par motif strict avant d'aller dans `gh` |
| 7 | motifs de net-retry : « GitHub API limit … » et « API rate limit exceeded » ne sont jamais écrits en non interactif ; parenthèse non échappée ; 429 et codeload absents ; un 401 propre à vivacity serait mis sur le dos de GitHub ; beaucoup de harnais n'impriment pas la stderr de l'outil | majeure | **pris** : seuls les messages réellement imprimés (Composer non interactif, `failResponse` 403/429 sur api/codeload, reqwest 401/403/429), échappés ; le libellé dit quel côté a été refusé, et un refus vu seulement côté vivacity est nommé « peut-être une régression de vivacity », pas « GitHub » ; la couverture partielle (harnais muets) est notée |
| 8 | auth-hosts.sh ne teste pas la redirection api → codeload ; « Warning: Accessing X over http » de Composer à filtrer | mineure | **pris** : le mandataire répond 302 vers codeload pour les zipballs api, le filtre est déclaré |

Écarté : « recopier auth.json dans chaque harnais plutôt que COMPOSER_AUTH »
(c'est l'alternative (a) du cadrage, re-proposée) — elle dépend de ce que
setup-php écrit, ne couvre pas le côté vivacity d'un harnais qui n'a pas
de home global, et doit être refaite à chaque nouveau harnais.

## Plan révisé (retour 1)

Il remplace le plan ci-dessus là où ils diffèrent (cache global des
harnais flex, plus de garde sur `github.token`), gardé tel que la
méta-analyse l'a lu.

1. `fetch.rs` : ordre de chargement de Composer (global < projet <
   COMPOSER_AUTH) ; `authorization_for(url)` par origine (inchangé
   ailleurs). Tests rouges d'abord.
2. `harness/auth-hosts.sh` : mandataire avec 302 api → codeload, filtre
   « Accessing … over http » déclaré, cas sans jeton / avec jeton
   (COMPOSER_AUTH) / jeton dans auth.json du projet ET COMPOSER_AUTH (ordre).
3. `net-retry.sh` + `harness/net-retry-selftest.sh` (motifs imprimés
   seulement, côté nommé).
4. flex-update.sh, flex-recipes.sh : cache de fichiers global.
5. Workflows : permissions, `COMPOSER_AUTH` sans garde, diagnostic
   `/rate_limit` dans `parity` (ci.yml), drift scindé avec job `report`
   protégé.
6. Docs.

## Exécution

- `fetch.rs` : `Auth::load` → `load_from` (global, projet, COMPOSER_AUTH) ;
  `authorization_for(host)` → `authorization_for_url(url)` (renommée : un
  appelant de la bibliothèque qui passait un hôte ne compile plus, au lieu
  de recevoir `None` en silence) ; `composer_origin` ; trois appelants.
  Après la revue : la paire (utilisateur, mot de passe) décide du rendu,
  comme `addAuthenticationOptions` (un http-basic à mot de passe `bearer`
  ou `x-oauth-basic` se rend comme chez Composer).
- `net-retry.sh` : classement du refus de GitHub ; après la revue, la
  recherche se fait sur le journal privé de tout blanc (l'encadré de
  Symfony coupe un long message à 76 colonnes, n'importe où).
- `harness/net-retry-selftest.sh` (13 cas) + `fixtures/net-retry/` (extrait
  du journal du run 40894179601, horodatages du runner retirés).
- `harness/auth-hosts.sh` (mandataire python3 local).
- flex-update.sh, flex-recipes.sh : `CACHE_DIR` global.
- Workflows : ci, windows, action-test, bench, ci-cache, corpus, drift.

## Vérification

Machine : Darwin arm64 (M4 Max), PHP 8.4.27 / Composer 2.10.3, bash 3.2.57.

- Tests Rust de `fetch.rs` : écrits d'abord ; contre l'ancienne sémantique
  (règle par hôte, ordre d'avant, adaptée à la nouvelle signature) les
  trois tests sont rouges — `Some("token ghtok")` pour codeload,
  `Some("token g")` au lieu de `Bearer`, `"project"` au lieu de `"env"` —
  verts après.
- `harness/auth-hosts.sh` avec le binaire de main 5768dc9 : **rouge** sur
  `env`, `order` et `env (vivacity)` (codeload, github.com,
  objects.github.com reçoivent `token T-ENV`, le domaine GHE `token T-GHE`,
  l'auth.json du projet passe devant COMPOSER_AUTH) ; Composer donne le
  journal attendu ligne à ligne. Avec le correctif : 10/10 OK, y compris
  « sans jeton = avec jeton » pour stdout, stderr et projet des deux côtés.
- `harness/net-retry-selftest.sh` : 13/13 sur le nouveau script (bash
  3.2), 8 FAIL sur celui de main (les cas GitHub), les 5 cas de
  comportement inchangé verts sur les deux.
- Rapport de drift.yml rejoué en local avec un faux `gh` : un fichier
  valide ouvre l'issue (label créé avant) ; une version piégée
  (`$(touch …)`, accents graves) devient `(unreadable)` sans exécution ; un
  canal inconnu est ignoré ; aucun fichier : rien, sortie 0.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo build --release` : verts. `cargo test` sous un `COMPOSER_AUTH`
  factice : tests unitaires de vivacity-core (191), vivacity (81),
  vivacity-resolver (77), vivacity-autoload (20) verts ; les tests
  d'intégration ne tournent pas ici — ils exigent `fixtures/work`
  (fixtures/make.sh) et le disque de la machine n'avait plus la place (le
  volume est descendu à 120 Mo pendant le chantier) : le pas « Tests » de
  ci.yml les fait tourner, sous le nouveau `COMPOSER_AUTH`.
- `actionlint` : propre sur les huit workflows. `shellcheck` : notes
  d'information seulement, sur des formes déjà présentes.
- **Non vérifié ici** : flex-update.sh et flex-recipes.sh sur le cache
  global (~7,7 Go de répertoire de travail chacun, le disque ne les tenait
  pas) — le job `parity` de ci.yml les fait tourner sur les deux OS.

## Revue (sous-agent frais, diff seul)

Aucun bug bloquant ; vérifiés corrects : ordre de fusion, préséance,
règles d'origine, port explicite, `findAuthOrigin`, GHE, `format()` (JSON
valide), job `report`, motifs de net-retry sous bash 3.2 et 5, filtre http
d'auth-hosts. Pris : renommage `authorization_for_url` ; motifs sur texte
sans blancs + deux cas encadrés au selftest (avant ce changement, une
coupure de Symfony cachait la 403 de codeload et la 429) ; rendu par
valeurs ; commentaires (redirection, casse, COMPOSER_AUTH mal formé) ;
incohérences de ce plan. Notés comme écarts (HANDOVER) : jetons GitLab,
`x-token-auth` de Bitbucket, `custom-headers`, `client-certificate` ; une
redirection vers une origine qui a ses propres identifiants (Composer les
ajoute, reqwest non) ; la casse des hôtes et des clés.
