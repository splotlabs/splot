## 1. CDF Row Boundary

- [x] 1.1 Add `TileCoeffBaseBobCdf`, `TileCoeffBaseIdtxCdf`, `TileCoeffBrIdtxCdf`, and `TileIdtxSignCdf` storage, default loading, selector, immutable and mutable row access, bounds errors, copy isolation, average, and frame-end scaling to the coefficient CDF row bundle.
- [x] 1.2 Add focused CDF tests for generated default selection, invalid IDTX/FSC selector axes, tile-copy isolation, lifecycle averaging/scaling, and mutable symbol-reader handoff.

## 2. Tracking And Verification

- [x] 2.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `xtask/src/decoder_conformance_coverage.rs`, and `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-IDTX-CDF-ROWS`.
- [x] 2.2 Regenerate generated status documents and validate OpenSpec, feature status, decoder support, decoder conformance coverage, focused tests, and CI.
