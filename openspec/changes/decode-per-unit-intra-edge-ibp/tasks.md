# Tasks

## 1. Per-unit edge filters and IBP
- [x] 1.1 Tile-wide luma smoothness grid + inter-walk recording.
- [x] 1.2 § 7.13.2.7 one-sided/middle filter resolution on the per-unit
      arms (strength, corner, numPx; frame-edge corner fallback).
- [x] 1.3 § 7.13.2.9 blend routing on the even-delta one-sided arms
      (primary + opposite-edge secondary, per-edge filters).
- [x] 1.4 Per-unit § 5.20.7.29 WAIP re-plan (unit dims, zone re-select,
      square single-unit plans onto the rect arms).

## 2. IntraBC
- [x] 2.1 Fractional-vector § 7.13.3.18 BILINEAR prediction over the
      current frame (the geometry already derived scaling + clipping).

## 3. Verification
- [x] 3.1 ac0ej3 coded frame 2 pre-filter LUMA byte-exact vs the AVM
      oracle (730,992 divergent samples at batch start, 0 after);
      frames 0/1 byte-exact; frame-0 sentinel intact.
- [x] 3.2 22-stream AVM differential corpus byte-exact; full test suite;
      unit tests pin the spec branch logic, the grid, and both reorder
      arms.
- [x] 3.3 Remaining divergence attributed with named owners: pre-filter
      chroma and the § 7.17 deblock geometry hand-off (diagnosis reports
      committed to the mission workspace).
