## Why

Toward a decodable temporal unit. The decoder's minimal-tier IVF frame requires its OBUs in
order: `OBU_TEMPORAL_DELIMITER`, `OBU_SEQUENCE_HEADER`, then the frame OBU. The frame OBU is
done (brick 5). This adds the first of the two missing OBUs — the temporal delimiter, the
simplest (an empty payload).

## What Changes

- Add `ENC-TEMPORAL-DELIMITER-OBU` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `encode_temporal_delimiter_obu() -> Result<Vec<u8>, WriteError>`: the no-extension
  `OBU_TEMPORAL_DELIMITER` § 5.2.2 header (inferred `obu_mlayer_id == 0`, global
  `obu_xlayer_id == 31`) with an empty payload, in Annex B framing (§ B.2) via
  `write_annexb_obu` — the canonical two bytes `[0x01, 0x08]`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder temporal-delimiter OBU primitive.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the function, one
  test), `crates/splot-core/src/headers/frame/mod.rs` (re-export).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new public function `encode_temporal_delimiter_obu`, `splot-core`.
  No dependency-graph change.
- Validator/CLI impact: none.
