## Why

`DECODE-TILE-CDF-SELECTION-BOUNDARY` now derives left/above-neighbor contexts
for four partition-entry CDF families, but `do_square_split` still accepts only
caller-supplied contexts. Adding the remaining § 8.3.2 square-split derivation
keeps the tile CDF boundary moving toward real `read_partition()` wiring while
preserving the current unsupported runtime decode boundary.

## What Changes

- Add crate-private AV2 § 8.3.2 `do_square_split` context derivation to the
  existing `splot-decode` tile CDF context module.
- Accept bounded `MiSizes`, `AvailU`, and `AvailL` facts for the square-split
  formula in addition to the existing `bSize`, `PlaneStart`, `r`, and `c`
  coordinates.
- Return the existing `TileCdfSelector::DoSquareSplit` selector and preserve the
  existing `TileDoSquareSplitCdf[0][ctx]` row bounds.
- Add math, bounds, and selector-row handoff tests.
- Update decoder roadmap, support matrix/status, implementation matrix, and
  decoder-support OpenSpec text.
- Keep `DECODE-TILE-CDF-SELECTION-BOUNDARY` partial.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: extend the existing Tile CDF selection boundary
  requirement to include bounded `do_square_split` context derivation.

## Impact

- Feature ID: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
- Code: `crates/splot-decode/src/tile_payload/cdf.rs` and
  `crates/splot-decode/src/tile_payload/cdf/context.rs`.
- Docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support/status docs,
  and `docs/IMPLEMENTATION-MATRIX.toml`.
- No new public API, dependency, scheduler, CLI, reconstruction, output,
  AVM/dav2d, or runtime decode support.
