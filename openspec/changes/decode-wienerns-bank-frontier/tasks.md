## 1. Parser Model

- [x] 1.1 Add a restoration Wiener NS submodule with the AV2 5.20.10.6
      frame-level fixed-coded parser helpers and spec-cited tap tables.
- [x] 1.2 Extend `LrParams` / `LrPlaneParams` so a completed
      `frame_filters_on == true` parse carries explicit frame-filter bank data.
- [x] 1.3 Wire `parse_lr_params()` to call the new frame-level parser for the
      intra `readFrameFilters == 1` path and reserve unsupported branches for
      later rows.

## 2. Runtime And Tests

- [x] 2.1 Update the minimal runtime local decoder mission diagnostic expectation so the live
      stream no longer reports `unsupported_wienerns_filter`.
- [x] 2.2 Add parser tests for the local decoder mission luma two-class bank shape, a
      non-frame-filter no-op case, and EOF/edge cases inside the bank parser.
- [x] 2.3 Update inspect/runtime tests that depend on
      `StoppedBeforeWienerNsFilter` so non-Wiener coverage stops still behave as
      before.

## 3. Tracking And Validation

- [x] 3.1 Add implementation-matrix and decoder-support rows for
      `DECODE-WIENERNS-BANK-FRONTIER`, and update adjacent local decoder mission notes.
- [x] 3.2 Regenerate generated status/coverage docs and mark proofs in the
      matrix/support rows.
- [x] 3.3 Run OpenSpec validation, feature/decoder-support checks, focused
      parser/runtime tests, conformance, fixture checks, deletion checks, and
      `cargo xtask ci`.
