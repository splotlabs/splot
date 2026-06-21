## Why

Sub-brick 5b-ii. With the EOB-coefficient `coeff_br` landed (5b-i), the non-EOB
coefficient (the DC at scan index 0 when eob==2) still capped at magnitude 4. This
adds its `coeff_br`, so both coefficients of an eob<=2 LF block span 1..=7.

Unlike the EOB coefficient (whose `coeff_br` context is a constant because its
running `Level[]` is empty), the non-EOB DC's `coeff_br` context is data-dependent —
derived from the already-written EOB AC neighbour. The decoder's
`Mag_Ref_Offset_With_Tx_Class[class]` is exactly the first 3 entries of
`splot_core::tables::conversion::SIG_REF_DIFF_OFFSET[class]` (verified for all three
classes), which the encoder already imports — so no hand-coded offset table and no
splot-decode dependency are needed.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-DC-BR` as a private `splot-encode` encoder-tool feature.
- Add `coeff_br_lf_luma_context(pos, bwl, txw, txh, tx_class, level)` mirroring the
  decoder `CoeffBrContext::ctx`: the first-`num` `SIG_REF_DIFF_OFFSET` neighbour sum
  (each clamped to `MAX_BASE_BR_RANGE - 1 = 5`), `mag = Min((sum+1)>>1, 6)`, then the
  2D-LF-luma mapping (DC -> `mag`, non-DC LF -> `mag + 7`).
- Emit the non-EOB DC `coeff_br` (interleaved, before the `Level[]` write) when its
  magnitude exceeds the base tier, with the helper-derived context and symbol
  `mag - 5`. Lift the non-EOB magnitude limit from 4 to 7. Extend
  `recover_quant_from_tokens` to read the interleaved `coeff_br` for every base-pass
  coefficient.
- Add the routed `CoeffBrLf` ctx-1 and ctx-3 CDF rows (from the generated splot-core
  table). Keep the TCQ-off precondition (magnitude <= 7).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: extend the general low-frequency coefficient walk so the non-EOB
  coefficient carries the data-dependent `coeff_br` base-range tier (magnitude 1..=7).

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/{general_walk.rs,
  coeff_base_lf.rs, cdf_rows.rs}` (+ tests), `coefficient_tokenization.rs`,
  `block_symbol_trace/{cdf_rows,mod}.rs`.
- Scope (explicitly NOT claimed): magnitudes beyond 7 (golomb), eob > 2 / eob_extra,
  high-frequency or chroma coefficients, sizes other than 4x4, types other than
  DCT_DCT, packets, decoder context conformance (the §8.2 roundtrip proves
  self-consistency only). The helper's 1-D-class `pos==0` branch mirrors the decoder
  but is exercised only when the high-frequency / non-2D sub-bricks land.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status
  / spec coverage.
