## 1. Coefficient EOB Helper

- [x] 1.1 Add crate-private `DECODE-COEFF-EOB-VALUE-STATE` EOB value input/output types and typed errors in `coeff_loop.rs`.
- [x] 1.2 Implement checked AV2 § 5.20.7.27 nonzero `eob` arithmetic from caller-provided `eobPt`, `eob_extra`, and packed `eob_extra_bit` refinements.
- [x] 1.3 Add focused tests for small `eobPt`, `eob_extra` refinement, max AV2 EOB, invalid zero/oversized `eobPt`, and invalid refinement bits.

## 2. Tracking And Docs

- [x] 2.1 Add `DECODE-COEFF-EOB-VALUE-STATE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 2.2 Add `coeff-eob-value-state` to `docs/DECODER-SUPPORT-MATRIX.toml` and decoder conformance coverage grouping.
- [x] 2.3 Refresh generated feature/status/spec/decoder-support documentation.

## 3. Verification

- [x] 3.1 Run `openspec validate coeff-eob-value-state --strict`.
- [x] 3.2 Run focused `splot-decode` coefficient-loop tests.
- [x] 3.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-decoder-conformance-coverage`, and full `cargo xtask ci`.
