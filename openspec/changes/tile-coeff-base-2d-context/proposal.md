## Why

`coeff_base` is the main AV2 § 8.3.2 significant-coefficient CDF context and the
last of the `coeff_base` family (after eob/bob, br, and the IDTX variants). It is
the most intricate — five candidate banks selected from a neighbour-magnitude
sum — and is verifiable now against the spec over a caller-provided level slice.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the `coeff_base` context.
- In `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` add
  `CoeffBaseContext` (`pos`, `bwl`, `txw`, `txh`, `plane`, `is_lf`, `is_hidden`,
  `c`, `tx_class`) with `select(&[u32]) -> CoeffBaseSelection` implementing
  § 8.3.2 `coeff_base`:
  - sum the significant-neighbour `Level[]` magnitudes at the generated
    `Sig_Ref_Diff_Offset[txClass]` offsets (`num` = 5 luma, 3 chroma-2D, 2
    chroma-non-2D), each clamped by the position-dependent `magLimit` (5 for
    low-frequency near-DC samples unless parity-hidden DC, else 3);
  - `ctx = (mag + 1) >> 1`;
  - select one of five banks (`CoeffBaseSelection`): `Ph` (parity-hidden DC,
    `Min(ctx,4)`, overriding the rest), chroma `Uv` / `LfUv`, luma low-frequency
    `Lf`, luma high-frequency `Hf` — each with its bank-specific context offset.
- Use the generated `splot_core::tables::conversion::SIG_REF_DIFF_OFFSET` (no
  hand-written duplicate); add the `SIG_REF_DIFF_OFFSET_NUM`,
  `LF_SIG_COEF_CONTEXTS_2D`, and `LF_SIG_COEF_CONTEXTS_2D_UV` constants.

`select` reads a caller-provided row-major `Level[]` slice with checked shifts,
saturating flat-index geometry, and a slice-length guard, so out-of-range or
short-slice reads contribute `0` and it is total and panic-free. The `txClass` is
a caller-resolved scalar index (no `splot-recon` import).

Non-goals:

- No `coeffs()` decode-loop wiring (the derivation is not read by any decode
  path), so the minimal-fixture decode output is unchanged.
- No sign contexts (`dc_sign` ctx, `idtx_sign`), and no full per-transform-block
  level/sign buffers.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `coeff_base` significant-coefficient CDF context
  (the full five-bank selection) in the tile CDF selection subset, while broader
  § 8.3 coefficient CDF selection and the coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
