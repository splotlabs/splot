## 1. Scan Boundary

- [x] 1.1 Add `FscCoeffScanWalk` and `walk_fsc_coeff_scan` over caller-resolved `segEob` and `scan[c]`.
- [x] 1.2 Add focused tests for forward `bob..segEob` order, `eob > segEob`, scan-length bounds, and position bounds without mutation.

## 2. Tracking And Verification

- [x] 2.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `xtask/src/decoder_conformance_coverage.rs`, and `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-FSC-SCAN-WALK`.
- [x] 2.2 Regenerate generated status documents and validate OpenSpec, feature status, decoder support, decoder conformance coverage, focused tests, and CI.
