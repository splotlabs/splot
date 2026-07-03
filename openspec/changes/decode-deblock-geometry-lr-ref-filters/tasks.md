# Tasks

## 1. Deblock geometry
- [x] 1.1 Per-unit luma deblock records from the residual execution.
- [x] 1.2 Coding-block origins on every record; isBlockEdge vs
      MiRowBase/MiColBase; per-plane chroma bases.

## 2. LR reference filters
- [x] 2.1 Retain frame-level Wiener-NS bank taps per reference slot.
- [x] 2.2 Ordered search_frame_filters entries per plane; reference-hit
      and PC-Wiener-offset resolution in the filter match.

## 3. Verification
- [x] 3.1 ac0ej3 coded frames 0-2 POST-FILTER luma byte-exact vs the AVM
      oracle (per-stage: deblock, CDEF, CCSO, LR all exact); frame-0
      sentinel intact; 22-stream corpus byte-exact; full test suite.
