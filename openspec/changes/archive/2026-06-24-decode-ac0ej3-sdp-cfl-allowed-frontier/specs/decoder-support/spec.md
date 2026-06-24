## ADDED Requirements

### Requirement: ac0ej3 SDP CflAllowedInSdp support row

The decoder support model SHALL track
`DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER` as a distinct partial ac0ej3 row. The
row SHALL describe that the minimal runtime retains AV2 §5.20.3.1
`CflAllowedInSdp` state for intra SDP chroma leaves and applies it to AV2
§5.20.5.6 chroma mode-info so disabled CfL and MHCCP syntax are not read from
the local ac0ej3 stream. The row SHALL remain fail-closed before decoded frame
samples, loop-restoration filtering/output, reference refresh, or successful
ac0ej3 decode.

#### Scenario: Matrix evidence records the SDP CfL-allowed boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-sdp-cfl-allowed-frontier` appears with Feature ID
  `DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER`
- **AND** the row cites AV2 §5.20.3.1 and §5.20.5.6
- **AND** it lists focused traversal/mode-info tests plus the local ac0ej3
  runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3
  decode

### Requirement: ac0ej3 intra prelude transform support row

The decoder support model SHALL track
`DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` as a distinct partial ac0ej3 row. The
row SHALL describe that the minimal runtime consumes the observed AV2 §5.20.5.3
`use_intrabc`, §5.20.10.1 CDEF, and §5.20.5.11 delta-Q prelude syntax before
§5.20.5.5 luma mode and §5.20.6 transform partition parsing in the local
ac0ej3 stream. The row SHALL also record the pre-tile unsupported-tool gate and
the chroma-offset leaf rejection. The row SHALL remain fail-closed before
decoded frame samples, loop-restoration filtering/output, reference refresh, or
successful ac0ej3 decode.

#### Scenario: Matrix evidence records the intra prelude transform boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-intra-prelude-tx-frontier` appears with Feature ID
  `DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER`
- **AND** the row cites AV2 §5.20.5.3, §5.20.5.11, §5.20.6, and §5.20.10.1
- **AND** it lists focused prelude/tool-gate/chroma-offset tests plus the local
  ac0ej3 runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3
  decode
