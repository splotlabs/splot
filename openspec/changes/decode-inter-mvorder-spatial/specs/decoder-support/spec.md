## ADDED Requirements

### Requirement: distinct-neighbour-MV stack-ordering support row
The decoder support model SHALL track `DECODE-INTER-MVORDER-SPATIAL` as a distinct
partial `splot-decode` row named `inter-mvorder-spatial`. The row SHALL cite AV2
§ 5.20.3, § 5.20.7.6, § 5.20.7.8, § 7.11.2, § 7.12.2, § 7.12.2.6, § 7.12.2.12, and
§ 7.13.3.18, SHALL record the distinct-MV decode tests, the `find_mv_stack`
ordering unit test, and the conformance manifest test, and SHALL carry the
reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the distinct-MV inter fixture. The
row SHALL keep the § 7.12.2.20 large-block (> 32x32) MVP combinations, the deferred
temporal / compound / warp / ref-MV-bank / derived-SMVP / DRL-reorder / scan-col MV
candidates, and a multi-superblock skip == 0 residual out of scope as deferred work
(all gated absent before any output).

#### Scenario: Matrix records narrow distinct-MV stack-ordering support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-mvorder-spatial` appears with Feature ID
  `DECODE-INTER-MVORDER-SPATIAL`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim the § 7.12.2.20 large-block MVP combinations or a
  multi-superblock skip == 0 residual

#### Scenario: distinct-MV stack ordering is verified bit-exact
- **WHEN** the committed `syn-2frame-inter-mvorder-64x64.ivf` is decoded
- **THEN** `splot decode --output-format raw` reproduces the avmdec / dav2d raw
  output byte-for-byte (md5 `284e1450b42180f02de7415ab0367bfe`)
- **AND** an interior NEARMV block whose left and above neighbours hold DIFFERENT
  motion vectors reconstructs the slot-1 (above) candidate from the § 7.12.2
  spatial-neighbour MV stack, pinning the left-before-above ordering
