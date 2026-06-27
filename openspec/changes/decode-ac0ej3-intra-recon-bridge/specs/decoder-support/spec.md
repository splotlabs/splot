## MODIFIED Requirements

### Requirement: ac0ej3 selectable transform-record support row

The `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` support row SHALL document that
the ac0ej3 selectable transform-record path now reconstructs the verified
NON-IntrABC general-intra DC region of frame 0 into a current-frame workspace and
proves it bit-exact against the pre-filter reconstruction oracle, while keeping
the public decode fail-closed before output. The row SHALL retain its prior
parse-frontier requirements and SHALL NOT claim IntrABC reconstruction, non-DC
intra modes, in-loop filtering, decoded-frame output, reference refresh, or
successful ac0ej3 decode.

#### Scenario: Support row reflects the reconstruction bridge

- **WHEN** the decoder support docs are generated for
  `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`
- **THEN** the row notes the reconstructed first-superblock region and its
  oracle-verified bit-exact proof
- **AND** it continues to mark the public ac0ej3 decode as fail-closed before
  output
