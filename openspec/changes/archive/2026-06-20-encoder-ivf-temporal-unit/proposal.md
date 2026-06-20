## Why

The integration finale's first half. The encoder now emits all three OBUs the decoder's
minimal-tier IVF frame requires — the temporal delimiter (brick 6), the sequence header
(byte-exact to the conformance vector), and the frame OBU (brick 5). This assembles them
into one IVF temporal unit, the first complete container the encoder produces.

## What Changes

- Add `ENC-MINIMAL-INTRA-IVF` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `encode_minimal_intra_clk_ivf(tile_data: &[u8]) -> Result<Vec<u8>,
  MinimalIntraIvfError>`: concatenate the temporal-delimiter, sequence-header, and frame
  Annex B OBUs into one AV2 temporal unit (in the decoder-required order) and wrap it in one
  `AV02` 64x64 IVF frame (`write_ivf_header` / `write_ivf_frame`). The sequence header and the
  frame header are consistent — both describe the frozen 64x64 single-picture `Block64x64`
  tier (verified by comparing `CoreSeqView::from_sequence` of the sequence header to the view
  the frame header is built against).
- Add the `MinimalIntraIvfError` typed error (`SequenceHeader` / `Frame` / `Write` / `Ivf`
  arms).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder minimal-intra IVF temporal-unit
  assembler.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the function, the
  typed error, two tests), `crates/splot-core/src/headers/frame/mod.rs` (re-exports).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new public function and one new public error, `splot-core`. No
  dependency-graph change.
- Validator/CLI impact: none.
