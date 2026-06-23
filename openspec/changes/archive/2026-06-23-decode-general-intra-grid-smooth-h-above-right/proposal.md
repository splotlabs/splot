## Why

The general intra decode admits a full-superblock § 7.13.2.13 `SMOOTH_H_PRED`
luma block only in the FIRST superblock row (`frontier.r == 0`), where the
§ 7.13.2.1 top-right sentinel `AboveRow[w]` is the no-neighbour fallback. At
superblock row > 0 the `SMOOTH_H_PRED` predictor (`predH2`) reads the real
reconstructed above-right sentinel — the bottom row of the already-decoded
diagonally-above-right superblock — over the luma (`sub_x == 0`) path that no
oracle fixture had exercised, so it was rejected with the
`general_intra_smooth_h_above_right_unverified` diagnostic
(`DECODE-GENERAL-INTRA-GRID-NEIGHBOUR-NONDC`). The SMOOTH chroma 2-D grid path
(`DECODE-GENERAL-INTRA-GRID`, `sub_x == 1`) and the SMOOTH_V grid path already
read the real cross-superblock above-right bit-exact through the same
plane-general edge builder + above-right resolver; the only missing piece for
luma is an oracle fixture that proves the `sub_x == 0` above-right VALUE path.
SMOOTH is reliably encoder-selected over a horizontal gradient, so this is
generatable.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-GRID-SMOOTH-H-ABOVE-RIGHT`.
- Lift the first-superblock-row gate on the full-superblock § 7.13.2.13
  `SMOOTH_H_PRED` luma neighbour-edge decode: admit a full-superblock
  (`n4w == FULL_SB_N4_LUMA`) `SMOOTH_H_PRED` luma block at ANY 2-D grid position.
  The admission match collapses the prior first-row-admit / row>0-reject arms into
  one `n4w == FULL_SB_N4_LUMA` admit.
- Keep the reconstruction unchanged:
  `reconstruct_general_intra_luma_nondc_neighbour_block_into` already delegates to
  the plane-general `reconstruct_general_intra_smooth_over_edges_into`, which derives
  the luma § 7.13.2.1 `num4AboveRight` from `luma_num4_above_right_from_block_decoded`
  (§ 5.20.7.25 `count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state) and
  resolves the top-right sentinel `AboveRow[w]` via `resolve_smooth_above_right_sentinel`
  — the same machinery the SMOOTH chroma / SMOOTH_V grid paths already use bit-exact.
- Add the project-owned `syn-shgrid-intra-128x128-q80.ivf` fixture (a 2x2
  superblock grid whose bottom-left, row>0, non-rightmost superblock codes
  `SMOOTH_H_PRED` luma over a horizontal gradient and reads the real reconstructed
  cross-superblock above-right value 200, not the edge-clamp 100) and prove it
  decodes bit-exactly to the avmdec AND dav2d oracle, where the old code rejected
  the frame.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-grid-smooth-h-above-right`: Crate-private general intra
  full-superblock `SMOOTH_H_PRED` luma decode reading a real reconstructed
  cross-superblock above-right sentinel `AboveRow[w]` at superblock row > 0.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the
  full-superblock SMOOTH_H luma cross-superblock above-right decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs` (the admission match, via
  the `runtime_minimal/general_intra.rs` module) and the doc comment on
  `crates/splot-decode/src/runtime_minimal_recon.rs`
  `resolve_smooth_above_right_sentinel` (now reachable for the luma row>0
  above-right value). No new public surface; the recon code path is reused.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. SMOOTH_H sub-partitioned
  (SPLIT-child) cross-superblock above-right, SMOOTH_V below-left sub-block
  sentinels, SMOOTH chroma sub-blocks, directional / PAETH neighbour cases,
  multiple tiles, inter prediction, and in-loop filters remain out of scope and
  rejected with structured diagnostics.
