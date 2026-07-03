# Tasks: optimize-decode-first-frame-latency

## 1. Measurement

- [x] 1.1 Capture repository state, toolchain versions, and clean baseline release build.
- [x] 1.2 Run one warm-up plus five measured baseline runs for raw/hash and default/`--threads 1`.
- [x] 1.3 Capture a local sampling profile for the measured runtime hotspot.
- [x] 1.4 Capture `--threads 1/2/4/8/10/auto` raw-output determinism and
      timing-disabled scaling after worker-attribution tracing.

## 2. Hot-Path Implementation

- [x] 2.1 Add `INFRA-DECODE-FIRST-FRAME-LATENCY` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 2.2 Reduce measured Wiener NS luma hot-path allocation and collection overhead while preserving fail-atomic output.
- [x] 2.3 Re-measure the four benchmark variants and inspect whether PC-Wiener/CDEF remain dominant.
- [x] 2.4 If the first slice is insufficient, implement the next measured non-invasive hot-path cleanup.
  - Note: cell-grid subclass and discardable-output variants were measured and reverted because they were neutral or slower.
- [x] 2.5 Extend `SPLOT_DECODE_TIMING` with bounded-source/runtime parse
      timing plus pool-scoped worker attribution for existing deblock, CDEF,
      CCSO, and Wiener NS LR parallel stages.
- [x] 2.6 Break down and reduce the remaining serial `ac0ej3_region_reconstruct`
      bottleneck so `--threads 8/10` scaling improves beyond the current
      ~1.8x total raw speedup, or document the exact unavoidable dependency.
  - Note: timing now attributes the fixed region to the selectable transform
    record handoff (`1849` blocks, `2160` luma records, `544` chroma groups)
    and its serial coefficient/sink reconstruction dependencies. The safe
    cleanup keeps raw SHA identical and improves clean timing-disabled raw
    medians to `--threads 1` 293.973 ms, `2` 212.678 ms, `4` 165.522 ms, `8`
    146.067 ms, `10` 145.348 ms; hash medians are `--threads 1` 289.626 ms
    and `--threads 10` 149.012 ms. Linear speedup is still impossible in this
    shape because the transform/sink walk is decode-order and current-frame
    dependent, and atomic output publication is outside the worker pool.

## 3. Verification

- [x] 3.1 Add or update focused bit-exact/fail-atomic tests for touched filter helpers.
- [x] 3.2 Verify raw sha256 and hash output are unchanged before and after.
- [x] 3.3 Run focused crate tests for touched modules.
- [x] 3.4 Run `cargo fmt --all -- --check`.
- [x] 3.5 Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- [x] 3.6 Run `cargo test --workspace --all-targets --locked`.
- [x] 3.7 Run `cargo xtask feature-status` and `cargo xtask check-feature-status`.
- [x] 3.8 Run `cargo xtask ci`.
- [x] 3.9 Run focused `splot-parallel`, `splot-recon`, and `splot-decode`
      tests after adding worker-attribution tracing.
- [x] 3.10 Confirm raw SHA-256 is identical across `--threads
      1/2/4/8/10/auto` for the local `ac0ej3.ivf` fixture.
