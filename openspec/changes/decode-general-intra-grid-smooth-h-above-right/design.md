## Context

The 2-D grid non-DC SMOOTH brick (`DECODE-GENERAL-INTRA-GRID-NEIGHBOUR-NONDC`)
admits a full-superblock § 7.13.2.13 `SMOOTH_V_PRED` luma block at any 2-D grid
position and a `SMOOTH_H_PRED` luma block only in the FIRST superblock row
(`frontier.r == 0`), where the § 7.13.2.1 top-right sentinel `AboveRow[w]` is the
no-neighbour fallback. `SMOOTH_V_PRED`'s predictor never reads the above-right
sentinel VALUE, so its row>0 path was fully verified; `SMOOTH_H_PRED`'s `predH2`
DOES read `AboveRow[w]`, and at row > 0 that is the real reconstructed bottom row
of the diagonally-above-right superblock — a luma (`sub_x == 0`) above-right VALUE
path no oracle fixture had exercised, so it was rejected
(`general_intra_smooth_h_above_right_unverified`) and deferred here.

The SMOOTH chroma grid brick (`DECODE-GENERAL-INTRA-GRID`, `sub_x == 1`) had
already proven the real § 7.13.2.1 above-right sentinel bit-exact for a row>0
full-superblock SMOOTH chroma block, via `count_top_right_avail` (§ 5.20.7.25)
over the § 5.20.2.3 `BlockDecoded` state and `resolve_smooth_above_right_sentinel`.
The luma neighbour reconstruction
(`reconstruct_general_intra_luma_nondc_neighbour_block_into`) delegates to the SAME
plane-general edge builder + above-right resolver (`reconstruct_general_intra_smooth_over_edges_into`),
so the row>0 SMOOTH_H luma case is already implemented in the reconstruction layer
— only the admission gate blocks it.

## Decisions

- **Lift the gate, not the reconstruction.** The only change is in the admission
  match in `runtime_minimal/general_intra.rs`: the two prior `SMOOTH_H_PRED`
  full-superblock arms (`frontier.r == 0` admit + the row>0 reject) collapse into
  one `(Some(SupportedNonDcLumaMode::SmoothHorizontal), _) if n4w == FULL_SB_N4_LUMA => {}`.
  The recon delegate is untouched; it already derives `num4AboveRight` from
  `luma_num4_above_right_from_block_decoded` and runs `resolve_smooth_above_right_sentinel`.

- **Verified-subset discipline.** Only the full-superblock (`n4w == FULL_SB_N4_LUMA`)
  SMOOTH_H row>0 cross-superblock above-right is admitted (the case the
  `syn-shgrid` fixture proves bit-exact). The SMOOTH_H SPLIT-child cross-superblock
  above-right (superblock-relative row 0) keeps the
  `general_intra_smooth_h_above_right_unverified` diagnostic; SMOOTH_V below-left
  sub-block sentinels, SMOOTH chroma sub-blocks
  (`general_intra_smooth_chroma_subblock`, pinned by `syn-svsplit-intra-64x64-q140.ivf`),
  and directional / PAETH neighbour cases stay deferred.

- **Oracle anchor.** The committed `syn-shgrid-intra-128x128-q80.ivf` (2x2
  superblock grid: bottom-left SMOOTH_H luma over a horizontal gradient with DC
  chroma; top-right a distinct flat luma 200) was confirmed via temporary
  instrumentation to read `num4AboveRight == 16` and the real above-right value 200
  (vs the edge-clamp 100) for the bottom-left row>0 block, and to be rejected by the
  pre-change code. avmdec and dav2d agree byte-for-byte (md5
  `fe420ce870c13a8055aa83fd5aa64740`); the pinned splot frame hash is
  `d1ce39cc3d79f5c46fdea67ad57ec4edd5dfed088ee39fd7029fda1bbb11e0e8`.

## Risks / Trade-offs

- The fixture's bottom-left luma is a horizontal gradient that the encoder reliably
  codes as SMOOTH_H; the other three superblocks are DC luma with supported chroma
  (DC, or full-superblock SMOOTH). Per-superblock distinct flat chroma values force
  the encoder's RDO to pick DC chroma (a uniform-chroma frame ties chroma modes and
  the encoder picks an unsupported H_PRED chroma); the fixture's chroma was tuned so
  every block resolves to a supported chroma predictor.
- dav2d (local build commit `f4f96cb0`) agrees with avmdec on this clean
  SMOOTH-over-gradient frame; an earlier horizontal-gradient-luma variant produced
  an off-by-one dav2d divergence in the AC residual, so the fixture was simplified to
  a DC-luma background that keeps all three decoders byte-identical.
