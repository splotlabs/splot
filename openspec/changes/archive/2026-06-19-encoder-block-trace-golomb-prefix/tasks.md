## 1. Golomb-prefix trace

- [x] 1.1 Add `compose_intra_dc_golomb_prefix_block_trace(magnitude, negative)` in `block_symbol_trace` for magnitude 18..=525: mode prefix + golomb level tokens + `dc_sign` CDF + `cMax` q_length zeros + `golomb_length` unary + `coeff_rem` `L(length)` + all-zero U/V, with `compose_minimal_intra_dc_golomb_prefix_block_trace` the canonical +18 case.
- [x] 1.2 Cite AV2 §5.20.7.28 (`read_quant` golomb-prefix) and §8.2.5 (`L(n)`); derive `length = GetMsb(x - 6)`, `golomb_zeros = length - k`, `coeff_rem`, `xBase` and verify vs the decoder's `read_quant`.
- [x] 1.3 Reject magnitudes outside 18..=525 at runtime via the typed `BlockSymbolTraceGolombMagnitudeOutOfRange` error.

## 2. Tests

- [x] 2.1 Prove the canonical +18 trace orders the mode prefix, level tokens, `dc_sign`, q_length zeros, golomb_length unary, `coeff_rem`, then all-zero U/V.
- [x] 2.2 Prove the trace roundtrips deterministically through one §8.2 coder.
- [x] 2.3 Prove the decoded golomb-prefix bits reconstruct the encoded magnitude via the decoder's `read_quant` golomb-prefix arithmetic, across the supported range (every magnitude 18..=525) and the length boundaries.
- [x] 2.4 Prove out-of-range magnitudes (17 and 526) are rejected and the boundaries (18, 525) accepted.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming multi-coefficient blocks, chroma golomb, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
