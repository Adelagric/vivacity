#!/usr/bin/env bash
# `remove` et le second chargement des plugins (docs/plans/parity-gaps-026.md,
# 18g), sans réseau.
#
# `BaseCommand::initialize` charge les plugins installés sur le manifeste tel
# qu'il est ; un refus y est avalé (`tryComposer`). `RemoveCommand` édite
# composer.json, écrit « has been updated », puis recrée le `Composer`
# (RemoveCommand.php:216-222) : les plugins sont jugés sur le manifeste
# édité. Ici, aaa/a (requis, non couvert par `allow-plugins`) est refusé au
# premier chargement ; une fois retiré, il n'est plus requis (un
# avertissement) et zzz/b, requis et sans exigence de `composer-plugin-api`
# (un plugin que vivacity rend à Composer plutôt que de le juger), décide.
# vivacity doit rendre la commande avant sa première ligne et avant
# l'édition. Plugins factices : seul installed.json les déclare (aucun
# n'est chargé, aucun code lu). Comparés : code, stdout, stderr (ce qui suit
# la ligne `delegating to` côté vivacity), composer.json ; sous
# `--no-fallback` : sortie 3, rien avant la raison, composer.json intact.
#
# Usage : harness/remove-plugin-load.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VIVACITY="${VIVACITY_BIN:-$ROOT/target/release/vivacity}"
WORK="${VIVACITY_HARNESS_DIR:-/tmp/vivacity-harness}/remove-plugin-load"
[ -x "$VIVACITY" ] || { echo "binaire absent : cargo build --release"; exit 1; }
command -v composer >/dev/null || { echo "composer requis"; exit 1; }
composer --version --no-ansi 2>/dev/null | tail -1
rm -rf "$WORK"; mkdir -p "$WORK/home"; echo '{}' > "$WORK/home/config.json"
status=0
export COMPOSER_HOME="$WORK/home" COMPOSER_NO_INTERACTION=1 COMPOSER_ROOT_VERSION=1.0.0 COLUMNS=400

project() { # <répertoire>
  mkdir -p "$1/vendor/composer" "$1/vendor/aaa/a" "$1/vendor/zzz/b"
  printf '{"name": "vivacity/remove-plugin-load", "repositories": [{"packagist.org": false}], "require": {"aaa/a": "1.0.0", "zzz/b": "1.0.0"}}\n' > "$1/composer.json"
  cat > "$1/vendor/composer/installed.json" <<'EOF'
{"packages": [
  {"name": "aaa/a", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "composer-plugin", "require": {"composer-plugin-api": "^2.0"}, "extra": {"class": "Aaa\\A"}, "install-path": "../aaa/a"},
  {"name": "zzz/b", "version": "1.0.0", "version_normalized": "1.0.0.0", "type": "composer-plugin", "extra": {"class": "Zzz\\B"}, "install-path": "../zzz/b"}
], "dev": true, "dev-package-names": []}
EOF
}

# case_ <nom> <arguments…>
case_() {
  local name="$1"; shift
  local d="$WORK/$name" side bad=""
  for side in ref viv nofb; do project "$d/$side"; done
  cp "$d/ref/composer.json" "$d/start.json"
  (cd "$d/ref" && composer "$@" --no-ansi --no-scripts > "$d/ref.out" 2> "$d/ref.err"); echo $? > "$d/ref.code"
  (cd "$d/viv" && "$VIVACITY" "$@" --no-ansi --no-scripts > "$d/viv.out" 2> "$d/viv.err"); echo $? > "$d/viv.code"
  (cd "$d/nofb" && "$VIVACITY" "$@" --no-ansi --no-scripts --no-fallback > "$d/nofb.out" 2> "$d/nofb.err"); echo $? > "$d/nofb.code"
  awk 'f {print} /^vivacity: delegating to /{f=1}' "$d/viv.err" > "$d/viv.err.cmp"
  [ "$(cat "$d/ref.code")" = "$(cat "$d/viv.code")" ] || bad="$bad codes $(cat "$d/ref.code") vs $(cat "$d/viv.code");"
  cmp -s "$d/ref.out" "$d/viv.out" || bad="$bad stdout différente;"
  cmp -s "$d/ref.err" "$d/viv.err.cmp" || bad="$bad stderr différente;"
  cmp -s "$d/ref/composer.json" "$d/viv/composer.json" || bad="$bad composer.json différent;"
  grep -q '^vivacity: delegating to ' "$d/viv.err" || bad="$bad vivacity ne rend pas la commande;"
  sed -n '/^vivacity: /q;p' "$d/viv.err" > "$d/viv.pre"
  [ ! -s "$d/viv.pre" ] || bad="$bad vivacity écrit $(wc -l < "$d/viv.pre" | tr -d ' ') ligne(s) avant de rendre la commande;"
  [ "$(cat "$d/nofb.code")" = 3 ] || bad="$bad --no-fallback sort en $(cat "$d/nofb.code"), pas 3;"
  sed -n '/^vivacity: /q;p' "$d/nofb.err" | grep -q . && bad="$bad --no-fallback : des lignes avant la raison;"
  cmp -s "$d/nofb/composer.json" "$d/start.json" || bad="$bad --no-fallback a touché composer.json;"
  grep -q 'zzz/b' "$d/ref.err" || bad="$bad Composer ne nomme pas zzz/b (cas mal construit);"
  if [ -z "$bad" ]; then echo "OK   $name : même code ($(cat "$d/ref.code")), même sortie, rendu avant toute ligne"
  else echo "FAIL $name :$bad"; diff "$d/ref.err" "$d/viv.err.cmp" | head -8; status=1; fi
}

case_ second-load-hand-over remove aaa/a --no-install
case_ second-load-hand-over-update-deps remove aaa/a --no-install --update-with-dependencies
exit $status
