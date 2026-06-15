# encoder-tools delta: frame-header-writer-tiling

## ADDED Requirements

### Requirement: frame-header tile_info writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.18.7.2 `tile_info()`
parser, reusing the shared § 5.18.7.3 `tile_params()` writer. For every `TileInfo` the parser
can produce, reparsing the written bits SHALL yield the original (`parse(write(x)) == x`),
byte-exactly. The writer SHALL never panic: a `TileInfo` the parser could not have produced
SHALL be rejected with a typed writer error before any bit is written.

To make the explicit-branch round-trip byte-exact, the model and parser MAY surface the
derived `TileParams` the modeled path otherwise discards (a maintainer-approved exception to
the additive / read-only-parser rule); the surfacing SHALL NOT change the bits read
(`consumed_bits` is unchanged).

#### Scenario: tile_info round-trips across the reuse, explicit, and bridge paths

- **WHEN** a parsed `tile_info()` is written with the same gating inputs and reparsed
- **THEN** the reparsed `TileInfo` SHALL equal the original, across the reuse-eligible /
  inferred-reuse / explicit (uniform and non-uniform) / bridge layouts and the multi-tile
  `context_update_tile_id` / `tile_size_bytes` tail (with and without the avg-CDF gate).

#### Scenario: a non-reproducible tile_info is rejected before any bit

- **WHEN** a `TileInfo` carries a layout that does not match the `reuse_tile_params()` /
  `tile_params()` re-derivation, an inferred `reuse_tile_info` that disagrees with its gate, a
  reserved-level layout, a gated-off non-zero `context_update_tile_id`, or a
  `tile_size_bytes` whose presence / range disagrees with the syntax
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
