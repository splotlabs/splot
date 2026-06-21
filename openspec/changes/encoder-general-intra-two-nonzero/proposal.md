## Why

The encoder's first block with **two nonzero coefficients**. Every prior coded block had exactly
one nonzero coefficient (a single DC, or a single AC). This codes a level-4 AC at scan index 1
**and** a level-1 DC at scan index 0 in one eob=2 block, exercising the non-EOB `coeff_base`
nonzero path and the sign pass with two signs in scan order.

## What Changes

- Add `ENC-GENERAL-INTRA-TWO-NONZERO` (splot-encode + splot-cli oracle).
- Add `general_intra_64x64_luma_two_nonzero_tokens`: the base pass (`txb_skip=0`,
  `eob_pt_1024=1`, AC `coeff_base_eob` symbol 3 at ctx 1, DC `coeff_base` symbol 1 at the
  AC-level-derived ctx 2) then the DC `dc_sign` (CDF); the caller appends the AC `sign_bit` bypass.
- Add `compose_general_intra_two_nonzero_block_trace` and `emit_minimal_intra_two_nonzero_ivf()`.
  No new CDF rows (reuses the visible-AC ctx-2 DC row and the shared `dc_sign` row).
- Add the cross-crate oracle: `splot decode` reconstructs the visible-AC cosine superimposed on a
  DC offset (each row constant; top 14 rows 129, the rest 128), with flat 128 chroma.

## Capabilities

### Modified Capabilities

- `encoder-tools`: add a requirement for the first two-nonzero-coefficient block.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_coded.rs`,
  `crates/splot-encode/src/coefficient_tokenization.rs` (re-export),
  `crates/splot-encode/src/general_intra_trace.rs`, `crates/splot-encode/src/lib.rs`,
  `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
