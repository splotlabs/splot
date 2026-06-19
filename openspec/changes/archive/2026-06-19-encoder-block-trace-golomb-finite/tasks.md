## 1. Golomb level tokens

- [x] 1.1 Factor out `luma_dc_sign_token` and add `luma_dc_golomb_level_tokens(coeff_cdf_q_ctx)` (fixed `all_zero=0`/`eob_pt_16=0`/`coeff_base_eob=LF_NUM_BASE_LEVELS`/`coeff_br=COEFF_BASE_RANGE`) in `coefficient_tokenization`.
- [x] 1.2 Cite AV2 §5.20.7.27 (`maxLevel`) and §5.20.7.28 (`read_quant` finite-q golomb); derive `m`/`k`/`cMax` from `hrLevelAvg = 0` and verify vs the decoder's `read_quant`.

## 2. Golomb block trace and tests

- [x] 2.1 Extend `block_symbol_trace` with a parameterized `compose_intra_dc_golomb_block_trace(magnitude, negative)` over the finite-q range 8..=17 (mode prefix + golomb level tokens + `dc_sign` CDF + finite-q `coeff_rem` bypass bits + all-zero U/V), with `compose_minimal_intra_dc_golomb_block_trace` the canonical +10 case.
- [x] 2.2 Prove the trace is the mode prefix, level tokens, `dc_sign`, then the golomb bypass bits, then all-zero U/V, in order (the sign precedes the §5.20.7.28 read_quant bits).
- [x] 2.3 Prove the trace roundtrips deterministically through one §8.2 coder.
- [x] 2.4 Prove the decoded golomb bits reconstruct the encoded magnitude via the decoder's `read_quant` finite-q arithmetic (the conformance check) for the canonical +10 case and for every magnitude across the finite-q range 8..=17.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming the golomb-prefix tier, multi-coefficient blocks, chroma golomb, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
