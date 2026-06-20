## ADDED Requirements

### Requirement: General intra single-row-or-column multi-superblock support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-MULTIROW` as a
distinct partial `splot-decode` row named `general-intra-multirow`. The row SHALL
cite AV2 § 5.18.3, § 5.20.2.1, § 5.20.4.1, and § 7.13.2, SHALL record the
single-column 64x128 oracle test plus the single-row regression test, SHALL carry
the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the 64x128 fixture, and SHALL
keep 2-D grid frames, directional luma, SMOOTH chroma on sub-partitioned blocks,
partial frames, and multiple tiles out of scope.

#### Scenario: Matrix records the single-row-or-column multi-superblock support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-multirow` appears with Feature ID
  `DECODE-GENERAL-INTRA-MULTIROW`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim 2-D grid frames, directional luma, sub-partitioned
  SMOOTH chroma, or multiple tiles
