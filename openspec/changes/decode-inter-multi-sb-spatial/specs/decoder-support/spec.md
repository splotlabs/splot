## ADDED Requirements

### Requirement: Multi-superblock inter MV-stack support row
The decoder support model SHALL track `DECODE-INTER-MULTI-SB-SPATIAL` as a
distinct partial `splot-decode` row named `inter-multi-sb-spatial`. The row SHALL
cite AV2 § 5.18.3, § 5.20.2.1, § 5.20.3, § 5.20.7.6, § 7.11.2, § 7.12.2, and
§ 7.13.3.18, SHALL record the multi-superblock decode tests and the conformance
manifest test, and SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for
the multi-superblock inter fixture. The row SHALL keep a full 2-D superblock grid
(both dimensions greater than 64), a multi-superblock skip == 0 residual, and the
deferred temporal / compound / warp / ref-MV-bank / derived-SMVP / DRL-reorder MV
candidates out of scope as deferred work (all gated absent before any output).

#### Scenario: Matrix records narrow multi-superblock inter MV-stack support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-multi-sb-spatial` appears with Feature ID
  `DECODE-INTER-MULTI-SB-SPATIAL`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim a full 2-D superblock grid or a multi-superblock
  skip == 0 residual

#### Scenario: Multi-superblock inter decode is verified bit-exact
- **WHEN** the committed `syn-2sb-inter-128x64-q80.ivf` is decoded
- **THEN** `splot decode --output-format raw` reproduces the avmdec / dav2d raw
  output byte-for-byte (md5 `477a993d671e93d37b92a0d368c238ff`)
- **AND** the second-superblock NEARMV block reconstructs the first-superblock
  NEWMV block's motion vector from the § 7.12.2 spatial-neighbour MV stack across
  the superblock boundary
