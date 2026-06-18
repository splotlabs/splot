## 1. EOB Symbol Reader

- [x] 1.1 Add crate-private `DECODE-COEFF-EOB-SYMBOL-READ` input/output and typed error plumbing in `coeff_loop.rs`.
- [x] 1.2 Implement the AV2 § 5.20.7.27 EOB symbol-read sequence over caller-resolved `EobPtSize`, `coeff_cdf_q_ctx`, and `eob_ctx`.
- [x] 1.3 Add focused tests for EOB point CDF consumption, size-specific EOB-point extra literals, EOB-extra symbol/literal refinement handling, disabled CDF update behavior, and invalid selectors.

## 2. Tracking And Docs

- [x] 2.1 Add `DECODE-COEFF-EOB-SYMBOL-READ` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 2.2 Add `coeff-eob-symbol-read` to `docs/DECODER-SUPPORT-MATRIX.toml` and decoder conformance coverage grouping.
- [x] 2.3 Refresh generated feature/status/spec/decoder-support documentation and the decoder roadmap note.

## 3. Verification

- [x] 3.1 Run `openspec validate coeff-eob-symbol-read --strict`.
- [x] 3.2 Run focused `splot-decode` coefficient-loop and block-symbol CDF tests.
- [x] 3.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-decoder-conformance-coverage`, and full `cargo xtask ci`.
