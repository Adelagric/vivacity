# M7 — Ce que coûte un artefact de cache CI (2026-09-16)

Question posée par Seldaek sur composer/composer#11132 : si le store extrait
remplace les zips dans le répertoire de cache, produire et restaurer
l'artefact de cache CI (tar + zstd, ce que fait `actions/cache`) devient-il
plus lent que le gain à l'installation ? `bench/ci-cache.sh`, exécuté par
`.github/workflows/ci-cache.yml` (run 35118103891) sur ubuntu-latest (x86_64,
4 vCPU, ext4), fixture Sylius (276 paquets), hyperfine 5 runs.

| | cache zips (Composer) | store extrait (vivacity) |
|---|---|---|
| sur disque | 100 MB, 276 fichiers | 403 MB, 40 552 fichiers |
| produce (tar \| zstd -3) | 201 ms | 998 ms |
| restore | 156 ms | 1 741 ms |
| artefact | 86 MB | 66 MB |

| install, vendor supprimé | médiane |
|---|---|
| composer, zips chauds | 5 373 ms |
| vivacity, zips chauds, store froid | 2 145 ms |
| vivacity, store chaud | 997 ms |

Lecture. Mettre le store en artefact coûte ~2,4 s de plus par job
(produce + restore) pour gagner ~1,1 s à l'installation (store froid →
chaud) : ça ne paie pas, Seldaek a raison — de peu sur ext4, et l'artefact
compressé est même plus petit (zstd voit le contenu redondant entre
paquets). vivacity n'en a pas besoin : l'artefact CI reste le cache zips de
Composer (même `COMPOSER_CACHE_DIR`), le store en est dérivé à la volée ; un
job CI à une seule installation passe de 5,4 s à 2,1 s sans rien changer à
son cache, et le store chaud (1,0 s) est le cas du deuxième install sur la
même machine — celui du poste de dev.

Même mesure sur macOS (M4 Max, APFS, 14 cœurs, 2026-09-16) : produce 14,9 s,
restore 16,3 s pour le store contre 0,23 s / 0,22 s pour les zips ; install
6,5 s / 4,5 s / 0,73 s. APFS est lent à créer 40 000 fichiers ; le chiffre
Linux est celui qui compte pour un worker CI.
