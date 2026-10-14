# Découper `crates/vivacity/src/lib.rs` par commande — plan

> Point de forme de la critique externe (2026-09-30) : un fichier de 4 600 lignes,
> devenu 4 649. Niveau 1 : réversible, multi-fichiers, sans changement de
> comportement — vérifié par la suite et tous les harnais, qui exercent les cinq
> commandes.

## Ce qui ne bouge pas, exprès

**La surface CLI reste dans `lib.rs`** : `Cli` et les cinq structures d'arguments
clap, `run`, `dispatch`, les codes de sortie. Luther (ePHPm) a un patch en
attente d'upstream qui touche exactement ces structures (options globales
`-n`/`-q`/`-v`/`--no-ansi`, `ephpm/vivace#2`) ; les déplacer maintenant lui
imposerait un rebase pour rien. Elles pourront partir dans un `cli.rs` une fois
sa PR mergée.

## La carte

| module | contenu |
|---|---|
| `lib.rs` | CLI, `run`, `dispatch`, `trace`, codes de sortie (`TransportFailure`, `exit_code_of`, `session_error`), `ARGV`, `project_dir` |
| `fallback.rs` | `resolution_fallback`, `delegate_resolution`, `fallback_or_fail`, `which_composer` |
| `manifest.rs` | refus du manifeste (`refuse_alternate_manifest`, `refuse_invalid_root`), lectures de config (`config_bool`, `effective_optimize`, `config_lock_enabled`), `php_truthy`, `composer_dir_of`, plugin de fusion (`merge_*`, `refuse_merge_plugin_resolution`) |
| `install.rs` | `run_install`, lignes de sortie (`download_lines`, `operation_lines`), émulations de plugins (`emulate_*`), identité installée (`installed_as_locked`, `package_identity`, `unchanged_local_repository`, `installed_packages`), `print_funding`, `abandoned_warnings` |
| `dump.rs` | `run_dump`, `dump_autoload_plan`, `PlannedDump`, `dump_autoload` |
| `update.rs` | `run_update*`, `install_after_update`, `print_post_update`, `resolve_and_lock`, `patched_manifest`, `post_update_report`, contraintes temporaires, transport HTTP, `PlatformOutcome`, `missing_requirement_info`, `unused_locked_packages` |
| `remove.rs` | `run_remove`, `locally_installed` |
| `flex_install.rs` | Flex à l'install et après l'update (`flex_*`, `emulate_flex_install`, `print_flex_post_update`) — `flex.rs` reste le filtre de résolution |

Les modules de tests suivent leur sujet.

## Mécanique

Un extracteur qui prend chaque item de premier niveau avec ses commentaires de
doc et ses attributs, le colle dans son module, et le retire de `lib.rs`. Chaque
module commence par `use super::*;` (un enfant voit les items privés de son
parent) ; les items déplacés deviennent `pub(crate)`, et `lib.rs` fait
`mod x; use x::*;` — `require.rs`, qui importe par `crate::…`, continue de
résoudre sans être touché. Un module par commit, chacun compilé, lint et testé.

## Méta-analyse (auto, niveau 1)

- **Un changement de comportement caché** : un `include_str!` relatif suit son
  fichier — tous les fichiers sont dans `src/`, donc inchangé, à vérifier par
  `grep`. Un `#[cfg]` doit voyager avec son item : l'extracteur prend les
  attributs au-dessus.
- **Des commentaires de section orphelins** : à relire dans le diff.
- **`run_install` fait 720 lignes à lui seul.** Le couper est un autre chantier,
  plus risqué (logique, pas déplacement) : hors périmètre ici, noté pour la suite.
- **Le blame** : un déplacement casse `git blame` simple ; `git blame -C -C` le
  suit. Acceptable.

## Vérification

`cargo fmt --check`, clippy `-D warnings` sur macOS et
`x86_64-pc-windows-gnu`, la suite complète, et le balayage de tous les harnais à
la fin — ils appellent les cinq commandes et comparent à Composer.

## Résultat (2026-10-14)

Fait, avec un écart au plan : **un seul commit** au lieu d'un par module. Les
déplacements s'entrecroisent dans `lib.rs` (les `use x::*` successifs, des
items appelés d'un module à l'autre) ; chaque module a été compilé et linté à
son tour, mais l'historique intermédiaire n'apportait rien qu'un `git blame -C
-C` ne donne. La preuve d'absence de perte est mécanique (multiensemble des
lignes, voir DECISIONS du 2026-10-14) plutôt que par commit.

| fichier | lignes |
|---|---|
| `lib.rs` | 548 (4 649 avant) |
| `install.rs` | 1 407 |
| `update.rs` | 1 090 |
| `flex_install.rs` | 667 |
| `dump.rs` | 305 |
| `remove.rs` | 300 |
| `manifest.rs` | 208 |
| `fallback.rs` | 191 |

Les méta-points vérifiés : aucun `include_str!`/`include_bytes!` dans les
modules déplacés ; les seuls `#[cfg]` sont les `#[cfg(test)]` des modules de
tests et un `#[cfg(unix)]` intérieur à `which_composer`, partis avec leur item ;
aucun commentaire de section orphelin, aucun commentaire de doc détaché (un
préexistant, réparé). Suite : `run_install` (~720 lignes) et `cli.rs` après la
PR de Luther.
