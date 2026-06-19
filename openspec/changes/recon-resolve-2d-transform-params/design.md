## Context

`RECON-INVERSE-TRANSFORM-2D-OUTER` deliberately left `row_type` / `col_type` /
`row_shift` / `col_shift` as caller-resolved fields of `InverseTransform2dOuter`.
The subsequent `RECON-TRANSFORM-SHIFT-LOOKUP` and `RECON-GET-TRANSFORM-1D-TYPE`
rows built the two § 7.15.4 derivations a caller needs to fill those fields, and
the `get-transform-1d-type` row recorded the "combined transform-parameter
resolve helper" as the remaining follow-on. This change is that follow-on.

## Goals / Non-Goals

**Goals:**

- Add a single helper that resolves the § 7.15.4 transform-parameter set
  (`row_type`, `col_type`, `row_shift`, `col_shift`, `plane_tx_type_is_idtx`)
  from one transform-block fact source.
- Eliminate the dual-source hazard at the call site by deriving all
  transform-size/type fields from the same `(plane_tx_type, log2_width,
  log2_height)` the result stores.
- Keep it a total, panic-free `const fn` with no runtime rewiring and no new
  error variant.

**Non-Goals:**

- The § 7.15.4 DPCM-direction selection from the prediction mode (`dpcm` stays a
  caller fact).
- Any wiring into the runtime decode path, `compute_tx_type`, the secondary
  transform, or the coefficient entropy decode.

## Decisions

- **Realize the helper as `InverseTransform2dOuter::resolve`, building the whole
  struct.** A constructor that takes `(log2_width, log2_height)` once and uses it
  for the shift lookup, the adjusted-size type derivation, and the stored
  dimensions makes the three internally consistent by construction. This directly
  applies the lesson that a single recon primitive must never mix an internal
  table lookup with a separately-supplied derived dimension.
- **Feed the per-pass `get_transform_1d_type` calls the adjusted sample sizes.**
  § 7.15.4.1 sets `rowType = get_transform_1d_type(0, w)` and
  `colType = get_transform_1d_type(1, h)` with `w = 1 << adjLog2W`,
  `h = 1 << adjLog2H`, where `adjLog2{W,H} = Min(log2{W,H}, 5)`. The DDT
  substitution's `sz != 4` guard therefore keys off the adjusted size, so a
  64-wide / 4-tall shape can substitute the row pass but not the column pass.
- **Feed `transform_shift` the original log2 dims.** `Transform_Shift` is keyed
  by `txSz` (the original `(log2W, log2H)`), so the shift lookup uses the
  unadjusted dims, matching the existing `RECON-TRANSFORM-SHIFT-LOOKUP` contract.
- **Validate before resolving.** `transform_shift` rejects a non-`TX_SIZES_ALL`
  shape before any adjusted-size arithmetic runs, and `get_transform_1d_type`
  rejects an out-of-range `PlaneTxType`; a rejected call therefore resolves no
  partial parameters and needs no new error variant.
- **Keep it `const fn`.** Like its sibling derivations, the helper resolves a
  fixed transform shape at compile time, anchored by a module-level
  `const`-evaluated spec contract pinning TX_4X4 DCT_DCT.

## Risks / Trade-offs

- A wrong per-pass size argument would silently mis-derive the DDT substitution.
  The `resolve_applies_ddt_substitution_per_pass_on_the_adjusted_size` test pins
  the asymmetric 8x4 case where one pass substitutes and the other does not.
- The helper is loaded ahead of its runtime caller. That matches the established
  recon-primitive pattern (the whole residual-math stack was built ahead of
  wiring); the helper is exercised by tests and a compile-time const contract,
  and the matrix/roadmap keep the runtime wiring, `compute_tx_type`, and full
  decode partial or unimplemented.
