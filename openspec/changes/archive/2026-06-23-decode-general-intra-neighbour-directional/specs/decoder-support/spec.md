## ADDED Requirements

### Requirement: General intra neighbour-having directional support row
The decoder support model SHALL track
`DECODE-GENERAL-INTRA-NEIGHBOUR-DIRECTIONAL` as a distinct partial `splot-decode`
row named `general-intra-neighbour-directional`. The row SHALL cite AV2
§ 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.8, § 8.2.4, and § 9.2, SHALL record
the multi-superblock neighbour-having directional D135 luma + follow chroma oracle
test, SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the
`syn-rdir-intra-128x64-q80.ivf` fixture, and SHALL keep a row>0 D135 block, a
directional NEIGHBOUR (`ctx != 0`) escape, sub-superblock directional blocks, other
directional angles, non-zero angle deltas, non-64x64 frames, and multiple tiles out
of scope.

#### Scenario: Matrix records the neighbour-having directional support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-neighbour-directional` appears with Feature ID
  `DECODE-GENERAL-INTRA-NEIGHBOUR-DIRECTIONAL`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim a row>0 D135 block, a directional-neighbour escape,
  other directional angles, or multiple tiles
