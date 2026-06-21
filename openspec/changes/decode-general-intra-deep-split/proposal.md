## Why

The general intra decode path proves a single split: the `syn-quad` fixture
splits a 64x64 superblock once into four 32x32 DC blocks. Real AV2 intra frames
nest partitions much deeper, so the verified subset must be pushed to a deeper
partition tree. The smallest verifiable step is a two-level square SPLIT where a
sub-32x32 16x16 leaf DC-predicts from a reconstructed sibling 16x16 neighbour
INSIDE the parent 32x32 sub-block.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-DEEP-SPLIT`.
- Prove the general intra decode handles a partition tree two levels deep: the
  64x64 superblock SPLITs into four 32x32 quadrants and one 32x32 SPLITs AGAIN
  into four square 16x16 DC_PRED leaves (the other three quadrants stay 32x32
  DC_PRED) — one level deeper than `syn-quad`.
- No production code change: the § 5.20.4.1 partition recursion
  (`child_calls` PARTITION_SPLIT) already pushes the 16x16 children depth-first,
  and the § 7.13.2.10 DC predictor reads its in-frame left column / above row from
  the persistent workspace. The § 5.20.2.3 left/above availability for the DC
  predictor is frame-position-based (`x == 0` / `y == 0`), so a sub-32x32 16x16
  leaf correctly DC-predicts from the already-reconstructed sibling neighbour
  with NO § 5.20.2.3 `BlockDecoded` flag state required (the DC predictor never
  reads the above-right / below-left sentinels that `BlockDecoded` gates). The
  deeper square DC split is bit-identical to the verified shared
  `reconstruct_general_intra_block_into` / partition-walk code, just at a smaller
  transform log2 and one recursion level deeper.
- Add the project-owned `syn-deep-intra-64x64-q120.ivf` fixture and prove it
  decodes bit-exactly to the avmdec/dav2d oracle.
- Add a decode test pinning the per-block DC values and the frame hash; confirm
  all existing general intra fixtures still decode bit-exact and a non-DC /
  rectangular-leaf deeper split still rejects.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-deep-split`: A two-level square SPLIT partition decode
  where a sub-32x32 16x16 DC leaf predicts from its reconstructed sibling
  neighbour inside the parent 32x32 sub-block.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra deeper (sub-32x32) square SPLIT decode.

## Impact

- Adds `tests/conformance/vectors/valid/syn-deep-intra-64x64-q120.ivf` and a
  decode test in
  `crates/splot-decode/src/runtime_minimal/general_intra_tests.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and generated status/coverage
  docs.
- No production decode code, public API, dependency graph, encoder, or validator
  changes. The § 5.20.2.3 per-block `BlockDecoded` flag state itself (only needed
  for non-DC / SMOOTH sub-superblock blocks reading the § 7.13.2.1 above-right /
  below-left sentinels), rectangular-leaf and non-DC sub-32x32 partitions,
  non-64x64 frames, inter prediction, in-loop filters, and live in-CI AVM/dav2d
  remain out of scope.
