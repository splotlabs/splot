## ADDED Requirements

### Requirement: General intra single-block directional-angle luma support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-ANGLE` as a distinct
partial `splot-decode` row named `general-intra-angle`. The row SHALL cite AV2
§ 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.8, § 8.2.4, and § 9.2, SHALL record
the directional oracle test and the `y_mode_offset` escape reconstruction test,
SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the hedge
directional fixture, and SHALL keep non-zero angle deltas, the other directional
modes, multi-block directional prediction, directional chroma, and non-64x64
frames out of scope.

#### Scenario: Matrix records narrow directional-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-angle` appears with Feature ID
  `DECODE-GENERAL-INTRA-ANGLE`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim non-zero angle deltas, the other directional modes,
  multi-block directional prediction, directional chroma, or non-64x64 frames
