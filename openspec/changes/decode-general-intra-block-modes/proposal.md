## Why

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The general intra decode path (`DECODE-GENERAL-INTRA-FRAME-FRONTIER`) reaches
the AV2 § 5.20.3.1 single-block partition frontier and then stops. The next step
toward decoding a real AVM-generated minimal-tool intra frame is to decode the
§ 5.20.5.3 `intra_frame_mode_info` mode symbols — the frozen minimal-tier trace
hardcodes and asserts those values and reads them in a non-spec order, so the
general path needs its own spec-order, non-asserting mode decode.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-BLOCK-MODES`.
- Add a crate-private `decode_general_intra_block_modes` that decodes the
  AV2 § 5.20.5.3 mode symbols in spec order: `read_intra_y_mode`
  (`y_mode_set`, then `y_mode_index` with the § 8.3.2 tile-origin context),
  reconstructing the typed non-directional luma `YMode`, then
  `read_intra_uv_mode` (`uv_mode` with the § 8.3.2 `is_directional_mode(YMode)`
  context plus the `uv_mode == CHROMA_MODE_COUNT - 1` `uv_mode_idx` escape).
- Wire it into the general intra frame path after the partition frontier,
  advancing the structured rejection from the partition frontier
  (`general_intra_block_decode_unimplemented`) to the residual step
  (`general_intra_residual_decode_unimplemented`).
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.
- Add a unit test (DC luma mode + a chroma mode in spec order) and update the
  CLI test to assert the general intra fixture now decodes modes and reaches the
  residual step.
- Update decoder tracking, roadmap, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-block-modes`: Crate-private spec-order decode of the
  AV2 § 5.20.5.3 intra block mode-info symbols for the general intra path.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra block mode-info decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/general_intra_block.rs` (new),
  `crates/splot-decode/src/tile_payload.rs`,
  `crates/splot-decode/src/runtime_minimal.rs`, and
  `crates/splot-cli/tests/decode_cli.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, `docs/DECODER-ROADMAP.md`, and
  generated status/coverage docs.
- No public API, dependency graph, encoder, validator, typed `UVMode`
  reconstruction, coefficient decode, dequantization, inverse transform,
  residual add, reconstruction, output, reference-refresh, or in-repo AVM/dav2d
  integration changes are in scope.
