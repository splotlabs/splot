# Change: optimize-decode-first-frame-latency

## Feature IDs

- `INFRA-DECODE-FIRST-FRAME-LATENCY`

## Why

After `INFRA-DECODE-SERIAL-HOT-PATHS`, the motivating
`splot decode --quiet --output-format raw --limit=1` command is still well
above the 30 fps first-frame target. A warmed baseline on the local 1080p10 IVF
stream measures about 147 ms for raw default output and about 135 ms for hash
default output; attribution shows input read, context construction, planning,
and raw serialization are sub-2 ms each, while runtime decode is about 132 ms.

## What Changes

- Keep the opt-in `SPLOT_DECODE_TIMING` trace as the source of before/after
  attribution for first-frame decode latency.
- Optimize only measured first-frame runtime hot paths that preserve bit-exact
  decoded output, deterministic thread behavior, and the existing decode
  contract.
- Prefer allocation, copy, and loop-shape reductions in reconstruction/filter
  code before considering broader planner, input, or concurrency changes.
- Update implementation-matrix proof with the measured bottleneck, tests, and
  final timing results.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `runtime`: add first-frame decode latency constraints for measured,
  bit-exact hot-path optimization under the existing owned worker-pool and
  zero-copy policies.

## Impact

- Affected code: `crates/splot-decode`, `crates/splot-recon`, and targeted
  `crates/splot-cli` timing or output paths only if later measurements justify
  them.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml` and the
  `runtime` OpenSpec delta.
- No AV2 syntax, diagnostics, validator behavior, dependency graph, licensing,
  or `--threads` semantics change.
