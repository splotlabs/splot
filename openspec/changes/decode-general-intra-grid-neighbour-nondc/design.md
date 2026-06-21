## Context

The multi-block neighbour SMOOTH brick (`DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH`)
decoded the first general-intra non-DC luma block reading a REAL reconstructed
neighbour edge, but conservatively gated it to the FIRST superblock row
(`frontier.r == 0`, `haveAbove == 0`), where only the real reconstructed left
column is read and the § 7.13.2.1 above row + top-right sentinel are the
no-neighbour fallback. A row>0 SMOOTH luma block reads the real reconstructed
above row and (when non-rightmost) a real above-right sentinel; that brick
rejected it (`general_intra_multirow_neighbour_non_dc`) and deferred it here.

The SMOOTH chroma grid brick (`DECODE-GENERAL-INTRA-GRID`) had already proven the
real above row + real § 7.13.2.1 above-right sentinel bit-exact for a row>0
full-superblock SMOOTH chroma block, via `full_sb_num4_above_right` (§ 5.20.7.25
`count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state) and
`resolve_smooth_above_right_sentinel`. The luma neighbour reconstruction
(`reconstruct_general_intra_luma_nondc_neighbour_block_into`) delegates to the SAME
plane-general edge builder + above-right resolver, so the row>0 luma case is
already implemented in the reconstruction layer — only the admission gate blocks it.

## Decisions

- **Lift the gate, not the reconstruction.** The only change is in the admission /
  dispatch in `runtime_minimal.rs`: the two prior arms (first-row admit + row>0
  reject) collapse into one `(Some(_), _) if n4w == FULL_SB_N4_LUMA => {}`, and
  `nondc_luma_has_neighbour` drops its `&& frontier.r == 0`. The recon delegate is
  untouched; it already reads `haveAbove`/`haveLeft` per position and runs the
  above-right resolver.

- **Reuse the plane-general edge builder.** For a row>0 luma block § 7.13.2.1
  supplies the real reconstructed above row `CurrFrame[0][y - 1][...]` (the bottom
  row of the already-decoded above superblock) via the workspace `intra_dc_edges_for_rect`
  above samples; SMOOTH_V reads `AboveRow[j]` and the bottom-left sentinel `LeftCol[h]`;
  SMOOTH_H reads `LeftCol[i]` and the top-right sentinel `AboveRow[w]`. The luma
  above-right (`full_sb_num4_above_right(c, n4w, mi_cols, sub_x == 0)`) is already
  computed and passed by the dispatch; the resolver returns the real reconstructed
  above-right when decoded and in-frame.

- **No IDIF / edge-filter synthesis.** SMOOTH prediction is § 7.13.2.13 linear
  interpolation over the edges, not an § 7.13.2.8 angle copy, so the non-flat real
  above-row edge reconstructs bit-exact (unlike a directional angle, where the
  `enableIdif == 0` bilinear reduction equals the spec IDIF 4-tap only for a flat
  edge). Neighbour-having directional luma stays rejected here.

- **Keep the § 8.3.2 ctx rejection intact.** SMOOTH_V/H are non-directional
  (`modeDelta < NON_DIRECTIONAL_MODES_COUNT`), so the `y_mode_index` ctx stays 0 and
  they are admitted; a directional neighbour (`ctx != 0`) is still rejected before
  any symbol is read.

- **Verify with a fixture whose row>0 block the old code rejected.**
  `syn-vgrid-intra-192x128-q120` is a 3x2 superblock grid (vertical gradient with a
  small per-superblock-column tint, flat chroma); the right two columns code as
  SMOOTH_V_PRED luma, and the decisive block is the row>0 SMOOTH_V luma superblock at
  the MIDDLE (non-rightmost) column. Mode instrumentation (since removed) confirmed
  `y_mode == SMOOTH_V_PRED == 10` at `frontier.r == 16, frontier.c == 16`, and the OLD
  code rejected the frame with `general_intra_multirow_neighbour_non_dc`. With the gate
  lifted it decodes bit-exact to avmdec AND dav2d (md5 `136a87190eeecb1ccd32e7cf27861c9c`).

## Risks / Trade-offs

- The above-right resolver runs for the non-rightmost row>0 SMOOTH_V luma block but
  SMOOTH_V's predictor does not read the top-right sentinel value, so the fixture
  exercises the resolver code path without asserting the sentinel value drives luma
  output; a SMOOTH_H / full-SMOOTH luma block reading the above-right value is a
  natural follow-on. The full-superblock `num4AboveRight` derivation is exact for the
  admitted subset (the same helper the chroma grid uses); a general per-transform-block
  `count_top_right_avail` arrives with the sub-partitioned brick.
- Sub-superblock (split) non-DC blocks, neighbour-having directional luma (real IDIF),
  the `ctx != 0` `y_mode_index` decode, multiple tiles, inter, and in-loop filters
  remain deferred.
