#!/usr/bin/env python3
"""Adds `#[link_name = "vivacity_<name>"]` to every `pub fn pcre2_*_{8,16,32}`
of a pcre2-sys `bindings.rs` (crates/vivacity-pcre2-sys/src/bindings.rs).
Idempotent; rerun after rebasing the fork on a newer pcre2-sys."""
import re, sys
path = sys.argv[1] if len(sys.argv) > 1 else "crates/vivacity-pcre2-sys/src/bindings.rs"
src = open(path).read()
out, n = [], 0
lines = src.split("\n")
for i, line in enumerate(lines):
    m = re.match(r"^(\s*)pub fn (pcre2_\w+_(?:8|16|32))\(", line)
    if m and not (i > 0 and "link_name" in lines[i - 1]):
        out.append(f'{m.group(1)}#[link_name = "vivacity_{m.group(2)}"]')
        n += 1
    out.append(line)
open(path, "w").write("\n".join(out))
print(f"{n} bindings prefixed")
