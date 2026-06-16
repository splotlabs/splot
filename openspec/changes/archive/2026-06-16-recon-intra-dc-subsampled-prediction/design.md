## Context

Before this change, the `splot-recon` intra module supported AV2 §7.13.2.10
square and rectangular DC prediction, §7.13.2.2 basic/PAETH prediction,
§7.13.2.13 smooth prediction, and the cardinal pAngle 90/180 subset of
§7.13.2.8 directional prediction, while AV2 §7.13.2.11 DC intra prediction
subsampled process support for large chroma DC/CfL paths was missing.

This change is source-backed reconstruction work only. It does not parse tile
syntax, decide the §7.13.2.1 `largeChroma`/`UV_CFL_PRED` dispatch predicate, or
expand the runtime `splot decode` tier.

## Goals / Non-Goals

**Goals:**

- Add `RECON-INTRA-DC-SUBSAMPLED-PREDICTION` as a supported
  source-backed primitive row.
- Implement a scheduler-free `splot-recon` primitive for AV2 §7.13.2.11 over
  prepared `LeftCol[0..h]` and `AboveRow[0..w]` samples.
- Reuse the AV2 approximate-divide path from the existing DC implementation
  rather than normal integer division.
- Add a current-frame workspace helper that can write subsampled DC prediction
  from in-storage neighboring edges without inventing AV2 edge availability.
- Extend the existing recon intra fuzz target and status docs.

**Non-Goals:**

- No full `predict_intra()` dispatcher.
- No CfL luma-subsampling, chroma-from-luma transform, or `UV_CFL_PRED`
  syntax/runtime handling.
- No data-driven prediction, IBP, general directional prediction, IDIF, MRL,
  edge filtering, transform, residual, loop filtering, film grain, reference
  refresh, or broad runtime decode support.
- No AVM/dav2d integration, new dependencies, or dependency graph changes.

## Decisions

1. Put the primitive in `splot-recon`, not `splot-decode`.
   `splot-recon` owns decoded-frame storage and scalar reconstruction
   primitives. The primitive is useful to future tile/block reconstruction
   callers and remains independent of decode orchestration.

2. Add a small subsampled-DC module instead of growing `intra.rs`.
   `crates/splot-recon/src/intra.rs` is already at the soft line budget. The
   new code should live in a dedicated module and reuse narrowly exposed DC
   helpers for approximate division, clipping, and output-shape validation.

3. Keep the direct primitive prepared-edge based.
   The API accepts `IntraDcEdges<'_, T>` and `IntraRectBlockSize`, validating the
   full edge slices even when §7.13.2.11 samples every other index for
   dimensions greater than 32. This keeps edge preparation and AV2 availability
   outside the primitive, matching existing `splot-recon` intra APIs.

4. Workspace prediction derives only in-storage edges.
   The workspace helper may produce the no-edge midpoint when neither edge is
   available, but it must not invent §7.13.2.1 fallback samples for missing
   left-only or above-only availability. Runtime callers that need spec fallback
   samples must materialize them explicitly in a later dispatch-oriented change.

5. Reuse `recon_intra_prediction_bytes`.
   The existing fuzz target already exercises source-backed intra primitives and
   workspace paths. Adding subsampled DC branches keeps CI fuzz-smoke bounded
   and avoids a redundant target.

## Risks / Trade-offs

- Subsampled DC support could be mistaken for full CfL or broad chroma intra
  prediction. Mitigation: narrow names, OpenSpec non-goals, matrix notes, and
  unchanged partial broad rows.
- Moving shared DC helpers can regress existing square/rectangular DC behavior.
  Mitigation: retain existing tests and add focused tests for the new helper
  users before status docs are updated.
- Workspace helper semantics for missing edges can be overinterpreted. Mitigation:
  document that workspace fallback is not full §7.13.2.1 edge preparation and
  keep runtime decode behavior unchanged.
