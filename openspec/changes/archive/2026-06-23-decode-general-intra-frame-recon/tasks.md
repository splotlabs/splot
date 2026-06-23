## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-FRAME-RECON` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-frame-recon`.

## 2. Implementation

- [x] 2.1 Add `decode_general_intra_chroma_coeffs` reading the U/V `all_zero` symbol with the §8 parsing CDF (plane 0/1 vs V) and routing the nonzero pass to produce the chroma `Quant[]`.
- [x] 2.2 Generalize the luma reconstruction into `reconstruct_general_intra_block` composing §7.14.4 dequant (with the TCQ `dqDenom` term), §7.15.4 inverse transform, and §7.14.3 residual add over the §7.13.2 DC prediction.
- [x] 2.3 Validate §8.2.4 `exit_symbol()`, assemble the frame, and wire the full decode into `decode_general_minimal_intra_frame`.
- [x] 2.4 Replace the "reaches chroma" CLI test with the bit-exact full-frame decode test and add reconstructed-plane / frame-hash tests.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
