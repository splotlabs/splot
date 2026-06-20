## Why

Header-bridge brick 4. The keystone (`build_minimal_intra_clk_core`) produces the
`(FrameHeaderCore, CoreSeqView)` pair, and the writer-bridge bricks 1–2 added the
`TileGroupStructure` / `TileGroupFraming` single-tile constructors. This connects them:
the first encoder writer-input end-point that drives `write_tile_group_obu` to emit a
§ 5.19 `tile_group_obu()` payload from coded tile bytes — the first time the bridge pieces
compose into framed OBU output.

## What Changes

- Add `ENC-MINIMAL-INTRA-TILE-GROUP-OBU` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `encode_minimal_intra_clk_tile_group_obu(tile_data: &[u8]) -> Result<Vec<u8>,
  MinimalIntraTileGroupError>`: builds the matched frozen-tier `(core, seq)` via
  `build_minimal_intra_clk_core`, frames `tile_data` as the single (last) tile of the first
  tile group (`TileGroupStructure::single_tile_first_group` /
  `TileGroupFraming::single_tile`), and drives `write_tile_group_obu` to return the
  `tile_group_obu()` payload (embedded frame header + § 5.20.1 tile framing + tile data) —
  **not** the § 5.2.2 OBU header / size wrapper.
- Add the `MinimalIntraTileGroupError` typed error (`Core` / `Write` arms).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input minimal-intra tile-group
  OBU payload assembler.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the function, the
  typed error, the tests), `crates/splot-core/src/headers/frame/mod.rs` (re-export).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new public function `encode_minimal_intra_clk_tile_group_obu` and
  one new public error `MinimalIntraTileGroupError`, both `splot-core`. No dependency-graph
  change.
- Validator/CLI impact: none.
