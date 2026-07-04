## ADDED Requirements

### Requirement: local decoder mission LR Runtime Storage Retention Support Row

The decoder support model SHALL track `DECODE-LR-RUNTIME-STORAGE-RETENTION` as a distinct local decoder mission row named `lr-runtime-storage-retention`. The row SHALL record that the live local decoder mission path derives and limit-checks 10-bit loop-restoration frame-buffer shapes plus the frame-wide `LrTxSkip` grid before failing closed, and SHALL keep decoded sample population, `FilterClass` grid retention, `SubclassLookup`, loop-restoration filtering, 10-bit output, reference refresh, and successful local decoder mission decode unsupported until separately proven.

#### Scenario: Matrix records runtime-storage frontier
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** `lr-runtime-storage-retention` appears with Feature ID `DECODE-LR-RUNTIME-STORAGE-RETENTION`
- **AND** it cites AV2 §6.4.1, §7.20.1, §7.20.2, §7.20.3, §7.20.4, and §8.3.2 as applicable source sections
- **AND** it names focused tests for the live unsupported diagnostic and limit failure behavior
- **AND** it does not claim live loop-restoration filtering, output, reference refresh, AVM/dav2d equality, or successful local decoder mission decode
