## ADDED Requirements

### Requirement: General intra row>0 directional corner support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER` as
a distinct partial `splot-decode` row named `general-intra-directional-corner`. The
row SHALL cite AV2 § 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.7, § 7.13.2.8,
and § 8.2.4, SHALL record the row>0 directional D135 luma + follow chroma oracle
test reading the real § 7.13.2.1 corner, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the `syn-d135row-intra-128x128-q80.ivf`
fixture, and SHALL keep the row>0 first-column (`!haveLeft && haveAbove`) position,
any row>0 D157, sub-superblock directional blocks, non-zero angle deltas, the
directional-neighbour (`ctx != 0`) escape reorder, non-64x64 superblock blocks, and
multiple tiles out of scope.

#### Scenario: Matrix records the row>0 directional corner support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-directional-corner` appears with Feature ID
  `DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim a row>0 first-column position, a row>0 D157 block,
  sub-superblock directional blocks, non-zero angle deltas, or multiple tiles
