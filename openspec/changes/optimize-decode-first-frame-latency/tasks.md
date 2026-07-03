# Tasks: optimize-decode-first-frame-latency

## 1. Measurement

- [x] 1.1 Capture repository state, toolchain versions, and clean baseline release build.
- [x] 1.2 Run one warm-up plus five measured baseline runs for raw/hash and default/`--threads 1`.
- [x] 1.3 Capture a local sampling profile for the measured runtime hotspot.

## 2. Hot-Path Implementation

- [x] 2.1 Add `INFRA-DECODE-FIRST-FRAME-LATENCY` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 2.2 Reduce measured Wiener NS luma hot-path allocation and collection overhead while preserving fail-atomic output.
- [x] 2.3 Re-measure the four benchmark variants and inspect whether PC-Wiener/CDEF remain dominant.
- [x] 2.4 If the first slice is insufficient, implement the next measured non-invasive hot-path cleanup.
  - Note: cell-grid subclass and discardable-output variants were measured and reverted because they were neutral or slower.

## 3. Verification

- [x] 3.1 Add or update focused bit-exact/fail-atomic tests for touched filter helpers.
- [x] 3.2 Verify raw sha256 and hash output are unchanged before and after.
- [x] 3.3 Run focused crate tests for touched modules.
- [x] 3.4 Run `cargo fmt --all -- --check`.
- [x] 3.5 Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- [x] 3.6 Run `cargo test --workspace --all-targets --locked`.
- [x] 3.7 Run `cargo xtask feature-status` and `cargo xtask check-feature-status`.
- [x] 3.8 Run `cargo xtask ci`.
