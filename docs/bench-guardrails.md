# Benchmark CI Guardrails

CI runs `cargo bench --bench system_bench` on every PR targeting `main` or `dev`.
A regression >20% on any baseline-tracked benchmark causes a CI failure.

## What Is Checked

`rust/benches/ci-baseline.json` stores mean nanosecond times for every
benchmark variant in `system_bench.rs`. The CI comparison script
(`rust/benches/check_regression.py`) walks `target/criterion/` after the
bench run and compares each result against its baseline entry.

Benchmarks with a baseline `< 500ns` are skipped (avoids noise on O(1)
paths like `npc_regen` where bucket prediction dominates variance).

## Updating the Baseline

Run after an intentional performance improvement (or when CI machine variance
causes false positives on a stable run):

```sh
cd rust
cargo bench --bench system_bench
python3 benches/update_baseline.py
```

`update_baseline.py` reads `target/criterion/` and overwrites
`benches/ci-baseline.json` with the new mean times. Commit the updated file
with a message noting the commit and date, e.g.:

```
bench: update ci-baseline to 2026-03-20 (abc1234)
```

If `update_baseline.py` does not exist yet, update `ci-baseline.json` manually:
replace `mean_ns` values with the new Criterion mean (in nanoseconds) from
`target/criterion/<group>/<id>/new/estimates.json` -> `mean.point_estimate * 1e9`.

## Threshold Tuning

The default threshold is 20%. To adjust it:

- **Per-repo**: set a GitHub Actions variable `BENCH_THRESHOLD` to a number
  (e.g., `25`) in Settings > Secrets and variables > Actions > Variables.
- **Temporary warn-only**: set `BENCH_WARN_ONLY=1` as a repo variable to
  emit warnings without failing CI. Remove after the flaky window passes.

## Skipping the Benchmark Job

Add the `skip-bench` label to the PR or include `[skip-bench]` in a commit
message to skip the bench job entirely. Use this for doc-only PRs or when
the benchmark run is not relevant to the change.

## Reading CI Results

The bench job posts a step summary table to the GitHub Actions run with three
sections:
- **Regressions**: benchmarks >threshold% slower than baseline
- **Improvements**: benchmarks >10% faster than baseline (informational)
- **Passing**: everything else

Criterion HTML reports are uploaded as the `criterion-results-<sha>` artifact
(retained 30 days). Download and open `index.html` for violin plots and
detailed statistical analysis.

## Adding New Benchmarks to the Baseline

After writing a new benchmark in `system_bench.rs`:
1. Run `cargo bench --bench system_bench -- <new_group_name>` locally.
2. Add entries to `ci-baseline.json` for each variant (group/count).
3. Commit both the bench code and baseline together.

Missing baseline entries are silently skipped (no false positives for new
benchmarks before their first baseline run).

## False Positive Runbook

CI machines have variable load. If a green PR starts failing with a
borderline regression (~21-25%):

1. Rerun the bench job once from the GitHub Actions UI.
2. If it passes, the first run was a noisy outlier.
3. If it consistently fails at 21-25%, consider raising `BENCH_THRESHOLD`
   to 25% via the repo variable, or investigate the actual change.
4. If the regression is real and intentional, update the baseline (see above).
