## Why

After the position-only coefficient base contexts (`coeff_base_eob`,
`coeff_base_bob`), the next §8.3.2 coefficient context toward the §5.20.7.27
`coeffs()` decode loop is `coeff_br` — the coefficient base-range context, and the
first context that reads the per-transform-block `Level[]` magnitudes. It is
verifiable now against the spec over a caller-provided level slice.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the `coeff_br` context.
- In `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` add
  `CoeffBrContext` (`pos`, `bwl`, `txw`, `txh`, `plane`, `is_lf`, `tx_class`) with
  a `const fn ctx(&[u32]) -> usize` implementing §8.3.2 `coeff_br`:
  - derive `row`/`col` from `pos` and `bwl`;
  - sum up to three neighbour `Level[]` magnitudes at the
    `Mag_Ref_Offset_With_Tx_Class[txClass]` offsets (only the first two for
    non-2D chroma), each clamped to `MAX_BASE_BR_RANGE - 1`;
  - `mag = Min((mag + 1) >> 1, 6)`;
  - offset by plane (chroma `Min(mag, 3)`), DC position (non-2D `mag + 7`), or
    low-frequency (`mag + 7`).
- Add the `MAG_REF_OFFSET_WITH_TX_CLASS` table and `MAX_BASE_BR_RANGE` constant;
  reuse `splot_recon::TransformClass`.

`CoeffBrContext` reads a caller-provided row-major `Level[]` slice; out-of-bounds
and short-slice neighbour reads contribute 0, so the function is total and
panic-free. A module-level `const` compile-time spec-contract check (the
non-test consumer) pins the core arithmetic.

Non-goals:

- No `coeffs()` decode-loop wiring (the context is not read by any decode path),
  so the minimal-fixture decode output is unchanged.
- No `coeff_base`, no IDTX-variant contexts, no sign contexts (`dc_sign` ctx,
  `idtx_sign`), and no full per-transform-block level/sign buffers.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `coeff_br` coefficient base-range CDF context in
  the tile CDF selection subset, while broader §8.3 coefficient CDF selection and
  the coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
