## 1. Coded chroma DC tokenization

- [x] 1.1 Add a `CoeffBaseLfEobUv` (`TileCoeffBaseLfEobUvCdf`) CDF-row selector to `coefficient_tokenization`.
- [x] 1.2 Add `pub(crate)` `chroma_u_dc_coded_tokens(coeff_cdf_q_ctx, magnitude, negative)` returning the four ordered coded chroma U DC tokens (base tier).
- [x] 1.3 Cite AV2 §5.20.7.27 (chroma `residual()`, `eobCtx = (plane>0)?2:is_inter`) and §8.3.2 (chroma base-eob CDF, `dc_sign` ptype 1); verify the contexts against the decoder's `base_eob_selector`/`dc_sign` derivation.

## 2. Coded chroma block trace and tests

- [x] 2.1 Extend `block_symbol_trace` with `compose_minimal_intra_dc_coded_chroma_block_trace` (coded luma + coded U + all-zero V) and route the chroma `eob_pt_16`/`coeff_base_lf_eob_uv`/chroma `dc_sign` CDF rows.
- [x] 2.2 Prove the trace is the mode prefix, coded luma residual, coded U residual, then all-zero V `txb_skip`, in order.
- [x] 2.3 Prove the twelve-symbol sequence roundtrips deterministically through one §8.2 coder with shared CDF state.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming chroma base-range/golomb, multi-coefficient blocks, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
