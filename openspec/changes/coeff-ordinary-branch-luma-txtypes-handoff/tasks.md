## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for the luma `TxTypes` handoff.
- [x] 1.3 Update `docs/DECODER-ROADMAP.md` and regenerate generated status/coverage docs.

## 2. Implementation

- [x] 2.1 Extend the ordinary branch transform-type config chain with caller-resolved luma `TxTypes`.
- [x] 2.2 Implement the AV2 section 5.20.7.29 non-lossless luma `TxTypes` path before chroma-specific logic.
- [x] 2.3 Preserve existing all-zero, chroma mapping, chroma-DCT-only, and chroma unsupported-subset behavior.

## 3. Tests and Validation

- [x] 3.1 Add focused ordinary branch tests for luma `TxTypes`, chroma-only fallback bypass, and fail-atomic invalid luma domains.
- [x] 3.2 Run focused coefficient-loop tests and OpenSpec/tracking checks.
- [x] 3.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
