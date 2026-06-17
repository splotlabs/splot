## Why

The minimal flat-intra block-symbol trace currently selects every block-symbol
CDF row with a hardcoded `ctx` literal. The § 8.3.2 Cdf selection process derives
those contexts from decoded/neighbour state. Replacing the first such literal —
`y_mode_index` — with its spec-grounded derivation is the smallest honest step
into § 8.3.2 block-symbol context derivation on the decoder frontier, and is
provably no-output-change for the single-block tile-origin fixture.

## What Changes

- Add `crates/splot-decode/src/tile_payload/cdf/block_context.rs` with
  `YModeIndexContext`, deriving the § 8.3.2 `y_mode_index` context
  `ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) + (get_joint_mode(1)
  >= NON_DIRECTIONAL_MODES_COUNT)`.
- Model the single-block tile-origin case (`MiRow == MiCol == 0`), where both
  `get_joint_mode` neighbours are out of frame so each returns `DC_PRED`
  (ctx 0), via `YModeIndexContext::tile_origin_block()`.
- Thread the derived ctx into the minimal block-symbol trace
  (`block_symbol.rs::minimal_trace_items`) in place of the `YModeIndex { ctx: 0 }`
  literal; the existing no-output-change snapshot
  (`block_symbol_frontier_accepts_minimal_fixture_trace`) proves the derived value
  matches.
- Mark the in-frame `IntraJointModes` neighbour lookup deferred with a
  `TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY)` marker (the minimal frontier
  tracks no neighbour mode state yet).
- Update feature tracking, decoder support notes, and OpenSpec artifacts.

Non-goals:

- No `uv_mode`, `txb_skip`, or `v_txb_skip` context derivation (still literal),
  no `YMode` reconstruction, and no consume-trace sequential refactor.
- No in-frame `get_joint_mode` neighbour lookup, no neighbour mode-state
  tracking.
- No partition decisions, full § 8.3 CDF selection, `decode_tile()`,
  reconstruction, hashes, Y4M output, reference refresh, or runtime support
  changes.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records that the tile CDF selection boundary now derives the
  first § 8.3.2 block-symbol context (`y_mode_index`) instead of a literal, while
  the boundary remains partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf.rs`
- `crates/splot-decode/src/tile_payload/cdf/block_context.rs`
- `crates/splot-decode/src/tile_payload/block_symbol.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
