## ADDED Requirements

### Requirement: 2-D-grid inter MV-stack support row
The decoder support model SHALL track `DECODE-INTER-GRID-SPATIAL` as a distinct
partial `splot-decode` row named `inter-grid-spatial`. The row SHALL cite AV2
§ 5.18.3, § 5.20.2.1, § 5.20.3, § 5.20.7.6, § 7.11.2, § 7.12.2, § 7.12.2.6, and
§ 7.13.3.18, SHALL record the 2-D-grid decode tests, the `find_mv_stack`
availability unit tests, and the conformance manifest test, and SHALL carry the
reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the 2-D-grid inter fixture. The
row SHALL keep a partial (non-multiple-of-64) frame size, a multi-superblock
skip == 0 residual, and the deferred temporal / compound / warp / ref-MV-bank /
derived-SMVP / DRL-reorder MV candidates out of scope as deferred work (all gated
absent before any output).

#### Scenario: Matrix records narrow 2-D-grid inter MV-stack support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-grid-spatial` appears with Feature ID
  `DECODE-INTER-GRID-SPATIAL`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim a partial (non-multiple-of-64) frame size or a
  multi-superblock skip == 0 residual

#### Scenario: 2-D-grid inter decode is verified bit-exact
- **WHEN** the committed `syn-grid-inter-128x128-q80.ivf` is decoded
- **THEN** `splot decode --output-format raw` reproduces the avmdec / dav2d raw
  output byte-for-byte (md5 `897bf67e72ec04cb7275fae08eab700c`)
- **AND** a second-superblock-row NEARMV block reconstructs the first-row NEWMV
  block's motion vector from the § 7.12.2 spatial-neighbour MV stack across the
  superblock-row boundary
