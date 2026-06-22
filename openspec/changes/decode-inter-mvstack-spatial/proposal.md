## Why

The inter decoder decodes exactly one 64x64 inter block per frame
(`DECODE-FIRST-INTER-FRAME-FRONTIER` zero-MV copy, `DECODE-INTER-SUBPEL-MV`
sub-pel, `DECODE-INTER-RESIDUAL-DCT` skip=0 residual). Real content needs
MULTI-BLOCK inter frames where a later block's motion vector is predicted from a
decoded neighbour block — the inter analog of the intra partition-depth unlock.
The AV2 entry point is `find_mv_stack` (§ 7.12.2), driven by the § 7.11.2 mode
context and the § 5.20.7.2 neighbour-buffer contexts.

The smallest bit-exact-verifiable step is a two-frame stream whose 64x64 inter
superblock is split into four 32x32 single-reference inter blocks: block 0 is
NEWMV (a non-zero MV) and the later three are NEARMV that must predict block 0's
MV from the spatial-neighbour MV stack. Both oracles agree byte-for-byte.

## What Changes

- Add Feature ID `DECODE-INTER-MVSTACK-SPATIAL`.
- Add the spatial single-prediction `find_mv_stack` / `find_mode_ctx` /
  neighbour-context kernel (`find_mv_stack.rs`): the § 7.11.2 mode context, the
  § 5.20.7.2 neighbour-buffer `is_inter` / `skip_flag` contexts, and the
  § 7.12.2 spatial scan-point MV stack (steps 7–15, extra-search global fallback,
  clamp), with a per-MI neighbour MV grid. Unit-tested against the fixture's
  worked example.
- Wire it into the per-leaf inter block decode: lift the single-64x64 gate, walk
  every § 5.20.3 leaf inter block, derive its contexts from the already-decoded
  neighbours, read mode_info, resolve the § 5.20.7.13 assign_mv MV from the
  DRL-selected stack candidate, record the block into the grid, and § 7.13.3.18
  motion-compensate each block at its own luma-space rectangle.
- Add the project-owned `syn-2frame-inter-mvstack-64x64.ivf` fixture (frame 0 =
  DC_PRED intra key; frame 1 = 64x64 SPLIT into four 32x32 inter blocks, block 0
  NEWMV + three NEARMV, all skip=1). Prove avmdec `--rawvideo --i420` and dav2d
  `--demuxer ivf` agree byte-for-byte (md5 `e5b581a55433785c0071b635d5642083`).
- Register the fixture in the conformance manifest (`expect = "clean"`) and add
  the reciprocal LOCAL-REFERENCE-EVIDENCE entry.
- Add decode tests pinning the bit-exact output and the kernel's worked example.

## Capabilities

### New Capabilities
- `decode-inter-mvstack-spatial`: A multi-block inter frame decodes bit-exact,
  with a later block's motion vector predicted from a decoded neighbour block via
  the § 7.11/§ 7.12 spatial MV-context + MV-stack processes.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row.

## Impact

- Adds `tests/conformance/vectors/valid/syn-2frame-inter-mvstack-64x64.ivf`, the
  `find_mv_stack.rs` kernel module + tests, and decode tests in
  `crates/splot-decode/src/runtime_minimal/inter/tests.rs` and
  `crates/splot-cli/tests/decode_cli.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and the
  generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. Temporal /
  compound / warp / ref-MV-bank / derived-SMVP / DRL-reorder candidates, the
  § 7.12.2.5 scan-col wider reach, the large-block extra MVP combinations, and a
  multi-block skip == 0 residual remain out of scope (deferred with explicit
  spec TODOs, all gated absent before any output).
