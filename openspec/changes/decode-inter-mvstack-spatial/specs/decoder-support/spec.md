## ADDED Requirements

### Requirement: Multi-block inter MV stack support row
The decoder support model SHALL track `DECODE-INTER-MVSTACK-SPATIAL` as a distinct
partial `splot-decode` row named `inter-mvstack-spatial`. The row SHALL cite AV2
§ 5.20.3, § 5.20.7.2, § 5.20.7.6, § 5.20.7.8, § 7.11.2, § 7.11.3, § 7.12.2, and
§ 7.13.3.18, SHALL record the kernel worked-example tests plus the multi-block
decode tests and the conformance manifest test, and SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the multi-block inter fixture. The row SHALL
keep temporal / compound / warp / ref-MV-bank / derived-SMVP / DRL-reorder
candidates, the § 7.12.2.5 scan-col wider reach, the large-block extra MVP
combinations, and a multi-block skip == 0 residual out of scope as deferred work
(all gated absent before any output).

#### Scenario: Matrix records narrow multi-block inter MV-stack support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-mvstack-spatial` appears with Feature ID
  `DECODE-INTER-MVSTACK-SPATIAL`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim temporal / compound / warp MV candidates, the ref-MV
  bank, the DRL reorder sort, or a multi-block skip == 0 residual

#### Scenario: Multi-block inter decode is verified bit-exact
- **WHEN** the committed `syn-2frame-inter-mvstack-64x64.ivf` is decoded
- **THEN** `splot decode --output-format raw` reproduces the avmdec / dav2d raw
  output byte-for-byte (md5 `e5b581a55433785c0071b635d5642083`)
- **AND** the later NEARMV blocks reconstruct the earlier NEWMV block's motion
  vector from the § 7.12.2 spatial-neighbour MV stack
