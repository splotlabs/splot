## ADDED Requirements

### Requirement: General intra 2-D grid non-DC SMOOTH luma support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-GRID-NEIGHBOUR-NONDC`
as a distinct partial `splot-decode` row named
`general-intra-grid-neighbour-nondc`. The row SHALL cite AV2 § 5.20.2.3,
§ 5.20.5.3, § 5.20.7.25, § 5.20.7.27, § 7.13.2.1, and § 7.13.2.13, SHALL record
the 2-D grid 192x128 row > 0 SMOOTH_V luma oracle test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the 192x128 fixture, and SHALL keep
neighbour-having directional luma, the `ctx != 0` `y_mode_index` decode,
sub-superblock (split) non-DC blocks, and multiple tiles out of scope.

#### Scenario: Matrix records the 2-D grid non-DC SMOOTH luma support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-grid-neighbour-nondc` appears with Feature ID
  `DECODE-GENERAL-INTRA-GRID-NEIGHBOUR-NONDC`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim neighbour-having directional luma, sub-partitioned
  non-DC blocks, or multiple tiles
