## ADDED Requirements

### Requirement: General intra cardinal directional support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-CARDINAL` as a distinct
partial `splot-decode` row named `general-intra-cardinal`. The row SHALL cite AV2
§ 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.7, § 7.13.2.8, § 8.2.4, and § 9.2,
SHALL record the cardinal V_PRED and H_PRED neighbour-having luma + follow chroma
oracle tests, SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointers for the
`syn-vpred-intra-64x128-q160.ivf` and `syn-hpred-intra-128x64-q180.ivf` fixtures,
and SHALL keep a first-superblock-row V_PRED / first-column H_PRED, a directional
NEIGHBOUR (`ctx != 0`) cardinal escape, sub-superblock cardinal blocks, non-cardinal
angles, non-zero angle deltas, the `y_second_mode` path, non-64x64 frames, and
multiple tiles out of scope.

#### Scenario: Matrix records the cardinal directional support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-cardinal` appears with Feature ID
  `DECODE-GENERAL-INTRA-CARDINAL`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim a first-superblock-row V_PRED, a directional-neighbour
  cardinal escape, non-cardinal angles, or multiple tiles
