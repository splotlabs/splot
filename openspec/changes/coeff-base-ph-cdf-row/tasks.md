## 1. CDF Row Boundary

- [x] 1.1 Add `TileCoeffBasePhCdf` storage, default loading, selector, immutable and mutable row access, bounds errors, copy isolation, average, and frame-end scaling to the coefficient CDF row bundle.
- [x] 1.2 Add focused CDF tests for generated default selection, invalid Ph selector axes, tile-copy isolation, and mutable symbol-reader handoff.

## 2. Derived First-Pass Consumption

- [x] 2.1 Map `CoeffBaseSelection::Ph` to `CoeffCdfSelector::BasePh` in the state-derived base/level first-pass helper and remove the Ph-only unsupported error path.
- [x] 2.2 Add eob>=5 hidden-parity coverage proving the first pass reaches and consumes `BasePh` at the final DC coefficient.

## 3. Tracking And Verification

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `xtask/src/decoder_conformance_coverage.rs`, and `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-BASE-PH-CDF-ROW`.
- [x] 3.2 Regenerate generated status documents and validate OpenSpec, feature status, decoder support, decoder conformance coverage, and CI.
