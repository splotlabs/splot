## Why

After `y_mode_index`, the next hardcoded block-symbol context literal in the
minimal flat-intra trace is `uv_mode`. Its § 8.3.2 context is
`is_directional_mode(YMode)`, so deriving it requires reconstructing the typed
luma `YMode` from the decoded `y_mode_set` / `y_mode_index` and restructuring the
trace so a later symbol's context can depend on earlier decodes — the
foundational entropy-decode pattern. It is provably no-output-change for the
single-block flat-intra fixture (`YMode == DC_PRED`, non-directional, ctx 0).

## What Changes

- Extend `crates/splot-decode/src/tile_payload/cdf/block_context.rs` with an
  `IntraYMode` model (`is_directional` per § 5 `is_directional_mode`,
  `V_PRED..=D67_PRED`), `reconstruct_minimal_y_mode(y_mode_set, y_mode_index)`
  (§ 5 `intra_y_mode_info` / `get_intra_y_mode_set` / `Reordered_Y_Mode` for the
  supported `y_mode_set == 0` non-directional subset), and `uv_mode_ctx(YMode)`
  (§ 8.3.2 `is_directional_mode(YMode)`).
- Refactor `block_symbol.rs::consume_trace` from static-array iteration to a
  sequential decode (helper `decode_block_symbol`), reconstruct `YMode` after the
  `y_mode_set` / `y_mode_index` decodes, and select `uv_mode` with the derived
  context.
- Add a typed `MinimalBlockSymbolTraceError::UnsupportedYMode` for inputs outside
  the supported reconstruction subset (unreachable for the asserted trace, kept
  total/panic-free) and route it to a `decode/unsupported-feature` diagnostic.
- The existing no-output-change snapshot
  (`block_symbol_frontier_accepts_minimal_fixture_trace`) proves the derived
  contexts match the previous literals.
- Update feature tracking and OpenSpec artifacts.

Non-goals:

- No `txb_skip` / `v_txb_skip` context derivation (still literal), no in-frame
  `get_joint_mode` neighbour lookup, and no directional / `y_mode_offset` escape /
  `y_mode_set != 0` (`y_second_mode`) `YMode` reconstruction paths.
- No partition decisions, full § 8.3 CDF selection, `decode_tile()`,
  reconstruction, hashes, Y4M output, reference refresh, or runtime support
  changes.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records that the tile CDF selection boundary now derives the
  `uv_mode` block-symbol context (from the reconstructed luma `YMode`) and decodes
  the block-symbol trace sequentially, while the boundary remains partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/block_context.rs`
- `crates/splot-decode/src/tile_payload/block_symbol.rs`
- `crates/splot-decode/src/runtime_minimal.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
