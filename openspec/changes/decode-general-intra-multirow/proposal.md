## Why

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The general intra decode admits a single superblock row (width a multiple of 64,
height exactly 64). The §5.20.2.1 superblock raster loop already iterates rows and
columns and the DC path already reads above neighbours, so a single COLUMN of
superblocks (height a multiple of 64, width 64) reconstructs correctly too — it
just lacked an oracle fixture and an admission widening. A full 2-D grid is NOT
yet safe: a non-rightmost row>0 superblock has a decoded above-right neighbour
that the §7.13.2.13 SMOOTH chroma sentinel does not yet read.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-MULTIROW`.
- Relax `is_general_minimal_intra` to accept frames whose width and height are
  positive multiples of 64 AND (width == 64 OR height == 64) — a single row or
  single column of 64x64 superblocks. 2-D grid frames remain rejected.
- Add the project-owned `syn-2sbcol-intra-64x128-q80.ivf` fixture (two vertically
  stacked 64x64 DC superblocks) and prove it decodes bit-exactly to the
  avmdec/dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-multirow`: Crate-private single-row-or-column
  multi-superblock general intra decode, with the second-row superblock
  DC-predicting from the reconstructed above neighbour and full-superblock SMOOTH
  chroma where no above-right neighbour is decoded.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra single-row-or-column multi-superblock decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs` (the admission size gate).
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No public API, dependency graph, encoder, or validator changes. 2-D grid frames
  (needing the §7.13.2.1 above-right sentinel), directional luma, SMOOTH chroma on
  sub-partitioned blocks, partial (non-multiple-of-64) frames, multiple tiles,
  inter prediction, and in-loop filters remain out of scope.
