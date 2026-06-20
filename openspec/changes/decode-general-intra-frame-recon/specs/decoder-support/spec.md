## ADDED Requirements

### Requirement: General intra full frame reconstruction support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-FRAME-RECON` as a
distinct partial `splot-decode` row named `general-intra-frame-recon`. The row
SHALL cite AV2 § 5.20.7.27, § 7.13.2, § 7.14.2, § 7.14.4, § 7.15.4, § 7.14.3,
§ 8.2.4, and § 8.3.2, SHALL record the reconstructed-plane and frame-hash tests
plus the CLI test proving the general intra fixture decodes bit-exactly to the
avmdec/dav2d oracle, and SHALL keep split partitions, multiple blocks, multiple
tiles, non-64x64 frames, chroma `cctx`/CfL, inter prediction, in-loop filters,
and public APIs out of scope.

#### Scenario: Matrix records narrow full frame reconstruction support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-frame-recon` appears with Feature ID
  `DECODE-GENERAL-INTRA-FRAME-RECON`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim split partitions, multiple blocks, multiple tiles,
  inter prediction, or in-loop filters

#### Scenario: Coverage tracks the new full frame reconstruction
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-frame-recon` and Feature ID `DECODE-GENERAL-INTRA-FRAME-RECON`
- **AND** broader tile payload coverage remains partial
