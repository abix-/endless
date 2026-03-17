#!/usr/bin/env python3
"""
CI regression checker for Criterion benchmarks.

Walks target/criterion/ for estimates.json files, compares mean time
against ci-baseline.json, and exits non-zero if any benchmark regressed
by more than BENCH_THRESHOLD percent (default: 20).

Usage:
  python3 benches/check_regression.py [--threshold 20] [--warn-only]

Set BENCH_THRESHOLD env var to override the threshold without editing the
workflow. Set BENCH_WARN_ONLY=1 to emit warnings instead of failing.

Exit codes:
  0  all benchmarks within threshold (or no matching baseline entries)
  1  one or more regressions detected (unless --warn-only)
"""

import argparse
import json
import os
import sys
from pathlib import Path


def load_baseline(baseline_path: Path) -> dict:
    with open(baseline_path) as f:
        data = json.load(f)
    return {k: v for k, v in data.items() if not k.startswith("_")}


def collect_results(criterion_dir: Path) -> dict:
    """Walk criterion output dir, return {group/id: mean_ns}."""
    results = {}
    if not criterion_dir.exists():
        return results
    for estimates in criterion_dir.rglob("new/estimates.json"):
        # path: .../criterion/<group>/<id>/new/estimates.json
        parts = estimates.parts
        try:
            new_idx = parts.index("new")
            bench_id = parts[new_idx - 1]
            group = parts[new_idx - 2]
        except (ValueError, IndexError):
            continue
        key = f"{group}/{bench_id}"
        try:
            with open(estimates) as f:
                est = json.load(f)
            mean_s = est["mean"]["point_estimate"]
            results[key] = mean_s * 1e9  # convert to nanoseconds
        except (KeyError, json.JSONDecodeError):
            pass
    return results


def format_ns(ns: float) -> str:
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.1f}ms"
    if ns >= 1_000:
        return f"{ns / 1_000:.0f}us"
    return f"{ns:.0f}ns"


def main() -> int:
    parser = argparse.ArgumentParser(description="Check Criterion benchmark regressions")
    parser.add_argument("--threshold", type=float, default=None,
                        help="Regression threshold percent (default: 20)")
    parser.add_argument("--warn-only", action="store_true",
                        help="Emit warnings but do not fail")
    args = parser.parse_args()

    threshold = args.threshold
    if threshold is None:
        threshold = float(os.environ.get("BENCH_THRESHOLD", "20"))

    warn_only = args.warn_only or os.environ.get("BENCH_WARN_ONLY", "") == "1"

    script_dir = Path(__file__).parent
    repo_root = script_dir.parent  # rust/
    baseline_path = script_dir / "ci-baseline.json"
    criterion_dir = repo_root / "target" / "criterion"

    if not baseline_path.exists():
        print(f"ERROR: baseline not found at {baseline_path}", file=sys.stderr)
        return 1

    baseline = load_baseline(baseline_path)
    results = collect_results(criterion_dir)

    if not results:
        print(f"WARNING: no Criterion output found in {criterion_dir}")
        print("  Run: cargo bench --bench system_bench")
        return 0

    regressions = []
    improvements = []
    checked = []

    for key, base_ns in sorted(baseline.items()):
        if key not in results:
            continue
        curr_ns = results[key]
        # avoid divide-by-zero for near-zero baselines (npc_regen O(1) buckets)
        if base_ns < 500:
            continue
        pct = (curr_ns - base_ns) / base_ns * 100.0
        checked.append((key, base_ns, curr_ns, pct))
        if pct > threshold:
            regressions.append((key, base_ns, curr_ns, pct))
        elif pct < -10:
            improvements.append((key, base_ns, curr_ns, pct))

    # --- GitHub step summary output ---
    summary_file = os.environ.get("GITHUB_STEP_SUMMARY")
    lines = ["## Benchmark Regression Report\n"]
    lines.append(f"Threshold: {threshold:.0f}%  |  Benchmarks checked: {len(checked)}\n")

    if regressions:
        lines.append(f"\n### Regressions ({len(regressions)})\n")
        lines.append("| Benchmark | Baseline | Current | Delta |\n")
        lines.append("|-----------|----------|---------|-------|\n")
        for key, base, curr, pct in regressions:
            sign = "+" if pct > 0 else ""
            lines.append(f"| {key} | {format_ns(base)} | {format_ns(curr)} | {sign}{pct:.1f}% |\n")

    if improvements:
        lines.append(f"\n### Improvements ({len(improvements)})\n")
        lines.append("| Benchmark | Baseline | Current | Delta |\n")
        lines.append("|-----------|----------|---------|-------|\n")
        for key, base, curr, pct in improvements:
            lines.append(f"| {key} | {format_ns(base)} | {format_ns(curr)} | {pct:.1f}% |\n")

    passing = [r for r in checked if r not in regressions and r not in improvements]
    if passing:
        lines.append(f"\n### Passing ({len(passing)})\n")
        lines.append("| Benchmark | Baseline | Current | Delta |\n")
        lines.append("|-----------|----------|---------|-------|\n")
        for key, base, curr, pct in passing:
            sign = "+" if pct > 0 else ""
            lines.append(f"| {key} | {format_ns(base)} | {format_ns(curr)} | {sign}{pct:.1f}% |\n")

    summary = "".join(lines)
    if summary_file:
        with open(summary_file, "a") as f:
            f.write(summary)
    print(summary)

    if regressions:
        print(f"\nFAIL: {len(regressions)} benchmark(s) regressed by >{threshold:.0f}%")
        for key, base, curr, pct in regressions:
            print(f"  {key}: {format_ns(base)} -> {format_ns(curr)} (+{pct:.1f}%)")
        print()
        print("To update the baseline after an intentional perf change:")
        print("  See docs/bench-guardrails.md")
        if warn_only:
            print("(warn-only mode: not failing CI)")
            return 0
        return 1

    if not checked:
        print("No baseline entries matched current results -- skipping regression check")
        return 0

    print(f"OK: all {len(checked)} benchmarks within {threshold:.0f}% threshold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
