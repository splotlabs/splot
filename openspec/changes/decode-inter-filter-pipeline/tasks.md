# Tasks

## 1. Filter pipeline reuse (§ 7.2)
- [x] 1.1 Split the intra sink: `finish_intra_reconstruction` preamble +
      `into_filtered_frame` pipeline; add `for_final_filtering`.
- [x] 1.2 Record inter deblock geometry per transform (+ § 5.20.6.2
      max-rect skip tiling); retain CDEF/CCSO grids + LR blocks/filters.
- [x] 1.3 Invoke the pipeline before freeze; lift the filter gate arms.

## 2. Motion-mode reads (§ 5.20.7.14 / § 7.11.4)
- [x] 2.1 Derive `WarpSampleFound[0]` in the warp-context scan.
- [x] 2.2 Read `use_extend_warp` / `use_local_warp` with fail-closed
      prediction defers; wire the two CDF banks.

## 3. Entropy fixes surfaced by the fixture sweep
- [x] 3.1 Read inter chroma `cctx_type` per § 5.20.7.27; defer nonzero.
- [x] 3.2 Retain and defer § 5.18.7.12 CCSO reference reuse.

## 4. Verification
- [x] 4.1 Commit deblock/CDEF/CCSO-active inter fixtures, each byte-exact
      vs avmdec (manifest + local-reference evidence + pinned hash tests).
- [x] 4.2 Frame-0 sentinel and all committed fixtures unchanged.
- [x] 4.3 Root-cause the sweep's mismatch findings and close the
      confident-wrong surface: multi-transform-unit intra prediction defers
      unless the split is provably block-equivalent (§ 5.20.7.24 per-unit
      prediction is the follow-on change, oracle streams retained), and the
      invented num_total_refs >= 2 clause on reference_select is dropped
      (1-reference reference_select frames defer at the compound context).
