## Context

`splot-recon` already exposes source-backed intra prediction primitives for DC,
subsampled DC, IBP DC, PAETH, smooth, and the H/V cardinal directional cases.
The next decoder-conformance gap is AV2 §7.13.2.8 directional-angle prediction,
but the full §7.13.2.7 directional process also needs mode/angle derivation,
wide-angle mapping, MRL, edge filtering, luma IDIF, negative logical edge
indices, directional IBP, and runtime tile dispatch.

This change intentionally implements a small primitive that advances that path
without changing crate dependencies or overclaiming full directional prediction.
The supported slice is chroma, no-MRL, non-IDIF, one-sided pAngles `45`, `67`,
and `203`, grounded in AV2 v1.0.0 §7.13.2.8 and §9.2.

## Goals / Non-Goals

**Goals:**

- Add Feature ID `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`.
- Add a public `splot-recon` API for one-sided prepared-edge angular prediction.
- Support pAngles `45`, `67`, and `203` with §7.13.2.8 non-IDIF bilinear math.
- Validate all public inputs before writing to caller-owned output storage.
- Extend unit tests, fuzz target coverage, matrices, generated docs, and
  OpenSpec records for the narrow capability.

**Non-Goals:**

- Full §7.13.2.1 edge availability/fallback preparation.
- Full §7.13.2.7 mode dispatch, `Mode_To_Angle` derivation, `AngleDelta*`,
  `ANGLE_STEP`, `Mrl_Index_To_Delta`, or `wide_angle_mapping`.
- Middle pAngles `113`, `135`, and `157`, which require two-edge signed logical
  indexing.
- Luma IDIF and the `Dr_Interp_Filter` table.
- MRL, directional IBP, workspace edge synthesis, runtime `splot decode`, AVM or
  dav2d integration, new dependencies, or encoder-facing changes.

## Decisions

1. Add a new module instead of growing the cardinal module.

   `crates/splot-recon/src/intra_directional_angle.rs` keeps the angular
   interpolation code and typed errors separate from the existing H/V cardinal
   API. The cardinal module remains the owner of pAngles `90` and `180`.

2. Accept explicit checked pAngles instead of mode syntax.

   The primitive accepts an `IntraDirectionalAngle` wrapper with constructors for
   `D45`, `D67`, and `D203`. It does not consume AV2 block syntax or derive
   angles from `mode`, `AngleDeltaY`, `AngleDeltaUV`, MRL, or wide-angle mapping;
   those belong in future decode/dispatch layers.

3. Require already-prepared one-sided edge slices.

   For pAngles `45` and `67`, callers provide `AboveRow[0..w+h)`. For pAngle
   `203`, callers provide `LeftCol[0..w+h)`. The primitive does not synthesize
   above-right or below-left fallback samples and therefore does not need
   workspace/tile availability state.

4. Encode only the needed derivative constants.

   The no-IDIF pAngles in this slice require `Dr_Intra_Derivative[45] = 64` and
   `Dr_Intra_Derivative[67] = 24`. Encoding those spec-defined table entries in
   `splot-recon` avoids adding a `splot-core` dependency or copying generated
   table plumbing into the reconstruction crate.

5. Validate before mutation.

   The function validates sample type, output shape, edge length, edge sample
   range, pAngle support, and checked arithmetic before any output write. The
   prediction loops then use only validated indices and arithmetic.

## Risks / Trade-offs

- Narrow public surface may look like less progress than full directional
  prediction -> matrix/docs explicitly state which pAngles and process branches
  are supported and which remain open.
- Hardcoding two derivative entries can drift if the scope expands -> tests and
  docs cite AV2 §9.2, and future broader support should introduce a dedicated
  recon-local generated table or checked table module.
- No workspace helper means runtime code cannot call this directly from in-place
  neighbors yet -> this avoids dishonest edge synthesis; future §7.13.2.1 work
  can materialize prepared edges and pass them to this primitive.
- Bilinear arithmetic errors could wrap on hostile inputs -> size bounds,
  derivative bounds, and checked intermediate arithmetic are validated before
  mutation.
