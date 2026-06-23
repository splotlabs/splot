## Overview

This change composes the existing `loop_restoration_source_sample` selector with
zero-copy immutable frame views. It does not alter the selector semantics; it
only performs the storage read implied by AV2 section 7.20.2 once the selector
has chosen `CurrFrame` or `CdefFrame`.

## Design

- Add `LoopRestorationSourceSampleValue<T>` containing the resolved
  `LoopRestorationSourceSample` and the selected sample value.
- Add `loop_restoration_source_sample_value(plane, x, y, bounds, curr_frame,
  cdef_frame)` in `crates/splot-recon/src/loop_restoration.rs`.
- Require `curr_frame.info() == cdef_frame.info()` so a source switch cannot
  silently mix different frame geometry or output metadata.
- Use the existing `FrameRef::plane` and `PlaneRef::visible_rows` APIs for the
  read. The helper treats the selector's returned coordinates as visible-plane
  coordinates and lets `PlaneRef` handle any non-zero visible origin in the
  backing storage.
- Return typed `ReconError` values for frame metadata mismatch, missing selected
  chroma plane, and caller-resolved bounds that address outside the selected
  visible plane.

## Out Of Scope

- Deriving `LoopRestorationSourceBounds` from the section 7.20.1 restoration
  traversal.
- Calling the Wiener NS or chroma loop-restoration filters.
- PC-Wiener classification, GDF, BRU, CDEF/CCSO orchestration, and runtime
  decode output.
