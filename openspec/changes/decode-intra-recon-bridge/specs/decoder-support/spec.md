## MODIFIED Requirements

### Requirement: local decoder mission selectable transform-record support row

The `DECODE-SELECTABLE-TRANSFORM-RECORDS` support row SHALL document that
the local decoder mission selectable transform-record path now reconstructs the verified
NON-IntrABC general-intra DC region of frame 0 into a current-frame workspace and
proves it bit-exact against the pre-filter reconstruction oracle, while keeping
the public decode fail-closed before output. The row SHALL retain its prior
parse-frontier requirements and SHALL NOT claim IntrABC reconstruction, non-DC
intra modes, in-loop filtering, decoded-frame output, reference refresh, or
successful local decoder mission decode.

#### Scenario: Support row reflects the reconstruction bridge

- **WHEN** the decoder support docs are generated for
  `DECODE-SELECTABLE-TRANSFORM-RECORDS`
- **THEN** the row notes the reconstructed first-superblock region and its
  oracle-verified bit-exact proof
- **AND** it continues to mark the public local decoder mission decode as fail-closed before
  output
