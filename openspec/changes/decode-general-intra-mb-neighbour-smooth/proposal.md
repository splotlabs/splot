## Why

The general intra decode path reconstructs DC multi-block frames and the first
non-DC / directional luma modes (§ 7.13.2.13 `SMOOTH_V`/`SMOOTH_H`, § 7.13.2.8
`D135`), but every non-DC / directional luma block so far is gated to the
no-neighbour top-left block, where the § 7.13.2.1 prediction edges reduce to flat
fallbacks. Real AV2 frames code non-DC blocks that read the genuine reconstructed
neighbour edge of an already-decoded block. The next step is the first MULTI-BLOCK
non-DC luma prediction that reads a REAL reconstructed neighbour edge.

The brief expected a cardinal `V_PRED`/`H_PRED` right superblock; the committed
`syn-mbvg-128x64-q80.ivf` fixture instead codes BOTH 64x64 superblocks as
§ 7.13.2.13 `SMOOTH_V_PRED` (confirmed by temporary mode instrumentation). This
is in scope and bit-exact: `SMOOTH_V` is § 7.13.2.13 linear interpolation, not a
§ 7.13.2.8 angle copy, so it reads the real neighbour edge with NO IDIF /
edge-filter synthesis and is bit-exact even over a non-flat neighbour edge.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH`.
- Add `reconstruct_general_intra_luma_nondc_neighbour_block_into`: for a non-DC
  SMOOTH_V/H luma block that has a reconstructed neighbour, build the § 7.13.2.1
  `LeftCol` / `AboveRow` edges from the partially-built frame (real reconstructed
  left column / above row, the bottom-left and top-right sentinels, and the
  no-above / no-left fallbacks per § 7.13.2.1), run the shared `splot-recon`
  § 7.13.2.13 smooth predictor, and add the § 5.20.7.27 residual.
- Reuse the existing § 7.13.2.1 edge builder (generalized from chroma-only to
  luma/chroma as `build_smooth_edges`) and the § 7.13.2.1 top-right sentinel
  resolver; generalize the `num4AboveRight` derivation to luma (`sub_x == 0`).
- Relax the multi-block non-DC luma gate to ALLOW a neighbour-having SMOOTH_V/H
  full-superblock luma block (reading the real neighbour edge) while keeping the
  no-neighbour top-left non-DC / directional path. KEEP rejecting:
  neighbour-having § 7.13.2.8 directional (D135) luma (needs the real IDIF 4-tap
  interpolation over a non-flat edge), sub-superblock non-DC, and the
  not-yet-verified luma / chroma modes, each with a structured
  `decode/unsupported-feature` diagnostic before reconstruction.
- Add the project-owned `syn-mbvg-intra-128x64-q80.ivf` fixture and prove it
  decodes bit-exactly to the avmdec AND dav2d oracle.
- Replace the hardcoded tile-origin `y_mode_index` § 8.3.2 context with the real
  neighbour-derived context: add a per-MI `IntraJointModes` grid
  (`TileIntraJointModeState`) threaded through the general intra partition walk,
  compute `ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) +
  (get_joint_mode(1) >= NON_DIRECTIONAL_MODES_COUNT)` (§ 8.3.2 / § 5.20.5.3
  `get_joint_mode`) before reading any `y_mode_set` / `y_mode_index` symbol, and
  REJECT the unverified `ctx != 0` (directional-neighbour) case with a structured
  `decode/unsupported-feature` diagnostic instead of misdecoding with the wrong
  CDF. This fixes the codex P2 on PR #385 and the latent #383 case (a DC block
  below/right of a `D135` block now rejects rather than misdecodes). No
  `ctx == 1` oracle fixture is possible: the minimal-tool avmenc couples a `D135`
  luma block to `uv_mode == 0` chroma that splot rejects, so a multi-block frame
  whose `D135` block splot decodes past is not encoder-producible.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-mb-neighbour-smooth`: Crate-private multi-block non-DC
  (§ 7.13.2.13 SMOOTH_V/SMOOTH_H) luma intra prediction over the § 7.13.2.1
  edges read from a REAL reconstructed neighbour of an already-decoded block,
  plus residual.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra multi-block non-DC luma decode over a reconstructed neighbour edge.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`, the new
  `crates/splot-decode/src/tile_payload/intra_joint_modes.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_block.rs`,
  `crates/splot-decode/src/tile_payload/partition_traversal.rs`,
  `crates/splot-decode/src/tile_payload/runtime_frontier.rs`, and
  `crates/splot-decode/src/tile_payload/cdf/block_context.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and generated status docs.
- No public API, dependency graph, encoder, or validator changes.
  Neighbour-having § 7.13.2.8 directional luma over a real non-flat edge (real
  IDIF 4-tap interpolation), the in-frame directional-neighbour `y_mode_index`
  reorder and the `ctx != 0` `y_mode_index` decode itself (the per-MI
  `IntraJointModes` grid and the `ctx != 0` reject now land; only the unverified
  `ctx != 0` decode is deferred to DECODE-GENERAL-INTRA-ANGLE), sub-superblock
  non-DC blocks, SMOOTH / PAETH luma, non-DC chroma neighbour edges beyond the
  existing SMOOTH chroma path, multiple tiles, inter prediction, in-loop filters,
  and live in-CI AVM/dav2d remain out of scope.
