# Couper `run_install` en phases nommées — plan

> Suite notée le 2026-10-14 (`docs/plans/split-lib-rs.md`) : après le découpage
> de `lib.rs`, `run_install` (`crates/vivacity/src/install.rs`) faisait encore
> 737 lignes d'un seul tenant. Niveau 2 : le contrat est la parité à l'octet
> (stdout, stderr, code de sortie, fichiers écrits, dans le même ordre), et
> `install` est appelé par toutes les commandes et tous les harnais. Méta-analyse
> et revue par sous-agents frais.

## Le problème

Une fonction de 737 lignes qui suit déjà, dans ses commentaires, la structure
de Composer (`InstallCommand`, puis `Installer::run` → `doInstall`) mais qu'on
ne peut pas lire en regard de lui : quinze phases, une quarantaine de variables
locales vivantes de bout en bout, et des points de sortie (`return Ok(n)`,
`fallback_or_fail`) mêlés aux calculs. Un changement dans une phase demande de
relire tout ce qui la précède pour savoir ce qui est encore en vie.

**Non-objectif, explicitement : corriger quoi que ce soit.** Un défaut repéré en
route est noté dans le résultat et laissé tel quel.

## Décisions

- **Un contexte, `Run`**, construit par la première phase : ce que toutes les
  phases partagent et que plus rien ne modifie après le chargement — les
  arguments, `t0`, le projet, le texte de composer.json (le content-hash et
  `config.lock` lisent le fichier), le manifeste effectif (préférence
  d'installation appliquée, racine fusionnée), la racine fusionnée elle-même,
  le lock (`Value` et `Lock`), `with_dev`, le `Runner` de `--run-scripts`.
- **Une fonction par phase**, dans l'ordre d'`Installer`. Celles qui peuvent
  arrêter la commande rendent `anyhow::Result<Flow<T>>`, avec
  `type Flow<T = ()> = std::ops::ControlFlow<i32, T>` : `Break(code)` est la
  sortie (le code que l'ancien `return Ok(code)` rendait), `Continue(v)` ce
  dont la suite a besoin. Une macro `proceed!` fait pour `Flow` ce que `?` fait
  pour `Result`, dans `run_install` seulement. Pas de type à moi :
  `ControlFlow` dit déjà « continuer ou s'arrêter avec ».
- **Les états intermédiaires sont des structures nommées** quand ils groupent
  plusieurs valeurs : `LocalTransaction` (l'arène, les présents, les noms du
  raccourci no-op, la transaction, les exigences manquantes), `Installed` (le
  rapport de l'installeur et son runtime tokio).
- **Le runtime tokio vit jusqu'à la fin de `run_install`, comme avant.** Son
  `drop` attend les tâches `spawn_blocking` encore en cours ; le lâcher à la
  fin de la phase d'installation déplacerait cette attente avant le dump. Sur
  le chemin nominal tout est joint et ça ne changerait rien d'observable, mais
  le refactor ne doit pas avoir à le prouver : `Installed` le garde.
- **Les `trace(…)` restent dans leur phase, à la même position**, mêmes
  libellés : ce sont des points de mesure, pas des frontières de phase.
- **Un seul fichier.** `install.rs` garde `run_install`, ses phases juste en
  dessous dans l'ordre d'exécution, puis les assistants et les émulations
  inchangés. Un module frère pour les phases aurait rendu le diff illisible
  (tout déplacé) pour un gain nul : `run_install` est le sommaire.
- **Aucun littéral ne change**, y compris les doublons : les deux boucles
  « abandonnés » (dry run et install réelle) restent deux boucles, pas un
  assistant — la preuve mécanique (multiensemble des littéraux identique)
  reste alors vide de toute explication.

## La carte

| phase | ce qu'elle fait (contrepartie Composer) | sorties possibles |
|---|---|---|
| `read_manifests` | `--no-install` refusé, manifestes (refus de `InstallCommand`/`Factory`), préférence d'installation effective, lock (ou le lock virtuel d'un dry run), `with_dev` | 1, codes de refus, erreurs |
| `merge_plugin` | racine de composer-merge-plugin (INIT), rendue dans le contexte | fallback |
| `pre_install_cmd` | `--run-scripts` : le `Runner` (rendu dans le contexte), `pre-install-cmd` (`Installer::run`, avant tout) | 3, code du script |
| `command_listeners` | `COMMAND` de bamarni/composer-bin-plugin | — |
| `headline` | en-tête, notice de vérification de plateforme, avertissement de fraîcheur (`doInstall`) ; rend `verify_start`, que lisent `join_policy` et le `say_lock_resolution` posé dans `run_install` entre `report_missing` et `report_operations` | — |
| `spawn_policy_check` | la requête de listes de blocage, sur son thread | erreur de transport |
| `check_platform` | calculée, rapportée après la jointure | erreur |
| `local_repository_transaction` | `LocalRepoTransaction` (raccourci no-op sur les entrées brutes), `getMissingRequirementInfo` | erreur |
| `analyze_scope` | portée, calculée pendant l'attente | — |
| `plan_autoload_early` | le dump planifié pendant l'attente réseau | — (un `Result` gardé) |
| `join_policy` | jointure : problèmes de politique | 2 |
| `report_platform` | plateforme | 4 |
| `report_missing` | exigences manquantes (fallback merge-plugin) | fallback, 4 |
| `report_operations` | `Package operations` / `Nothing to install…` | — |
| `dry_run` | lignes d'opérations, abandonnés, funding | 0 |
| `native_layout` | décision de portée : fallback, notes, `--check-scope` | fallback, 0 |
| `execute_operations` | lignes, téléchargements, transaction, refus d'émulation, puis les abandonnés (`Installer::run`, avant le dump, hors `--no-autoloader`) | fallback, 1, erreur |
| `dump_autoloader` | `pre-autoload-dump`, dump (planifié ou non), plugins `post-autoload-dump` émulés, `post-autoload-dump` | code du script, erreur |
| `post_install` | dealerdirect, extension-installers, funding, Flex, `post-install-cmd` | code du script, erreur |
| `say_summary` | la ligne finale de vivacity | 0 |

## Invariants (ce que la relecture doit vérifier, phase par phase)

1. **Le thread de politique** démarre après l'avertissement de fraîcheur et
   avant la plateforme, la transaction et la portée ; il est joint avant toute
   impression ou écriture au-delà ; ordre de jointure politique (2) →
   plateforme (4) → exigences manquantes — l'ordre de vivacity aujourd'hui,
   que le commentaire attribue à `doInstall` à tort (voir la méta du
   sous-agent : Composer teste les exigences manquantes d'abord). Une erreur
   entre le lancement et la jointure (plateforme, dépôts, exigences) le
   détache comme avant (le handle est lâché au retour).
2. **Le dump planifié pendant l'attente** : seulement quand la transaction est
   vide, jamais avec `--dry-run`, `--no-autoloader` ou `--run-scripts`, sur une
   portée native ; son `Result` n'est inspecté que là où le dump tourne (même
   point, même texte d'erreur).
3. **Le raccourci no-op** (`unchanged_local_repository` sur les entrées
   brutes, arène non chargée) reste.
4. **Les points de fallback** (`fallback_or_fail`) restent à la même place par
   rapport à la sortie : racine fusionnée (avant tout), exigences manquantes
   sous merge-plugin non installé, portée non native (après « Package
   operations », après le retour du dry run), refus d'émulation de
   l'installeur (après la transaction tentée, runtime vivant).
5. **Les différences `after_update`** : pas de thread de politique, pas de
   notice de vérification ni d'avertissement de fraîcheur, pas de ligne de
   résolution, pas d'abandonnés, pas de funding ni de Flex install, c3 en mode
   update ; `handed_over` posé par `fallback_or_fail` seulement.
6. **Les `trace`** : mêmes libellés, mêmes positions relatives.
7. **L'ordre des erreurs `?`** : chaque opération faillible garde sa place
   relative (par exemple `http_transport` avant la plateforme, `operation_lines`
   avant la création du runtime).
8. **Les durées de vie** : rien de ce qui a un `Drop` observable ne change de
   moment — le runtime tokio (voir plus haut) ; les autres valeurs sont des
   données sans `Drop` (vérifié : les seuls `impl Drop` du dépôt sont pcre2 et
   `io::Scope`, hors de `run_install`).

## Vérification et critère « vert »

- Preuve mécanique : multiensemble des littéraux de chaîne de `install.rs`
  avant/après, diff vide.
- `cargo fmt --check` ; clippy `-D warnings` macOS et `x86_64-pc-windows-gnu` ;
  `cargo test` (304 attendus).
- `cargo build --release` puis **tous** les harnais (ils appellent tous
  `install`), chacun à 0, dont `diff-vendor --with-autoloader` cache froid puis
  chaud.
- Perf : no-op `install --no-plugins` sur une copie installée de laravel, 5
  passages, médiane, binaire de `main` contre celui-ci.

## Méta-analyse adversariale, première passe (auteur du plan)

- **`ControlFlow` contre un enum maison.** Un enum à moi (`Exit`/`Continue`)
  ne dirait rien de plus et ajouterait un type ; `ControlFlow` n'a pas de `?`
  stable dans une fonction qui rend `Result`, d'où la macro. Risque : une
  macro cache un `return` — limitée à `run_install`, un seul bras, nom
  explicite.
- **Le contexte qui grossit.** Le danger d'un `Run` est qu'il devienne le sac
  de toutes les variables. Règle tenue : n'y va que ce qui est fixé avant la
  première impression et lu par plusieurs phases ; le reste passe en
  arguments, ce qui montre dans chaque signature ce que la phase consomme.
- **Le `Runner` dans `Run`** alors qu'il est créé par la deuxième phase : soit
  un champ posé après coup, soit un argument de plus pour trois phases. Champ
  posé une fois dans `run_install`, juste après `pre_install_cmd` — lisible
  dans le sommaire. (Revu après la méta : `pre_install_cmd` rend le contexte.)
- **Le clone du manifeste fusionné** (`m.manifest.clone()`) : gardé tel quel,
  un refactor ne change pas les coûts.
- **L'ombre `report`** : dans l'ancien code, le rapport du dump masque celui de
  l'installeur à l'intérieur du bloc du dump. Les deux deviennent des noms
  distincts dans des fonctions distinctes ; à relire pour qu'aucun champ ne
  soit lu sur le mauvais.
- **Une sortie oubliée.** Chaque `return` de l'ancien code doit avoir son
  `Break` ou son `Err` : à recompter (21 `return`/`bail!` dans l'ancienne
  fonction).

## Méta-analyse (sous-agent frais, sur le code, 2026-10-16) — et ce qui change

Le relecteur a lu `run_install`, ses appelants, `fallback_or_fail` et
`docs/reference/Installer.php`. Ce qu'il a trouvé, et la suite donnée :

- **La preuve par multiensemble ne voit pas l'ordre** (majeur). Un littéral
  déplacé de l'autre côté d'un `?`, un `say!` devenu `say_always!` (même
  `eprintln!`, autre niveau sous `-q`/`-v`), un code de sortie changé (ce n'est
  pas une chaîne) passent tous. **Adopté** : la preuve compare désormais des
  *séquences* — les littéraux dans l'ordre du fichier, les paires (macro ou
  fonction appelée, littéral), les codes de sortie, les appels à effet — et la
  séquence des libellés `VIVACITY_TRACE` produite par l'ancien et le nouveau
  binaire sur une fixture. Pour que la séquence des littéraux reste identique,
  deux choix du premier jet sont revenus en arrière : la préférence
  d'installation est réécrite en ligne dans `read_manifests` (un assistant
  défini après elle déplaçait ses trois littéraux), et les avertissements
  « abandonné » de l'installation réelle sont à la fin d'`execute_operations`
  plutôt que dans `run_install` (qui est en tête de fichier).
- **Le décompte des sorties était faux** (21 annoncées) : 20 `return` et 3
  `bail!` dans l'ancienne fonction, plus 32 `?` et le `Ok(0)` final, et chaque
  `fallback_or_fail` a trois issues (3, le code de Composer, une erreur).
  Recompté mécaniquement avant/après (voir le résultat).
- **L'ordre de jointure n'est pas celui de `doInstall`** (majeur, préexistant).
  Composer teste `getMissingRequirementInfo` (sortie 4) *avant* `createPool`
  et le solveur (`Installer.php`, l. 778-786 puis 808-836) : exigences
  manquantes d'abord, politique et plateforme ensuite. vivacity fait
  politique (2) → plateforme (4) → manquantes. Visible seulement quand un lock
  cumule une exigence manquante et un paquet bloqué ou un échec de
  plateforme. **Non corrigé** (non-objectif) ; l'invariant 1 décrit l'ordre de
  vivacity, pas celui de Composer, et le défaut est noté au résultat.
- **`COMMAND` de bamarni après `pre-install-cmd`** (mineur, préexistant) :
  Composer émet `PluginEvents::COMMAND` dans la commande, avant
  `$install->run()` (même schéma dans `UpdateCommand.php`), donc avant
  `pre-install-cmd`. Visible seulement avec `--run-scripts`, bamarni actif et
  un `pre-install-cmd` qui écrit. Noté, laissé.
- **Le thread de politique imprime lui-même** (la ligne « could not be fully
  loaded » de `repository.rs`), à un moment quelconque entre lancement et
  jointure : aucune phase entre les deux ne doit gagner d'impression.
  Vérifié — `check_platform`, `local_repository_transaction`, `analyze_scope`,
  `plan_autoload_early` n'impriment rien, comme avant.
- **Le calcul faillible avant la jointure doit le rester** :
  `missing_requirement_info(...)?` et `platform_packages(...)?` sortent en
  erreur avant la jointure (thread détaché). Tenu : calculés dans
  `local_repository_transaction` et `check_platform`, seulement *rapportés*
  par `report_missing` et `report_platform`.
- **`ScopeReport` et la disposition** : le relecteur craignait un
  `layout.clone()`. `native_layout` prend le rapport par valeur et en *sort*
  la disposition (`let Some(layout) = scope.layout else …`, déplacement
  partiel ; `skipped_plugins` reste lisible) : ni clone ni emprunt à travers
  les phases.
- **Funding avant `post-install-cmd` chez Composer** (nit, préexistant) :
  `Installer.php` imprime le funding (l. 413-433) avant `POST_INSTALL_CMD`
  (l. 440), où écoutent dealerdirect et les extension-installers ; vivacity
  les fait avant le funding. Ils n'impriment rien sauf en échec. Noté.
- **Sur-découpage** : il proposait de fusionner `join_policy`,
  `report_platform` et `report_missing`. **Écarté** : l'ordre de ces trois
  étapes est précisément le défaut noté plus haut ; trois appels dans
  `run_install` le montrent là où on lira pour le corriger. `say_summary`
  reste une fonction (en fin de fichier, son littéral garde sa place dans la
  séquence). `dry_run` rend `i32` : il ne peut pas échouer.
- **`Run` modifié après coup** (nit) : **adopté** — `pre_install_cmd` prend le
  contexte par valeur et le rend avec son `Runner` (`Run { runner, ..run }`),
  comme `merge_plugin` le rend avec la racine fusionnée ; plus de `mut`.
- **Mesure de perf trop faible** : 5 passages sous le bruit d'un chemin de
  40-120 ms. **Adopté** en plus de la médiane demandée : hyperfine, 3 de
  chauffe, 30 mesurés, avec et sans plugins.

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-16) — et ce qui a changé

Rien de bloquant : il a refait à la main le diff fonction par fonction
(conditions, arguments — `local` contre `lock`, `previously_present`, le mode
update de c3 —, ombres, ordre des `?` par rapport aux impressions, durées de
vie du runtime et du handle, mises à jour de structure `..run`, appelants).
Corrigé, commentaires seulement :

- la doc nouvelle de `run_install` répétait que le rapport suit l'ordre de
  `doInstall` (faux, voir la méta) : reformulée. Le commentaire d'origine à la
  jointure, déplacé tel quel, garde l'affirmation — à corriger avec le défaut ;
- la doc de `local_repository_transaction` décrivait des sorties qu'elle ne
  produit plus (reprise du commentaire d'origine) : elle dit maintenant ce
  qu'elle calcule et qui le rapporte ; « stops here with code 4 » dit où ;
- le repère `` `Installer::run` `` au-dessus de `command_listeners` renvoie au
  défaut `COMMAND` ; un repère `` `Installer::run`, after `doInstall` `` marque le
  retour dans `run()` avant le dump ;
- `dump_autoloader` : « le dump d'`Installer::run`, après `doInstall` »
  (`Installer.php`, l. 376-397) ; le commentaire d'origine sur « optimized »,
  qui l'attribue à `doInstall`, est laissé ;
- le paramètre `installed: &InstallReport` de `dump_autoloader`/`post_install`
  s'appelle `report` : dans `post_install`, un `let installed: Vec<…>` le
  masquait avec un autre type ;
- la doc de `post_install` suit l'ordre réel (Flex après le funding).

## Résultat (2026-10-16)

`run_install` : 45 lignes qui se lisent contre `Installer::run`/`doInstall`.
Les phases, dans l'ordre (lignes, signature et accolades comprises, doc non
comptée) : `read_manifests` 91, `merge_plugin` 27, `pre_install_cmd` 21,
`command_listeners` 10, `headline` 32, `spawn_policy_check` 24,
`check_platform` 39, `local_repository_transaction` 53, `analyze_scope` 17,
`plan_autoload_early` 47, `join_policy` 34, `report_platform` 33,
`report_missing` 38, `report_operations` 29, `dry_run` 16, `native_layout` 30,
`execute_operations` 94, `dump_autoloader` 72, `post_autoload_dump_plugins`
71, `post_install` 54, `say_summary` 21 ; plus `Run` (18), `LocalTransaction`
(11), `Installed` (4), `Flow`, `PolicyThread` et la macro `proceed!`.
`install.rs` passe de 1 440 à 1 754 lignes (docs des phases, signatures) ;
les assistants, émulations et tests en dessous sont inchangés. Un seul
fichier, comme décidé.

**Preuves mécaniques** (scripts dans le répertoire de travail, ancien
`install.rs` = `git show main:…`) :

- séquence des 250 littéraux (tokeniseur Rust : commentaires sautés, chaînes
  brutes et caractères gérés) : **identique** ; multiensemble `grep -o
  '"[^"]*"'` (248) : identique ;
- séquence des paires (macro ou fonction appelée, littéral) : identique — voit
  un `say!` devenu `say_always!` ;
- séquence des sorties (`return Ok(n)`/`Break(n)`, `bail!`, `return Err`) :
  identique une fois `Break(n)` lu `return Ok(n)`, à un écart près : le
  `return Ok(0)` du dry run est la valeur `0` que rend `dry_run` ;
- séquence des appels à effet (116) : identique à deux déplacements près, tous
  deux dus au placement : l'appel `say_lock_resolution` d'après les exigences
  manquantes est dans `run_install` (en tête de fichier), et
  `post_autoload_dump_plugins`, appelée par `dump_autoloader` avant
  `post-autoload-dump`, est définie après elle ;
- **vu rouge** : `say!` → `say_always!` sur « Nothing to install… » et une
  sortie 2 → 1 dans `join_policy` font chacune échouer le contrôle concerné ;
- `VIVACITY_TRACE=1`, ancien contre nouveau binaire sur une copie installée de
  laravel, 12 modes (`--offline`, `--no-plugins`, `--dry-run`, `--check-scope`,
  `-v`, `-q`, `--no-autoloader`, `--no-dev`, `--prefer-source`, `--no-fallback`,
  `--no-install`) : libellés dans le même ordre, stderr (durées masquées),
  stdout et code identiques. Une fois, la ligne `filter lists` — imprimée par
  le thread de politique — est arrivée une place plus tôt : course entre
  threads, pas d'ordre de code ; six reprises alternées donnent la même
  séquence des deux côtés.

**Portes** : `cargo fmt --check` ; clippy `-D warnings` macOS et
`x86_64-pc-windows-gnu` ; `cargo test` 304 réussis, 0 échec (36 binaires),
comme sur `main`. **Harnais** (binaire final, chacun dans son répertoire) :
les 29 passages sortent à 0 — diff-vendor, diff-vendor `--with-autoloader`
deux fois (froid puis chaud), update, steps, removal, transitions,
path-repos, vendor-dir, merge-plugin, scripts, tar-dist, yii2-composer,
bin-plugin, wp-core, custom-dirs, package-versions, flex-install,
flex-update, download-lines, artifact-repo, compare-selftest, two-stage,
transport-exit, root-scan, boot, drift-reference (129 fichiers identiques),
verbosity, fallback-args — 584 lignes `OK`, 0 `FAIL`. Un premier balayage
s'était arrêté sur un disque plein (« No space left on device », dans
Composer comme dans les scripts) : relancé en entier après nettoyage.

**Perf** (Mac M4 Max, laravel installé, hyperfine `-N`, binaires de `main` et
de cette branche alternés) : `install --no-plugins` 5 passages, médiane
114,0 ms (`main`) contre 113,8 ms ; 30 passages après 3 de chauffe, 119,7
contre 116,4 ms (en ligne, σ 8-12 ms) ; `--no-plugins --offline` 47,3 contre
47,3 ms ; `--offline` avec plugins 48,1 contre 48,4 ms. Dans le bruit.

**Défauts vus, laissés tels quels** (préexistants, suites possibles) :

1. L'ordre de la jointure : vivacity rapporte politique (2) → plateforme (4)
   → exigences manquantes ; `doInstall` teste les exigences manquantes avant
   le pool et le solveur. Un lock qui cumule les deux sort autrement que chez
   Composer. Le commentaire de la jointure dit le contraire du code de
   référence.
2. Le `COMMAND` de bamarni passe après `pre-install-cmd` (Composer : avant
   `Installer::run`). Visible avec `--run-scripts`. **Corrigé le
   2026-10-16** (branche `install-order`, commit « Print install and
   update events in Composer's order », `docs/plans/install-order.md`),
   avec le même défaut, plus visible, sur `update`/`require`/`remove`.
3. Funding avant `POST_INSTALL_CMD` chez Composer ; vivacity émule
   dealerdirect et les extension-installers avant le funding. Visible
   seulement sur un échec de ces émulations. **Corrigé le 2026-10-16**
   (même commit) ; la prémisse était fausse : ces plugins écrivent chez
   Composer en cas nominal (phpstan à chaque install), vivacity n'imprime
   pas leurs lignes du tout — et codeception/c3, lui, écrivait avant le
   funding, ce qui rendait le défaut visible sans `--run-scripts`.
4. La doc d'`installed_packages` est détachée au-dessus de celle
   d'`unchanged_local_repository` (depuis avant le découpage de `lib.rs`).
5. Le commentaire « `Installer::doInstall`: "optimized" » attribue à
   `doInstall` une ligne d'`Installer::run`.
