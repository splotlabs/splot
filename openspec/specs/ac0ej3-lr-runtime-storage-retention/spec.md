## Purpose

Track the ac0ej3 fail-closed runtime frontier after classified-Wiener storage
helper wiring, before live decoded samples or `LrTxSkip` values are populated.

## Requirements

### Requirement: ac0ej3 LR Runtime Storage Retention Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` as the live
fail-closed ac0ej3 frontier after classified-Wiener storage-helper wiring. The
runtime SHALL derive the two active-bit-depth loop-restoration frame-buffer
storage shapes (`CurrFrame` and `CdefFrame`; 10-bit for the ac0ej3 mission
stream) and the frame-wide `LrTxSkip` grid dimensions from parsed AV2
sequence/frame facts, SHALL enforce the per-frame decoded-frame limit and the
aggregate retained-storage limit for those retained storage shapes, and SHALL
stop before any unpopulated storage is used as decoded source values.

#### Scenario: Live ac0ej3 reaches storage-retention frontier

- **WHEN** the local ac0ej3 mission stream reaches active classified-luma Wiener
  NS loop-restoration units
- **THEN** the runtime derives the required active-bit-depth current/CDEF frame
  storage footprint (10-bit for the ac0ej3 mission stream) and `LrTxSkip` grid
  dimensions
- **AND** the runtime returns `decode/unsupported-feature` referencing
  `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION`
- **AND** the diagnostic states that decoded sample population, `LrTxSkip`
  values, filtering, output, and reference refresh are still unsupported

#### Scenario: Storage footprint limits fail before unsupported diagnostic

- **WHEN** caller-provided decode limits are lower than the derived
  active-bit-depth frame storage footprint (10-bit for the ac0ej3 mission
  stream) or aggregate retained-storage footprint including `LrTxSkip` storage
- **THEN** the runtime returns `decode/resource-limit`
- **AND** the runtime does not emit the storage-retention unsupported-feature
  diagnostic

#### Scenario: No fabricated classification values

- **WHEN** the runtime has only derived storage dimensions and byte budgets
- **THEN** it SHALL NOT call the storage-backed `FilterClass` derivation helper
  with zero-filled or otherwise fabricated `CurrFrame`, `CdefFrame`, or
  `LrTxSkip` values
