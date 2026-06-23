## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for the chroma-inter `TxTypes` handoff.
- [x] 1.3 Update `docs/DECODER-ROADMAP.md` and regenerate generated status/coverage docs.

## 2. Implementation

- [x] 2.1 Extend the ordinary branch transform-type config chain with caller-resolved chroma-inter `TxTypes`.
- [x] 2.2 Add the AV2 section 5.20.7.29 `Tx_Type_In_Set_Inter` membership table and non-lossless chroma-inter branch.
- [x] 2.3 Preserve existing all-zero, luma, chroma-DCT-only, chroma intra, directional UV, and lossless behavior.

## 3. Tests and Validation

- [x] 3.1 Add focused ordinary branch tests for chroma-inter `TxTypes` membership, fallback, and fail-atomic invalid domains.
- [x] 3.2 Run focused coefficient-loop tests and OpenSpec/tracking checks.
- [x] 3.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
