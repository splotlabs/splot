## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-LUMA-COEFFS` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-luma-coeffs`.

## 2. Implementation

- [x] 2.1 Add `decode_general_intra_luma_coeffs` reading the `all_zero` symbol with the §8.3.2 context and routing the nonzero pass to produce the luma `Quant[]`.
- [x] 2.2 Wire it into `decode_general_minimal_intra_frame` after mode decode, reporting the chroma step as unsupported.
- [x] 2.3 Add unit tests for the `txb_skip` transform-size context and update the CLI test for the new chroma diagnostic.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
