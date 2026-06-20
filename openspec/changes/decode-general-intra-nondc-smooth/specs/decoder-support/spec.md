## ADDED Requirements

### Requirement: General intra single-block non-DC luma smooth support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH` as
a distinct partial `splot-decode` row named `general-intra-nondc-luma-smooth`.
The row SHALL cite AV2 § 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.13, and
§ 8.2.4, SHALL record the two smooth oracle tests, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointers for the vertical- and horizontal-gradient
fixtures, and SHALL keep the remaining non-DC modes, directional modes,
multi-block non-DC prediction, non-DC chroma, and non-64x64 frames out of scope.

#### Scenario: Matrix records narrow non-DC smooth support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-nondc-luma-smooth` appears with Feature ID
  `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim multi-block non-DC prediction, directional modes,
  non-DC chroma, or non-64x64 frames
