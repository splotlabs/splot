## Context

`splot-recon` already has scalar, scheduler-free primitives for AV2 v1.0.0 §7.13.2.8 one-sided pAngles `45`, `67`, and `203`, and middle pAngles `113`, `135`, and `157`. `CurrentFrameWorkspace` already bridges several source-backed intra primitives by gathering in-storage prepared edges and writing into checked plane storage, but it does not yet expose directional-angle workspace handoff.

The design must preserve the existing crate boundaries: no new dependency edge, no runtime `splot-decode` integration, no fallback edge synthesis, and no scheduler state in `splot-recon`. The committed AV2 spec mirror remains the source for section citations, and `docs/IMPLEMENTATION-MATRIX.toml` remains the status source.

## Goals / Non-Goals

**Goals:**

- Add Feature ID `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`.
- Add additive public `CurrentFrameWorkspace` methods for one-sided and middle directional-angle prediction.
- Gather only fully available in-storage prepared chroma edges and call the existing non-IDIF directional-angle primitives.
- Reject luma (`PlaneId::Y`) until IDIF is implemented.
- Validate target geometry and edge availability before any target mutation.
- Add unit/fuzz evidence and update support/conformance metadata.

**Non-Goals:**

- No AV2 §7.13.2.1 fallback edge synthesis or availability policy.
- No full §7.13.2.7 `predict_intra()` dispatch, MRL, luma IDIF, angle deltas, wide-angle mapping, or directional IBP.
- No runtime `splot decode` behavior, `splot-decode -> splot-recon` dependency change, external decoder integration, or AVM/dav2d invocation.
- No new third-party dependency or copied third-party source/prose/tables.

## Decisions

1. Add typed workspace entrypoints only.

   `CurrentFrameWorkspace` will expose helpers that accept `IntraDirectionalAngle` or `IntraMiddleDirectionalAngle`. Callers that have raw AV2 pAngles can use the existing `try_from_p_angle()` constructors before calling workspace methods. This keeps unsupported-angle handling explicit and avoids extra public workspace API surface.

2. Reject luma before gathering edges.

   AV2 §7.13.2.8 enables IDIF on plane 0. The existing directional-angle primitives intentionally cover the non-IDIF path, so workspace helpers reject `PlaneId::Y` with a typed luma-IDIF-unsupported error before edge gathering or target mutation.

3. Gather exact prepared-edge scratch inside `CurrentFramePlane`.

   One-sided pAngles `45` and `67` gather `AboveRow[0..w+h)` from row `y - 1`, columns `x..x+w+h`; pAngle `203` gathers `LeftCol[0..w+h)` from column `x - 1`, rows `y..y+w+h`. Middle pAngles gather `AboveRow[-1..w)` from row `y - 1`, columns `x-1..x+w`, and `LeftCol[-1..h)` from column `x - 1`, rows `y-1..y+h`. The scratch vectors are owned by the workspace helper and borrowed by the primitive for validation and prediction.

4. Use one workspace-specific missing-edge error.

   Primitive missing-edge errors do not include the workspace plane or target rectangle. A new workspace error will include `plane`, `p_angle`, `edge`, and `rect`, matching the existing PAETH/smooth/cardinal workspace error style and making boundary failures diagnosable without claiming validator diagnostics.

5. Check row/column spans explicitly before row slicing.

   `row_range` validates total backing storage length, not semantic row width. Directional one-sided above and middle above ranges can be wider than the target block, so the helper will explicitly check `x + edge_len <= storage_width` or `x + w <= storage_width` before slicing. Left-column cases will explicitly check `y + edge_len <= storage_height` or `y + h <= storage_height`.

6. Rely on existing primitive validation for sample range, output shape, and arithmetic after edge availability is proven.

   The workspace validates target geometry and edge availability before creating `&mut self.samples[output_start..]`. The primitive then validates sample type, edge length, edge sample range, output stride/length, pAngle support, and arithmetic before writing. This preserves the existing no-partial-mutation pattern for invalid prepared inputs.

## Risks / Trade-offs

- [Risk] Workspace helpers may be mistaken for full AV2 edge preparation. -> Mitigation: docs, matrix notes, specs, and PR text explicitly limit them to fully available in-storage prepared edges and list fallback synthesis/runtime dispatch as non-goals.
- [Risk] Above-edge row gathering can accidentally cross into the next row. -> Mitigation: add explicit storage-width span checks before using `row_range`.
- [Risk] Missing-edge errors could lose context if primitive errors are reused. -> Mitigation: add workspace-specific edge-unavailable variants with plane and rectangle context.
- [Risk] Fuzz coverage may only exercise direct primitives. -> Mitigation: extend both interior and random workspace fuzz cases to call the new public workspace helpers with valid and boundary-selected coordinates.

## Migration Plan

This is an additive library API change. Existing callers keep their current behavior. Rollback is deleting the new helper methods, errors, tests, matrix row, and fuzz calls before release.

## Open Questions

None. The slice is intentionally limited to source-backed workspace handoff over already-modeled directional-angle primitives.
