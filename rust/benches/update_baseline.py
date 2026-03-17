#!/usr/bin/env python3
"""
Update ci-baseline.json from the latest Criterion run.

Run after `cargo bench --bench system_bench` to snapshot new baselines.
Only updates entries that already exist in ci-baseline.json (does not add
new benchmarks automatically -- add them manually first).

Usage:
  cd rust
  cargo bench --bench system_bench
  python3 benches/update_baseline.py
"""

import json
import sys
from pathlib import Path


def collect_results(criterion_dir: Path) -> dict:
    results = {}
    if not criterion_dir.exists():
        return results
    for estimates in criterion_dir.rglob("new/estimates.json"):
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
            results[key] = round(mean_s * 1e9)
        except (KeyError, json.JSONDecodeError):
            pass
    return results


def main() -> int:
    script_dir = Path(__file__).parent
    repo_root = script_dir.parent
    baseline_path = script_dir / "ci-baseline.json"
    criterion_dir = repo_root / "target" / "criterion"

    if not baseline_path.exists():
        print(f"ERROR: {baseline_path} not found", file=sys.stderr)
        return 1

    with open(baseline_path) as f:
        baseline = json.load(f)

    results = collect_results(criterion_dir)
    if not results:
        print(f"ERROR: no Criterion output in {criterion_dir}", file=sys.stderr)
        print("  Run: cargo bench --bench system_bench", file=sys.stderr)
        return 1

    updated = 0
    for key in list(baseline.keys()):
        if key.startswith("_"):
            continue
        if key in results:
            old_ns = baseline[key]["mean_ns"]
            new_ns = results[key]
            pct = (new_ns - old_ns) / max(old_ns, 1) * 100
            baseline[key]["mean_ns"] = new_ns
            sign = "+" if pct > 0 else ""
            print(f"  {key}: {old_ns} -> {new_ns} ns ({sign}{pct:.1f}%)")
            updated += 1
        else:
            print(f"  SKIP (no result): {key}")

    import datetime
    baseline["_updated"] = datetime.date.today().isoformat()

    with open(baseline_path, "w") as f:
        json.dump(baseline, f, indent=2)
        f.write("\n")

    print(f"\nUpdated {updated} entries in {baseline_path}")
    print("Commit the updated ci-baseline.json with the new date/commit reference.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
