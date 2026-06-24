## ADDED Requirements

### Requirement: ac0ej3 LR Runtime Storage Retention Support Row

The decoder support model SHALL track `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` as a distinct ac0ej3 row named `ac0ej3-lr-runtime-storage-retention`. The row SHALL record that the live ac0ej3 path derives and limit-checks 10-bit loop-restoration frame-buffer shapes plus the frame-wide `LrTxSkip` grid before failing closed, and SHALL keep decoded sample population, `FilterClass` grid retention, `SubclassLookup`, loop-restoration filtering, 10-bit output, reference refresh, and successful ac0ej3 decode unsupported until separately proven.

#### Scenario: Matrix records runtime-storage frontier
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** `ac0ej3-lr-runtime-storage-retention` appears with Feature ID `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION`
- **AND** it cites AV2 §6.4.1, §7.20.1, §7.20.2, §7.20.3, §7.20.4, and §8.3.2 as applicable source sections
- **AND** it names focused tests for the live unsupported diagnostic and limit failure behavior
- **AND** it does not claim live loop-restoration filtering, output, reference refresh, AVM/dav2d equality, or successful ac0ej3 decode
