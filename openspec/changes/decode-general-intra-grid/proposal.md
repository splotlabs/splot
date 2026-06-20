## Why

The general intra decode admits a single superblock ROW or single COLUMN (width
or height exactly 64). The §5.20.2.1 superblock raster loop already iterates a
full 2-D grid, but a non-rightmost row>0 superblock has a decoded above-right
neighbour, and the §7.13.2.13 SMOOTH chroma top-right sentinel `AboveRow[w]` was
built by edge-clamping (repeat-last), which mispredicts a 2-D grid with non-uniform
chroma. Reading the real reconstructed above-right sample per §7.13.2.1 unblocks
the common real case: a full 2-D grid of superblocks.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-GRID`.
- Read the §7.13.2.1 SMOOTH chroma top-right sentinel `AboveRow[w]` from the real
  reconstructed above-right sample when it is decoded (`num4AboveRight > 0`),
  derived faithfully to §5.20.7.25 `count_top_right_avail` over the §5.20.2.3
  `BlockDecoded` state for the full-superblock case.
- Add a total/checked `CurrentFrameWorkspace::reconstructed_sample` accessor
  (splot-recon) to read an arbitrary already-reconstructed sample.
- Relax `is_general_minimal_intra` to accept frames whose width and height are
  both positive multiples of 64 (dropping the `width == 64 || height == 64`
  restriction) — a full 2-D grid.
- Add the project-owned `syn-grid-intra-128x128-q80.ivf` fixture (uniform luma;
  distinct flat chroma per quadrant with a SMOOTH bottom-left superblock whose
  above-right differs) and prove it decodes bit-exactly to the avmdec/dav2d
  oracle, where the old repeat-last sentinel mismatched.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-grid`: Crate-private full 2-D superblock grid general
  intra decode, with the §7.13.2.13 SMOOTH chroma top-right sentinel reading the
  real reconstructed above-right neighbour.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra full 2-D superblock grid decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs` (the admission size gate
  and the `num4AboveRight` derivation) and
  `crates/splot-decode/src/runtime_minimal_recon.rs` (the SMOOTH chroma sentinel
  read), plus `crates/splot-recon/src/workspace.rs` (the new
  `reconstructed_sample` accessor).
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. The `reconstructed_sample`
  accessor is the only new public surface. Directional luma, SMOOTH chroma on
  sub-partitioned blocks, partial (non-multiple-of-64) frames, multiple tiles,
  inter prediction, and in-loop filters remain out of scope.
