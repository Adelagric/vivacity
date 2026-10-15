# Le fallback rejoue la ligne de commande — plan

> Suite notée hors périmètre par `docs/plans/v0.21-verbosity.md` (revue,
> majeur 2). Niveau 2 : le fallback est le filet de tout ce que vivacity ne
> reproduit pas, et ePHPm embarque `vivacity::run` dans son processus.

## Le problème, lu dans le code (2026-10-16)

`fallback.rs` a trois façons de rendre la main à Composer :

| fonction | argv | processus | répertoire | `--offline` |
|---|---|---|---|---|
| `delegate_for_verbosity` (0.21) | argv d'origine, `forwarded_arguments` | fils | celui de vivacity | `COMPOSER_DISABLE_NETWORK=1` |
| `delegate_resolution` (update/require/remove) | argv d'origine, `forwarded_arguments` | fils | **`current_dir(project)`** | **perdu** |
| `fallback_or_fail` (install) | **reconstruit** de 9 champs | **`exec()` sous unix** | `current_dir(project)` | **perdu** |

Mesuré avec un faux `composer` qui enregistre son argv (binaire 0.21.0) :

- `install --dry-run --working-dir proj --offline` sur la fixture merge-plugin
  privée de son fichier requis (`ext/composer.ext.json`) : Composer reçoit
  `install --no-scripts` — **une installation réelle** pour un dry run, sans
  `--offline`. Le retour anticipé du dry run (`install.rs`, avant l'analyse de
  portée) ne protège que le fallback de portée, pas les deux fallbacks du
  plugin de fusion, qui sont avant lui.
- `install --working-dir proj --offline --no-progress -v --no-plugins
  --ignore-platform-reqs` sur laravel avec `preferred-install: source` :
  Composer reçoit `install --no-scripts --no-plugins --ignore-platform-reqs`,
  dans `proj`. `--no-progress` perdu ; `--prefer-*`, `--apcu*`,
  `--no-blocking`, `-n`, `--no-ansi`, `--dry-run` le seraient aussi.
- `delegate_resolution` : `--working-dir proj` relatif est appliqué deux fois
  — vivacity calcule `project = cwd/proj`, puis lance Composer *dans*
  `project` avec `--working-dir proj` : Composer cherche `cwd/proj/proj`.
- `exec()` remplace le processus : chez un embarqueur, l'hôte disparaît.

## Décisions

1. **Un seul modèle : l'argv d'origine, en processus fils, dans le répertoire
   de vivacity.** `fallback_or_fail` et `delegate_resolution` passent par la
   même fonction que `delegate_for_verbosity` (`hand_over`) : la ligne
   `vivacity: delegating to …`, `SHELL_VERBOSITY`, `COMPOSER_DISABLE_NETWORK=1`
   sous `--offline`, le code de sortie du fils. Un `--working-dir` relatif
   garde son sens parce que le répertoire courant ne change pas — Composer
   l'applique lui-même (`Application::doRun`, `chdir`).
2. **`forwarded_arguments` reconnaît les alias** (`r`, `rm`) pour le filtre de
   `--no-autoloader` : `vivacity rm x --no-autoloader -vv` le passait à un
   `remove` qui ne le connaît pas. Et `delegate_resolution` filtre d'après la
   commande de l'argv, pas d'après son paramètre : la délégation Flex de
   `resolve_and_lock` passe `Update` même pour un `remove`.
3. **Phase d'installation d'un `update`/`require`/`remove` : une ligne
   construite, complète, pas l'argv traduit.** À ce point la résolution est
   faite : composer.lock est écrit (sauf `config.lock: false`), composer.json
   l'est pour `require`/`remove`, vendor/ n'est pas touché. Composer, lui,
   enchaîne `doInstall` sur le lock qu'il vient de résoudre. Rejouer la
   commande d'origine ferait refaire la résolution (lignes en double) et,
   pour `remove`/`require`, une seconde édition d'un manifeste déjà édité
   (`remove` d'un paquet déjà retiré : « is not required in your
   composer.json »). Traduire l'argv demanderait une table par commande
   (`--update-no-dev` → `--no-dev`, `--dev` de `require` à jeter, options à
   valeur séparée à sauter, `-oW` groupés) : fragile. Donc `composer install`
   construit depuis les arguments analysés de l'update (`after_update_line`) :
   `--no-dev`, `--no-progress`, `--no-autoloader`, `-o`, `-a`,
   `--apcu-autoloader[-prefix]`, `--ignore-platform-req[s]`, `--no-plugins`,
   `--no-blocking`, `--no-security-blocking`, `-n`, `--no-ansi` (propagés
   depuis `require`/`remove`, qui les avaient), `--working-dir` tel que donné,
   `--dry-run` (inatteignable aujourd'hui — un dry run sort avant la portée et
   la fusion échoue avant l'install — mais un fallback ne doit jamais écrire
   à la place d'un dry run), et `--no-scripts` (ni `update` ni
   `require`/`remove` n'ont `--run-scripts`). Exclus, exprès :
   l'**audit** — `install --audit` pose aussi `setErrorOnAudit` (sortie 5 sur
   un paquet vulnérable) là où `update` audite sans échouer : ce n'est pas le
   même régime, et vivacity n'audite pas nativement non plus ; `--prefer-*` —
   `update`/`remove` ne les acceptent pas, `require` les accepte sans les lire
   (le point est déjà noté dans CONTRIBUTING).
4. **Après une installation rendue à Composer, vivacity n'imprime pas son
   rapport d'après-update.** `composer install` imprime déjà les abandonnés et
   le funding (`Installer::run`) ; le rapport de vivacity les doublait
   (`require`/`remove` le faisaient déjà en mode fils ; `update` passait par
   `exec` et n'imprimait rien après). Ce qui manque reste : les suggestions,
   que seul `update` imprime. Le `updateRequirementsAfterResolution` de
   `require` (contraintes devinées) tourne toujours : il écrit composer.json.
5. **`--dry-run` sur un lock hors portée : reste natif.** Ce que la portée
   détecte (disposition, sources, plugins à l'install) ne change pas la liste
   d'opérations d'un dry run, lue dans le lock. Mesuré : laravel avec
   `preferred-install: source`, `install --dry-run --no-plugins` — stderr et
   stdout identiques à Composer 2.10.3 (112 lignes). Les fallbacks atteints
   avant le retour du dry run (plugin de fusion) transmettent désormais
   `--dry-run` : Composer fait un dry run, jamais une installation.
6. **`spawn_fallback` disparaît** (deux champs `#[arg(skip)]`) : sans `exec`,
   il ne veut plus rien dire. `InstallArgs` gagne à la place
   `fallback_line: Option<Vec<OsString>>` (la ligne de la décision 3) et
   `handed_over: Cell<bool>` (la décision 4).

## Fichiers, dans l'ordre

1. `crates/vivacity/src/fallback.rs` — `hand_over`, `command_line`,
   `after_update_line`, `fallback_or_fail` et `delegate_resolution` dessus
   (paramètre `project` retiré des deux), alias dans `forwarded_arguments`,
   module `#[cfg(test)]`.
2. `crates/vivacity/src/lib.rs` — `InstallArgs` (`fallback_line`,
   `handed_over`), `UpdateArgs` (`spawn_fallback` retiré).
3. `crates/vivacity/src/install.rs` — appels de `fallback_or_fail`.
4. `crates/vivacity/src/update.rs` — `install_after_update` (ligne construite,
   rend `(statut, rendu)`), `run_update_resolved`, `delegate_resolution` ×2.
5. `crates/vivacity/src/require.rs`, `remove.rs` — `-n`/`--no-ansi` dans
   l'`UpdateArgs` synthétisé, `spawn_fallback` retiré, rapport sauté si rendu.
6. `harness/fallback-args.sh` + `.github/workflows/ci.yml`.
7. CHANGELOG, DECISIONS, HANDOVER, README, CONTRIBUTING (liste des gates).

## Tests et critère « vert »

- Unitaires (`fallback.rs`) : `--` (rien après n'est filtré, `--no-scripts`
  inséré avant), chaque option propre à vivacity retirée, position de
  `--no-scripts`, `--run-scripts` → pas de `--no-scripts`, `--no-autoloader`
  retiré sur `require`/`remove`/`r`/`rm` et gardé sur `install`/`update`,
  `after_update_line` depuis un `update` et depuis les arguments réels.
- `harness/fallback-args.sh` : faux `composer` en tête du `PATH` qui enregistre
  argv (une ligne par argument), cwd, `SHELL_VERBOSITY`,
  `COMPOSER_DISABLE_NETWORK`, et sort avec un code choisi. Scénarios :
  (a) `install` hors portée (laravel, `preferred-install: source`) depuis le
  répertoire parent avec `--working-dir` relatif, sous `-q`, `-v`, avec
  `--prefer-source`, `--no-progress`, `--offline`, `--run-scripts` ; le dry
  run du même lock, natif (rien d'enregistré) ; le dry run hors portée du
  plugin de fusion, `--dry-run` transmis ; le code de sortie du fils rendu ;
  (b) `update`/`require`/`remove` avec un plugin installé hors liste,
  `--working-dir` relatif ; (c) la phase d'installation d'un `update` et d'un
  `require` (paquets `path` et `preferred-install: source` : un dépôt
  `package` n'est pas lu par l'`update` de vivacity), lock écrit avant ;
  (d) `--no-fallback` : sortie 3, rien d'enregistré ; (e) `-vv` inchangé.
  Vu rouge sur le binaire 0.21.0 avant d'être compté vert.
- Gates : fmt, clippy (hôte et `x86_64-pc-windows-gnu`), `cargo test`, et les
  harnais transitions, steps, diff-vendor, verbosity, transport-exit,
  two-stage, flex-update, merge-plugin : la sortie normale ne bouge pas.

## Méta-analyse adversariale, première passe (auteur du plan)

**Options que vivacity accepte et que `composer install` refuserait.**
Comparé à `composer install --help` (2.10.3) : `InstallArgs` n'a en propre que
`--no-fallback`, `--check-scope`, `--offline`, `--run-scripts`, tous retirés
par `forwarded_arguments`. `--no-install` existe chez Composer (pour refuser,
comme vivacity, qui sort avant tout fallback). Côté phase d'installation, la
ligne est construite : seules des options d'`install` y entrent ; le
`--no-autoloader` d'un `remove`/`require` (propre à vivacity là-bas) y est
légitime, `install` le connaît.

**`--`.** Rien après `--` n'est filtré, `--no-scripts` est inséré avant
(sinon Composer le lirait comme un argument). clap refuse `install -- x` (pas
d'argument positionnel) ; `require -- a/b` est le cas réel. Testé.

**`--working-dir` relatif.** Le modèle tient parce que le fils hérite du
répertoire de vivacity. Risque : un appelant qui comptait sur
`current_dir(project)` — vérifié : `delegate_resolution` et `fallback_or_fail`
sont les seuls, et sans `--working-dir` `project` *est* le répertoire courant.
La phase d'installation transmet `--working-dir` tel que donné, pas le
chemin absolu : même sens, même répertoire courant. Forme `--working-dir=x` :
transmise telle quelle. `-d` : vivacity ne l'accepte pas (clap sort en 2).

**`COMPOSER`.** `refuse_alternate_manifest` refuse toute autre valeur que
`composer.json` avant d'arriver au fallback ; le fils hérite de
l'environnement de toute façon.

**La phase d'installation.** Trois chemins l'atteignent (`update`, `require`,
`remove`) avec un `UpdateArgs` synthétisé par les deux derniers : tout ce qui
n'y est pas recopié est perdu — d'où `-n`/`--no-ansi` à propager, et un test
qui construit la ligne depuis un vrai argv. Un dry run d'update installe
depuis un lock virtuel : un fallback y lirait le composer.lock du disque (pas
le bon) — inatteignable aujourd'hui (voir décision 3), `--dry-run` transmis
garantit au moins qu'il n'écrit rien. `config.lock: false` : le lock n'est
pas écrit et l'install relit celui du disque, natif comme rendu — hors
périmètre, noté.

**Le rapport d'après-update (décision 4).** Le passage d'`exec` à un fils
aurait fait imprimer à `update` son rapport *après* celui de Composer
(abandonnés et funding en double) : régression évitée par `handed_over`.

**`.bat` sous Windows.** `which_composer` trouve `composer.bat` ; Rust lance un
`.bat` par `cmd.exe` et **refuse** (erreur `InvalidInput`) un argument qu'il
ne sait pas échapper sans risque. L'ancienne ligne reconstruite n'avait que
des options sûres ; l'argv de l'utilisateur peut contenir n'importe quoi
(`--apcu-autoloader-prefix='a&b'`). Résultat : `Error: cannot run composer`,
sortie 1, rien de lancé — refus sûr, déjà le comportement de
`delegate_resolution` et `delegate_for_verbosity`. Non testé sous Windows
(le harnais tourne sur ubuntu et macos).

**Codes de sortie.** Le code du fils est rendu ; tué par un signal → 1 (avec
`exec`, le processus mourait du signal lui-même : un shell voyait 128+n).
`Ctrl-C` : le terminal l'envoie au groupe, vivacity et Composer meurent
ensemble.

**Niveau de la ligne `delegating to`.** Normal (muette sous `-q`), comme
avant ; elle montre maintenant la ligne transmise entière. Les harnais qui la
cherchent (`steps.sh` : `delegating to \`composer`, `diff-vendor.sh` :
`delegating to`) restent valides.

**`--offline` sur le vrai fallback.** `diff-vendor.sh` lance
`vivacity install --offline` sur sylius, rector, drupal, qui passent par le
fallback : Composer y tournait **avec** le réseau, il tournera sous
`COMPOSER_DISABLE_NETWORK=1`. Il lit ses dists dans le cache que la référence
vient de remplir, mais le chargement de la liste de filtrage (packagist) et
tout ce qui n'est pas en cache échoueront. C'est le sens de `--offline` ;
c'est aussi un changement observable : à mesurer sur diff-vendor avant de
conclure.

**Trouvés en lisant, hors périmètre, notés pour la suite :**
- Le fallback de portée d'`install` est décidé *après* que vivacity a imprimé
  l'en-tête, « Verifying… », « Package operations » (et la liste `-v`) :
  Composer imprime ensuite les siens — en double sur stderr. Mesuré sur
  laravel ci-dessus.
- `--run-scripts` hors portée : `pre-install-cmd` est lancé par vivacity
  avant la décision de portée, puis Composer (sans `--no-scripts`) le relance.
- `require --prefer-source` est accepté et ignoré (CONTRIBUTING dit « rejeté »).
- La ligne de raison de `delegate_resolution` nomme `update` quand la
  délégation Flex de `resolve_and_lock` part d'un `remove`.

## Méta-analyse (sous-agent frais, sur le code, 2026-10-16) — et ce qui change

Huit constats ; deux majeurs ont rouvert le plan.

**1. Majeur — la délégation Flex de `resolve_and_lock` n'arrête pas la
commande.** `update.rs` rendait `Resolved { status: code }` ; sur un succès de
Composer (0), `run_update_resolved` enchaînait l'install par-dessus (lock relu
du disque, en-tête, autoloader, funding en double), `require` réécrivait ses
contraintes devinées et le hash du lock, `remove` restaurait ou vérifiait
« still present » par-dessus Composer. Atteignable : `flex_reinstall_reason`
(Flex verrouillé, pas installé — un clone frais) vaut pour les trois
commandes. **Changement :** `Resolved::hand_over` — `resolve_and_lock` ne lance
plus Composer, il dit pourquoi ; l'appelant le lance (`hand_resolution_over`)
et rend son code, sans rien après.

**2. Majeur — et Composer recevait un composer.json déjà édité** par
`require`/`remove` : exactement la double édition que la décision 3 écarte
pour la phase d'installation. **Changement :** l'appelant défait son édition
en silence avant de lancer Composer (`Files::restore` pour `require`, la
sauvegarde pour `remove`) ; sous `--no-fallback` aussi.

**3. Majeur — `--prefer-*` : le plan avait tort.** `RequireCommand` les lit
(`getPreferredInstallOptions` → `setPreferSource`/`setPreferDist`) et
`update` les accepte chez Composer ; vivacity `require` les acceptait sans les
lire, son install posait des dists là où Composer prend les sources.
**Changement :** champs internes `prefer_*` de `UpdateArgs`, remplis par
`require`, lus par la phase d'installation (la portée les voit, la ligne
construite les porte). L'`update` de vivacity ne les accepte toujours pas
(CONTRIBUTING, corrigé : il disait `require` refusant). Une préférence
impossible est refusée avant la résolution, comme `doUpdate` (ajouté après la
revue, qui a vu le lock laissé sur le disque sinon).

**4. Mineur — `config.lock: false`.** Composer pointe alors le Locker sur le
périphérique nul (`Factory.php:424-428`) : `composer install` ne lit pas le
lock, il résout à nouveau. Rendue à Composer, la phase d'installation fait
donc un update — le même résultat qu'un update. Côté natif, `require` refuse
ce cas ; `update`/`remove` relisent le lock du disque — écart préexistant,
noté, hors périmètre.

**5. Mineur — le fallback d'un dry run d'update est atteignable** (plugin de
fusion seulement verrouillé : `refuse_merge_plugin_resolution` le laisse
passer, l'install le fusionne). `--dry-run` transmis : rien d'écrit ; la liste
de Composer vient alors du lock du disque, pas du lock virtuel — écart noté.

**6. Mineur — `--offline` atteint aussi `harness/boot.sh`** (sylius, rector,
drupal par le fallback, plugins actifs) : ajouté à la liste des harnais
lancés pour cette vérification.

**7. Mineur — le fallback d'`install` dépend maintenant de l'`ARGV` global**
(comme ceux de résolution, et comme le niveau de verbosité) : deux `run`
concurrents dans un même processus se le partagent. Inchangé, noté.

**8. Mineur — la sortie d'une install rendue après update diffère encore**
(« Verifying lock file contents… » de `composer install`, qu'un update
n'imprime pas ; l'en-tête déjà imprimé par vivacity) ; et le message
`--no-fallback` « no partial vendor/ was written » reste vrai mais tait que le
lock l'est déjà. Noté.

Vérifié juste par la méta : la double application de `--working-dir` et ses
seuls appelants ; `-d` et `install -- x` refusés par clap ; aucune option
d'`InstallArgs` que `composer install` refuserait une fois les nôtres
retirées ; l'exclusion de l'audit ; l'absence de `--no-autoloader` sur le
`require`/`remove` de Composer et le trou des alias ; le saut du rapport
après une install rendue (seules les suggestions manquent,
`Installer.php:338-362`) ; l'ordre de la mise à jour des contraintes de
`require` (`RequireCommand.php:337-340`, après l'install) ; le retour du dry
run avant la portée ; les lignes `delegating to` cherchées par les harnais.

Harnais étendu en conséquence : scénario f (Flex verrouillé, pas installé,
sur `update`/`remove`/`require`, avec le cksum du composer.json que Composer
reçoit, et `--no-fallback`), et `require --prefer-source`.

## Revue indépendante (sous-agent frais, sur le diff, 2026-10-16) — et ce qui a changé

Aucun défaut de flot de contrôle dans le code Rust ; neuf constats.

1. CONTRIBUTING décrivait encore `require` sans `--prefer-*` : réécrit (et le
   reste du chemin pour `update`).
2. **Le scénario f ne jouait qu'un Composer qui échoue (4)** — or le défaut
   trouvé par la méta était la suite d'un *succès*. Joué maintenant avec 0 et
   4, pour les trois commandes.
3. Rien ne prouvait que f passe par la délégation d'après la résolution : le
   harnais exige la raison de `flex_reinstall_reason` (« would install
   itself »), qu'aucun contrôle d'avant l'édition ne donne.
4. Le commentaire d'`after_update_line` disait le dry run inatteignable,
   contre la méta §5 : corrigé.
5. Une préférence impossible, refusée à la phase d'installation, laissait le
   lock écrit quand il n'existait pas avant : refusée maintenant avant la
   résolution (`c-require-bad-prefer`).
6. `Files::restore`, utilisé pour la délégation, supprimait un lock existant
   quand composer.json venait d'être créé : la délégation défait l'édition de
   composer.json seule (`Files::undo_edit`) — le lock n'est pas encore écrit.
7. Valeurs en mots séparés dans la ligne construite : `--name=value` en un
   mot (Symfony refuse une valeur séparée qui commence par `-`).
8. `c-require-fails` vérifie aussi la restauration du lock.
9. Les dates (2026-10-16, demandées) et le scénario c du plan (paquets `path`,
   pas un dépôt `package`) : le second corrigé.

## Résultat (2026-10-16)

`harness/fallback-args.sh` : 29 cas, 21 rouges sur le binaire 0.21.0, tous
verts ; tests unitaires de `forwarded_arguments` et `after_update_line` ; la
suite complète et les harnais transitions, steps, diff-vendor, verbosity,
transport-exit, two-stage, flex-update, merge-plugin, boot verts — la sortie
normale n'a pas bougé (drupal passe par le fallback dans diff-vendor et boot,
désormais sous `COMPOSER_DISABLE_NETWORK=1`, depuis le cache : identique).
Restent, notés pour la suite : le fallback de portée d'`install` décidé après
l'en-tête et « Package operations » (doublés par Composer) ; `pre-install-cmd`
deux fois sous `--run-scripts` hors portée ; une délégation d'après la
résolution réimprime « has been updated » / « Running composer update » ;
`--prefer-*` sur `update` ; `config.lock: false` à l'install d'un
`update`/`remove` natif.
