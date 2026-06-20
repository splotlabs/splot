## Why

The general intra decode admits a single superblock row (width a multiple of
64, height exactly 64). The height restriction was a conservative guard while the
multi-superblock-row neighbour handling was unverified. The §5.20.2.1 superblock
raster loop already iterates rows and columns, the DC path already reads above
neighbours, and full-superblock SMOOTH chroma is row-independent, so a full grid
of 64x64 DC superblocks reconstructs correctly — it just lacked an oracle fixture.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-MULTIROW`.
- Relax `is_general_minimal_intra` to accept frames whose width and height are
  both positive multiples of 64 (a grid of 64x64 superblocks), not only height
  exactly 64.
- Add the project-owned `syn-uniform-intra-128x128-q80.ivf` fixture (a 2x2 grid
  of flat 64x64 DC superblocks) and prove it decodes bit-exactly to the
  avmdec/dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-multirow`: Crate-private multi-superblock-row (grid)
  general intra decode, with second-row superblocks DC-predicting from the
  reconstructed first-row neighbours and full-superblock SMOOTH chroma at any row.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra multi-superblock-row decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs` (the admission size gate).
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No public API, dependency graph, encoder, or validator changes. Distinct
  multi-row content (which prefers a directional luma mode), directional luma,
  SMOOTH chroma on sub-partitioned blocks, partial (non-multiple-of-64) frames,
  multiple tiles, inter prediction, and in-loop filters remain out of scope.
