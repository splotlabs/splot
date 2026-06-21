## ADDED Requirements

### Requirement: General intra directional-follow chroma support row
The decoder support model SHALL track
`DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA` as a distinct partial
`splot-decode` row named `general-intra-directional-follow-chroma`. The row SHALL
cite AV2 § 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.8, § 8.2.4, and § 9.2,
SHALL record the single-block 64x64 directional-follow D135 chroma oracle test,
SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the
`syn-dfchroma-intra-64x64-q80.ivf` fixture, and SHALL keep neighbour-having
directional chroma, other directional chroma angles, the non-follow `D135_PRED`
scan pairing, CfL/CCTX/MHCCP chroma, sub-superblock chroma, and multiple tiles out
of scope.

#### Scenario: Matrix records the directional-follow chroma support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-directional-follow-chroma` appears with Feature ID
  `DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim neighbour-having directional chroma, CfL/CCTX/MHCCP
  chroma, or multiple tiles
