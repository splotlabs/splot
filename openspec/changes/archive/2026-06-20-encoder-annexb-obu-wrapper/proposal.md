## Why

Header-bridge brick 5. Brick 4 emits the bare § 5.19 `tile_group_obu()` payload, which has
no length framing and so cannot stand alone. This wraps it in AV2 Annex B framing (§ B.2) —
a `leb128` size prefix, the § 5.2.2 OBU header, the payload — producing a **self-delimiting**
OBU that reparses cleanly: the first time the encoder emits a complete OBU unit.

## What Changes

- Add `ENC-MINIMAL-INTRA-CLK-ANNEXB-OBU` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `encode_minimal_intra_clk_annexb_obu(tile_data: &[u8]) -> Result<Vec<u8>,
  MinimalIntraTileGroupError>`: assemble the brick-4 `tile_group_obu()` payload, build the
  no-extension `OBU_CLOSED_LOOP_KEY` § 5.2.2 header (inferred layer ids `0`), and drive
  `write_annexb_obu` to return the Annex B OBU (size prefix + header + payload).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input minimal-intra Annex B OBU
  wrapper.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the function, two
  tests), `crates/splot-core/src/headers/frame/mod.rs` (re-export).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new public function `encode_minimal_intra_clk_annexb_obu`
  (reusing the existing `MinimalIntraTileGroupError`), `splot-core`. No dependency-graph
  change.
- Validator/CLI impact: none.
