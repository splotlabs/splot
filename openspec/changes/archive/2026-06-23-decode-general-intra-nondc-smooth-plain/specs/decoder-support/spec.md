## ADDED Requirements

### Requirement: General intra single-block plain SMOOTH luma support row
The decoder support model SHALL track
`DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH-PLAIN` as a distinct partial
`splot-decode` row named `general-intra-nondc-luma-smooth-plain`. The row SHALL
cite AV2 § 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.13, § 8.2.4, and § 9.2,
SHALL record the plain-SMOOTH oracle test and the non-DC-chroma rejection test,
SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointers for the positive
2-D smooth fixture and the negative non-DC-chroma fixture, and SHALL keep
neighbour-having plain SMOOTH, sub-64x64 plain SMOOTH, plain SMOOTH chroma,
PAETH, and non-64x64 frames out of scope.

#### Scenario: Matrix records narrow plain SMOOTH support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-nondc-luma-smooth-plain` appears with Feature ID
  `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH-PLAIN`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim neighbour-having plain SMOOTH, sub-64x64 plain
  SMOOTH, plain SMOOTH chroma, or non-64x64 frames
