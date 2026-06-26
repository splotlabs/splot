## ADDED Requirements

### Requirement: General intra § 7.17 deblocking-filter decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-DEBLOCK` as a
distinct partial `splot-decode` row named `general-intra-deblock`. The row SHALL
cite AV2 § 7.17.1, § 7.17.2, § 7.17.5, § 7.17.6, and § 9.2, SHALL record the
deblock-active intra oracle tests, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointers for the
`syn-2sb-deblock-intra-128x64-q100.ivf`,
`syn-2sb-deblock-intra-128x64-q98.ivf`, and
`syn-2sb-deblockwide-intra-128x64-q100.ivf` fixtures, and SHALL keep nonzero
`df_delta_q`, 10-bit deblock, sub-PU edges, segmentation / lossless segments,
multiple tiles, the other in-loop filters, inter frames, and public APIs out of
scope.

#### Scenario: Matrix records general intra deblocking support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-deblock` appears with Feature ID
  `DECODE-GENERAL-INTRA-DEBLOCK`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim nonzero `df_delta_q`, 10-bit deblock, the other
  in-loop filters, or inter frames

#### Scenario: Coverage tracks the new deblocking-filter decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the coverage includes row `general-intra-deblock` and Feature ID
  `DECODE-GENERAL-INTRA-DEBLOCK`
- **AND** broader in-loop-filter coverage remains partial
