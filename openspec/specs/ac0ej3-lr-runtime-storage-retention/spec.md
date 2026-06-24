## Purpose

Track the ac0ej3 fail-closed runtime frontier after classified-Wiener storage
helper wiring, before live decoded samples or `LrTxSkip` values are populated.

## Requirements

### Requirement: ac0ej3 LR Runtime Storage Retention Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` as the live
fail-closed ac0ej3 frontier after classified-Wiener storage-helper wiring. The
runtime SHALL derive the two 10-bit loop-restoration frame-buffer storage shapes
(`CurrFrame` and `CdefFrame`) and the frame-wide `LrTxSkip` grid dimensions from
parsed AV2 sequence/frame facts, SHALL enforce existing decode limits for those
retained storage shapes, and SHALL stop before any unpopulated storage is used
as decoded source values.

#### Scenario: Live ac0ej3 reaches storage-retention frontier

- **WHEN** the local ac0ej3 mission stream reaches active classified-luma Wiener
  NS loop-restoration units
- **THEN** the runtime derives the required 10-bit current/CDEF frame storage
  footprint and `LrTxSkip` grid dimensions
- **AND** the runtime returns `decode/unsupported-feature` referencing
  `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION`
- **AND** the diagnostic states that decoded sample population, `LrTxSkip`
  values, filtering, output, and reference refresh are still unsupported

#### Scenario: Storage footprint limits fail before unsupported diagnostic

- **WHEN** caller-provided decode limits are lower than the derived 10-bit frame
  or `LrTxSkip` storage footprint
- **THEN** the runtime returns `decode/resource-limit`
- **AND** the runtime does not emit the storage-retention unsupported-feature
  diagnostic

#### Scenario: No fabricated classification values

- **WHEN** the runtime has only derived storage dimensions and byte budgets
- **THEN** it SHALL NOT call the storage-backed `FilterClass` derivation helper
  with zero-filled or otherwise fabricated `CurrFrame`, `CdefFrame`, or
  `LrTxSkip` values
