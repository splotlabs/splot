## Why

The § 7.15.4 outer 2D inverse transform (`RECON-INVERSE-TRANSFORM-2D-OUTER`) and
its two parameter derivations — the `Transform_Shift` lookup
(`RECON-TRANSFORM-SHIFT-LOOKUP`) and `get_transform_1d_type`
(`RECON-GET-TRANSFORM-1D-TYPE`) — are all in place, but they are composed by the
caller: `inverse_transform_2d_outer` takes `row_type` / `col_type` / `row_shift`
/ `col_shift` pre-resolved. The combined § 7.15.4 transform-parameter resolve
helper that ties the two derivations together (named as a deferred follow-on on
the `get-transform-1d-type` row) is the clean, self-contained next residual-path
brick, and the genuine prerequisite the runtime needs once a coefficient block
hands it a dequantized array.

## What Changes

- Add Feature ID `RECON-RESOLVE-2D-TRANSFORM-PARAMS`.
- Add `InverseTransform2dOuter::resolve(plane_tx_type, log2_width, log2_height,
  use_ddt, lossless, bit_depth, dpcm) -> Result<Self>` to
  `crates/splot-recon/src/inverse_transform_2d_outer.rs`.
- Derive `row_shift` / `col_shift` from `Transform_Shift[txSz]` via
  `transform_shift(log2_width, log2_height)`, `row_type` / `col_type` from
  `get_transform_1d_type` over the *adjusted* per-pass sample sizes
  `w = 1 << Min(log2_width, 5)` and `h = 1 << Min(log2_height, 5)` (exactly the
  § 7.15.4.1 `rowType = get_transform_1d_type(0, w)` /
  `colType = get_transform_1d_type(1, h)`), and `plane_tx_type_is_idtx` from
  `PlaneTxType == IDTX (9)`.
- Resolve every transform-size-and-type-derived value from one
  `(plane_tx_type, log2_width, log2_height)` source that the result also stores,
  so the shifts, the per-pass types, and the stored dimensions cannot disagree.
- Keep the helper total and panic-free: validate the shape via `transform_shift`
  before any adjusted-size arithmetic and the type via `get_transform_1d_type`,
  reusing the existing `ReconError::InvalidTransformShiftShape` and
  `ReconError::InvalidPlaneTxType` variants, so a rejected call resolves no
  partial parameters.
- Preserve the current runtime `splot decode` behavior and all existing
  hash/raw/Y4M output bytes (a `const fn` constructor, no runtime rewiring).
- Add focused tests proving the helper-argument wiring, the per-pass adjusted-size
  DDT substitution, end-to-end equivalence with manual params, fail-atomic
  rejection, and totality across all shapes/types.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate
  `//!` docs.

Non-goals:

- No § 7.15.4 DPCM-direction selection from `YMode` / `UVMode` (the `dpcm` field
  stays a caller fact).
- No wiring into the runtime decode path, no § 7.15.3 secondary transform, no
  § 5.20.7.29 `compute_tx_type` that produces `PlaneTxType`, no coefficient
  entropy decode that produces `Quant`, no dependency-graph change, and no
  AVM/dav2d invocation.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the combined § 7.15.4
  transform-parameter resolve helper.

## Impact

- Affected code: `crates/splot-recon/src/inverse_transform_2d_outer.rs`,
  `crates/splot-recon/src/lib.rs`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `const fn` associated function on the existing
  exported `InverseTransform2dOuter`; no breaking changes.
- Diagnostics impact: none; existing minimal runtime diagnostics and output
  bytes remain unchanged.
- Dependencies and licensing: no new dependencies and no licensing changes.
