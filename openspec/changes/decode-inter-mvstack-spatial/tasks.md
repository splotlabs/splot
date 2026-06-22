# Tasks

## 1. Fixture (verify oracles first)
- [x] 1.1 Generate `syn-2frame-inter-mvstack-64x64.ivf` from a project-owned
      synthetic Y4M (four flat 32x32 luma quadrants + flat chroma; frame 1 shifted
      left 6 luma samples) at `--qp 80 --sb-size 64 --min-partition-size 32
      --max-partition-size 32 --enable-rect-partitions=1` with broad decode tools
      disabled.
- [x] 1.2 Confirm avmdec `--rawvideo --i420` == dav2d `--demuxer ivf`
      byte-for-byte (md5 `e5b581a55433785c0071b635d5642083`).
- [x] 1.3 Confirm via the AVM inspection oracle that frame 1 is a 64x64 SPLIT into
      four 32x32 inter blocks: block 0 NEWMV (col 48), blocks 1-3 NEARMV (col 48,
      predicted from block 0 — not the zero fallback).
- [x] 1.4 Register in the conformance manifest + reciprocal
      LOCAL-REFERENCE-EVIDENCE entry.

## 2. find_mv_stack kernel (§ 7.11 / § 7.12 spatial subset)
- [x] 2.1 Per-MI neighbour MV grid (`IsInters` / `RefFrames[0]` / `YModes` /
      `Mvs[0]` / `Skips`).
- [x] 2.2 § 7.11.2 `find_mode_ctx` (leftA/aboveA/leftB/aboveB) → NewMvContext.
- [x] 2.3 § 5.20.7.2 neighbour-buffer `is_inter` / `skip_flag` § 8.3.2 contexts.
- [x] 2.4 § 7.12.2 spatial scan-point MV stack (steps 7–15, § 7.12.2.20
      extra-search global fallback, § 7.12.2.23 clamp).
- [x] 2.5 Unit tests for the fixture's worked example.

## 3. Per-leaf inter block decode wiring
- [x] 3.1 Lift the single-64x64 gate; decode every § 5.20.3 leaf inter block.
- [x] 3.2 Derive each block's contexts from the grid; read mode_info with them.
- [x] 3.3 § 5.20.7.8 DRL → RefMvIdx; § 5.20.7.13 assign_mv from the stack candidate
      (NEARMV = predictor; NEWMV = clamp(predictor + delta); GLOBALMV = zero).
- [x] 3.4 Record each block into the grid; § 7.13.3.18 MC each block at its rect.

## 4. Verify + gate
- [x] 4.1 `splot decode syn-2frame-inter-mvstack-64x64.ivf --output-format raw`
      == oracle md5 byte-for-byte.
- [x] 4.2 Single-block inter + general-intra fixtures byte-identical (no regression).
- [x] 4.3 `cargo xtask ci` passes; `openspec validate --all` clean.

## 5. Deferred (out of scope, gated absent before output)
- [ ] 5.1 Temporal MV candidates (§ 7.12.2.7 / § 7.12.2.8).
- [ ] 5.2 Compound prediction + compound search / derived / TIP candidates.
- [ ] 5.3 Warp candidates + find-warp-samples; ref-MV bank; derived-SMVP.
- [ ] 5.4 DRL reorder sort (§ 7.12.2.19); § 7.12.2.5 scan-col wider reach.
- [ ] 5.5 Large-block (> 32x32) extra MVP combinations; multi-block skip == 0
      residual (per-block transform sizes).
