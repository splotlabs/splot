## ADDED Requirements

### Requirement: General intra block mode-info decode
The decoder SHALL provide a crate-private general intra block mode-info decode
that reads the AV2 § 5.20.5.3 `intra_frame_mode_info` mode symbols in spec order
for a minimal-tool intra block — `read_intra_y_mode` (`y_mode_set`, then
`y_mode_index` with the § 8.3.2 tile-origin context) reconstructing the typed
non-directional luma `YMode`, then `read_intra_uv_mode` (`uv_mode` with the
§ 8.3.2 `is_directional_mode(YMode)` context plus the
`uv_mode == CHROMA_MODE_COUNT - 1` `uv_mode_idx` escape) — without the frozen
minimal-tier trace's value assertions. It SHALL be wired into the general intra
frame path after the § 5.20.3.1 partition frontier so the structured
`decode/unsupported-feature` rejection advances to the residual step. It SHALL
NOT reconstruct the typed `UVMode`, decode the residual or transform-block
syntax, read coefficient symbols, write `Quant`, dequantize, inverse transform,
add residuals, reconstruct pixels, or invoke AVM or dav2d.

#### Scenario: General intra fixture decodes mode info and reaches the residual step
- **WHEN** `splot decode` is given the committed minimal-tool intra key frame
  `syn-flat-intra-64x64-q80.ivf`
- **THEN** the general intra path decodes the § 5.20.5.3 `y_mode_set`,
  `y_mode_index`, and `uv_mode` symbols after the partition frontier
- **AND** it emits a `decode/unsupported-feature` diagnostic with reason
  `general_intra_residual_decode_unimplemented`

#### Scenario: Non-directional luma mode reconstructs in spec order
- **WHEN** the mode decode reads `y_mode_set == 0` and `y_mode_index == 0`
- **THEN** the reconstructed luma `YMode` is `DC_PRED`
- **AND** the `uv_mode` is decoded with the `is_directional_mode(DC_PRED) == 0`
  context and yields a valid chroma-mode-list index

#### Scenario: base_q_idx == 255 frames route to the frozen tier, not the general path
- **WHEN** `splot decode` is given an intra key frame with `base_q_idx == 255`
- **THEN** the general intra mode decode does not run for that frame; it routes
  to the frozen minimal hash tier
- **AND** the committed `syn-flat-intra-64x64-minimal.ivf` fixture is no longer a
  `base_q_idx == 255` frame: change `decode-minimal-fixture-avm-skip-polarity`
  replaced it with the AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream
  that routes through the general intra path
