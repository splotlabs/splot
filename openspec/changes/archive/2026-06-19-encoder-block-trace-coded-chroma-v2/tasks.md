## 1. Coded chroma DC tokenization

- [x] 1.1 Add a `CoeffBaseLfEobUv` (`TileCoeffBaseLfEobUvCdf`) CDF-row selector to `coefficient_tokenization`.
- [x] 1.2 Add `pub(crate)` `chroma_u_dc_coded_coeff_tokens(coeff_cdf_q_ctx, magnitude)` returning the three coded chroma U DC CDF tokens, rejecting out-of-tier magnitude via a typed `CoefficientTokenizationUnsupportedChromaMagnitude` error.
- [x] 1.3 Cite AV2 §5.20.7.27 (chroma `residual()`, `eobCtx=(plane>0)?2:is_inter`, the `sign_bit`/`dc_sign`/`dc_sign_horz_vert` branches) and §8.3.2 (chroma base-eob CDF, V EobU context); verify vs the decoder.

## 2. Coded chroma block trace and tests

- [x] 2.1 Extend `block_symbol_trace` with `compose_minimal_intra_dc_coded_chroma_block_trace` (coded luma + coded U CDF + U `sign_bit` bypass + all-zero V at EobU ctx 6) and route the chroma CDF rows.
- [x] 2.2 Prove the trace is the mode prefix, coded luma residual, coded U CDF residual, the U `sign_bit` bypass literal, then the all-zero V `txb_skip`, in order.
- [x] 2.3 Prove the twelve-token sequence roundtrips deterministically through one §8.2 coder with shared CDF state.
- [x] 2.4 Add the chroma-context and magnitude-rejection tokenization tests.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming chroma base-range/golomb, V coded coefficients, multi-coefficient blocks, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
