#!/usr/bin/env python3
"""Performance gate on the vivacity/Composer ratio, from hyperfine JSON.

Usage:
    bench/gate.py <work-dir> --baseline bench/results/baseline-ratio.json [--tolerance 0.15]
    bench/gate.py <work-dir> --baseline bench/results/baseline-ratio.json --write-baseline
    bench/gate.py --merge <work-dir>... --baseline bench/results/baseline-ratio.json
    bench/gate.py --self-test

<work-dir> holds the `<fixture>.json` files `bench/ci-bench.sh` exports
(one hyperfine run per fixture, commands `composer-<scenario>` and
`vivace-<scenario>`).

Why a ratio: a GitHub runner's speed varies tens of percent from one run
to the next on identical code, so a baseline in seconds chases the runner,
not the code. `ratio = vivacity_median / composer_median` for the same
scenario, measured in the same job, on the same runner, minutes apart:
the runner's speed cancels out of the ratio — provided both sides scale
with it. A network-bound scenario does not (measured 2026-09-19: Composer
no-op 446–926 ms across identical runs, vivacity 104–140, ratio swinging
past any tolerance), which is why ci-bench.sh runs both tools with
`--no-blocking`: CPU-bound on both sides.

Fails (exit 1) when a scenario's ratio exceeds `baseline * (1 + tolerance)`
AND the regression costs at least 5 ms in this run's own seconds
(`vivacity_median - baseline * composer_median`): a 10 ms scenario swings
past any relative tolerance on a clock wobble that does not matter.
A scenario absent from the baseline, or whose Composer half is missing, is
reported and skipped, never failed.

`--write-baseline` records this run's ratios instead of comparing — one
run, which sits wherever the noise put it. `--merge` takes several work
directories (downloaded CI artifacts) and writes the per-scenario MEDIAN
across them, which is how the committed baseline should be produced.
"""

import argparse
import json
import statistics
import sys
from pathlib import Path

TOLERANCE_DEFAULT = 0.15
SLACK_SECONDS = 0.005
SCENARIOS = ("noop", "warm", "dump-o")
FIXTURES = ("laravel", "symfony", "sylius")


def medians(path):
    """{command: median seconds} of one hyperfine export."""
    data = json.loads(Path(path).read_text())
    return {r["command"]: float(r["median"]) for r in data["results"]}


def ratios_of(work):
    """{"<fixture>/<scenario>": (ratio, vivacity_median, composer_median)}."""
    out = {}
    for path in sorted(Path(work).glob("*.json")):
        fixture = path.stem
        med = medians(path)
        for sc in SCENARIOS:
            c = med.get(f"composer-{sc}")
            v = med.get(f"vivace-{sc}")
            if c is None or v is None or c <= 0:
                continue
            out[f"{fixture}/{sc}"] = (v / c, v, c)
    return out


def compare(current, baseline, tolerance):
    """Returns (rows, failed). rows: (key, ratio, base, verdict)."""
    rows, failed = [], False
    for key, (ratio, v, c) in sorted(current.items()):
        base = baseline.get(key)
        if base is None:
            rows.append((key, ratio, None, "no baseline"))
            continue
        cost = v - base * c
        if ratio > base * (1 + tolerance) and cost >= SLACK_SECONDS:
            rows.append((key, ratio, base, f"FAIL (+{cost * 1000:.0f} ms)"))
            failed = True
        elif ratio > base * (1 + tolerance):
            rows.append((key, ratio, base, f"ok (within {SLACK_SECONDS * 1000:.0f} ms slack)"))
        else:
            rows.append((key, ratio, base, "ok"))
    return rows, failed


def render(rows):
    lines = ["| scenario | ratio (vivacity / Composer) | baseline | verdict |", "|---|---|---|---|"]
    for key, ratio, base, verdict in rows:
        b = "—" if base is None else f"{base:.3f}"
        lines.append(f"| {key} | {ratio:.3f} | {b} | {verdict} |")
    return "\n".join(lines)


def self_test():
    base = {"fx/noop": 0.10, "fx/warm": 0.20}
    # 20 % worse on noop but 2 ms in seconds: slack, not a failure.
    cur = {"fx/noop": (0.12, 0.012, 0.100), "fx/warm": (0.20, 0.200, 1.000)}
    rows, failed = compare(cur, base, 0.15)
    assert not failed, rows
    # 20 % worse on warm and 40 ms in seconds: failure.
    cur = {"fx/noop": (0.10, 0.010, 0.100), "fx/warm": (0.24, 0.240, 1.000)}
    rows, failed = compare(cur, base, 0.15)
    assert failed and rows[1][3].startswith("FAIL"), rows
    # Unknown scenario: reported, not failed.
    cur = {"fx/dump-o": (9.0, 9.0, 1.0)}
    rows, failed = compare(cur, base, 0.15)
    assert not failed and rows[0][3] == "no baseline", rows
    # The full set the gate requires when a baseline exists.
    assert len([f"{fx}/{sc}" for fx in FIXTURES for sc in SCENARIOS]) == 9
    print("self-test ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("work", nargs="*", help="work directory with <fixture>.json hyperfine exports")
    ap.add_argument("--baseline", type=Path)
    ap.add_argument("--tolerance", type=float, default=TOLERANCE_DEFAULT)
    ap.add_argument("--write-baseline", action="store_true")
    ap.add_argument("--merge", action="store_true", help="write the median ratio across several work directories")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.work or args.baseline is None:
        ap.error("a work directory and --baseline are required")

    if args.merge:
        per_key = {}
        for work in args.work:
            for key, (ratio, _, _) in ratios_of(work).items():
                per_key.setdefault(key, []).append(ratio)
        merged = {k: statistics.median(v) for k, v in sorted(per_key.items())}
        args.baseline.write_text(json.dumps(merged, indent=2) + "\n")
        print(f"baseline written from {len(args.work)} runs: {args.baseline}")
        return 0

    current = ratios_of(args.work[0])
    if args.write_baseline:
        args.baseline.write_text(json.dumps({k: r for k, (r, _, _) in current.items()}, indent=2) + "\n")
        print(f"baseline written from one run: {args.baseline}")
        return 0
    if not args.baseline.is_file():
        print(f"::notice::no baseline at {args.baseline}, gate skipped")
        print(render([(k, r, None, "no baseline") for k, (r, _, _) in sorted(current.items())]))
        return 0
    baseline = json.loads(args.baseline.read_text())
    # Every fixture × scenario must be there: a bench that died half-way
    # (2026-09-19: an invalid flag on dump-autoload, hidden behind `tee`)
    # must not pass as "nothing regressed".
    expected = [f"{fx}/{sc}" for fx in FIXTURES for sc in SCENARIOS]
    missing = [k for k in expected if k not in current]
    if missing:
        print(f"::error::bench results missing for: {', '.join(missing)}")
        return 1
    rows, failed = compare(current, baseline, args.tolerance)
    table = render(rows)
    print(table)
    summary = Path(__import__("os").environ.get("GITHUB_STEP_SUMMARY", "/dev/null"))
    with summary.open("a") as f:
        f.write(f"\n## perf gate (tolerance {args.tolerance:.0%}, slack {SLACK_SECONDS * 1000:.0f} ms)\n\n{table}\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
