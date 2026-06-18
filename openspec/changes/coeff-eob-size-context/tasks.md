## 1. OpenSpec and Feature Tracking

- [x] 1.1 Validate the `coeff-eob-size-context` OpenSpec artifacts.
- [x] 1.2 Add `DECODE-COEFF-EOB-SIZE-CONTEXT` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Add the corresponding decoder support and conformance-coverage rows.

## 2. Decode Helper

- [x] 2.1 Add crate-private input and helper logic in `tile_payload/coeff_loop.rs` for deriving `EobPtSize`, `eobCtx`, and `NonZeroCoeffEobSymbolInput`.
- [x] 2.2 Return a typed coefficient-loop context error for invalid transform log2 dimensions.

## 3. Tests and Documentation

- [x] 3.1 Add focused unit tests for all EOB size classes, log2 clamping, invalid log2 rejection, luma/chroma `eobCtx`, and symbol-reader input composition.
- [x] 3.2 Update decoder roadmap/status notes to make the new derivation clear while preserving partial runtime support claims.
- [x] 3.3 Regenerate feature/status/support/coverage Markdown outputs.

## 4. Verification

- [x] 4.1 Run focused `splot-decode` coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, decoder support, decoder conformance coverage, and OpenSpec validation.
- [x] 4.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
