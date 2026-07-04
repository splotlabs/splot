## Why

Coded frame 2 of the local decoder mission stream — and every real AVM inter
frame — enables the in-loop filter chain (deblock/CDEF/CCSO/LR), which the
inter frontier rejected at the frame level. The filter orchestration already
exists and is AVM-verified on the intra key-frame path; the inter path
parsed all the filter syntax and discarded it. Behind the gate, fixture
sweeps against avmenc-default streams exposed two entropy desyncs the
narrow committed fixtures had masked: the § 5.20.7.27 `cctx_type` read was
skipped for every inter chroma transform, and § 5.18.7.12
`reuse_ccso`/`sb_reuse_ccso` frames desynced instead of deferring.

## What Changes

- Run the shared § 7.2 final filter pipeline (deblock → CDEF → CCSO → LR)
  on inter frames: the block walk records § 7.17 deblock geometry per
  decoded transform (§ 5.20.6.2 `Max_Tx_Size_Rect` tiling for skip blocks),
  retains the CDEF/CCSO unit grids and LR source blocks it already parses,
  and the frame decode applies the pipeline before freeze. The intra sink's
  pipeline is reused via a `for_final_filtering` workspace constructor; the
  intra-only completion checks move to `finish_intra_reconstruction`.
- Read the § 5.20.7.14 WARP_NEWMV `use_extend_warp` / `use_local_warp`
  motion-mode symbols over § 7.11.4 `WarpSampleFound[0]` (new
  `TileUseExtendWarpCdf[3]` / `TileUseLocalWarpCdf[4]` wiring); EXTENDWARP /
  LOCALWARP prediction defers fail-closed.
- Read the § 5.20.7.27 `cctx_type` symbol for inter chroma transforms
  (`(is_inter || eob != 1) && is_cctx_allowed()`); a nonzero inter value
  defers until the cross-chroma transform reconstruction lands.
- Retain § 5.18.7.12 `reuse_ccso` / `sb_reuse_ccso` on `CcsoPlaneParams`
  and defer frames that use either (reference filter-control retention is a
  later change).
- Lift the deblock/CDEF/LR/CCSO arms from the inter frame-tools gate
  (GDF, film grain, and skip-mode stay gated).

## Impact

- Three new committed fixtures decode byte-identical to
  `avmdec --i420 --rawvideo`: deblock-active, CDEF-active, and fresh-coded
  three-plane CCSO-active inter frames (the CCSO stream is AVM-only
  evidence — dav2d diverges on its AV2 CCSO application).
- The local decoder mission frontier moves from the frame-level filter gate (byte 8345)
  into coded frame 2's tile: the first WARPMV inter-intra block defers at
  byte 8371 (`inter_warp_interintra_unimplemented`) — interintra
  prediction is the next mission family.
- Touches `splot-decode` (inter runtime, recon sink split, CDF wiring),
  `splot-core` (motion-mode index exports, CCSO reuse retention),
  conformance manifest + local-reference evidence. No dependency-graph
  changes.
