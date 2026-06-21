## Why

The first **multi-coefficient** (`eob > 1`) frame. Every prior coded frame had a single DC
coefficient per plane; this emits an eob=2 luma block — one nonzero AC coefficient plus a zero
DC — exercising the scan walk and the non-EOB `coeff_base` with its `Level[]`-derived § 8.3.2
low-frequency context at the general `TX_64X64` transform.

## What Changes

- Add `ENC-GENERAL-INTRA-TWO-COEFF` as an encoder feature (splot-encode + splot-cli oracle).
- Add `general_intra_64x64_luma_two_coeff_tokens`: the eob=2 luma tokens (`txb_skip=0`,
  `eob_pt_1024=1`, AC `coeff_base_eob` at ctx 1, DC `coeff_base` at the `Level[]`-derived ctx 1)
  at the general `TX_64X64` contexts. The 32x32 (TX_64X64-coded) scan maps scan index 1 to
  raster row 1 col 0, the same DC-neighbour relation as the 4x4 tier, so the contexts match.
- Add two `BlockSymbolTraceCdfRows` rows (the AC `coeff_base_eob` and DC `coeff_base` at
  `tx_size 4`).
- Add `compose_general_intra_two_coeff_block_trace` (do_split + modes + the eob=2 luma + AC
  `sign_bit` bypass + skipped U/V) and `emit_minimal_intra_two_coeff_ivf()`.
- Add the cross-crate oracle: `splot decode` validates the eob=2 entropy stream (§ 8.2.4
  `exit_symbol`) and reconstructs the frame. The level-1 AC residual is sub-visible (flat 128);
  a visibly-non-flat AC needs the per-AC-level DC `coeff_base` context, a follow-up.

## Capabilities

### Modified Capabilities

- `encoder-tools`: add a requirement for the first multi-coefficient (eob>1) intra frame.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_coded.rs`,
  `crates/splot-encode/src/block_symbol_trace.rs` (two rows + routing),
  `crates/splot-encode/src/general_intra_trace.rs`, `crates/splot-encode/src/lib.rs`,
  `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
