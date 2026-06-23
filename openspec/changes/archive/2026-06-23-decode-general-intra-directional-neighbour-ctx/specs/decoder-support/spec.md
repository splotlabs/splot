## ADDED Requirements

### Requirement: General intra directional-neighbour y_mode_index support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-DIRECTIONAL-NEIGHBOUR-CTX`
as a distinct partial `splot-decode` row named
`general-intra-directional-neighbour-ctx`. The row SHALL cite AV2 § 5.20.5.3,
§ 5.20.5.5, § 5.20.7.27, § 7.13.2.13, and § 8.3.2, SHALL record the 128x64
directional-neighbour SMOOTH_V luma oracle test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the 128x64 fixture, and SHALL keep the
§ 5.20.5.3 in-frame directional-neighbour reorder, directional neighbour-reading
luma over a real non-flat edge, the `y_mode_set != 0` path, sub-superblock non-DC
blocks, and multiple tiles out of scope.

#### Scenario: Matrix records the directional-neighbour ctx support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-directional-neighbour-ctx` appears with Feature ID
  `DECODE-GENERAL-INTRA-DIRECTIONAL-NEIGHBOUR-CTX`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim the § 5.20.5.3 in-frame directional-neighbour reorder,
  directional neighbour-reading luma, or multiple tiles
