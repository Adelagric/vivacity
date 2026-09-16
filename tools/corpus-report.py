#!/usr/bin/env python3
"""results.jsonl of harness/corpus.sh -> the Markdown report docs/corpus/<date>.md.

Stable ordering (by name, then mode) so two reports diff cleanly. Counts by
project AND by distinct name@version package (a corpus of Laravel apps
shares most of its packages); the fallback reasons ranked; every diff with
its first lines; the plugins annotated with the events they listen to
(all active in the baseline: `--no-scripts` only skips composer.json scripts).

Usage: tools/corpus-report.py <results.jsonl> [<date>] > docs/corpus/<date>.md
"""
import collections
import datetime
import json
import re
import sys

# What each plugin listens to (getSubscribedEvents), for the reader: in
# Composer, `--no-scripts` only switches off the composer.json scripts
# (`EventDispatcher::setRunScripts`); every plugin listener still runs in
# the baseline, so a plugin in this table is ACTIVE there, never inert.
PLUGIN_EVENTS = {
    "symfony/thanks": "post-install-cmd/post-update-cmd only",
    "ergebnis/composer-normalize": "a command, no install-time listener",
    "bamarni/composer-bin-plugin": "post-install-cmd/post-update-cmd",
    "drupal/core-composer-scaffold": "post-install-cmd/post-update-cmd/post-package-*",
    "drupal/core-project-message": "post-create-project-cmd/post-install-cmd",
    "mautic/core-composer-scaffold": "post-install-cmd/post-update-cmd",
    "mautic/core-project-message": "post-create-project-cmd/post-install-cmd",
    "phpstan/extension-installer": "pre-autoload-dump (already emulated as benign)",
    "dealerdirect/phpcodesniffer-composer-installer": "post-install-cmd/post-update-cmd",
    "cweagans/composer-patches": "pre-install-cmd/pre-update-cmd + post-package-install",
    "wikimedia/composer-merge-plugin": "pre-install-cmd/pre-update-cmd/post-package-install (also plugin init merging)",
    "laminas/laminas-component-installer": "post-package-install/uninstall",
    "yiisoft/yii2-composer": "post-create-project-cmd/post-install-cmd (also installer)",
    "spiral/composer-publish-plugin": "post-install-cmd/post-update-cmd",
    "ondrejmirtes/composer-attribute-collector": "post-autoload-dump",
    "metasyntactical/composer-plugin-license-check": "pre-operations-exec (active without scripts)",
    "mlocati/composer-patcher": "pre-install-cmd/pre-update-cmd + post-package-*",
    "silverstripe/vendor-plugin": "installer + post-install-cmd",
    "silverstripe/recipe-plugin": "post-package-install/update",
}


def load(path):
    rows = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    rows.sort(key=lambda r: (r["name"], r["mode"]))
    return rows


def reason_key(reason):
    m = re.match(r"plugin (\S+) is not on", reason)
    if m:
        return ("plugin", m.group(1))
    m = re.match(r"plugin (\S+) changes the install layout", reason)
    if m:
        return ("layout-plugin", m.group(1))
    m = re.match(r"package (\S+) has no usable dist", reason)
    if m:
        return ("no-dist", m.group(1))
    m = re.match(r"config (\S+) ", reason)
    if m:
        return ("config", m.group(1))
    m = re.search(r"framework `([^`]*)`", reason)
    if m:
        return ("installers", m.group(1))
    m = re.match(r"installers: \S+ \(([^)]*)\)", reason)
    if m:
        return ("installers", m.group(1))
    return ("other", reason)


def main():
    path = sys.argv[1]
    date = sys.argv[2] if len(sys.argv) > 2 else datetime.date.today().isoformat()
    rows = load(path)
    modes = sorted({r["mode"] for r in rows}, key=lambda m: (m != "dev", m))
    out = []
    p = out.append
    p(f"# Corpus report — {date}")
    p("")
    p("Baseline: Composer 2.10.3 `install --no-scripts` (plugins loaded and fully "
      "active — `--no-scripts` only skips the composer.json scripts) against "
      "`vivacity install --no-fallback`, `--ignore-platform-req=ext-*` on both "
      "sides (`php` too where this machine's PHP fails the lock, noted per entry). "
      "Buckets: **native** = 0 diff in `vendor/` (files, modes, link targets); "
      "**fallback** = vivacity stopped before any write, with the reason; "
      "**diff** = a parity bug; **unavailable** = Composer itself failed. "
      "See `fixtures/corpus/README.md` for the inputs and `harness/corpus.sh` "
      "for the method.")
    p("")
    names = sorted({r["name"] for r in rows})
    p(f"Entries: {len(names)}.")
    p("")
    for mode in modes:
        mrows = [r for r in rows if r["mode"] == mode]
        p(f"## Mode `{mode}`")
        p("")
        by_bucket = collections.Counter(r["bucket"] for r in mrows)
        counted = [r for r in mrows if r["bucket"] != "unavailable"]
        total = len(counted)
        p("| bucket | projects | share | distinct packages |")
        p("|---|---:|---:|---:|")
        for bucket in ["native", "fallback", "diff", "unavailable"]:
            brows = [r for r in mrows if r["bucket"] == bucket]
            pk = {pkg for r in brows for pkg in r.get("packages", [])}
            share = f"{100 * len(brows) / total:.0f} %" if total and bucket != "unavailable" else "—"
            p(f"| {bucket} | {len(brows)} | {share} | {len(pk)} |")
        all_pk = {pkg for r in counted for pkg in r.get("packages", [])}
        native_pk = {pkg for r in counted if r["bucket"] == "native" for pkg in r.get("packages", [])}
        p("")
        p(f"Distinct `name@version` packages across the counted entries: {len(all_pk)}; "
          f"laid out natively at least once: {len(native_pk)} "
          f"({100 * len(native_pk) / len(all_pk):.0f} %)." if all_pk else "")
        p("")
        # Reasons
        # One count per entry and reason (a Mautic lock lists forty themes);
        # the packages without a dist are one row, with their names.
        reasons = collections.Counter()
        nodist_packages = collections.Counter()
        for r in mrows:
            if r["bucket"] == "fallback":
                keys = {reason_key(reason) for reason in r.get("reasons", [])}
                nodist = {what for kind, what in keys if kind == "no-dist"}
                for pkg in nodist:
                    nodist_packages[pkg] += 1
                keys = {k for k in keys if k[0] != "no-dist"}
                if nodist:
                    keys.add(("no-dist", "(packages without a zip dist)"))
                for key in keys:
                    reasons[key] += 1
        if reasons:
            p("### Fallback reasons, ranked")
            p("")
            p("| reason | entries | note |")
            p("|---|---:|---|")
            for (kind, what), n in sorted(reasons.items(), key=lambda kv: (-kv[1], kv[0])):
                note = ""
                if kind in ("plugin", "layout-plugin") and what in PLUGIN_EVENTS:
                    note = f"listens to {PLUGIN_EVENTS[what]} (active in the baseline)"
                elif kind == "no-dist":
                    top = ", ".join(f"`{n}` ({c})" for n, c in nodist_packages.most_common(6))
                    note = f"a `git`/`vcs`-only source or asset-packagist: {top}…"
                p(f"| {kind} `{what}` | {n} | {note} |")
            p("")
        diffs = [r for r in mrows if r["bucket"] == "diff"]
        if diffs:
            p("### Diffs")
            p("")
            p("A diff is a parity gap. A file written by a plugin vivacity installs as a "
              "plain library (`BENIGN_PLUGINS`, qualified against `--no-plugins`) is "
              "counted here too: emulating it, or handing the project to Composer, "
              "is the decision the report asks for.")
            p("")
            for r in diffs:
                p(f"- **{r['name']}** — `{r.get('provenance', '')}`")
                p("  ```")
                for line in (r.get("detail") or "").splitlines()[:12]:
                    p(f"  {line}")
                p("  ```")
            p("")
        unavailable = [r for r in mrows if r["bucket"] == "unavailable"]
        if unavailable:
            p("### Unavailable (Composer failed)")
            p("")
            for r in unavailable:
                first = (r.get("detail") or "").strip().splitlines()
                p(f"- **{r['name']}** — {first[-1] if first else ''}")
            p("")
        # Per-entry table
        p("### Entries")
        p("")
        p("| entry | bucket | packages | composer s | vivacity s | ignored | detail |")
        p("|---|---|---:|---:|---:|---|---|")
        for r in mrows:
            detail = ""
            if r["bucket"] == "fallback":
                detail = "; ".join(r.get("reasons", []))[:160]
            elif r["bucket"] in ("diff", "unavailable"):
                lines = (r.get("detail") or "").strip().splitlines()
                detail = (lines[0] if lines else "")[:160]
            ignored = r.get("ignored", "").replace("--ignore-platform-req=", "")
            p(f"| {r['name']} | {r['bucket']} | {len(r.get('packages', []))} | "
              f"{r.get('seconds_composer', '')} | {r.get('seconds_vivacity', '')} | {ignored} | {detail} |")
        p("")
    # Scripts and platform, once (mode-independent)
    dev_rows = [r for r in rows if r["mode"] == modes[0]]
    with_scripts = [r for r in dev_rows if r.get("scripts")]
    p("## Scripts declared (the next cadrage)")
    p("")
    p(f"{len(with_scripts)} of {len(dev_rows)} entries declare `scripts`. Events, ranked:")
    p("")
    ev = collections.Counter(e for r in with_scripts for e in r["scripts"])
    for e, n in sorted(ev.items(), key=lambda kv: (-kv[1], kv[0])):
        p(f"- `{e}`: {n}")
    p("")
    p("## Platform requirements this machine failed")
    p("")
    pf = collections.Counter()
    for r in dev_rows:
        for f in r.get("platform_failures", []):
            pf[re.sub(r" \(required by [^)]*\)", "", f)] += 1
    if pf:
        for f, n in sorted(pf.items(), key=lambda kv: (-kv[1], kv[0])):
            p(f"- `{f}`: {n}")
    else:
        p("None.")
    p("")
    print("\n".join(out))


if __name__ == "__main__":
    main()
