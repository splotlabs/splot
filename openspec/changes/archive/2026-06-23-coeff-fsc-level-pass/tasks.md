## 1. FSC Level Pass

- [x] 1.1 Add `NonZeroCoeffFscLevelPass` and `apply_nonzero_coeff_fsc_level_pass`.
- [x] 1.2 Add focused tests for BaseBob first-entry selection, BaseIdtx
  current-level context derivation, conditional BrIdtx consumption, tx-size
  clamping, and static preflight no-consumption behavior.

## 2. Tracking And Verification

- [x] 2.1 Update `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and
  `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-FSC-LEVEL-PASS`.
- [x] 2.2 Regenerate generated status documents and validate OpenSpec, feature
  status, decoder support, decoder conformance coverage, focused tests, and CI.
