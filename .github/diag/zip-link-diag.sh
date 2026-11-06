#!/usr/bin/env bash
# What Composer does with a zip holding a link that leads out of the package, and
# which engine extracts it. Prints everything; decides nothing. A scenario is a PATH.
# usage: zip-link-diag.sh <label> <path-for-composer>
set -u
label="$1"; run_path="$2"
base="$(mktemp -d)"; base="$(cd "$base" && pwd -P)"
work="$base/work"; www="$base/www"; mkdir -p "$work" "$www"
printf 'x\n' > "$base/witness"; chmod 600 "$base/witness"
python3 -I - "$www" <<'PY'
import json, stat, sys, zipfile
www = sys.argv[1]
z = zipfile.ZipFile(f"{www}/link.zip", "w", zipfile.ZIP_DEFLATED)
z.writestr("composer.json", '{"name": "acme/tool"}\n')
i = zipfile.ZipInfo("bin/out")
i.create_system = 3
i.external_attr = (stat.S_IFLNK | 0o777) << 16
z.writestr(i, "../../../../../witness")
z.close()
PY
port=$(python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
python3 -I - "$www" "$port" <<'PY'
import json, sys
www, port = sys.argv[1], sys.argv[2]
json.dump({"packages": {"acme/tool": {"1.0.0": {"name": "acme/tool", "version": "1.0.0", "bin": ["bin/out"], "type": "library",
  "dist": {"type": "zip", "url": f"http://127.0.0.1:{port}/link.zip", "reference": "abc", "shasum": ""}}}}}, open(f"{www}/packages.json", "w"))
PY
python3 -I -m http.server "$port" --bind 127.0.0.1 --directory "$www" >"$base/srv.log" 2>&1 &
srv=$!
trap 'kill "$srv" 2>/dev/null; rm -rf "${base:?}"' EXIT
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do curl -s -o /dev/null "http://127.0.0.1:$port/packages.json" && break; python3 -I -c 'import time; time.sleep(0.3)'; done
cat > "$work/composer.json" <<JSON
{"name":"acme/root","version":"1.0.0","repositories":[{"type":"composer","url":"http://127.0.0.1:$port"},{"packagist.org":false}],"require":{"acme/tool":"1.0.0"},"config":{"secure-http":false}}
JSON
echo "--- direct extraction with the unzip on the PATH"
( PATH="$run_path"; if command -v unzip >/dev/null; then unzip -qq "$www/link.zip" -d "$base/direct"; echo "unzip rc=$?"; ls -ld "$base/direct/bin/out" 2>&1; [ -L "$base/direct/bin/out" ] && echo "direct type: symlink -> $(readlink "$base/direct/bin/out")"; else echo "no unzip"; fi )
run() { ( cd "$work" && env -u COMPOSER_AUTH PATH="$run_path" COMPOSER_HOME="$base/home" COMPOSER_CACHE_DIR="$base/cache" COMPOSER_NO_INTERACTION=1 COMPOSER_NO_AUDIT=1 composer "$@" ); }
echo "=== scenario: $label"
( PATH="$run_path"; for t in unzip 7z 7zz php composer; do printf '%s: ' "$t"; command -v "$t" || echo none; done )
run update --no-install --no-scripts --no-progress >"$base/lock.out" 2>"$base/lock.err"; echo "lock rc=$?"
rm -rf "${base:?}/cache"
run install --no-interaction --no-scripts --no-progress >"$base/out" 2>"$base/err"; echo "install rc=$?"
echo "--- stderr, whole"; cat "$base/err"
echo "--- stdout, whole"; cat "$base/out"
echo "--- bin/out"; ls -ld "$work/vendor/acme/tool/bin/out" 2>&1
[ -e "$work/vendor/acme/tool/bin/out" ] || [ -L "$work/vendor/acme/tool/bin/out" ] && { [ -L "$work/vendor/acme/tool/bin/out" ] && echo "type: symlink -> $(readlink "$work/vendor/acme/tool/bin/out")" || echo "type: not a link, $(wc -c < "$work/vendor/acme/tool/bin/out") bytes"; }
echo "--- vendor/bin"; ls -l "$work/vendor/bin" 2>&1
echo "--- witness mode"; stat -f '%Lp' "$base/witness" 2>/dev/null || stat -c '%a' "$base/witness"
printf -- '--- phrase of the test: '; grep -c ': the bin resolves to a path outside of the package directory' "$base/err"
rm -rf "${work:?}/vendor" "${base:?}/cache"
run install --no-interaction --no-scripts --no-progress -vvv >"$base/vout" 2>"$base/verr"; echo "--- -vvv rc=$?, engine lines"
grep -h -iE 'unzip|7z|ZipArchive|Extracting|falling back' "$base/verr" "$base/vout" | cut -c1-200
