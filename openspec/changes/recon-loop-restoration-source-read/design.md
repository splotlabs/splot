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
- Require `curr_frame.info() == cdef_frame.info()` and matching selected-plane
  view geometry so a source switch cannot silently mix frame metadata, visible
  origins, or strides.
- Read the selector's returned current-plane coordinates as absolute
  coded-storage coordinates in the selected `PlaneRef` backing buffer. The
  helper does not apply a non-zero visible crop origin before computing
  `y * stride + x`.
- Return typed `ReconError` values for frame metadata or plane-view geometry
  mismatch, caller-resolved chroma subsampling that does not match the source
  frame pixel format, missing selected chroma plane, caller-resolved bounds
  outside the selected coded plane, unsupported sample storage for the frame bit
  depth, and source samples outside the active bit-depth range.

## Out Of Scope

- Deriving `LoopRestorationSourceBounds` from the section 7.20.1 restoration
  traversal.
- Calling the Wiener NS or chroma loop-restoration filters.
- PC-Wiener classification, GDF, BRU, CDEF/CCSO orchestration, and runtime
  decode output.
