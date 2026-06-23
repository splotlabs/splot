## ADDED Requirements

### Requirement: General intra multi-block non-DC luma neighbour-edge support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH` as a
distinct partial `splot-decode` row named `general-intra-mb-neighbour-smooth`. The row
SHALL cite AV2 § 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.13, § 8.2.4, and
§ 9.2, SHALL record the multi-block SMOOTH_V neighbour-edge oracle test, SHALL
carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the multi-block
vertical-gradient fixture, and SHALL keep neighbour-having directional luma,
sub-superblock non-DC blocks, the in-frame directional-neighbour `y_mode_index`
reorder, non-DC chroma neighbour edges, and non-64x64-superblock non-DC blocks out
of scope.

#### Scenario: Matrix records narrow multi-block non-DC neighbour-edge support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-mb-neighbour-smooth` appears with Feature ID
  `DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim neighbour-having directional luma, sub-superblock
  non-DC blocks, the in-frame directional-neighbour reorder, non-DC chroma
  neighbour edges, or non-64x64-superblock non-DC blocks
