## Context

`DECODE-LR-CLASSIFIED-WIENER-STORAGE` proved the private helper path that can derive §7.20.4 classified-Wiener `FilterClass` values when supplied decoded `CurrFrame`/`CdefFrame` views and a bounded `LrTxSkip` grid. The live local decoder mission runtime still stops before any such live storage shape is retained: it reaches the loop-restoration frontier before the existing 8-bit-only output path and before tile reconstruction can populate 10-bit frame buffers or transform-skip state.

## Goals / Non-Goals

**Goals:**
- Add `DECODE-LR-RUNTIME-STORAGE-RETENTION` as the live fail-closed frontier after classified-Wiener storage-helper wiring.
- Derive the required 10-bit current/CDEF frame storage footprint and frame-wide `LrTxSkip` grid dimensions from parsed sequence/frame facts.
- Apply existing `DecodeLimits` before future allocation or output paths could use the retained storage shape.
- Update the live local decoder mission diagnostic and support docs to name the new boundary.

**Non-Goals:**
- No decoded sample population, no fake source sample values, and no fake `LrTxSkip` values.
- No §7.20.3/§7.20.4 filter application, `FilterClass` grid retention, `SubclassLookup`, 10-bit output, reference refresh, or AVM/dav2d equality claim.
- No new crate dependency or dependency-direction change.

## Decisions

1. Keep the retention proof in `splot-decode`.
   - Rationale: the missing piece is runtime ownership/limit policy around existing recon frame types, not a new recon primitive.
   - Alternative considered: adding a public `splot-recon` frame allocator API. That would widen the API surface before the decoder has a real caller that can populate the buffers.

2. Retain storage shape and byte budgets, not sample values.
   - Rationale: local decoder mission still lacks tile reconstruction into 10-bit current/CDEF frames and live transform-skip retention; using zero-filled buffers for classification would create confident wrong behavior.
   - Alternative considered: instantiate zero-filled `DecodedFrame<u16>` / `LrTxSkip` storage and stop afterward. That proves allocation but risks obscuring the fact that no decoded values exist.

3. Use existing decode limits.
   - Rationale: `MaxDecodedFrameBytes`, `MaxLumaSamplesPerFrame`, and allocation-length checks already gate decoded frame storage. The `LrTxSkip` grid is bounded by the frame MI grid and checked under the same decoded-storage policy until a dedicated workspace-storage limit exists.
   - Alternative considered: adding a new limit. That would be a broader API change and is not required to advance this fail-closed frontier.

## Risks / Trade-offs

- Limit semantics are conservative: the grid is checked with decoded-storage limits even though it is not a decoded plane. Mitigation: document this in the feature row and tests, and keep the grid byte count explicit.
- The diagnostic advances without producing pixels. Mitigation: the unsupported message explicitly states that storage is unpopulated and filtering/output/reference refresh are not applied.
