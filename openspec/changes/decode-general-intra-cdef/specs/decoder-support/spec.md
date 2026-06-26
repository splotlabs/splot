## ADDED Requirements

### Requirement: General intra § 7.18 CDEF decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-CDEF` as a distinct
partial `splot-decode` row named `general-intra-cdef`. The row SHALL cite AV2
§ 7.18, § 7.18.1, § 7.18.2, § 7.18.3, § 5.20.10.1, and § 9.2, SHALL record the
CDEF-active intra oracle tests, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointers for the `syn-2sb-cdef-intra-128x64-q130.ivf`,
`syn-2sb-cdef-intra-128x64-q120.ivf`, and
`syn-2sb-cdefdeblock-intra-128x64-q100.ivf` fixtures, and SHALL keep
multi-strength frames, `cdef_on_skip_txfm_frame_enable == 0` skip handling, 10-bit
CDEF, lossless / segmentation, multiple tiles, the other in-loop filters, inter
frames, and public APIs out of scope.

#### Scenario: Matrix records general intra CDEF support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-cdef` appears with Feature ID
  `DECODE-GENERAL-INTRA-CDEF`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim multi-strength CDEF, 10-bit CDEF, the other in-loop
  filters, or inter frames

#### Scenario: Coverage tracks the new CDEF decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the coverage includes row `general-intra-cdef` and Feature ID
  `DECODE-GENERAL-INTRA-CDEF`
- **AND** broader in-loop-filter coverage remains partial
