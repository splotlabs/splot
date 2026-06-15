# Agent Log

## Planning

- Orchestrator selected `tile-partition-context-derivation` as the next narrow
  expansion of `DECODE-TILE-CDF-SELECTION-BOUNDARY` after PR #152 merged
  `TileRectTypeCdf` support.
- Planning explorer `019ecb80-7b69-7c30-a7f9-3f290562ddfa`: PASS. Confirmed
  the coherent next slice is left/above-neighbor § 8.3.2 context derivation for
  `do_split`, `rect_type`, `do_ext_partition`, and
  `do_uneven_4way_partition`, with `do_square_split`, `read_partition()`, and
  `decode_tile()` explicitly out of scope.

## Implementation

- Added `crates/splot-decode/src/tile_payload/cdf/context.rs` as the
  crate-private § 8.3.2 context-derivation child module.
- Added bounded `PartitionContextInput` and `RectPartitionType` helpers.
- Derived existing `TileCdfSelector` values for `do_split`, `rect_type`,
  `do_ext_partition`, and `do_uneven_4way_partition` from `bSize`,
  `PlaneStart`, `r`/`c`, `LeftMiSizes`, `AboveMiSizes`, generated § 9.2
  conversion tables, and the local § 8.3.2 adjustment arrays.
- Added typed `TileCdfError` variants for invalid `bSize`, unavailable
  left/above slots, invalid neighbor block-size entries, second-neighbor index
  overflow, and impossible negative conversion-table values.
- Kept `do_square_split`, syntax reads, partition decisions,
  `read_partition()`, and `decode_tile()` out of scope.

## Testing

- `cargo test -p splot-decode tile_payload::cdf --locked`: PASS.
- `cargo test -p splot-decode tile_payload --locked`: PASS.
- `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`:
  PASS.
- `openspec validate tile-partition-context-derivation --strict`: PASS.
- `cargo xtask check-feature-status`: PASS.
- `cargo xtask check-decoder-support`: PASS.
- `cargo xtask check-decoder-conformance-coverage`: PASS.
- `cargo xtask ci`: PASS.
- `git diff --check`: PASS.
- `openspec archive tile-partition-context-derivation --yes`: PASS. Folded the
  delta into `openspec/specs/decoder-support/spec.md` and archived the change
  under `openspec/changes/archive/2026-06-15-tile-partition-context-derivation/`.
- Post-archive `openspec validate --all --no-interactive`: PASS.
- Post-archive `cargo xtask check-feature-status`: PASS.
- Post-archive `cargo xtask check-decoder-support`: PASS.
- Post-archive `cargo xtask check-decoder-conformance-coverage`: PASS.
- Post-archive `cargo xtask ci`: PASS.
- Added math tests for `do_split`, `rect_type`, horizontal ext, vertical ext,
  and uneven contexts.
- Added bounds tests for invalid `bSize`, invalid `PlaneStart`, missing
  left-neighbor slot, second-half above-neighbor offset, and invalid neighbor
  block-size entry.
- Added selector-row handoff tests proving derived selectors index generated
  default CDF rows.

## Documentation

- Updated `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, and
  `docs/IMPLEMENTATION-MATRIX.toml` to record the new left/above-derived
  contexts while keeping the feature partial.
- Regenerated `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.
- Confirmed the changed file list contains no AVM/dav2d source, snippets,
  binaries, submodules, dependencies, wrappers, scripts, CI jobs, required
  `xtask` commands, or mandatory tests.

## Reviews

- Correctness/spec review `019ecb95-6077-7b03-b3ef-6cebfaf5f2fb`: PASS.
  Nested conclusions: spec exactness PASS, reference evidence/source risk PASS,
  edge cases/bounds PASS. Non-blocking wording note fixed by changing active
  spec delta `decode_partition()` wording to `read_partition()`.
- Safety review `019ecb95-644d-7a02-9a04-28d0b58f266d`: PASS. Nested
  conclusions: arithmetic/allocation PASS, panic-free arbitrary bytes PASS,
  output-file atomicity PASS/not relevant.
- Performance/data-layout review `019ecb95-6971-7b82-885f-9e088c78577f`:
  PASS. Nested conclusions: data layout PASS, thread determinism PASS,
  avoidable copies PASS. Non-blocking note accepted: selector derivation and row
  access both validate bounds to preserve the existing safe boundary.
